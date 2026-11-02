//! Word documents (docx, doc), OpenDocument text, RTF, Markdown and HTML, read, laid out into pages
//! and drawn by wordcraft, as its app prints them: headers and footers at full strength, no comment
//! balloons, no proofing marks. A password opens a protected Word document.
//!
//! Opening reads and lays out the document once, to know it opens and its pages' sizes, and keeps
//! only where its bytes are: a renderer, or a pool, reads and lays it out again on its first
//! render and keeps that for the renders after, until it goes, as a drawing keeps its tree.
//!
//! A docx and an odt are zips: this backend is asked before the zip backend, and knows one by the
//! names of the files that start it, or a Word document by its own name.
//!
//! Markdown is read by pulldown-cmark, into the HTML wordcraft's importer reads, the one that can be
//! given pictures. The pictures both refer to are drawn by the backends that show them, through
//! `source/picture.rs`, when a page they are on first renders.

use std::borrow::Cow;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, PoisonError, RwLock};

use pulldown_cmark::{Event, Options, Parser};
use wordcraft_doc::para::{InlineObject, Wrap};
use wordcraft_doc::{Block, StoryRef};
use wordcraft_layout::{DocLayout, LayoutCache, LayoutOptions, Page, Placed, ViewMode};
use wordcraft_render::RenderOptions;

use super::head::{COMPOUND, Names, ZIP, mimetype};
use super::picture::{Pictures, Promised};
use super::threads::Threads;
use super::{Again, Backend, Format, Frame, Input, Opened, Pixels, Pool, Render, Reopenable, Request, Source, guarded};
use crate::args::{OptionSpec, Parsed};

/// Word documents, OpenDocument text, RTF, Markdown and HTML, laid out and drawn by wordcraft.
pub(super) struct Wordcraft;

const OPTIONS: &[OptionSpec] =
    &[OptionSpec { name: "--password", short: None, value: Some("PW"), help: "the password of a protected Word document" }];

impl Backend for Wordcraft {
    fn name(&self) -> &'static str {
        "wordcraft"
    }

    fn shows(&self) -> &'static str {
        "documents"
    }

    fn formats(&self) -> Vec<Format> {
        vec![WORD, ODT, RTF, MARKDOWN, HTML]
    }

    fn credits(&self) -> &[&'static str] {
        &["wordcraft", "pulldown-cmark"]
    }

    fn options(&self) -> &'static [OptionSpec] {
        OPTIONS
    }

    fn opens(&self, path: &str, head: &[u8]) -> bool {
        Kind::of(path, head).is_some()
    }

    fn open(&self, input: Input, options: &Parsed<'_>) -> Result<Opened, String> {
        let path = input.path.clone();
        let error = |e: String| format!("{path}: {e}");
        let kind = Kind::of(&path, &input.head).ok_or_else(|| error("not a document this reads".into()))?;
        let password = options.value("--password").map(String::from);
        let read_as = |bytes: &[u8], beside: Option<&Path>| caught(|| laid_out(bytes, kind, password.as_deref(), &path, beside));
        let (stored, laid) = match input.reopenable()? {
            Reopenable::Disk(file) => {
                let bytes = read(file).map_err(error)?;
                (Stored::Disk(PathBuf::from(&path)), read_as(&bytes, Path::new(&path).parent()))
            }
            Reopenable::Bytes(bytes) => {
                let bytes = within(bytes).map_err(error)?;
                let laid = read_as(&bytes, None);
                (Stored::Memory(bytes), laid)
            }
            Reopenable::Again(again) => {
                let bytes = again().and_then(within).map_err(error)?;
                (Stored::Held(again), read_as(&bytes, None))
            }
        };
        // Only the pages' sizes are kept: the document goes, and a render reads it again.
        let sizes = laid.map_err(error)?.layout.pages.iter().map(|page| (page.w, page.h)).collect();
        Ok(Opened::Pages(Box::new(Paged(Arc::new(Document { stored, kind, password, path, sizes })))))
    }
}

/// The largest file this reads: 256 MiB.
const MAX_FILE: u64 = 256 << 20;

/// The largest Markdown or HTML file this reads: 64 MiB. A page may hold far more than it lays out:
/// a single-file page's scripts, styles and pictures in `data:` URLs cost next to nothing.
const MAX_MARKED_UP: usize = 64 << 20;

/// The most marks a Markdown or HTML file may have ([`marked_up`]): what it costs goes by them, up
/// to 9 KB each between importing and laying out.
const MAX_MARKS: usize = 500_000;

/// The most text of a Markdown or HTML file that is laid out: 8 MiB, some 5,000 pages, which takes
/// 20 seconds to open and 1.6 GB.
const MAX_TEXT: usize = 8 << 20;

/// The most pixels a page is drawn with: 64 million, 256 MiB.
const MAX_PIXELS: u64 = 64_000_000;

/// A Word document, by what it is named where its first bytes can't tell: a document, a
/// template, with macros or without, in either format, since Word opens each by what it holds.
const WORD: Format = Format { name: "Word", extensions: &["docx", "docm", "dotx", "dotm", "doc", "dot"] };

/// OpenDocument text, by its name where its first file doesn't say: a document or a template.
const ODT: Format = Format { name: "OpenDocument text", extensions: &["odt", "ott"] };

/// RTF, by its first bytes alone.
const RTF: Format = Format { name: "RTF", extensions: &[] };

/// Markdown, by its name alone, since nothing in how it starts says so.
const MARKDOWN: Format = Format { name: "Markdown", extensions: &["md", "markdown", "mdown", "mkd"] };

/// HTML, by how it starts, or by its name where that doesn't say (a fragment of a page).
const HTML: Format = Format { name: "HTML", extensions: &["html", "htm", "xhtml"] };

/// What an OpenDocument text says it is, in its first file: a document or a template.
const ODT_TYPE: &[u8] = b"application/vnd.oasis.opendocument.text";

/// What kind of document a file is, which says how it is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A Word package (docx and the rest), a zip.
    Package,
    /// A compound file: a Word 97-2003 document, or a Word package protected with a password.
    Compound,
    /// OpenDocument text, a zip whose first file says so.
    Odt,
    Rtf,
    Markdown,
    Html,
}

impl Kind {
    /// The kind of the file at `path`, which starts with `head`, if this backend reads it. An RTF
    /// says so in its first bytes whatever its name (one is often saved as `.doc`), an odt in its
    /// first file. A Word package is a zip with a file in `word/` among the first, or named as a
    /// Word document; a compound file only when named so, since spreadsheets and slides are too.
    /// HTML says so by how it starts; and text may be Markdown or HTML when named so, which a PDF
    /// or a picture misnamed isn't.
    fn of(path: &str, head: &[u8]) -> Option<Self> {
        if head.starts_with(b"{\\rtf") {
            return Some(Self::Rtf);
        }
        if head.starts_with(ZIP) {
            if mimetype(head).is_some_and(|m| m.starts_with(ODT_TYPE)) || ODT.named(path) {
                return Some(Self::Odt);
            }
            let word = Names(head).any(|name| name.starts_with(b"word/"));
            return (word || WORD.named(path)).then_some(Self::Package);
        }
        if head.starts_with(COMPOUND) {
            return WORD.named(path).then_some(Self::Compound);
        }
        if starts_as_html(head) {
            return Some(Self::Html);
        }
        if !wordcraft_formats::txt::is_text(head) {
            return None;
        }
        MARKDOWN.named(path).then_some(Self::Markdown).or(HTML.named(path).then_some(Self::Html))
    }
}

/// Whether a file starting with `head` says it is HTML: a doctype or an `html` element, in any case,
/// after whatever may come first (a byte order mark, blank space, comments, an XML declaration).
fn starts_as_html(head: &[u8]) -> bool {
    let mut rest = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head).trim_ascii_start();
    loop {
        let end: &[u8] = match rest {
            [b'<', b'!', b'-', b'-', ..] => b"-->",
            [b'<', b'?', ..] => b"?>",
            _ => break,
        };
        let Some(at) = rest.windows(end.len()).position(|bytes| bytes == end) else { return false };
        rest = rest.get(at.saturating_add(end.len())..).unwrap_or_default().trim_ascii_start();
    }
    [&b"<!doctype html"[..], b"<html"].iter().any(|tag| {
        let named = rest.get(..tag.len()).is_some_and(|start| start.eq_ignore_ascii_case(tag));
        named && rest.get(tag.len()).is_none_or(|next| next.is_ascii_whitespace() || *next == b'>')
    })
}

/// An open document.
struct Paged(Arc<Document>);

struct Document {
    stored: Stored,
    kind: Kind,
    password: Option<String>,
    /// As given, for the fields that show the file's name.
    path: String,
    /// Each page's, in points.
    sizes: Vec<(f32, f32)>,
}

/// Where a document's bytes are.
enum Stored {
    /// In a file on disk, read again to render.
    Disk(PathBuf),
    /// As they came through a pipe, which can't be read again.
    Memory(Vec<u8>),
    /// In another file (an archive), read from it again to render.
    Held(Again),
}

/// A document read and laid out.
struct Laid {
    /// Written to only to hold a picture a page wants, or let go of one: pages render side by side.
    document: RwLock<wordcraft_doc::Document>,
    layout: DocLayout,
    pictures: Promised,
}

impl Laid {
    /// `page`'s pictures drawn, for it to render, and what the reader should know of them.
    fn drawn_pictures(&self, page: &Page, first: bool) -> Vec<String> {
        let mut notes = Vec::new();
        if first && self.pictures.left_out() > 0 {
            notes.push(format!("{} pictures it refers to are left out: missing, or not what {} shows", self.pictures.left_out(), crate::NAME));
        }
        if self.pictures.is_empty() {
            return notes;
        }
        let (changes, failed) = {
            let document = self.document.read().unwrap_or_else(PoisonError::into_inner);
            self.pictures.draw(&pictures_on(&document, page))
        };
        if failed {
            notes.push("a picture it refers to could not be drawn".into());
        }
        if !changes.is_empty() {
            self.document.write().unwrap_or_else(PoisonError::into_inner).media.extend(changes);
        }
        notes
    }
}

/// What `document` holds for the pictures on `page`, by key: of each paragraph on it, even one that
/// goes on to another page.
fn pictures_on<'a>(document: &'a wordcraft_doc::Document, page: &Page) -> Vec<&'a str> {
    let paragraph = |item: &Placed| match item {
        Placed::Lines { story, path, .. } => document.para(*story, path),
        _ => None,
    };
    let picture = |object: &'a InlineObject| match object {
        InlineObject::Image { media, .. } => Some(media.as_str()),
        _ => None,
    };
    page.items.iter().filter_map(paragraph).flat_map(|para| para.objects.iter()).filter_map(picture).collect()
}

/// A document laid out, by the first render of a renderer or a pool, each of which has its own:
/// an error, or a panic's, is kept too, so a document that fails isn't read again for every
/// render.
type Cache = OnceLock<Result<Arc<Laid>, String>>;

impl Source for Paged {
    fn pages(&self) -> usize {
        self.0.sizes.len()
    }

    fn size(&self, page: usize) -> Option<(f32, f32)> {
        self.0.sizes.get(page).copied()
    }

    /// A page scales: fitted to the window, it fills it, as a PDF's does.
    fn natural(&self, _page: usize) -> bool {
        false
    }

    fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
        Ok(Box::new(Renderer { document: self.0.clone(), laid: Cache::new() }))
    }

    fn pool(&self, threads: usize) -> Box<dyn Pool> {
        let (document, laid) = (self.0.clone(), Arc::new(Cache::new()));
        Box::new(Threads::new(threads, Arc::new(move |request| document.render(request, &laid))))
    }
}

struct Renderer {
    document: Arc<Document>,
    laid: Cache,
}

impl Render for Renderer {
    fn render(&mut self, request: Request) -> Frame {
        guarded(request, || self.document.render(request, &self.laid))
    }
}

impl Document {
    fn render(&self, request: Request, laid: &Cache) -> Frame {
        self.rendered(request, laid).unwrap_or_else(|e| Frame::failed(request, e))
    }

    fn rendered(&self, request: Request, laid: &Cache) -> Result<Frame, String> {
        let laid = laid
            .get_or_init(|| {
                let bytes = self.bytes()?;
                caught(|| laid_out(&bytes, self.kind, self.password.as_deref(), &self.path, self.beside())).map(Arc::new)
            })
            .clone()?;
        let page = laid.layout.pages.get(request.page).ok_or_else(|| format!("the document has no page {}", request.page.saturating_add(1)))?;
        let (w, h) = (pixels(page.w, request.scale)?, pixels(page.h, request.scale)?);
        let side = u64::from(wordcraft_render::MAX_SIDE);
        if w > side || h > side || w.saturating_mul(h) > MAX_PIXELS {
            return Err(format!("{w} × {h} pixels is too large to render"));
        }
        let warnings = laid.drawn_pictures(page, request.page == 0);
        let mut options = RenderOptions::default();
        // A printed page: headers and footers dimmed only while the body is being edited.
        options.display.dim_header = false;
        let document = laid.document.read().unwrap_or_else(PoisonError::into_inner);
        let drawn = wordcraft_render::render_page(&document, page, request.scale, &options);
        // Premultiplied RGBA, row by row, as a frame's are.
        Ok(Frame { request, width: drawn.width, height: drawn.height, rgba: Pixels::new(drawn.pixels), error: None, warnings })
    }

    /// The directory it is in, if it is a file on disk: what the pictures it refers to are near.
    fn beside(&self) -> Option<&Path> {
        match &self.stored {
            Stored::Disk(path) => path.parent(),
            Stored::Memory(_) | Stored::Held(_) => None,
        }
    }

    /// Its bytes: the ones kept, or the file's again.
    fn bytes(&self) -> Result<Cow<'_, [u8]>, String> {
        let again = match &self.stored {
            Stored::Memory(bytes) => return Ok(Cow::Borrowed(&bytes[..])),
            Stored::Disk(path) => File::open(path).map_err(|e| e.to_string()).and_then(read),
            Stored::Held(again) => again().and_then(within),
        };
        again.map(Cow::Owned).map_err(|e| format!("reading the file again: {e}"))
    }
}

/// `size` points at `scale`, in whole pixels, as wordcraft rounds them.
fn pixels(size: f32, scale: f32) -> Result<u64, String> {
    let scaled = (f64::from(size) * f64::from(scale)).ceil();
    if !(scale > 0.0 && scaled.is_finite() && scaled <= 1e15) {
        return Err(format!("can't render at scale {scale}"));
    }
    // In range: checked just above.
    Ok((scaled as u64).max(1))
}

/// The document in `bytes`, of `kind`, read with `password` if it is protected, and laid out into
/// pages. `path` is the file's, for the fields that show it, and `beside` the directory it is in, for
/// the pictures it refers to.
fn laid_out(bytes: &[u8], kind: Kind, password: Option<&str>, path: &str, beside: Option<&Path>) -> Result<Laid, String> {
    let pictures = Pictures::beside(beside);
    let read = match kind {
        Kind::Package => wordcraft_docx::read_with_password(bytes, password).map_err(|e| e.to_string()),
        Kind::Compound if wordcraft_docx::is_encrypted(bytes) => wordcraft_docx::read_with_password(bytes, password).map_err(|e| e.to_string()),
        Kind::Compound => wordcraft_docbin::read_with_password(bytes, password).map_err(|e| e.to_string()),
        Kind::Odt => imported("odt", bytes),
        Kind::Rtf => imported("rtf", bytes),
        Kind::Markdown => markdown(bytes, &pictures),
        Kind::Html => html(bytes, &pictures),
    };
    let mut document = read.map_err(|e| match e == wordcraft_docx::PASSWORD_REQUIRED {
        true => format!("{e}: give it with --password"),
        false => e,
    })?;
    document.ensure_nonempty();
    let name = Path::new(path).file_name().map_or_else(|| path.into(), |name| name.to_string_lossy());
    let options = LayoutOptions { view: ViewMode::Print, filename: Some(Arc::from(&*name)), filepath: Some(Arc::from(path)), ..Default::default() };
    let layout = wordcraft_layout::layout(&document, &mut LayoutCache::new(), &options);
    if layout.pages.is_empty() {
        return Err("the document has no pages to show".into());
    }
    let pictures = pictures.promised_in(&document.media);
    Ok(Laid { document: RwLock::new(document), layout, pictures })
}

/// The document in `bytes`, in the format `extension` names, through wordcraft's importers.
fn imported(extension: &str, bytes: &[u8]) -> Result<wordcraft_doc::Document, String> {
    let page = wordcraft_doc::SectionProps::default();
    wordcraft_formats::import_on(extension, bytes, &page).unwrap_or_else(|| Err(format!("wordcraft reads no .{extension}")))
}

/// The Markdown in `bytes`, where any punctuation or line may start a run, a cell or a paragraph.
fn markdown(bytes: &[u8], pictures: &Pictures<'_>) -> Result<wordcraft_doc::Document, String> {
    let as_html = |bytes: &[u8]| markdown_as_html(&wordcraft_formats::txt::decode(bytes));
    marked_up(|b| b.is_ascii_punctuation() || matches!(b, b'\r' | b'\n'), bytes, as_html, pictures)
}

/// The HTML in `bytes`, where a tag or, in a `pre`, a line may.
fn html(bytes: &[u8], pictures: &Pictures<'_>) -> Result<wordcraft_doc::Document, String> {
    marked_up(|b| matches!(b, b'<' | b'\r' | b'\n'), bytes, wordcraft_formats::html::decode, pictures)
}

/// `text`, Markdown, as HTML for wordcraft's importer to read
/// ([ADR-0014](../../docs/adr/0014-markdown-through-pulldown-cmark-into-wordcrafts-html-importer.md)):
/// GitHub's tables, strikethrough, task lists and footnotes, with what it would leave out written
/// otherwise, a table's grid and a task's checkbox, and front matter (`---`) left out.
fn markdown_as_html(text: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS;
    let events = Parser::new_ext(text, options).map(|event| match event {
        Event::TaskListMarker(true) => Event::Text("☑ ".into()),
        Event::TaskListMarker(false) => Event::Text("☐ ".into()),
        other => other,
    });
    let mut out = String::with_capacity(text.len());
    pulldown_cmark::html::push_html(&mut out, events);
    out.replace("<table>", "<table border=\"1\">")
}

/// The Markdown or HTML in `bytes`, unless it costs too much to read and lay out
/// ([ADR-0012](../../docs/adr/0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md)).
/// `mark` says which bytes may start something of its own, counted before it is imported, since
/// importing them is what costs; its text, what laying it out costs, can only be told after, since
/// a page's scripts and styles aren't text.
fn marked_up(
    mark: impl Fn(&u8) -> bool,
    bytes: &[u8],
    as_html: impl FnOnce(&[u8]) -> String,
    pictures: &Pictures<'_>,
) -> Result<wordcraft_doc::Document, String> {
    if bytes.len() > MAX_MARKED_UP {
        return Err(format!("the document is more than {} MiB", MAX_MARKED_UP >> 20));
    }
    if bytes.iter().filter(|b| mark(b)).count() > MAX_MARKS {
        return Err(format!("the document has more than {MAX_MARKS} lines and marks"));
    }
    let page = wordcraft_doc::SectionProps::default();
    let fit = (page.text_width(), page.text_height());
    let mut document = wordcraft_formats::html::import_on(&as_html(bytes), &|source: &str| pictures.measured(source, fit), &page);
    fit_to_the_page(&mut document.body, fit.1);
    let body = StoryRef::Body;
    let paragraphs = document.para_paths(body);
    let text = paragraphs.iter().filter_map(|path| document.para(body, path)).fold(0, |all: usize, para| all.saturating_add(para.text.len()));
    if text > MAX_TEXT {
        return Err(format!("the document has more than {} MiB of text", MAX_TEXT >> 20));
    }
    Ok(document)
}

/// Pictures taller than `height` shrunk to it, keeping their shape: laid out as they are, they would
/// be cut off at the foot of the page. The width of one is already within the column, and the size of a
/// picture referred to within both: this is for those a document gives a size to, or holds itself.
fn fit_to_the_page(blocks: &mut [Arc<Block>], height: f32) {
    for block in blocks {
        match Arc::make_mut(block) {
            Block::Para(para) => {
                let mut changed = false;
                for object in &mut para.objects {
                    if let InlineObject::Image { w, h, float, .. } = object
                        && float.wrap == Wrap::Inline
                        && h.is_finite()
                        && *h > height
                    {
                        *w = (*w * height / *h).max(1.0);
                        *h = height;
                        changed = true;
                    }
                }
                if changed {
                    para.touch();
                }
            }
            Block::Table(table) => {
                for cell in table.rows.iter_mut().flat_map(|row| row.cells.iter_mut()) {
                    fit_to_the_page(&mut cell.blocks, height);
                }
            }
        }
    }
}

/// What `read` returns, or an error if wordcraft panics: its bug costs the document, not the
/// viewer.
fn caught<T>(read: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|_| Err("wordcraft failed on this document".into()))
}

/// All of `file`, as long as it is no larger than [`MAX_FILE`].
fn read(file: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    within(bytes)
}

/// `bytes`, as long as they are no more than [`MAX_FILE`].
fn within(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    match u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE {
        true => Err(format!("the document is more than {} MiB", MAX_FILE >> 20)),
        false => Ok(bytes),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::super::head::zip_head;
    use super::*;

    const COMPOUND_HEAD: [u8; 16] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0, 0, 0, 0, 0, 0, 0];

    #[test]
    fn an_rtf_is_known_by_its_first_bytes_whatever_its_name() {
        for path in ["notes.rtf", "report.doc", "untitled"] {
            assert_eq!(Kind::of(path, b"{\\rtf1\\ansi Hello}"), Some(Kind::Rtf), "{path}");
        }
        assert_eq!(Kind::of("notes.rtf", b"plain text"), None, "named so, it isn't one");
    }

    #[test]
    fn a_word_package_is_a_zip_with_word_files_first_or_named_so() {
        let package =
            zip_head(&[("[Content_Types].xml", b"<Types/>"), ("_rels/.rels", b"<Relationships/>"), ("word/document.xml", b"<w:document/>")]);
        for path in ["a.docx", "a.zip", "a"] {
            assert_eq!(Kind::of(path, &package), Some(Kind::Package), "{path}");
        }
        let other = zip_head(&[("notes.txt", b"text")]);
        for path in ["a.docx", "a.DOCM", "a.dotx", "a.dotm", "a.doc", "a.dot"] {
            assert_eq!(Kind::of(path, &other), Some(Kind::Package), "{path}: named as one");
        }
        for path in ["a.zip", "a.xlsx", "a.epub", "docx"] {
            assert_eq!(Kind::of(path, &other), None, "{path}");
        }
    }

    #[test]
    fn an_odt_is_a_zip_whose_first_file_says_so() {
        let odt = zip_head(&[("mimetype", ODT_TYPE), ("content.xml", b"<office:document-content/>")]);
        assert_eq!((Kind::of("a.odt", &odt), Kind::of("a.zip", &odt)), (Some(Kind::Odt), Some(Kind::Odt)));
        let template = zip_head(&[("mimetype", b"application/vnd.oasis.opendocument.text-template")]);
        assert_eq!(Kind::of("a.ott", &template), Some(Kind::Odt));
        let sheet = zip_head(&[("mimetype", b"application/vnd.oasis.opendocument.spreadsheet")]);
        assert_eq!(Kind::of("a.ods", &sheet), None);
        let epub = zip_head(&[("mimetype", b"application/epub+zip")]);
        assert_eq!(Kind::of("a.epub", &epub), None);
    }

    #[test]
    fn a_compound_file_is_a_word_document_only_when_named_so() {
        for path in ["a.doc", "a.DOT", "a.docx"] {
            assert_eq!(Kind::of(path, &COMPOUND_HEAD), Some(Kind::Compound), "{path}");
        }
        for path in ["a.xls", "a.ppt", "a.msg", "a"] {
            assert_eq!(Kind::of(path, &COMPOUND_HEAD), None, "{path}");
        }
    }

    #[test]
    fn markdown_is_text_named_so() {
        for path in ["a.md", "a.MD", "a.markdown", "a.mdown", "a.mkd"] {
            assert_eq!(Kind::of(path, b"# Title\n"), Some(Kind::Markdown), "{path}");
        }
        assert_eq!(Kind::of("empty.md", b""), Some(Kind::Markdown));
        assert_eq!(Kind::of("a.md", &[0xFF, 0xFE, b'#', 0, b' ', 0]), Some(Kind::Markdown), "UTF-16, by its byte order mark");
        for path in ["a.txt", "md", "a.md.txt"] {
            assert_eq!(Kind::of(path, b"# Title\n"), None, "{path}");
        }
    }

    /// A PDF, a picture or a zip given a text name is what it is, for the backend that shows it.
    #[test]
    fn what_is_not_text_is_not_markdown_or_html_by_its_name() {
        for path in ["a.md", "a.html"] {
            for head in [&b"%PDF-1.7\n"[..], b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR", b"PK\x03\x04", &COMPOUND_HEAD] {
                assert_eq!(Kind::of(path, head), None, "{path}: {head:?}");
            }
        }
    }

    #[test]
    fn html_is_known_by_how_it_starts_whatever_its_name() {
        for head in [
            &b"<!DOCTYPE html>"[..],
            b"<!doctype HTML PUBLIC \"-//W3C//DTD HTML 4.01//EN\">",
            b"<html lang=en>",
            b"<HTML>",
            b"<html",
            b"\xEF\xBB\xBF\n  <html>",
            b"<?xml version=\"1.0\"?>\n<!-- made by hand -->\n<!DOCTYPE html>",
        ] {
            for path in ["a.html", "a.txt", "untitled"] {
                assert_eq!(Kind::of(path, head), Some(Kind::Html), "{path}: {head:?}");
            }
        }
        for head in [&b"<htmlish>"[..], b"<!doctype xml>", b"<!-- never ends <html>", b"<p>a fragment</p>", b"<svg/>", b"text <html>"] {
            assert_eq!(Kind::of("untitled", head), None, "{head:?}");
        }
    }

    #[test]
    fn html_that_doesnt_say_so_is_text_named_so() {
        for path in ["a.html", "a.HTM", "a.xhtml"] {
            assert_eq!(Kind::of(path, b"<p>a fragment</p>"), Some(Kind::Html), "{path}");
        }
    }

    /// What a limit is for: the marks and the text of what is read, not the bytes of the file.
    #[test]
    fn markdown_and_html_are_refused_past_their_limits() {
        let nowhere = Pictures::beside(None);
        let markdown = |bytes: &[u8]| markdown(bytes, &nowhere);
        let html = |bytes: &[u8]| html(bytes, &nowhere);
        let refused = |what: &str, read: &dyn Fn(&[u8]) -> Result<wordcraft_doc::Document, String>, bytes: &[u8]| {
            assert!(read(bytes).is_err_and(|e| e.contains(what)), "{what}");
        };
        refused("lines and marks", &markdown, "*a".repeat(MAX_MARKS + 1).as_bytes());
        refused("lines and marks", &markdown, "\r".repeat(MAX_MARKS + 1).as_bytes());
        refused("lines and marks", &html, "<b>".repeat(MAX_MARKS + 1).as_bytes());
        refused("lines and marks", &html, format!("<pre>{}", "\n".repeat(MAX_MARKS + 1)).as_bytes());

        let text = "a".repeat(MAX_TEXT + 1);
        refused("of text", &markdown, text.as_bytes());
        refused("of text", &html, format!("<p>{text}</p>").as_bytes());
        assert!(markdown("a".repeat(MAX_TEXT).as_bytes()).is_ok(), "as much text as it may have");

        let hidden = format!("<script>{text}</script><style>{text}</style><img src=\"data:image/png;base64,{text}\"><p>Shown</p>");
        assert!(html(hidden.as_bytes()).is_ok(), "a page's text is what counts, not its scripts, styles and pictures");
        refused(" MiB", &html, format!("<script>{}</script>", "a".repeat(MAX_MARKED_UP)).as_bytes());
    }

    /// The pager's way: pages rendering side by side on a pool's threads, each drawing its pictures
    /// into what they share, and every page showing its own, however they fall out.
    #[cfg(feature = "image")]
    #[test]
    fn a_pools_threads_draw_their_pages_pictures() {
        let dir = std::env::temp_dir().join(format!("clview-word-pool-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut document = String::new();
        let pages = 6;
        for n in 0..pages {
            // A picture of its own on each page, and one they all share.
            let own = 40u8.saturating_mul(n + 1);
            let pixels: Vec<u8> = [own, 0, 0, 255].repeat(400);
            std::fs::write(dir.join(format!("{n}.png")), crate::encode::encode_png(20, 20, &pixels, crate::encode::PngEffort::Fast).unwrap())
                .unwrap();
            document.push_str(&format!("<p style=\"page-break-after:always\"><img src=\"{n}.png\"> <img src=\"0.png\"></p>"));
        }
        let path = dir.join("a.html");
        std::fs::write(&path, document).unwrap();

        let input = crate::source::Input::open(path.to_str().unwrap()).unwrap();
        let Ok(Opened::Pages(source)) = Wordcraft.open(input, &Parsed::default()) else { panic!("not pages") };
        assert!(source.pages() >= usize::from(pages));
        let pool = source.pool(4);
        let requests: Vec<Request> = (0..usize::from(pages)).map(|page| Request { page, scale: 1.0, tag: 0 }).collect();
        let started = std::time::Instant::now();
        let mut done = vec![false; requests.len()];
        while done.iter().any(|page| !page) {
            assert!(started.elapsed().as_secs() < 60, "still waiting for {done:?}");
            pool.set_queue(requests.iter().filter(|request| !done[request.page]).copied().collect());
            let Some(frame) = pool.try_recv() else {
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            };
            assert_eq!(frame.error, None);
            let page = frame.request.page;
            let own = 40u8.saturating_mul(u8::try_from(page).unwrap() + 1);
            let shows = frame.rgba.as_chunks::<4>().0.iter().any(|pixel| pixel[0].abs_diff(own) < 3 && pixel[1] < 3 && pixel[2] < 3);
            assert!(shows, "page {page} doesn't show its picture");
            done[page] = true;
        }
    }

    #[test]
    fn a_page_is_drawn_with_its_size_at_the_scale_rounded_up() {
        assert_eq!(pixels(612.0, 1.0), Ok(612));
        assert_eq!(pixels(200.0, 1.005), Ok(201));
        assert_eq!(pixels(0.0, 2.0), Ok(1), "at least one pixel");
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(pixels(612.0, scale).is_err(), "{scale}");
        }
        assert!(pixels(f32::MAX, f32::MAX).is_err());
    }

    proptest! {
        /// Whatever a file starts with, and whatever its name, choosing never panics.
        #[test]
        fn any_first_bytes_are_safe(head in proptest::collection::vec(any::<u8>(), 0..1024), zip in any::<bool>(), name in "[a-z.]{0,8}") {
            let head = if zip { [ZIP, &head].concat() } else { head };
            let _ = Kind::of(&name, &head);
        }
    }
}
