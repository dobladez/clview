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

use pulldown_cmark::{BlockQuoteKind, Event, Options, Parser, Tag};
use wordcraft_doc::para::{InlineObject, Wrap};
use wordcraft_doc::props::{Border, BorderStyle, Borders, LineSpacing};
use wordcraft_doc::{Block, Rgb, StoryRef, TextColor};
use wordcraft_layout::{DocLayout, LayoutCache, LayoutOptions, Page, Placed, ViewMode};
use wordcraft_render::RenderOptions;

use super::head::{COMPOUND, Names, ZIP, mimetype};
use super::picture::{Pictures, Promised};
use super::threads::Threads;
use super::{Again, Backend, Deferred, Format, Frame, Input, Opened, Paper, Pixels, Pool, Render, Reopenable, Request, Source, guarded};
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

    /// It lays out the whole document to open it.
    fn deferred(&self, path: &str, head: &[u8]) -> Option<Deferred> {
        Some(Deferred { dark_form: matches!(Kind::of(path, head)?, Kind::Markdown | Kind::Html) })
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
/// a single-file page's scripts and pictures in `data:` URLs cost next to nothing, and what wordcraft
/// reads of its styles, it bounds ([ADR-0017](../../docs/adr/0017-htmls-style-rules-apply-as-far-as-wordcraft-reads-them.md)).
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
        let on_page: Vec<String> = {
            let document = self.document.read().unwrap_or_else(PoisonError::into_inner);
            pictures_on(&document, page).into_iter().map(str::to_owned).collect()
        };
        let on_page: Vec<&str> = on_page.iter().map(String::as_str).collect();
        // Nothing locked while they are drawn, which may take a while; then, under one lock, held and
        // in the document at once, or another page could find one drawn and render its stand-in.
        let drawn = self.pictures.draw(&on_page);
        if !drawn.is_empty() {
            let mut document = self.document.write().unwrap_or_else(PoisonError::into_inner);
            let changes = self.pictures.hold(drawn, &on_page);
            document.media.extend(changes);
        }
        if self.pictures.failed(&on_page) {
            notes.push("a picture it refers to could not be drawn".into());
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

    /// Markdown and HTML have no paper of their own, unlike what is laid out to be printed.
    fn dark_form(&self) -> bool {
        matches!(self.0.kind, Kind::Markdown | Kind::Html)
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
        let dark = match request.paper {
            Paper::Dark(paper) if matches!(self.kind, Kind::Markdown | Kind::Html) => Some(paper),
            _ => None,
        };
        if let Some(paper) = dark {
            options.dark = true;
            options.dark_paper = grey_of(paper);
        }
        let document = laid.document.read().unwrap_or_else(PoisonError::into_inner);
        let mut drawn = wordcraft_render::render_page(&document, page, request.scale, &options);
        if let Some(paper) = dark {
            on_paper(&mut drawn.pixels, paper);
        }
        // Premultiplied RGBA, row by row, as a frame's are.
        Ok(Frame { request, width: drawn.width, height: drawn.height, rgba: Pixels::new(drawn.pixels), error: None, warnings, again: false })
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

/// The grey as bright as `colour`: what wordcraft's dark page can be.
fn grey_of([r, g, b]: [u8; 3]) -> u8 {
    let luma = (2126 * u32::from(r) + 7152 * u32::from(g) + 722 * u32::from(b)) / 10000;
    u8::try_from(luma).unwrap_or(u8::MAX)
}

/// `pixels`, a dark page on the grey of `paper`, on `paper` itself, so the page is exactly the colour
/// behind it: each channel stretched to keep its white and meet that colour at the grey. A picture on
/// the page shifts with it, as little as the colour is from grey.
fn on_paper(pixels: &mut [u8], paper: [u8; 3]) {
    let grey = grey_of(paper);
    if paper == [grey; 3] {
        return;
    }
    let stretch = |to: u8| -> [u8; 256] {
        let (grey, to) = (u32::from(grey), u32::from(to));
        std::array::from_fn(|from| {
            // Within a byte: `from` is below 256, and each stays between its two ends.
            let from = from as u32;
            let at = if from <= grey { from * to / grey.max(1) } else { to + (from - grey) * (255 - to) / (255 - grey).max(1) };
            at.min(255) as u8
        })
    };
    let [r, g, b] = paper.map(stretch);
    for pixel in pixels.as_chunks_mut::<4>().0 {
        // Each channel a byte, so each index is within its table.
        *pixel = [r[usize::from(pixel[0])], g[usize::from(pixel[1])], b[usize::from(pixel[2])], pixel[3]];
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
/// GitHub's tables, strikethrough, task lists, footnotes and alerts, with what it would leave out
/// written otherwise, a table's grid, a task's checkbox and what an alert is, and front matter (`---`)
/// left out.
fn markdown_as_html(text: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS;
    let mut alert = None;
    let events = Parser::new_ext(text, options).flat_map(|event| {
        // What an alert is goes on the line above its first paragraph's text, or else alone.
        let (before, after) = match (alert.take(), &event) {
            (Some(kind), Event::Start(Tag::Paragraph)) => (None, Some(Event::InlineHtml(format!("{}<br>", alert_label(kind)).into()))),
            (Some(kind), _) => (Some(Event::Html(format!("<p>{}</p>", alert_label(kind)).into())), None),
            (None, _) => (None, None),
        };
        if let Event::Start(Tag::BlockQuote(Some(kind))) = &event {
            alert = Some(*kind);
        }
        let event = match event {
            Event::TaskListMarker(true) => Event::Text("☑ ".into()),
            Event::TaskListMarker(false) => Event::Text("☐ ".into()),
            other => other,
        };
        [before, Some(event), after].into_iter().flatten()
    });
    let mut out = String::with_capacity(text.len());
    pulldown_cmark::html::push_html(&mut out, events);
    numbers_in_their_footnotes(&out.replace("<table>", "<table border=\"1\">"))
}

/// What an alert (`> [!NOTE]`) says it is, and its colour.
fn alert(kind: BlockQuoteKind) -> (&'static str, Rgb) {
    match kind {
        BlockQuoteKind::Note => ("Note", LINK),
        BlockQuoteKind::Tip => ("Tip", Rgb(0x1A, 0x7F, 0x37)),
        BlockQuoteKind::Important => ("Important", Rgb(0x4F, 0x2D, 0x94)),
        BlockQuoteKind::Warning => ("Warning", Rgb(0x9A, 0x67, 0x00)),
        BlockQuoteKind::Caution => ("Caution", Rgb(0xA4, 0x0E, 0x26)),
    }
}

/// What an alert is, in its colour, for the top of its quote.
fn alert_label(kind: BlockQuoteKind) -> String {
    let (name, Rgb(r, g, b)) = alert(kind);
    format!("<strong style=\"color:#{r:02X}{g:02X}{b:02X}\">{name}</strong>")
}

/// `html` with each footnote's number in its first paragraph, in smaller text, rather than on a line of
/// its own above it. What is written by hand to look like one, but isn't, stays as it is. Only what comes
/// right after each number is looked at, so a file of them takes no longer than a file of anything.
fn numbers_in_their_footnotes(html: &str) -> String {
    const NUMBER: &str = "<sup class=\"footnote-definition-label\">";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some((before, after)) = rest.split_once(NUMBER) {
        out.push_str(before);
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        // ASCII digits, so that's a char boundary.
        let (number, text) = after.split_at(digits);
        match text.strip_prefix("</sup>\n<p>") {
            Some(text) => {
                out.push_str("<p style=\"font-size:9.5pt\"><sup>");
                out.push_str(number);
                out.push_str("</sup> ");
                rest = text;
            }
            None => {
                out.push_str(NUMBER);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
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
    // US Letter, as nothing says otherwise. Less above and below than as printed, for more on a screen.
    let page = wordcraft_doc::SectionProps { margin_top: 54.0, margin_bottom: 54.0, ..Default::default() };
    let fit = (page.text_width(), page.text_height());
    // As a fragment, which takes the document's own formatting, here `styled`'s. `import_on` reads a web
    // page, as Word opens one: Times New Roman, and no code style. Only the top and bottom differ from
    // the default page, and tables size to its width, so they can be set after
    // ([ADR-0017](../../docs/adr/0017-htmls-style-rules-apply-as-far-as-wordcraft-reads-them.md)).
    let mut document = wordcraft_formats::html::import_with(&as_html(bytes), &|source: &str| pictures.measured(source, fit));
    document.last_section = page;
    fit_to_the_page(&mut document.body, fit.1);
    styled(&mut document);
    let body = StoryRef::Body;
    let paragraphs = document.para_paths(body);
    let text = paragraphs.iter().filter_map(|path| document.para(body, path)).fold(0, |all: usize, para| all.saturating_add(para.text.len()));
    if text > MAX_TEXT {
        return Err(format!("the document has more than {} MiB of text", MAX_TEXT >> 20));
    }
    Ok(document)
}

/// The faces of a Markdown or HTML page's text and code, which come with wordcraft, so that a page
/// looks the same on any machine.
const TEXT_FACE: &str = "Source Sans 3";
const CODE_FACE: &str = "JetBrains Mono";

/// The size of its text, and of a code block's, at which 80 columns fit on a line of the block.
const TEXT_SIZE: f32 = 11.0;
const CODE_SIZE: f32 = 9.4;

/// The room below a paragraph of text.
const TEXT_AFTER: f32 = 9.0;

/// The size of code among text, for the size of that text: a monospaced face looks the larger.
const CODE_AMONG_TEXT: f32 = 0.88;

/// Its colours, for white paper: wordcraft's dark page turns each into what suits dark paper.
const INK: Rgb = Rgb(0x1F, 0x23, 0x28);
const QUIET_INK: Rgb = Rgb(0x59, 0x63, 0x6E);
const HAIRLINE: Rgb = Rgb(0xD0, 0xD7, 0xDE);
const WASH: Rgb = Rgb(0xF6, 0xF8, 0xFA);
const LINK: Rgb = Rgb(0x09, 0x69, 0xDA);

/// Each heading's size, the space before and after it, and whether a line runs under it.
const HEADINGS: [(f32, f32, f32, bool); 6] = [
    (23.0, 18.0, 11.0, true),
    (17.5, 18.0, 9.0, true),
    (14.0, 15.0, 6.0, false),
    (12.0, 13.0, 4.0, false),
    (11.0, 11.0, 4.0, false),
    (10.0, 11.0, 4.0, false),
];

/// How `document`, Markdown or HTML just imported, looks: what the importer gives is a word processor's
/// defaults, and such a file says little of its own
/// ([ADR-0016](../../docs/adr/0016-how-markdown-and-html-look.md)). What its own CSS says stays,
/// being on the text itself.
fn styled(document: &mut wordcraft_doc::Document) {
    let colour = |rgb| Some(TextColor::Rgb(rgb));
    let line = |width, color, space| Border { style: BorderStyle::Single, width, color: Some(color), space };
    let styles = &mut document.styles;
    styles.default_chr.font = Some(TEXT_FACE.into());
    styles.default_chr.size = Some(TEXT_SIZE);
    styles.default_chr.color = colour(INK);
    styles.default_para.line_spacing = Some(LineSpacing::Multiple(1.15));
    styles.default_para.space_after = Some(TEXT_AFTER);
    for (level, (size, before, after, ruled)) in HEADINGS.into_iter().enumerate() {
        if let Some(heading) = styles.get_mut(&format!("Heading{}", level + 1)) {
            heading.para.space_before = Some(before);
            heading.para.space_after = Some(after);
            heading.para.line_spacing = Some(LineSpacing::Multiple(1.05));
            heading.para.borders = ruled.then(|| Borders { bottom: Some(line(0.5, HAIRLINE, 3.0)), ..Default::default() });
            heading.chr.font = Some(TEXT_FACE.into());
            heading.chr.size = Some(size);
            heading.chr.bold = Some(true);
            heading.chr.italic = None;
            heading.chr.color = colour(if level == 5 { QUIET_INK } else { INK });
        }
    }
    if let Some(quote) = styles.get_mut("Quote") {
        quote.para.align = Some(wordcraft_doc::Align::Left);
        quote.para.space_before = Some(0.0);
        // The bar on the left edge of the text, from its distance and half its width.
        quote.para.indent_left = Some(11.25);
        quote.para.borders = Some(Borders { left: Some(line(2.5, HAIRLINE, 10.0)), ..Default::default() });
        quote.chr.italic = None;
        quote.chr.color = colour(QUIET_INK);
    }
    // Each line of a code block is a paragraph: alike, they are one box, its edges the colour of its
    // wash for the room they keep around the code, which is indented by that room to keep it in the column.
    if let Some(code) = styles.get_mut("Code") {
        code.para.space_before = Some(4.0);
        code.para.space_after = Some(14.0);
        code.para.contextual_spacing = Some(true);
        code.para.line_spacing = Some(LineSpacing::Multiple(1.05));
        code.para.indent_left = Some(8.0);
        code.para.indent_right = Some(8.0);
        code.para.shading = Some(WASH);
        code.para.borders = Some(Borders::all(line(0.5, WASH, 8.0)));
        code.chr.font = Some(CODE_FACE.into());
        code.chr.size = Some(CODE_SIZE);
    }
    if let Some(item) = styles.get_mut("ListParagraph") {
        item.para.contextual_spacing = Some(false);
        item.para.space_after = Some(2.0);
    }
    if let Some(parts) = styles.get_mut("TableGrid").and_then(|grid| grid.table.as_mut()) {
        parts.borders = Some(Borders::all(line(0.5, HAIRLINE, 0.0)));
        parts.cell_margins = Some([3.5, 6.0, 3.5, 6.0]);
        parts.header_fill = Some(WASH);
        parts.header_chr.bold = Some(true);
    }
    for level in document.numbering.abstracts.iter_mut().flat_map(|list| list.levels.iter_mut().enumerate()) {
        let (depth, level) = level;
        level.indent = 20.0 + 18.0 * depth as f32;
        level.hanging = 14.0;
        level.tab = None;
    }
    if let Some(Block::Para(first)) = document.body.first_mut().map(Arc::make_mut) {
        first.props.space_before = Some(0.0);
    }
    styled_blocks(&mut document.body, &document.numbering);
}

/// What a style can't say, in `blocks`: what depends on the blocks beside one, or on the text in it.
fn styled_blocks(blocks: &mut [Arc<Block>], numbering: &wordcraft_doc::Numbering) {
    let list_of = |block: Option<&Arc<Block>>| match block.map(|block| &**block) {
        Some(Block::Para(para)) => para.props.numbering.map(|list| list.num),
        _ => None,
    };
    for at in 0..blocks.len() {
        let next_list = list_of(blocks.get(at + 1));
        let after_table = at.checked_sub(1).and_then(|before| blocks.get(before)).is_some_and(|block| matches!(**block, Block::Table(_)));
        let Some(block) = blocks.get_mut(at) else { break };
        match Arc::make_mut(block) {
            Block::Para(para) => {
                styled_para(para, next_list, after_table, numbering);
                para.touch();
            }
            Block::Table(table) => {
                for cell in table.rows.iter_mut().flat_map(|row| row.cells.iter_mut()) {
                    styled_blocks(&mut cell.blocks, numbering);
                }
            }
        }
    }
}

/// `para`, before what may be a block of the list `next_list`, and maybe after a table.
fn styled_para(para: &mut wordcraft_doc::Paragraph, next_list: Option<u32>, after_table: bool, numbering: &wordcraft_doc::Numbering) {
    let heading = para.props.style.as_deref().and_then(|style| style.strip_prefix("Heading")).and_then(|level| level.parse::<usize>().ok());
    let text_size = heading.and_then(|level| HEADINGS.get(level.wrapping_sub(1))).map_or(TEXT_SIZE, |heading| heading.0);
    for run in &mut para.runs {
        // Code among text is what the importer gives this font. Its face alone says so: a shading
        // would show a seam between each two letters (ADR-0016).
        if run.props.font.as_deref() == Some(wordcraft_formats::model::MONO_FONT) {
            run.props.font = Some(CODE_FACE.into());
            run.props.size = Some((run.props.size.unwrap_or(text_size) * CODE_AMONG_TEXT).max(1.0));
        }
        if run.props.link.is_some() && run.props.color.is_none() {
            run.props.color = Some(TextColor::Rgb(LINK));
        }
    }
    let list = para.props.numbering.map(|list| list.num);
    if list.is_some() && list != next_list {
        para.props.space_after = Some(TEXT_AFTER);
    }
    // A task's box takes its bullet's place, and its text the text's.
    if ["☑ ", "☐ "].iter().any(|task| para.text.starts_with(task)) {
        let level = para.props.numbering.and_then(|list| numbering.abstract_of(list.num)?.levels.get(usize::from(list.level)));
        if let Some(level) = level.filter(|level| level.format == wordcraft_doc::section::NumFormat::Bullet) {
            para.props.indent_left = Some(level.indent);
            para.props.indent_first = Some(-level.hanging);
            para.props.numbering = None;
            // A box is three bytes, and its space is the fourth, as just checked. The same length, so
            // the runs stay as they are.
            para.text.replace_range(3..4, "\t");
        }
    }
    // A rule is a paragraph of nothing with a line under it.
    if para.text.is_empty()
        && let Some(bottom) = para.props.borders.as_mut().and_then(|borders| borders.bottom.as_mut())
    {
        bottom.color = Some(HAIRLINE);
        para.props.space_before = Some(6.0);
        para.props.space_after = Some(14.0);
    }
    if after_table && heading.is_none() {
        para.props.space_before = Some(14.0);
    }
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

    #[test]
    fn a_dark_page_is_exactly_the_paper_asked_for_and_keeps_its_white() {
        assert_eq!(grey_of([0x1E; 3]), 0x1E);
        assert_eq!(grey_of([255; 3]), 255);
        for paper in [[0x1E, 0x2E, 0x3E], [0, 0, 0], [0x28, 0x2C, 0x34], [0x00, 0x2B, 0x36], [0x1E, 0x1E, 0x1E]] {
            let grey = grey_of(paper);
            let mut pixels = [grey, grey, grey, 255, 255, 255, 255, 255, 40, 200, 90, 77];
            on_paper(&mut pixels, paper);
            assert_eq!(pixels[..4], [paper[0], paper[1], paper[2], 255], "the page: {paper:?}");
            assert_eq!(pixels[4..8], [255; 4], "white: {paper:?}");
            assert_eq!(pixels[11], 77, "what is seen through stays: {paper:?}");
        }
        let mut grey = [0x1E, 0x1E, 0x1E, 255, 80, 90, 100, 255];
        on_paper(&mut grey, [0x1E; 3]);
        assert_eq!(grey, [0x1E, 0x1E, 0x1E, 255, 80, 90, 100, 255], "a grey terminal: nothing to do");
    }

    /// The page's corner, drawn through `backend` on `paper`, from a file of `contents` named `name`.
    fn corner(name: &str, contents: &[u8], paper: Paper) -> (bool, [u8; 3]) {
        let dir = std::env::temp_dir().join(format!("clview-word-corner-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        let input = crate::source::Input::open(path.to_str().unwrap()).unwrap();
        let Ok(Opened::Pages(source)) = Wordcraft.open(input, &Parsed::default()) else { panic!("not pages: {name}") };
        let frame = source.renderer().unwrap().render(Request { page: 0, scale: 1.0, tag: 0, paper });
        assert_eq!(frame.error, None);
        (source.dark_form(), [frame.rgba[0], frame.rgba[1], frame.rgba[2]])
    }

    /// Markdown and HTML go on the paper asked for; what is laid out to be printed stays as printed.
    #[test]
    fn only_markdown_and_html_have_a_dark_form() {
        const PAPER: [u8; 3] = [0x1E, 0x2E, 0x3E];
        for (name, contents) in [("a.md", &b"# One"[..]), ("a.html", b"<p>One</p>")] {
            assert_eq!(corner(name, contents, Paper::Dark(PAPER)), (true, PAPER), "{name}");
            assert_eq!(corner(name, contents, Paper::Printed), (true, [255; 3]), "{name}");
        }
        assert_eq!(corner("a.rtf", b"{\\rtf1\\ansi One}", Paper::Dark(PAPER)), (false, [255; 3]));
    }

    /// How many pixels to a point the styling's tests look at a page.
    const SCALE: f32 = 2.0;
    /// The page's left margin, and its width, in those pixels.
    const MARGIN: u32 = 144;
    const PAGE_WIDTH: u32 = 1224;

    /// The first page of this Markdown on `paper`.
    fn looked_at(markdown: &str, paper: Paper) -> Frame {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let at = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("clview-word-looks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{at}.md"));
        std::fs::write(&path, markdown).unwrap();
        let input = crate::source::Input::open(path.to_str().unwrap()).unwrap();
        let Ok(Opened::Pages(source)) = Wordcraft.open(input, &Parsed::default()) else { panic!("not pages") };
        let frame = source.renderer().unwrap().render(Request { page: 0, scale: SCALE, tag: 0, paper });
        assert_eq!(frame.error, None);
        assert_eq!(frame.width, PAGE_WIDTH);
        std::fs::remove_file(&path).unwrap();
        frame
    }

    fn colour_at(frame: &Frame, x: u32, y: u32) -> [u8; 3] {
        let at = (y as usize * frame.width as usize + x as usize) * 4;
        [frame.rgba[at], frame.rgba[at + 1], frame.rgba[at + 2]]
    }

    fn brightness([r, g, b]: [u8; 3]) -> u8 {
        grey_of([r, g, b])
    }

    /// Dark ink on white: not a line or a wash, which are faint.
    fn dark_ink(colour: [u8; 3]) -> bool {
        brightness(colour) < 128
    }

    /// Anything on white.
    fn not_white(colour: [u8; 3]) -> bool {
        colour != [255; 3]
    }

    /// A line of text, in pixels.
    #[derive(Clone, Copy, Debug)]
    struct Inked {
        top: u32,
        bottom: u32,
        left: u32,
        right: u32,
    }

    /// What `ink` finds on a page, in lines: rows with some, apart from others by rows with none.
    fn lines_of(frame: &Frame, ink: fn([u8; 3]) -> bool) -> Vec<Inked> {
        let mut lines: Vec<Inked> = Vec::new();
        let mut in_line = false;
        for y in 0..frame.height {
            let inked: Vec<u32> = (0..frame.width).filter(|x| ink(colour_at(frame, *x, y))).collect();
            let (Some(left), Some(right)) = (inked.first().copied(), inked.last().copied()) else {
                in_line = false;
                continue;
            };
            match lines.last_mut().filter(|_| in_line) {
                Some(line) => *line = Inked { bottom: y, left: line.left.min(left), right: line.right.max(right), ..*line },
                None => lines.push(Inked { top: y, bottom: y, left, right }),
            }
            in_line = true;
        }
        lines
    }

    /// What decides that a page isn't a word processor's: Word documents stay as they say.
    #[test]
    fn only_markdown_and_html_are_given_this_look() {
        let font = |bytes: &[u8], kind| {
            let laid = laid_out(bytes, kind, None, "a", None).unwrap();
            let document = laid.document.read().unwrap();
            document.styles.default_chr.font.clone().unwrap()
        };
        assert_eq!(font(b"One", Kind::Markdown), TEXT_FACE);
        assert_eq!(font(b"<p>One</p>", Kind::Html), TEXT_FACE);
        assert_ne!(font(b"{\\rtf1\\ansi One}", Kind::Rtf), TEXT_FACE);
    }

    /// wordcraft reads HTML two ways: as a web page, set in Times New Roman at a browser's sizes over
    /// any style of the document's, or as a fragment, which takes the document's. This look is styles.
    #[test]
    fn text_and_headings_take_this_look_not_a_web_pages() {
        let nowhere = Pictures::beside(None);
        let documents = [markdown(b"# Head\n\nText", &nowhere).unwrap(), html(b"<h1>Head</h1><p>Text</p>", &nowhere).unwrap()];
        for document in &documents {
            let paragraphs = document.para_paths(StoryRef::Body);
            assert_eq!(paragraphs.len(), 2);
            for para in paragraphs.iter().filter_map(|path| document.para(StoryRef::Body, path)) {
                assert_eq!(para.runs.len(), 1, "{:?}", para.text);
                let props = &para.runs[0].props;
                assert_eq!((props.font.as_deref(), props.size), (None, None), "{:?}", para.text);
            }
        }
    }

    #[test]
    fn headings_step_down_and_the_first_starts_the_page() {
        let frame = looked_at("# Hxg\n\n## Hxg\n\n### Hxg\n\n#### Hxg\n\n##### Hxg\n\n###### Hxg", Paper::Printed);
        let lines = lines_of(&frame, dark_ink);
        assert_eq!(lines.len(), 6, "{lines:?}");
        let heights: Vec<u32> = lines.iter().map(|line| line.bottom - line.top).collect();
        assert!(heights.is_sorted_by(|a, b| a > b), "{heights:?}");
        // The space before it would leave it further down: 54 points of margin, and some of its size.
        assert!(lines[0].top < 2 * 72, "{:?}", lines[0]);
    }

    /// A word processor's code block is its lines one under the other: here, one box the width of the
    /// text, blank lines included, on either paper.
    #[test]
    fn a_code_block_is_one_box_as_wide_as_the_text() {
        let box_of = |paper| {
            let frame = looked_at("Above\n\n```\none\n\nthree\n```\n\nBelow", paper);
            let empty = colour_at(&frame, 0, 0);
            // Inside its right edge, which the short text above and below it doesn't reach.
            let x = PAGE_WIDTH - MARGIN - 4;
            let rows: Vec<u32> = (0..frame.height).filter(|y| colour_at(&frame, x, *y) != empty).collect();
            let (top, bottom) = (rows[0], *rows.last().unwrap());
            assert_eq!(rows.len() as u32, bottom - top + 1, "{paper:?}: a gap in it");
            let middle = (top + bottom) / 2;
            let wash = colour_at(&frame, x, middle);
            // Its lines are fills that meet on parts of a pixel, by a step or two that doesn't show.
            let near = |colour: [u8; 3]| colour.iter().zip(wash).all(|(a, b)| a.abs_diff(b) <= 3);
            assert!((top + 1..bottom).all(|y| near(colour_at(&frame, x, y))), "{paper:?}: a seam in it");
            for (x, inside) in [(MARGIN - 3, false), (MARGIN + 3, true), (PAGE_WIDTH - MARGIN - 3, true), (PAGE_WIDTH - MARGIN + 3, false)] {
                assert_eq!(colour_at(&frame, x, middle) != empty, inside, "{paper:?}: at {x}");
            }
            (top, bottom, empty, wash, frame)
        };
        let (top, bottom, _, wash, frame) = box_of(Paper::Printed);
        assert!(wash.iter().zip([WASH.0, WASH.1, WASH.2]).all(|(a, b)| a.abs_diff(b) <= 3), "{wash:?}");
        let lines = lines_of(&frame, dark_ink);
        let [above, one, three, below] = lines[..] else { panic!("{lines:?}") };
        assert!(above.bottom < top && top < one.top && three.bottom < bottom && bottom < below.top, "{top}..{bottom}: {lines:?}");
        // On dark paper, a little lighter than the page, not a hole in it or a light in it.
        let (_, _, empty, wash, _) = box_of(Paper::Dark([0x1E; 3]));
        assert_eq!(empty, [0x1E; 3]);
        assert!(brightness(wash) > brightness(empty) && brightness(wash) < brightness(empty) + 16, "{wash:?}");
    }

    /// A paragraph's room around each line would leave them apart, in their box.
    #[test]
    fn a_code_blocks_lines_are_close() {
        let lines = lines_of(&looked_at("```\nHxg\nHxg\n```", Paper::Printed), dark_ink);
        let [one, two] = lines[..] else { panic!("{lines:?}") };
        assert!(two.top - one.top < 2 * 16, "{lines:?}");
    }

    #[test]
    fn code_is_monospaced_among_text_and_in_a_block_as_text_isnt() {
        let frame = looked_at("`llllllllll`\n\n`MMMMMMMMMM`\n\n```\nllllllllll\nMMMMMMMMMM\n```\n\nllllllllll\n\nMMMMMMMMMM", Paper::Printed);
        let lines = lines_of(&frame, dark_ink);
        assert_eq!(lines.len(), 6, "{lines:?}");
        let width = |at: usize| (lines[at].right - lines[at].left) as f32;
        for (narrow, wide, what) in [(0, 1, "among text"), (2, 3, "in a block")] {
            assert!(width(narrow) / width(wide) > 0.85, "{what}: {} of {}", width(narrow), width(wide));
        }
        assert!(width(4) / width(5) < 0.6, "text: {} of {}", width(4), width(5));
    }

    /// What most code is written to: wrapped sooner, every long line of it would break.
    #[test]
    fn eighty_columns_of_code_fit_on_a_line() {
        let lines = |columns: usize| lines_of(&looked_at(&format!("```\n{}\n```", "0".repeat(columns)), Paper::Printed), dark_ink).len();
        assert_eq!((lines(80), lines(81)), (1, 2));
    }

    #[test]
    fn a_quote_is_where_text_is_with_a_bar_beside_it() {
        let frame = looked_at("> One\n>\n> Two", Paper::Printed);
        let lines = lines_of(&frame, dark_ink);
        let [one, two] = lines[..] else { panic!("{lines:?}") };
        // As a word processor has it, it would be in the middle of the page.
        assert!(one.left < MARGIN + 2 * 14 && two.left < MARGIN + 2 * 14, "{lines:?}");
        // The bar goes on between its paragraphs.
        let bar = (MARGIN..one.left).find(|x| not_white(colour_at(&frame, *x, one.top))).expect("a bar");
        assert!((one.top..=two.bottom).all(|y| not_white(colour_at(&frame, bar, y))), "the bar stops");
    }

    #[test]
    fn a_link_stands_out_on_either_paper_and_text_doesnt() {
        let blue = |markdown: &str, paper| {
            let frame = looked_at(markdown, paper);
            frame.rgba.as_chunks::<4>().0.iter().any(|pixel| i32::from(pixel[2]) > i32::from(pixel[0]) + 60)
        };
        for paper in [Paper::Printed, Paper::Dark([0x1E; 3])] {
            assert!(blue("[One](https://example.com)", paper), "{paper:?}");
            assert!(!blue("One", paper), "{paper:?}");
        }
    }

    #[test]
    fn lists_stand_apart_as_their_items_dont() {
        let lines = lines_of(&looked_at("- one\n- two\n\n1. three\n2. four\n\nAfter", Paper::Printed), dark_ink);
        let [one, two, three, four, after] = lines[..] else { panic!("{lines:?}") };
        let (items, lists) = (two.top - one.bottom, three.top - two.bottom);
        assert!(lists >= items + 2 * 5, "{items} between items, {lists} between lists");
        assert!(after.top - four.bottom >= items + 2 * 5, "after a list: {lines:?}");
    }

    /// Only a bullet goes, since a numbered task keeps its number; and what is in a table is styled too.
    #[test]
    fn a_tasks_box_takes_the_place_of_its_bullet() {
        let laid = laid_out(b"- one\n- [x] two\n\n1. [x] three\n\n| [`a`](b) |\n|---|\n", Kind::Markdown, None, "a", None).unwrap();
        let document = laid.document.read().unwrap();
        let paras: Vec<&wordcraft_doc::Paragraph> =
            document.para_paths(StoryRef::Body).iter().filter_map(|path| document.para(StoryRef::Body, path)).collect();
        let [one, two, three, cell, ..] = paras[..] else { panic!("{} paragraphs", paras.len()) };
        assert!(one.props.numbering.is_some());
        assert_eq!((two.props.numbering, two.text.as_str()), (None, "☑\ttwo"));
        assert_eq!(two.props.indent_left.unwrap() + two.props.indent_first.unwrap(), 6.0, "where the bullet would be");
        assert!(three.props.numbering.is_some() && three.text == "☑ three", "{:?}", three.text);
        let run = &cell.runs[0].props;
        assert_eq!((run.font.as_deref(), run.color), (Some(CODE_FACE), Some(TextColor::Rgb(LINK))));
    }

    #[test]
    fn a_table_keeps_its_distance_from_what_follows() {
        let lines = lines_of(&looked_at("| One |\n|---|\n| Two |\n\nAfter", Paper::Printed), not_white);
        let [table, after] = lines[..] else { panic!("{lines:?}") };
        assert!(after.top - table.bottom >= 2 * 10, "{lines:?}");
    }

    #[test]
    fn a_footnotes_number_is_on_its_line() {
        let lines = lines_of(&looked_at("One[^1]\n\n[^1]: Two", Paper::Printed), dark_ink);
        assert_eq!(lines.len(), 2, "{lines:?}");
        // What only looks like one, cut short or with no number, stays as it is.
        for html in ["<sup class=\"footnote-definition-label\">", "<sup class=\"footnote-definition-label\">x</sup>\n<p>y", "z"] {
            assert_eq!(numbers_in_their_footnotes(html), html);
        }
    }

    /// A dark page inverts a colour's lightness, which leaves one of middling lightness about as it was:
    /// a purple fine on white came out at 2.3 to 1 on dark paper.
    #[test]
    fn what_is_coloured_can_be_read_on_either_paper() {
        let contrast = |a: Rgb, b: Rgb| {
            let linear = |channel: u8| {
                let share = f32::from(channel) / 255.0;
                if share <= 0.04045 { share / 12.92 } else { ((share + 0.055) / 1.055).powf(2.4) }
            };
            let light = |c: Rgb| 0.2126 * linear(c.0) + 0.7152 * linear(c.1) + 0.0722 * linear(c.2);
            let (a, b) = (light(a), light(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        let kinds = [BlockQuoteKind::Note, BlockQuoteKind::Tip, BlockQuoteKind::Important, BlockQuoteKind::Warning, BlockQuoteKind::Caution];
        let dark = RenderOptions { dark: true, dark_paper: 0x1E, ..Default::default() };
        for colour in [INK, QUIET_INK, LINK].into_iter().chain(kinds.map(|kind| alert(kind).1)) {
            assert!(contrast(colour, Rgb::WHITE) >= 4.5, "{colour:?} on white");
            assert!(contrast(dark.ink(colour), Rgb(0x1E, 0x1E, 0x1E)) >= 4.5, "{colour:?} on dark, as {:?}", dark.ink(colour));
        }
    }

    /// As written, an alert is a quote that starts with its tag.
    #[test]
    fn an_alert_says_what_it_is() {
        let html = markdown_as_html("> [!WARNING]\n> Mind it\n\n> [!MAYBE]\n> Not one");
        assert!(html.contains(">Warning</strong>") && !html.contains("[!WARNING]"), "{html}");
        assert!(html.contains("[!MAYBE]"), "{html}");
        // Right above its text, a line's height (about 17.5 points) from it, not a paragraph apart.
        let lines = lines_of(&looked_at("> [!NOTE]\n> Mind it", Paper::Printed), dark_ink);
        let [kind, text] = lines[..] else { panic!("{lines:?}") };
        assert!(text.top - kind.top < 2 * 20, "{lines:?}");
        // With no paragraph to go on, it still says.
        for alert in ["> [!TIP]\n> - One", "> [!TIP]"] {
            assert!(markdown_as_html(alert).contains(">Tip</strong>"), "{alert}");
        }
    }

    /// The pager's way: pages rendering side by side on a pool's threads, each drawing its pictures
    /// into what they share, and every page showing them, its own and the one on every page. Many times
    /// over, as a page that finds that one drawn by another thread must not render its stand-in, which
    /// only shows sometimes.
    #[cfg(feature = "image")]
    #[test]
    fn a_pools_threads_draw_their_pages_pictures() {
        let dir = std::env::temp_dir().join(format!("clview-word-pool-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let picture = |red: u8| {
            let pixels: Vec<u8> = [red, 0, 0, 255].repeat(400);
            crate::encode::encode_png(20, 20, &pixels, crate::encode::PngEffort::Fast).unwrap()
        };
        let shows =
            |frame: &Frame, red: u8| frame.rgba.as_chunks::<4>().0.iter().any(|pixel| pixel[0].abs_diff(red) < 3 && pixel[1] < 3 && pixel[2] < 3);
        const SHARED: u8 = 250;
        std::fs::write(dir.join("shared.png"), picture(SHARED)).unwrap();
        let pages = 6u8;
        let mut document = String::new();
        for n in 0..pages {
            std::fs::write(dir.join(format!("{n}.png")), picture(40 * (n + 1))).unwrap();
            document.push_str(&format!("<p style=\"page-break-after:always\"><img src=\"{n}.png\"> <img src=\"shared.png\"></p>"));
        }
        let path = dir.join("a.html");
        std::fs::write(&path, document).unwrap();

        for round in 0..300 {
            let input = crate::source::Input::open(path.to_str().unwrap()).unwrap();
            let Ok(Opened::Pages(source)) = Wordcraft.open(input, &Parsed::default()) else { panic!("not pages") };
            assert!(source.pages() >= usize::from(pages));
            let pool = source.pool(4);
            let requests: Vec<Request> =
                (0..usize::from(pages)).map(|page| Request { page, scale: 0.5, tag: 0, paper: Default::default() }).collect();
            let started = std::time::Instant::now();
            let mut done = vec![false; requests.len()];
            while done.iter().any(|page| !page) {
                assert!(started.elapsed().as_secs() < 60, "still waiting for {done:?}");
                pool.set_queue(requests.iter().filter(|request| !done[request.page]).copied().collect());
                let Some(frame) = pool.try_recv() else {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                };
                assert_eq!(frame.error, None);
                let page = frame.request.page;
                assert!(shows(&frame, 40 * (u8::try_from(page).unwrap() + 1)), "round {round}: page {page} doesn't show its picture");
                assert!(shows(&frame, SHARED), "round {round}: page {page} doesn't show the one they share");
                done[page] = true;
            }
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
