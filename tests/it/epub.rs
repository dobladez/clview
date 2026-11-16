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
    book_with(chapters, more, "", "")
}

/// [`book_of`], its package with `metadata` and manifest `items` of its own besides.
pub(crate) fn book_with(chapters: &[String], more: &[(&str, Vec<u8>)], metadata: &str, items: &str) -> Vec<(String, Vec<u8>)> {
    let chapter_items: String =
        (1..=chapters.len()).map(|n| format!(r#"<item id="ch{n}" href="Text/ch{n}.xhtml" media-type="application/xhtml+xml"/>"#)).collect();
    let itemrefs: String = (1..=chapters.len()).map(|n| format!(r#"<itemref idref="ch{n}"/>"#)).collect();
    let mut files = vec![
        ("META-INF/container.xml".to_owned(), CONTAINER.as_bytes().to_vec()),
        ("OEBPS/content.opf".to_owned(), package(metadata, &format!("{chapter_items}{items}"), &itemrefs).into_bytes()),
    ];
    files.extend(chapters.iter().enumerate().map(|(n, body)| (format!("OEBPS/Text/ch{}.xhtml", n + 1), chapter(body).into_bytes())));
    files.extend(more.iter().map(|(name, bytes)| ((*name).to_owned(), bytes.clone())));
    files
}

/// A book of three chapters, each a heading, a line and a picture of its own colour, red, blue
/// and green, in `OEBPS/Images/`, reached from the chapter by `../Images/`.
pub(crate) fn three_chapters() -> Vec<(String, Vec<u8>)> {
    let (chapters, pictures) = coloured_chapters();
    book_of(&chapters, &pictures)
}

/// The bodies and the pictures of [`three_chapters`]'s book.
pub(crate) fn coloured_chapters() -> (Vec<String>, Vec<(&'static str, Vec<u8>)>) {
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
        vec![("OEBPS/Images/red.png", plain(120, RED)), ("OEBPS/Images/blue.png", plain(120, BLUE)), ("OEBPS/Images/green.png", plain(120, GREEN))];
    (chapters, pictures)
}

const YELLOW: [u8; 3] = [230, 200, 20];

/// [`three_chapters`]'s book with a yellow cover its package names, as EPUB 3 does; and with
/// `first` as a chapter before the others, if given.
fn with_a_cover(first: Option<&str>) -> Vec<(String, Vec<u8>)> {
    let (mut chapters, mut pictures) = coloured_chapters();
    if let Some(first) = first {
        chapters.insert(0, first.to_owned());
    }
    pictures.push(("OEBPS/Images/cover.png", plain(200, YELLOW)));
    book_with(&chapters, &pictures, "", r#"<item id="cover" href="Images/cover.png" media-type="image/png" properties="cover-image"/>"#)
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

/// The cover the package names is the book's first page, before its first chapter, and so its tile
/// in a list; a book whose first chapter is its cover's page, a drawing that wraps the picture,
/// shows it once.
#[test]
fn a_books_cover_is_its_first_page_and_its_tile_in_a_list() {
    let yellow = |page: &image::RgbImage| of(page, YELLOW) > 0;
    let dir = tmp("epub-cover", "shelf");
    std::fs::create_dir_all(&dir).unwrap();
    let book = write(dir.join("a.epub"), &zipped(&with_a_cover(None), Mimetype::Stored));
    let (shown, said) = pages(&book, &[]);
    assert!(said.is_empty() && shown.len() == 4 && yellow(&shown[0]), "{said}: {} pages", shown.len());
    assert_eq!((colours(&shown[..1]), colours(&shown[1..])), (vec![[false; 3]], IN_ORDER.to_vec()), "the cover alone, then the chapters");
    let wrapper = r#"<div><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 200 200"><image width="200" height="200" xlink:href="../Images/cover.png"/></svg></div>"#;
    let wrapped = write(dir.join("b.epub"), &zipped(&with_a_cover(Some(wrapper)), Mimetype::Stored));
    let (shown, said) = pages(&wrapped, &[]);
    assert!(said.is_empty() && shown.len() == 4 && yellow(&shown[0]) && !shown[1..].iter().any(yellow), "{said}: {} pages", shown.len());
    // In a list, each book's tile is its first page: its cover.
    let (tiles, said) = pages(&dir, &[]);
    assert!(said.is_empty() && tiles.len() == 2 && tiles.iter().all(yellow), "{said}: {} tiles", tiles.len());
}

/// A chapter the book protects (DRM) is left out, the first page saying how many are, and the
/// rest shown; a font only obfuscated is no matter; a book none of whose chapters can be read is
/// refused. A book made to be laid out page by page says it is laid out flowing.
#[test]
fn a_protected_chapter_is_left_out_and_a_book_all_protected_refused() {
    let data = |algorithm: &str, uri: &str| {
        format!(
            r#"<EncryptedData xmlns="http://www.w3.org/2001/04/xmlenc#"><EncryptionMethod Algorithm="{algorithm}"/><CipherData><CipherReference URI="{uri}"/></CipherData></EncryptedData>"#
        )
    };
    let encryption = |chapters: &[usize]| {
        let chapters: String =
            chapters.iter().map(|n| data("http://www.w3.org/2001/04/xmlenc#aes256-cbc", &format!("OEBPS/Text/ch{n}.xhtml"))).collect();
        let font = data("http://www.idpf.org/2008/embedding", "OEBPS/Fonts/a.otf");
        format!(r#"<?xml version="1.0"?><encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container">{font}{chapters}</encryption>"#)
            .into_bytes()
    };
    let chapters: Vec<String> = (1..=5).map(|n| format!("<p>Chapter {n}</p>")).collect();
    let book = |name: &str, protected: &[usize]| {
        let mut files = book_of(&chapters, &[("META-INF/encryption.xml", encryption(protected))]);
        // What is protected is no text: its bytes are whatever its cipher made of them.
        for n in protected {
            files.iter_mut().find(|(name, _)| *name == format!("OEBPS/Text/ch{n}.xhtml")).unwrap().1 = vec![0x9F, 0x00, 0xFF, 0x80];
        }
        write(tmp("epub-protected", name), &zipped(&files, Mimetype::Stored))
    };
    let (shown, said) = pages(&book("two.epub", &[2, 4]), &[]);
    assert_eq!((shown.len(), said.as_str()), (3, "warning: page 1: 2 chapters are protected (DRM)\n"));
    let (shown, said) = pages(&book("fonts.epub", &[]), &[]);
    assert_eq!((shown.len(), said.as_str()), (5, ""), "a font obfuscated is no protected file");
    let all = book("all.epub", &[1, 2, 3, 4, 5]);
    let out = view(&all, &["--protocol", "kitty"]);
    assert!(!out.status.success() && out.stdout.is_empty());
    assert_eq!(text(&out.stderr), format!("clv: {}: it is protected (DRM)\n", all.display()));
    // Fixed layout, said with the first page, and laid out as any other book.
    let fixed = book_with(&chapters, &[], r#"<meta property="rendition:layout">pre-paginated</meta>"#, "");
    let (shown, said) = pages(&write(tmp("epub-protected", "fixed.epub"), &zipped(&fixed, Mimetype::Stored)), &[]);
    assert_eq!(shown.len(), 5);
    assert_eq!(said, "warning: page 1: a fixed-layout book: its pages are laid out as its text flows, not as they were made\n");
}

/// A nav document whose table of contents is `list`.
pub(crate) fn nav(list: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Contents</title></head><body><nav epub:type="toc"><h1>Contents</h1>{list}</nav></body></html>"#
    )
    .into_bytes()
}

/// The manifest's item for a nav document at `OEBPS/nav.xhtml`.
pub(crate) const NAV_ITEM: &str = r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#;

/// A nav's list of [`three_chapters`]'s chapters, as a nav at `OEBPS/nav.xhtml` writes them.
pub(crate) const THREE_ENTRIES: &str = r##"<ol><li><a href="Text/ch1.xhtml">The red one</a></li><li><a href="Text/ch2.xhtml#start">The blue one</a></li><li><a href="Text/ch3.xhtml">The green one</a></li></ol>"##;

/// A book's table of contents is a Contents page after its cover, before its first chapter; none
/// is made for a book whose own page of contents is in its reading order. A table of contents
/// that can't be read (an NCX nested 100,000 deep, more than is read of one) leaves the book
/// without the page, its first page saying so; one nested as deep as fits is read.
#[test]
fn a_books_contents_page_comes_after_its_cover_and_isnt_made_twice() {
    let yellow = |page: &image::RgbImage| of(page, YELLOW) > 0;
    let cover = r#"<item id="cover" href="Images/cover.png" media-type="image/png" properties="cover-image"/>"#;
    let (chapters, mut pictures) = coloured_chapters();
    pictures.push(("OEBPS/Images/cover.png", plain(200, YELLOW)));
    let shown = |name: &str, files: &[(String, Vec<u8>)]| pages(&write(tmp("epub-contents", name), &zipped(files, Mimetype::Stored)), &[]);
    let with = |more: &[(&'static str, Vec<u8>)]| [pictures.clone(), more.to_vec()].concat();
    // The cover, the Contents page, then the three chapters.
    let book = book_with(&chapters, &with(&[("OEBPS/nav.xhtml", nav(THREE_ENTRIES))]), "", &format!("{cover}{NAV_ITEM}"));
    let (pages, said) = shown("book.epub", &book);
    assert_eq!((pages.len(), said.as_str()), (5, ""));
    assert!(yellow(&pages[0]) && !yellow(&pages[1]) && colours(&pages[1..2]) == [[false; 3]], "the Contents page, of text alone");
    assert_eq!(colours(&pages[2..]), IN_ORDER);
    // The nav document in the reading order is the book's own: a chapter, and no page of clv's.
    let mut own = book.clone();
    let package = own.iter_mut().find(|(name, _)| name == "OEBPS/content.opf").unwrap();
    package.1 = String::from_utf8(package.1.clone()).unwrap().replace("<spine>", r#"<spine><itemref idref="nav"/>"#).into_bytes();
    let (pages, said) = shown("own.epub", &own);
    assert_eq!((pages.len(), said.as_str()), (5, ""), "the cover, the book's own contents, its chapters");
    // An NCX nested 100,000 deep: past what is read of one, so never given to the worker.
    let point = r#"<navPoint><navLabel><text>x</text></navLabel><content src="Text/ch1.xhtml"/>"#;
    let deep = format!(
        r#"<?xml version="1.0"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><navMap>{}{}</navMap></ncx>"#,
        point.repeat(100_000),
        "</navPoint>".repeat(100_000)
    );
    let ncx = r#"<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>"#;
    let nested = book_with(&chapters, &with(&[("OEBPS/toc.ncx", deep.into_bytes())]), "", &format!("{cover}{ncx}"));
    let (pages, said) = shown("nested.epub", &nested);
    assert_eq!((pages.len(), said.as_str()), (4, "warning: page 1: its table of contents couldn't be read: it has no Contents page\n"));
    assert_eq!(colours(&pages[1..]), IN_ORDER, "the book is whole");
    // A nav nested 20,000 deep, within what is read: rbook reads it, and the page is made.
    let list = format!("{}{}", r#"<ol><li><a href="Text/ch1.xhtml">x</a>"#.repeat(20_000), "</li></ol>".repeat(20_000));
    let within = book_with(&chapters, &with(&[("OEBPS/nav.xhtml", nav(&list))]), "", &format!("{cover}{NAV_ITEM}"));
    assert!(within.iter().any(|(name, bytes)| name == "OEBPS/nav.xhtml" && bytes.len() < 1 << 20));
    let (pages, said) = shown("within.epub", &within);
    assert!(said.contains("its Contents page has the first 4096 entries") && pages.len() > 5 && yellow(&pages[0]), "{said}: {} pages", pages.len());
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
