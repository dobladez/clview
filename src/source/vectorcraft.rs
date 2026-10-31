//! Vector drawings saved by vectorcraft (`.vectorcraft`, and `.drawcraft` from before its rename),
//! drawn by vectorcraft as its app exports them: each artboard a page, its size in points; guides
//! and template layers left out; the document's white background where it has one. A gzipped one
//! reaches this backend through the tar backend, unpacked.
//!
//! Opening parses the drawing once, to know it opens and its artboards' sizes, and keeps only where
//! its bytes are: a renderer, or a pool, parses it again on its first render (a few milliseconds;
//! 50 for 20,000 shapes) and keeps it, with a vectorcraft renderer for each thread drawing, whose
//! caches make the next render of the page quicker, until it goes.
//!
//! A drawing is untrusted: a page larger than [`MAX_SIDE`] or [`MAX_PIXELS`] is refused, and one
//! that takes longer than [`MAX_RENDER_TIME`] to draw is stopped, at the next object vectorcraft
//! draws. The decoded pictures all renderers share are held to [`IMAGE_CACHE`].

use std::borrow::Cow;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Once, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use vectorcraft_doc::cancel::{self, CancelToken};
use vectorcraft_doc::setup::Background;
use vectorcraft_render::{RenderOptions, Renderer};

use super::threads::Threads;
use super::{Again, Backend, Frame, Input, Opened, Pixels, Pool, Render, Reopenable, Request, Source, guarded};
use crate::args::{OptionSpec, Parsed};

/// Drawings saved by vectorcraft, drawn by vectorcraft.
pub(super) struct Vectorcraft;

impl Backend for Vectorcraft {
    fn name(&self) -> &'static str {
        "vectorcraft"
    }

    fn shows(&self) -> &'static str {
        "vectorcraft drawings"
    }

    fn options(&self) -> &'static [OptionSpec] {
        &[]
    }

    fn opens(&self, _path: &str, head: &[u8]) -> bool {
        is_drawing(head)
    }

    fn open(&self, input: Input, _options: &Parsed<'_>) -> Result<Opened, String> {
        IMAGE_CACHE_SET.call_once(|| vectorcraft_render::set_image_cache_budget(IMAGE_CACHE));
        let path = input.path.clone();
        let error = |e: String| format!("{path}: {e}");
        let (stored, parsed) = match input.reopenable()? {
            Reopenable::Disk(file) => {
                let bytes = read(file).map_err(error)?;
                (Stored::Disk(PathBuf::from(&path)), parse(&bytes))
            }
            Reopenable::Bytes(bytes) => {
                let bytes = within(bytes).map_err(error)?;
                let parsed = parse(&bytes);
                (Stored::Memory(bytes), parsed)
            }
            Reopenable::Again(again) => {
                let bytes = again().and_then(within).map_err(error)?;
                (Stored::Held(again), parse(&bytes))
            }
        };
        // Only the artboards' sizes are kept: the drawing goes, and a render parses it again.
        let parsed = parsed.map_err(error)?;
        let sizes: Vec<(f32, f32)> = parsed.document.artboards.iter().map(|a| (a.rect.width() as f32, a.rect.height() as f32)).collect();
        if let Some(n) = sizes.iter().position(|&(w, h)| !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0)) {
            return Err(error(format!("artboard {} has no size to show", n.saturating_add(1))));
        }
        Ok(Opened::Pages(Box::new(Artboards(Arc::new(Drawing { stored, sizes })))))
    }
}

/// The largest file this reads: 256 MiB.
const MAX_FILE: u64 = 256 << 20;

/// The widest or tallest page drawn, in pixels.
const MAX_SIDE: u64 = 16_384;

/// The most pixels a page is drawn with: 64 million, 256 MiB.
const MAX_PIXELS: u64 = 64_000_000;

/// How long a page may take to draw: past it, vectorcraft is told to stop.
const MAX_RENDER_TIME: Duration = Duration::from_secs(10);

/// What the decoded pictures of every drawing may take together: 64 MiB, rather than vectorcraft's
/// own default of an eighth of the machine's memory (up to 1 GiB), which suits an editor.
const IMAGE_CACHE: usize = 64 << 20;

static IMAGE_CACHE_SET: Once = Once::new();

/// Whether a file starting with `head` is a drawing vectorcraft saved: JSON whose first key is
/// `format`, saying `vectorcraft` (or `drawcraft`, its name before), as vectorcraft writes it.
/// Any other JSON is not one, even one that mentions vectorcraft.
fn is_drawing(head: &[u8]) -> bool {
    fn after<'a>(s: &'a [u8], token: &[u8]) -> Option<&'a [u8]> {
        s.trim_ascii_start().strip_prefix(token)
    }
    let value = after(head, b"{").and_then(|s| after(s, b"\"format\"")).and_then(|s| after(s, b":")).map(<[u8]>::trim_ascii_start);
    value.is_some_and(|v| v.starts_with(b"\"vectorcraft\"") || v.starts_with(b"\"drawcraft\""))
}

/// An open drawing.
struct Artboards(Arc<Drawing>);

struct Drawing {
    stored: Stored,
    /// Each artboard's, in points.
    sizes: Vec<(f32, f32)>,
}

/// Where a drawing's bytes are.
enum Stored {
    /// In a file on disk, read again to render.
    Disk(PathBuf),
    /// As they came through a pipe, which can't be read again.
    Memory(Vec<u8>),
    /// In another file (an archive), read from it again to render.
    Held(Again),
}

/// A drawing parsed.
struct Drawn {
    document: vectorcraft_doc::Document,
    /// What vectorcraft did to bring a damaged or old file within its limits.
    notes: Vec<String>,
}

/// What a renderer or a pool draws with, each its own: the drawing, parsed by its first render
/// (an error, or a panic's, kept too, so a drawing that fails isn't parsed again for every render),
/// and a vectorcraft renderer for each thread that has drawn, kept between renders.
#[derive(Default)]
struct Kept {
    parsed: OnceLock<Result<Arc<Drawn>, String>>,
    renderers: Mutex<Vec<Renderer>>,
}

impl Source for Artboards {
    fn pages(&self) -> usize {
        self.0.sizes.len()
    }

    fn size(&self, page: usize) -> Option<(f32, f32)> {
        self.0.sizes.get(page).copied()
    }

    /// A drawing scales: fitted to the window, it fills it, as a PDF's page does.
    fn natural(&self, _page: usize) -> bool {
        false
    }

    fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
        Ok(Box::new(Single { drawing: self.0.clone(), kept: Kept::default() }))
    }

    fn pool(&self, threads: usize) -> Box<dyn Pool> {
        let (drawing, kept) = (self.0.clone(), Arc::new(Kept::default()));
        Box::new(Threads::new(threads, Arc::new(move |request| drawing.render(request, &kept))))
    }
}

struct Single {
    drawing: Arc<Drawing>,
    kept: Kept,
}

impl Render for Single {
    fn render(&mut self, request: Request) -> Frame {
        guarded(request, || self.drawing.render(request, &self.kept))
    }
}

impl Drawing {
    fn render(&self, request: Request, kept: &Kept) -> Frame {
        self.rendered(request, kept).unwrap_or_else(|e| Frame::failed(request, e))
    }

    fn rendered(&self, request: Request, kept: &Kept) -> Result<Frame, String> {
        let parsed = kept.parsed.get_or_init(|| self.bytes().and_then(|bytes| parse(&bytes)).map(Arc::new)).clone()?;
        let page = request.page;
        let artboard = parsed.document.artboards.get(page).ok_or_else(|| format!("the drawing has no artboard {}", page.saturating_add(1)))?;
        let scale = f64::from(request.scale);
        let (w, h) = vectorcraft_render::raster_size(artboard.rect, scale).map_err(|_| format!("can't draw it at scale {scale}"))?;
        let (wide, high) = (u64::from(w), u64::from(h));
        if wide > MAX_SIDE || high > MAX_SIDE || wide.saturating_mul(high) > MAX_PIXELS {
            return Err(format!("{w} × {h} pixels is too large to render"));
        }
        let white = parsed.document.setup.background == Background::White;
        let options = RenderOptions {
            background: white.then_some([255; 4]),
            // An export's: no guides or template layers, no marks for fonts the system lacks.
            skip_templates: true,
            highlight_substituted_fonts: false,
            image_mips: true,
            ..RenderOptions::default()
        };
        let mut renderer = lock(&kept.renderers).pop().unwrap_or_else(|| {
            let mut renderer = Renderer::new();
            // One thread each: the pool's threads draw the pages side by side.
            renderer.threads = 0;
            renderer
        });
        let token = CancelToken::new();
        let watched = Watched::new(&token, MAX_RENDER_TIME);
        let drawn = cancel::scope(&token, None, || renderer.render_region_with(&parsed.document, artboard.rect, scale, &options));
        drop(watched);
        lock(&kept.renderers).push(renderer);
        if token.is_cancelled() {
            return Err(format!("drawing it took more than {} s", MAX_RENDER_TIME.as_secs()));
        }
        // Premultiplied RGBA, row by row, as a frame's are.
        Ok(Frame { request, width: drawn.width, height: drawn.height, rgba: Pixels::new(drawn.pixels), error: None, warnings: parsed.notes.clone() })
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

/// The drawing in `bytes`, or an error if it isn't one, has no artboards, or vectorcraft panics.
fn parse(bytes: &[u8]) -> Result<Drawn, String> {
    let read = || {
        let file = vectorcraft_format::load_file(bytes).map_err(|e| e.to_string())?;
        if file.doc.artboards.is_empty() {
            return Err("the drawing has no artboards to show".to_string());
        }
        Ok(Drawn { document: file.doc, notes: file.depth_notes })
    };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|_| Err("vectorcraft failed on this drawing".into()))
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
        true => Err(format!("the drawing is more than {} MiB", MAX_FILE >> 20)),
        false => Ok(bytes),
    }
}

/// The lock, even after a thread panicked holding it: what it guards is whole between statements.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The renders being drawn, each with its cancel token and when its time is up, and a thread that
/// cancels each one whose time is up.
struct Watchdog {
    renders: Mutex<Vec<(u64, CancelToken, Instant)>>,
    /// A render added, whose time may be up before the others'.
    wake: Condvar,
}

/// The watchdog, its thread started on first use; `None` if no thread could be: renders then take
/// as long as they take.
fn watchdog() -> Option<&'static Watchdog> {
    static WATCHDOG: OnceLock<Option<&'static Watchdog>> = OnceLock::new();
    *WATCHDOG.get_or_init(|| {
        let watchdog: &'static Watchdog = Box::leak(Box::new(Watchdog { renders: Mutex::new(Vec::new()), wake: Condvar::new() }));
        std::thread::Builder::new().name("watchdog".into()).spawn(move || watch(watchdog)).ok().map(|_| watchdog)
    })
}

/// Cancel each render whose time is up, waking when the next one's is.
fn watch(watchdog: &Watchdog) {
    let mut renders = lock(&watchdog.renders);
    loop {
        let now = Instant::now();
        renders.retain(|(_, token, until)| {
            let up = *until <= now;
            if up {
                token.cancel();
            }
            !up
        });
        renders = match renders.iter().map(|(_, _, until)| *until).min() {
            Some(next) => watchdog.wake.wait_timeout(renders, next.saturating_duration_since(now)).map_or_else(|e| e.into_inner().0, |r| r.0),
            None => watchdog.wake.wait(renders).unwrap_or_else(PoisonError::into_inner),
        };
    }
}

/// A render the watchdog cancels once its time has passed, unless this has gone.
struct Watched(Option<u64>);

impl Watched {
    /// `token` cancelled once `limit` has passed.
    fn new(token: &CancelToken, limit: Duration) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let Some(watchdog) = watchdog() else { return Self(None) };
        let Some(until) = Instant::now().checked_add(limit) else { return Self(None) };
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        lock(&watchdog.renders).push((id, token.clone(), until));
        watchdog.wake.notify_one();
        Self(Some(id))
    }
}

impl Drop for Watched {
    fn drop(&mut self) {
        if let (Some(id), Some(watchdog)) = (self.0, watchdog()) {
            lock(&watchdog.renders).retain(|(watched, _, _)| *watched != id);
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use vectorcraft_geom::Rect;

    use super::*;

    #[test]
    fn a_drawing_is_json_that_says_so_first() {
        for head in [
            &br#"{"format":"vectorcraft","version":4}"#[..],
            br#"{"format":"drawcraft","version":2}"#,
            b"{\n  \"format\": \"vectorcraft\",\n  \"version\": 4",
            b" \r\n\t{ \"format\" :\"vectorcraft\"",
        ] {
            assert!(is_drawing(head), "{}", String::from_utf8_lossy(head));
        }
        for head in [
            &br#"{"name":"vectorcraft","format":"vectorcraft"}"#[..],
            br#"{"format":"vectorcraft2"}"#,
            br#"{"format":"svg"}"#,
            br#"["format","vectorcraft"]"#,
            br#"{"format""#,
            b"{\\rtf1 vectorcraft",
            b"",
        ] {
            assert!(!is_drawing(head), "{}", String::from_utf8_lossy(head));
        }
    }

    #[test]
    fn the_watchdog_stops_a_render_once_its_time_is_up_and_only_then() {
        let (late, early) = (CancelToken::new(), CancelToken::new());
        let watched = Watched::new(&late, Duration::from_millis(50));
        let gone = Watched::new(&early, Duration::from_millis(50));
        drop(gone);
        assert!(!late.is_cancelled(), "not before its time");
        std::thread::sleep(Duration::from_millis(300));
        assert!(late.is_cancelled(), "its time is up");
        assert!(!early.is_cancelled(), "a render done before its time is left alone");
        drop(watched);
    }

    /// A drawing with one artboard of `w` × `h` points, saved.
    fn saved(w: f64, h: f64) -> Vec<u8> {
        let mut document = vectorcraft_doc::Document::new(100.0, 100.0);
        document.artboards[0].rect = Rect::new(0.0, 0.0, w, h);
        vectorcraft_format::save(&document, false)
    }

    #[test]
    fn a_page_too_large_to_draw_is_refused() {
        let render = |w: f64, h: f64, scale: f32| {
            let drawing = Drawing { stored: Stored::Memory(saved(w, h)), sizes: vec![(w as f32, h as f32)] };
            let frame = drawing.render(Request { page: 0, scale, tag: 0 }, &Kept::default());
            frame.error.map_or(Ok((frame.width, frame.height)), Err)
        };
        assert_eq!(render(200.0, 100.0, 2.0), Ok((400, 200)));
        assert_eq!(render(20_000.0, 10.0, 1.0), Err("20000 × 10 pixels is too large to render".into()), "wider than 16384");
        assert_eq!(render(10_000.0, 10_000.0, 1.0), Err("10000 × 10000 pixels is too large to render".into()), "more than 64 million");
        assert!(render(200.0, 100.0, f32::NAN).is_err_and(|e| e.starts_with("can't draw it at scale")));
    }

    proptest! {
        /// Whatever a file starts with, choosing never panics.
        #[test]
        fn any_first_bytes_are_safe(head in proptest::collection::vec(any::<u8>(), 0..1024), json in any::<bool>()) {
            let head = if json { [&b"{\"format\":"[..], &head].concat() } else { head };
            let _ = is_drawing(&head);
        }
    }
}
