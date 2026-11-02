//! Pictures a document refers to by a path, drawn by whichever backend shows them: what this build
//! shows as a picture, a drawing or a PDF's page, a document shows too, turned as it would show it.
//! Only what is on this machine, near the document: nothing is ever fetched
//! ([ADR-0013](../../docs/adr/0013-a-documents-pictures-are-drawn-by-the-backends.md)).
//!
//! A document is laid out around a stand-in for each picture ([`Pictures::measured`]), a PNG saying
//! only the size to show it at, so that opening one reads no picture's pixels and a list of documents
//! holds none. A picture is drawn when a page it is on first renders ([`Promised::draw`]), into the
//! place its stand-in was given, at a resolution of its own: a small drawing stays sharp.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use super::{BACKENDS, Backend, Input, Opened, Request, Source, guarded, shown_by};
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

/// The most pictures a document may refer to. Each is opened to learn its size.
const MAX_PICTURES: usize = 1024;

/// The most a document holds of its pictures, as PNGs, before it lets go of those drawn longest ago:
/// 256 MiB, some 50 photographs at [`MAX_SIDE`].
const MAX_HELD: usize = 256 << 20;

/// What a document holds of a picture: its stand-in, or the picture, as a PNG.
type Png = Arc<Vec<u8>>;

/// What `read` returns, or an error if a backend panics: its bug costs the picture, not the document.
fn caught<T>(read: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|_| Err("it failed on this picture".into()))
}

/// The pictures of a document being read, each on its way to a [`Promise`].
pub(super) struct Pictures<'a> {
    /// The directory the document is in, what its pictures' paths start from; none for one come
    /// through a pipe or held in an archive, whose pictures can't be reached.
    beside: Option<&'a Path>,
    /// By the name a document refers to it by: its stand-in, or none if it isn't shown.
    seen: RefCell<HashMap<String, Option<Png>>>,
    promises: RefCell<HashMap<Png, Promise>>,
    /// How many it refers to on this machine aren't shown.
    left_out: Cell<usize>,
}

impl<'a> Pictures<'a> {
    pub(super) fn beside(beside: Option<&'a Path>) -> Self {
        Self { beside, seen: RefCell::default(), promises: RefCell::default(), left_out: Cell::new(0) }
    }

    /// A stand-in for the picture `reference` names, to fit in `fit` (width and height, in points),
    /// if it is one this build shows, on this machine. None otherwise, and the document shows what it
    /// says instead of it.
    pub(super) fn measured(&self, reference: &str, fit: (f32, f32)) -> Option<Png> {
        let beside = self.beside?;
        let name = local(reference)?;
        if let Some(known) = self.seen.borrow().get(&name) {
            return known.clone();
        }
        let stand_in = (self.seen.borrow().len() < MAX_PICTURES).then(|| self.promised(&beside.join(&name), fit)).flatten();
        if stand_in.is_none() {
            self.left_out.set(self.left_out.get().saturating_add(1));
        }
        self.seen.borrow_mut().insert(name, stand_in.clone());
        stand_in
    }

    /// The stand-in for the picture at `path`, and the promise to draw it.
    fn promised(&self, path: &Path, fit: (f32, f32)) -> Option<Png> {
        let path = path.to_str()?;
        let (size, natural) = caught(|| {
            let source = opened(path)?;
            let size = source.size(0).ok_or("it has no first page")?;
            Ok((size, source.natural(0)))
        })
        .ok()?;
        let shown = shown(size, fit)?;
        let mut promises = self.promises.borrow_mut();
        let stand_in = Arc::new(stand_in(shown, promises.len()).ok()?);
        promises.insert(stand_in.clone(), Promise { path: path.to_owned(), scale: scale(size, natural, shown), stand_in: stand_in.clone() });
        Some(stand_in)
    }

    /// These pictures as a document that was read holds them: `media`, by key, what it holds of
    /// each, the stand-ins among them being the ones promised.
    pub(super) fn promised_in(self, media: &BTreeMap<String, Png>) -> Promised {
        let promises = self.promises.into_inner();
        let by_key = media.iter().filter_map(|(key, held)| Some((key.clone(), promises.get(held)?.clone()))).collect();
        Promised { by_key, left_out: self.left_out.get(), max_held: MAX_HELD, held: Mutex::default() }
    }
}

/// A picture to draw when it is wanted.
#[derive(Clone)]
struct Promise {
    path: String,
    /// What to draw it at.
    scale: f32,
    /// What a document held until now, and holds again once it lets go of the picture.
    stand_in: Png,
}

impl Promise {
    /// The picture, as a PNG.
    fn draw(&self) -> Result<Vec<u8>, String> {
        caught(|| {
            let source = opened(&self.path)?;
            let mut renderer = source.renderer()?;
            let request = Request { page: 0, scale: self.scale, tag: 0 };
            let frame = guarded(request, || renderer.render(request));
            match frame.error {
                Some(error) => Err(error),
                None => encode_png(frame.width, frame.height, &frame.rgba, PngEffort::Fast),
            }
        })
    }
}

/// A document's pictures, drawn as its pages want them.
pub(super) struct Promised {
    by_key: HashMap<String, Promise>,
    left_out: usize,
    max_held: usize,
    held: Mutex<Held>,
}

/// What a document holds of its pictures now.
#[derive(Default)]
struct Held {
    /// Each drawn, by its key, and its size in bytes, the one drawn longest ago first.
    drawn: VecDeque<(String, usize)>,
    bytes: usize,
    /// Those that could not be drawn, not tried again.
    failed: HashSet<String>,
}

impl Promised {
    /// How many pictures the document refers to on this machine are left out, missing or not what
    /// this build shows.
    pub(super) fn left_out(&self) -> usize {
        self.left_out
    }

    /// Whether any of the document's pictures is promised.
    pub(super) fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    /// What the document should hold now that a page showing `on_page` (keys of what it holds) is
    /// wanted, by key, and whether one of those could not be drawn. Each is drawn only once, unless
    /// it was let go of. Nothing is locked while they are drawn: pages already drawn go on rendering.
    pub(super) fn draw(&self, on_page: &[&str]) -> (Vec<(String, Png)>, bool) {
        let wanted: Vec<(&str, &Promise)> = {
            let held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
            let done = |key: &str| held.failed.contains(key) || held.drawn.iter().any(|(drawn, _)| drawn == key);
            on_page.iter().filter(|key| !done(key)).filter_map(|key| Some((*key, self.by_key.get(*key)?))).collect()
        };
        let drawn: Vec<_> = wanted.into_iter().map(|(key, promise)| (key, promise.draw())).collect();

        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let mut changes = Vec::new();
        for (key, picture) in drawn {
            match picture {
                // Another thread drew it meanwhile, for a page of its own.
                Ok(_) if held.drawn.iter().any(|(drawn, _)| drawn == key) => {}
                Ok(picture) => {
                    held.bytes = held.bytes.saturating_add(picture.len());
                    held.drawn.push_back((key.to_owned(), picture.len()));
                    changes.push((key.to_owned(), Arc::new(picture)));
                }
                Err(_) => {
                    held.failed.insert(key.to_owned());
                }
            }
        }
        while held.bytes > self.max_held {
            let Some(oldest) = held.drawn.iter().position(|(key, _)| !on_page.contains(&key.as_str())) else { break };
            let Some((key, size)) = held.drawn.remove(oldest) else { break };
            held.bytes = held.bytes.saturating_sub(size);
            if let Some(promise) = self.by_key.get(&key) {
                changes.push((key, promise.stand_in.clone()));
            }
        }
        let failed = on_page.iter().any(|key| held.failed.contains(*key));
        (changes, failed)
    }
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

    /// Promises with a picture to draw, each a file of `png`, and what they hold at most.
    #[cfg(feature = "image")]
    fn promised(keys: &[&str], png: &[u8], max_held: usize) -> Promised {
        let path = std::env::temp_dir().join(format!("clview-picture-{}-{}.png", std::process::id(), keys.join("-")));
        std::fs::write(&path, png).unwrap();
        let promise = |key: &&str| {
            let stand_in = Arc::new(key.as_bytes().to_vec());
            (key.to_string(), Promise { path: path.to_str().unwrap().to_owned(), scale: 1.0, stand_in })
        };
        Promised { by_key: keys.iter().map(promise).collect(), left_out: 0, max_held, held: Mutex::default() }
    }

    #[cfg(feature = "image")]
    fn picture() -> Vec<u8> {
        encode_png(2, 2, &[255; 16], PngEffort::Fast).unwrap()
    }

    #[cfg(feature = "image")]
    #[test]
    fn a_picture_is_drawn_once_and_only_those_a_page_shows() {
        let promised = promised(&["a", "b"], &picture(), MAX_HELD);
        let (changes, failed) = promised.draw(&["a", "not promised"]);
        assert!(!failed);
        assert_eq!(changes.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(size_of(&changes[0].1), (2, 2), "the picture itself");
        assert!(promised.draw(&["a"]).0.is_empty(), "already drawn");
        assert_eq!(promised.draw(&["b"]).0.len(), 1);
    }

    /// Past what it holds, it lets go of what was drawn longest ago, back to its stand-in, but not of
    /// what the page wanted shows; let go of, it is drawn again when it is wanted.
    #[cfg(feature = "image")]
    #[test]
    fn past_the_most_it_holds_a_document_lets_go_of_what_it_drew_longest_ago() {
        let png = picture();
        let promised = promised(&["a", "b", "c"], &png, png.len() * 2);
        assert_eq!(promised.draw(&["a"]).0.len(), 1);
        assert_eq!(promised.draw(&["b"]).0.len(), 1);
        let (changes, _) = promised.draw(&["c"]);
        let what = |key: &str| changes.iter().find(|(k, _)| k == key).map(|(_, held)| held.clone());
        assert_eq!(what("c").map(|held| held.len()), Some(png.len()), "drawn");
        assert_eq!(what("a").unwrap().as_slice(), b"a", "its stand-in again");
        assert!(what("b").is_none(), "within what it holds");
        assert_eq!(promised.draw(&["a"]).0.iter().filter(|(key, _)| key == "a").count(), 1, "drawn again");

        let everything = promised.draw(&["a", "b", "c"]);
        assert!(everything.0.iter().all(|(_, held)| held.len() == png.len()), "a page's own are never let go of");
    }

    #[cfg(feature = "image")]
    #[test]
    fn a_picture_that_cant_be_drawn_says_so_and_isnt_tried_again() {
        // A PNG's header, enough to open, and then nothing to decode.
        let mut broken = picture();
        broken.truncate(40);
        let promised = promised(&["broken"], &broken, MAX_HELD);
        assert_eq!(promised.draw(&["broken"]), (Vec::new(), true));
        assert_eq!(promised.draw(&["other"]), (Vec::new(), false), "it says so for a page it is on");
        assert_eq!(promised.draw(&["broken"]), (Vec::new(), true));
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
}
