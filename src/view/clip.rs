//! Copying a page to the clipboard: the pager's `c`. ADR-0009 has why it goes the way it does.
//!
//! To the terminal's own clipboard first, through kitty's clipboard protocol (OSC 5522,
//! <https://sw.kovidgoyal.net/kitty/clipboard/>), which carries a picture as `image/png` and
//! reaches the terminal over SSH, and through tmux's passthrough, as the pages do. Whether the
//! terminal speaks it is asked once, when the reader first copies: DECRQM for mode 5522. A
//! terminal that doesn't (Ghostty, WezTerm) has only OSC 52, which carries text, so a picture
//! goes to the system's clipboard instead when the terminal runs on this machine; over SSH there
//! is no way to the reader's.
//!
//! The pager never waits for a copy: the page renders on the level's pool beside the pages, its
//! PNG is made on a thread of its own, and it goes to the terminal a round of packets at a time,
//! the keys read in between ([`Copier::on`]).

use std::borrow::Cow;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use base64::Engine as _;

use super::encode::{self, PngEffort};
use super::fit::{MAX_AREA, MAX_PIXELS, Size};
use super::keys::{CLIP_ID, ClipError, Key};
use super::kitty::{self, Wrap};
use crate::source::{Frame, Request};

/// Pixels per unit of a page's size it is copied at, when it has no pixels of its own: 144 per
/// inch for a PDF's points, twice a drawing's CSS pixels, as a 2× screen shows them. A picture is
/// copied at its own pixels. Either within [`MAX_PIXELS`] a side and [`MAX_AREA`] in all, as a page
/// rendered for the screen is.
pub(crate) const SCALE: f32 = 2.0;
/// The bytes of the picture each packet carries, before base64: the protocol's most.
const CHUNK: usize = 4096;
/// The largest PNG sent: what the protocol has every terminal take, at least.
pub(crate) const MAX_PNG: usize = 64 << 20;
/// About the bytes of escape codes written in a round of the pager, so keys are read between
/// rounds, over a slow link too: 32 packets.
const ROUND: usize = 128 << 10;
/// How long the terminal has to say whether it speaks the protocol. Terminals answer DECRQM in
/// milliseconds, or say they don't know the mode; one that says nothing doesn't speak it.
const ASK_WAIT: Duration = Duration::from_secs(1);
/// How long a write waits for the terminal's answer before the reader is told there is none
/// (inside tmux, the answer goes to the pane tmux shows, which may be another by then).
const SENT_WAIT: Duration = Duration::from_secs(5);
/// How long the pager waits at its end for an answer still due, so it doesn't land on the shell's
/// command line.
pub(crate) const ANSWER_WAIT: Duration = Duration::from_secs(1);
/// How long the pager waits at its end for the system's clipboard to be let go of: on X11, handed
/// to a clipboard manager, if there is one.
const RELEASE_WAIT: Duration = Duration::from_millis(500);
/// How often a copy is looked at while it waits on a thread or an answer.
const POLL: Duration = Duration::from_millis(10);
/// The type a picture is copied as.
const PNG_TYPE: &str = "image/png";

/// The scale page `size` is copied at: its own pixels for a picture, [`SCALE`] otherwise, within
/// [`MAX_PIXELS`] a side and [`MAX_AREA`] in all.
pub(crate) fn scale(size: Size) -> Result<f32, String> {
    let Size { w, h, natural } = size;
    let positive = |v: f32| v.is_finite() && v > 0.0;
    if !(positive(w) && positive(h)) {
        return Err("it has no size".into());
    }
    let wanted = if natural { 1.0 } else { SCALE };
    // In f64, where the area of any f32 page is finite.
    let by_area = (MAX_AREA / (f64::from(w) * f64::from(h))).sqrt() as f32;
    let scale = wanted.min(MAX_PIXELS / w).min(MAX_PIXELS / h).min(by_area);
    if positive(scale) { Ok(scale) } else { Err("it is too large or too small to copy".into()) }
}

/// DECRQM for mode 5522: whether the terminal speaks the clipboard protocol. It answers
/// `CSI ? 5522 ; Ps $ y` ([`Key::Clipboard`]).
pub(crate) fn ask(out: &mut Vec<u8>, wrap: Wrap) {
    kitty::pass(out, b"\x1b[?5522$p", wrap);
}

/// One packet of the clipboard protocol: `OSC 5522 ; metadata ; base64 data ST`, for tmux each
/// in a passthrough sequence of its own, which keeps them small.
fn packet(out: &mut Vec<u8>, metadata: &str, data: &[u8], wrap: Wrap) {
    let mut code = Vec::with_capacity(data.len().saturating_mul(4) / 3 + metadata.len() + 16);
    code.extend_from_slice(b"\x1b]5522;");
    code.extend_from_slice(metadata.as_bytes());
    if !data.is_empty() {
        code.push(b';');
        code.extend_from_slice(base64::engine::general_purpose::STANDARD.encode(data).as_bytes());
    }
    code.extend_from_slice(b"\x1b\\");
    kitty::pass(out, &code, wrap);
}

/// A PNG on its way to the terminal's clipboard as `image/png`: a packet to start, a packet for
/// each [`CHUNK`] of it, and a packet to end, a round of them at a time. The terminal answers the
/// end, or an error at any time, under the write's `id` ([`Key::Copied`]).
pub(crate) struct Write {
    png: Vec<u8>,
    id: u32,
    wrap: Wrap,
    /// The chunks written so far.
    sent: usize,
    begun: bool,
    ended: bool,
}

impl Write {
    pub(crate) fn new(png: Vec<u8>, id: u32, wrap: Wrap) -> Self {
        Self { png, id, wrap, sent: 0, begun: false, ended: false }
    }

    /// The next packets into `out`, as many as `bytes` of escape codes take and at least one: the
    /// start first, the end last. Whether any are left after them.
    pub(crate) fn next(&mut self, out: &mut Vec<u8>, bytes: usize) -> bool {
        if self.ended {
            return false;
        }
        let (from, id) = (out.len(), format!("id={CLIP_ID}{}", self.id));
        if !self.begun {
            packet(out, &format!("type=write:{id}"), b"", self.wrap);
            self.begun = true;
        }
        let mime = base64::engine::general_purpose::STANDARD.encode(PNG_TYPE);
        let mut chunks = self.png.chunks(CHUNK).skip(self.sent);
        while out.len().saturating_sub(from) < bytes {
            let Some(chunk) = chunks.next() else {
                packet(out, &format!("type=wdata:{id}"), b"", self.wrap);
                self.ended = true;
                return false;
            };
            packet(out, &format!("type=wdata:mime={mime}:{id}"), chunk, self.wrap);
            self.sent = self.sent.saturating_add(1);
        }
        true
    }
}

/// Which clipboard a copy goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Way {
    /// The terminal's, through the clipboard protocol.
    Terminal,
    /// The system's, on this machine.
    System,
    /// None: the terminal has no clipboard for pictures, and it is on another machine.
    Nowhere,
}

/// The way to a clipboard, once the terminal said whether it `speaks` the protocol: its own when
/// it does, wherever it is; the system's when it runs on this machine (`local`); none otherwise.
fn way(speaks: bool, local: bool) -> Way {
    match (speaks, local) {
        (true, _) => Way::Terminal,
        (false, true) => Way::System,
        (false, false) => Way::Nowhere,
    }
}

/// What the terminal said of the clipboard protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Support {
    Unknown,
    /// Asked then, no answer yet.
    Asked(Instant),
    Known(bool),
}

/// A copy under way.
struct Copying {
    /// What the status line calls it: `page 3`, `beach.jpg`.
    what: String,
    /// The level whose pool renders it.
    level: u64,
    /// A picture's own size, when it is copied smaller than that.
    smaller: Option<(u32, u32)>,
    /// Its size in pixels, once rendered.
    size: (u32, u32),
    stage: Stage,
}

enum Stage {
    /// Asked of the level's pool, which keeps it in its queue.
    Rendering(Request),
    /// Rendered, the terminal's answer about the protocol still to come.
    Rendered(Frame),
    /// Being made a PNG on a thread of its own.
    Encoding(Receiver<Result<Vec<u8>, String>>),
    /// Going to the terminal a round at a time.
    Sending(Write),
    /// All sent, since then: the terminal's answer to come under `id`. `said` once the reader
    /// has been told it hasn't come; a late one is still said.
    Sent { id: u32, since: Instant, said: bool },
    /// With the system's clipboard, on its thread.
    System(Receiver<Result<(), String>>),
}

/// What [`Copier::advance`] made of a copy.
enum Next {
    /// Still under way.
    Wait(Copying),
    /// Still under way, with news for the status line.
    Say(Copying, String),
    /// Over: what the status line says.
    Done(String),
}

/// Copies pages to the clipboard for the pager, one at a time, without ever making it wait.
pub(crate) struct Copier {
    /// The terminal runs on this machine, so the system's clipboard is the reader's.
    local: bool,
    effort: PngEffort,
    wrap: Wrap,
    support: Support,
    copy: Option<Copying>,
    /// Clipboard writes so far: each one's id.
    writes: u32,
    /// The system's clipboard, once a copy went there.
    system: Option<System>,
}

impl Copier {
    /// For a terminal on this machine or not (`local`), whose pages go as PNGs compressed with
    /// `effort` and written through `wrap`.
    pub(crate) fn new(local: bool, effort: PngEffort, wrap: Wrap) -> Self {
        Self { local, effort, wrap, support: Support::Unknown, copy: None, writes: 0, system: None }
    }

    /// Start copying `what`, `page` of level `level`'s source, which is `size` large (`None` past
    /// its end), its render tagged `tag`, unless a copy is under way. The first time, the
    /// question about the clipboard protocol goes into `out`. What the status line says.
    #[expect(clippy::too_many_arguments, reason = "what is copied, from where, and how it is told apart")]
    pub(crate) fn start(&mut self, what: String, level: u64, page: usize, size: Option<Size>, tag: u64, now: Instant, out: &mut Vec<u8>) -> String {
        if let Some(copy) = &self.copy
            && !matches!(copy.stage, Stage::Sent { said: true, .. })
        {
            return format!("still copying {}", copy.what);
        }
        let Some(size) = size else { return format!("can't copy {what}: it isn't there") };
        let scale = match scale(size) {
            Ok(scale) => scale,
            Err(why) => return format!("can't copy {what}: {why}"),
        };
        if self.support == Support::Unknown {
            ask(out, self.wrap);
            self.support = Support::Asked(now);
        }
        // Finite and positive (`scale` checked), so the casts saturate at worst.
        let smaller = (size.natural && scale < 1.0).then(|| (size.w.round() as u32, size.h.round() as u32));
        let said = format!("copying {what}…");
        let stage = Stage::Rendering(Request { page, scale, tag });
        self.copy = Some(Copying { what, level, smaller, size: (0, 0), stage });
        said
    }

    /// The render it waits for on level `level`'s pool: it goes in that pool's queue, first.
    pub(crate) fn request(&self, level: u64) -> Option<Request> {
        match self.copy.as_ref() {
            Some(Copying { level: of, stage: Stage::Rendering(request), .. }) if *of == level => Some(*request),
            _ => None,
        }
    }

    /// Whether a render tagged `tag` is the one it waits for.
    pub(crate) fn wants(&self, tag: u64) -> bool {
        matches!(&self.copy, Some(Copying { stage: Stage::Rendering(request), .. }) if request.tag == tag)
    }

    /// The render it waited for: [`Copier::on`] carries it on.
    pub(crate) fn rendered(&mut self, frame: Frame) {
        if let Some(copy) = self.copy.as_mut() {
            copy.size = (frame.width, frame.height);
            copy.stage = Stage::Rendered(frame);
        }
    }

    /// The reader left level `level`: a copy still rendering on its pool stops, and says so.
    pub(crate) fn left(&mut self, level: u64) -> Option<String> {
        let copy = self.copy.take_if(|copy| copy.level == level && matches!(copy.stage, Stage::Rendering(_)))?;
        Some(format!("stopped copying {}", copy.what))
    }

    /// tmux threw output away: a write under way, or one just sent, may have lost packets.
    pub(crate) fn lost(&mut self) -> Option<String> {
        let copy = self.copy.take_if(|copy| matches!(copy.stage, Stage::Sending(_) | Stage::Sent { said: false, .. }))?;
        Some(format!("tmux dropped part of the copy of {}; c copies it again", copy.what))
    }

    /// The terminal's answer about the clipboard: whether it speaks the protocol, or how a write
    /// went. What the status line says, when that ends a copy.
    pub(crate) fn answer(&mut self, key: Key) -> Option<String> {
        match key {
            Key::Clipboard(speaks) => {
                self.support = Support::Known(speaks);
                None
            }
            Key::Copied(id, status) => {
                let ours = match self.copy.as_ref().map(|c| &c.stage) {
                    Some(Stage::Sending(write)) => write.id == id,
                    Some(Stage::Sent { id: sent, .. }) => *sent == id,
                    _ => false,
                };
                let copy = self.copy.take_if(|_| ours)?;
                Some(match status {
                    Ok(()) => copied(&copy),
                    Err(error) => format!("the terminal didn't copy {}: {}", copy.what, why(error)),
                })
            }
            _ => None,
        }
    }

    /// Carry the copy on: wait for the answer about the protocol, have the PNG made, put the next
    /// round of packets into `out`, wait for the answer to them. What the status line says, when
    /// there is news.
    pub(crate) fn on(&mut self, now: Instant, out: &mut Vec<u8>) -> Option<String> {
        let copy = self.copy.take()?;
        match self.advance(copy, now, out) {
            Next::Wait(copy) => {
                self.copy = Some(copy);
                None
            }
            Next::Say(copy, said) => {
                self.copy = Some(copy);
                Some(said)
            }
            Next::Done(said) => Some(said),
        }
    }

    fn advance(&mut self, mut copy: Copying, now: Instant, out: &mut Vec<u8>) -> Next {
        let stage = std::mem::replace(&mut copy.stage, Stage::Sent { id: 0, since: now, said: true });
        copy.stage = match stage {
            Stage::Rendered(frame) => {
                if let Some(e) = &frame.error {
                    return Next::Done(format!("can't copy {}: {e}", copy.what));
                }
                let speaks = match self.support {
                    Support::Known(speaks) => speaks,
                    Support::Asked(at) if now.saturating_duration_since(at) < ASK_WAIT => {
                        copy.stage = Stage::Rendered(frame);
                        return Next::Wait(copy);
                    }
                    // Asked and no answer: a terminal that doesn't know DECRQM.
                    Support::Asked(_) | Support::Unknown => {
                        self.support = Support::Known(false);
                        false
                    }
                };
                match way(speaks, self.local) {
                    Way::Terminal => match png(frame, self.effort) {
                        Ok(encoding) => Stage::Encoding(encoding),
                        Err(e) => return Next::Done(format!("can't copy {}: {e}", copy.what)),
                    },
                    Way::System => match self.system(frame) {
                        Ok(done) => Stage::System(done),
                        Err(e) => return Next::Done(format!("can't copy {}: {e}", copy.what)),
                    },
                    Way::Nowhere => {
                        return Next::Done(format!(
                            "can't copy {}: this terminal has no clipboard for pictures, and the system's isn't yours over SSH",
                            copy.what
                        ));
                    }
                }
            }
            Stage::Encoding(png) => match png.try_recv() {
                Err(TryRecvError::Empty) => Stage::Encoding(png),
                Err(TryRecvError::Disconnected) => return Next::Done(format!("can't copy {}: making its PNG failed", copy.what)),
                Ok(Err(e)) => return Next::Done(format!("can't copy {}: {e}", copy.what)),
                Ok(Ok(png)) if png.len() > MAX_PNG => {
                    return Next::Done(format!("can't copy {}: its PNG is {} MB, more than a terminal need take", copy.what, png.len() >> 20));
                }
                Ok(Ok(png)) => {
                    self.writes = self.writes.wrapping_add(1);
                    Stage::Sending(Write::new(png, self.writes, self.wrap))
                }
            },
            Stage::Sending(mut write) => match write.next(out, ROUND) {
                true => Stage::Sending(write),
                false => Stage::Sent { id: write.id, since: now, said: false },
            },
            Stage::Sent { id, since, said: false } if now.saturating_duration_since(since) >= SENT_WAIT => {
                copy.stage = Stage::Sent { id, since, said: true };
                let said = format!("sent {} to the terminal's clipboard; it hasn't said whether it took it", copy.what);
                return Next::Say(copy, said);
            }
            Stage::System(done) => match done.try_recv() {
                Err(TryRecvError::Empty) => Stage::System(done),
                Err(TryRecvError::Disconnected) => return Next::Done(format!("can't copy {}: the clipboard failed", copy.what)),
                Ok(Err(e)) => return Next::Done(format!("can't copy {}: {e}", copy.what)),
                Ok(Ok(())) => return Next::Done(copied(&copy)),
            },
            stage @ (Stage::Rendering(_) | Stage::Sent { .. }) => stage,
        };
        Next::Wait(copy)
    }

    /// `frame` to the system's clipboard, on its thread, started the first time: its answer.
    fn system(&mut self, frame: Frame) -> Result<Receiver<Result<(), String>>, String> {
        let system = match self.system.take() {
            Some(system) => system,
            None => System::start()?,
        };
        let (answer, done) = sync_channel(1);
        // Never waits: copies go one at a time, so the thread has nothing waiting for it. One that
        // ended (it panicked) is let go, and started again for the next copy.
        match system.jobs.as_ref().map(|jobs| jobs.try_send((frame, answer))) {
            Some(Ok(())) => {
                self.system = Some(system);
                Ok(done)
            }
            Some(Err(TrySendError::Full(_))) => {
                self.system = Some(system);
                Err("the clipboard is busy".into())
            }
            Some(Err(TrySendError::Disconnected(_))) | None => Err("the clipboard failed".into()),
        }
    }

    /// How soon [`Copier::on`] has more to do: at once while packets go out, soon while a render,
    /// a thread or an answer is waited for. `None` with no copy under way.
    pub(crate) fn wait(&self) -> Option<Duration> {
        match &self.copy.as_ref()?.stage {
            Stage::Sending(_) => Some(Duration::ZERO),
            Stage::Sent { said: true, .. } => None,
            _ => Some(POLL),
        }
    }

    /// When the last code the terminal answers went out (the question, a write's end), while its
    /// answer is still to come.
    pub(crate) fn due(&self) -> Option<Instant> {
        match (self.support, self.copy.as_ref().map(|c| &c.stage)) {
            (_, Some(Stage::Sent { since, said: false, .. })) => Some(*since),
            (Support::Asked(at), _) => Some(at),
            _ => None,
        }
    }
}

/// What the status line says of `copy` once copied: `copied page 3, 1224 × 1584 pixels`.
fn copied(copy: &Copying) -> String {
    let (w, h) = copy.size;
    match copy.smaller {
        Some((full_w, full_h)) => format!("copied {}, {w} × {h} pixels of its {full_w} × {full_h}", copy.what),
        None => format!("copied {}, {w} × {h} pixels", copy.what),
    }
}

/// Why the terminal didn't copy, in words.
fn why(error: ClipError) -> &'static str {
    match error {
        ClipError::Denied => "not allowed (EPERM)",
        ClipError::Busy => "busy (EBUSY); c tries again",
        ClipError::TooLarge => "too large for it (EFBIG)",
        ClipError::Failed => "it failed",
    }
}

/// `frame` made a PNG with `effort`, on a thread of its own: the PNG, or why not, from the
/// receiver.
fn png(frame: Frame, effort: PngEffort) -> Result<Receiver<Result<Vec<u8>, String>>, String> {
    let (done, png) = sync_channel(1);
    std::thread::Builder::new()
        .name("view-copy".into())
        .spawn(move || {
            let _ = done.send(encode::encode_png(frame.width, frame.height, &frame.rgba, effort));
        })
        .map_err(|e| format!("making its PNG: {e}"))?;
    Ok(png)
}

/// A copy for the system's clipboard, and where its answer goes.
type Job = (Frame, SyncSender<Result<(), String>>);

/// The system's clipboard, kept on a thread of its own for the pager's run: on X11 a picture is
/// on the clipboard only while the program that put it there is there to hand it over.
struct System {
    /// `None` once the pager ends, which ends the thread.
    jobs: Option<SyncSender<Job>>,
    /// Said by the thread once it let go of the clipboard.
    ended: Receiver<()>,
}

impl System {
    fn start() -> Result<Self, String> {
        let (jobs, take) = sync_channel::<Job>(1);
        let (end, ended) = sync_channel(1);
        std::thread::Builder::new()
            .name("view-clipboard".into())
            .spawn(move || {
                let mut clipboard = None;
                for (frame, answer) in take {
                    let _ = answer.send(put(&mut clipboard, &frame));
                }
                drop(clipboard);
                let _ = end.send(());
            })
            .map_err(|e| format!("the clipboard: {e}"))?;
        Ok(Self { jobs: Some(jobs), ended })
    }
}

impl Drop for System {
    /// The thread lets go of the clipboard once nothing more can come: on X11 that hands it to a
    /// clipboard manager, if there is one. Waited for a while, after the terminal is put back.
    fn drop(&mut self) {
        self.jobs = None;
        let _ = self.ended.recv_timeout(RELEASE_WAIT);
    }
}

/// `frame`'s pixels on the system's clipboard, `clipboard` opened the first time.
fn put(clipboard: &mut Option<arboard::Clipboard>, frame: &Frame) -> Result<(), String> {
    encode::check_raster(frame.width, frame.height, &frame.rgba)?;
    let (width, height) = (usize::try_from(frame.width), usize::try_from(frame.height));
    let (Ok(width), Ok(height)) = (width, height) else { return Err("too large".into()) };
    let mut rgba = frame.rgba.to_vec();
    encode::unpremultiply(&mut rgba);
    let clipboard = match clipboard {
        Some(clipboard) => clipboard,
        None => clipboard.insert(arboard::Clipboard::new().map_err(|e| format!("no clipboard here ({e})"))?),
    };
    clipboard.set_image(arboard::ImageData { width, height, bytes: Cow::Owned(rgba) }).map_err(|e| format!("the clipboard: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Pixels;

    /// The packets in `bytes`, as (metadata, decoded data) pairs. Panics on anything else.
    fn packets(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        let text = std::str::from_utf8(bytes).unwrap();
        let mut out = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            let body = rest.strip_prefix("\x1b]5522;").unwrap_or_else(|| panic!("not a packet: {rest:?}"));
            let end = body.find("\x1b\\").unwrap();
            let (metadata, data) = body[..end].split_once(';').unwrap_or((&body[..end], ""));
            out.push((metadata.to_string(), base64::engine::general_purpose::STANDARD.decode(data).unwrap()));
            rest = &body[end + 2..];
        }
        out
    }

    #[test]
    fn a_picture_is_copied_at_its_own_pixels_and_a_page_at_twice_its_points() {
        let picture = |w, h| Size { w, h, natural: true };
        let page = |w, h| Size { w, h, natural: false };
        assert_eq!(scale(picture(640.0, 480.0)), Ok(1.0));
        // US Letter at 144 per inch: 1224 × 1584 pixels.
        assert_eq!(scale(page(612.0, 792.0)), Ok(2.0));
        // Within 8192 a side and 16 megapixels in all, as on screen: a long strip, a large
        // photo, a poster.
        assert_eq!(scale(picture(20_000.0, 100.0)), Ok(MAX_PIXELS / 20_000.0));
        let photo = scale(picture(6000.0, 4000.0)).unwrap();
        assert!((f64::from(6000.0 * photo) * f64::from(4000.0 * photo) - MAX_AREA).abs() < 1e3, "{photo}");
        let poster = scale(page(2384.0, 3370.0)).unwrap();
        assert!(poster < SCALE && f64::from(2384.0 * poster) * f64::from(3370.0 * poster) <= MAX_AREA * 1.0001);
        // However large, a page is copied within those; a size that is no size is no copy, and
        // never a panic.
        let huge = scale(page(f32::MAX, f32::MAX)).unwrap();
        assert!(huge > 0.0 && f64::from(f32::MAX * huge).powi(2) <= MAX_AREA * 1.0001, "{huge}");
        for (w, h) in [(0.0, 10.0), (f32::NAN, 1.0), (-3.0, 4.0), (f32::INFINITY, 1.0), (1.0, f32::NEG_INFINITY)] {
            assert!(scale(page(w, h)).is_err(), "{w} × {h}");
        }
    }

    #[test]
    fn the_question_is_decrqm_for_mode_5522_wrapped_for_tmux() {
        let mut out = Vec::new();
        ask(&mut out, Wrap::None);
        assert_eq!(out, b"\x1b[?5522$p");
        out.clear();
        ask(&mut out, Wrap::Tmux);
        assert_eq!(out, b"\x1bPtmux;\x1b\x1b[?5522$p\x1b\\");
    }

    #[test]
    fn a_write_is_a_start_the_png_in_chunks_of_4096_and_an_end() {
        let png: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let mut write = Write::new(png.clone(), 7, Wrap::None);
        let mut out = Vec::new();
        assert!(!write.next(&mut out, usize::MAX), "all of it in one round");
        let packets = packets(&out);
        let mime = base64::engine::general_purpose::STANDARD.encode("image/png");
        let metadata: Vec<&str> = packets.iter().map(|(m, _)| m.as_str()).collect();
        let chunk = format!("type=wdata:mime={mime}:id=clv-7");
        assert_eq!(metadata, ["type=write:id=clv-7", &chunk, &chunk, &chunk, "type=wdata:id=clv-7"]);
        assert_eq!(packets.iter().map(|(_, d)| d.len()).collect::<Vec<_>>(), [0, 4096, 4096, 1808, 0]);
        assert_eq!(packets.iter().flat_map(|(_, d)| d.clone()).collect::<Vec<_>>(), png);
        // Nothing more once ended.
        out.clear();
        assert!(!write.next(&mut out, usize::MAX));
        assert!(out.is_empty());
    }

    /// A round writes as many packets as fit its bytes, at least one, so the keys are read
    /// between rounds; the rounds together are the whole write.
    #[test]
    fn a_write_goes_a_round_at_a_time() {
        let png = vec![0x5a; 3 * CHUNK + 1];
        let mut write = Write::new(png.clone(), 1, Wrap::None);
        let (mut all, mut rounds) = (Vec::new(), 0);
        loop {
            let mut out = Vec::new();
            let more = write.next(&mut out, 1);
            assert!(!out.is_empty());
            all.extend_from_slice(&out);
            rounds += 1;
            if !more {
                break;
            }
        }
        // The start, then a chunk a round, then the end.
        assert_eq!(rounds, 6);
        assert_eq!(packets(&all).into_iter().flat_map(|(_, d)| d).collect::<Vec<_>>(), png);
    }

    #[test]
    fn through_tmux_each_packet_is_a_passthrough_of_its_own() {
        let png = vec![1u8; 5000];
        let (mut plain, mut wrapped) = (Vec::new(), Vec::new());
        Write::new(png.clone(), 3, Wrap::None).next(&mut plain, usize::MAX);
        Write::new(png, 3, Wrap::Tmux).next(&mut wrapped, usize::MAX);
        assert_eq!(wrapped.windows(7).filter(|w| w == b"\x1bPtmux;").count(), 4);
        let unwrapped: Vec<u8> =
            String::from_utf8(wrapped).unwrap().replace("\x1bPtmux;", "").replace("\x1b\\\x1b\\", "\x1b\\").replace("\x1b\x1b", "\x1b").into_bytes();
        assert_eq!(unwrapped, plain);
    }

    #[test]
    fn a_copy_goes_to_the_terminal_when_it_speaks_the_protocol_else_to_this_machines() {
        assert_eq!(way(true, true), Way::Terminal);
        assert_eq!(way(true, false), Way::Terminal, "over SSH too");
        assert_eq!(way(false, true), Way::System);
        assert_eq!(way(false, false), Way::Nowhere);
    }

    /// A 2 × 1 page rendered for `request`, or failed.
    fn frame(request: Request, error: Option<&str>) -> Frame {
        let rgba = Pixels::new(vec![255u8, 0, 0, 255, 0, 0, 255, 255]);
        Frame { request, width: 2, height: 1, rgba, error: error.map(String::from), warnings: Vec::new() }
    }

    const PAGE: Size = Size { w: 1.0, h: 0.5, natural: false };

    /// `copier`'s copy carried on until all of it went to the terminal, `out` collecting it,
    /// with nothing said meanwhile.
    fn until_sent(copier: &mut Copier, out: &mut Vec<u8>) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !matches!(copier.copy.as_ref().map(|c| &c.stage), Some(Stage::Sent { .. })) {
            assert_eq!(copier.on(Instant::now(), out), None);
            assert!(Instant::now() < deadline, "never sent");
        }
    }

    #[test]
    fn a_copy_to_the_terminal_asks_renders_sends_and_says_what_it_answered() {
        // Over SSH, so the system's clipboard is never touched here.
        let mut copier = Copier::new(false, PngEffort::Fast, Wrap::None);
        let now = Instant::now();
        let mut out = Vec::new();
        assert_eq!(copier.start("page 3".into(), 1, 2, Some(PAGE), 40, now, &mut out), "copying page 3…");
        assert_eq!(out, b"\x1b[?5522$p", "asked the first time");
        // Its render goes in the level's queue, and only that level's.
        assert_eq!(copier.request(1), Some(Request { page: 2, scale: 2.0, tag: 40 }));
        assert_eq!(copier.request(2), None);
        assert!(copier.wants(40) && !copier.wants(41));
        // Another copy waits for this one.
        let mut again = Vec::new();
        assert_eq!(copier.start("page 4".into(), 1, 3, Some(PAGE), 41, now, &mut again), "still copying page 3");
        assert!(again.is_empty());
        copier.rendered(frame(Request { page: 2, scale: 2.0, tag: 40 }, None));
        assert_eq!(copier.wait(), Some(POLL));
        // Until the terminal answers, the page waits.
        out.clear();
        assert_eq!(copier.on(now, &mut out), None);
        assert!(copier.answer(Key::Clipboard(true)).is_none());
        let mut out = Vec::new();
        until_sent(&mut copier, &mut out);
        let packets = packets(&out);
        assert_eq!(packets.first().map(|(m, _)| m.as_str()), Some("type=write:id=clv-1"));
        let png: Vec<u8> = packets.iter().flat_map(|(_, d)| d.clone()).collect();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "{png:?}");
        assert!(copier.due().is_some(), "its answer waited for at the end");
        // An answer to someone else's write, or to an older one, says nothing.
        assert_eq!(copier.answer(Key::Copied(9, Ok(()))), None);
        assert_eq!(copier.answer(Key::Copied(1, Ok(()))).as_deref(), Some("copied page 3, 2 × 1 pixels"));
        assert!(copier.copy.is_none() && copier.wait().is_none() && copier.due().is_none());
        // The terminal is asked once: the next copy goes straight on.
        let mut out = Vec::new();
        copier.start("page 4".into(), 1, 3, Some(PAGE), 41, now, &mut out);
        assert!(out.is_empty());
        copier.rendered(frame(Request { page: 3, scale: 2.0, tag: 41 }, None));
        until_sent(&mut copier, &mut Vec::new());
        assert_eq!(copier.answer(Key::Copied(2, Err(ClipError::Denied))).as_deref(), Some("the terminal didn't copy page 4: not allowed (EPERM)"));
    }

    #[test]
    fn a_terminal_without_the_protocol_over_ssh_gets_no_copy_and_is_told_why() {
        let mut copier = Copier::new(false, PngEffort::Fast, Wrap::None);
        let then = Instant::now();
        copier.start("beach.jpg".into(), 0, 4, Some(PAGE), 7, then, &mut Vec::new());
        copier.rendered(frame(Request { page: 4, scale: 2.0, tag: 7 }, None));
        // No answer at all, a second on: it doesn't speak it.
        let said = copier.on(then + ASK_WAIT, &mut Vec::new()).unwrap();
        assert!(said.starts_with("can't copy beach.jpg: this terminal has no clipboard for pictures"), "{said}");
        // Said so, it isn't asked again.
        let mut out = Vec::new();
        copier.start("beach.jpg".into(), 0, 4, Some(PAGE), 8, then, &mut out);
        assert!(out.is_empty());
        copier.rendered(frame(Request { page: 4, scale: 2.0, tag: 8 }, None));
        assert!(copier.on(then, &mut Vec::new()).is_some_and(|s| s.starts_with("can't copy")));
    }

    #[test]
    fn a_copy_that_fails_or_is_left_says_so() {
        let mut copier = Copier::new(false, PngEffort::Fast, Wrap::None);
        let now = Instant::now();
        copier.start("page 1".into(), 0, 0, Some(PAGE), 1, now, &mut Vec::new());
        copier.rendered(frame(Request { page: 0, scale: 2.0, tag: 1 }, Some("broken")));
        assert_eq!(copier.on(now, &mut Vec::new()).as_deref(), Some("can't copy page 1: broken"));
        // A page with no size, or none at all, is never asked for.
        assert_eq!(
            copier.start("page 2".into(), 0, 1, Some(Size { w: 0.0, h: 1.0, natural: false }), 2, now, &mut Vec::new()),
            "can't copy page 2: it has no size"
        );
        assert_eq!(copier.start("page 9".into(), 0, 8, None, 2, now, &mut Vec::new()), "can't copy page 9: it isn't there");
        assert!(copier.copy.is_none());
        // Leaving its level while it renders stops it; leaving another level doesn't.
        copier.start("page 2".into(), 5, 1, Some(PAGE), 3, now, &mut Vec::new());
        assert_eq!(copier.left(4), None);
        assert_eq!(copier.left(5).as_deref(), Some("stopped copying page 2"));
        assert!(copier.request(5).is_none());
        // A large picture is copied smaller, and says of what.
        let photo = Size { w: 6000.0, h: 4000.0, natural: true };
        copier.start("photo.jpg".into(), 0, 0, Some(photo), 4, now, &mut Vec::new());
        let copy = copier.copy.as_mut().unwrap();
        copy.size = (4899, 3266);
        assert_eq!(copied(copy), "copied photo.jpg, 4899 × 3266 pixels of its 6000 × 4000");
    }

    #[test]
    fn a_write_tmux_dropped_part_of_or_the_terminal_never_answered_says_so() {
        let mut copier = Copier::new(false, PngEffort::Fast, Wrap::Tmux);
        let then = Instant::now();
        copier.start("page 1".into(), 0, 0, Some(PAGE), 1, then, &mut Vec::new());
        copier.answer(Key::Clipboard(true));
        assert_eq!(copier.lost(), None, "nothing went out yet");
        copier.rendered(frame(Request { page: 0, scale: 2.0, tag: 1 }, None));
        until_sent(&mut copier, &mut Vec::new());
        let sent = Instant::now();
        assert_eq!(copier.on(sent, &mut Vec::new()), None);
        let said = copier.on(sent + SENT_WAIT, &mut Vec::new()).unwrap();
        assert!(said.starts_with("sent page 1 to the terminal's clipboard"), "{said}");
        // Said once; a late answer is still said, and another copy may start meanwhile.
        assert_eq!((copier.on(sent + SENT_WAIT * 2, &mut Vec::new()), copier.wait(), copier.due()), (None, None, None));
        assert_eq!(copier.answer(Key::Copied(1, Ok(()))).as_deref(), Some("copied page 1, 2 × 1 pixels"));
        // tmux throwing output away while one goes out ends it.
        copier.start("page 2".into(), 0, 1, Some(PAGE), 2, then, &mut Vec::new());
        copier.rendered(frame(Request { page: 1, scale: 2.0, tag: 2 }, None));
        while !matches!(copier.copy.as_ref().map(|c| &c.stage), Some(Stage::Sending(_) | Stage::Sent { .. })) {
            copier.on(Instant::now(), &mut Vec::new());
        }
        assert_eq!(copier.lost().as_deref(), Some("tmux dropped part of the copy of page 2; c copies it again"));
        assert!(copier.copy.is_none());
    }
}
