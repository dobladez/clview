//! The SVG backend, `src/source/svg.rs`, through the viewer: what resvg draws for a drawing, the
//! limits a drawing is held to, and the rule that nothing else names resvg or what it renders
//! with, so it can be swapped for another renderer. Its own tests read drawings directly.
#![cfg(all(unix, feature = "svg"))]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::view::{images, names, png_size, text, tmp, view};

const BIN: &str = env!("CARGO_BIN_EXE_clv");

/// A drawing `width` × `height` px of `body`.
fn svg(width: u32, height: u32, body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}">{body}</svg>"#)
}

/// A drawing `width` × `height` px, red on its left half and blue on its right.
fn halves(width: u32, height: u32) -> String {
    let half = width / 2;
    svg(width, height, &format!("<rect width='{half}' height='{height}' fill='red'/><rect x='{half}' width='{half}' height='{height}' fill='blue'/>"))
}

fn gzipped(bytes: &[u8]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(bytes).unwrap();
    gz.finish().unwrap()
}

fn written(test: &str, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
    let path = tmp(test, name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// What `clv` printed for `path` with `args`, which must succeed without a word: each image.
fn printed(path: &Path, args: &[&str]) -> Vec<image::RgbaImage> {
    let out = view(path, &[&["--protocol", "kitty"], args].concat());
    assert!(out.status.success() && out.stderr.is_empty(), "{args:?}: {}", text(&out.stderr));
    images(&out.stdout).iter().map(|d| image::load_from_memory(&d.png).unwrap().into_rgba8()).collect()
}

/// A drawing is a page: fitted to the window as a PDF's is, not kept to its own size as a
/// picture is, and drawn at the size shown, so its edges stay sharp.
#[test]
fn a_drawing_fills_the_window_drawn_at_the_size_shown() {
    let path = written("svg-fit", "halves.svg", halves(300, 200));
    // 300 × 200 px in the 800 × 460 px the tests' window leaves: 2.3 times as large.
    let shown = printed(&path, &[]);
    assert_eq!(shown.iter().map(|p| p.dimensions()).collect::<Vec<_>>(), [(690, 460)]);
    // The halves meet at 345 px: no pixel between them is a blend, as a picture enlarged would.
    let picture = &shown[0];
    assert_eq!((picture.get_pixel(344, 230).0, picture.get_pixel(345, 230).0), ([255, 0, 0, 255], [0, 0, 255, 255]));
    assert_eq!(printed(&path, &["--zoom", "width"])[0].dimensions(), (800, 533));
    let out = view(&path, &["--protocol", "kitty", "--pages", "2"]);
    assert_eq!(text(&out.stderr), "clv: no page 2 (halves.svg has 1 page)\n");
}

/// A drawing is known by how it starts, whatever its name, an svgz by what it unpacks to; other
/// XML is not one, even named `.svg`. A head too long to tell goes by the name.
#[test]
fn a_drawing_is_known_by_its_content_and_an_svgz_by_what_it_unpacks_to() {
    let named_otherwise = written("svg-known", "drawing.txt", format!("<?xml version='1.0'?>\n<!-- a drawing -->\n{}", halves(40, 20)));
    assert_eq!(printed(&named_otherwise, &[]).len(), 1);
    let svgz = written("svg-known", "drawing.svgz", gzipped(halves(40, 20).as_bytes()));
    assert_eq!(printed(&svgz, &[]).len(), 1);
    let plist = written("svg-known", "list.svg", "<?xml version='1.0'?><plist version='1.0'><dict/></plist>");
    let out = view(&plist, &["--protocol", "kitty"]);
    assert!(!out.status.success() && text(&out.stderr).contains("list.svg: not a file clv shows"), "{}", text(&out.stderr));
    let licence = format!("<?xml version='1.0'?>\n<!--\n{}-->\n{}", "Licensed to anyone who reads this far.\n".repeat(40), halves(40, 20));
    assert!(licence.find("<svg").unwrap() > 1024);
    assert_eq!(printed(&written("svg-known", "licensed.svg", &licence), &[]).len(), 1);
    let out = view(&written("svg-known", "licensed.xml", &licence), &["--protocol", "kitty"]);
    assert!(text(&out.stderr).contains("not a file clv shows"), "{}", text(&out.stderr));
}

/// SVG is asked before PDF: a drawing whose text holds a PDF's header within its first 1024
/// bytes, as a diagram of one might, is still a drawing.
#[test]
fn a_drawing_that_mentions_a_pdf_header_is_still_a_drawing() {
    let text = svg(300, 200, "<desc>How a PDF starts: %PDF-1.7</desc><rect width='300' height='200' fill='teal'/>");
    let path = written("svg-pdf", "about-pdfs.svg", text);
    assert_eq!(printed(&path, &[]).iter().map(|p| p.dimensions()).collect::<Vec<_>>(), [(690, 460)]);
}

/// A drawing through a pipe can't be read again when it renders, so it is kept as it came.
#[test]
fn a_drawing_through_a_pipe_is_shown() {
    let mut child = Command::new(BIN)
        .args(["/dev/stdin", "--protocol", "kitty"])
        .env("TERM", "dumb")
        .env_remove("TMUX")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&gzipped(halves(300, 200).as_bytes())).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(images(&out.stdout).iter().map(|d| png_size(&d.png)).collect::<Vec<_>>(), [(690, 460)]);
}

/// In a directory, drawings are tiles as pictures are; and in an archive, read from it.
#[test]
fn drawings_in_a_directory_or_an_archive_are_tiles() {
    let dir = tmp("svg-dir", "drawings");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.svg"), halves(300, 200)).unwrap();
    std::fs::write(dir.join("b.svgz"), gzipped(halves(100, 100).as_bytes())).unwrap();
    std::fs::write(dir.join("notes.xml"), "<?xml version='1.0'?><notes/>").unwrap();
    let out = view(&dir, &["--protocol", "kitty"]);
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    assert_eq!(names(&out.stdout), [" a.svg b.svgz"]);
    // Two to a row in 80 columns: square places 390 × 380 px, which each fills one way.
    let sizes: Vec<(u32, u32)> = images(&out.stdout).iter().map(|d| png_size(&d.png)).collect();
    assert_eq!(sizes, [(390, 260), (380, 380)]);
    #[cfg(feature = "zip")]
    {
        let mut zip = ::zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file("plans/c.svg", ::zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(halves(40, 20).as_bytes()).unwrap();
        let archive = written("svg-dir", "plans.zip", zip.finish().unwrap().into_inner());
        let out = view(&archive, &["--protocol", "kitty", "-r"]);
        assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
        assert_eq!(names(&out.stdout), [" plans/c.svg"]);
    }
}

/// A drawing reads nothing but itself: a picture it links to on disk isn't drawn, and the page
/// says how many links it didn't follow.
#[test]
fn links_outside_the_drawing_are_not_followed_and_said() {
    let red = written("svg-links", "red.svg", svg(10, 10, "<rect width='10' height='10' fill='red'/>"));
    let linking = written("svg-links", "linking.svg", svg(10, 10, &format!("<image width='10' height='10' href='{}'/>", red.display())));
    let out = view(&linking, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stderr), "warning: page 1: 1 link to another file or the network was not followed\n");
    let picture = image::load_from_memory(&images(&out.stdout)[0].png).unwrap().into_rgba8();
    assert!(picture.pixels().all(|p| p.0[3] == 0), "nothing of red.svg");
}

/// What a render would take is estimated before it starts: a page that would take too much
/// fails, with why, and smaller renders of the same drawing (its tile in a list) still come.
#[test]
fn a_drawing_that_would_take_too_much_to_render_fails_its_page() {
    // Forty layers in layers, each as large as resvg allows: five times the canvas each way, 32 MB.
    let body = format!("{}<rect x='-1e6' y='-1e6' width='2e6' height='2e6' fill='teal'/>{}", "<g opacity='0.9'>".repeat(40), "</g>".repeat(40));
    let path = written("svg-heavy", "layers.svg", svg(300, 200, &body));
    let out = view(&path, &["--protocol", "kitty"]);
    assert!(!out.status.success() && images(&out.stdout).is_empty());
    assert!(text(&out.stderr).contains("page 1: drawing it at 690 × 460 would take more than 1024 MiB"), "{}", text(&out.stderr));
    let other = written("svg-heavy", "other.svg", halves(40, 20));
    let out = view(&path, &[other.to_str().unwrap(), "--protocol", "kitty", "--zoom", "4"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(names(&out.stdout), [" layers.svg other.svg"]);
}

/// A drawing that isn't one after all, or is cut short, is refused with why, under its name.
#[test]
fn a_broken_drawing_is_refused_with_why() {
    let cut = halves(40, 20);
    let path = written("svg-broken", "cut.svg", &cut[..cut.len() - 20]);
    let out = view(&path, &["--protocol", "kitty"]);
    assert!(!out.status.success() && out.stdout.is_empty());
    assert!(text(&out.stderr).starts_with(&format!("clv: {}: not a well-formed SVG: ", path.display())), "{}", text(&out.stderr));
    let wrong = written("svg-broken", "html.svg", "<svg><rect/></svg>");
    let out = view(&wrong, &["--protocol", "kitty"]);
    assert!(text(&out.stderr).contains("isn't <svg> in the SVG namespace"), "{}", text(&out.stderr));
}

/// The backend takes no options, so `--help` has no heading for it: only the formats it shows,
/// by resvg.
#[test]
fn help_lists_the_formats_by_resvg_and_no_heading_for_a_backend_without_options() {
    let out = Command::new(BIN).arg("--help").output().unwrap();
    let help = text(&out.stdout);
    assert!(help.contains("\n  SVG drawings, by resvg: SVG (.svg), svgz (.svgz)\n") && !help.contains("\nSVG drawings, by resvg:\n"), "{help}");
}

/// The libraries that parse and render SVG.
const LIBRARIES: [&str; 6] = ["resvg", "usvg", "tiny_skia", "tiny-skia", "roxmltree", "fontdb"];

/// Only this backend and its tests name resvg and what it renders with: the rest of the code
/// would read the same with another SVG renderer.
#[test]
fn nothing_but_this_backend_names_resvg() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let own = [root.join("src/source/svg.rs"), root.join("tests/it/resvg.rs")];
    let (mut read, mut named) = (0, Vec::new());
    let mut dirs = vec![root.join("src"), root.join("tests")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
            if own.contains(&path) || path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            read += 1;
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                let lower = line.to_lowercase();
                if line.trim() != "mod resvg;" && LIBRARIES.iter().any(|library| lower.contains(library)) {
                    named.push(format!("{}:{}: {}", path.strip_prefix(root).unwrap().display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(read >= 20, "read {read} files");
    assert!(named.is_empty(), "named outside the SVG backend:\n{}", named.join("\n"));
}

/// The estimate of what a render takes follows how resvg 0.49 renders (`Estimate` in
/// `src/source/svg.rs`, ADR-0002): moving resvg means checking it again, then this.
#[test]
fn the_render_estimate_is_for_the_resvg_this_builds() {
    let lock = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.lock")).unwrap();
    for library in ["resvg", "usvg", "tiny-skia"] {
        let at = lock.find(&format!("[[package]]\nname = \"{library}\"\n")).unwrap_or_else(|| panic!("{library} isn't locked"));
        let version = lock[at..].lines().nth(2).unwrap();
        let expected = if library == "tiny-skia" { "version = \"0.12.0\"" } else { "version = \"0.49.0\"" };
        assert_eq!(version, expected, "{library} moved: check the estimate against it");
    }
}
