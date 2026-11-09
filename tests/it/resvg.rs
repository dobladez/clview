//! The SVG backend, `src/source/svg.rs`, through the viewer: what resvg draws for a drawing, the
//! limits a drawing is held to, its HTML labels (`src/source/svg/html.rs`), and the rule that
//! nothing else names resvg, Blitz or what they render with, so either can be swapped for
//! another. Its own tests read drawings directly.
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
    assert_eq!(printed(&path, &["--per-row", "width"])[0].dimensions(), (800, 533));
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
    let out = view(&path, &[other.to_str().unwrap(), "--protocol", "kitty", "--per-row", "4"]);
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

/// A `<foreignObject>`, 20 × 10, holding `html` in an XHTML `div`, with `attributes`.
fn label(attributes: &str, html: &str) -> String {
    format!("<foreignObject width='20' height='10' {attributes}><div xmlns='http://www.w3.org/1999/xhtml'>{html}</div></foreignObject>")
}

/// What `clv` warned of `body`, a 40 × 20 drawing, printed: each warning after `page 1: `.
fn warned(test: &str, body: &str) -> Vec<String> {
    let path = written(test, "labels.svg", svg(40, 20, body));
    let out = view(&path, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{body}: {}", text(&out.stderr));
    // One line, its warnings joined by "; ".
    text(&out.stderr)
        .lines()
        .flat_map(|l| l.strip_prefix("warning: page 1: ").unwrap_or(l).split("; ").map(str::to_string).collect::<Vec<_>>())
        .collect()
}

/// How many HTML labels `clv` said it didn't draw, of `body`.
fn undrawn(test: &str, body: &str) -> usize {
    let warnings = warned(test, body);
    let line = warnings.iter().find(|l| l.contains("(<foreignObject>)") && l.ends_with("not drawn"));
    line.map_or(0, |l| l.split(' ').next().unwrap().parse().unwrap())
}

/// One rule says which labels a browser draws, and the warning counts those this build doesn't:
/// none in a definition, a hidden one, or one a `<switch>` doesn't take, as usvg takes its
/// conditions (`requiredFeatures` met); those a `<use>` repeats are counted too, as left out.
#[test]
fn the_labels_a_browser_draws_are_told_by_one_rule() {
    let fo = label("", "a");
    let cases: Vec<(String, usize)> = vec![
        (fo.clone(), 1),
        (format!("<g>{fo}{fo}</g>"), 2),
        (format!("<defs>{fo}</defs>"), 0),
        (format!("<symbol id='s'>{fo}</symbol>"), 0),
        (format!("<clipPath id='c'>{fo}</clipPath>"), 0),
        (format!("<mask id='m'>{fo}</mask>"), 0),
        (format!("<pattern id='p'>{fo}</pattern>"), 0),
        (format!("<marker id='k'>{fo}</marker>"), 0),
        (label("display='none'", "a"), 0),
        (label("style='color: red; display : none'", "a"), 0),
        (format!("<g visibility='hidden'>{fo}</g>"), 0),
        (format!("<g style='visibility:collapse'>{fo}</g>"), 0),
        (format!("<switch>{fo}<rect width='1' height='1'/></switch>"), 1),
        (format!("<switch>{}{fo}</switch>", label("systemLanguage='fr'", "a")), 1),
        (format!("<switch>{}{fo}</switch>", label("systemLanguage='en-GB, fr'", "a")), 1),
        (format!("<switch>{}<rect width='1' height='1'/></switch>", label("requiredExtensions='http://example.com/x'", "a")), 0),
        (
            format!(
                "<switch>{}<rect width='1' height='1'/></switch>",
                label("requiredFeatures='http://www.w3.org/TR/SVG11/feature#Extensibility'", "a")
            ),
            1,
        ),
        (format!("<switch><g requiredFeatures='http://www.w3.org/TR/SVG11/feature#Extensibility'/>{fo}</switch>"), 0),
        (format!("<switch><g><switch>{fo}</switch></g>{fo}</switch>"), 1),
        (format!("<switch><rect width='1' height='1'/><g>{fo}</g></switch>"), 0),
        (format!("<defs><g id='l'>{fo}</g></defs><use href='#l'/>"), 1),
        (label("", &format!("<svg xmlns='http://www.w3.org/2000/svg'>{fo}</svg>")), 1),
    ];
    for (body, expected) in cases {
        assert_eq!(undrawn("svg-labels", &body), expected, "{body}");
    }
}

/// A `<switch>`'s fallback after the label a browser takes (draw.io's "[Not supported by
/// viewer]"), or after a child asking for `requiredFeatures` usvg lacks (newer draw.io's "Text is
/// not SVG"), isn't drawn where labels are, and is where they aren't, as it was.
#[test]
fn a_fallback_is_drawn_only_where_labels_are_not() {
    let red = "<rect width='40' height='20' fill='red'/>";
    for first in [label("", "a"), "<g requiredFeatures='http://www.w3.org/TR/SVG11/feature#Extensibility'/>".to_string()] {
        let path = written("svg-fallback", "fallback.svg", svg(40, 20, &format!("<switch>{first}{red}</switch>")));
        let out = view(&path, &["--protocol", "kitty"]);
        assert!(out.status.success(), "{}", text(&out.stderr));
        let picture = image::load_from_memory(&images(&out.stdout)[0].png).unwrap().into_rgba8();
        let expected = if cfg!(feature = "svg-html") { [0, 0, 0, 0] } else { [255, 0, 0, 255] };
        assert_eq!(picture.get_pixel(10, 10).0, expected, "{first}");
    }
}

/// The fallbacks are taken out of a drawing already bounded: entities a fallback holds, past what
/// a drawing may expand to, refuse it before anything is parsed.
#[test]
fn entities_in_a_fallback_are_refused_before_it_is_parsed() {
    let big = "x".repeat(64 << 10);
    let fallback = format!("<text>{}</text>", "&big;".repeat(2100));
    let body = format!("<switch>{}{fallback}</switch>", label("", "a"));
    let drawing = format!("<!DOCTYPE svg [<!ENTITY big '{big}'>]>{}", svg(40, 20, &body));
    let out = view(&written("svg-entities", "fallback.svg", drawing), &["--protocol", "kitty"]);
    assert!(!out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("its entities expand to more than 128 MiB"), "{}", text(&out.stderr));
}

/// What a drawing's labels may cost is bounded as they are found and written out: past a limit,
/// that label or the rest are left out, and said.
#[cfg(feature = "svg-html")]
#[test]
fn labels_past_their_limits_are_left_out_and_said() {
    let said = |body: &str, why: &str, n: usize| {
        let warnings = warned("svg-limits", body);
        let one = if n == 1 { "1 HTML label (<foreignObject>) was".to_string() } else { format!("{n} HTML labels (<foreignObject>) were") };
        let expected = format!("{one} left out, {why}");
        assert!(warnings.contains(&expected), "{expected}: {warnings:?}");
    };
    let deep = |n: usize| label("", &format!("{}x{}", "<div>".repeat(n), "</div>".repeat(n)));
    // The label's own div is its first level.
    said(&format!("{}{}{}", deep(63), deep(64), deep(63)), "nested more than 64 deep", 1);
    assert!(!warned("svg-limits", &deep(63)).iter().any(|w| w.contains("left out")));
    said(&label("", "x").repeat(1026), "past 1024 of them", 2);
    said(&format!("{}{}", label("", &"y".repeat((1 << 20) + 1)), label("", "z")), "past 1 MiB of text", 2);
    said(&label("", &"<i/>".repeat(100_000)), "past 100000 elements", 1);
    let title = "t".repeat(1000);
    said(&label("", &format!("<i title='{title}'/>").repeat(9000)), "past 8 MiB of markup", 1);
}

/// The libraries that parse and render SVG.
const LIBRARIES: [&str; 6] = ["resvg", "usvg", "tiny_skia", "tiny-skia", "roxmltree", "fontdb"];

/// Only this backend and its tests name resvg and what it renders with: the rest of the code
/// would read the same with another SVG renderer.
#[test]
fn nothing_but_this_backend_names_resvg() {
    crate::fixtures::named_only_in(&["src/source/svg.rs", "tests/it/resvg.rs"], &["mod resvg;"], &LIBRARIES);
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

/// What draws an SVG's HTML labels: Blitz, and what it lays out and paints with.
const HTML_LIBRARIES: [&str; 7] = ["blitz", "anyrender", "kurbo", "stylo", "parley", "taffy", "wuff"];

/// Only the labels' painter and these tests name Blitz and what it paints with: the SVG backend
/// would read the same with another HTML engine.
#[test]
fn nothing_but_svg_html_names_blitz() {
    crate::fixtures::named_only_in(&["src/source/svg/html.rs", "tests/it/resvg.rs"], &[], &HTML_LIBRARIES);
}

/// The limits on what a drawing's labels cost were measured with these versions (ADR-0024):
/// moving any of them means measuring again, then this.
#[test]
fn the_label_limits_are_for_the_blitz_this_builds() {
    let lock = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.lock")).unwrap();
    let blitz = ["blitz-dom", "blitz-html", "blitz-paint", "blitz-traits"].map(|library| (library, "0.3.0-beta.4"));
    for (library, expected) in blitz.into_iter().chain([("anyrender", "0.14.0"), ("anyrender_vello_cpu", "0.18.0")]) {
        let at = lock.find(&format!("[[package]]\nname = \"{library}\"\n")).unwrap_or_else(|| panic!("{library} isn't locked"));
        let version = lock[at..].lines().nth(2).unwrap();
        assert_eq!(version, format!("version = \"{expected}\""), "{library} moved: measure the labels' limits again");
    }
}
