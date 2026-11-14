//! The Mermaid backend, `src/source/mermaid.rs`, through the viewer: which files it opens, what
//! merman draws of them, through the SVG backend, the limits a diagram is held to, the worker
//! process it is drawn in, and the rule that nothing else names merman.
#![cfg(all(unix, feature = "mermaid"))]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::view::{images, names, text, tmp, view};

const BIN: &str = env!("CARGO_BIN_EXE_clv");

/// A fixture of `tests/data/`.
fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
}

fn written(test: &str, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
    let path = tmp(test, name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// What `clv` printed for `path`, which must succeed without a word: each image.
fn printed(path: &Path) -> Vec<image::RgbaImage> {
    let out = view(path, &["--protocol", "kitty"]);
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    images(&out.stdout).iter().map(|d| image::load_from_memory(&d.png).unwrap().into_rgba8()).collect()
}

/// What `clv` said on stderr for `path`, which it must refuse, printing nothing.
fn refused(path: &Path) -> String {
    let out = view(path, &["--protocol", "kitty"]);
    assert!(!out.status.success() && out.stdout.is_empty(), "{}", text(&out.stderr));
    text(&out.stderr)
}

/// Where `picture` has pixels near `colour`, as the share of its width from its left where they
/// start and end; `None` where it has none.
fn spread(picture: &image::RgbaImage, colour: [u8; 3]) -> Option<(f32, f32)> {
    let near = |p: &image::Rgba<u8>| p.0.iter().zip(colour).all(|(a, b)| a.abs_diff(b) < 40);
    let xs: Vec<u32> = picture.enumerate_pixels().filter(|(_, _, p)| near(p)).map(|(x, _, _)| x).collect();
    let width = picture.width() as f32;
    Some((*xs.iter().min()? as f32 / width, *xs.iter().max()? as f32 / width))
}

/// A diagram is one page, drawn as merman draws it: `flow.mmd`'s red node on the left and its blue
/// one on the right, on white, as Mermaid's ink is made for; and a `.mermaid` by its name.
#[test]
fn a_diagram_is_drawn_as_merman_draws_it() {
    let shown = printed(&data("flow.mmd"));
    assert_eq!(shown.len(), 1);
    let picture = &shown[0];
    let (red, blue) = (spread(picture, [255, 0, 0]).unwrap(), spread(picture, [0, 0, 255]).unwrap());
    assert!(red.1 < 0.5 && blue.0 > 0.5, "red at {red:?}, blue at {blue:?}");
    on_white(picture);
    let shown = printed(&data("sequence.mermaid"));
    assert_eq!(shown.len(), 1);
    let ink = shown[0].pixels().filter(|p| p.0[..3].iter().all(|&v| v < 128)).count();
    assert!(ink > 100, "{ink} dark pixels");
    // A sequence diagram's viewBox starts left of and above its origin (-50, -10): white from there.
    on_white(&shown[0]);
}

/// Whether `picture` is on white over all of it, so on a dark terminal too, where an SVG drawing's
/// transparency shows the terminal's background: opaque, its corners white.
fn on_white(picture: &image::RgbaImage) {
    let (w, h) = picture.dimensions();
    for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
        assert_eq!(picture.get_pixel(x, y).0, [255, 255, 255, 255], "on white at {x}, {y}");
    }
    assert!(picture.pixels().all(|p| p.0[3] == 255), "opaque");
}

/// A `.mmd` is opened only when it starts as a diagram, within its first 1024 bytes: past blank
/// lines, `%%` comments and front matter, a kind of diagram merman draws, or an `init` directive.
/// MultiMarkdown, which names its files so too, is left unopened, as is a diagram that starts past
/// them, or whose first line is cut there.
#[test]
fn a_mmd_is_opened_when_it_starts_as_a_diagram() {
    let not_shown = |stderr: &str| stderr.contains(": not a file clv shows (it shows ");
    let said = refused(&data("notes.mmd"));
    assert!(not_shown(&said), "{said}");
    let flow = "flowchart LR\n  a --> b\n";
    let starts = [
        format!("\u{feff}\n\n%% a comment\n{flow}"),
        format!("---\ntitle: A flow\n---\n{flow}"),
        format!("%%{{init: {{\"theme\": \"forest\"}}}}%%\n{flow}"),
        "stateDiagram-v2\n  [*] --> Still\n".into(),
        "graph TD\r\n  a --> b\r\n".into(),
    ];
    for (n, start) in starts.iter().enumerate() {
        let path = written("mermaid-mmd", &format!("starts-{n}.mmd"), start);
        assert_eq!(printed(&path).len(), 1, "{start:?}");
    }
    let comments = "%% a comment of forty bytes, give or take\n".repeat(25);
    assert!(comments.len() > 1024);
    let past = written("mermaid-mmd", "past.mmd", format!("{comments}{flow}"));
    assert!(not_shown(&refused(&past)));
    // Its first line cut by the 1024th byte: no line.
    let cut = written("mermaid-mmd", "cut.mmd", format!("{}{flow}", "%%\n".repeat(341)));
    assert!(not_shown(&refused(&cut)));
    // A character cut there, past its first line, leaves that line whole.
    let after = format!("{flow}%%  {}\n", "é".repeat(600));
    assert!(!after.is_char_boundary(1024));
    assert_eq!(printed(&written("mermaid-mmd", "after.mmd", after)).len(), 1);
    let wrong = written("mermaid-mmd", "wrong.mmd", "flowcharts LR\n  a --> b\n");
    assert!(not_shown(&refused(&wrong)));
}

/// A diagram has no signature, so through a pipe it is no diagram.
#[test]
fn a_diagram_through_a_pipe_is_refused() {
    for name in ["-", "/dev/stdin"] {
        let mut child = Command::new(BIN)
            .args([name, "--protocol", "kitty"])
            .env("TERM", "dumb")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // It may refuse before it has read it all.
        let _ = child.stdin.take().unwrap().write_all(&std::fs::read(data("flow.mmd")).unwrap());
        let out = child.wait_with_output().unwrap();
        assert!(!out.status.success() && out.stdout.is_empty(), "{name}");
        // `-` is no name of clv's for stdin: no such file.
        let said = if name == "-" { "-: No such file" } else { "not a file clv shows" };
        assert!(text(&out.stderr).contains(said), "{name}: {}", text(&out.stderr));
    }
}

/// A diagram merman can't read is refused with merman's message: one line, short, under the
/// file's name.
#[test]
fn a_broken_diagram_is_refused_with_merman_s_message() {
    let path = data("broken.mmd");
    let said = refused(&path);
    let prefix = format!("clv: {}: merman: ", path.display());
    assert!(said.starts_with(&prefix) && said.ends_with('\n'), "{said}");
    let message = &said[prefix.len()..said.len() - 1];
    assert!(!message.is_empty() && message.len() <= 256 && !message.chars().any(char::is_control), "{message:?}");
}

/// A diagram past 1 MiB, or not UTF-8, is refused before merman reads it; one past Mermaid's 500
/// edges, by merman, with its message.
#[test]
fn a_diagram_past_its_limits_is_refused_with_why() {
    let large = written("mermaid-limits", "large.mmd", format!("flowchart LR\n{}", "%% padding\n".repeat(100_000)));
    let said = refused(&large);
    assert!(said.ends_with("large.mmd: the diagram is more than 1 MiB\n"), "{said}");
    let latin = written("mermaid-limits", "latin.mmd", b"flowchart LR\n  a[Caf\xe9] --> b\n");
    let said = refused(&latin);
    assert!(said.ends_with("latin.mmd: the diagram isn't UTF-8 text\n"), "{said}");
    let edges: String = (0..501).map(|i| format!("  n{i} --> n{}\n", i + 1)).collect();
    let many = written("mermaid-limits", "edges.mmd", format!("flowchart LR\n{edges}"));
    let said = refused(&many);
    assert!(said.contains("edges.mmd: merman: ") && said.contains("edge limit exceeded"), "{said}");
}

/// Only this backend and its tests name merman: the rest of the code would read the same with
/// another Mermaid renderer.
#[test]
fn nothing_but_this_backend_names_merman() {
    crate::fixtures::named_only_in(&["src/source/mermaid.rs", "tests/it/mermaid.rs"], &["mod mermaid;"], &["merman"]);
}

/// A list of diagrams, printed: each one a tile.
#[test]
fn diagrams_in_a_directory_are_tiles() {
    let dir = tmp("mermaid-dir", "diagrams");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(data("flow.mmd"), dir.join("a.mmd")).unwrap();
    std::fs::copy(data("sequence.mermaid"), dir.join("b.mermaid")).unwrap();
    std::fs::copy(data("notes.mmd"), dir.join("c.mmd")).unwrap();
    let out = view(&dir, &["--protocol", "kitty"]);
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    assert_eq!(names(&out.stdout), [" a.mmd b.mermaid"]);
    assert_eq!(images(&out.stdout).len(), 2);
}

/// The diagrams made to cost merman the most within its limits (ADR-0031's `gen within`), each one
/// limit's worst: drawn in the worker within its CPU time and heap, one after another, a list's
/// tiles (the densest then past the SVG backend's limits); and past one of merman's limits (its
/// layout's work), refused with its message, not killed.
#[test]
fn the_costliest_diagrams_merman_accepts_are_drawn_under_the_caps() {
    use std::fmt::Write as _;
    let dir = tmp("mermaid-costliest", "diagrams");
    std::fs::create_dir_all(&dir).unwrap();
    let put = |name: &str, text: String| std::fs::write(dir.join(name), text).unwrap();
    let mut s = String::from("flowchart LR\n");
    (0..32).for_each(|i| (i + 1..32).for_each(|j| writeln!(s, "  a{i} --> a{j}").unwrap()));
    put("complete-32.mmd", s);
    let mut s = String::from("flowchart TD\n");
    (0..15_000).for_each(|i| writeln!(s, "  n{i}[Node {i}]").unwrap());
    put("nodes-15000.mmd", s);
    let mut s = String::from("classDiagram\n");
    for c in 0..300 {
        writeln!(s, "class C{c} {{").unwrap();
        (0..50).for_each(|m| writeln!(s, "  +field{m} int").unwrap());
        s.push_str("}\n");
    }
    put("class-15000.mmd", s);
    put("gantt-years.mmd", "gantt\n  dateFormat YYYY-MM-DD\n  tickInterval 1day\n  section A\n  Long :a, 0001-01-01, 9999-12-31\n".into());
    let mut s = String::from("sequenceDiagram\n");
    (0..15_000).for_each(|i| writeln!(s, "  p{} ->> p{}: m{i}", i % 20, (i * 7 + 3) % 20).unwrap());
    put("sequence-15000.mmd", s);
    let mut s = String::from("pie\n");
    (0..10_000).for_each(|i| writeln!(s, "  \"s{i}\" : {}", i + 1).unwrap());
    put("pie-10000.mmd", s);
    put("label-unbroken.mmd", format!("flowchart TD\n  A[\"{}\"]\n", "x".repeat(1_000_000)));
    let mut s = String::from("flowchart TD\n");
    (0..100).for_each(|i| writeln!(s, "subgraph s{i}\n  x{i}").unwrap());
    s.push_str(&"end\n".repeat(100));
    (0..499).for_each(|i| writeln!(s, "  x{} --> x{}", i % 100, (i * 37 + 11) % 100).unwrap());
    put("z-nested-crossing.mmd", s);
    let out = view(&dir, &["--protocol", "kitty"]);
    let stderr = text(&out.stderr);
    assert!(!stderr.contains("merman took too") && !stderr.contains("merman failed"), "{stderr}");
    // Each tile opened as it is printed: what fails is said at the end, by its page.
    assert!(stderr.contains("z-nested-crossing.mmd: merman: resource limit"), "{stderr}");
    // Drawn by merman, its 496 edges' arrowheads are past the SVG backend's own limits.
    assert!(stderr.contains("complete-32.mmd: it comes to more than 128 MiB once what its uses, markers and references repeat"), "{stderr}");
    assert!(!out.status.success());
    assert_eq!(images(&out.stdout).len(), 6, "{stderr}");
}

/// A list's diagram is opened when its tile is wanted: `--pages 1` never reaches a broken one past
/// it; printed whole, the broken one costs its own tile, its place kept, and says why at the end, by
/// its page.
#[test]
fn a_lists_diagram_is_opened_when_its_tile_is_wanted_and_a_broken_one_costs_its_tile() {
    let dir = tmp("mermaid-later", "diagrams");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(data("flow.mmd"), dir.join("a.mmd")).unwrap();
    std::fs::copy(data("broken.mmd"), dir.join("b.mmd")).unwrap();
    std::fs::copy(data("sequence.mermaid"), dir.join("c.mermaid")).unwrap();
    let out = view(&dir, &["--protocol", "kitty", "--pages", "1"]);
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    assert_eq!(images(&out.stdout).len(), 1);
    let out = view(&dir, &["--protocol", "kitty"]);
    assert_eq!(names(&out.stdout), [" a.mmd b.mmd c.mermaid"], "its place kept");
    assert_eq!(images(&out.stdout).len(), 2, "the others shown");
    let stderr = text(&out.stderr);
    assert!(!out.status.success() && stderr.starts_with("clv: page 2: b.mmd: merman: "), "{stderr}");
}

/// In text cells, a diagram is what the SVG backend makes of its drawing: its ink in characters.
#[test]
fn a_diagram_in_text_cells() {
    for protocol in ["ascii", "blocks"] {
        let out = view(&data("flow.mmd"), &["--protocol", protocol]);
        assert!(out.status.success() && out.stderr.is_empty(), "{protocol}: {}", text(&out.stderr));
        let shown = text(&out.stdout);
        assert!(shown.lines().count() > 5 && shown.chars().filter(|c| !c.is_whitespace()).count() > 100, "{protocol}: {shown}");
    }
}

/// A diagram in a zip is read only to the most a diagram may be: one that unpacks past the archive's
/// own 512 MiB is refused as a diagram past 1 MiB, unpacked no further, not as a file past what the
/// archive unpacks. It compresses to some 600 KB.
#[cfg(feature = "zip")]
#[test]
fn a_diagram_in_an_archive_is_unpacked_only_to_the_most_a_diagram_may_be() {
    let path = tmp("mermaid-archive", "big.zip");
    let mut zip = ::zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    zip.start_file("big.mmd", ::zip::write::SimpleFileOptions::default().large_file(true)).unwrap();
    zip.write_all(b"flowchart LR\n").unwrap();
    let piece = "%% padding\n".repeat((1 << 20) / 11);
    for _ in 0..520 {
        zip.write_all(piece.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let out = view(&path, &["--protocol", "kitty"]);
    let stderr = text(&out.stderr);
    assert!(!out.status.success() && stderr.contains("big.mmd: the diagram is more than 1 MiB"), "{stderr}");
}
