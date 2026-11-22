//! PDFs rendered by pdfcraft, the way its app draws them: comments without an appearance of their
//! own are drawn, a dynamic XFA form shows the pages its template lays out into, and either
//! password opens a protected file.
//!
//! Their text, for a search, is read by pdfcraft too, a page a chunk, in a worker process of its
//! own ([`TEXT`]): what a page's text takes to read can't be bounded before it is read (a page of
//! three million glyphs took 870 MiB), so the worker's heap and CPU time are, as they are spent,
//! and a page past them costs the search that page
//! ([ADR-0027](../../docs/adr/0027-a-pdfs-text-read-in-a-worker-process-for-its-heap-cap.md)).
//!
//! Their links are the link annotations pdfcraft read as the file was opened, kept with what places
//! them on their pages as shown ([`Linked`]); only a named destination, looked up when a link to
//! another file names one here, reads the file again, on the links' own thread ([`PageLinks`]).
//!
//! A list's tile is the file itself, its pages measured by hayro-syntax as pdfcraft measures them:
//! no display copy, and no annotations read. Gone into, the file is opened whole
//! ([ADR-0042](../../docs/adr/0042-a-pdfs-tile-is-the-file-itself-measured-by-hayro.md)).

use std::ops::Range;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pdfcraft_engine::links::resolve_remote_document_path;
use pdfcraft_render::text::TextFilter;
use pdfcraft_render::{
    DestView, LinkTarget, PageInfo, PageRenderer, PageText, RemoteDestination, RenderConfig, RenderPool, RenderRequest, RenderWarning, RenderedPage,
    RequestKind,
};

use super::texts::{Answer, Ending, Queue};
use super::worker::{self, Died, FAILED, Link, Session, WALL, Worker};
use super::{
    Ask, Backend, Deferred, Format, Frame, Got, Input, Job, Links, MAX_ADDRESS, MAX_LINK_RECTS, MAX_LINKS, MAX_NAME, Opened, Pixels, Request, Room,
    Source, Target, Texts, Told, Values, Want,
};
use crate::args::{OptionSpec, Parsed};

/// The integration tests' PDFs, written by hand: the one with links is read by the tests here too.
#[cfg(test)]
#[path = "../../tests/it/fixtures.rs"]
#[expect(dead_code, reason = "only the PDF with links is read here")]
mod fixtures;

/// PDFs, rendered by pdfcraft.
pub(super) struct Pdfcraft;

const OPTIONS: &[OptionSpec] =
    &[OptionSpec { name: "--password", short: None, value: Some("PW"), help: "the user or the owner password of an encrypted PDF", alias: None }];

impl Backend for Pdfcraft {
    fn name(&self) -> &'static str {
        "pdfcraft"
    }

    fn shows(&self) -> &'static str {
        "PDFs"
    }

    fn formats(&self) -> Vec<Format> {
        vec![Format::of("PDF", &[]).named_by(&["pdf"])]
    }

    fn credits(&self) -> &[&'static str] {
        &["pdfcraft"]
    }

    fn options(&self) -> &'static [OptionSpec] {
        OPTIONS
    }

    /// Sure of a file that starts with its header.
    fn opens(&self, _path: &str, head: &[u8]) -> bool {
        head.starts_with(HEADER)
    }

    /// Loosely, of one with its header further in: a zip's first file, stored, or a drawing's
    /// text may hold it there too.
    fn opens_loosely(&self, _path: &str, head: &[u8]) -> bool {
        is_pdf(head)
    }

    fn draws_pictures(&self) -> bool {
        true
    }

    /// It reads the whole file to open it.
    fn deferred(&self, _path: &str, _head: &[u8]) -> Option<Deferred> {
        Some(Deferred { settings: &[] })
    }

    fn open(&self, input: Input, options: &Parsed<'_>) -> Result<Opened, String> {
        let path = input.path.clone();
        Ok(Opened::Pages(Box::new(Pdf::open(&path, Arc::new(input.bytes()?), options.value("--password"))?)))
    }

    /// The file itself, its pages measured ([`Pdf::tile`]); one that can't be measured so is opened
    /// whole, which says why where it can't be opened either.
    fn open_tile(&self, input: Input, options: &Parsed<'_>, _room: Room) -> Result<Opened, String> {
        let (path, password) = (input.path.clone(), options.value("--password"));
        let bytes = Arc::new(input.bytes()?);
        let pdf = match Pdf::tile(&path, bytes.clone(), password) {
            Some(tile) => tile,
            None => Pdf::open(&path, bytes, password)?,
        };
        Ok(Opened::Pages(Box::new(pdf)))
    }
}

/// Whether a file starting with `head` is a PDF: its header (`%PDF-` and the version) within the
/// first 1024 bytes. ISO 32000-2 §7.5.2 puts the header first; readers have long allowed bytes
/// before it, up to that bound.
fn is_pdf(head: &[u8]) -> bool {
    head.get(..1024).unwrap_or(head).windows(HEADER.len()).any(|w| w == HEADER)
}

/// What a PDF's header starts with.
const HEADER: &[u8] = b"%PDF-";

/// The most pages a PDF may have to be opened. A page tree may name one page over and over, six
/// bytes a name, so a file of a few megabytes can have millions; real documents stay far below.
const MAX_PAGES: usize = 100_000;

/// An open PDF: the display copy the renderers read, each page's size as shown, and its links.
pub(super) struct Pdf {
    bytes: Arc<Vec<u8>>,
    config: RenderConfig,
    /// In points, after `/Rotate` and `/UserUnit`.
    sizes: Vec<(f32, f32)>,
    linked: Arc<Linked>,
    /// For a list's tile, the file itself: what opens it whole.
    whole: Option<Whole>,
}

/// What opens a PDF whole that was opened for a list's tile: its path, and the password given.
#[derive(Clone)]
struct Whole {
    path: String,
    password: Option<String>,
}

/// A PDF's links as pdfcraft read them when the file was opened, and what places them on their
/// pages as shown: each page's box and turn.
struct Linked {
    pages: Vec<PageInfo>,
    /// The links, a page's together, in the pages' order.
    links: Vec<pdfcraft_render::Link>,
    /// Where each page's links start in `links`, and after the last page, where they end.
    from: Vec<usize>,
}

impl Linked {
    fn new(pages: Vec<PageInfo>, mut links: Vec<pdfcraft_render::Link>) -> Self {
        links.sort_by_key(|link| link.page);
        let from = (0..=pages.len()).map(|page| links.partition_point(|link| link.page < page)).collect();
        Self { pages, links, from }
    }

    /// Page `page`'s links, at most [`MAX_LINKS`], and how many more it has.
    fn on(&self, page: usize) -> (Vec<super::Link>, usize) {
        let (Some(info), Some(from), Some(to)) = (self.pages.get(page), self.from.get(page), self.from.get(page.saturating_add(1))) else {
            return (Vec::new(), 0);
        };
        let on = self.links.get(*from..*to).unwrap_or_default();
        let links = on.iter().take(MAX_LINKS).map(|link| self.link(page, info, link)).collect();
        (links, on.len().saturating_sub(MAX_LINKS))
    }

    /// `link`, on page `page`, as the viewer follows it: where it is active on the page as shown
    /// (its quadrilaterals' boxes, or else its rectangle), and where it leads.
    fn link(&self, page: usize, info: &PageInfo, link: &pdfcraft_render::Link) -> super::Link {
        let [x0, y0, x1, y1] = link.rect;
        let rects: Vec<[f32; 4]> = match link.quads.is_empty() {
            true => shown(info, &[(x0, y0), (x1, y1)]).into_iter().collect(),
            false => {
                let corners = |quad: &[f32; 8]| [(quad[0], quad[1]), (quad[2], quad[3]), (quad[4], quad[5]), (quad[6], quad[7])];
                link.quads.iter().take(MAX_LINK_RECTS).filter_map(|quad| shown(info, &corners(quad))).collect()
            }
        };
        let (address, target) = self.target(&link.target);
        super::Link { page, rects, address, target }
    }

    /// Where a link of pdfcraft's leads, and its address for the reader to copy, if it has one.
    fn target(&self, target: &LinkTarget) -> (Option<String>, Target) {
        let other = |says: &str| (None, Target::Other(says.into()));
        match target {
            LinkTarget::Page(page, view) => match self.pages.get(*page) {
                Some(info) => (None, Target::Place { page: *page, y: down(info, *view) }),
                None => other("a link to a page this document doesn't have"),
            },
            LinkTarget::Uri(address) if address.len() > MAX_ADDRESS => other("a web address too long to follow"),
            LinkTarget::Uri(address) => (Some(address.clone()), Target::Web(address.clone())),
            LinkTarget::JavaScript(_) => other(SCRIPT),
            LinkTarget::SetLayers { .. } => other("a link that shows or hides layers, which clv doesn't"),
            LinkTarget::RemoteGoTo { file, destination, .. } => {
                // Whether it is a local path at all, pdfcraft says, against a document that stands
                // for any: nothing is looked for on disk here, and what the path leads to is the
                // viewer's to check.
                if let Err(why) = resolve_remote_document_path(Some(std::path::Path::new("/a/document.pdf")), file) {
                    return other(&format!("a link to another file, not followed: {why}"));
                }
                let (fragment, page) = match destination {
                    RemoteDestination::Default => (None, None),
                    RemoteDestination::Page { page, .. } => (None, Some(*page)),
                    RemoteDestination::Named(name) if name.len() > MAX_NAME => return other("a link to a place whose name is too long to follow"),
                    RemoteDestination::Named(name) => (Some(name.clone()), None),
                };
                let address = fragment.as_ref().map_or_else(|| file.clone(), |name| format!("{file}#{name}"));
                (Some(address), Target::File { path: file.clone(), fragment, page })
            }
            LinkTarget::Other(kind) => match kind.as_str() {
                "JavaScript" => other(SCRIPT),
                "Launch" => other("a link that starts a program, which clv doesn't"),
                "URI" => other("a web address that can't be read"),
                "GoToR" => other("a link to another file that can't be read"),
                "" => other("a link that leads nowhere"),
                kind => {
                    let kind: String = kind.chars().filter(char::is_ascii_alphanumeric).take(32).collect();
                    other(&format!("a link of a kind clv doesn't follow ({kind})"))
                }
            },
        }
    }
}

/// What the reader is told of a link that runs a script.
const SCRIPT: &str = "a script, which clv doesn't run";

/// The box, on `page` as shown, of the points `corners` in its user space: left, top, right and
/// bottom in points from its top left. `None` if any of it isn't a number.
fn shown(page: &PageInfo, corners: &[(f32, f32)]) -> Option<[f32; 4]> {
    let mut rect = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
    for (x, y) in corners {
        let [across, down] = page.user_to_view(*x, *y);
        if !across.is_finite() || !down.is_finite() {
            return None;
        }
        rect = [rect[0].min(across), rect[1].min(down), rect[2].max(across), rect[3].max(down)];
    }
    rect.iter().all(|side| side.is_finite()).then_some(rect)
}

/// How far down `page` as shown the destination `view` is, in points from its top: by its `top`
/// for `/XYZ` and `/FitH`, by its rectangle's top as shown for `/FitR`, and the page's top for the
/// rest and for an operand left out. Through the page's box and turn, so that on a page turned a
/// quarter, where the user space's `left` runs down the page shown, `/XYZ` goes by its `left`.
fn down(page: &PageInfo, view: DestView) -> f32 {
    let by = |left: Option<f32>, top: Option<f32>| page.dest_fraction(left, top, 0)[1].map_or(0.0, |share| share * page.height);
    let y = match view {
        DestView::Top | DestView::Fit | DestView::FitV { .. } => 0.0,
        DestView::Xyz { left, top, .. } => by(left, top),
        DestView::FitH { top } => by(None, top),
        DestView::FitR { rect: [x0, y0, x1, y1] } => shown(page, &[(x0, y0), (x1, y1)]).map_or(0.0, |rect| rect[1]),
    };
    // `clamp` takes no bound that isn't a number, nor a top under zero.
    let height = if page.height.is_finite() { page.height.max(0.0) } else { 0.0 };
    if y.is_finite() { y.clamp(0.0, height) } else { 0.0 }
}

impl Pdf {
    fn open(path: &str, bytes: Arc<Vec<u8>>, password: Option<&str>) -> Result<Self, String> {
        let display = displayed(path, bytes, password)?;
        let info = display.info.map_err(|e| format!("{path}: {e}"))?;
        if info.pages.is_empty() {
            return Err(format!("{path}: no pages to show"));
        }
        // Each page the tree names costs some 2 KB to parse, again for the renderer: refused here,
        // that second parse is never paid (a million pages: 3.7 s and 3.4 GB to open otherwise).
        if info.pages.len() > MAX_PAGES {
            return Err(format!("{path}: {} pages, and the most clv opens is {MAX_PAGES}", info.pages.len()));
        }
        Ok(Self {
            bytes: display.bytes,
            config: RenderConfig { password: display.password.as_deref().map(Arc::from), reject_oversize: true, ..Default::default() },
            sizes: info.pages.iter().map(|p| (p.width, p.height)).collect(),
            linked: Arc::new(Linked::new(info.pages, info.links)),
            whole: None,
        })
    }

    /// The file `bytes`, read from `path`, as a list's tile shows it, opened with `password`: the
    /// file itself rendered, with no display copy, its pages measured by hayro-syntax as pdfcraft's
    /// `inspect` measures them, and none of its annotations read, links among them. `None` where
    /// hayro-syntax can't read it, or it has no pages or more than [`MAX_PAGES`].
    fn tile(path: &str, bytes: Arc<Vec<u8>>, password: Option<&str>) -> Option<Self> {
        let render_password = pdfcraft_engine::render_password_for(bytes.clone(), password);
        // A parser's panic costs the tile's shortcut, not the file.
        let pages = std::panic::catch_unwind(AssertUnwindSafe(|| measured(bytes.clone(), render_password.as_deref()))).ok().flatten()?;
        if pages.is_empty() || pages.len() > MAX_PAGES {
            return None;
        }
        Some(Self {
            bytes,
            config: RenderConfig { password: render_password.as_deref().map(Arc::from), reject_oversize: true, ..Default::default() },
            sizes: pages.iter().map(|p| (p.width, p.height)).collect(),
            linked: Arc::new(Linked::new(pages, Vec::new())),
            whole: Some(Whole { path: path.to_owned(), password: password.map(str::to_owned) }),
        })
    }
}

/// The pages of the PDF in `bytes`, opened with `password`, measured as pdfcraft's `inspect` measures
/// them: each its size as shown, its crop box and its turn, a page with no usable size as US Letter.
/// `None` where hayro-syntax can't read the file.
fn measured(bytes: Arc<Vec<u8>>, password: Option<&str>) -> Option<Vec<PageInfo>> {
    use hayro_syntax::page::Rotation;
    let pdf = hayro_syntax::Pdf::new_with_password(bytes, password.unwrap_or("")).ok()?;
    let pages = pdf.pages().iter().enumerate().map(|(i, page)| {
        let (w, h) = page.render_dimensions();
        let (w, h) = if w.is_finite() && h.is_finite() && w >= 1.0 && h >= 1.0 { (w, h) } else { (612.0, 792.0) };
        let c = page.intersected_crop_box();
        let rotation = match page.rotation() {
            Rotation::None => 0,
            Rotation::Horizontal => 90,
            Rotation::Flipped => 180,
            Rotation::FlippedHorizontal => 270,
        };
        PageInfo { width: w, height: h, label: (i + 1).to_string(), crop: [c.x0 as f32, c.y0 as f32, c.x1 as f32, c.y1 as f32], rotation }
    });
    Some(pages.collect())
}

impl Source for Pdf {
    fn pages(&self) -> usize {
        self.sizes.len()
    }

    fn size(&self, page: usize) -> Option<(f32, f32)> {
        self.sizes.get(page).copied()
    }

    fn natural(&self, _page: usize) -> bool {
        false
    }

    fn renderer(&self) -> Result<Box<dyn super::Render + '_>, String> {
        let renderer = PageRenderer::new(self.bytes.clone(), self.config.clone());
        // `inspect` read the file, but the renderer's own parser may not.
        if renderer.page_count() == 0 {
            return Err("the renderer can't read this document".into());
        }
        Ok(Box::new(Renderer(renderer)))
    }

    fn pool(&self, threads: usize) -> Box<dyn super::Pool> {
        Box::new(Pool(RenderPool::new(self.bytes.clone(), threads, self.config.clone())))
    }

    /// Its pages' text, read in a worker from here on: of the copy shown, opened with the
    /// password that opened it.
    fn texts(&self) -> Option<Box<dyn Texts>> {
        Some(Box::new(PagesText::new(self.bytes.clone(), self.config.password.clone(), self.sizes.len())))
    }

    /// Its link annotations, as read when it was opened; a named destination is looked up in the
    /// copy shown, with the password that opened it.
    fn links(&self) -> Option<Box<dyn Links>> {
        Some(Box::new(PageLinks::new(self.bytes.clone(), self.config.password.clone(), self.linked.clone())))
    }

    /// A tile's, the file opened whole; nothing else lays out again.
    fn relayout(&self, _values: Values, _room: Room) -> Option<Job> {
        let (whole, bytes) = (self.whole.clone()?, self.bytes.clone());
        Some(Box::new(move || Pdf::open(&whole.path, bytes, whole.password.as_deref()).map(|pdf| Box::new(pdf) as Box<dyn Source + Send>)))
    }

    fn tile(&self) -> bool {
        self.whole.is_some()
    }
}

/// A PDF's links for the viewer ([`Links`]), answered on the queue's thread: a page's from what
/// was read as the file was opened, and where a named destination is by reading the file again,
/// which is why it has a thread.
struct PageLinks {
    queue: Queue<Told>,
}

impl PageLinks {
    fn new(bytes: Arc<Vec<u8>>, password: Option<Arc<str>>, linked: Arc<Linked>) -> Self {
        let queue = Queue::new();
        queue.serve("pdf links", move |ask| match ask {
            Ask::Page(page) => {
                let (links, more) = linked.on(page);
                Told::Page { page, links: Ok(links), more }
            }
            Ask::Name(name) => {
                let found = (name.len() <= MAX_NAME).then(|| pdfcraft_render::named_destination(bytes.clone(), password.as_deref(), &name)).flatten();
                let at = found.and_then(|(page, view)| linked.pages.get(page).map(|info| (page, down(info, view))));
                Told::Name { name, at: Ok(at) }
            }
        });
        Self { queue }
    }
}

impl Links for PageLinks {
    fn set_queue(&self, wanted: Vec<Ask>) {
        self.queue.set_queue(wanted);
    }

    fn try_recv(&self) -> Option<Told> {
        self.queue.try_recv()
    }
}

struct Renderer(PageRenderer);

impl super::Render for Renderer {
    fn render(&mut self, request: Request) -> Frame {
        frame(self.0.render(pdfcraft_request(request)))
    }
}

/// pdfcraft's render threads and their watchdog: a page that takes too long is told to stop at
/// its next drawing operator, and comes back failed.
struct Pool(RenderPool);

impl super::Pool for Pool {
    fn set_queue(&self, requests: Vec<Request>) {
        self.0.set_queue(requests.into_iter().map(pdfcraft_request).collect());
    }

    fn try_recv(&self) -> Option<Frame> {
        self.0.try_recv().map(frame)
    }
}

fn pdfcraft_request(r: Request) -> RenderRequest {
    RenderRequest { page: r.page, kind: RequestKind::Pixels, tile: None, scale: r.scale, tag: r.tag, preview: false }
}

/// A page pdfcraft rendered, its warnings put into words.
fn frame(p: RenderedPage) -> Frame {
    let request = Request { page: p.request.page, scale: p.request.scale, tag: p.request.tag, ..Default::default() };
    let warnings = p
        .warnings
        .iter()
        .map(|w| match w {
            RenderWarning::ContentTruncated => "content past the page's safety budget was skipped",
            RenderWarning::ColourFallback => "a colour that could not be computed was painted in a fallback colour",
            RenderWarning::MalformedIndexedImage => "an image whose colour table is malformed may be drawn wrongly, or not at all",
            RenderWarning::Type3WidthsMissing => "a font without the widths it must give was spaced by its glyphs' own",
        })
        .map(String::from)
        .collect();
    Frame { request, width: p.width, height: p.height, rgba: Pixels::new(p.rgba), error: p.error, warnings, again: false }
}

// The text's worker: its requests and answers.

/// The password the PDF opens with, in UTF-8, empty for none. Answered by [`DONE`].
const PASSWORD: u8 = 1;

/// The PDF's bytes, parsed and kept. Answered by [`DONE`], or [`FAILED`] for one pdfcraft can't
/// parse.
const OPEN: u8 = 2;

/// A page's text: the page, from 0. Answered by [`TEXT_READ`].
const READ: u8 = 3;

/// Where the characters `from..to` of a page's text are: the page, `from` and `to`. Answered by
/// [`PLACED`].
const PLACE: u8 = 4;

/// Done, with nothing to say.
const DONE: u8 = 1;

/// A page's text as it is searched ([`searched`]), in UTF-8, at most [`MAX_PAGE_TEXT`].
const TEXT_READ: u8 = 2;

/// A range's rectangles, at most [`MAX_LINES`]: each its left, top, right and bottom in points
/// from the page's top left.
const PLACED: u8 = 3;

/// Why the worker answered [`FAILED`]: a request it couldn't read, or out of turn; a document or a
/// page pdfcraft couldn't read; a page with more text than is given.
const MALFORMED: u8 = 2;
const UNREAD: u8 = 3;
const TOO_LONG: u8 = 4;

/// The largest PDF whose text is read: the worker holds it whole, as the viewer does.
const MAX_PDF: usize = 512 << 20;

/// The most the worker's heap holds: the PDF, and 256 MiB to parse it and read a page's text in,
/// sixteen times the most a page of 1,542 documents took (ADR-0027).
const MAX_HEAP: usize = MAX_PDF + (256 << 20);

/// The longest password given to the worker.
const MAX_PASSWORD: usize = 1 << 16;

/// The most text a page gives, in bytes: what a search keeps of one chunk.
const MAX_PAGE_TEXT: usize = 1 << 20;

/// The most rectangles a range gives: one for each line it covers.
const MAX_LINES: usize = 64;

/// The CPU seconds parsing the PDF may take, and reading one page's text or placing a range of it
/// (which may read the page again): of 64,940 pages measured, two took longer (ADR-0027).
const OPEN_CPU: u64 = 5;
const PAGE_CPU: u64 = 5;

/// How long reading a document's text may take in all, the requests' time added up: past it, the
/// rest is left unread.
const DOCUMENT: Duration = Duration::from_secs(120);

/// How many workers may be killed reading a document's text, each costing its page: past it, the
/// rest is left unread.
const MAX_KILLS: u32 = 3;

/// The worker that reads a PDF's text: `clv --worker pdf-text`.
pub(crate) struct TextWorker;

pub(crate) static TEXT: TextWorker = TextWorker;

impl Worker for TextWorker {
    fn name(&self) -> &'static str {
        "pdf-text"
    }

    fn takes(&self, kind: u8) -> Option<(u64, usize)> {
        match kind {
            PASSWORD => Some((1, MAX_PASSWORD)),
            OPEN => Some((OPEN_CPU, MAX_PDF)),
            READ => Some((PAGE_CPU, 4)),
            PLACE => Some((PAGE_CPU, 12)),
            _ => None,
        }
    }

    fn heap(&self) -> usize {
        MAX_HEAP
    }

    fn session(&self) -> Box<dyn Session> {
        Box::new(Reading::default())
    }
}

/// What the worker keeps between requests: the password, the document once given, and the last
/// page read, for the ranges placed on it next.
#[derive(Default)]
struct Reading {
    password: Option<Arc<str>>,
    document: Option<PageRenderer>,
    page: Option<(u32, Read)>,
}

/// A page's text as pdfcraft read it, and for each glyph, the character of the text searched its
/// own text starts at.
struct Read {
    text: PageText,
    starts: Vec<u32>,
}

impl Session for Reading {
    fn answer(&mut self, kind: u8, payload: Vec<u8>) -> (u8, Vec<u8>) {
        let failed = |why: u8| (FAILED, vec![why]);
        match kind {
            PASSWORD => match String::from_utf8(payload) {
                Ok(password) => {
                    self.password = (!password.is_empty()).then(|| Arc::from(password));
                    (DONE, Vec::new())
                }
                Err(_) => failed(MALFORMED),
            },
            OPEN => {
                let config = RenderConfig { password: self.password.clone(), reject_oversize: true, ..Default::default() };
                let document = PageRenderer::new(Arc::new(payload), config);
                if document.page_count() == 0 {
                    return failed(UNREAD);
                }
                self.document = Some(document);
                self.page = None;
                (DONE, Vec::new())
            }
            READ => {
                let (Ok(page), Some(document)) = (<[u8; 4]>::try_from(payload.as_slice()).map(u32::from_le_bytes), self.document.as_mut()) else {
                    return failed(MALFORMED);
                };
                // The last page's goes first: two pages' glyphs are never held together.
                self.page = None;
                match read(document, page) {
                    Ok((text, read)) => {
                        self.page = Some((page, read));
                        (TEXT_READ, text.into_bytes())
                    }
                    Err(why) => failed(why),
                }
            }
            PLACE => {
                let (Ok([page, from, to]), Some(document)) = (numbers::<3>(&payload), self.document.as_mut()) else { return failed(MALFORMED) };
                if self.page.as_ref().is_none_or(|(read, _)| *read != page) {
                    self.page = None;
                    match read(document, page) {
                        Ok((_, read)) => self.page = Some((page, read)),
                        Err(why) => return failed(why),
                    }
                }
                let rects = self.page.as_ref().map_or_else(Vec::new, |(_, read)| read.placed(from, to));
                (PLACED, rects.iter().flatten().flat_map(|v| v.to_le_bytes()).collect())
            }
            _ => failed(MALFORMED),
        }
    }
}

/// `N` numbers of four bytes each, little-endian, which are all of `payload`.
fn numbers<const N: usize>(payload: &[u8]) -> Result<[u32; N], ()> {
    let mut out = [0u32; N];
    let (chunks, rest) = payload.as_chunks::<4>();
    if chunks.len() != N || !rest.is_empty() {
        return Err(());
    }
    for (number, bytes) in out.iter_mut().zip(chunks) {
        *number = u32::from_le_bytes(*bytes);
    }
    Ok(out)
}

/// Page `page`'s text as `document` reads it: the text as it is searched, and what places a range
/// of it; or why not ([`UNREAD`], [`TOO_LONG`]). All the text pdfcraft reads on the page, an
/// invisible layer's (OCR) too.
fn read(document: &mut PageRenderer, page: u32) -> Result<(String, Read), u8> {
    let text = usize::try_from(page).ok().and_then(|page| document.filtered_text(page, TextFilter::default()).ok()).ok_or(UNREAD)?;
    let (searched, starts) = searched(&text).ok_or(TOO_LONG)?;
    Ok((searched, Read { text, starts }))
}

/// A page's text as it is searched: its glyphs' text in pdfcraft's reading order, a space where it
/// found a gap between words and between one line and the next, so a word hyphenated at a line's
/// end is two words, as drawn. A ligature is the letters pdfcraft gives it, and they share its box.
/// With it, the character each glyph's text starts at. `None` past [`MAX_PAGE_TEXT`].
fn searched(text: &PageText) -> Option<(String, Vec<u32>)> {
    let mut out = String::new();
    let mut starts = Vec::with_capacity(text.glyphs.len());
    let mut chars = 0u32;
    let mut line = None;
    for (at, glyph) in text.glyphs.iter().enumerate() {
        let on = text.line_of.get(at).copied();
        let gap = text.space_before.get(at).copied().unwrap_or(false);
        if at > 0 && (on != line || gap) {
            out.push(' ');
            chars = chars.saturating_add(1);
        }
        line = on;
        starts.push(chars);
        out.push_str(&glyph.text);
        chars = chars.saturating_add(u32::try_from(glyph.text.chars().count()).unwrap_or(u32::MAX));
        if out.len() > MAX_PAGE_TEXT {
            return None;
        }
    }
    Some((out, starts))
}

impl Read {
    /// The rectangles the characters `from..to` of the text searched cover: the boxes of the glyphs
    /// they are in, merged along each line, at most [`MAX_LINES`]. None for a range that is only
    /// the spaces between glyphs, or isn't in the text.
    fn placed(&self, from: u32, to: u32) -> Vec<[f32; 4]> {
        // The glyph `from` is in: the last that starts at or before it, if its text reaches it;
        // otherwise the next. And the glyphs that start before `to`.
        let ends = |at: usize| {
            let chars = self.text.glyphs.get(at).map_or(0, |glyph| glyph.text.chars().count());
            self.starts.get(at).copied().unwrap_or(u32::MAX).saturating_add(u32::try_from(chars).unwrap_or(u32::MAX))
        };
        let before = self.starts.partition_point(|start| *start <= from).saturating_sub(1);
        let first = if ends(before) > from { before } else { before.saturating_add(1) };
        let last = self.starts.partition_point(|start| *start < to);
        if from >= to || first >= last {
            return Vec::new();
        }
        let rects = self.text.line_rects(first..last).into_iter().filter(|rect| rect.iter().all(|v| v.is_finite()));
        rects.take(MAX_LINES).collect()
    }
}

// The viewer's side.

/// A PDF's text for a search ([`Texts`]): a chunk a page, read on the queue's thread
/// ([`texts::Queue`]), which asks a worker for what is wanted, a page's text or a range's
/// rectangles at a time, and starts another worker past one killed. Dropped, it kills the worker
/// without waiting for it.
struct PagesText {
    queue: Queue,
    pages: usize,
}

impl PagesText {
    fn new(bytes: Arc<Vec<u8>>, password: Option<Arc<str>>, pages: usize) -> Self {
        let queue: Queue = Queue::new();
        let mut reader = Reader { end: queue.ending(), bytes, password, link: None, kills: 0, spent: Duration::ZERO, ended: None };
        if reader.bytes.len() > MAX_PDF {
            let (size, most) = (reader.bytes.len() >> 20, MAX_PDF >> 20);
            reader.ended = Some(format!("this PDF is too large to search: {size} MiB, and the most is {most}"));
        } else if reader.password.as_ref().is_some_and(|password| password.len() > MAX_PASSWORD) {
            reader.ended = Some("its password is too long for its text to be read".into());
        }
        queue.start("pdf text", reader);
        Self { queue, pages }
    }
}

impl Texts for PagesText {
    fn chunks(&self) -> usize {
        self.pages
    }

    fn chunks_on(&self, page: usize) -> Range<usize> {
        page..page.saturating_add(1)
    }

    fn set_queue(&self, wanted: Vec<Want>) {
        self.queue.set_queue(wanted);
    }

    fn try_recv(&self) -> Option<Got> {
        self.queue.try_recv()
    }
}

/// The thread's side of a [`PagesText`].
struct Reader {
    /// What the search's end kills: the worker, while there is one.
    end: Ending,
    bytes: Arc<Vec<u8>>,
    password: Option<Arc<str>>,
    /// The worker, once started and given the document.
    link: Option<Link>,
    /// Workers killed so far, and the time the requests took.
    kills: u32,
    spent: Duration,
    /// Why nothing more is read, once nothing is.
    ended: Option<String>,
}

/// What became of a request to the worker.
enum Asked {
    /// Its answer.
    Answer(u8, Vec<u8>),
    /// The worker is gone, killed or dead, and why.
    Gone(Died),
    /// No worker could be started, in words for the reader: none will be.
    Never(String),
}

impl Answer for Reader {
    /// Page `page`'s text, from the worker; or why not, for the reader.
    fn text(&mut self, page: usize) -> Result<String, String> {
        if self.ended.is_none() && self.spent > DOCUMENT {
            self.ended = Some(format!("its text took more than {} s to read: the rest is left unread", DOCUMENT.as_secs()));
        }
        if let Some(why) = &self.ended {
            return Err(why.clone());
        }
        let number = page.saturating_add(1);
        let Ok(at) = u32::try_from(page) else { return Err(format!("page {number} is past the pages whose text is read")) };
        match self.ask(READ, at.to_le_bytes().to_vec(), PAGE_CPU, MAX_PAGE_TEXT) {
            Asked::Answer(TEXT_READ, text) => match String::from_utf8(text) {
                Ok(text) => Ok(text),
                Err(_) => Err(self.refuse(number)),
            },
            Asked::Answer(FAILED, why) if why == [TOO_LONG] => Err(format!("page {number} has more text than a search keeps of one page")),
            Asked::Answer(FAILED, why) if why == [UNREAD] => Err(format!("pdfcraft couldn't read page {number}'s text")),
            Asked::Answer(..) => Err(self.refuse(number)),
            Asked::Gone(died) => Err(self.gone(died, number)),
            Asked::Never(why) => Err(why),
        }
    }

    /// Where the characters `from..to` of page `page`'s text are, from the worker: each rectangle
    /// on that page. The time it takes isn't the document's, which reading alone spends.
    fn rects(&mut self, page: usize, from: usize, to: usize) -> Result<Vec<(usize, [f32; 4])>, String> {
        if let Some(why) = self.ended.as_ref().filter(|_| self.kills > MAX_KILLS || self.link.is_none()) {
            return Err(why.clone());
        }
        let number = page.saturating_add(1);
        let (Ok(at), Ok(from), Ok(to)) = (u32::try_from(page), u32::try_from(from), u32::try_from(to)) else {
            return Err(format!("page {number} is past the pages whose text is read"));
        };
        let payload = [at, from, to].iter().flat_map(|v| v.to_le_bytes()).collect();
        let spent = self.spent;
        let asked = self.ask(PLACE, payload, PAGE_CPU, MAX_LINES.saturating_mul(16));
        self.spent = spent;
        match asked {
            Asked::Answer(PLACED, rects) => {
                let (rects, rest) = rects.as_chunks::<16>();
                if !rest.is_empty() {
                    return Err(self.refuse(number));
                }
                let side = |bytes: &[u8]| <[u8; 4]>::try_from(bytes).map_or(f32::NAN, f32::from_le_bytes);
                let rect = |rect: &[u8; 16]| {
                    let (sides, _) = rect.as_chunks::<4>();
                    let mut out = [f32::NAN; 4];
                    out.iter_mut().zip(sides).for_each(|(out, bytes)| *out = side(bytes));
                    (page, out)
                };
                Ok(rects.iter().map(rect).collect())
            }
            Asked::Answer(FAILED, why) if why == [TOO_LONG] || why == [UNREAD] => Ok(Vec::new()),
            Asked::Answer(..) => Err(self.refuse(number)),
            Asked::Gone(died) => Err(self.gone(died, number)),
            Asked::Never(why) => Err(why),
        }
    }
}

impl Reader {
    /// A request of `kind` to the worker, started and given the document first if there is none, its
    /// answer at most `longest` bytes and `cpu` seconds' work.
    fn ask(&mut self, kind: u8, payload: Vec<u8>, cpu: u64, longest: usize) -> Asked {
        if self.link.is_none() {
            match self.start() {
                Ok(link) => self.link = Some(link),
                Err(asked) => return asked,
            }
        }
        let Some(link) = self.link.as_mut() else { return Asked::Never("no process to read its text in".into()) };
        let begun = Instant::now();
        let answer = link.request(kind, payload, wall(cpu), longest);
        self.spent = self.spent.saturating_add(begun.elapsed());
        match answer {
            Ok((kind, payload)) => Asked::Answer(kind, payload),
            Err(died) => Asked::Gone(died),
        }
    }

    /// A worker started and given the password and the document; the search can kill it from here
    /// on. Or what became of the request that was to be asked of it.
    fn start(&mut self) -> Result<Link, Asked> {
        let mut link = match worker::spawn(&TEXT) {
            Ok(link) => link,
            Err(Died::TooMany) => {
                let why = format!("too many of {}'s worker processes at once: its text is left unread", crate::NAME);
                self.ended = Some(why.clone());
                return Err(Asked::Never(why));
            }
            Err(died) => return Err(Asked::Gone(died)),
        };
        let kill = link.killer();
        if !self.end.on_end(Some(Box::new(move || kill.kill()))) {
            return Err(Asked::Gone(Died::Ended("let go of".into())));
        }
        let password = self.password.as_deref().unwrap_or("").as_bytes().to_vec();
        let begun = Instant::now();
        let opened = link.request(PASSWORD, password, wall(1), 1).and_then(|said| match said {
            (DONE, _) => link.request_shared(OPEN, self.bytes.clone(), wall(OPEN_CPU), 1),
            other => Ok(other),
        });
        self.spent = self.spent.saturating_add(begun.elapsed());
        match opened {
            Ok((DONE, _)) => Ok(link),
            Ok((FAILED, why)) if why == [UNREAD] => {
                let why = "pdfcraft couldn't read this document for its text".to_owned();
                self.ended = Some(why.clone());
                Err(Asked::Never(why))
            }
            Ok(_) => Err(Asked::Gone(link.refuse())),
            Err(died) => Err(Asked::Gone(died)),
        }
    }

    /// The worker said what it isn't asked to say, on page `number`: killed, and why that page is
    /// unread ([`Reader::gone`]).
    fn refuse(&mut self, number: usize) -> String {
        let died = self.link.as_mut().map_or(Died::Malformed, Link::refuse);
        self.gone(died, number)
    }

    /// The worker is gone, for `died`, on page `number`: why that page is unread, for the reader. A
    /// worker killed is counted, and past [`MAX_KILLS`] of them the rest is left unread; the next
    /// request starts another.
    fn gone(&mut self, died: Died, number: usize) -> String {
        self.link = None;
        self.end.on_end(None);
        self.kills = self.kills.saturating_add(1);
        let why = match died {
            Died::TimedOut | Died::Cpu => format!("page {number}'s text took too long to read"),
            Died::Memory => format!("page {number}'s text took too much memory to read"),
            Died::Version => {
                let why = format!("{} was replaced while it ran: open the file again to search it", crate::NAME);
                self.ended = Some(why.clone());
                return why;
            }
            Died::TooMany | Died::Unstarted(_) => format!("no process to read page {number}'s text in"),
            Died::Ended(_) | Died::Malformed => format!("pdfcraft failed on page {number}'s text"),
        };
        if self.kills > MAX_KILLS && self.ended.is_none() {
            self.ended = Some("reading its text failed too often: the rest is left unread".into());
        }
        why
    }
}

/// How long the viewer waits for an answer of `cpu` seconds' work.
fn wall(cpu: u64) -> Duration {
    Duration::from_secs(cpu.saturating_mul(u64::from(WALL)))
}

/// What is drawn for a file, as the pdfcraft app draws it.
struct Displayed {
    /// The display copy the renderer reads: comments missing their /AP streams are drawn, and a
    /// dynamic XFA form is laid out. The file itself is left intact.
    bytes: Arc<Vec<u8>>,
    /// The password for the renderer, which takes only the user password of R2–R4 files: for one
    /// opened with its owner password, the user password that recovers.
    password: Option<String>,
    /// The displayed document's pages and the rest (a laid-out form's, not its placeholder's),
    /// or why the renderer can't open the file.
    info: Result<pdfcraft_render::DocInfo, String>,
}

/// [`Displayed`] for the file `bytes` read from `path`, opened with `password` (either one).
fn displayed(path: &str, bytes: Arc<Vec<u8>>, password: Option<&str>) -> Result<Displayed, String> {
    // `Session::open` and the display copy take either password; the renderer doesn't.
    let render_password = pdfcraft_engine::render_password_for(bytes.clone(), password);
    let info = pdfcraft_render::inspect(bytes.clone(), render_password.as_deref()).map_err(|e| e.to_string());
    if info.as_ref().is_ok_and(|info| info.xfa == Some(pdfcraft_render::Xfa::Dynamic)) {
        let mut session = pdfcraft_engine::Session::new();
        let id = session.open(path, None, bytes, password).map_err(|e| e.to_string())?;
        let doc = session.get(id).ok_or("the document could not be opened")?;
        return Ok(Displayed { bytes: doc.display.clone(), password: render_password, info: Ok(doc.info.clone()) });
    }
    Ok(Displayed { bytes: pdfcraft_engine::display_bytes_for(bytes, password), password: render_password, info })
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, PoisonError};

    use super::*;

    /// No test file makes a page fail to render, so this is what says a render's error gets through.
    #[test]
    fn a_frame_keeps_why_its_render_failed() {
        let request = RenderRequest { page: 2, kind: RequestKind::Pixels, tile: None, scale: 1.5, tag: 9, preview: false };
        let rgba = pdfcraft_render::Pixels::from(vec![0xFF80_8080; 2]);
        let failed =
            RenderedPage { request, width: 2, height: 1, rgba, error: Some("no page box".into()), text: None, warnings: Vec::new(), millis: 0 };
        assert_eq!(frame(failed).error.as_deref(), Some("no page box"));
    }

    /// A PDF of one 300 × 300 pt page per content stream of `pages`, in Helvetica as `/F1`.
    fn pdf(pages: &[&str]) -> Vec<u8> {
        let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            format!("<< /Type /Pages /Kids [{}] /Count {} /MediaBox [0 0 300 300] >>", kids.join(" "), pages.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        ];
        for (i, content) in pages.iter().enumerate() {
            objects.push(format!("<< /Type /Page /Parent 2 0 R /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>", 5 + 2 * i));
            objects.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
        }
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
        out
    }

    /// Two lines of 12 pt text, the first drawn in two strings that meet inside a word, the second
    /// starting with the rest of a word hyphenated at the first's end; and a page with none.
    const LINES: &str = "BT /F1 12 Tf 20 250 Td (Search wor) Tj (ds well-) Tj 0 -40 Td (known here) Tj ET";

    /// A worker's session given `document`, as the viewer gives it.
    fn given(document: &[u8], password: &str) -> Reading {
        let mut reading = Reading::default();
        assert_eq!(reading.answer(PASSWORD, password.as_bytes().to_vec()), (DONE, Vec::new()));
        assert_eq!(reading.answer(OPEN, document.to_vec()), (DONE, Vec::new()));
        reading
    }

    /// The rectangles the session answers for the characters `from..to` of `page`.
    fn placed(reading: &mut Reading, page: u32, from: u32, to: u32) -> Vec<[f32; 4]> {
        let (kind, rects) = reading.answer(PLACE, [page, from, to].iter().flat_map(|v| v.to_le_bytes()).collect());
        assert_eq!((kind, rects.len() % 16), (PLACED, 0));
        let side = |bytes: &[u8]| f32::from_le_bytes(bytes.try_into().unwrap());
        rects.chunks(16).map(|rect| [side(&rect[..4]), side(&rect[4..8]), side(&rect[8..12]), side(&rect[12..])]).collect()
    }

    /// A page's text as it is searched: its words as drawn, a space between words and between
    /// lines, so a word hyphenated at a line's end is two words. A page without text is empty, and
    /// a page that isn't there is unread.
    #[test]
    fn a_pages_text_is_its_words_with_a_space_between_lines() {
        let mut reading = given(&pdf(&[LINES, "0 0 10 10 re f"]), "");
        assert_eq!(reading.answer(READ, 0u32.to_le_bytes().to_vec()), (TEXT_READ, b"Search words well- known here".to_vec()));
        assert_eq!(reading.answer(READ, 1u32.to_le_bytes().to_vec()), (TEXT_READ, Vec::new()));
        assert_eq!(reading.answer(READ, 2u32.to_le_bytes().to_vec()), (FAILED, vec![UNREAD]));
        // Out of turn, or not what it is asked.
        assert_eq!(reading.answer(READ, vec![0; 3]), (FAILED, vec![MALFORMED]));
        assert_eq!(reading.answer(PLACE, vec![0; 11]), (FAILED, vec![MALFORMED]));
        assert_eq!(reading.answer(99, Vec::new()), (FAILED, vec![MALFORMED]));
        assert_eq!(Reading::default().answer(READ, 0u32.to_le_bytes().to_vec()), (FAILED, vec![MALFORMED]), "before any document");
        assert_eq!(Reading::default().answer(OPEN, b"not a PDF".to_vec()), (FAILED, vec![UNREAD]));
        assert_eq!(Reading::default().answer(PASSWORD, vec![0xFF]), (FAILED, vec![MALFORMED]));
    }

    /// Where a range of a page's text is: the boxes of its glyphs, merged along each line, in points
    /// from the page's top left; one for each line it covers, none for the space between two words
    /// or a range outside the text.
    #[test]
    fn a_range_of_a_pages_text_is_a_rectangle_for_each_line_it_covers() {
        let mut reading = given(&pdf(&[LINES, "BT /F1 12 Tf 20 250 Td (other) Tj ET"]), "");
        // "words", across the two strings: one rectangle on the first line, 50 pt down a 300 pt page.
        let words = placed(&mut reading, 0, 7, 12);
        let [[x0, y0, x1, y1]] = words[..] else { panic!("{words:?}") };
        assert!(x0 > 20.0 && x1 > x0 + 20.0 && x1 < 120.0, "{words:?}");
        assert!((35.0..50.0).contains(&y0) && (50.0..56.0).contains(&y1), "{words:?}");
        // "well- known", across the line's end: a rectangle on each line, the second 40 pt lower.
        let both = placed(&mut reading, 0, 13, 24);
        let [first, second] = both[..] else { panic!("{both:?}") };
        assert!(first[0] > x1 && (first[1] - y0).abs() < 1.0 && (second[1] - y0 - 40.0).abs() < 1.0 && second[0] < 25.0, "{both:?}");
        // Narrower for fewer letters; the whole text; and what covers nothing.
        let one = placed(&mut reading, 0, 0, 1);
        assert!(one.len() == 1 && one[0][2] - one[0][0] < 12.0, "{one:?}");
        assert_eq!(placed(&mut reading, 0, 0, 1000).len(), 2);
        let space = placed(&mut reading, 0, 6, 7);
        assert!(space.len() == 1 && space[0][2] - space[0][0] < 5.0, "a space drawn is a glyph: {space:?}");
        assert_eq!(placed(&mut reading, 0, 18, 19), Vec::<[f32; 4]>::new(), "the space a line's end became is none");
        assert_eq!(placed(&mut reading, 0, 12, 5), Vec::<[f32; 4]>::new(), "backwards");
        assert_eq!(placed(&mut reading, 0, 500, 600), Vec::<[f32; 4]>::new());
        // Another page's, read for it, and back: the page kept follows what is asked.
        assert_eq!(placed(&mut reading, 1, 0, 5).len(), 1);
        assert_eq!(placed(&mut reading, 0, 7, 12), words);
        assert_eq!(reading.answer(PLACE, [9u32, 0, 1].iter().flat_map(|v| v.to_le_bytes()).collect()), (FAILED, vec![UNREAD]));
    }

    /// One at a time: each starts a worker, and the workers alive at once are counted.
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    /// What `texts` gives for `wanted`, each once, within ten seconds.
    fn gave(texts: &dyn Texts, wanted: Vec<Want>) -> Vec<Got> {
        let (mut got, begun) = (Vec::new(), Instant::now());
        texts.set_queue(wanted.clone());
        while got.len() < wanted.len() && begun.elapsed() < Duration::from_secs(10) {
            // Listed again while waited for, as the viewer may: given once all the same.
            texts.set_queue(wanted.clone());
            match texts.try_recv() {
                Some(one) => got.push(one),
                None => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        std::thread::sleep(Duration::from_millis(20));
        assert!(texts.try_recv().is_none(), "given twice");
        got
    }

    /// A PDF's text through its worker, as the viewer asks for it: a chunk a page, each page's text,
    /// a range's rectangles on its page, and what isn't wanted any more never given.
    #[test]
    fn a_pdfs_text_comes_a_page_a_chunk_through_its_worker() {
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
        let source = Pdf::open("lines.pdf", Arc::new(pdf(&[LINES, "BT /F1 12 Tf 20 250 Td (other) Tj ET"])), None).unwrap();
        let texts = source.texts().unwrap();
        assert_eq!((texts.chunks(), texts.chunks_on(0), texts.chunks_on(1)), (2, 0..1, 1..2));
        let rects = Want::Rects { chunk: 0, from: 7, to: 12 };
        let got = gave(&*texts, vec![Want::Text(1), rects, Want::Text(0), Want::Text(7)]);
        let [Got::Text(1, Ok(other)), Got::Rects(asked, Ok(placed)), Got::Text(0, Ok(lines)), Got::Text(7, Err(why))] = &got[..] else {
            panic!("{} given", got.len())
        };
        assert_eq!((other.as_str(), lines.as_str(), asked), ("other", "Search words well- known here", &rects));
        assert!(matches!(placed[..], [(0, [x0, _, x1, _])] if x0 > 20.0 && x1 > x0), "{placed:?}");
        assert_eq!(why, "pdfcraft couldn't read page 8's text");
        // Nothing wanted: nothing given.
        texts.set_queue(Vec::new());
        std::thread::sleep(Duration::from_millis(20));
        assert!(texts.try_recv().is_none());
    }

    /// Past the time a document's text may take in all, the rest is left unread, each page asked
    /// answered with why; where the matches read are is still answered. A password too long to give
    /// the worker leaves it all unread.
    #[test]
    fn past_the_documents_time_the_rest_of_its_text_is_left_unread() {
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
        let queue: Queue = Queue::new();
        let bytes = Arc::new(pdf(&[LINES, LINES]));
        let mut reader = Reader { end: queue.ending(), bytes, password: None, link: None, kills: 0, spent: Duration::ZERO, ended: None };
        assert_eq!(reader.text(0).as_deref(), Ok("Search words well- known here"));
        assert!(reader.spent > Duration::ZERO && reader.spent < DOCUMENT);
        reader.spent = DOCUMENT + Duration::from_millis(1);
        let why = "its text took more than 120 s to read: the rest is left unread";
        assert_eq!((reader.text(1), reader.text(0)), (Err(why.into()), Err(why.into())));
        assert_eq!(reader.rects(0, 7, 12).map(|rects| rects.len()), Ok(1), "the worker is still there for the matches read");
        assert_eq!(reader.spent, DOCUMENT + Duration::from_millis(1), "placing a match isn't the reading's time");
        // Past the kills, nothing more is asked of any worker; why the reading ended stays what it was.
        reader.kills = MAX_KILLS;
        assert_eq!(reader.gone(Died::Memory, 2), "page 2's text took too much memory to read");
        assert_eq!(reader.rects(0, 7, 12), Err(why.into()));
        reader.ended = None;
        assert_eq!(reader.gone(Died::Cpu, 2), "page 2's text took too long to read");
        assert_eq!(reader.text(0), Err("reading its text failed too often: the rest is left unread".into()));
        let source = Pdf::open("lines.pdf", Arc::new(pdf(&[LINES])), None).unwrap();
        let texts = PagesText::new(source.bytes.clone(), Some(Arc::from("x".repeat(MAX_PASSWORD + 1))), 1);
        let got = gave(&texts, vec![Want::Text(0)]);
        assert!(matches!(&got[..], [Got::Text(0, Err(why))] if why == "its password is too long for its text to be read"));
    }

    /// What `links` gives for `wanted`, each once, within ten seconds.
    fn told(links: &dyn Links, wanted: Vec<Ask>) -> Vec<Told> {
        let (mut got, begun) = (Vec::new(), Instant::now());
        while got.len() < wanted.len() && begun.elapsed() < Duration::from_secs(10) {
            links.set_queue(wanted.clone());
            match links.try_recv() {
                Some(one) => got.push(one),
                None => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        std::thread::sleep(Duration::from_millis(20));
        assert!(links.try_recv().is_none(), "given twice");
        got
    }

    /// Page `page`'s links, as its source gives them.
    fn links_on(links: &dyn Links, page: usize) -> Vec<super::super::Link> {
        let mut got = told(links, vec![Ask::Page(page)]);
        let Some(Told::Page { page: answered, links: Ok(links), more: 0 }) = got.pop() else { panic!("no links for page {page}") };
        assert_eq!(answered, page);
        links
    }

    /// A list's tile is the file itself, its pages measured as the file opened whole measures them (a
    /// page cropped and turned, one of 2-point units), and no link read; its relayout opens the file
    /// whole. A file hayro-syntax can't read has no tile, and is opened whole, which says why.
    #[test]
    fn a_tile_measures_the_pages_as_the_whole_file_does_and_opens_it_whole() {
        let bytes = Arc::new(fixtures::linked());
        let whole = Pdf::open("links.pdf", bytes.clone(), None).unwrap();
        let tile = Pdf::tile("links.pdf", bytes, None).unwrap();
        assert!(tile.tile() && !whole.tile());
        assert_eq!(tile.sizes, whole.sizes);
        let crops = |pdf: &Pdf| pdf.linked.pages.iter().map(|page| (page.crop, page.rotation)).collect::<Vec<_>>();
        assert_eq!(crops(&tile), crops(&whole));
        assert!(tile.linked.links.is_empty() && !whole.linked.links.is_empty());
        let room = Room { cols: 60, rows: 40, cell_w: 10.0, cell_h: 20.0, gap: 1, per_row: None };
        let opened = tile.relayout(Values::default(), room).expect("a job")().unwrap();
        assert!(!opened.tile() && opened.pages() == whole.pages() && opened.relayout(Values::default(), room).is_none());
        assert!(Pdf::tile("broken.pdf", Arc::new(b"%PDF-1.7\nno more".to_vec()), None).is_none());
    }

    /// A PDF's links are its link annotations, each where it is active on its page as shown, in
    /// points from the page's top left: on a page as it is, on one cropped and turned a quarter, on
    /// one in units of two points; a link active over its quadrilaterals has a rectangle for each.
    #[test]
    fn a_pdfs_links_are_where_they_are_active_on_their_pages_as_shown() {
        let source = Pdf::open("links.pdf", Arc::new(fixtures::linked()), None).unwrap();
        assert_eq!((source.size(0), source.size(1), source.size(2)), (Some((612.0, 792.0)), Some((600.0, 400.0)), Some((600.0, 600.0))));
        let links = source.links().unwrap();
        let first = links_on(&*links, 0);
        assert_eq!(first.len(), 14);
        let near = |rect: [f32; 4], to: [f32; 4]| rect.iter().zip(to).all(|(side, to)| (side - to).abs() < 0.01);
        for (i, link) in first.iter().take(13).enumerate() {
            let top = 72.0 + 40.0 * i as f32;
            assert!(link.page == 0 && link.rects.len() == 1 && near(link.rects[0], [72.0, top, 272.0, top + 20.0]), "link {i}: {:?}", link.rects);
        }
        let lines = &first[13].rects;
        assert!(
            lines.len() == 2 && near(lines[0], [72.0, 592.0, 300.0, 612.0]) && near(lines[1], [72.0, 612.0, 200.0, 632.0]),
            "a rectangle a line: {lines:?}"
        );
        // `/Rect [150 600 250 650]` in a box `[100 100 500 700]` turned a quarter clockwise: what
        // was 50 to 100 units under the box's top is as far from the right edge of the 600 pt shown.
        let turned = links_on(&*links, 1);
        assert_eq!(turned.len(), 1);
        assert!(near(turned[0].rects[0], [500.0, 50.0, 550.0, 150.0]), "{:?}", turned[0].rects);
        // `/Rect [30 240 130 270]` of 300 × 300 units of 2 pt.
        let doubled = links_on(&*links, 2);
        assert!(doubled.len() == 1 && near(doubled[0].rects[0], [60.0, 60.0, 260.0, 120.0]), "{:?}", doubled[0].rects);
        // A page the document hasn't has none.
        assert!(links_on(&*links, 3).is_empty() && links_on(&*links, usize::MAX).is_empty());
    }

    /// Where a PDF's links lead: a page and how far down it as shown, for each kind of destination,
    /// on a page turned and cropped too, where `/XYZ` goes by its `left`; a web address; another
    /// file, with the place or the page in it; and what is never followed, said in words. Only a
    /// web address and a file have an address to copy.
    #[test]
    fn a_pdfs_links_lead_to_places_addresses_and_files() {
        let source = Pdf::open("links.pdf", Arc::new(fixtures::linked()), None).unwrap();
        let links = source.links().unwrap();
        let first = links_on(&*links, 0);
        let leads: Vec<(Option<&str>, &Target)> = first.iter().map(|link| (link.address.as_deref(), &link.target)).collect();
        let place = |page: usize, y: f32| Target::Place { page, y };
        let file =
            |fragment: Option<&str>, page: Option<usize>| Target::File { path: "other.pdf".into(), fragment: fragment.map(String::from), page };
        // `/XYZ 0 200` of 300 units, each 2 pt: a third of the way down 600 pt.
        assert_eq!(leads[0], (None, &place(2, 200.0)));
        // The destination `chapter`, `/XYZ 300 400` on the turned page: half way along the 400
        // units its box is wide, which run down the 400 pt it is tall as shown.
        assert_eq!(leads[1], (None, &place(1, 200.0)));
        assert_eq!(leads[2], (Some("https://example.com/a?b=c"), &Target::Web("https://example.com/a?b=c".into())));
        assert_eq!(leads[3], (Some("other.pdf#intro"), &file(Some("intro"), None)));
        assert_eq!(leads[4], (Some("other.pdf"), &file(None, Some(2))));
        // On the turned page: `/FitH`'s top runs across it, and `/FitV`, `/Fit` and nothing given
        // are its top; `/FitR 200 300 400 500` is the box from 100 to 300 pt down it.
        assert_eq!((leads[5], leads[6]), ((None, &place(1, 0.0)), (None, &place(1, 0.0))));
        let (None, Target::Place { page: 1, y }) = leads[7] else { panic!("{:?}", leads[7]) };
        assert!((y - 100.0).abs() < 0.01, "{y}");
        assert_eq!((leads[8], leads[9]), ((None, &place(1, 0.0)), (None, &place(1, 0.0))));
        // On a page as it is, `/FitR 72 500 300 700` is 92 pt under its top.
        assert_eq!(leads[10], (None, &place(0, 92.0)));
        assert_eq!(leads[11], (None, &Target::Other("a script, which clv doesn't run".into())));
        assert_eq!(leads[12], (None, &Target::Other("a link that starts a program, which clv doesn't".into())));
        assert_eq!(leads[13], (None, &place(2, 0.0)));
        let turned = links_on(&*links, 1);
        assert_eq!((turned[0].address.as_deref(), &turned[0].target), (None, &place(0, 0.0)));
        let doubled = links_on(&*links, 2);
        assert_eq!(doubled[0].target, Target::Web("mailto:someone@example.com".into()));
    }

    /// Each kind of destination on a page as it is, turned each way, cropped, and in larger units:
    /// how far down the page as shown it is. `/XYZ` goes by its `top`, or on a page turned a
    /// quarter by its `left`; `/FitH` by its `top`; `/FitR` by the top of its rectangle as shown;
    /// the rest, and an operand left out or off the page, by the page's top or its nearest edge.
    #[test]
    fn links_destinations_go_down_the_page_as_shown() {
        // A box of 400 × 600 units at (100, 100), as it is and turned; as shown, 400 × 600 or 600 × 400.
        let page = |rotation: u16, unit: f32| {
            let (width, height) = if rotation.is_multiple_of(180) { (400.0, 600.0) } else { (600.0, 400.0) };
            PageInfo { width: width * unit, height: height * unit, label: String::new(), crop: [100.0, 100.0, 500.0, 700.0], rotation }
        };
        let xyz = |left: Option<f32>, top: Option<f32>| DestView::Xyz { left, top, zoom: None };
        let near = |y: f32, to: f32| (y - to).abs() < 0.01;
        let rect = DestView::FitR { rect: [200.0, 300.0, 400.0, 500.0] };
        // As it is: by `top`, 700 the page's top edge.
        assert!(near(down(&page(0, 1.0), xyz(Some(300.0), Some(550.0))), 150.0));
        assert!(near(down(&page(0, 1.0), DestView::FitH { top: Some(550.0) }), 150.0));
        assert!(near(down(&page(0, 1.0), rect), 200.0));
        assert!(near(down(&page(0, 2.0), xyz(None, Some(550.0))), 300.0), "in units of 2 pt");
        // Turned a quarter clockwise: `left` runs down the page shown; anticlockwise, up it.
        assert!(near(down(&page(90, 1.0), xyz(Some(200.0), Some(550.0))), 100.0));
        assert!(near(down(&page(270, 1.0), xyz(Some(200.0), Some(550.0))), 300.0));
        assert!(near(down(&page(90, 1.0), rect), 100.0) && near(down(&page(270, 1.0), rect), 100.0));
        // Upside down: `top` runs up the page shown.
        assert!(near(down(&page(180, 1.0), xyz(Some(200.0), Some(550.0))), 450.0));
        assert!(near(down(&page(180, 1.0), rect), 200.0));
        for turn in [0, 90, 180, 270] {
            let page = page(turn, 1.0);
            for top in [DestView::Top, DestView::Fit, DestView::FitV { left: Some(300.0) }, xyz(None, None), DestView::FitH { top: None }] {
                assert_eq!(down(&page, top), 0.0, "{top:?} turned {turn}");
            }
            // Off the page, or no number: an edge of the page, never past it.
            for wild in [f32::NAN, f32::INFINITY, -1e30, 1e30] {
                let y = down(&page, xyz(Some(wild), Some(wild)));
                assert!((0.0..=page.height).contains(&y), "{wild} turned {turn}: {y}");
                let y = down(&page, DestView::FitR { rect: [wild, wild, 300.0, 300.0] });
                assert!((0.0..=page.height).contains(&y), "{wild} turned {turn}: {y}");
            }
        }
        let broken = PageInfo { height: f32::NAN, ..page(0, 1.0) };
        assert_eq!(down(&broken, xyz(None, Some(550.0))), 0.0);
    }

    /// A page's links past the most a page gives are counted, not given; a web address or a
    /// place's name too long to follow, a page the document hasn't, and a path that is no local
    /// file's are said rather than given; and a rectangle that isn't numbers is left out.
    #[test]
    fn links_past_the_caps_are_left_out_and_said() {
        let page = PageInfo { width: 100.0, height: 100.0, label: String::new(), crop: [0.0, 0.0, 100.0, 100.0], rotation: 0 };
        let link = |target: LinkTarget| pdfcraft_render::Link { page: 0, index: 0, rect: [10.0, 10.0, 20.0, 20.0], quads: Vec::new(), target };
        let many = (0..MAX_LINKS + 5).map(|_| link(LinkTarget::Page(0, DestView::Top))).collect();
        let linked = Linked::new(vec![page.clone(), page.clone()], many);
        let (links, more) = linked.on(0);
        assert_eq!((links.len(), more, linked.on(1).0.len(), linked.on(2).0.len()), (MAX_LINKS, 5, 0, 0));
        let remote =
            |file: &str, destination: RemoteDestination| LinkTarget::RemoteGoTo { launch: false, file: file.into(), destination, new_window: None };
        let said = |target: LinkTarget| match linked.target(&target) {
            (None, Target::Other(says)) => says,
            other => panic!("{other:?}"),
        };
        assert_eq!(said(LinkTarget::Uri("x".repeat(MAX_ADDRESS + 1))), "a web address too long to follow");
        assert_eq!(said(LinkTarget::Page(2, DestView::Top)), "a link to a page this document doesn't have");
        assert_eq!(said(remote("a.pdf", RemoteDestination::Named("x".repeat(MAX_NAME + 1)))), "a link to a place whose name is too long to follow");
        for file in ["", "https://example.com/a.pdf", "file:///a.pdf", "a\u{202e}fdp.pdf", &"x".repeat(MAX_ADDRESS + 1)] {
            assert!(said(remote(file, RemoteDestination::Default)).starts_with("a link to another file, not followed: "), "{file:?}");
        }
        assert_eq!(said(LinkTarget::Other("SubmitForm".into())), "a link of a kind clv doesn't follow (SubmitForm)");
        assert_eq!(said(LinkTarget::Other("Sub\x1b[2Jmit".repeat(20))).len(), "a link of a kind clv doesn't follow ()".len() + 32);
        assert_eq!(said(LinkTarget::Other(String::new())), "a link that leads nowhere");
        assert_eq!(said(LinkTarget::JavaScript("app.alert(1)".into())), SCRIPT);
        // Sixty-four of a link's quadrilaterals, and none that isn't numbers.
        let quads = pdfcraft_render::Link {
            quads: vec![[10.0, 10.0, 20.0, 10.0, 10.0, 20.0, 20.0, 20.0]; MAX_LINK_RECTS + 9],
            ..link(LinkTarget::Other(String::new()))
        };
        assert_eq!(linked.link(0, &page, &quads).rects.len(), MAX_LINK_RECTS);
        let broken = pdfcraft_render::Link { rect: [f32::NAN, 10.0, 20.0, 20.0], ..link(LinkTarget::Other(String::new())) };
        assert!(linked.link(0, &page, &broken).rects.is_empty());
        let odd = pdfcraft_render::Link { quads: vec![[f32::INFINITY; 8], [10.0, 10.0, 20.0, 10.0, 10.0, 20.0, 20.0, 20.0]], ..broken };
        assert_eq!(linked.link(0, &page, &odd).rects, [[10.0, 80.0, 20.0, 90.0]]);
    }

    /// A named destination is found in the file, by its name alone: the page it is on and how far
    /// down it; a name the document hasn't, or one too long to be one, is nowhere.
    #[test]
    fn links_named_destinations_are_looked_up_in_the_file() {
        let source = Pdf::open("links.pdf", Arc::new(fixtures::linked()), None).unwrap();
        let links = source.links().unwrap();
        let asked = vec![Ask::Name("chapter".into()), Ask::Name("Chapter".into()), Ask::Name("x".repeat(MAX_NAME + 1)), Ask::Page(1)];
        let got = told(&*links, asked);
        let [Told::Name { name, at: Ok(Some((1, y))) }, Told::Name { at: Ok(None), .. }, Told::Name { at: Ok(None), .. }, Told::Page { page: 1, .. }] =
            &got[..]
        else {
            panic!("{} told", got.len())
        };
        assert!(name == "chapter" && (y - 200.0).abs() < 0.01, "{name} at {y}");
    }

    #[test]
    fn a_pdf_is_a_file_with_the_header_in_its_first_kilobyte() {
        assert!(is_pdf(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n"));
        assert!(is_pdf(b"%PDF-2.0"));
        let prefixed = [vec![b' '; 1019], b"%PDF-1.4".to_vec()].concat();
        assert!(is_pdf(&prefixed), "the header may start as late as byte 1019");
        assert!(!is_pdf(&[vec![b' '; 1020], b"%PDF-1.4".to_vec()].concat()), "it must end within the first 1024 bytes");
        assert!(!is_pdf(b"%PDF"), "a truncated header");
        assert!(!is_pdf(b"\x89PNG\r\n\x1a\n"));
        assert!(!is_pdf(b""));
        // Sure only of a header at the start; loosely of one further in.
        assert!(Pdfcraft.opens("a", b"%PDF-1.7\n") && Pdfcraft.opens_loosely("a", b"%PDF-1.7\n"));
        assert!(!Pdfcraft.opens("a.pdf", &prefixed) && Pdfcraft.opens_loosely("a", &prefixed));
    }
}
