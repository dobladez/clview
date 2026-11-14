//! Pictures a document refers to by a path, drawn by whichever backend shows them: what this build
//! shows as a picture, a drawing or a PDF's page, a document shows too, turned as it would show it.
//! Only what is on this machine, near the document: nothing is ever fetched
//! ([ADR-0013](../../docs/adr/0013-a-documents-pictures-are-drawn-by-the-backends.md)).
//!
//! A fenced block of the document is a picture too, when a backend lists the language it is
//! written in ([`Pictures::fenced`]): drawn from the block's text, by that backend, as a picture by
//! path is from its file, and so with no directory to find it in
//! ([ADR-0032](../../docs/adr/0032-a-fenced-block-is-drawn-by-the-backend-that-lists-its-language.md)).
//!
//! A document is laid out around a stand-in for each picture ([`Pictures::measured`]), a PNG saying
//! only the size to show it at, so that opening one reads no picture's pixels and a list of documents
//! holds none. A picture is drawn when a page it is on first renders ([`Promised::draw`]), into the
//! place its stand-in was given, at a resolution of its own: a small drawing stays sharp.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use super::{Again, BACKENDS, Backend, Bounded, HEAD, Input, Opened, Request, Source, guarded, shown_by};
use crate::args::Parsed;
use crate::encode::{PngEffort, encode_png};

/// The points a picture's pixel, or a unit of a drawing's or a page's size, is shown at: a pixel of
/// CSS's, as Markdown and HTML mean their pictures' sizes.
const POINTS: f32 = 0.75;

/// The longest side a picture is held at, in pixels: more than the column it fills shows, at three
/// pixels a point and more, on any but the largest windows.
const MAX_SIDE: f32 = 1600.0;

/// The pixels a point of what scales (a drawing, a PDF's page) is drawn at: a page in a window of
/// 2,000 pixels shows about three.
const SHARPNESS: f32 = 3.0;

/// The most pictures a document may refer to, the fenced blocks it would draw among them. Each is
/// opened to learn its size.
const MAX_PICTURES: usize = 1024;

/// What the key of a fenced block's picture starts with, before what is made afresh for each reading
/// of a document ([`Pictures::fenced`]): as a URL's scheme, so that it is never a path ([`local`]).
const FENCED: &str = "fenced:";

/// The most a document holds of its pictures, as PNGs, before it lets go of those drawn longest ago:
/// 256 MiB, some 50 photographs at [`MAX_SIDE`].
const MAX_HELD: usize = 256 << 20;

/// What a document holds of a picture: its stand-in, or the picture, as a PNG.
type Png = Arc<Vec<u8>>;

/// What `read` returns, or an error if a backend panics: its bug costs the picture, not the document.
fn caught<T>(read: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|_| Err("it failed on this picture".into()))
}

#[cfg(test)]
thread_local! {
    /// The backends that draw the fenced blocks of the documents a test opens on this thread, in
    /// place of those built: for the documents' backend's tests, which name no backend.
    pub(super) static AMONG: Cell<Option<&'static [&'static dyn Backend]>> = const { Cell::new(None) };
}

/// The pictures of a document being read, each on its way to a [`Promise`].
pub(super) struct Pictures<'a> {
    /// The directory the document is in, what its pictures' paths start from; none for one come
    /// through a pipe or held in an archive, whose pictures can't be reached.
    beside: Option<&'a Path>,
    /// The backends asked which draws a fenced block: those built, but in the tests.
    backends: &'a [&'static dyn Backend],
    /// By the name a document refers to it by: its stand-in, or none if it isn't shown.
    seen: RefCell<HashMap<String, Option<Png>>>,
    /// A fenced block's stand-in, by the key the document's HTML is given for it.
    blocks: RefCell<HashMap<String, Png>>,
    /// What comes in each block's key, made afresh for this reading of the document: nothing the
    /// document can know, so nothing it can write in an `<img src>` of its own.
    afresh: u64,
    /// How many fenced blocks it was asked to draw, drawn or not.
    fences: Cell<usize>,
    /// How many of them aren't drawn, and stay code.
    undrawn: Cell<usize>,
    /// Whether no fenced block is drawn: each stays the code it is ([`Pictures::blocks_as_code`]).
    as_code: bool,
    /// How many blocks a backend lists are shown as code, each with a marker of its own
    /// ([`Pictures::coded`]).
    coded: Cell<usize>,
    promises: RefCell<HashMap<Png, Promise>>,
    /// How many it refers to on this machine aren't shown.
    left_out: Cell<usize>,
}

impl<'a> Pictures<'a> {
    pub(super) fn beside(beside: Option<&'a Path>) -> Self {
        #[cfg(test)]
        if let Some(backends) = AMONG.get() {
            return Self::among(beside, backends);
        }
        Self::among(beside, BACKENDS)
    }

    /// [`Pictures::beside`], a fenced block drawn by one of `backends`.
    pub(super) fn among(beside: Option<&'a Path>, backends: &'a [&'static dyn Backend]) -> Self {
        Self {
            beside,
            backends,
            seen: RefCell::default(),
            blocks: RefCell::default(),
            afresh: afresh(),
            fences: Cell::new(0),
            undrawn: Cell::new(0),
            as_code: false,
            coded: Cell::new(0),
            promises: RefCell::default(),
            left_out: Cell::new(0),
        }
    }

    /// These pictures, with no fenced block drawn: each stays the code it is, and none is counted.
    pub(super) fn blocks_as_code(self) -> Self {
        Self { as_code: true, ..self }
    }

    /// Whether a backend lists `language` for a fenced block, drawn here or not: a block a document
    /// shows as a picture in one layout and as its code in another.
    pub(super) fn lists(&self, language: &str) -> bool {
        super::fenced(language, self.backends).is_some()
    }

    /// The marker of a block a backend lists that is shown as its code: a name for the invisible
    /// anchors the document's HTML puts on its lines, by which the lines are known to be one block's
    /// once the document is read ([`Promised::coded`]). Made with what is afresh for this reading, as
    /// a block's key is, so no anchor a document writes itself is one.
    pub(super) fn coded(&self) -> String {
        let block = self.coded.get();
        self.coded.set(block.saturating_add(1));
        format!("{}{block}", coded_prefix(self.afresh))
    }

    /// How many pictures the document has asked for so far, by path or as fenced blocks, shown or not.
    fn asked(&self) -> usize {
        self.seen.borrow().len().saturating_add(self.fences.get())
    }

    /// The key of a picture of the fenced block `text`, written in `language`, to fit in `fit`: what
    /// the document's HTML names in an `<img src>` of the block, for [`Pictures::measured`] to give
    /// its stand-in. The backend that lists the language opens the text to size it, under a name of
    /// its format (`block.mmd`), straight through its `open`: whether it would open a file of that
    /// name is never asked. None for a language no backend lists; and for a block that isn't drawn,
    /// which is counted ([`Promised::undrawn`]): one its backend can't open, or past the document's
    /// [`MAX_PICTURES`]. The document shows either as its code.
    pub(super) fn fenced(&self, language: &str, text: &str, fit: (f32, f32)) -> Option<String> {
        if self.as_code {
            return None;
        }
        let (backend, format) = super::fenced(language, self.backends)?;
        let within = self.asked() < MAX_PICTURES;
        self.fences.set(self.fences.get().saturating_add(1));
        let measured = || {
            let name = format.extensions.first().map_or_else(|| "block".to_owned(), |extension| format!("block.{extension}"));
            self.promised(Origin::Block { backend, name, text: Arc::new(text.as_bytes().to_vec()) }, fit)
        };
        let Some(stand_in) = within.then(measured).flatten() else {
            self.undrawn.set(self.undrawn.get().saturating_add(1));
            return None;
        };
        let mut blocks = self.blocks.borrow_mut();
        let key = format!("{FENCED}{:016x}:{}", self.afresh, blocks.len());
        blocks.insert(key.clone(), stand_in);
        Some(key)
    }

    /// A stand-in for the picture `reference` names, to fit in `fit` (width and height, in points),
    /// if it is one this build shows, on this machine. None otherwise, and the document shows what it
    /// says instead of it.
    pub(super) fn measured(&self, reference: &str, fit: (f32, f32)) -> Option<Png> {
        // A fenced block's key, as `fenced` gave it: nothing the document can have written itself,
        // and nothing `local` takes for a path.
        if let Some(block) = self.blocks.borrow().get(reference) {
            return Some(block.clone());
        }
        let beside = self.beside?;
        let name = local(reference)?;
        if let Some(known) = self.seen.borrow().get(&name) {
            return known.clone();
        }
        let path = beside.join(&name);
        let promised = || self.promised(Origin::Path(path.to_str()?.to_owned()), fit);
        let stand_in = (self.asked() < MAX_PICTURES).then(promised).flatten();
        if stand_in.is_none() {
            self.left_out.set(self.left_out.get().saturating_add(1));
        }
        self.seen.borrow_mut().insert(name, stand_in.clone());
        stand_in
    }

    /// The stand-in for the picture drawn from `origin`, and the promise to draw it.
    fn promised(&self, origin: Origin, fit: (f32, f32)) -> Option<Png> {
        let (size, natural) = caught(|| {
            let source = origin.opened()?;
            let size = source.size(0).ok_or("it has no first page")?;
            Ok((size, source.natural(0)))
        })
        .ok()?;
        let shown = shown(size, fit)?;
        let mut promises = self.promises.borrow_mut();
        let stand_in = Arc::new(stand_in(shown, promises.len()).ok()?);
        promises.insert(stand_in.clone(), Promise { origin, scale: scale(size, natural, shown), shown, stand_in: stand_in.clone() });
        Some(stand_in)
    }

    /// The size in points `held`, a stand-in, was measured to be shown at, if it is one: exactly, where
    /// its own pixels say it only to the nearest one.
    pub(super) fn shown(&self, held: &Png) -> Option<(f32, f32)> {
        self.promises.borrow().get(held).map(|promise| promise.shown)
    }

    /// These pictures as a document that was read holds them: `media`, by key, what it holds of
    /// each, the stand-ins among them being the ones promised.
    pub(super) fn promised_in(self, media: &BTreeMap<String, Png>) -> Promised {
        let promises = self.promises.into_inner();
        let by_key = media.iter().filter_map(|(key, held)| Some((key.clone(), promises.get(held)?.clone()))).collect();
        let (left_out, undrawn, fenced) = (self.left_out.get(), self.undrawn.get(), self.blocks.borrow().len());
        Promised { by_key, left_out, undrawn, fenced, coded: coded_prefix(self.afresh), max_held: MAX_HELD, held: Mutex::default() }
    }
}

/// What the marker of a block shown as code starts with, in a reading whose keys hold `afresh`: the
/// block's number follows.
fn coded_prefix(afresh: u64) -> String {
    format!("{FENCED}{afresh:016x}:code:")
}

/// A number no document can know, made afresh each time: the hash of nothing under a key of std's
/// `RandomState`, which is keyed from the system's randomness and never twice the same.
fn afresh() -> u64 {
    use std::hash::{BuildHasher as _, Hasher as _};
    std::collections::hash_map::RandomState::new().build_hasher().finish()
}

/// What a picture is drawn from.
#[derive(Clone)]
enum Origin {
    /// A file near the document, by its path.
    Path(String),
    /// A fenced block of the document: its text, for the backend that lists its language to open
    /// under `name`.
    Block { backend: &'static dyn Backend, name: String, text: Arc<Vec<u8>> },
}

impl Origin {
    /// The picture opened, by the backend that draws it.
    fn opened(&self) -> Result<Box<dyn Source>, String> {
        let (backend, name, text) = match self {
            Self::Path(path) => return opened(path),
            Self::Block { backend, name, text } => (backend, name, text),
        };
        let head = text.get(..HEAD as usize).unwrap_or(text.as_slice()).to_vec();
        let (whole, within) = (text.clone(), text.clone());
        let again: Again = Arc::new(move || Ok(whole.to_vec()));
        // A backend that reads only so much of a file is given no copy of a longer text.
        let bounded: Bounded = Arc::new(move |most| Ok((u64::try_from(within.len()).unwrap_or(u64::MAX) <= most).then(|| within.to_vec())));
        match backend.open(Input::held_bounded(name.clone(), head, again, bounded), &Parsed::default())? {
            Opened::Pages(source) => Ok(source),
            Opened::Files(_) => Err("it holds files".into()),
        }
    }
}

/// A picture to draw when it is wanted.
#[derive(Clone)]
struct Promise {
    origin: Origin,
    /// What to draw it at.
    scale: f32,
    /// The size in points to show it at.
    shown: (f32, f32),
    /// What a document held until now, and holds again once it lets go of the picture.
    stand_in: Png,
}

impl Promise {
    /// Whether the picture is drawn for the paper it is shown on: a fenced block, which its backend
    /// may draw otherwise on dark paper ([`Request::on_dark`]). A picture by path is one picture.
    fn follows_paper(&self) -> bool {
        matches!(self.origin, Origin::Block { .. })
    }

    /// The picture, as a PNG; as a list's `tile` where the page is one, so that its backend leaves
    /// out what a tile shouldn't cost (an SVG's HTML labels); and for dark paper of the colour `dark`
    /// where the page is on it and the picture follows its paper.
    fn draw(&self, tile: bool, dark: Option<[u8; 3]>) -> Result<Vec<u8>, String> {
        caught(|| {
            let source = self.origin.opened()?;
            let mut renderer = source.renderer()?;
            let dark = dark.filter(|_| self.follows_paper());
            let request = Request { page: 0, scale: self.scale, tile, on_dark: dark.is_some(), dark: dark.unwrap_or_default(), ..Default::default() };
            let frame = guarded(request, || renderer.render(request));
            if let Some(error) = frame.error {
                return Err(error);
            }
            // Decided once and for all: on white paper the white can't be seen. Not for what was
            // drawn for dark paper, whose ink is that paper's.
            match dark.is_none() && thin_and_near_black(&frame.rgba) {
                true => encode_png(frame.width, frame.height, &on_white(&frame.rgba), PngEffort::Fast),
                false => encode_png(frame.width, frame.height, &frame.rgba, PngEffort::Fast),
            }
        })
    }
}

/// A document's pictures, drawn as its pages want them.
pub(super) struct Promised {
    by_key: HashMap<String, Promise>,
    left_out: usize,
    undrawn: usize,
    /// How many fenced blocks the document shows as pictures.
    fenced: usize,
    /// What the markers of its blocks shown as code start with ([`Pictures::coded`]).
    coded: String,
    max_held: usize,
    held: Mutex<Held>,
}

/// What a document holds of its pictures now.
#[derive(Default)]
struct Held {
    /// Each drawn, by its key, and its size in bytes, the one drawn longest ago first.
    drawn: VecDeque<(String, usize)>,
    /// Those of `drawn` drawn as a list's tile, without what a tile leaves out: drawn again for
    /// a page.
    as_tiles: HashSet<String>,
    /// Those of `drawn` drawn for dark paper, of the pictures that follow their paper: drawn again
    /// for a page on light paper, as the others are for one on dark.
    for_dark: HashSet<String>,
    bytes: usize,
    /// Those that could not be drawn, not tried again: each with the paper it was to be drawn for,
    /// if it follows its paper (dark, or not), so that one that fails for one paper is still tried
    /// for the other.
    failed: HashSet<(String, bool)>,
}

impl Held {
    /// Whether the picture `key` is held as a page wants it: drawn, for a page rather than a tile
    /// unless a `tile` wants it, and for the paper it is on (`dark`, where it follows its paper).
    fn shows(&self, key: &str, tile: bool, dark: bool) -> bool {
        self.drawn.iter().any(|(drawn, _)| drawn == key) && (tile || !self.as_tiles.contains(key)) && self.for_dark.contains(key) == dark
    }

    /// Whether the picture `key` is held as drawn for the other paper than `dark`: nothing a page on
    /// this one may show.
    fn for_the_other(&self, key: &str, dark: bool) -> bool {
        self.drawn.iter().any(|(drawn, _)| drawn == key) && self.for_dark.contains(key) != dark
    }
}

impl Promised {
    /// How many pictures the document refers to on this machine are left out, missing or not what
    /// this build shows.
    pub(super) fn left_out(&self) -> usize {
        self.left_out
    }

    /// How many fenced blocks the document would draw aren't drawn, and show as their code: those
    /// their backend couldn't open, or past the most pictures a document has.
    pub(super) fn undrawn(&self) -> usize {
        self.undrawn
    }

    /// How many fenced blocks the document shows as pictures, where it would have shown their code:
    /// that many blocks' text is no text of the document's.
    pub(super) fn fenced(&self) -> usize {
        self.fenced
    }

    /// Whether `key`, of what the document holds, is a fenced block's picture.
    pub(super) fn is_block(&self, key: &str) -> bool {
        self.by_key.get(key).is_some_and(Promise::follows_paper)
    }

    /// Which of the document's blocks shown as code `name` is the marker of, if it is one
    /// ([`Pictures::coded`]): its number among them.
    pub(super) fn coded(&self, name: &str) -> Option<usize> {
        name.strip_prefix(self.coded.as_str())?.parse().ok()
    }

    /// Whether any of the document's pictures is promised.
    pub(super) fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    /// Whether the picture `key` is one drawn for dark paper on a page on dark paper (`dark`): one
    /// that follows its paper. Any other is drawn the same on either.
    fn for_dark(&self, key: &str, dark: bool) -> bool {
        dark && self.by_key.get(key).is_some_and(Promise::follows_paper)
    }

    /// Those of `on_page` (keys of what a document holds) not drawn yet, drawn, or why not; for a
    /// list's `tile`, as tiles, and one drawn as a tile is drawn again for a page; for a page on dark
    /// paper of the colour `dark`, those that follow their paper for it, and one drawn for the other
    /// paper is drawn again. Nothing is locked meanwhile: other pages go on rendering, and another
    /// thread may draw one of these too.
    pub(super) fn draw<'k>(&self, on_page: &[&'k str], tile: bool, dark: Option<[u8; 3]>) -> Vec<(&'k str, Result<Vec<u8>, String>)> {
        let wanted: Vec<(&str, &Promise)> = {
            let held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
            let done = |key: &str| {
                let for_dark = self.for_dark(key, dark.is_some());
                held.failed.contains(&(key.to_owned(), for_dark)) || held.shows(key, tile, for_dark)
            };
            on_page.iter().filter(|key| !done(key)).filter_map(|key| Some((*key, self.by_key.get(*key)?))).collect()
        };
        wanted.into_iter().map(|(key, promise)| (key, promise.draw(tile, dark))).collect()
    }

    /// Whether every picture of `on_page` that is promised is held as a page wants it, a `tile` or
    /// not, on `dark` paper or not; or could not be drawn for that paper, and isn't held as drawn for
    /// the other, so that the document has its stand-in. The caller holds the document's lock for
    /// reading: then what the document holds is what this says, since nothing is held or let go of
    /// but under that lock for writing ([`Promised::hold`]).
    pub(super) fn settled(&self, on_page: &[&str], tile: bool, dark: bool) -> bool {
        let held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        on_page.iter().filter(|key| self.by_key.contains_key(**key)).all(|key| {
            let for_dark = self.for_dark(key, dark);
            match held.failed.contains(&((*key).to_owned(), for_dark)) {
                true => !held.for_the_other(key, for_dark),
                false => held.shows(key, tile, for_dark),
            }
        })
    }

    /// What the document should now hold, by key, for `drawn` ([`Promised::draw`]) on a page showing
    /// `on_page`, on `dark` paper or not: what it holds is let go of past [`MAX_HELD`], but not that
    /// page's. The caller holds the document's lock for writing from before this until what it returns
    /// is in the document. So a page that finds a picture drawn and goes to render finds it held, not
    /// its stand-in, since it must wait for that lock to read the document; unless another page had it
    /// let go of or drawn otherwise meanwhile, which [`Promised::settled`] tells.
    ///
    /// One that could not be drawn for this page's paper, now or before, and is held as drawn for the
    /// other, is let go of, back to its stand-in: a page shows the picture drawn for its paper or the
    /// stand-in, never the other paper's. A failure is kept for as long as the document is laid out as
    /// it is, each paper's apart, as a picture by path's is: not tried again until it is laid out again.
    pub(super) fn hold(&self, drawn: Vec<(&str, Result<Vec<u8>, String>)>, on_page: &[&str], tile: bool, dark: bool) -> Vec<(String, Png)> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let mut changes = Vec::new();
        for (key, picture) in drawn {
            let for_dark = self.for_dark(key, dark);
            match picture {
                Ok(picture) => {
                    if let Some(at) = held.drawn.iter().position(|(drawn, _)| drawn == key) {
                        // Another thread drew it meanwhile, for a page of its own; unless as a
                        // tile, and this is a page's, or for the other paper: this takes its place.
                        if held.shows(key, tile, for_dark) {
                            continue;
                        }
                        if let Some((_, size)) = held.drawn.remove(at) {
                            held.bytes = held.bytes.saturating_sub(size);
                        }
                    }
                    held.bytes = held.bytes.saturating_add(picture.len());
                    held.drawn.push_back((key.to_owned(), picture.len()));
                    match tile {
                        true => held.as_tiles.insert(key.to_owned()),
                        false => held.as_tiles.remove(key),
                    };
                    match for_dark {
                        true => held.for_dark.insert(key.to_owned()),
                        false => held.for_dark.remove(key),
                    };
                    changes.push((key.to_owned(), Arc::new(picture)));
                }
                Err(_) => {
                    held.failed.insert((key.to_owned(), for_dark));
                }
            }
        }
        for key in on_page {
            let for_dark = self.for_dark(key, dark);
            if !(held.failed.contains(&((*key).to_owned(), for_dark)) && held.for_the_other(key, for_dark)) {
                continue;
            }
            let Some(at) = held.drawn.iter().position(|(drawn, _)| drawn == key) else { continue };
            if let Some((key, size)) = held.drawn.remove(at) {
                held.as_tiles.remove(&key);
                held.for_dark.remove(&key);
                held.bytes = held.bytes.saturating_sub(size);
                if let Some(promise) = self.by_key.get(&key) {
                    changes.push((key, promise.stand_in.clone()));
                }
            }
        }
        while held.bytes > self.max_held {
            let Some(oldest) = held.drawn.iter().position(|(key, _)| !on_page.contains(&key.as_str())) else { break };
            let Some((key, size)) = held.drawn.remove(oldest) else { break };
            held.as_tiles.remove(&key);
            held.for_dark.remove(&key);
            held.bytes = held.bytes.saturating_sub(size);
            if let Some(promise) = self.by_key.get(&key) {
                changes.push((key, promise.stand_in.clone()));
            }
        }
        changes
    }

    /// Whether any of `on_page` could not be drawn, for a page on `dark` paper or not.
    pub(super) fn failed(&self, on_page: &[&str], dark: bool) -> bool {
        let held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        on_page.iter().any(|key| held.failed.contains(&((*key).to_owned(), self.for_dark(key, dark))))
    }
}

/// Whether a picture, as premultiplied RGBA, is thin ink that dark paper would swallow: near black
/// (a mean luma under 48 of what is opaque), on under half of it, the rest clear, as a logo or a line
/// drawing made for white paper is. A dark window's screenshot, however clear its shadow's margins, is
/// more than half window, and shows on dark paper as it is (ADR-0015).
fn thin_and_near_black(rgba: &[u8]) -> bool {
    let (mut clear, mut opaque, mut luma) = (0u64, 0u64, 0u64);
    let pixels = rgba.as_chunks::<4>().0;
    for [r, g, b, a] in pixels {
        match *a {
            0..16 => clear = clear.saturating_add(1),
            129.. => {
                opaque = opaque.saturating_add(1);
                // A luma in hundredths of a thousandth, undoing the premultiplication: below 2^32.
                let premultiplied = 2126 * u64::from(*r) + 7152 * u64::from(*g) + 722 * u64::from(*b);
                luma = luma.saturating_add(premultiplied * 255 / u64::from(*a) / 10000);
            }
            _ => {}
        }
    }
    let all = pixels.len() as u64;
    opaque > 0 && clear.saturating_mul(20) >= all && opaque.saturating_mul(2) < all && luma < opaque.saturating_mul(48)
}

/// `rgba`, premultiplied, on white.
fn on_white(rgba: &[u8]) -> Vec<u8> {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|[r, g, b, a]| [r.saturating_add(255 - a), g.saturating_add(255 - a), b.saturating_add(255 - a), 255])
        .collect()
}

/// The picture at `path` opened, by the first backend whose files may be pictures. Only a regular
/// file: a pipe or a device would never end, and a directory is no picture.
fn opened(path: &str) -> Result<Box<dyn Source>, String> {
    if !std::fs::metadata(path).map_err(|e| e.to_string())?.is_file() {
        return Err("not a file".into());
    }
    let input = Input::open(path)?;
    let backends: Vec<&dyn Backend> = BACKENDS.iter().copied().filter(|backend| backend.draws_pictures()).collect();
    let (backend, input) = shown_by(input, &backends).ok_or("not a picture this build shows")?;
    match backend.open(input, &Parsed::default())? {
        Opened::Pages(source) => Ok(source),
        Opened::Files(_) => Err("it holds files".into()),
    }
}

/// The path `reference` names on this machine, from the document's directory: none for a URL,
/// whatever its scheme (nothing is fetched), or an absolute path, which on the web a document's
/// pictures mean from their site, not from the root of this machine. `..` may lead out of the
/// document's directory, as `docs/usage.md` to `../assets/`.
fn local(reference: &str) -> Option<String> {
    let reference = reference.split(['?', '#']).next()?.trim();
    let scheme = reference.split_once(':').is_some_and(|(before, _)| !before.contains('/'));
    if reference.is_empty() || scheme {
        return None;
    }
    // After unescaping, since `%2Fetc` is `/etc`, and joined to a directory would replace it.
    let name = unescaped(reference)?;
    (!name.starts_with(['/', '\\', '~']) && Path::new(&name).is_relative()).then_some(name)
}

/// `text` with its `%XX` escapes undone, if that leaves text.
fn unescaped(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let hex = |at: usize| bytes.get(at).and_then(|digit| char::from(*digit).to_digit(16));
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0usize;
    while let Some(byte) = bytes.get(at) {
        match (*byte, hex(at.saturating_add(1)), hex(at.saturating_add(2))) {
            (b'%', Some(high), Some(low)) => {
                out.push(u8::try_from(high.saturating_mul(16).saturating_add(low)).ok()?);
                at = at.saturating_add(3);
            }
            _ => {
                out.push(*byte);
                at = at.saturating_add(1);
            }
        }
    }
    String::from_utf8(out).ok()
}

/// The size in points to show a picture of `size` at: its own, or smaller to fit in `fit`, keeping its
/// shape. None for a size that is none.
fn shown(size: (f32, f32), fit: (f32, f32)) -> Option<(f32, f32)> {
    let (w, h) = (size.0 * POINTS, size.1 * POINTS);
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return None;
    }
    let shrink = (fit.0 / w).min(fit.1 / h).min(1.0);
    Some(((w * shrink).max(1.0), (h * shrink).max(1.0)))
}

/// What to draw a picture of `size` at, shown at `shown`: a picture with pixels of its own at those,
/// and what scales as sharp as a page would want it, either within [`MAX_SIDE`].
fn scale(size: (f32, f32), natural: bool, shown: (f32, f32)) -> f32 {
    let wanted = if natural { 1.0 } else { shown.0 * SHARPNESS / size.0 };
    wanted.min(MAX_SIDE / size.0.max(size.1))
}

/// A PNG of a light grey that says only the size to show a picture at, `id` making it another PNG
/// than any other picture's of that size, since a document holds one of what is the same. If the
/// picture can't be drawn, this is what stays in its place.
fn stand_in(shown: (f32, f32), id: usize) -> Result<Vec<u8>, String> {
    let error = |e: png::EncodingError| e.to_string();
    // A size in points is a pixel count at `POINTS`, as a document takes it. `shown` is within a
    // page's text, so these rows are short.
    let side = |points: f32| ((points / POINTS).round() as u32).max(1);
    let (width, height) = (side(shown.0), side(shown.1));
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::One);
    encoder.set_palette(vec![0xD0, 0xD0, 0xD0]);
    encoder.add_text_chunk("id".into(), id.to_string()).map_err(error)?;
    let mut writer = encoder.write_header().map_err(error)?;
    let rows = (width as usize).div_ceil(8).saturating_mul(height as usize);
    writer.write_image_data(&vec![0; rows]).map_err(error)?;
    writer.finish().map_err(error)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sizes in `png`'s header.
    fn size_of(png: &[u8]) -> (u32, u32) {
        let info = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
        info.info().size()
    }

    #[test]
    fn what_a_document_may_refer_to_is_a_path_from_it() {
        for (reference, name) in [
            ("a.png", "a.png"),
            ("./a.png", "./a.png"),
            ("assets/a.png", "assets/a.png"),
            ("../assets/a.png", "../assets/a.png"),
            ("my%20picture.png", "my picture.png"),
            ("caf%C3%A9.png", "café.png"),
            ("100%.png", "100%.png"),
            ("a.png?raw=true", "a.png"),
            ("a.png#top", "a.png"),
            ("  a.png ", "a.png"),
            ("./with:colon.png", "./with:colon.png"),
        ] {
            assert_eq!(local(reference).as_deref(), Some(name), "{reference}");
        }
    }

    /// Nothing is fetched, and nothing is read from anywhere but near the document.
    #[test]
    fn a_url_or_an_absolute_path_is_not_read() {
        for reference in [
            "http://example.com/a.png",
            "https://example.com/a.png",
            "HTTPS://example.com/a.png",
            "//example.com/a.png",
            "file:///etc/hosts",
            "data:image/png;base64,AAAA",
            "ftp://example.com/a.png",
            "/etc/hosts",
            "%2Fetc%2Fhosts",
            "~/a.png",
            "%7E/a.png",
            "\\a.png",
            "",
            "   ",
            "#top",
            "?raw=true",
            "caf%FF.png",
        ] {
            assert_eq!(local(reference), None, "{reference}");
        }
    }

    #[test]
    fn a_picture_is_shown_at_its_own_size_or_smaller_keeping_its_shape() {
        let fit = (468.0, 648.0);
        assert_eq!(shown((200.0, 100.0), fit), Some((150.0, 75.0)), "at its size");
        assert_eq!(shown((3000.0, 1500.0), fit), Some((468.0, 234.0)), "to the column");
        assert_eq!(shown((1000.0, 8000.0), fit), Some((81.0, 648.0)), "to the page");
        assert_eq!(shown((1.0, 100_000.0), fit), Some((1.0, 648.0)), "never to nothing");
        for size in [(0.0, 10.0), (10.0, 0.0), (-1.0, 10.0), (f32::NAN, 10.0), (f32::INFINITY, 10.0)] {
            assert_eq!(shown(size, fit), None, "{size:?}");
        }
    }

    #[test]
    fn a_picture_is_drawn_no_larger_than_its_pixels_and_what_scales_sharply() {
        assert_eq!(scale((200.0, 100.0), true, (150.0, 75.0)), 1.0, "its own pixels");
        assert_eq!(scale((3200.0, 1600.0), true, (468.0, 234.0)), 0.5, "within the most it holds");
        assert_eq!(scale((24.0, 24.0), false, (18.0, 18.0)), 2.25, "a small drawing, three pixels a point");
        assert_eq!(scale((10_000.0, 10_000.0), false, (468.0, 468.0)), 0.1404, "a large one");
        assert_eq!(scale((612.0, 792.0), false, (459.0, 594.0)), 1600.0 / 792.0, "a page, within the most it holds");
    }

    /// What a document goes by: the size it says, in pixels of three quarters of a point, and that
    /// two pictures of a size aren't taken for one.
    #[test]
    fn a_stand_in_says_the_size_to_show_a_picture_at() {
        let one = stand_in((468.0, 648.0), 0).unwrap();
        assert_eq!(size_of(&one), (624, 864));
        assert_eq!(size_of(&stand_in((0.1, 0.1), 0).unwrap()), (1, 1));
        assert_ne!(one, stand_in((468.0, 648.0), 1).unwrap());
        assert!(one.len() < 1024, "{}", one.len());
    }

    /// `pixels` premultiplied RGBA, `clear` of them clear out of `of`, the rest this colour and opaque.
    fn with(colour: [u8; 3], opaque: usize, of: usize) -> Vec<u8> {
        let [r, g, b] = colour;
        [[r, g, b, 255].repeat(opaque), [0; 4].repeat(of - opaque)].concat()
    }

    #[test]
    fn only_thin_near_black_ink_would_be_swallowed_by_dark_paper() {
        const BLACK: [u8; 3] = [10, 10, 10];
        assert!(thin_and_near_black(&with(BLACK, 44, 100)), "a logo");
        assert!(thin_and_near_black(&with([45, 45, 45], 25, 100)), "an icon nearly as dark as the paper");
        assert!(!thin_and_near_black(&with(BLACK, 64, 100)), "a dark window, its shadow's margins clear");
        assert!(!thin_and_near_black(&with(BLACK, 100, 100)), "a dark picture with nothing clear");
        assert!(!thin_and_near_black(&with(BLACK, 96, 100)), "rounded corners");
        assert!(!thin_and_near_black(&with([200, 40, 40], 25, 100)), "a colour that shows");
        assert!(!thin_and_near_black(&with(BLACK, 0, 100)), "nothing at all");
        assert!(!thin_and_near_black(&[]));
        let half_seen: Vec<u8> = [100, 100, 100, 100].repeat(100);
        assert!(!thin_and_near_black(&half_seen), "nothing opaque or clear to go by");
    }

    #[test]
    fn what_is_put_on_white_keeps_its_ink() {
        assert_eq!(on_white(&[0, 0, 0, 0]), [255, 255, 255, 255], "clear");
        assert_eq!(on_white(&[10, 20, 30, 255]), [10, 20, 30, 255], "opaque");
        assert_eq!(on_white(&[50, 50, 50, 128]), [177, 177, 177, 255], "half seen");
    }

    /// What a document does for a page showing `on_page`: what it should now hold, and whether one of
    /// them could not be drawn.
    #[cfg(feature = "image")]
    fn drawn(promised: &Promised, on_page: &[&str]) -> (Vec<(String, Png)>, bool) {
        drawn_as(promised, on_page, false)
    }

    /// [`drawn`], for a page that is a list's tile where `tile`.
    #[cfg(feature = "image")]
    fn drawn_as(promised: &Promised, on_page: &[&str], tile: bool) -> (Vec<(String, Png)>, bool) {
        let changes = promised.hold(promised.draw(on_page, tile, None), on_page, tile, false);
        (changes, promised.failed(on_page, false))
    }

    /// Promises with a picture to draw, each a file of `png`, and what they hold at most.
    #[cfg(feature = "image")]
    fn promised(keys: &[&str], png: &[u8], max_held: usize) -> Promised {
        let path = std::env::temp_dir().join(format!("clview-picture-{}-{}.png", std::process::id(), keys.join("-")));
        std::fs::write(&path, png).unwrap();
        let promise = |key: &&str| {
            let stand_in = Arc::new(key.as_bytes().to_vec());
            (key.to_string(), Promise { origin: Origin::Path(path.to_str().unwrap().to_owned()), scale: 1.0, shown: (1.0, 1.0), stand_in })
        };
        Promised {
            by_key: keys.iter().map(promise).collect(),
            left_out: 0,
            undrawn: 0,
            fenced: 0,
            coded: String::new(),
            max_held,
            held: Mutex::default(),
        }
    }

    #[cfg(feature = "image")]
    fn picture() -> Vec<u8> {
        encode_png(2, 2, &[255; 16], PngEffort::Fast).unwrap()
    }

    #[cfg(feature = "image")]
    #[test]
    fn a_picture_is_drawn_once_and_only_those_a_page_shows() {
        let promised = promised(&["a", "b"], &picture(), MAX_HELD);
        let (changes, failed) = drawn(&promised, &["a", "not promised"]);
        assert!(!failed);
        assert_eq!(changes.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(size_of(&changes[0].1), (2, 2), "the picture itself");
        assert!(drawn(&promised, &["a"]).0.is_empty(), "already drawn");
        assert_eq!(drawn(&promised, &["b"]).0.len(), 1);
    }

    /// A picture drawn for a list's tile is drawn again for a page, which may show more of it (an
    /// SVG's HTML labels), and takes its place; drawn for a page, it serves a tile as it is.
    #[cfg(feature = "image")]
    #[test]
    fn a_picture_drawn_as_a_tile_is_drawn_again_for_a_page() {
        let promised = promised(&["a", "b", "tile"], &picture(), MAX_HELD);
        assert_eq!(drawn_as(&promised, &["a"], true).0.len(), 1);
        assert!(drawn_as(&promised, &["a"], true).0.is_empty(), "a tile's, drawn once");
        let (changes, _) = drawn_as(&promised, &["a"], false);
        assert_eq!(changes.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(), ["a"], "drawn again for a page");
        assert!(drawn_as(&promised, &["a"], true).0.is_empty(), "a page's serves a tile");
        assert!(drawn_as(&promised, &["a"], false).0.is_empty());
        assert_eq!(drawn_as(&promised, &["b"], false).0.len(), 1);
        assert!(drawn_as(&promised, &["b"], true).0.is_empty());
        assert_eq!(promised.held.lock().unwrap().bytes, picture().len() * 2, "the tile's picture let go of");
    }

    /// Past what it holds, it lets go of what was drawn longest ago, back to its stand-in, but not of
    /// what the page wanted shows; let go of, it is drawn again when it is wanted.
    #[cfg(feature = "image")]
    #[test]
    fn past_the_most_it_holds_a_document_lets_go_of_what_it_drew_longest_ago() {
        let png = picture();
        let promised = promised(&["a", "b", "c"], &png, png.len() * 2);
        assert_eq!(drawn(&promised, &["a"]).0.len(), 1);
        assert_eq!(drawn(&promised, &["b"]).0.len(), 1);
        let (changes, _) = drawn(&promised, &["c"]);
        let what = |key: &str| changes.iter().find(|(k, _)| k == key).map(|(_, held)| held.clone());
        assert_eq!(what("c").map(|held| held.len()), Some(png.len()), "drawn");
        assert_eq!(what("a").unwrap().as_slice(), b"a", "its stand-in again");
        assert!(what("b").is_none(), "within what it holds");
        assert_eq!(drawn(&promised, &["a"]).0.iter().filter(|(key, _)| key == "a").count(), 1, "drawn again");

        let everything = drawn(&promised, &["a", "b", "c"]);
        assert!(everything.0.iter().all(|(_, held)| held.len() == png.len()), "a page's own are never let go of");
    }

    #[cfg(feature = "image")]
    #[test]
    fn a_picture_that_cant_be_drawn_says_so_and_isnt_tried_again() {
        // A PNG's header, enough to open, and then nothing to decode.
        let mut broken = picture();
        broken.truncate(40);
        let promised = promised(&["broken"], &broken, MAX_HELD);
        assert_eq!(drawn(&promised, &["broken"]), (Vec::new(), true));
        assert_eq!(drawn(&promised, &["other"]), (Vec::new(), false), "it says so for a page it is on");
        assert_eq!(drawn(&promised, &["broken"]), (Vec::new(), true));
    }

    #[cfg(feature = "image")]
    #[test]
    fn a_document_refers_to_so_many_pictures_and_says_how_many_it_left_out() {
        let dir = std::env::temp_dir().join(format!("clview-pictures-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = picture();
        for n in 0..=MAX_PICTURES {
            std::fs::write(dir.join(format!("{n}.png")), &png).unwrap();
        }
        let pictures = Pictures::beside(Some(&dir));
        let fit = (468.0, 648.0);
        for n in 0..MAX_PICTURES {
            assert!(pictures.measured(&format!("{n}.png"), fit).is_some(), "{n}");
        }
        assert!(pictures.measured(&format!("{MAX_PICTURES}.png"), fit).is_none(), "one too many");
        assert_eq!(pictures.left_out.get(), 1);
        assert!(pictures.measured("0.png", fit).is_some(), "one seen is not another");
        assert!(pictures.measured("http://example.com/a.png", fit).is_none());
        assert_eq!(pictures.left_out.get(), 1, "a URL is not left out: it isn't on this machine");
    }

    #[cfg(feature = "image")]
    #[test]
    fn a_document_without_a_directory_has_no_pictures() {
        let pictures = Pictures::beside(None);
        assert!(pictures.measured("a.png", (468.0, 648.0)).is_none());
        assert_eq!(pictures.left_out.get(), 0);
    }

    use crate::source::tests::Sketches;

    /// The backends of the tests of fenced blocks: one draws a ` ```sketch `, of a size its text
    /// says, and one lists a fence but draws no pictures.
    static SKETCHES: Sketches = Sketches { fences: &["sketch"], pictures: true };
    static HOLDS: Sketches = Sketches { fences: &["held"], pictures: false };
    static DRAWING: [&dyn Backend; 2] = [&HOLDS, &SKETCHES];

    const FIT: (f32, f32) = (468.0, 648.0);

    /// A fenced block in a language a backend lists is sized by that backend opening its text, with
    /// no directory to find anything in, and drawn by it when its page wants it; in any other
    /// language it is none of this module's, and one its backend refuses is counted as not drawn.
    #[test]
    fn a_fenced_block_is_a_picture_of_its_text_by_the_backend_that_lists_its_language() {
        let pictures = Pictures::among(None, &DRAWING);
        assert!(pictures.lists("sketch") && pictures.lists("SKETCH") && !pictures.lists("svg") && !pictures.lists("held"));
        let key = pictures.fenced("Sketch", "400 200\n", FIT).expect("drawn");
        let stand_in = pictures.measured(&key, FIT).expect("its stand-in");
        assert_eq!(size_of(&stand_in), (400, 200));
        assert_eq!(pictures.shown(&stand_in), Some((300.0, 150.0)), "a unit of it three quarters of a point");
        let wide = pictures.fenced("sketch", "2000 1000", FIT).and_then(|key| pictures.measured(&key, FIT)).expect("drawn");
        assert_eq!(pictures.shown(&wide), Some((468.0, 234.0)), "to the column");
        assert_ne!(pictures.fenced("sketch", "400 200\n", FIT), Some(key), "a block is a picture of its own, whatever its text");
        for language in ["svg", "", "held", "txt"] {
            assert_eq!(pictures.fenced(language, "400 200", FIT), None, "{language:?}");
        }
        assert_eq!((pictures.fences.get(), pictures.undrawn.get()), (3, 0), "none of those is a block to draw");
        assert_eq!(pictures.fenced("sketch", "flowchart LR", FIT), None, "its backend refuses it");
        assert_eq!(pictures.fenced("sketch", "0 200", FIT), None, "no size to show it at");
        assert_eq!((pictures.fences.get(), pictures.undrawn.get()), (5, 2));
        // As the document holds it, by a key of its own: drawn from the block's text, at three pixels
        // a point.
        let media = BTreeMap::from([("rId1".to_owned(), stand_in.clone()), ("rId2".to_owned(), Arc::new(b"its own".to_vec()))]);
        let promised = pictures.promised_in(&media);
        assert_eq!((promised.undrawn(), promised.fenced(), promised.left_out(), promised.is_empty()), (2, 3, 0, false));
        let drawn = promised.draw(&["rId1", "rId2"], false, None);
        let [(key, Ok(png))] = drawn.as_slice() else { panic!("{} drawn", drawn.len()) };
        assert_eq!((*key, size_of(png)), ("rId1", (900, 450)));
    }

    /// The key a block's picture is known by holds what is made afresh for each reading of a document,
    /// so nothing a document writes in an `<img src>` of its own is one: not the key it would have
    /// were nothing made afresh, nor one of another reading. And a key is never a path.
    #[test]
    fn a_fenced_blocks_key_is_nothing_a_document_can_write() {
        let pictures = Pictures::among(None, &DRAWING);
        let key = pictures.fenced("sketch", "400 200", FIT).expect("drawn");
        assert!(key.starts_with(FENCED) && key.ends_with(":0") && key.len() == FENCED.len() + 18, "{key}");
        assert!(pictures.measured(&key, FIT).is_some());
        for forged in
            ["fenced:0000000000000000:0", "fenced:0", "fenced::0", "fenced:", "0", &key[..key.len() - 1], &format!("{key}0"), &key.to_uppercase()]
        {
            assert!(pictures.measured(forged, FIT).is_none(), "{forged}");
        }
        let again = Pictures::among(None, &DRAWING);
        let other = again.fenced("sketch", "400 200", FIT).expect("drawn");
        assert_ne!(other, key, "afresh each reading");
        assert!(pictures.measured(&other, FIT).is_none() && again.measured(&key, FIT).is_none());
        assert_eq!(local(&key), None, "never a path");
        // With a directory to look in, a forged key is still no file's name.
        let dir = std::env::temp_dir();
        let beside = Pictures::among(Some(&dir), &DRAWING);
        assert!(beside.measured("fenced:0000000000000000:0", FIT).is_none());
        assert_eq!((beside.left_out.get(), beside.seen.borrow().len()), (0, 0), "not looked for");
    }

    /// The colour of `png`'s first pixel: its red, green and blue, with or without an alpha after them.
    fn colour_of(png: &[u8]) -> [u8; 3] {
        let mut reader = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels).unwrap();
        [pixels[0], pixels[1], pixels[2]]
    }

    /// A fenced block follows its page's paper: drawn for light paper and for dark each when a page
    /// on it wants it, its backend told which, one taking the other's place in what the document
    /// holds, and neither drawn twice over. Whether a page finds its pictures as it wants them is
    /// told for its own paper.
    #[test]
    fn a_fenced_block_is_drawn_again_for_the_other_paper() {
        const RED: [u8; 3] = [255, 0, 0];
        const BLUE: [u8; 3] = [0, 0, 255];
        let paper = Some([30, 30, 30]);
        let pictures = Pictures::among(None, &DRAWING);
        let stand_in = pictures.fenced("sketch", "40 20", FIT).and_then(|key| pictures.measured(&key, FIT)).expect("drawn");
        let promised = pictures.promised_in(&BTreeMap::from([("rId1".to_owned(), stand_in)]));
        let page = ["rId1", "not promised"];
        assert!(!promised.settled(&page, false, false) && !promised.settled(&page, false, true), "not drawn yet");
        // What a page does, on light paper or dark: the colour its block is then held in, if drawn.
        let shown = |dark: Option<[u8; 3]>| {
            let changes = promised.hold(promised.draw(&page, false, dark), &page, false, dark.is_some());
            assert!(changes.len() <= 1 && promised.settled(&page, false, dark.is_some()) && !promised.settled(&page, false, dark.is_none()));
            assert!(!promised.failed(&page, dark.is_some()));
            changes.first().map(|(key, png)| (key.clone(), colour_of(png)))
        };
        assert_eq!(shown(None), Some(("rId1".to_owned(), RED)), "for light paper");
        assert_eq!(shown(None), None, "once");
        assert_eq!(shown(paper), Some(("rId1".to_owned(), BLUE)), "again for dark paper, its backend told");
        assert_eq!(shown(paper), None, "once");
        assert_eq!(promised.held.lock().unwrap().drawn.len(), 1, "one held, the other let go of");
        assert_eq!(shown(None), Some(("rId1".to_owned(), RED)), "and back");
        let held = promised.held.lock().unwrap();
        assert_eq!((held.drawn.len(), held.for_dark.len(), held.bytes), (1, 0, held.drawn[0].1));
    }

    /// A block that can't be drawn for one paper shows its stand-in there, never the picture drawn
    /// for the other paper: let go of when the failure comes, and again each time a page on the paper
    /// it failed for finds the other's held. On the other paper it is drawn as ever. The failure is
    /// kept: not tried again.
    #[test]
    fn a_fenced_block_that_fails_for_one_paper_is_its_stand_in_there() {
        let paper = Some([30, 30, 30]);
        let pictures = Pictures::among(None, &DRAWING);
        let stand_in = pictures.fenced("sketch", "40 20 day", FIT).and_then(|key| pictures.measured(&key, FIT)).expect("sized");
        let promised = pictures.promised_in(&BTreeMap::from([("rId1".to_owned(), stand_in.clone())]));
        let page = ["rId1"];
        // What a page does, on light paper or dark: how many it tried to draw, and what the document
        // should then hold of the block, if that changes.
        let shown = |dark: Option<[u8; 3]>| {
            let drawn = promised.draw(&page, false, dark);
            let tried = drawn.len();
            let changes = promised.hold(drawn, &page, false, dark.is_some());
            assert!(changes.len() <= 1 && promised.settled(&page, false, dark.is_some()), "settled for its own paper");
            assert_eq!(promised.failed(&page, dark.is_some()), dark.is_some(), "failed for dark paper alone");
            (tried, changes.into_iter().next().map(|(_, held)| held))
        };
        let (tried, light) = shown(None);
        assert_eq!((tried, light.as_deref().map(|png| colour_of(png))), (1, Some([255, 0, 0])), "drawn for light paper");
        assert!(!promised.settled(&page, false, true), "nothing for a page on dark paper yet");
        let (tried, dark) = shown(paper);
        assert!(tried == 1 && dark.is_some_and(|held| Arc::ptr_eq(&held, &stand_in)), "its stand-in, the light picture let go of");
        let held = || {
            let held = promised.held.lock().unwrap();
            (held.drawn.len(), held.bytes)
        };
        assert_eq!(held(), (0, 0));
        assert_eq!(shown(paper), (0, None), "not tried again, and the stand-in stays");
        let (tried, light) = shown(None);
        assert_eq!((tried, light.as_deref().map(|png| colour_of(png))), (1, Some([255, 0, 0])), "drawn again for light paper");
        assert!(!promised.settled(&page, false, true), "held for light: nothing a page on dark paper may show");
        let (tried, dark) = shown(paper);
        assert!(tried == 0 && dark.is_some_and(|held| Arc::ptr_eq(&held, &stand_in)), "back to its stand-in, with nothing drawn");
        assert_eq!(shown(paper), (0, None));
    }

    /// A picture by path is one picture, whatever paper its page is on: drawn once, its backend never
    /// told of dark paper, and held for a page on either.
    #[cfg(feature = "image")]
    #[test]
    fn a_picture_by_path_is_the_same_on_either_paper() {
        let promised = promised(&["a"], &picture(), MAX_HELD);
        let paper = Some([30, 30, 30]);
        let drawn = promised.draw(&["a"], false, paper);
        assert_eq!(drawn.len(), 1);
        assert_eq!(promised.hold(drawn, &["a"], false, true).len(), 1);
        assert!(promised.settled(&["a"], false, true) && promised.settled(&["a"], false, false));
        assert!(promised.draw(&["a"], false, None).is_empty() && promised.draw(&["a"], false, paper).is_empty(), "drawn once");
        assert!(promised.held.lock().unwrap().for_dark.is_empty());
    }

    /// With its blocks to be shown as code, a document draws none, whatever their language, and counts
    /// none as left undrawn: there is nothing to say of them.
    #[test]
    fn fenced_blocks_shown_as_code_are_none_of_the_pictures() {
        let pictures = Pictures::among(None, &DRAWING).blocks_as_code();
        assert!(pictures.lists("sketch") && !pictures.lists("svg"), "listed all the same");
        assert_eq!(pictures.fenced("sketch", "400 200", FIT), None);
        assert_eq!((pictures.fences.get(), pictures.undrawn.get()), (0, 0));
        // Each has a marker of its own instead, which only this reading's pictures know.
        let (first, second) = (pictures.coded(), pictures.coded());
        assert!(first.starts_with(FENCED) && first != second && local(&first).is_none(), "{first} {second}");
        let other = Pictures::among(None, &DRAWING).coded();
        let promised = pictures.promised_in(&BTreeMap::new());
        assert_eq!((promised.fenced(), promised.undrawn()), (0, 0));
        assert_eq!((promised.coded(&first), promised.coded(&second), promised.coded(&other)), (Some(0), Some(1), None));
        for forged in ["fenced:0000000000000000:code:0", "fenced:code:0", "0", "", &format!("{first}x"), &first[..first.len() - 1]] {
            assert_eq!(promised.coded(forged), None, "{forged}");
        }
    }

    /// Fenced blocks count among a document's pictures: past the most it has, a block isn't opened,
    /// stays code, and is counted as not drawn; as is a picture by path after as many blocks.
    #[test]
    fn fenced_blocks_count_among_the_pictures_a_document_has() {
        let dir = std::env::temp_dir();
        let pictures = Pictures::among(Some(&dir), &DRAWING);
        for n in 0..MAX_PICTURES - 1 {
            assert!(pictures.fenced("sketch", "40 20", FIT).is_some(), "{n}");
        }
        assert!(pictures.measured("clview-no-such-picture.png", FIT).is_none());
        assert_eq!((pictures.asked(), pictures.left_out.get()), (MAX_PICTURES, 1), "a picture by path counts too");
        assert_eq!(pictures.fenced("sketch", "40 20", FIT), None, "one too many");
        assert_eq!(pictures.fenced("sketch", "not a sketch", FIT), None);
        assert_eq!((pictures.undrawn.get(), pictures.promises.borrow().len()), (2, MAX_PICTURES - 1));
        assert_eq!(pictures.fenced("svg", "40 20", FIT), None);
        assert_eq!(pictures.undrawn.get(), 2, "a language none lists is no block left undrawn");
    }
}
