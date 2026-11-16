//! The epub backend, through the viewer: a book is its chapters, in its reading order, each on a
//! page of its own, with its pictures; which zips are books; what a book that can't be read, or a
//! chapter that can't, comes to; hostile packages, which cost the book and never the viewer; and
//! that nothing else names the library its package is read with. Its own tests
//! (`src/source/epub.rs`) read packages and assemble books directly; the pager's are in
//! `tests/it/view_pager.rs`.
#![cfg(all(unix, feature = "epub", feature = "image"))]

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ::zip::write::SimpleFileOptions;
use ::zip::{CompressionMethod, ZipWriter};

use crate::view::{images, text, tmp, view};

pub(crate) const RED: [u8; 3] = [220, 20, 20];
pub(crate) const BLUE: [u8; 3] = [20, 20, 220];
pub(crate) const GREEN: [u8; 3] = [20, 180, 20];

/// A `side` × `side` PNG of `colour`.
pub(crate) fn plain(side: u32, colour: [u8; 3]) -> Vec<u8> {
    let [r, g, b] = colour;
    let mut png = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(side, side, image::Rgba([r, g, b, 255])).write_to(&mut png, image::ImageFormat::Png).unwrap();
    png.into_inner()
}

/// How many of `page`'s pixels are about `colour`.
pub(crate) fn of(page: &image::RgbImage, colour: [u8; 3]) -> usize {
    page.pixels().filter(|pixel| pixel.0.iter().zip(colour).all(|(a, b)| a.abs_diff(b) < 12)).count()
}

/// How a book's first file, `mimetype`, is written.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Mimetype {
    /// First, stored as it is: as the format says.
    Stored,
    /// First, compressed, as some tools write it.
    Packed,
    /// Not at all.
    Absent,
}

/// A zip of `files`, by name and contents, deflated, after its `mimetype`.
pub(crate) fn zipped(files: &[(String, Vec<u8>)], mimetype: Mimetype) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let method = |method| SimpleFileOptions::default().compression_method(method);
    if mimetype != Mimetype::Absent {
        zip.start_file("mimetype", method(if mimetype == Mimetype::Stored { CompressionMethod::Stored } else { CompressionMethod::Deflated }))
            .unwrap();
        zip.write_all(b"application/epub+zip").unwrap();
    }
    for (name, contents) in files {
        zip.start_file(name.as_str(), method(CompressionMethod::Deflated)).unwrap();
        zip.write_all(contents).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// The container of a book whose package is `OEBPS/content.opf`.
const CONTAINER: &str = r#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;

/// A package: what its `<metadata>` holds beyond what it must, its manifest's `items` and its
/// spine's `itemrefs`, as written.
pub(crate) fn package(metadata: &str, items: &str, itemrefs: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">x</dc:identifier><dc:title>A book</dc:title><dc:language>en</dc:language>{metadata}</metadata><manifest>{items}</manifest><spine>{itemrefs}</spine></package>"#
    )
}

/// A chapter whose body is `body`, with a head and a stylesheet of its own, as a book's has.
pub(crate) fn chapter(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\">\n<head><title>A chapter</title><link rel=\"stylesheet\" href=\"../style.css\"/><style>p {{ color: red }}</style></head>\n<body>\n{body}\n</body>\n</html>\n"
    )
}

/// The files of a book of `chapters`, each a body, in `OEBPS/Text/`, named `ch1.xhtml` and on, in
/// that reading order, with `more` files, by their names from the book's top.
pub(crate) fn book_of(chapters: &[String], more: &[(&str, Vec<u8>)]) -> Vec<(String, Vec<u8>)> {
    let items: String =
        (1..=chapters.len()).map(|n| format!(r#"<item id="ch{n}" href="Text/ch{n}.xhtml" media-type="application/xhtml+xml"/>"#)).collect();
    let itemrefs: String = (1..=chapters.len()).map(|n| format!(r#"<itemref idref="ch{n}"/>"#)).collect();
    let mut files = vec![
        ("META-INF/container.xml".to_owned(), CONTAINER.as_bytes().to_vec()),
        ("OEBPS/content.opf".to_owned(), package("", &items, &itemrefs).into_bytes()),
    ];
    files.extend(chapters.iter().enumerate().map(|(n, body)| (format!("OEBPS/Text/ch{}.xhtml", n + 1), chapter(body).into_bytes())));
    files.extend(more.iter().map(|(name, bytes)| ((*name).to_owned(), bytes.clone())));
    files
}

/// A book of three chapters, each a heading, a line and a picture of its own colour, red, blue
/// and green, in `OEBPS/Images/`, reached from the chapter by `../Images/`.
pub(crate) fn three_chapters() -> Vec<(String, Vec<u8>)> {
    let chapters: Vec<String> = ["red", "blue", "green"]
        .iter()
        .enumerate()
        .map(|(n, colour)| {
            format!(
                "<h1 id=\"start\">Chapter {}</h1>\n<p>The {colour} one.</p>\n<p><img src=\"../Images/{colour}.png\" alt=\"{colour}\"/></p>",
                n + 1
            )
        })
        .collect();
    let pictures =
        [("OEBPS/Images/red.png", plain(120, RED)), ("OEBPS/Images/blue.png", plain(120, BLUE)), ("OEBPS/Images/green.png", plain(120, GREEN))];
    book_of(&chapters, &pictures)
}

pub(crate) fn write(path: PathBuf, bytes: &[u8]) -> PathBuf {
    std::fs::write(&path, bytes).unwrap();
    path
}

/// The pages `clv` printed of `path` with `args`, and what it said.
fn pages(path: &Path, args: &[&str]) -> (Vec<image::RgbImage>, String) {
    let out = view(path, &[&["--protocol", "kitty"], args].concat());
    (images(&out.stdout).iter().map(|page| image::load_from_memory(&page.png).unwrap().to_rgb8()).collect(), text(&out.stderr))
}

/// Which of red, blue and green each of `pages` shows.
fn colours(pages: &[image::RgbImage]) -> Vec<[bool; 3]> {
    pages.iter().map(|page| [RED, BLUE, GREEN].map(|colour| of(page, colour) > 0)).collect()
}

const IN_ORDER: [[bool; 3]; 3] = [[true, false, false], [false, true, false], [false, false, true]];

/// A book is its chapters in its reading order, each on a new page, with the pictures it refers to
/// from its chapters' own directory; and which zips are books: one that says so, whatever its
/// name, and one named `.epub` that doesn't say, its `mimetype` compressed or missing. A zip that
/// neither says so nor is named so is a list of its files.
#[test]
fn an_epub_is_a_book_its_chapters_in_order_each_on_a_new_page() {
    let files = three_chapters();
    for (name, mimetype) in [
        ("book.epub", Mimetype::Stored),
        ("unnamed.zip", Mimetype::Stored),
        ("unnamed", Mimetype::Stored),
        ("packed.epub", Mimetype::Packed),
        ("unsaid.epub", Mimetype::Absent),
    ] {
        let path = write(tmp("epub-book", name), &zipped(&files, mimetype));
        let (pages, said) = pages(&path, &[]);
        assert_eq!((colours(&pages), said.as_str()), (IN_ORDER.to_vec(), ""), "{name}");
    }
    // No book: its files, the pictures among them, as an archive's.
    let path = write(tmp("epub-book", "unsaid.zip"), &zipped(&files, Mimetype::Absent));
    let out = view(&path, &["--protocol", "kitty", "-r"]);
    assert_eq!(crate::view::names(&out.stdout).len(), 2, "a list: {}", text(&out.stderr));
}

/// A book comes through a pipe, kept whole; and from an archive, where its tile in the list is
/// its first page, read from the archive.
#[test]
fn an_epub_is_read_through_a_pipe_and_from_an_archive() {
    let book = zipped(&three_chapters(), Mimetype::Stored);
    let mut child = Command::new(env!("CARGO_BIN_EXE_clv"))
        .args(["/dev/stdin", "--protocol", "kitty"])
        .env("TERM", "dumb")
        .env_remove("TMUX")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let piped = book.clone();
    let writer = std::thread::spawn(move || stdin.write_all(&piped));
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap().unwrap();
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    let printed: Vec<image::RgbImage> = images(&out.stdout).iter().map(|page| image::load_from_memory(&page.png).unwrap().to_rgb8()).collect();
    assert_eq!(colours(&printed), IN_ORDER, "through a pipe");

    let shelf = write(tmp("epub-held", "shelf.zip"), &zipped(&[("books/book.epub".to_owned(), book)], Mimetype::Absent));
    let (tiles, said) = pages(&shelf, &["-r"]);
    assert_eq!((colours(&tiles), said.as_str()), (vec![IN_ORDER[0]], ""), "in an archive, its first page its tile");
}

/// A book that can't be read as one is refused, with why, and isn't shown as a zip instead:
/// without its container, without its package, with a package that lists no chapters.
#[test]
fn a_book_that_cant_be_read_is_refused_not_shown_as_a_zip() {
    let refused = |name: &str, files: &[(String, Vec<u8>)]| {
        let path = write(tmp("epub-refused", name), &zipped(files, Mimetype::Stored));
        let out = view(&path, &["--protocol", "kitty"]);
        assert!(!out.status.success() && out.stdout.is_empty(), "{name}");
        text(&out.stderr)
            .strip_prefix(&format!("clv: {}: ", path.display()))
            .unwrap_or_else(|| panic!("{name}: {}", text(&out.stderr)))
            .trim_end()
            .to_owned()
    };
    let chapter = ("OEBPS/Text/ch1.xhtml".to_owned(), chapter("<p>one</p>").into_bytes());
    assert_eq!(refused("no-container.epub", std::slice::from_ref(&chapter)), "it has no container (META-INF/container.xml)");
    let container = ("META-INF/container.xml".to_owned(), CONTAINER.as_bytes().to_vec());
    assert_eq!(refused("no-package.epub", &[container.clone(), chapter.clone()]), "it has no package (OEBPS/content.opf)");
    let item = r#"<item id="ch1" href="Text/ch1.xhtml" media-type="application/xhtml+xml"/>"#;
    let no_spine = ("OEBPS/content.opf".to_owned(), package("", item, "").into_bytes());
    assert_eq!(refused("no-spine.epub", &[container.clone(), no_spine, chapter.clone()]), "its package lists no chapters to read");
    let broken = ("OEBPS/content.opf".to_owned(), b"<package><manifest><item".to_vec());
    let why = refused("broken.epub", &[container, broken, chapter]);
    assert!(!why.is_empty() && why.len() <= 256 && !why.contains('\n'), "{why}");
}

/// Packages made to cost rbook the most cost the book at most, and never the viewer, which says
/// why and ends as it does for any file it can't show: a chain of metadata each refining the one
/// before, as long as a package may be, which rbook refuses or fails on; and 599,000 empty
/// metadata elements in under 4 MiB, which take the worker past its heap, and get it killed. A
/// manifest item whose media type has a character past ASCII, which rbook would panic on if asked
/// whether it is a font, is never asked: the book shows.
#[test]
fn a_hostile_package_costs_the_book_and_never_the_viewer() {
    let files = |metadata: &str, items: &str| {
        let item = r#"<item id="ch1" href="Text/ch1.xhtml" media-type="application/xhtml+xml"/>"#;
        vec![
            ("META-INF/container.xml".to_owned(), CONTAINER.as_bytes().to_vec()),
            ("OEBPS/content.opf".to_owned(), package(metadata, &format!("{item}{items}"), r#"<itemref idref="ch1"/>"#).into_bytes()),
            ("OEBPS/Text/ch1.xhtml".to_owned(), chapter("<p>one</p>").into_bytes()),
        ]
    };
    let shown = |name: &str, files: &[(String, Vec<u8>)]| {
        let path = write(tmp("epub-hostile", name), &zipped(files, Mimetype::Stored));
        let started = std::time::Instant::now();
        let out = view(&path, &["--protocol", "kitty"]);
        assert!(out.status.code().is_some(), "{name}: it ended by a signal");
        assert!(started.elapsed() < std::time::Duration::from_secs(20), "{name}: {:?}", started.elapsed());
        let said = text(&out.stderr);
        (images(&out.stdout).len(), said.strip_prefix(&format!("clv: {}: ", path.display())).map(|why| why.trim_end().to_owned()).unwrap_or(said))
    };
    let font = files("", "<item id=\"f\" href=\"f.otf\" media-type=\"application/font\u{e8}\"/>");
    assert_eq!(shown("font.epub", &font), (1, String::new()));
    let chain: String = (1..70_000).map(|n| format!(r##"<meta id="m{n}" refines="#m{}" property="p">v</meta>"##, n - 1)).collect();
    let refines = files(&format!(r#"<meta id="m0" property="dcterms:modified">2020</meta>{chain}"#), "");
    assert!(refines[1].1.len() > 3 << 20 && refines[1].1.len() < 4 << 20);
    let (pages, why) = shown("refines.epub", &refines);
    assert!(pages == 0 && (why == "its package couldn't be read" || why.starts_with("Cycle detected in metadata refinements")), "{why}");
    let empty = files(&"<meta/>".repeat(599_000), "");
    assert!(empty[1].1.len() < 4 << 20);
    assert_eq!(shown("empty.epub", &empty), (0, "its package couldn't be read".to_owned()));
    // And a package past what is read of one is never given to the worker.
    let large = files(&"<meta/>".repeat(600_000), "");
    assert_eq!(shown("large.epub", &large), (0, "its package is more than 4096 KiB".to_owned()));
}

/// A chapter the book doesn't hold is left out, the first page saying so, and the rest shown.
#[test]
fn a_chapter_missing_is_left_out_and_said() {
    let files: Vec<(String, Vec<u8>)> = three_chapters().into_iter().filter(|(name, _)| name != "OEBPS/Text/ch2.xhtml").collect();
    let path = write(tmp("epub-missing", "book.epub"), &zipped(&files, Mimetype::Stored));
    let (pages, said) = pages(&path, &[]);
    assert_eq!(colours(&pages), [IN_ORDER[0], IN_ORDER[2]]);
    assert_eq!(said, "warning: page 1: 1 chapter is left out: not in the book\n");
}

/// A book has a document's settings: laid out to a width in characters, its page narrower than
/// the window's, and on dark paper.
#[test]
fn a_book_is_laid_out_to_a_width_and_drawn_on_dark_paper() {
    let path = write(tmp("epub-settings", "book.epub"), &zipped(&three_chapters(), Mimetype::Stored));
    let shape = |page: &image::RgbImage| page.width() as f32 / page.height() as f32;
    let (window, _) = pages(&path, &[]);
    let (narrow, said) = pages(&path, &["--width", "30"]);
    assert!(said.is_empty() && narrow.len() >= 3, "{said}");
    assert!(shape(&narrow[0]) < shape(&window[0]) * 0.8, "a narrower page: {} and {}", shape(&narrow[0]), shape(&window[0]));
    let corner = |page: &image::RgbImage| page.get_pixel(0, 0).0;
    let (dark, said) = pages(&path, &["--theme", "dark"]);
    assert!(said.is_empty() && corner(&window[0]) == [255; 3] && corner(&dark[0]).iter().all(|c| *c < 80), "{:?}", corner(&dark[0]));
    assert_eq!(colours(&dark), IN_ORDER, "its pictures on it");
    // What only another kind of file takes is refused, as for any file.
    let out = view(&path, &["--protocol", "kitty", "--password", "x"]);
    assert!(!out.status.success() && out.stdout.is_empty(), "{}", text(&out.stderr));
}

/// `--help` lists the format, by its extension, and credits what reads it.
#[test]
fn help_lists_books_and_credits_rbook() {
    let out = Command::new(env!("CARGO_BIN_EXE_clv")).arg("--help").env("TERM", "dumb").env_remove("TMUX").output().unwrap();
    let help = text(&out.stdout);
    assert!(help.contains("EPUB") && help.contains("rbook"), "{help}");
}

/// rbook is named only by the backend that reads a package with it, and by these tests.
#[test]
fn nothing_but_this_backend_names_rbook() {
    crate::fixtures::named_only_in(&["src/source/epub.rs", "tests/it/epub.rs"], &[], &["rbook"]);
}
