//! The Word backend, `src/source/wordcraft.rs`: Word documents, OpenDocument text, RTF, Markdown
//! and HTML, as pages; the pictures the last two refer to, drawn by the backends that show them; a
//! protected Word document opened with its password; the rule that nothing else names wordcraft or
//! pulldown-cmark; and the pin. Its own tests read the first bytes it goes by.
#![cfg(all(unix, feature = "word"))]

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use ::zip::write::SimpleFileOptions;
use ::zip::{CompressionMethod, ZipWriter};
use base64::Engine as _;
use serde_json::Value;

use crate::view::{images, names, png_size, text, tmp, view};

/// A zip of `files`, by name and contents, in order: each deflated, but a file named `mimetype`
/// stored, as OpenDocument has it.
fn zip(files: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in files {
        let method = if *name == "mimetype" { CompressionMethod::Stored } else { CompressionMethod::Deflated };
        zip.start_file(*name, SimpleFileOptions::default().compression_method(method)).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// A docx of `pages` pages, 200 × 300 pt each, the n-th reading "Page n".
fn docx(pages: usize) -> Vec<u8> {
    let body: String = (1..=pages)
        .map(|n| {
            let page = format!("<w:p><w:r><w:t>Page {n}</w:t></w:r></w:p>");
            if n < pages { page + r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"# } else { page }
        })
        .collect();
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}<w:sectPr><w:pgSz w:w="4000" w:h="6000"/><w:pgMar w:top="720" w:right="720" w:bottom="720" w:left="720"/></w:sectPr></w:body></w:document>"#
    );
    zip(&[
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        ("word/document.xml", &document),
    ])
}

/// An OpenDocument text of one A5 page (420 × 595 pt), reading "Open".
fn odt() -> Vec<u8> {
    const OFFICE: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.3""#;
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-styles {OFFICE}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="148mm" fo:page-height="210mm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {OFFICE}><office:body><office:text><text:p>Open</text:p></office:text></office:body></office:document-content>"#
    );
    zip(&[
        ("mimetype", "application/vnd.oasis.opendocument.text"),
        (
            "META-INF/manifest.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/></manifest:manifest>"#,
        ),
        ("styles.xml", &styles),
        ("content.xml", &content),
    ])
}

/// An RTF of three US Letter pages (612 × 792 pt).
const RTF: &str = r"{\rtf1\ansi\deff0{\fonttbl{\f0 Times New Roman;}}\paperw12240\paperh15840 \f0\fs24 One\page Two\page Three\par}";

fn write(path: PathBuf, bytes: &[u8]) -> PathBuf {
    std::fs::write(&path, bytes).unwrap();
    path
}

/// The size of each page `clv` drew of `path`, given `args`.
fn drawn(path: &Path, args: &[&str]) -> Vec<(u32, u32)> {
    let out = view(path, &[&["--protocol", "kitty"], args].concat());
    assert!(out.status.success() && out.stderr.is_empty(), "{}: {}", path.display(), text(&out.stderr));
    images(&out.stdout).iter().map(|d| png_size(&d.png)).collect()
}

/// Whether every one of `sizes` is `w` × `h` in shape.
fn shaped(sizes: &[(u32, u32)], w: f32, h: f32) -> bool {
    sizes.iter().all(|(x, y)| (*x as f32 / *y as f32 - w / h).abs() < 0.01)
}

/// A docx, an odt and an RTF are pages, each its own size, as a PDF's are.
#[test]
fn a_word_document_odt_and_rtf_are_their_pages() {
    let pages = drawn(&write(tmp("word-pages", "three.docx"), &docx(3)), &[]);
    assert!(pages.len() == 3 && shaped(&pages, 200.0, 300.0), "{pages:?}");
    let pages = drawn(&write(tmp("word-pages", "a5.odt"), &odt()), &[]);
    assert!(pages.len() == 1 && shaped(&pages, 420.0, 595.0), "{pages:?}");
    let pages = drawn(&write(tmp("word-pages", "letter.rtf"), RTF.as_bytes()), &[]);
    assert!(pages.len() == 3 && shaped(&pages, 612.0, 792.0), "{pages:?}");
}

/// A Word package is known by what it holds, whatever its name: `word/` among its first files
/// (an RTF by its first bytes, as the backend's own tests check); named as a Word document, one
/// that isn't is the error, rather than a folder.
#[test]
fn a_document_is_known_by_what_it_holds() {
    assert_eq!(drawn(&write(tmp("word-known", "renamed.zip"), &docx(2)), &[]).len(), 2);
    let broken = write(tmp("word-known", "broken.docx"), &zip(&[("notes.txt", "not a document")]));
    let out = view(&broken, &["--protocol", "kitty"]);
    assert!(!out.status.success() && text(&out.stderr).starts_with(&format!("clv: {}: ", broken.display())), "{}", text(&out.stderr));
}

/// In a directory, or an archive, a document is a tile: its first page.
#[test]
fn in_a_list_a_document_is_its_first_page() {
    let dir = tmp("word-list", "shelf");
    std::fs::create_dir_all(&dir).unwrap();
    write(dir.join("a.docx"), &docx(3));
    write(dir.join("b.rtf"), RTF.as_bytes());
    let out = view(&dir, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(names(&out.stdout), [" a.docx b.rtf"]);
    let tiles: Vec<_> = images(&out.stdout).iter().map(|d| png_size(&d.png)).collect();
    assert!(shaped(&tiles[..1], 200.0, 300.0) && shaped(&tiles[1..], 612.0, 792.0), "{tiles:?}");
}

/// The first page `clv` draws of `document`, named `name`, as a PNG.
fn page(test: &str, name: &str, document: &str) -> Vec<u8> {
    let out = view(&write(tmp(test, name), document.as_bytes()), &["--protocol", "kitty"]);
    assert!(out.status.success(), "{name}: {}", text(&out.stderr));
    images(&out.stdout).swap_remove(0).png
}

/// A docx of one narrow page of long words, that asks for automatic hyphenation or doesn't.
fn long_words(hyphenated: bool) -> Vec<u8> {
    let document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:rPr><w:lang w:val="en-US"/></w:rPr><w:t>Internationalization considerations notwithstanding, extraordinarily comprehensive documentation characteristically demonstrates unquestionable professionalism</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="4000" w:h="6000"/><w:pgMar w:top="720" w:right="720" w:bottom="720" w:left="720"/></w:sectPr></w:body></w:document>"#;
    let settings = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{}</w:settings>"#,
        if hyphenated { "<w:autoHyphenation/>" } else { "" }
    );
    zip(&[
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        (
            "word/_rels/document.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/></Relationships>"#,
        ),
        ("word/document.xml", document),
        ("word/settings.xml", &settings),
    ])
}

/// Past the rev it is pinned at, wordcraft's dictionaries need a checkout at build time, and a build
/// without one hyphenates nothing while everything else looks right
/// ([ADR-0018](../../docs/adr/0018-wordcraft-stays-before-its-proofing-needs-a-checkout-to-build.md)).
#[test]
fn a_word_document_that_asks_for_hyphenation_gets_it() {
    let first_page = |name: &str, hyphenated: bool| {
        let out = view(&write(tmp("word-hyphens", name), &long_words(hyphenated)), &["--protocol", "kitty"]);
        assert!(out.status.success(), "{name}: {}", text(&out.stderr));
        images(&out.stdout).swap_remove(0).png
    };
    assert!(first_page("asks.docx", true) != first_page("doesnt.docx", false), "the pages are the same, hyphenated or not");
}

/// A character of the code face, in points: a column of the page, a cell of the room at 100%.
const COLUMN: f32 = 5.64;

/// The page `clv` lays a Markdown or HTML file out on when these tests print it, in points.
struct Page {
    w: f32,
    h: f32,
    /// Its text's width and height, within the margins: three columns at each side, a row above and below.
    text_w: f32,
    text_h: f32,
}

/// The page `clv` lays a Markdown or HTML file out on when these tests print it: the room the tty says
/// (the controlling terminal of the tests, if they have one: its columns, its rows less the one left
/// free, and its cell from the pixels it reports, or the 10 × 20 assumed), or with no terminal, as
/// under CI, the preview's 60 × 40 cells. As wide as the room, or a book's page where the room holds two
/// of 60 characters, or `width` characters between the margins, never wider than the room; as tall as
/// the room. The rules the backend's own tests check on rooms of their choosing, here at whatever room
/// the tests run in.
fn printed_page(width: Option<f32>) -> Page {
    let (cols, rows, shape) = match tty_window() {
        Some((cols, rows, shape)) => (cols, rows - 1, shape),
        None => (60, 40, 2.0),
    };
    let room = f32::from(cols) * COLUMN - 6.0 * COLUMN;
    let half = f32::from((cols - 2) / 2) * COLUMN - 6.0 * COLUMN;
    let text_w = match width {
        Some(columns) => (columns * COLUMN).min(room),
        None if half >= 60.0 * COLUMN => half,
        None => room,
    };
    let (row, text_h) = (shape * COLUMN, f32::from(rows) * shape * COLUMN - 2.0 * shape * COLUMN);
    Page { w: text_w + 6.0 * COLUMN, h: text_h + 2.0 * row, text_w, text_h }
}

/// The window the tests' controlling terminal reports, if they have one: its columns and rows, and the
/// shape of its cell (its height over its width, from the pixels it reports, or the 2 of the 10 × 20
/// cell `clv` assumes without them), as `clv` takes them.
fn tty_window() -> Option<(u16, u16, f32)> {
    let tty = std::fs::File::open("/dev/tty").ok().and_then(|tty| rustix::termios::tcgetwinsize(&tty).ok());
    let w = tty.filter(|w| w.ws_col > 0 && w.ws_row > 1)?;
    let pixels =
        (w.ws_xpixel > 0 && w.ws_ypixel > 0).then(|| f32::from(w.ws_ypixel) / f32::from(w.ws_row) / (f32::from(w.ws_xpixel) / f32::from(w.ws_col)));
    Some((w.ws_col, w.ws_row, pixels.unwrap_or(2.0)))
}

/// The page a list's documents are previewed on, in points: 60 × 40 cells at the cell `clv` started
/// with, the terminal's or the 10 × 20 assumed ([`tty_window`]).
fn preview_page() -> (f32, f32) {
    let shape = tty_window().map_or(2.0, |(_, _, shape)| shape);
    (60.0 * COLUMN, 40.0 * shape * COLUMN)
}

/// Neither gives a page size, so both are laid out on the window's: a page as wide and as tall as the
/// room the tty says, or the preview's with no terminal ([`printed_page`]).
#[test]
fn markdown_and_html_are_laid_out_into_pages() {
    let page = printed_page(None);
    let long = |open: &str, close: &str| -> String { (1..=200).map(|n| format!("{open}Paragraph {n}{close}\n\n")).collect() };
    for (name, document) in [("long.md", long("", "")), ("long.html", long("<p>", "</p>"))] {
        let pages = drawn(&write(tmp("word-text", name), document.as_bytes()), &[]);
        assert!(pages.len() > 1 && shaped(&pages, page.w, page.h), "{name}: {pages:?}, not {} × {}", page.w, page.h);
    }
}

/// What GitHub's Markdown has that a printed page needs written otherwise. Raw HTML is read, not
/// shown as its tags; a footnote isn't shown as how it is written; front matter says nothing on the
/// page; a task says if it is done; a table has its grid.
#[test]
fn markdown_is_read_as_github_writes_it() {
    let of = |document: &str| page("word-github", "a.md", document);
    assert_eq!(of("<p>One</p>"), of("One"), "raw HTML");
    assert_eq!(of("---\ntitle: Two\n---\n\nOne"), of("One"), "front matter");
    assert_ne!(of("One[^1]\n\n[^1]: Two"), of("One[^1]\n\n[^1]: Three"), "a footnote, which shows its note");
    let (done, to_do, neither) = (of("- [x] One"), of("- [ ] One"), of("- One"));
    assert!(done != to_do && done != neither && to_do != neither, "a task");
    let html = page("word-github", "a.html", "<table border=\"1\"><thead><tr><th>One</th></tr></thead><tbody><tr><td>Two</td></tr></tbody></table>");
    assert_eq!(of("| One |\n|---|\n| Two |"), html, "a table and its grid");
}

/// Read as what it is and not as text: a heading written two ways is one heading, which isn't the
/// same words as a paragraph.
#[test]
fn markdown_and_html_are_read_as_such() {
    let heading = page("word-marked", "a.md", "# One");
    assert_eq!(heading, page("word-marked", "a.md", "One\n==="));
    assert_ne!(heading, page("word-marked", "a.md", "One"));
    let heading = page("word-marked", "a.html", "<h1>One</h1>");
    assert_eq!(heading, page("word-marked", "a.html", "<H1 >One</H1 >"));
    assert_ne!(heading, page("word-marked", "a.html", "<p>One</p>"));
}

/// A PNG of `width` × `height`, of `colour`.
fn picture(width: u32, height: u32, colour: [u8; 3]) -> Vec<u8> {
    let [r, g, b] = colour;
    let mut png = Cursor::new(Vec::new());
    ::image::RgbaImage::from_pixel(width, height, ::image::Rgba([r, g, b, 255])).write_to(&mut png, ::image::ImageFormat::Png).unwrap();
    png.into_inner()
}

const RED: [u8; 3] = [220, 20, 20];

/// `page`'s size, and the width and height of what on it is about `colour`, if any is.
fn found(page: &[u8], colour: [u8; 3]) -> ((u32, u32), Option<(u32, u32)>) {
    let page = ::image::load_from_memory(page).unwrap().to_rgb8();
    let (mut least, mut most) = ((u32::MAX, u32::MAX), None::<(u32, u32)>);
    for (x, y, pixel) in page.enumerate_pixels() {
        if pixel.0.iter().zip(colour).all(|(a, b)| a.abs_diff(b) < 12) {
            least = (least.0.min(x), least.1.min(y));
            let (far_x, far_y) = most.unwrap_or((x, y));
            most = Some((far_x.max(x), far_y.max(y)));
        }
    }
    (page.dimensions(), most.map(|(x, y)| (x - least.0 + 1, y - least.1 + 1)))
}

/// A new directory for a test's files.
fn directory(test: &str) -> PathBuf {
    let dir = tmp(test, "here");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// However a document refers to a picture beside it, the picture is drawn, its own pixels on the
/// page; and so is one it holds in a `data:` URL.
#[cfg(feature = "image")]
#[test]
fn a_picture_a_document_refers_to_is_drawn() {
    let dir = directory("word-drawn");
    let red = picture(60, 60, RED);
    write(dir.join("pic.png"), &red);
    let held = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&red));
    let cases = [
        ("a.md", "One ![two](pic.png) three".to_string()),
        ("a.md", "One <img src=\"pic.png\" alt=\"two\"> three".to_string()),
        ("a.md", "![two][picture]\n\n[picture]: pic.png".to_string()),
        ("a.md", format!("One ![two]({held}) three")),
        ("a.html", "<p>One <img src=\"pic.png\" alt=\"two\"> three</p>".to_string()),
        ("a.html", format!("<p>One <img src=\"{held}\" alt=\"two\"> three</p>")),
    ];
    for (name, document) in cases {
        let path = write(dir.join(name), document.as_bytes());
        let out = view(&path, &["--protocol", "kitty"]);
        assert!(out.status.success() && out.stderr.is_empty(), "{document}: {}", text(&out.stderr));
        let (_, red) = found(&images(&out.stdout)[0].png, RED);
        assert!(red.is_some(), "{}", &document[..document.len().min(50)]);
    }
}

/// A picture's path is from the document, wherever `clv` runs: below it, above it, written as a URL
/// writes it, and from a document named by a path relative to where `clv` runs.
#[cfg(feature = "image")]
#[test]
fn a_picture_is_found_from_the_document() {
    let dir = directory("word-found");
    std::fs::create_dir_all(dir.join("docs/assets")).unwrap();
    write(dir.join("above.png"), &picture(60, 60, RED));
    write(dir.join("docs/assets/my picture.png"), &picture(60, 60, RED));
    for reference in ["../above.png", "./assets/my%20picture.png", "assets/my picture.png?raw=true", "assets/my%20picture.png#top"] {
        let document = format!("<p><img src=\"{reference}\" alt=\"two\"></p>");
        write(dir.join("docs/a.html"), document.as_bytes());
        let out = Command::new(env!("CARGO_BIN_EXE_clv"))
            .args(["docs/a.html", "--protocol", "kitty"])
            .current_dir(&dir)
            .env("TERM", "dumb")
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert!(out.status.success() && out.stderr.is_empty(), "{reference}: {}", text(&out.stderr));
        assert!(found(&images(&out.stdout)[0].png, RED).1.is_some(), "{reference}");
    }
}

/// A drawing or a PDF's page is a picture too, by the backend that shows it.
#[cfg(feature = "svg")]
#[test]
fn a_drawing_or_a_pdfs_page_is_a_picture() {
    let dir = directory("word-vector");
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="60" height="60"><rect width="60" height="60" fill="rgb(220,20,20)"/></svg>"#;
    write(dir.join("drawing.svg"), svg.as_bytes());
    write(dir.join("figure.pdf"), &crate::fixtures::letter("Figure", 2));
    let of = |document: &str| page("word-vector", "here/a.md", document);
    let (size, red) = found(&of("![two](drawing.svg)"), RED);
    let (width, height) = red.expect("the drawing");
    let expected = 45.0 / printed_page(None).w * size.0 as f32;
    // Square, but for a row or column that where it falls on the pixels leaves only partly covered.
    assert!((width as f32 - expected).abs() < 3.0 && width.abs_diff(height) <= 1, "{width} × {height}, not about {expected}");
    assert_ne!(of("![two](figure.pdf)"), of("![two](missing.pdf)"), "a PDF's page");
    assert_eq!(view(&write(dir.join("b.md"), b"![two](figure.pdf)"), &["--protocol", "kitty"]).status.code(), Some(0));
}

/// The size to show a picture at is its own, or smaller to fit the column and the page, always
/// keeping its shape.
#[cfg(feature = "image")]
#[test]
fn a_picture_fits_the_page_and_keeps_its_shape() {
    let dir = directory("word-fit");
    for (name, width, height) in [("small.png", 40, 20), ("wide.png", 3000, 300), ("tall.png", 300, 2400)] {
        write(dir.join(name), &picture(width, height, RED));
    }
    let of = |document: &str| found(&page("word-fit", "here/a.md", document), RED);
    let laid = printed_page(None);
    let ((page_w, _), small) = of("![two](small.png)");
    let points = page_w as f32 / laid.w;
    let about = |pixels: u32, wanted: f32| (pixels as f32 - wanted * points).abs() <= 2.0;
    let (w, h) = small.expect("small");
    assert!(about(w, 30.0) && about(h, 15.0), "at its size: {w} × {h}");

    let (w, h) = of("![two](wide.png)").1.expect("wide");
    assert!(about(w, laid.text_w) && about(h, laid.text_w / 10.0), "to the column: {w} × {h}");

    // 225 × 1800 pt as it is, shrunk to the page's text height (and the column, in a wide room).
    let (w, h) = of("![two](tall.png)").1.expect("tall");
    let shrunk = (laid.text_h / 1800.0).min(laid.text_w / 225.0);
    assert!(about(w, 225.0 * shrunk) && about(h, 1800.0 * shrunk), "to the page: {w} × {h}");

    // A size the document gives it: fitted after it is read.
    let (w, h) = of("<img src=\"small.png\" width=\"100\" height=\"2000\">").1.expect("given a size");
    assert!(about(w, 75.0 * laid.text_h / 1500.0) && about(h, laid.text_h), "to the page: {w} × {h}");
}

/// Opening a document draws none of its pictures, and a page draws only its own: a picture that can't
/// be drawn says so on its page and no other, and takes nothing from how the pages fall.
#[cfg(feature = "image")]
#[test]
fn a_picture_is_drawn_only_when_a_page_it_is_on_is() {
    let dir = directory("word-lazy");
    let mut broken = picture(600, 800, RED);
    broken.truncate(45);
    write(dir.join("broken.png"), &broken);
    write(dir.join("good.png"), &picture(600, 800, RED));
    let of = |picture: &str| format!("<p style=\"page-break-after:always\">One</p><p><img src=\"{picture}\" alt=\"two\"></p><p>Three</p>");
    let path = write(dir.join("a.html"), of("broken.png").as_bytes());
    let out = view(&path, &["--protocol", "kitty", "--pages", "1"]);
    assert!(out.status.success() && out.stderr.is_empty(), "the first page: {}", text(&out.stderr));
    let out = view(&path, &["--protocol", "kitty"]);
    assert!(out.status.success() && text(&out.stderr).contains("page 2: a picture it refers to could not be drawn"), "{}", text(&out.stderr));
    let broken = images(&out.stdout).len();
    let good = images(&view(&write(dir.join("b.html"), of("good.png").as_bytes()), &["--protocol", "kitty"]).stdout).len();
    assert_eq!(broken, good, "the same pages");
}

/// The first page says how many pictures on this machine it left out, and a URL isn't among them.
#[cfg(feature = "image")]
#[test]
fn a_document_says_how_many_pictures_it_left_out() {
    let dir = directory("word-left-out");
    write(dir.join("notes.txt"), b"not a picture");
    let path = write(dir.join("a.md"), b"![one](missing.png) ![two](notes.txt) ![three](https://example.com/a.png) ![four](/etc/hosts)");
    let out = view(&path, &["--protocol", "kitty"]);
    assert!(out.status.success());
    assert!(text(&out.stderr).contains("page 1: 2 pictures it refers to are left out"), "{}", text(&out.stderr));
}

/// `clv` with `args` where it can be seen to finish: a test of a pipe or a device mustn't hang.
fn finishes(args: &[&str], dir: &Path) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_clv"))
        .args(args)
        .current_dir(dir)
        .env("TERM", "dumb")
        .env_remove("TMUX")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed().as_secs() > 60 {
            child.kill().unwrap();
            panic!("it hangs on {args:?}");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    child.wait_with_output().unwrap()
}

/// Nothing is fetched: a server named by a picture is never called. Nothing is read but a regular
/// file near the document that a backend shows as a picture: a page referring to anything else is
/// as one referring to what isn't there.
#[cfg(feature = "image")]
#[test]
fn what_is_not_a_picture_near_the_document_is_not_read() {
    let dir = directory("word-not-read");
    let picture = write(dir.join("pic.png"), &picture(60, 60, RED));
    std::fs::create_dir_all(dir.join("folder")).unwrap();
    write(dir.join("other.md"), b"# Another document");
    write(dir.join("word.docx"), &docx(1));
    write(dir.join("archive.zip"), &zip(&[("a.md", "# In an archive")]));
    assert!(Command::new("mkfifo").arg(dir.join("pipe.png")).status().unwrap().success());

    let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    server.set_nonblocking(true).unwrap();
    let port = server.local_addr().unwrap().port();
    let absolute = picture.to_str().unwrap().to_owned();
    let escaped = absolute.replace('/', "%2F");

    let of = |reference: &str| {
        let document = write(dir.join("a.md"), format!("One ![two]({reference}) three").as_bytes());
        let out = finishes(&[document.to_str().unwrap(), "--protocol", "kitty"], &dir);
        assert!(out.status.success(), "{reference}: {}", text(&out.stderr));
        images(&out.stdout).swap_remove(0).png
    };
    let nothing = of("missing.png");
    let references = [
        format!("http://127.0.0.1:{port}/pic.png"),
        format!("//127.0.0.1:{port}/pic.png"),
        format!("file://{absolute}"),
        absolute.clone(),
        escaped,
        "folder".to_string(),
        "a.md".to_string(),
        "other.md".to_string(),
        "word.docx".to_string(),
        "archive.zip".to_string(),
        "pipe.png".to_string(),
    ];
    for reference in references {
        assert_eq!(of(&reference), nothing, "{reference}");
    }
    assert_eq!(server.accept().map(|_| ()).unwrap_err().kind(), std::io::ErrorKind::WouldBlock, "it called the server");
    assert_ne!(of("pic.png"), nothing, "and what is one is");
}

/// What comes through a pipe or from an archive has no directory of its own: the pictures it refers to
/// can't be reached, even with one of that name beside it, or where `clv` runs.
#[cfg(feature = "image")]
#[test]
fn what_has_no_directory_has_no_pictures() {
    let dir = directory("word-no-directory");
    write(dir.join("pic.png"), &picture(60, 60, RED));
    let piped = |reference: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_clv"))
            .args(["/dev/stdin", "--protocol", "kitty"])
            .current_dir(&dir)
            .env("TERM", "dumb")
            .env_remove("TMUX")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let document = format!("<!doctype html><p><img src=\"{reference}\" alt=\"two\"></p>");
        child.stdin.take().unwrap().write_all(document.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{}", text(&out.stderr));
        images(&out.stdout).swap_remove(0).png
    };
    assert_eq!(piped("pic.png"), piped("missing.png"), "through a pipe");

    let held = |reference: &str| {
        let archive = write(dir.join("a.zip"), &zip(&[("a.md", &format!("![two]({reference})"))]));
        let out = view(&archive, &["--protocol", "kitty"]);
        assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
        images(&out.stdout).swap_remove(0).png
    };
    assert_eq!(held("pic.png"), held("missing.png"), "in an archive");
}

/// The colour at a page's corner.
fn corner(page: &[u8]) -> [u8; 3] {
    ::image::load_from_memory(page).unwrap().to_rgb8().get_pixel(0, 0).0
}

/// `--theme` says what Markdown and HTML go on, in a page's every colour. With no terminal to ask,
/// they are as printed; and what is laid out to be printed stays so, whatever is said.
#[test]
fn the_theme_turns_markdown_and_html_dark_and_nothing_else() {
    let dark = [0x1E, 0x1E, 0x1E];
    let of = |name: &str, contents: &[u8], args: &[&str]| {
        let path = write(tmp("word-theme", name), contents);
        let out = view(&path, &[&["--protocol", "kitty"], args].concat());
        assert!(out.status.success(), "{name}: {}", text(&out.stderr));
        images(&out.stdout).swap_remove(0).png
    };
    for (name, contents) in [("a.md", &b"# One"[..]), ("a.html", b"<h1>One</h1>")] {
        assert_eq!(corner(&of(name, contents, &["--theme", "dark"])), dark, "{name}");
        assert_eq!(corner(&of(name, contents, &["--theme", "light"])), [255; 3], "{name}");
        assert_eq!(corner(&of(name, contents, &["--theme", "auto"])), [255; 3], "{name}: no terminal to go by");
        assert_eq!(corner(&of(name, contents, &[])), [255; 3], "{name}");
        // Its ink turned too: the page has something much lighter on it.
        let page = ::image::load_from_memory(&of(name, contents, &["--theme", "dark"])).unwrap().to_rgb8();
        assert!(page.pixels().any(|p| p.0.iter().all(|c| *c > 128)), "{name}: nothing light on the page");
    }
    assert_eq!(corner(&of("a.docx", &docx(1), &["--theme", "dark"])), [255; 3], "a Word document");
    assert_eq!(corner(&of("a.rtf", RTF.as_bytes(), &["--theme", "dark"])), [255; 3], "an RTF");
    assert_eq!(corner(&of("a.pdf", &crate::fixtures::letter("One", 1), &["--theme", "dark"])), [255; 3], "a PDF");

    let path = write(tmp("word-theme", "b.md"), b"# One");
    let out = view(&path, &["--protocol", "kitty", "--theme", "sepia"]);
    assert!(!out.status.success() && text(&out.stderr).contains("auto, light or dark"), "{}", text(&out.stderr));
}

/// A picture of near-black lines and nothing else would vanish on dark paper, so it comes with white
/// behind it; what shows on dark, or is mostly picture, keeps what is behind it.
#[cfg(feature = "svg")]
#[test]
fn dark_paper_doesnt_swallow_a_near_black_drawing() {
    let dir = directory("word-swallow");
    let drawing = |fill: &str, side: u32| {
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200"><rect width="{side}" height="{side}" fill="{fill}"/></svg>"#)
    };
    write(dir.join("logo.svg"), drawing("rgb(10,10,10)", 100).as_bytes());
    write(dir.join("colour.svg"), drawing("rgb(220,20,20)", 100).as_bytes());
    write(dir.join("window.svg"), drawing("rgb(10,10,10)", 180).as_bytes());
    let whites = |picture: &str, theme: &str| {
        let path = write(dir.join("a.md"), format!("![two]({picture})").as_bytes());
        let out = view(&path, &["--protocol", "kitty", "--theme", theme]);
        let page = ::image::load_from_memory(&images(&out.stdout)[0].png).unwrap().to_rgb8();
        page.pixels().filter(|p| p.0 == [255; 3]).count()
    };
    assert!(whites("logo.svg", "dark") > 100, "the logo has white behind it");
    assert_eq!(whites("colour.svg", "dark"), 0, "a colour shows without it");
    assert_eq!(whites("window.svg", "dark"), 0, "a dark window shows without it");
    let (of_logo, of_nothing) = (whites("logo.svg", "light"), whites("missing.svg", "light"));
    assert!(of_logo > of_nothing / 2, "on white paper, white behind it is nothing to see");
}

/// In a directory or an archive they are tiles, while text that is neither is left out. A list's
/// documents are previews, laid out on a fixed portrait page, 60 × 40 cells at the cell the run started
/// with, whatever the window.
#[test]
fn in_a_list_markdown_and_html_are_tiles() {
    let files = [("a.md", "# One"), ("b.html", "<p>Two</p>"), ("c.txt", "Three")];
    let dir = tmp("word-text-list", "shelf");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, contents) in files {
        write(dir.join(name), contents.as_bytes());
    }
    for path in [dir, write(tmp("word-text-list", "shelf.zip"), &zip(&files))] {
        let out = view(&path, &["--protocol", "kitty"]);
        assert!(out.status.success(), "{}", text(&out.stderr));
        assert_eq!(names(&out.stdout), [" a.md b.html"], "{}", path.display());
        let tiles: Vec<(u32, u32)> = images(&out.stdout).iter().map(|d| png_size(&d.png)).collect();
        let (w, h) = preview_page();
        assert!(shaped(&tiles, w, h), "{}: {tiles:?}, not {w} × {h}", path.display());
    }
}

/// Through a pipe there is no name to go by: HTML says what it is, and Markdown can't.
#[test]
fn through_a_pipe_html_says_what_it_is_and_markdown_cant() {
    let piped = |document: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_clv"))
            .args(["/dev/stdin", "--protocol", "kitty"])
            .env("TERM", "dumb")
            .env_remove("TMUX")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(document.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    };
    let out = piped("<!doctype html><p>One</p>");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(images(&out.stdout).len(), 1);
    assert!(!piped("# One").status.success());
}

/// One that would cost too much is refused with why, before that is spent; what a page has that
/// isn't laid out (here a script as long as the most text it may have) costs next to nothing.
#[test]
fn what_would_cost_too_much_is_refused_and_what_a_page_skips_is_not_counted() {
    let costly = write(tmp("word-cost", "stars.md"), "*a".repeat(600_000).as_bytes());
    let out = view(&costly, &["--protocol", "kitty"]);
    assert!(!out.status.success() && text(&out.stderr).contains("lines and marks"), "{}", text(&out.stderr));
    let script = format!("<script>{}</script><p>Shown</p>", "a".repeat(9 << 20));
    assert_eq!(drawn(&write(tmp("word-cost", "page.html"), script.as_bytes()), &[]).len(), 1);
}

/// `--width` lays Markdown and HTML out to lines of that many characters of code: narrower pages,
/// as tall as the window's, and more of them; `window` is the window's, as without it. It is taken
/// whatever the files, doing nothing to a PDF, and refused out of its range, saying what it takes.
#[test]
fn the_width_lays_markdown_and_html_out_to_so_many_characters() {
    let (window, at_40) = (printed_page(None), printed_page(Some(40.0)));
    let long: String = (1..=60).map(|n| format!("Paragraph {n}, with words enough in it to fill a line or two of a page.\n\n")).collect();
    for (name, document) in [("long.md", long.clone()), ("long.html", long.split("\n\n").map(|p| format!("<p>{p}</p>")).collect())] {
        let path = write(tmp("word-width", name), document.as_bytes());
        let (wide, narrow) = (drawn(&path, &[]), drawn(&path, &["--width", "40"]));
        assert!(shaped(&wide, window.w, window.h) && shaped(&narrow, at_40.w, at_40.h), "{name}: {wide:?} {narrow:?}");
        assert!(narrow.len() > wide.len(), "{name}: {} pages at 40, {} at the window's width", narrow.len(), wide.len());
        assert_eq!(drawn(&path, &["--width", "window"]), wide, "{name}");
        let out = view(&path, &["--protocol", "kitty", "--width", "300"]);
        assert!(!out.status.success() && text(&out.stderr).contains("--width: expected window or 20–200, got 300"), "{}", text(&out.stderr));
        // The font size: the same page, larger text, so more pages; refused out of its range, with its unit.
        let larger = drawn(&path, &["--font", "150"]);
        assert!(shaped(&larger, window.w, window.h) && larger.len() > wide.len(), "{name}: {} pages at 150%, {} at 100%", larger.len(), wide.len());
        let out = view(&path, &["--protocol", "kitty", "--font", "500"]);
        assert!(!out.status.success() && text(&out.stderr).contains("--font: expected 50–400%, got 500"), "{}", text(&out.stderr));
    }
    let pdf = write(tmp("word-width", "a.pdf"), &crate::fixtures::letter("One", 1));
    assert!(shaped(&drawn(&pdf, &["--width", "40", "--font", "200"]), 612.0, 792.0));
}

/// `--margin` sets Markdown and HTML's side margins, in characters of code at each side: at `window`
/// the page is the room's and its lines shorter, so there are more pages; at a width the line keeps its
/// length and the page grows by them. Refused out of its range; taken, and doing nothing, on a PDF.
#[test]
fn the_margin_shortens_the_line_or_widens_the_page() {
    let window = printed_page(None);
    let long: String = (1..=60).map(|n| format!("Paragraph {n}, with words enough in it to fill a line or two of a page.\n\n")).collect();
    for (name, document) in [("long.md", long.clone()), ("long.html", long.split("\n\n").map(|p| format!("<p>{p}</p>")).collect())] {
        let path = write(tmp("word-margin", name), document.as_bytes());
        let (three, ten) = (drawn(&path, &[]), drawn(&path, &["--margin", "10"]));
        assert!(shaped(&ten, window.w, window.h) && ten.len() > three.len(), "{name}: {} pages at 10, {} at 3", ten.len(), three.len());
        assert_eq!(drawn(&path, &["--margin", "3"]), three, "{name}: 3 is the default");
        // At 40 characters, 10 at each side: 60 columns.
        let at_40 = printed_page(Some(40.0));
        assert!(shaped(&drawn(&path, &["--width", "40", "--margin", "10"]), at_40.w + 14.0 * COLUMN, at_40.h), "{name}");
        let out = view(&path, &["--protocol", "kitty", "--margin", "30"]);
        assert!(!out.status.success() && text(&out.stderr).contains("--margin: expected 0–20, got 30"), "{}", text(&out.stderr));
    }
    let pdf = write(tmp("word-margin", "a.pdf"), &crate::fixtures::letter("One", 1));
    assert!(shaped(&drawn(&pdf, &["--margin", "10"]), 612.0, 792.0));
}

/// `--per-row` lays Markdown and HTML out in that many columns of the room printing lays out for, the
/// tty's or with none the preview's ([`printed_page`]): each its share of the room's columns less the
/// gaps, as tall, and more pages of them; printed that many to a row. A typed `--width` disagrees with
/// it, and wins, with a warning; on a PDF both hold, each for what it is about.
#[test]
fn per_row_lays_markdown_and_html_out_in_columns() {
    let window = printed_page(None);
    let cols = tty_window().map_or(60, |(cols, _, _)| cols);
    // Three where each holds 20 characters at 100%, else two.
    let n = if (cols.saturating_sub(4) / 3).saturating_sub(6) >= 20 { 3 } else { 2 };
    let column = f32::from((cols - 2 * (n - 1)) / n) * COLUMN;
    let long: String = (1..=60).map(|n| format!("Paragraph {n}, with words enough in it to fill a line or two of a page.\n\n")).collect();
    for (name, document) in [("long.md", long.clone()), ("long.html", long.split("\n\n").map(|p| format!("<p>{p}</p>")).collect())] {
        let path = write(tmp("word-per-row", name), document.as_bytes());
        let (alone, columns) = (drawn(&path, &[]), drawn(&path, &["--per-row", &n.to_string()]));
        assert!(shaped(&columns, column, window.h), "{name}: {columns:?}, not {column} × {}", window.h);
        assert!(columns.len() > alone.len(), "{name}: {} pages in columns, {} alone", columns.len(), alone.len());
        // Printed in rows: each page leaves the cursor where it was (`C=1`), not as a page alone does.
        let out = view(&path, &["--protocol", "kitty", "--per-row", &n.to_string()]);
        let placed = images(&out.stdout);
        assert!(placed.len() >= usize::from(n) && placed.iter().all(|d| d.keys.contains(",C=1,")), "{name}: rows of {n}");
        // `--width 40` and a count disagree: the width wins, said once.
        let out = view(&path, &["--protocol", "kitty", "--width", "40", "--per-row", "3"]);
        let at_40 = printed_page(Some(40.0));
        let sizes: Vec<(u32, u32)> = images(&out.stdout).iter().map(|d| png_size(&d.png)).collect();
        assert!(out.status.success() && shaped(&sizes, at_40.w, at_40.h), "{name}: {sizes:?}");
        assert_eq!(text(&out.stderr), format!("warning: --width and --per-row disagree on {name}: the width wins\n"), "{name}");
        // Its old name, as typed.
        let out = view(&path, &["--protocol", "kitty", "--width", "40", "--zoom", "3"]);
        assert!(text(&out.stderr).contains("--width and --zoom disagree"), "{}", text(&out.stderr));
    }
    let pdf = write(tmp("word-per-row", "a.pdf"), &crate::fixtures::letter("One", 3));
    let out = view(&pdf, &["--protocol", "kitty", "--width", "40", "--per-row", "3"]);
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
}

/// A protected Word document opens with its password, says it needs one without, and refuses a
/// wrong one.
#[test]
fn a_protected_document_opens_with_its_password() {
    let locked = wordcraft_docx::encrypt_with_spin_count(&docx(2), "secret", 1000).unwrap();
    let path = write(tmp("word-password", "locked.docx"), &locked);
    let pages = drawn(&path, &["--password", "secret"]);
    assert!(pages.len() == 2 && shaped(&pages, 200.0, 300.0), "{pages:?}");
    let out = view(&path, &["--protocol", "kitty"]);
    assert_eq!(text(&out.stderr), format!("clv: {}: this document is protected with a password: give it with --password\n", path.display()));
    let out = view(&path, &["--protocol", "kitty", "--password", "wrong"]);
    assert_eq!(text(&out.stderr), format!("clv: {}: the password is incorrect\n", path.display()));
}

/// The crates wordcraft is built from, as `cargo metadata` resolves them.
fn packages() -> Vec<Value> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let out = Command::new(cargo).args(["metadata", "--format-version", "1", "--manifest-path"]).arg(&manifest).output().unwrap();
    assert!(out.status.success(), "cargo metadata: {}", String::from_utf8_lossy(&out.stderr));
    let metadata: Value = serde_json::from_slice(&out.stdout).unwrap();
    metadata["packages"].as_array().unwrap().clone()
}

/// Every wordcraft crate comes from the one commit `Cargo.toml` pins. wordcraft patches crates
/// (`[patch.crates-io]`, which Cargo honours only in the root manifest) for what this backend
/// doesn't build, its PDF export and its app: none of them may be built here, stock, without the
/// patch.
#[test]
fn every_wordcraft_crate_is_from_the_pinned_commit_and_none_it_patches_is_built() {
    let packages = packages();
    let render = packages.iter().find(|p| p["name"] == "wordcraft-render").unwrap();
    let pinned = render["source"].as_str().unwrap();
    assert!(pinned.starts_with("git+https://github.com/storytold/wordcraft?rev="), "wordcraft-render from {pinned}");
    let ours: Vec<&Value> = packages.iter().filter(|p| p["name"].as_str().is_some_and(|n| n.starts_with("wordcraft-"))).collect();
    assert!(ours.len() >= 6, "{} wordcraft crates", ours.len());
    for p in ours {
        assert_eq!(p["source"].as_str(), Some(pinned), "{} is not at the pinned commit", p["name"]);
    }
    // wordcraft's root manifest at that commit: <root>/crates/render/Cargo.toml → <root>/Cargo.toml.
    let root = Path::new(render["manifest_path"].as_str().unwrap()).ancestors().nth(3).unwrap().join("Cargo.toml");
    let manifest: toml::Table = std::fs::read_to_string(&root).unwrap().parse().unwrap();
    let patched: Vec<&String> = manifest["patch"]["crates-io"].as_table().unwrap().keys().collect();
    assert!(patched.iter().any(|name| *name == "krilla"), "read from {}: {patched:?}", root.display());
    let built: Vec<&String> = patched.into_iter().filter(|name| packages.iter().any(|p| p["name"] == name.as_str())).collect();
    assert!(built.is_empty(), "wordcraft patches {built:?}, built here without the patch");
}

/// What the Word backend reads and renders with is named only by it and its tests (`CLAUDE.md`), so it can
/// be swapped for another.
#[test]
fn nothing_but_this_backend_names_wordcraft_or_pulldown_cmark() {
    crate::fixtures::named_only_in(
        &["src/source/wordcraft.rs", "tests/it/wordcraft.rs"],
        &["&wordcraft::Wordcraft,", "mod wordcraft;"],
        &["wordcraft", "pulldown"],
    );
}
