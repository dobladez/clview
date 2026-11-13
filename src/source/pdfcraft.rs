//! PDFs rendered by pdfcraft, the way its app draws them: comments without an appearance of their
//! own are drawn, a dynamic XFA form shows the pages its template lays out into, and either
//! password opens a protected file.
//!
//! Their text, for a search, is read by pdfcraft too, a page a chunk, in a worker process of its
//! own ([`TEXT`]): what a page's text takes to read can't be bounded before it is read (a page of
//! three million glyphs took 870 MiB), so the worker's heap and CPU time are, as they are spent,
//! and a page past them costs the search that page
//! ([ADR-0027](../../docs/adr/0027-a-pdfs-text-read-in-a-worker-process-for-its-heap-cap.md)).

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use pdfcraft_render::text::TextFilter;
use pdfcraft_render::{PageRenderer, PageText, RenderConfig, RenderPool, RenderRequest, RenderWarning, RenderedPage, RequestKind};

use super::worker::{self, Died, FAILED, Kill, Link, Session, WALL, Worker};
use super::{Backend, Deferred, Format, Frame, Got, Input, Opened, Pixels, Request, Source, Texts, Want};
use crate::args::{OptionSpec, Parsed};

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
        vec![Format { name: "PDF", extensions: &[] }]
    }

    fn credits(&self) -> &[&'static str] {
        &["pdfcraft"]
    }

    fn options(&self) -> &'static [OptionSpec] {
        OPTIONS
    }

    fn opens(&self, _path: &str, head: &[u8]) -> bool {
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
}

/// Whether a file starting with `head` is a PDF: its header (`%PDF-` and the version) within the
/// first 1024 bytes. ISO 32000-2 §7.5.2 puts the header first; readers have long allowed bytes
/// before it, up to that bound.
fn is_pdf(head: &[u8]) -> bool {
    head.get(..1024).unwrap_or(head).windows(5).any(|w| w == b"%PDF-")
}

/// The most pages a PDF may have to be opened. A page tree may name one page over and over, six
/// bytes a name, so a file of a few megabytes can have millions; real documents stay far below.
const MAX_PAGES: usize = 100_000;

/// An open PDF: the display copy the renderers read, and each page's size as shown.
pub(super) struct Pdf {
    bytes: Arc<Vec<u8>>,
    config: RenderConfig,
    /// In points, after `/Rotate` and `/UserUnit`.
    sizes: Vec<(f32, f32)>,
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
        })
    }
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

/// The most bytes of answers that wait to be taken: at it, no more is read until the viewer has
/// taken some. What a viewer busy elsewhere (writing to a slow terminal) leaves waiting is that
/// and one answer more, the one being read, whatever the document.
const MAX_WAITING: usize = 4 << 20;

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

/// A PDF's text for a search ([`Texts`]): a thread that asks a worker for what is wanted, a page's
/// text or a range's rectangles at a time, starts another worker past one killed, and gives what
/// comes back. Dropped, it kills the worker without waiting for it.
struct PagesText {
    shared: Arc<Shared>,
    pages: usize,
}

struct Shared {
    state: Mutex<State>,
    /// Work wanted, room for its answer, or time to stop.
    wake: Condvar,
}

/// Nothing here is looked up by going through a list: a document has as many requests as pages, and
/// the lock is the one the viewer takes between keys ([`Texts::try_recv`], [`Texts::set_queue`]).
#[derive(Default)]
struct State {
    /// The latest queue, the most urgent first; what it lists, to look one up; and how far into it
    /// every request has been given, where the next one to read is looked for from.
    queue: Vec<Want>,
    listed: HashSet<Want>,
    from: usize,
    /// Given while still in the queue: not read again when listed again.
    given: HashSet<Want>,
    /// The answers given and not yet taken, each to a request still in the queue, and the bytes
    /// they hold ([`weight`]): the thread reads no more while that is [`MAX_WAITING`] or over.
    answers: VecDeque<Got>,
    waiting: usize,
    /// The worker at work, to kill when the search ends.
    kill: Option<Kill>,
    stop: bool,
}

impl Shared {
    /// The most urgent thing wanted and not yet given, waited for, and for room for its answer.
    /// `None` when the search has ended.
    fn next(&self) -> Option<Want> {
        let mut state = lock(&self.state);
        loop {
            if state.stop {
                return None;
            }
            // Past what was given, each looked at once for a queue: a request given is never ungiven
            // while its queue stands.
            while state.waiting < MAX_WAITING
                && let Some(want) = state.queue.get(state.from).copied()
            {
                if !state.given.contains(&want) {
                    return Some(want);
                }
                state.from = state.from.saturating_add(1);
            }
            state = self.wake.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The answer to `want`, for the viewer to take, if it is still wanted: under the lock the queue
    /// is replaced under, so none is ever given for a request no longer listed. Whether the search
    /// goes on.
    fn give(&self, want: Want, got: Got) -> bool {
        let mut state = lock(&self.state);
        if state.listed.contains(&want) && !state.stop {
            state.given.insert(want);
            state.waiting = state.waiting.saturating_add(weight(&got));
            state.answers.push_back(got);
        }
        !state.stop
    }
}

/// What `got` answers.
fn asked(got: &Got) -> Want {
    match got {
        Got::Text(page, _) => Want::Text(*page),
        Got::Rects(want, _) => *want,
    }
}

/// The bytes an answer holds while it waits, near enough: its text or its rectangles, or why not,
/// and itself.
fn weight(got: &Got) -> usize {
    let held = match got {
        Got::Text(_, Ok(text)) | Got::Text(_, Err(text)) | Got::Rects(_, Err(text)) => text.len(),
        Got::Rects(_, Ok(rects)) => rects.len().saturating_mul(std::mem::size_of::<(usize, [f32; 4])>()),
    };
    held.saturating_add(std::mem::size_of::<Got>())
}

impl PagesText {
    fn new(bytes: Arc<Vec<u8>>, password: Option<Arc<str>>, pages: usize) -> Self {
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), wake: Condvar::new() });
        let reader = Reader { shared: shared.clone(), bytes, password, link: None, kills: 0, spent: Duration::ZERO, ended: None };
        // A thread that can't start leaves the text unread, which the search shows as still reading.
        let _ = std::thread::Builder::new().name("pdf text".into()).spawn(move || reader.run());
        Self { shared, pages }
    }
}

impl Texts for PagesText {
    fn chunks(&self) -> usize {
        self.pages
    }

    fn chunk_of(&self, page: usize) -> usize {
        page
    }

    /// An answer waiting for a request no longer listed is dropped with it: listed again, it is
    /// read again.
    fn set_queue(&self, wanted: Vec<Want>) {
        // Made before the lock is taken: the thread reads on meanwhile.
        let listed: HashSet<Want> = wanted.iter().copied().collect();
        let mut state = lock(&self.shared.state);
        state.given.retain(|given| listed.contains(given));
        state.answers.retain(|got| listed.contains(&asked(got)));
        state.waiting = state.answers.iter().map(weight).fold(0, usize::saturating_add);
        state.queue = wanted;
        state.listed = listed;
        state.from = 0;
        self.shared.wake.notify_all();
    }

    fn try_recv(&self) -> Option<Got> {
        let mut state = lock(&self.shared.state);
        let got = state.answers.pop_front()?;
        state.waiting = state.waiting.saturating_sub(weight(&got));
        // Room again, maybe: the thread reads on.
        self.shared.wake.notify_all();
        Some(got)
    }
}

impl Drop for PagesText {
    /// The worker is killed, which ends the thread's wait on it; nothing waits for the thread.
    fn drop(&mut self) {
        let kill = {
            let mut state = lock(&self.shared.state);
            state.stop = true;
            state.kill.take()
        };
        self.shared.wake.notify_all();
        if let Some(kill) = kill {
            kill.kill();
        }
    }
}

/// The lock, even after a thread panicked holding it: the state stays consistent between statements.
fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The thread's side of a [`PagesText`].
struct Reader {
    shared: Arc<Shared>,
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

impl Reader {
    fn run(mut self) {
        if self.bytes.len() > MAX_PDF {
            let (size, most) = (self.bytes.len() >> 20, MAX_PDF >> 20);
            self.ended = Some(format!("this PDF is too large to search: {size} MiB, and the most is {most}"));
        } else if self.password.as_ref().is_some_and(|password| password.len() > MAX_PASSWORD) {
            self.ended = Some("its password is too long for its text to be read".into());
        }
        while let Some(want) = self.shared.next() {
            let got = match want {
                Want::Text(page) => Got::Text(page, self.text(page)),
                Want::Rects { chunk, from, to } => Got::Rects(want, self.rects(chunk, from, to)),
            };
            if !self.shared.give(want, got) {
                return;
            }
        }
    }

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
        {
            let mut state = lock(&self.shared.state);
            if state.stop {
                return Err(Asked::Gone(Died::Ended("let go of".into())));
            }
            state.kill = Some(link.killer());
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
        lock(&self.shared.state).kill = None;
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
        assert_eq!((texts.chunks(), texts.chunk_of(0), texts.chunk_of(1)), (2, 0, 1));
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
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), wake: Condvar::new() });
        let bytes = Arc::new(pdf(&[LINES, LINES]));
        let mut reader = Reader { shared, bytes, password: None, link: None, kills: 0, spent: Duration::ZERO, ended: None };
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

    /// An answer waiting when its request is taken off the queue is never given, nor is the request
    /// counted as given: listed again, it is read again, and given once.
    #[test]
    fn an_answer_whose_request_was_dropped_never_comes() {
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
        let source = Pdf::open("lines.pdf", Arc::new(pdf(&[LINES, "BT /F1 12 Tf 20 250 Td (other) Tj ET"])), None).unwrap();
        let texts = PagesText::new(source.bytes.clone(), None, 2);
        let rects = Want::Rects { chunk: 0, from: 7, to: 12 };
        texts.set_queue(vec![Want::Text(0), Want::Text(1), rects]);
        let begun = Instant::now();
        while lock(&texts.shared.state).answers.len() < 3 {
            assert!(begun.elapsed() < Duration::from_secs(10), "its answers never came");
            std::thread::sleep(Duration::from_millis(1));
        }
        // All three wait; only the second page's is still wanted.
        texts.set_queue(vec![Want::Text(1)]);
        assert!(matches!(texts.try_recv(), Some(Got::Text(1, Ok(other))) if other == "other"));
        assert!(texts.try_recv().is_none(), "an answer to a request no longer listed");
        texts.set_queue(Vec::new());
        {
            let state = lock(&texts.shared.state);
            assert!(state.answers.is_empty() && state.given.is_empty() && state.waiting == 0);
        }
        // Listed again: read again, given once.
        let got = gave(&texts, vec![rects, Want::Text(0)]);
        assert!(
            matches!(&got[..], [Got::Rects(asked, Ok(placed)), Got::Text(0, Ok(_))] if *asked == rects && placed.len() == 1),
            "{} given",
            got.len()
        );
        // And one being read when its request is dropped isn't given when it is done.
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), wake: Condvar::new() });
        let texts = PagesText { shared: shared.clone(), pages: 1 };
        texts.set_queue(vec![Want::Text(0)]);
        assert_eq!(shared.next(), Some(Want::Text(0)));
        texts.set_queue(Vec::new());
        assert!(shared.give(Want::Text(0), Got::Text(0, Ok("read too late".into()))), "the search goes on");
        assert!(texts.try_recv().is_none() && lock(&shared.state).given.is_empty());
    }

    /// The answers that wait to be taken are bounded: with [`MAX_WAITING`] bytes of them waiting,
    /// nothing more is read until the viewer takes some, so a viewer that takes none for a while
    /// holds that and no more; taking them lets the reading go on, to the end. Here a megabyte a
    /// page, given as the thread gives them.
    #[test]
    fn answers_wait_for_the_viewer_within_a_budget() {
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), wake: Condvar::new() });
        let texts = PagesText { shared: shared.clone(), pages: 12 };
        texts.set_queue((0..12).map(Want::Text).collect());
        let reader = std::thread::spawn(move || {
            let mut read = 0;
            while let Some(want) = shared.next() {
                let Want::Text(page) = want else { break };
                if !shared.give(want, Got::Text(page, Ok("x".repeat(1 << 20)))) {
                    break;
                }
                read += 1;
            }
            read
        });
        // Left alone, it stops at the budget: four pages of a megabyte, never a fifth.
        std::thread::sleep(Duration::from_millis(200));
        {
            let state = lock(&texts.shared.state);
            assert_eq!(state.answers.len(), 4, "{} bytes waiting", state.waiting);
            assert!((MAX_WAITING..MAX_WAITING + (1 << 20)).contains(&state.waiting), "{}", state.waiting);
        }
        // Each one taken makes room for one more, to the last.
        let (mut pages, begun) = (Vec::new(), Instant::now());
        while pages.len() < 12 && begun.elapsed() < Duration::from_secs(10) {
            match texts.try_recv() {
                Some(Got::Text(page, Ok(text))) if text.len() == 1 << 20 => pages.push(page),
                Some(_) => panic!("not what was given"),
                None => std::thread::sleep(Duration::from_millis(1)),
            }
            assert!(lock(&texts.shared.state).waiting < MAX_WAITING + (1 << 20));
        }
        assert_eq!(pages, (0..12).collect::<Vec<_>>());
        assert_eq!(lock(&texts.shared.state).waiting, 0);
        // The search ended: the thread stops, though more may be wanted.
        texts.set_queue(vec![Want::Text(99)]);
        drop(texts);
        assert!((12..=13).contains(&reader.join().unwrap()));
    }

    /// What the viewer's side does stays quick whatever the pages: sixty thousand of them, empty,
    /// are asked for, given and taken, the queue set again half way, in well under the time one
    /// search of them by hand would take. Looking each request up among those given, one after
    /// another, under the lock the viewer takes, made this the square of the pages and worse.
    #[test]
    fn the_queue_stays_quick_with_tens_of_thousands_of_pages() {
        const PAGES: usize = 60_000;
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), wake: Condvar::new() });
        let texts = PagesText { shared: shared.clone(), pages: PAGES };
        let all: Vec<Want> = (0..PAGES).map(Want::Text).collect();
        let begun = Instant::now();
        texts.set_queue(all.clone());
        let reader = std::thread::spawn(move || {
            while let Some(want) = shared.next() {
                let Want::Text(page) = want else { break };
                if !shared.give(want, Got::Text(page, Ok(String::new()))) {
                    break;
                }
            }
        });
        let (mut taken, mut slowest, mut relisted) = (0usize, Duration::ZERO, Duration::ZERO);
        while taken < PAGES {
            assert!(begun.elapsed() < Duration::from_secs(20), "{taken} of {PAGES} pages taken in 20 s");
            // As the pager does between keys: all there is, then away for a moment.
            let asked = Instant::now();
            let got = texts.try_recv();
            slowest = slowest.max(asked.elapsed());
            match got {
                Some(_) => taken += 1,
                None => std::thread::sleep(Duration::from_millis(1)),
            }
            // Half way, the reader moves: the same pages wanted, in another order.
            if taken == PAGES / 2 && relisted.is_zero() {
                let asked = Instant::now();
                texts.set_queue(all.iter().rev().copied().collect());
                relisted = asked.elapsed().max(Duration::from_nanos(1));
            }
        }
        let took = begun.elapsed();
        drop(texts);
        reader.join().unwrap();
        eprintln!("{PAGES} pages in {took:?}; the slowest try_recv {slowest:?}; the queue set again in {relisted:?}");
        assert!(took < Duration::from_secs(5), "{took:?}");
        assert!(relisted < Duration::from_millis(500), "the queue set again in {relisted:?}");
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
    }
}
