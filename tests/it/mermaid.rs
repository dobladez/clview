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

/// gzip `text`, its trailing checksum made wrong with `corrupt`.
#[cfg(feature = "tar")]
fn gzipped(text: &[u8], corrupt: bool) -> Vec<u8> {
    use std::io::Write as _;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(text).unwrap();
    let mut out = gz.finish().unwrap();
    if corrupt {
        let at = out.len() - 8;
        out[at] ^= 0xff;
    }
    out
}

/// A file's compression is told by its signature, whatever its name, so gzip named `big.mmd` is a
/// diagram: unpacked only to the most a diagram may be, its decoder stopped a byte past 1 MiB, so a
/// wrong checksum at its end is never reached and it is refused as more than 1 MiB. One of exactly
/// 1 MiB is unpacked whole, and drawn.
#[cfg(feature = "tar")]
#[test]
fn a_gzip_named_as_a_diagram_is_unpacked_only_to_the_most_a_diagram_may_be() {
    let padded = |size: usize| {
        let mut text = b"flowchart LR\n  a --> b\n".to_vec();
        while text.len() < size {
            text.extend_from_slice(b"%% padding\n");
        }
        text.truncate(size - 1);
        text.push(b'\n');
        text
    };
    let big = written("mermaid-gzip", "big.mmd", gzipped(&padded(2 << 20), true));
    let said = refused(&big);
    assert!(said.ends_with("big.mmd: the diagram is more than 1 MiB\n"), "{said}");
    let exact = padded(1 << 20);
    assert_eq!(exact.len(), 1 << 20);
    assert_eq!(printed(&written("mermaid-gzip", "exact.mmd", gzipped(&exact, false))).len(), 1);
}

// A document's fenced blocks: ` ```mermaid ` drawn as its diagram (Spec-0009).

/// `flow.mmd`'s diagram, as a document's block holds it: its red node left of its blue one.
#[cfg(feature = "word")]
fn flow() -> String {
    std::fs::read_to_string(data("flow.mmd")).unwrap()
}

/// What `clv` printed of the document at `path`, with `args`: its pages, and what it said.
#[cfg(feature = "word")]
fn pages_at(path: &Path, args: &[&str]) -> (Vec<image::RgbaImage>, String) {
    let out = view(path, &[&["--protocol", "kitty"], args].concat());
    assert!(out.status.success(), "{}", text(&out.stderr));
    (images(&out.stdout).iter().map(|d| image::load_from_memory(&d.png).unwrap().into_rgba8()).collect(), text(&out.stderr))
}

/// [`pages_at`], for a document of `contents` named `name`.
#[cfg(feature = "word")]
fn pages_of(test: &str, name: &str, contents: &str) -> (Vec<image::RgbaImage>, String) {
    pages_at(&written(test, name, contents), &[])
}

/// Whether `page` shows the flow drawn: its red node, left of its blue one.
#[cfg(feature = "word")]
fn shows_flow(page: &image::RgbaImage) -> bool {
    match (spread(page, [255, 0, 0]), spread(page, [0, 0, 255])) {
        (Some(red), Some(blue)) => red.1 < blue.0,
        _ => false,
    }
}

/// How many of `page`'s pixels are dark: text's ink.
#[cfg(feature = "word")]
fn ink(page: &image::RgbaImage) -> usize {
    page.pixels().filter(|p| p.0[..3].iter().all(|&v| v < 128)).count()
}

/// A ` ```mermaid ` block in Markdown is drawn as its diagram, its language the first word of its
/// info string in any case, behind backticks or tildes. A block no format lists stays code, an SVG's
/// too, as does one that isn't fenced, or says its language otherwise.
#[cfg(feature = "word")]
#[test]
fn a_mermaid_block_in_markdown_is_drawn_as_its_diagram() {
    let flow = flow();
    for (open, close) in [("```mermaid", "```"), ("```Mermaid title=\"A flow\"", "```"), ("~~~~ MERMAID", "~~~~")] {
        let (pages, said) = pages_of("mermaid-block", "a.md", &format!("# A flow\n\nBefore.\n\n{open}\n{flow}{close}\n\nAfter.\n"));
        assert!(said.is_empty(), "{open}: {said}");
        assert!(pages.len() == 1 && shows_flow(&pages[0]), "{open}");
    }
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="60" height="60"><rect width="60" height="60" fill="rgb(255,0,0)"/></svg>"#;
    let indented: String = flow.lines().map(|line| format!("    {line}\n")).collect();
    let code = [
        format!("```svg\n{svg}\n```\n"),
        format!("```\n{flow}```\n"),
        indented,
        format!("```mmd\n{flow}```\n"),
        format!("```text mermaid\n{flow}```\n"),
    ];
    for block in code {
        let (pages, said) = pages_of("mermaid-block", "code.md", &format!("Before.\n\n{block}\nAfter.\n"));
        assert!(said.is_empty(), "{block}: {said}");
        let coloured = spread(&pages[0], [255, 0, 0]).or(spread(&pages[0], [0, 0, 255]));
        assert!(coloured.is_none() && ink(&pages[0]) > 500, "{block}: {coloured:?}, {} dark pixels", ink(&pages[0]));
    }
}

/// A block its backend can't draw (a broken diagram, one past the most a diagram may be) is the code
/// block it would have been, exactly, and the first page says how many weren't drawn; the others are
/// drawn.
#[cfg(feature = "word")]
#[test]
fn a_mermaid_block_that_cant_be_drawn_shows_as_its_code_and_the_first_page_says_so() {
    let (flow, broken) = (flow(), std::fs::read_to_string(data("broken.mmd")).unwrap());
    let document = |language: &str| format!("```mermaid\n{flow}```\n\nThen one that is broken.\n\n```{language}\n{broken}```\n");
    let (drawn, said) = pages_of("mermaid-undrawn", "a.md", &document("mermaid"));
    assert_eq!(said, "warning: page 1: one of its blocks could not be drawn, and is shown as its code\n");
    assert!(shows_flow(&drawn[0]), "the block before it is drawn");
    let (code, said) = pages_of("mermaid-undrawn", "a.md", &document("text"));
    assert!(said.is_empty(), "{said}");
    assert!(drawn[0] == code[0], "not the code block it would have been");
    let (_, said) = pages_of("mermaid-undrawn", "a.md", &format!("```mermaid\n{broken}```\n\n```mermaid\nno diagram\n```\n\n```mermaid\n```\n"));
    assert_eq!(said, "warning: page 1: 3 of its blocks could not be drawn, and are shown as their code\n");
    // Past 1 MiB, the most a diagram may be: code, its first page among many.
    let large = written("mermaid-undrawn", "large.md", format!("```mermaid\nflowchart LR\n{}```\n", "%% padding\n".repeat(100_000)));
    let (pages, said) = pages_at(&large, &["--pages", "1"]);
    assert_eq!(said, "warning: page 1: one of its blocks could not be drawn, and is shown as its code\n");
    assert!(ink(&pages[0]) > 500, "{} dark pixels", ink(&pages[0]));
}

/// A block's text travels in the document, so it is drawn where a picture by path can't be reached:
/// in Markdown come through a pipe, and held in an archive.
#[cfg(feature = "word")]
#[test]
fn a_mermaid_block_is_drawn_through_a_pipe_and_in_an_archive() {
    let document = format!("# A flow\n\n```mermaid\n{}```\n", flow());
    // Markdown is known by its name alone: a pipe named as one.
    let pipe = tmp("mermaid-piped", "piped.md");
    let _ = std::fs::remove_file(&pipe);
    assert!(Command::new("mkfifo").arg(&pipe).status().unwrap().success());
    let (to, contents) = (pipe.clone(), document.clone());
    let writer = std::thread::spawn(move || std::fs::write(to, contents).unwrap());
    let (pages, said) = pages_at(&pipe, &[]);
    writer.join().unwrap();
    assert!(said.is_empty(), "{said}");
    assert!(pages.len() == 1 && shows_flow(&pages[0]), "through a pipe");
    #[cfg(feature = "zip")]
    {
        let archive = tmp("mermaid-piped", "docs.zip");
        let mut zip = ::zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        zip.start_file("readme.md", ::zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(document.as_bytes()).unwrap();
        zip.finish().unwrap();
        let (tiles, said) = pages_at(&archive, &[]);
        assert!(said.is_empty(), "{said}");
        assert!(tiles.len() == 1 && shows_flow(&tiles[0]), "in a zip");
    }
}

/// A block's picture is known to the document by a key made afresh each time it is read, so an
/// `<img>` the document writes itself, naming the key a block would have otherwise, is no picture: it
/// shows its alt text, as one that names nothing here does.
#[cfg(feature = "word")]
#[test]
fn a_mermaid_blocks_key_written_in_the_document_is_no_picture() {
    let flow = flow();
    let document = |src: &str| format!("```mermaid\n{flow}```\n\nThen <img src=\"{src}\" alt=\"what it says instead\"> and the end.\n");
    let (forged, said) = pages_of("mermaid-forged", "a.md", &document("fenced:0000000000000000:0"));
    assert!(said.is_empty(), "{said}");
    let (nothing, said) = pages_of("mermaid-forged", "a.md", &document("https://example.com/nothing.png"));
    assert!(said.is_empty(), "{said}");
    assert!(shows_flow(&forged[0]) && forged[0] == nothing[0], "the block's picture, shown for a key the document wrote");
}

/// A document's blocks count among its 1,024 pictures: of 1,025, the last stays code, and the first
/// page says so.
#[cfg(feature = "word")]
#[test]
fn a_mermaid_block_past_the_most_pictures_a_document_has_stays_code() {
    let block = "```mermaid\npie\n  \"a\" : 1\n```\n\n";
    let path = written("mermaid-many", "many.md", block.repeat(1025));
    let (pages, said) = pages_at(&path, &["--pages", "1"]);
    assert_eq!(pages.len(), 1);
    assert_eq!(said, "warning: page 1: one of its blocks could not be drawn, and is shown as its code\n");
    let (_, said) = pages_at(&written("mermaid-many", "all.md", block.repeat(1024)), &["--pages", "1"]);
    assert!(said.is_empty(), "{said}");
}

/// `text` as HTML's text: its `&`, `<` and `>` as entities.
#[cfg(feature = "word")]
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// A `<pre>` of HTML that says it is Mermaid's is drawn as its diagram, its entities decoded (the
/// flow's `-->` is `--&gt;` there): by its `<code>`'s `language-mermaid`, as Markdown's renderers
/// write it, by GitHub's `lang`, and by pandoc's class. One that says another language, or none,
/// stays code, as does one in a comment.
#[cfg(feature = "word")]
#[test]
fn a_mermaid_block_in_html_is_drawn_as_its_diagram() {
    let flow = escaped(&flow());
    assert!(flow.contains("--&gt;"));
    let page = |block: &str| format!("<!doctype html>\n<html><body>\n<h1>A flow</h1>\n<p>Before.</p>\n{block}\n<p>After.</p>\n</body></html>\n");
    for block in [
        format!("<pre><code class=\"language-mermaid\">{flow}</code></pre>"),
        format!("<pre class=\"language-mermaid\">{flow}</pre>"),
        format!("<pre lang=\"mermaid\" aria-label=\"Raw mermaid code\">{flow}</pre>"),
        format!(
            "<div class=\"sourceCode\" id=\"cb1\"><pre\nclass=\"sourceCode mermaid\"><code class=\"sourceCode mermaid\">{flow}</code></pre></div>"
        ),
    ] {
        let (pages, said) = pages_of("mermaid-html", "a.html", &page(&block));
        assert!(said.is_empty(), "{block}: {said}");
        assert!(pages.len() == 1 && shows_flow(&pages[0]), "{block}");
    }
    for block in [
        format!("<pre><code class=\"language-text\">{flow}</code></pre>"),
        format!("<pre>{flow}</pre>"),
        format!("<!-- <pre lang=\"mermaid\">{flow}</pre> -->"),
    ] {
        let (pages, said) = pages_of("mermaid-html", "code.html", &page(&block));
        assert!(said.is_empty(), "{block}: {said}");
        assert!(spread(&pages[0], [255, 0, 0]).is_none(), "{block}");
    }
    // A broken one is the code it was, and the first page says so.
    let broken = escaped(&std::fs::read_to_string(data("broken.mmd")).unwrap());
    let document = |language: &str| page(&format!("<pre><code class=\"language-{language}\">{broken}</code></pre>"));
    let (drawn, said) = pages_of("mermaid-html", "broken.html", &document("mermaid"));
    assert_eq!(said, "warning: page 1: one of its blocks could not be drawn, and is shown as its code\n");
    let (code, _) = pages_of("mermaid-html", "broken.html", &document("text"));
    assert!(drawn[0] == code[0], "not the code block it would have been");
}

/// HTML says what it is by how it starts, so its blocks are drawn through a pipe; and an epub's
/// chapter, XHTML held in a zip, draws its own, where its pictures by path can't be reached.
#[cfg(feature = "word")]
#[test]
fn a_mermaid_block_in_html_is_drawn_through_a_pipe_and_in_an_epub() {
    let block = format!("<pre><code class=\"language-mermaid\">{}</code></pre>", escaped(&flow()));
    let mut child = Command::new(BIN)
        .args(["/dev/stdin", "--protocol", "kitty"])
        .env("TERM", "dumb")
        .env_remove("TMUX")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(format!("<!doctype html>\n<h1>A flow</h1>\n{block}\n").as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    let pages = images(&out.stdout);
    assert!(pages.len() == 1 && shows_flow(&image::load_from_memory(&pages[0].png).unwrap().into_rgba8()), "through a pipe");
    #[cfg(feature = "zip")]
    {
        let chapter = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\">\n<head><title>One</title></head>\n<body>\n<h1>Chapter one</h1>\n{block}\n</body>\n</html>\n"
        );
        let book = tmp("mermaid-epub", "book.epub");
        let mut zip = ::zip::ZipWriter::new(std::fs::File::create(&book).unwrap());
        let stored = ::zip::write::SimpleFileOptions::default().compression_method(::zip::CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();
        zip.start_file("chapter-1.xhtml", ::zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(chapter.as_bytes()).unwrap();
        zip.finish().unwrap();
        let (tiles, said) = pages_at(&book, &[]);
        assert!(said.is_empty(), "{said}");
        assert!(tiles.len() == 1 && shows_flow(&tiles[0]), "an epub's chapter");
    }
}
