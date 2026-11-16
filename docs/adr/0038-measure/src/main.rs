//! What a book's package costs rbook to read, as clv's `epub` worker reads it: time, and heap as
//! the allocator counts it, which is what the worker's cap counts. For ADR-0038.
//!
//! `measure read <x.epub>…` reads each book's package from a zip holding only `mimetype`,
//! `META-INF/container.xml` and the OPF, without its table of contents (`skip_toc`), and prints a
//! line: the OPF's size, the spine's and the manifest's lengths, the cover, the time and the most
//! heap held over what was held before. With `--toc`, the table of contents too, from a zip that
//! also holds the nav document or the NCX: its entries, and its depth.
//!
//! `measure gen <kind> <n> <out.epub>` writes a hostile book, synthetic, of one chapter: `items`
//! (a manifest of `n` items, and a spine of as many, up to 4,096), `refines` (a chain of `n`
//! metadata entries, each refining the one before), `refined` (`n` entries refining one), `metas`
//! (`n` empty `<meta/>` elements, the costliest a byte) and `font` (a manifest item whose media
//! type has a character past ASCII, `n` ignored); and, for the table of contents, `ncx` (an EPUB 2
//! NCX nested `n` deep), `nav` (an EPUB 3 nav nested `n` deep) and `entries` (a nav of `n` entries
//! side by side).

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{Cursor, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use rbook::Epub;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// The system's allocator, counting what is held and the most that was.
struct Counted;

static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let now = NOW.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(now, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        NOW.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static HEAP: Counted = Counted;

/// A zip of `files`, by name and contents, stored as they are.
fn zipped(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, contents) in files {
        zip.start_file(*name, stored).unwrap();
        zip.write_all(contents).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// The file called `name` in the zip `book`.
fn file(book: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    book.by_name(name).ok()?.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// The value of the first `name="…"` in `xml`.
fn attribute(xml: &str, name: &str) -> Option<String> {
    let at = xml.find(&format!("{name}=\""))? + name.len() + 2;
    Some(xml[at..].split('"').next()?.to_owned())
}

fn read(path: &str, toc: bool) {
    let mut book = ZipArchive::new(Cursor::new(std::fs::read(path).unwrap())).unwrap();
    let container = file(&mut book, "META-INF/container.xml").expect("container.xml");
    let opf_path = attribute(&String::from_utf8_lossy(&container), "full-path").expect("full-path");
    let opf = file(&mut book, &opf_path).expect("the OPF");
    let mut files: Vec<(String, Vec<u8>)> =
        vec![("mimetype".into(), b"application/epub+zip".to_vec()), ("META-INF/container.xml".into(), container), (opf_path.clone(), opf.clone())];
    // First without the table of contents, to learn which file holds it.
    let package = zipped(&files.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect::<Vec<_>>());
    let before = NOW.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let started = Instant::now();
    let epub = match Epub::options().strict(false).skip_toc(true).read(Cursor::new(package)) {
        Ok(epub) => epub,
        Err(e) => return println!("{path}: refused: {e}"),
    };
    let (took, heap) = (started.elapsed(), PEAK.load(Ordering::Relaxed) - before);
    let manifest = epub.manifest();
    let nav = manifest.by_property("nav").next().map(|entry| entry.href().decode().into_owned());
    let ncx = manifest.iter().find(|entry| entry.media_type() == "application/x-dtbncx+xml").map(|entry| entry.href().decode().into_owned());
    let cover = manifest.cover_image().map(|entry| entry.href().decode().into_owned());
    let linear = epub.spine().iter().filter(|entry| entry.is_linear()).count();
    let first = epub.spine().iter().next().and_then(|entry| entry.manifest_entry()).map(|entry| (entry.href().decode().into_owned(), entry.href_raw().decode().into_owned(), entry.media_type().to_owned()));
    println!(
        "{path}: epub {} opf {} bytes, spine {} ({linear} linear), manifest {}, cover {cover:?}, nav {nav:?}, ncx {ncx:?}, first {first:?}, {:.2} ms, {} KiB",
        epub.package().version_str(),
        opf.len(),
        epub.spine().len(),
        manifest.len(),
        took.as_secs_f64() * 1e3,
        heap >> 10,
    );
    if !toc {
        return;
    }
    for held in [nav, ncx].into_iter().flatten() {
        let name = held.trim_start_matches('/').to_owned();
        if let Some(bytes) = file(&mut book, &name) {
            files.push((name, bytes));
        }
    }
    drop(epub);
    let package = zipped(&files.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect::<Vec<_>>());
    let before = NOW.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let started = Instant::now();
    let epub = match Epub::options().strict(false).read(Cursor::new(package)) {
        Ok(epub) => epub,
        Err(e) => return println!("  its table of contents: refused: {e}"),
    };
    let (took, heap) = (started.elapsed(), PEAK.load(Ordering::Relaxed) - before);
    let toc = epub.toc();
    let Some(contents) = toc.contents() else { return println!("  its table of contents: none") };
    // Walked with a stack of its own, as the worker does.
    let (mut entries, mut deepest, mut stack) = (0usize, 0usize, vec![contents.iter()]);
    let mut shown = Vec::new();
    while let Some(level) = stack.last_mut() {
        let Some(entry) = level.next() else {
            stack.pop();
            continue;
        };
        entries += 1;
        deepest = deepest.max(stack.len());
        if shown.len() < 3 {
            shown.push((entry.label().to_owned(), entry.href().map(|href| href.decode().into_owned()), entry.href_raw().map(|href| href.decode().into_owned())));
        }
        stack.push(entry.iter());
    }
    let landmarks: Vec<_> = toc.landmarks().map(|root| root.iter().map(|e| (e.kind().as_str().to_owned(), e.href().map(|h| h.decode().into_owned()))).collect()).unwrap_or_default();
    println!("  its table of contents ({:?}): {entries} entries, {deepest} deep, {:.2} ms, {} KiB; {shown:?}; landmarks {landmarks:?}", contents.version(), took.as_secs_f64() * 1e3, heap >> 10);
}

/// A hostile book of `kind`, sized by `n`, written to `out`.
fn generated(kind: &str, n: usize, out: &str) {
    let mut metadata = String::from(r#"<dc:identifier id="id">x</dc:identifier><dc:title>t</dc:title><dc:language>en</dc:language>"#);
    let mut items = String::from(r#"<item id="one" href="one.xhtml" media-type="application/xhtml+xml"/>"#);
    let mut spine = String::from(r#"<itemref idref="one"/>"#);
    let mut toc: Option<(&str, String)> = None;
    match kind {
        "items" => {
            items = (0..n).map(|i| format!(r#"<item id="i{i}" href="c{i}.xhtml" media-type="application/xhtml+xml"/>"#)).collect();
            spine = (0..n.min(4096)).map(|i| format!(r#"<itemref idref="i{i}"/>"#)).collect();
        }
        "refines" => {
            metadata.push_str(r#"<meta id="m0" property="dcterms:modified">2020</meta>"#);
            metadata.extend((1..n).map(|i| format!(r##"<meta id="m{i}" refines="#m{}" property="p">v</meta>"##, i - 1)));
        }
        "refined" => {
            metadata.push_str(r#"<meta id="m0" property="dcterms:modified">2020</meta>"#);
            metadata.push_str(&r##"<meta refines="#m0" property="p">v</meta>"##.repeat(n));
        }
        "metas" => metadata.push_str(&"<meta/>".repeat(n)),
        "font" => items.push_str("<item id=\"f\" href=\"f.otf\" media-type=\"application/font\u{e8}\"/>"),
        "ncx" => {
            items.push_str(r#"<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>"#);
            spine = r#"<itemref idref="one"/>"#.to_owned();
            let points = format!(
                "{}{}",
                r#"<navPoint><navLabel><text>x</text></navLabel><content src="one.xhtml"/>"#.repeat(n),
                "</navPoint>".repeat(n)
            );
            toc = Some(("toc.ncx", format!(r#"<?xml version="1.0"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><navMap>{points}</navMap></ncx>"#)));
        }
        "nav" | "entries" => {
            items.push_str(r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#);
            let list = match kind {
                "nav" => format!("{}{}", r#"<ol><li><a href="one.xhtml">x</a>"#.repeat(n), "</li></ol>".repeat(n)),
                _ => format!("<ol>{}</ol>", r#"<li><a href="one.xhtml">x</a></li>"#.repeat(n)),
            };
            toc = Some((
                "nav.xhtml",
                format!(r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc">{list}</nav></body></html>"#),
            ));
        }
        _ => return eprintln!("no such kind: {kind}"),
    }
    if kind == "ncx" {
        spine = String::new();
    }
    // An NCX is the spine's to name.
    let (version, spine) = match kind {
        "ncx" => ("2.0", r#"<spine toc="ncx"><itemref idref="one"/></spine>"#.to_owned()),
        _ => ("3.0", format!("<spine>{spine}</spine>")),
    };
    let opf = format!(
        r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="{version}" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/">{metadata}</metadata><manifest>{items}</manifest>{spine}</package>"#
    );
    let container = r#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
    let mut files: Vec<(&str, &[u8])> = vec![
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("content.opf", opf.as_bytes()),
        ("one.xhtml", b"<html><body><p>one</p></body></html>"),
    ];
    if let Some((name, contents)) = &toc {
        files.push((name, contents.as_bytes()));
        println!("{out}: a table of contents of {} bytes", contents.len());
    }
    let book = zipped(&files);
    std::fs::write(out, book).unwrap();
    println!("{out}: an OPF of {} bytes", opf.len());
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("read") => {
            let toc = args.iter().any(|arg| arg == "--toc");
            for path in args.iter().skip(1).filter(|arg| *arg != "--toc") {
                read(path, toc);
            }
        }
        Some("gen") => match (args.get(1), args.get(2).and_then(|n| n.parse().ok()), args.get(3)) {
            (Some(kind), Some(n), Some(out)) => generated(kind, n, out),
            _ => eprintln!("measure gen <items|refines|refined|metas|font|ncx|nav|entries> <n> <out.epub>"),
        },
        _ => eprintln!("measure read [--toc] <x.epub>… | measure gen <kind> <n> <out.epub>"),
    }
}
