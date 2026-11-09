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
/// conditions (`requiredFeatures` met); those a `<use>` repeats are counted too, as left out. A
/// build that draws labels draws all but those (`pdf,svg` builds the other).
#[test]
fn the_labels_a_browser_draws_are_told_by_one_rule() {
    let fo = label("", "");
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
        (format!("<defs><symbol id='l'>{fo}</symbol></defs><use href='#l'/>"), 1),
        (format!("<defs><g id='l' display='none'>{fo}</g></defs><use href='#l'/>"), 0),
        (format!("<defs><g id='l'>{fo}</g></defs><use href='#l' style='display:none'/>"), 0),
        (format!("<defs><switch id='l'><rect width='1' height='1'/>{fo}</switch></defs><use href='#l'/>"), 0),
        (format!("<defs><g id='l'>{fo}</g><g id='m'><use href='#l'/></g></defs><use href='#m'/>"), 1),
        (format!("<defs><g id='l'>{fo}<use href='#l'/></g></defs><use href='#l'/>"), 1),
        (format!("<g display='none'><use href='#l'/></g><defs><g id='l'>{fo}</g></defs>"), 0),
        (label("", &format!("<svg xmlns='http://www.w3.org/2000/svg'>{fo}</svg>")), 1),
    ];
    for (body, expected) in cases {
        let expected = if cfg!(feature = "svg-html") && !body.contains("<use") { 0 } else { expected };
        assert_eq!(undrawn("svg-labels", &body), expected, "{body}");
    }
}

/// A `<switch>`'s fallback after the label a browser takes (draw.io's "[Not supported by
/// viewer]"), or after a child asking for `requiredFeatures` usvg lacks (newer draw.io's "Text is
/// not SVG"), isn't drawn where labels are, and is where they aren't, as it was.
#[test]
fn a_fallback_is_drawn_only_where_labels_are_not() {
    let red = "<rect width='40' height='20' fill='red'/>";
    for first in [label("", ""), "<g requiredFeatures='http://www.w3.org/TR/SVG11/feature#Extensibility'/>".to_string()] {
        let path = written("svg-fallback", "fallback.svg", svg(40, 20, &format!("<switch>{first}{red}</switch>")));
        let out = view(&path, &["--protocol", "kitty"]);
        assert!(out.status.success(), "{}", text(&out.stderr));
        let picture = image::load_from_memory(&images(&out.stdout)[0].png).unwrap().into_rgba8();
        let expected = if cfg!(feature = "svg-html") { [0, 0, 0, 0] } else { [255, 0, 0, 255] };
        assert_eq!(picture.get_pixel(10, 10).0, expected, "{first}");
    }
}

/// A `<switch>`'s child taken asking for `requiredFeatures` is drawn, as a browser draws it, where
/// labels are; where they aren't, usvg takes the fallback after it, as it did.
#[test]
fn a_switchs_child_taken_is_drawn_whatever_features_it_asks_for() {
    let taken = "<g requiredFeatures='http://www.w3.org/TR/SVG11/feature#Extensibility'><rect width='40' height='20' fill='blue'/></g>";
    let path = written("svg-features", "features.svg", svg(40, 20, &format!("<switch>{taken}<rect width='40' height='20' fill='red'/></switch>")));
    let out = view(&path, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let picture = image::load_from_memory(&images(&out.stdout)[0].png).unwrap().into_rgba8();
    let expected = if cfg!(feature = "svg-html") { [0, 0, 255, 255] } else { [255, 0, 0, 255] };
    assert_eq!(picture.get_pixel(10, 10).0, expected);
}

/// A fallback an entity holds is left in the drawing, as is the entity: cutting it would cut the
/// entity's declaration, and every element it makes elsewhere.
#[test]
fn a_fallback_from_an_entity_is_left_with_what_else_the_entity_makes() {
    let entity = "<!DOCTYPE svg [<!ENTITY box \"<rect x='20' width='20' height='20' fill='blue'/>\">]>";
    let body = format!("<switch>{}&box;</switch><g transform='translate(-20 0)'>&box;</g>", label("", ""));
    let path = written("svg-entity-fallback", "entity.svg", format!("{entity}{}", svg(40, 20, &body)));
    let out = view(&path, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let picture = image::load_from_memory(&images(&out.stdout)[0].png).unwrap().into_rgba8();
    // The rect the entity makes outside the switch, moved to the left half, is drawn.
    assert_eq!(picture.get_pixel(10, 10).0, [0, 0, 255, 255]);
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
    // Quotes are six bytes each escaped: 1.5 MiB of them is 9 MiB.
    said(&label("", &format!("<i title='{}'/>", "\"".repeat(3 << 19))), "past 8 MiB of markup", 1);
}

/// What `clv` drew of `body`, a drawing `width` × `height` (a viewBox as large), with `args`: its
/// page, and its warnings.
#[cfg(feature = "svg-html")]
fn page(test: &str, width: u32, height: u32, body: &str, args: &[&str]) -> (image::RgbaImage, Vec<String>) {
    let drawing = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{body}</svg>"#);
    let path = written(test, "page.svg", drawing);
    let out = view(&path, &[&["--protocol", "kitty"], args].concat());
    assert!(out.status.success(), "{body}: {}", text(&out.stderr));
    let warnings = text(&out.stderr)
        .lines()
        .flat_map(|l| l.strip_prefix("warning: page 1: ").unwrap_or(l).split("; ").map(str::to_string).collect::<Vec<_>>())
        .collect();
    (image::load_from_memory(&images(&out.stdout)[0].png).unwrap().into_rgba8(), warnings)
}

/// A `<foreignObject>` with `attributes`, its HTML `html` (an XHTML element).
#[cfg(feature = "svg-html")]
fn fo(attributes: &str, html: &str) -> String {
    format!("<foreignObject {attributes}>{}</foreignObject>", html.replacen("<div", "<div xmlns='http://www.w3.org/1999/xhtml'", 1))
}

/// A box filling its containing block, of `colour`.
#[cfg(feature = "svg-html")]
fn filled(colour: &str) -> String {
    format!("<div style='width:100%;height:100%;background:{colour}'/>")
}

#[cfg(feature = "svg-html")]
const BLUE: [u8; 4] = [0, 0, 255, 255];
#[cfg(feature = "svg-html")]
const RED: [u8; 4] = [255, 0, 0, 255];
#[cfg(feature = "svg-html")]
const GREEN: [u8; 4] = [0, 255, 0, 255];
#[cfg(feature = "svg-html")]
const NONE: [u8; 4] = [0, 0, 0, 0];

/// The colours of `picture` at `points`.
#[cfg(feature = "svg-html")]
fn at(picture: &image::RgbaImage, points: &[(u32, u32)]) -> Vec<[u8; 4]> {
    points.iter().map(|&(x, y)| picture.get_pixel(x, y).0).collect()
}

/// A label is placed where the drawing puts it, through its nested `<svg>`s' viewports and their
/// `preserveAspectRatio`, its length a percentage of the nearest viewport. The tests' window shows
/// a 40 × 20 drawing at 20 pixels a unit.
#[cfg(feature = "svg-html")]
#[test]
fn labels_are_placed_through_the_viewports_they_are_in() {
    // A nested svg on the right half, 10 units shown in 20: the label's 5 × 5 is 10 × 10.
    let nested = format!("<svg x='20' width='20' height='20' viewBox='0 0 10 10'>{}</svg>", fo("width='5' height='5'", &filled("blue")));
    let (picture, _) = page("svg-placed", 40, 20, &nested, &[]);
    assert_eq!(at(&picture, &[(500, 100), (300, 100), (700, 100), (500, 300)]), [BLUE, NONE, NONE, NONE]);
    // 10 × 20 units in 20 × 20, centred: 5 units in from the left, at one unit a unit.
    let centred = format!("<svg x='20' width='20' height='20' viewBox='0 0 10 20'>{}</svg>", fo("width='10' height='10'", &filled("blue")));
    let (picture, _) = page("svg-placed", 40, 20, &centred, &[]);
    assert_eq!(at(&picture, &[(490, 100), (510, 100), (690, 100), (710, 100)]), [NONE, BLUE, BLUE, NONE]);
    let percent = fo("x='50%' width='25%' height='50%'", &filled("red"));
    let (picture, _) = page("svg-placed", 40, 20, &percent, &[]);
    assert_eq!(at(&picture, &[(390, 100), (410, 100), (590, 100), (610, 100), (500, 210)]), [NONE, RED, RED, NONE, NONE]);
}

/// What a label draws past its box is drawn where its overflow is visible, and cut where it
/// isn't; and children sized in percentages are of their own label's box, whatever its size.
#[cfg(feature = "svg-html")]
#[test]
fn labels_draw_past_their_box_only_where_their_overflow_is_visible() {
    let past =
        "<div style='position:relative;width:5px;height:5px'><div style='position:absolute;left:15px;width:5px;height:5px;background:#0f0'/></div>";
    let (visible, _) = page("svg-overflow", 40, 20, &fo("width='5' height='5' style='overflow: visible'", past), &[]);
    let (hidden, _) = page("svg-overflow", 40, 20, &fo("width='5' height='5'", past), &[]);
    assert_eq!((visible.get_pixel(350, 50).0, hidden.get_pixel(350, 50).0), (GREEN, NONE));
    let halves = format!(
        "{}{}",
        fo("width='10' height='10'", "<div style='width:50%;height:100%;background:red'/>"),
        fo("x='20' width='20' height='20'", "<div style='width:50%;height:100%;background:blue'/>")
    );
    let (picture, _) = page("svg-overflow", 40, 20, &halves, &[]);
    assert_eq!(at(&picture, &[(90, 100), (110, 100), (90, 210), (590, 300), (610, 300)]), [RED, NONE, NONE, BLUE, NONE]);
}

/// A label is drawn over the whole drawing, at its ancestors' opacity: over a shape after it in the
/// drawing too, where a browser would draw the shape over it (a known difference).
#[cfg(feature = "svg-html")]
#[test]
fn labels_are_drawn_over_the_drawing_at_their_opacity() {
    let body = format!("<g opacity='0.5'><g style='opacity: 50%'>{}</g></g>", fo("width='20' height='20'", &filled("blue")));
    let (picture, _) = page("svg-opacity", 40, 20, &body, &[]);
    let [r, g, b, a] = picture.get_pixel(100, 100).0;
    assert!((r, g, b) == (0, 0, 255) && (62..=66).contains(&a), "{:?}", [r, g, b, a]);
    // Its own opacity by `style` its copy keeps, and is applied once.
    let body = fo("width='20' height='20' style='opacity: 0.5'", &filled("blue"));
    let (picture, _) = page("svg-opacity", 40, 20, &body, &[]);
    let [r, g, b, a] = picture.get_pixel(100, 100).0;
    assert!((r, g, b) == (0, 0, 255) && (126..=130).contains(&a), "{:?}", [r, g, b, a]);
    let body = format!("{}<rect width='20' height='20' fill='red'/>", fo("width='20' height='20'", &filled("blue")));
    let (picture, _) = page("svg-opacity", 40, 20, &body, &[]);
    assert_eq!(picture.get_pixel(100, 100).0, BLUE);
}

/// A label takes the drawing's stylesheets inside copies of its context: a rule on its
/// `<foreignObject>`'s class reaches it, and an ancestor's `color` is inherited.
#[cfg(feature = "svg-html")]
#[test]
fn labels_take_the_drawings_rules_in_their_context() {
    let body =
        format!("<style>.tag &gt; div {{ background: #00f }}</style>{}", fo("class='tag' width='20' height='20'", "<div style='height:100%'/>"));
    let (picture, _) = page("svg-rules", 40, 20, &body, &[]);
    assert_eq!(picture.get_pixel(100, 100).0, BLUE);
    let body = format!("<g color='#f00'>{}</g>", fo("width='20' height='20'", &filled("currentColor")));
    let (picture, _) = page("svg-rules", 40, 20, &body, &[]);
    assert_eq!(picture.get_pixel(100, 100).0, RED);
}

/// A picture a label embeds as a `data:` URL is drawn.
#[cfg(feature = "svg-html")]
#[test]
fn a_picture_a_label_embeds_is_drawn() {
    use base64::Engine as _;
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(2, 2, image::Rgba(BLUE)).write_to(&mut png, image::ImageFormat::Png).unwrap();
    let src = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    let body =
        fo("width='20' height='20'", &format!("<div><img src='data:image/png;base64,{src}' style='display:block;width:20px;height:20px'/></div>"));
    let (picture, _) = page("svg-picture", 40, 20, &body, &[]);
    assert_eq!(picture.get_pixel(100, 100).0, BLUE);
}

/// `<font>`'s `color` and `size` are hints, as a browser takes them: under any rule of the
/// drawing's, and left out when they don't parse. Its `face`, the same way, by `font_hints`.
#[cfg(feature = "svg-html")]
#[test]
fn a_fonts_hints_are_under_the_drawings_rules() {
    let coloured = |hint: &str, rule: &str| {
        let html = format!(
            "<div style='color:red'><font color='{hint}'><span style='display:block;width:20px;height:20px;background:currentColor'/></font></div>"
        );
        let (picture, _) = page("svg-hints", 40, 20, &format!("<style>{rule}</style>{}", fo("width='20' height='20'", &html)), &[]);
        picture.get_pixel(100, 100).0
    };
    assert_eq!([coloured("#0f0", ""), coloured("#0f0", "font { color: #00f }"), coloured("not a colour", "")], [GREEN, BLUE, RED]);
    // A box 1em a side, in a 100-unit drawing shown at 4.6 pixels a unit: 7 is 48 pixels.
    let sized = |hint: &str, rule: &str| {
        let html = format!(
            "<div style='font-size:16px'><font size='{hint}'><span style='display:block;width:1em;height:1em;background:blue'/></font></div>"
        );
        let (picture, _) = page("svg-hints", 100, 100, &format!("<style>{rule}</style>{}", fo("width='100' height='100'", &html)), &[]);
        [(60, 10), (200, 10)].map(|(x, y)| picture.get_pixel(x, y).0 == BLUE)
    };
    assert_eq!(sized("7", ""), [true, true]);
    assert_eq!(sized("+4", ""), [true, true]);
    assert_eq!(sized("7", "font { font-size: 10px }"), [false, false]);
    assert_eq!(sized("big", ""), [true, false]);
}

/// What painting a drawing's labels would take is estimated after they are laid out, before they
/// are painted, where CSS multiplies it: shadows, layers, and a picture's tiles spaced. Past 20
/// million steps, they are all left out, and said; at it, they are painted (and here left out
/// past the 200,000 commands recorded, which the estimate counts high to reach).
#[cfg(feature = "svg-html")]
#[test]
fn painting_too_long_is_refused_before_it_starts() {
    let too_long = "painting them would take too long";
    let shadows = |n: usize| {
        let list: Vec<String> = (0..n).map(|i| format!("{}px 0 1px #000", i % 5)).collect();
        let css = format!(".c{{display:inline-block;width:2px;height:2px;box-shadow:{}}}", list.join(","));
        let boxes = "<i class='c'></i>".repeat(1000);
        warned("svg-paint", &format!("<style>{css}</style>{}", fo("width='200' height='200'", &format!("<div>{boxes}</div>"))))
    };
    // Six steps a shadow: 1,000 boxes of 3,333 shadows are 19,998,000.
    assert!(!shadows(3333).iter().any(|w| w.contains(too_long)), "{:?}", shadows(3333));
    assert!(shadows(3334).iter().any(|w| w.contains(too_long)), "{:?}", shadows(3334));
    let layers = |n: usize| {
        let list: Vec<String> = (0..n).map(|i| format!("linear-gradient(#{:03x},#fff)", i % 4096)).collect();
        warned("svg-paint", &fo("width='20' height='20'", &format!("<div style='width:1px;height:1px;background-image:{}'/>", list.join(","))))
    };
    // 504 steps a gradient layer: 39,682 are 19,999,728.
    assert!(!layers(39_682).iter().any(|w| w.contains(too_long)));
    assert!(layers(39_683).iter().any(|w| w.contains(too_long)));
    let spaced = |side: u32| {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
        let css = format!(".s{{width:{side}px;height:{side}px;background:url(data:image/png;base64,{png}) 0 0/1px 1px space}}");
        warned("svg-paint", &format!("<style>{css}</style>{}", fo(&format!("width='{side}' height='{side}'"), "<div class='s'/>")))
    };
    // The document's own html and body are painted with each label: 4,000 shadows on either, six
    // steps each, are 24,000 steps a label, 24 million for 1,000 labels. (usvg's stylesheet parser
    // takes time as the square of a declaration's length: a longer list is a slow test.)
    let shared = |rule: &str, labels: usize| {
        let list: Vec<String> = (0..4_000).map(|i| format!("{}px 0 1px #000", i % 5)).collect();
        let css = format!("{rule}{{content:'';display:block;box-shadow:{}}}", list.join(","));
        warned("svg-paint", &format!("<style>{css}</style>{}", fo("width='1' height='1'", "<div/>").repeat(labels)))
    };
    assert!(!shared("html", 1).iter().any(|w| w.contains(too_long)));
    assert!(shared("html", 1000).iter().any(|w| w.contains(too_long)));
    assert!(shared("body::before", 1000).iter().any(|w| w.contains(too_long)));
    // A dot of a dotted border is 20 steps: 12 boxes of 10,000 pixels a side are 960,000 dots.
    let dotted = |boxes: usize| {
        let html =
            "<i style='position:absolute;left:0;top:0;width:10000px;height:10000px;border:1px dotted red;box-sizing:border-box'></i>".repeat(boxes);
        warned("svg-paint", &fo("width='10000' height='10000'", &format!("<div style='position:relative'>{html}</div>")))
    };
    assert!(!dotted(12).iter().any(|w| w.contains(too_long)), "{:?}", dotted(12));
    assert!(dotted(13).iter().any(|w| w.contains(too_long)));
    // A tile a pixel, and four steps around the layer: 4,472 pixels a side are 19,998,788.
    assert!(!spaced(4472).iter().any(|w| w.contains(too_long)));
    assert!(spaced(4473).iter().any(|w| w.contains(too_long)));
}

/// A label loads no stylesheet and no document: a `<link>`'s is not fetched, whatever its bytes
/// look like, nor an `@import`, a label's `<style>`'s or the drawing's. That `<style>`'s own
/// rules are taken, as the drawing's.
#[cfg(feature = "svg-html")]
#[test]
fn a_label_loads_no_stylesheet_or_document() {
    // Its first bytes are a font's: a font is fetched.
    let sheet = "data:text/css,OTTO%7B%7Ddiv%7Bbackground:blue%7D";
    let block = "<div style='width:20px;height:20px'/>";
    let linked = fo("width='20' height='20'", &format!("<div><link rel='stylesheet' href='{sheet}'/>{block}</div>"));
    assert_eq!(page("svg-loads", 40, 20, &linked, &[]).0.get_pixel(100, 100).0, NONE);
    let imported = fo("width='20' height='20'", &format!("<div><style>@import url({sheet});</style>{block}</div>"));
    assert_eq!(page("svg-loads", 40, 20, &imported, &[]).0.get_pixel(100, 100).0, NONE);
    let drawings = format!("<style>@import url({sheet});</style>{}", fo("width='20' height='20'", &format!("<div>{block}</div>")));
    assert_eq!(page("svg-loads", 40, 20, &drawings, &[]).0.get_pixel(100, 100).0, NONE);
    let own = fo("width='20' height='20'", &format!("<div><style>div div {{ background: red }}</style>{block}</div>"));
    assert_eq!(page("svg-loads", 40, 20, &own, &[]).0.get_pixel(100, 100).0, RED);
}

/// Each stage of what labels cost has its limit, past which they are left out, and said: their
/// commands recorded, a label's size laid out, and their elements matched against the drawing's
/// rules.
#[cfg(feature = "svg-html")]
#[test]
fn labels_past_what_they_may_cost_are_left_out_and_said() {
    let one = |n: usize, why: &str| {
        if n == 1 {
            format!("1 HTML label (<foreignObject>) was left out, {why}")
        } else {
            format!("{n} HTML labels (<foreignObject>) were left out, {why}")
        }
    };
    // 51,000 boxes, each its background, its border and two shadows: 204,000 commands.
    let css = ".c{display:inline-block;width:2px;height:2px;background:#00f;border:1px solid #000;box-shadow:1px 0 #f00,0 1px #0f0}";
    let boxes = "<i class='c'></i>".repeat(51_000);
    let commands = warned(
        "svg-cost",
        &format!("<style>{css}</style>{}", fo("width='1000' height='1000'", &format!("<div style='display:flex;flex-wrap:wrap'>{boxes}</div>"))),
    );
    assert!(commands.contains(&one(1, "painting them would take more than 200000 steps")), "{commands:?}");
    let wide = format!("{}{}", fo("width='16385' height='10'", "<div/>"), fo("width='16384' height='10'", "<div/>"));
    let past = "laid out past 16384 pixels a side, or an element of it past 24576";
    assert!(warned("svg-cost", &wide).contains(&one(1, past)));
    // An element of a label as wide as a label may draw, a dotted border round it: its dots are
    // made at its size. Past it, its label is left out; within it, drawn.
    let dotted = |w: u32| fo("width='400' height='100'", &format!("<div style='width:{w}px;height:20px;border:1px dotted red'/>"));
    // Its dots are past the paint work's limit too, which leaves it out first.
    assert!(warned("svg-cost", &dotted(1_000_000)).iter().any(|w| w.starts_with("1 HTML label (<foreignObject>) was left out")));
    assert!(!warned("svg-cost", &dotted(20_000)).iter().any(|w| w.contains("left out")));
    // Past it without any dots, its label is left out all the same.
    let wide_box = fo("width='400' height='100'", "<div style='width:30000px;height:20px;background:red'/>");
    assert!(warned("svg-cost", &wide_box).contains(&one(1, past)));
    // 1,200 rules, and 50 labels in 900 groups: each label's copies are 900 elements.
    let rules: String = (0..1200).map(|i| format!(".r{i}{{color:red}}")).collect();
    let labels = fo("width='1' height='1'", "<div/>").repeat(50);
    let nested = format!("<style>{rules}</style>{}{labels}{}", "<g>".repeat(900), "</g>".repeat(900));
    assert!(warned("svg-cost", &nested).contains(&one(50, "its stylesheets' rules are too many for its labels' elements")));
}

/// What a page's labels paint is in the page's estimate, with the drawing's: 13,000 boxes over a
/// label as large as the page are 4,160 million pixels, and the page fails with why, as a drawing
/// does. A label reads nothing but `data:` URLs: what else it asks for is told, as the drawing's
/// own links are.
#[cfg(feature = "svg-html")]
#[test]
fn what_a_pages_labels_paint_is_in_its_estimate_and_their_links_are_said() {
    let stacked = "<i style='position:absolute;left:0;top:0;width:100%;height:100%;background:rgba(0,0,255,0.01)'></i>".repeat(13_000);
    let drawing = svg(40, 20, &fo("width='40' height='20'", &format!("<div style='position:relative;height:100%'>{stacked}</div>")));
    let out = view(&written("svg-estimate", "stacked.svg", drawing), &["--protocol", "kitty"]);
    assert!(!out.status.success() && text(&out.stderr).contains("would take too long"), "{}", text(&out.stderr));
    let linking = fo("width='20' height='20'", "<div><img src='https://example.com/a.png'/><img src='other.png'/></div>");
    assert_eq!(warned("svg-estimate", &linking), ["2 links to other files or the network were not followed"]);
}

/// A label nested as deeply as labels may be, in nested grids (Blitz's costliest nesting), is laid
/// out and drawn on its thread's stack, in a debug build too (ADR-0024).
#[cfg(feature = "svg-html")]
#[test]
fn the_deepest_label_is_drawn_on_its_threads_stack() {
    let grids = format!("{}<div style='width:20px;height:20px;background:blue'/>{}", "<div style='display:grid'>".repeat(62), "</div>".repeat(62));
    let (picture, warnings) = page("svg-deepest", 40, 20, &fo("width='20' height='20'", &format!("<div>{grids}</div>")), &[]);
    assert!(warnings.iter().all(|w| !w.contains("left out")), "{warnings:?}");
    assert_eq!(picture.get_pixel(100, 100).0, BLUE);
}

/// A stylesheet's font faces and pictures by `data:` URL are no links not followed where labels
/// are drawn, which fetch them: only its others are. Where they aren't, usvg follows none of them,
/// and says so.
#[test]
fn a_stylesheets_data_urls_are_links_only_where_labels_are_not_drawn() {
    let css = "@font-face{font-family:X;src:url(data:font/woff2;base64,AAAA)} .l{background:url(data:image/png;base64,AAAA)} \
               .m{background:url(https://example.com/a.png)}";
    let body = format!("<style>{css}</style>{}", label("class='l m'", ""));
    let expected = if cfg!(feature = "svg-html") { 1 } else { 4 };
    let warnings = warned("svg-data-links", &body);
    let links = warnings.iter().find(|w| w.contains("not followed"));
    assert_eq!(links.map(|w| w.split(' ').next().unwrap().parse::<usize>().unwrap()), Some(expected), "{warnings:?}");
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
const HTML_LIBRARIES: [&str; 8] = ["blitz", "anyrender", "kurbo", "stylo", "parley", "taffy", "wuff", "peniko"];

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
    for (library, expected) in
        blitz.into_iter().chain([("anyrender", "0.14.0"), ("anyrender_vello_cpu", "0.18.0"), ("stylo", "0.22.0"), ("peniko", "0.6.1")])
    {
        let at = lock.find(&format!("[[package]]\nname = \"{library}\"\n")).unwrap_or_else(|| panic!("{library} isn't locked"));
        let version = lock[at..].lines().nth(2).unwrap();
        assert_eq!(version, format!("version = \"{expected}\""), "{library} moved: measure the labels' limits again");
    }
}
