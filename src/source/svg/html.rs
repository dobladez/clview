//! An SVG's HTML labels, laid out and painted by Blitz (Stylo's styles, Taffy's layout, Parley's
//! text) through anyrender into vello_cpu: the one file that names Blitz and what it paints with
//! (ADR-0024). `svg.rs` finds the labels, places them and writes them out; this lays them out in
//! one document, records each label's painting once, and replays a label into the part of a page
//! it lands on, at whatever scale that page is drawn.
//!
//! All of that runs in a worker process of its own, a drawing's ([`LABELS`]): Blitz's costs
//! can't be bounded before they are spent, so the worker's CPU time and heap are, as they are
//! spent, and past them it is killed and the drawing's labels left out (ADR-0025). The viewer's
//! side ([`Painted`]) asks it to lay the labels out ([`PREPARE`]), and for each part of a page a
//! label lands on, its pixels ([`RENDER`]). What a label fetches is still bounded before Blitz
//! has it: `data:` URLs only, each font and picture sized from its header.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyrender::recording::RenderCommand;
use anyrender::{PaintScene as _, Scene, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use base64::Engine as _;
use blitz_dom::{BaseDocument, DocumentConfig, FontContext, NodeId};
use blitz_html::HtmlDocument;
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use kurbo::{Affine, Rect};

use super::super::worker::{self, Died, FAILED, Link, Session, WALL, Worker};
use super::Label;

/// Who lays the labels out, credited with the backend's own.
pub(super) const CREDIT: &str = "Blitz";

/// The largest side of a label laid out, in CSS pixels: past it, it is left out (ADR-0024).
pub(super) const MAX_LAYOUT_SIDE: f32 = 16_384.0;

/// How far past its box an `overflow: visible` label is drawn, each way, in CSS pixels.
pub(super) const OVERFLOW: f32 = 4_096.0;

/// The most a `data:` font may unpack to, as its header says, and all of a drawing's.
const MAX_FONT: u64 = 32 << 20;
const MAX_FONTS: u64 = 64 << 20;

/// The most a `data:` picture may decode to, as its header says, and all of a drawing's.
const MAX_PICTURE: u64 = 64 << 20;
const MAX_PICTURES: u64 = 256 << 20;

/// The most `data:` URLs answered for a drawing.
const MAX_REQUESTS: usize = 1_024;

/// Between the labels in their document, in CSS pixels, so that painting one culls its neighbours.
const GAP: f32 = 8.0;

/// The widest row of the labels' document, in CSS pixels.
const ROW: f32 = 1_048_576.0;

/// The most links a drawing's labels count: what isn't a `data:` URL, not fetched.
const MAX_LINKS: usize = 1_024;

// The worker's requests and answers.

/// The labels to lay out and record: the drawing's stylesheets, and each label's size, overflow and
/// markup. Answered by [`PREPARED`].
const PREPARE: u8 = 1;

/// A label's part of a page: the label, the transform from its box's CSS pixels to the part, and
/// the part's size. Answered by [`PIXELS`] or [`NOTHING`].
const RENDER: u8 = 2;

/// Each label's region, or none for one left out, and the links the labels asked for (what isn't
/// a `data:` URL, not fetched): each at most 256 bytes, at most [`MAX_LINKS`] of them, to be
/// counted with the drawing's own, never shown.
const PREPARED: u8 = 1;

/// A part's pixels: premultiplied RGBA, row by row, exactly its size.
const PIXELS: u8 = 2;

/// No pixels: the label was left out.
const NOTHING: u8 = 3;

/// Why the worker answered [`FAILED`]: a request it couldn't read.
const MALFORMED: u8 = 2;

/// The CPU seconds laying a drawing's labels out and recording them may take, and a render:
/// five times the slowest measured within the limits before them, and about what a page's own
/// render is held to (ADR-0025).
const PREPARE_CPU: u64 = 10;
const RENDER_CPU: u64 = 5;

/// The longest [`PREPARE`]: the drawing's stylesheets, at most its file expanded, and the labels'
/// markup, at most 8 MiB.
const MAX_PREPARE: usize = 256 << 20;

/// The longest [`RENDER`].
const MAX_RENDER: usize = 64;

/// The longest [`PREPARED`]: 1,024 labels' regions, and 1,024 links of 256 bytes.
const MAX_PREPARED: usize = 1 << 20;

/// The longest link a worker tells of, in bytes: hashed and counted, never shown.
const LINK_BYTES: usize = 256;

/// The worker that lays a drawing's labels out and paints them: `clv --worker svg-html`.
pub(crate) struct Labels;

pub(crate) static LABELS: Labels = Labels;

impl Worker for Labels {
    fn name(&self) -> &'static str {
        "svg-html"
    }

    fn takes(&self, kind: u8) -> Option<(u64, usize)> {
        match kind {
            PREPARE => Some((PREPARE_CPU, MAX_PREPARE)),
            RENDER => Some((RENDER_CPU, MAX_RENDER)),
            _ => None,
        }
    }

    fn session(&self) -> Box<dyn Session> {
        Box::new(Kept::default())
    }
}

/// A label as the worker is given it: its box's size in CSS pixels, whether what it holds is drawn
/// past it, and its markup.
struct Given {
    width: f32,
    height: f32,
    visible: bool,
    markup: String,
}

/// What the worker keeps between requests: each label's recording, once laid out.
#[derive(Default)]
struct Kept {
    labels: Vec<Option<Recorded>>,
}

/// One label's painting, recorded at one CSS pixel to the pixel, from its box's top left.
struct Recorded {
    scene: Scene,
    /// What it paints over, in those pixels: its box, and what it draws past it (overflow), at
    /// most [`OVERFLOW`] each way.
    region: Rect,
}

impl Session for Kept {
    fn answer(&mut self, kind: u8, payload: Vec<u8>) -> (u8, Vec<u8>) {
        match kind {
            PREPARE => {
                let Some((css, labels)) = given(&payload) else { return (FAILED, vec![MALFORMED]) };
                drop(payload);
                let (recorded, links) = recorded(&css, &labels);
                let mut out = Vec::new();
                out.extend_from_slice(&u32::try_from(recorded.len()).unwrap_or(u32::MAX).to_le_bytes());
                for label in &recorded {
                    let region = label.as_ref().map(|l| l.region);
                    out.push(u8::from(region.is_some()));
                    for v in region.map_or([0.0; 4], |r| [r.x0, r.y0, r.x1, r.y1]) {
                        out.extend_from_slice(&v.to_le_bytes());
                    }
                }
                out.extend_from_slice(&u32::try_from(links.len()).unwrap_or(u32::MAX).to_le_bytes());
                for link in &links {
                    if put_string(&mut out, link).is_err() {
                        return (FAILED, vec![MALFORMED]);
                    }
                }
                self.labels = recorded;
                (PREPARED, out)
            }
            RENDER => {
                let mut read = Reading(&payload);
                let (Some(index), Some(to), Some(w), Some(h)) = (read.u32(), read.f64s::<6>(), read.u32(), read.u32()) else {
                    return (FAILED, vec![MALFORMED]);
                };
                let label = usize::try_from(index).ok().and_then(|i| self.labels.get(i)).and_then(Option::as_ref);
                match label.and_then(|label| painted(label, to, (w, h))) {
                    Some(rgba) => (PIXELS, rgba),
                    None => (NOTHING, Vec::new()),
                }
            }
            _ => (FAILED, vec![MALFORMED]),
        }
    }
}

/// What a request's payload says, read from its start: each read `None` past its end.
struct Reading<'a>(&'a [u8]);

impl<'a> Reading<'a> {
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let (taken, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(taken)
    }

    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.bytes(N).and_then(|b| <[u8; N]>::try_from(b).ok())
    }

    fn u32(&mut self) -> Option<u32> {
        self.array().map(u32::from_le_bytes)
    }

    fn f32(&mut self) -> Option<f32> {
        self.array().map(f32::from_le_bytes)
    }

    fn f64s<const N: usize>(&mut self) -> Option<[f64; N]> {
        let mut out = [0.0; N];
        for v in &mut out {
            *v = self.array().map(f64::from_le_bytes)?;
        }
        Some(out)
    }

    /// A string: its length (four bytes), then its UTF-8.
    fn string(&mut self) -> Option<String> {
        let n = usize::try_from(self.u32()?).ok()?;
        String::from_utf8(self.bytes(n)?.to_vec()).ok()
    }
}

/// A string written as [`Reading::string`] reads it.
fn put_string(out: &mut Vec<u8>, s: &str) -> Result<(), String> {
    let n = u32::try_from(s.len()).map_err(|_| "their stylesheets are too long".to_string())?;
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

/// A [`PREPARE`]'s payload, read: the stylesheets, and the labels. `None` for one that isn't one.
fn given(payload: &[u8]) -> Option<(String, Vec<Given>)> {
    let mut read = Reading(payload);
    let css = read.string()?;
    let n = read.u32()?;
    let mut labels = Vec::new();
    for _ in 0..n {
        let (width, height) = (read.f32()?, read.f32()?);
        let visible = read.array::<1>()? != [0];
        let markup = read.string()?;
        if !(width.is_finite() && height.is_finite() && width >= 0.0 && height >= 0.0) {
            return None;
        }
        labels.push(Given { width, height, visible, markup });
    }
    read.0.is_empty().then_some((css, labels))
}

// The viewer's side.

/// A drawing's labels, laid out and recorded in its worker, which replays them for each page.
pub(super) struct Painted {
    link: Mutex<Link>,
    /// What each label paints over, in its box's CSS pixels: `None` for one left out (laid out
    /// past [`MAX_LAYOUT_SIDE`]).
    regions: Vec<Option<[f64; 4]>>,
    /// The links the labels asked for that aren't `data:` URLs, not followed: to count with the
    /// drawing's own, never to show.
    pub(super) links: Vec<String>,
}

/// How long the viewer waits for an answer of `cpu` seconds' work.
fn wall(cpu: u64) -> Duration {
    Duration::from_secs(cpu.saturating_mul(u64::from(WALL)))
}

/// Why the labels are left out, in the viewer's words, as the worker went: `died`.
fn said(died: &Died) -> String {
    match died {
        Died::TooMany => "too many drawings' labels at once".into(),
        Died::Unstarted(why) => format!("no process to lay them out in ({why})"),
        Died::TimedOut | Died::Cpu => "Blitz took too long on them".into(),
        Died::Memory => "Blitz took too much memory on them".into(),
        Died::Version => "clv was replaced while it ran: open the drawing again".into(),
        Died::Ended(_) | Died::Malformed => "Blitz failed on them".into(),
    }
}

/// The `labels`, under the drawing's stylesheets `css`, laid out and recorded in a worker of
/// their own; or why they all are left out.
pub(super) fn prepare(css: &str, labels: &[Label]) -> Result<Painted, String> {
    let mut payload = Vec::new();
    put_string(&mut payload, css)?;
    payload.extend_from_slice(&u32::try_from(labels.len()).map_err(|_| "too many of them".to_string())?.to_le_bytes());
    for label in labels {
        payload.extend_from_slice(&label.width.to_le_bytes());
        payload.extend_from_slice(&label.height.to_le_bytes());
        payload.push(u8::from(label.visible));
        put_string(&mut payload, &label.markup)?;
    }
    let mut link = worker::spawn(&LABELS).map_err(|d| said(&d))?;
    let (kind, answer) = link.request(PREPARE, payload, wall(PREPARE_CPU), MAX_PREPARED).map_err(|d| said(&d))?;
    let mut read = Reading(&answer);
    if kind != PREPARED || read.u32().and_then(|n| usize::try_from(n).ok()) != Some(labels.len()) {
        return Err(said(&link.refuse()));
    }
    let mut regions = Vec::with_capacity(labels.len());
    for _ in labels {
        let (Some(present), Some(region)) = (read.array::<1>(), read.f64s::<4>()) else { return Err(said(&link.refuse())) };
        let [x0, y0, x1, y1] = region;
        // A region is its box, and what overflows it within OVERFLOW: a few tens of thousands of pixels.
        let within = region.iter().all(|v| v.is_finite() && v.abs() <= 1e6) && x0 <= x1 && y0 <= y1;
        match present {
            [0] => regions.push(None),
            [1] if within => regions.push(Some(region)),
            _ => return Err(said(&link.refuse())),
        }
    }
    let Some(count) = read.u32().and_then(|n| usize::try_from(n).ok()).filter(|n| *n <= MAX_LINKS) else { return Err(said(&link.refuse())) };
    let mut links = Vec::with_capacity(count);
    for _ in 0..count {
        match read.string() {
            Some(url) if url.len() <= LINK_BYTES => links.push(url),
            _ => return Err(said(&link.refuse())),
        }
    }
    if !read.0.is_empty() {
        return Err(said(&link.refuse()));
    }
    Ok(Painted { link: Mutex::new(link), regions, links })
}

impl Painted {
    /// What label `index` paints over, in its box's CSS pixels from its top left: `None` when it
    /// was left out.
    pub(super) fn region(&self, index: usize) -> Option<[f64; 4]> {
        self.regions.get(index).copied().flatten()
    }

    /// Label `index` replayed through `to` into `size` pixels: premultiplied RGBA, row by row;
    /// `None` when it was left out. An error when the worker is gone, now or before: why.
    pub(super) fn paint(&self, index: usize, to: [f64; 6], size: (u32, u32)) -> Result<Option<Vec<u8>>, String> {
        if self.region(index).is_none() || size.0 == 0 || size.1 == 0 || to.iter().any(|v| !v.is_finite()) {
            return Ok(None);
        }
        let bytes = u64::from(size.0).saturating_mul(u64::from(size.1)).saturating_mul(4);
        let Ok(bytes) = usize::try_from(bytes) else { return Ok(None) };
        let mut payload = Vec::with_capacity(MAX_RENDER);
        payload.extend_from_slice(&u32::try_from(index).unwrap_or(u32::MAX).to_le_bytes());
        for v in to {
            payload.extend_from_slice(&v.to_le_bytes());
        }
        payload.extend_from_slice(&size.0.to_le_bytes());
        payload.extend_from_slice(&size.1.to_le_bytes());
        let mut link = self.link.lock().unwrap_or_else(PoisonError::into_inner);
        match link.request(RENDER, payload, wall(RENDER_CPU), bytes) {
            Ok((PIXELS, rgba)) if rgba.len() == bytes => Ok(Some(rgba)),
            Ok((NOTHING, none)) if none.is_empty() => Ok(None),
            Ok(_) => Err(said(&link.refuse())),
            Err(died) => Err(said(&died)),
        }
    }
}

// The worker's side.

/// Where a label's slot is in the labels' document, its top left in CSS pixels.
type At = (f32, f32);

/// Where each label is in the labels' document: rows about as wide as all of them would be square,
/// so the document's coordinates stay small; and the document's size. A label larger than
/// [`MAX_LAYOUT_SIDE`] has no place.
fn packed(labels: &[Given]) -> (Vec<Option<At>>, (f32, f32)) {
    let slot = |l: &Given| {
        let margin = if l.visible { OVERFLOW } else { 0.0 };
        (l.width + 2.0 * margin, l.height + 2.0 * margin)
    };
    let fits = |l: &Given| l.width <= MAX_LAYOUT_SIDE && l.height <= MAX_LAYOUT_SIDE;
    let area: f64 = labels.iter().filter(|l| fits(l)).map(|l| f64::from(slot(l).0 + GAP) * f64::from(slot(l).1 + GAP)).sum();
    let widest = labels.iter().filter(|l| fits(l)).map(|l| slot(l).0 + GAP).fold(0.0f32, f32::max);
    // Within f32: at most 1,024 labels of at most 24,576 pixels a side.
    let row = (area.sqrt() as f32).clamp(widest, ROW.max(widest));
    let (mut x, mut y, mut row_height, mut width) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut at = Vec::with_capacity(labels.len());
    for label in labels {
        if !fits(label) {
            at.push(None);
            continue;
        }
        let (w, h) = slot(label);
        if x > 0.0 && x + w > row {
            (x, y, row_height) = (0.0, y + row_height, 0.0);
        }
        at.push(Some((x, y)));
        x += w.ceil() + GAP;
        width = width.max(x);
        row_height = row_height.max(h.ceil() + GAP);
    }
    (at, (width, y + row_height))
}

/// The labels' document: the drawing's stylesheets, then each label in a slot of its own at
/// `at`, placed by a transform (Blitz culls by a box that counts an element's position twice:
/// placed by `left` and `top`, a slot wouldn't cull its neighbours), its box inside the slot, the
/// containing block of what it holds.
fn document(css: &str, labels: &[Given], at: &[Option<At>]) -> String {
    let mut out = String::from("<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><style><![CDATA[");
    out.push_str(&css.replace("]]>", "]]]]><![CDATA[>"));
    out.push_str(
        "]]></style><style>html,body{margin:0 !important;padding:0 !important;background:transparent !important;\
         overflow:hidden !important}</style></head><body>",
    );
    for (label, at) in labels.iter().zip(at) {
        let Some((x, y)) = at else { continue };
        let margin = if label.visible { OVERFLOW } else { 0.0 };
        let (w, h) = (label.width, label.height);
        out.push_str(&format!(
            "<div style=\"position:absolute;left:0;top:0;transform:translate({x}px,{y}px);width:{}px;height:{}px;overflow:hidden;\
             margin:0;padding:0;border:0\"><div style=\"position:absolute;left:{margin}px;top:{margin}px;width:{w}px;height:{h}px;\
             margin:0;padding:0;border:0\">",
            w + 2.0 * margin,
            h + 2.0 * margin
        ));
        out.push_str(&label.markup);
        out.push_str("</div></div>");
    }
    out.push_str("</body></html>");
    out
}

/// The drawing's stylesheets `css` parsed by Stylo, as Blitz will parse them, and written out again
/// without their `@import`s: Blitz asks for a URL without saying what for, and no stylesheet is
/// ever fetched.
fn without_imports(css: &str) -> String {
    use style::context::QuirksMode;
    use style::media_queries::MediaList;
    use style::servo_arc::Arc as ServoArc;
    use style::shared_lock::{SharedRwLock, ToCssWithGuard};
    use style::stylesheets::{AllowImportRules, CssRule, Origin, Stylesheet};
    let mut out = String::new();
    let Ok(url) = blitz_traits::net::Url::parse("file:///") else { return out };
    let lock = SharedRwLock::new();
    let media = ServoArc::new(lock.wrap(MediaList::empty()));
    let sheet = Stylesheet::from_str(css, url.into(), Origin::Author, media, lock.clone(), None, None, QuirksMode::NoQuirks, AllowImportRules::No);
    let guard = lock.read();
    let contents = sheet.contents.read_with(&guard);
    for rule in contents.rules.read_with(&guard).0.iter() {
        if matches!(rule, CssRule::Import(_)) {
            continue;
        }
        let _ = rule.to_css(&guard, &mut out);
        out.push('\n');
    }
    out
}

/// The `labels` laid out in one document and each recorded (`None` for one left out), and the
/// links they asked for.
fn recorded(css: &str, labels: &[Given]) -> (Vec<Option<Recorded>>, Vec<String>) {
    let (at, size) = packed(labels);
    let markup = document(&without_imports(css), labels, &at);
    let net = Arc::new(DataOnly::default());
    let config = DocumentConfig {
        // Within u32: the labels' document is at most some 800,000 pixels a side.
        viewport: Some(Viewport::new(size.0.ceil().max(1.0) as u32, size.1.ceil().max(1.0) as u32, 1.0, ColorScheme::Light)),
        net_provider: Some(net.clone()),
        font_ctx: Some(fonts()),
        // A base a relative link resolves against (Blitz's own, a `data:` URL, can't be one, and
        // it panics), to a file not read: told of, as any other link.
        base_url: Some("file:///".into()),
        ..DocumentConfig::default()
    };
    let mut doc = HtmlDocument::from_xml(&markup, config);
    drop(markup);
    doc.handle_messages();
    // A picture a style asks for (`background-image`) is fetched as the styles resolve, and
    // answered after: they resolve again with it.
    doc.resolve(0.0);
    if anonymous_cells(&mut doc) {
        doc.handle_messages();
        doc.resolve(0.0);
    }
    let boxes = boxes(&doc);
    let mut recorded = Vec::with_capacity(labels.len());
    let mut slots = boxes.into_iter();
    for (label, at) in labels.iter().zip(&at) {
        // A label with no place has no slot: the next slot is the next placed label's.
        let (Some((x, y)), Some(region)) = (at, at.and_then(|_| slots.next())) else {
            recorded.push(None);
            continue;
        };
        let margin = f64::from(if label.visible { OVERFLOW } else { 0.0 });
        let (sw, sh) = (f64::from(label.width) + 2.0 * margin, f64::from(label.height) + 2.0 * margin);
        doc.set_viewport_scroll(blitz_dom::Point { x: f64::from(*x), y: f64::from(*y) });
        let mut scene = Scene::new();
        // Within u32: a slot is at most 24,576 pixels a side.
        blitz_paint::paint_scene(&mut scene, &mut doc, 1.0, sw.ceil() as u32, sh.ceil() as u32, 0, 0);
        // From the slot's top left to the box's.
        let region = (region + kurbo::Vec2::new(-margin, -margin)).intersect(Rect::new(-margin, -margin, sw - margin, sh - margin));
        let scene = Scene { tolerance: scene.tolerance, commands: scene.commands.into_iter().map(|c| moved(c, -margin)).collect() };
        recorded.push(Some(Recorded { scene, region }));
    }
    drop(doc);
    let links = std::mem::take(&mut *net.links.lock().unwrap_or_else(PoisonError::into_inner));
    (recorded, links)
}

/// The worker's fonts: the system's as the drawing's own text has them, the files `svg.rs` found
/// for it, each mapped by Blitz's own font cache (never read into memory) and registered into its
/// collection, and the generic families set as `svg.rs` sets them for the drawing's text; nothing
/// asked of fontconfig or CoreText, so a static build has them too (ADR-0025's update). A file
/// fontique can't read is skipped.
fn fonts() -> FontContext {
    use parley::fontique::{Blob, Collection, CollectionOptions, GenericFamily, SourceCache, SourceId, SourceInfo, SourceKind};
    let found = super::label_fonts();
    let mut collection = Collection::new(CollectionOptions { shared: false, system_fonts: false });
    let mut source_cache = SourceCache::default();
    for file in found.files {
        let blob = match file {
            super::FontFile::Path(path) => source_cache.get(&SourceInfo::new(SourceId::new(), SourceKind::Path(Arc::from(path)))),
            super::FontFile::Bytes(bytes) => Some(Blob::new(bytes)),
        };
        if let Some(blob) = blob {
            collection.register_fonts(blob, None);
        }
    }
    for (generic, name) in found.generics {
        let Some(id) = collection.family_id(&name) else { continue };
        let generics: &[GenericFamily] = match generic {
            super::Generic::Serif => &[GenericFamily::Serif, GenericFamily::UiSerif],
            super::Generic::SansSerif => &[GenericFamily::SansSerif, GenericFamily::SystemUi, GenericFamily::UiSansSerif, GenericFamily::UiRounded],
            super::Generic::Monospace => &[GenericFamily::Monospace, GenericFamily::UiMonospace],
            super::Generic::Cursive => &[GenericFamily::Cursive],
            super::Generic::Fantasy => &[GenericFamily::Fantasy],
        };
        for generic in generics {
            collection.set_generic_families(*generic, std::iter::once(id));
        }
    }
    for (tag, names) in FALLBACKS {
        let ids: Vec<_> = names.iter().filter_map(|name| collection.family_id(name)).collect();
        collection.set_fallbacks(parley::fontique::Script::from_bytes(**tag), ids.into_iter());
    }
    let emoji: Vec<_> = EMOJI.iter().filter_map(|name| collection.family_id(name)).collect();
    collection.set_generic_families(GenericFamily::Emoji, emoji.into_iter());
    FontContext { collection, source_cache }
}

/// The families a script's text falls back to where the family a label names has no glyph for
/// it, which the system's own matching (fontconfig, CoreText) would supply and the collection
/// built from files can't: Apple's, Google's Noto, then the usual free ones, those the system has,
/// in that order. A script not here falls back to nothing, as the drawing's own text wouldn't.
const FALLBACKS: &[(&[u8; 4], &[&str])] = &[
    (b"Latn", &["Helvetica", "Arial", "DejaVu Sans", "Liberation Sans", "Noto Sans"]),
    (b"Grek", &["Helvetica", "Arial", "DejaVu Sans", "Liberation Sans", "Noto Sans"]),
    (b"Cyrl", &["Helvetica", "Arial", "DejaVu Sans", "Liberation Sans", "Noto Sans"]),
    (b"Zyyy", &["Helvetica", "Arial", "DejaVu Sans", "Liberation Sans", "Noto Sans"]),
    (
        b"Hani",
        &[
            "PingFang SC",
            "Hiragino Sans",
            "Hiragino Kaku Gothic ProN",
            "STHeiti",
            "Heiti SC",
            "Apple SD Gothic Neo",
            "Noto Sans CJK SC",
            "Noto Sans CJK JP",
            "Noto Sans SC",
            "Source Han Sans",
            "WenQuanYi Micro Hei",
            "Droid Sans Fallback",
        ],
    ),
    (b"Hira", &["Hiragino Sans", "Hiragino Kaku Gothic ProN", "Noto Sans CJK JP", "Noto Sans JP", "Droid Sans Fallback"]),
    (b"Kana", &["Hiragino Sans", "Hiragino Kaku Gothic ProN", "Noto Sans CJK JP", "Noto Sans JP", "Droid Sans Fallback"]),
    (b"Hang", &["Apple SD Gothic Neo", "AppleGothic", "Noto Sans CJK KR", "Noto Sans KR", "NanumGothic", "Droid Sans Fallback"]),
    (b"Arab", &["Geeza Pro", "Noto Sans Arabic", "Noto Naskh Arabic", "DejaVu Sans"]),
    (b"Hebr", &["Arial Hebrew", "Noto Sans Hebrew", "DejaVu Sans"]),
    (b"Thai", &["Thonburi", "Noto Sans Thai", "Garuda"]),
    (b"Deva", &["Kohinoor Devanagari", "Devanagari MT", "Noto Sans Devanagari", "Lohit Devanagari"]),
    (b"Beng", &["Kohinoor Bangla", "Bangla MN", "Noto Sans Bengali"]),
    (b"Taml", &["Tamil MN", "Noto Sans Tamil"]),
];

/// The families emoji are drawn with, the first the system has.
const EMOJI: &[&str] = &["Apple Color Emoji", "Noto Color Emoji", "Segoe UI Emoji", "Twemoji Mozilla"];

/// Blitz 0.3.0-beta.4 lays out none of a `display: table` box's children that aren't table parts:
/// CSS 2.1 (§17.2.1) gives them anonymous boxes, and Blitz's `collect_table_cells` drops them
/// ("probably a table caption": Blitz's issue #1430, its PRs #783 and #1370), so Mermaid's edge
/// labels, a `<span>` in a `display: table` box, came out blank. Until Blitz does it, this does,
/// once the styles have resolved and before layout, as CSS does: in a table or a row group, each
/// run of children that are cells or content shares one anonymous row; in a row, its own or an
/// anonymous one, each run of content gets one anonymous cell. The wrappers are elements named
/// `clv-row` and `clv-cell`, so no rule on a name or a class reaches them (a rule on `*` does);
/// the nodes moved into them keep the styles Stylo gave them in the tree as written, so the rules
/// that matched them there still hold, `#t > :nth-child(3)` included. The tree is edited in place,
/// each box's children listed anew once, so it costs what the labels hold.
/// Says whether anything moved, so that the styles and layout need resolving again.
/// `blitz_still_drops_inline_content_in_a_table` fails once Blitz draws them: then take this out,
/// and close dobladez/clview#1, which tracks it.
fn anonymous_cells(doc: &mut BaseDocument) -> bool {
    // What each box holds, read before anything moves.
    let mut plans: Vec<(NodeId, Vec<Wrapped>)> = Vec::new();
    let mut walk = vec![doc.root_element().id];
    while let Some(id) = walk.pop() {
        let Some(node) = doc.get_node(id) else { continue };
        walk.extend(node.children.iter().copied());
        let Some(inside) = node.primary_styles().map(|s| s.clone_display().inside()) else { continue };
        let kinds: Vec<(NodeId, Kind)> = node.children.iter().map(|child| (*child, kind(doc, *child))).collect();
        use style::values::specified::box_::DisplayInside as I;
        let plan = match inside {
            I::Table | I::TableRowGroup | I::TableHeaderGroup | I::TableFooterGroup => rows_in(&kinds),
            I::TableRow => cells_in(&kinds),
            _ => None,
        };
        if let Some(plan) = plan {
            plans.push((id, plan));
        }
    }
    if plans.is_empty() {
        return false;
    }
    for (parent, plan) in plans {
        let mut wrappers = Vec::new();
        let children = realized(doc, parent, plan, &mut wrappers);
        if let Some(node) = doc.get_node_mut(parent) {
            node.children = children.into_iter().collect();
        }
        // Stylo visits a wrapper, unstyled, when the path down to it is marked dirty; what it
        // holds keeps its styles, and the next resolve recascades the inherited ones under it.
        for wrapper in wrappers {
            if let Some(node) = doc.get_node(wrapper) {
                node.mark_ancestors_dirty();
            }
        }
    }
    // Every box is laid out anew at the next resolve: the construction damage that would single
    // out the edited ones is Blitz's own, not reachable from outside, and a label's tree is laid
    // out once more at most.
    doc.set_incremental_layout(false);
    true
}

/// What a child is to the table box that holds it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Stays where it is: a caption, a column, a hidden or absolutely positioned child, a
    /// `display: contents` one, a comment.
    Part,
    /// A row or a row group: a row of its own at table level.
    Row,
    Cell,
    /// Anything else, which CSS gives an anonymous cell (and row).
    Content,
    /// Text of spaces alone, which makes no box: it goes with a run under way.
    Spaces,
}

fn kind(doc: &BaseDocument, id: NodeId) -> Kind {
    use style::values::specified::box_::{DisplayInside, DisplayOutside};
    let Some(node) = doc.get_node(id) else { return Kind::Part };
    match (node.element_data(), node.text_data()) {
        (None, Some(text)) if text.content.trim().is_empty() => Kind::Spaces,
        (None, Some(_)) => Kind::Content,
        (None, None) => Kind::Part,
        (Some(_), _) => {
            let Some(style) = node.primary_styles() else { return Kind::Part };
            let display = style.clone_display();
            if style.clone_position().is_absolutely_positioned() {
                return Kind::Part;
            }
            match (display.outside(), display.inside()) {
                (DisplayOutside::None | DisplayOutside::TableCaption, _) | (_, DisplayInside::Contents) => Kind::Part,
                (_, DisplayInside::TableRow | DisplayInside::TableRowGroup | DisplayInside::TableHeaderGroup | DisplayInside::TableFooterGroup) => {
                    Kind::Row
                }
                (_, DisplayInside::TableColumn | DisplayInside::TableColumnGroup) => Kind::Part,
                (_, DisplayInside::TableCell) => Kind::Cell,
                _ => Kind::Content,
            }
        }
    }
}

/// A box's children as they will be: each as it was, or a new wrapper of a `display` holding some.
enum Wrapped {
    Keep(NodeId),
    Wrap(&'static str, Vec<Wrapped>),
}

/// The children of a table or a row group regrouped: each run of cells and content in one
/// anonymous row, the content in it in anonymous cells; `None` where nothing needs one.
fn rows_in(kinds: &[(NodeId, Kind)]) -> Option<Vec<Wrapped>> {
    let in_a_row = |k: Kind| matches!(k, Kind::Cell | Kind::Content);
    if !kinds.iter().any(|(_, k)| matches!(k, Kind::Content)) {
        return None;
    }
    let mut out = Vec::new();
    let mut run: Vec<(NodeId, Kind)> = Vec::new();
    let close = |run: &mut Vec<(NodeId, Kind)>, out: &mut Vec<Wrapped>| {
        if !run.is_empty() {
            let cells = cells_in(run).unwrap_or_else(|| run.iter().map(|(id, _)| Wrapped::Keep(*id)).collect());
            out.push(Wrapped::Wrap("table-row", cells));
            run.clear();
        }
    };
    for (id, k) in kinds.iter().copied() {
        if in_a_row(k) || (k == Kind::Spaces && !run.is_empty()) {
            run.push((id, k));
        } else {
            close(&mut run, &mut out);
            out.push(Wrapped::Keep(id));
        }
    }
    close(&mut run, &mut out);
    Some(out)
}

/// The children of a row regrouped: each run of content in one anonymous cell; `None` where
/// nothing needs one.
fn cells_in(kinds: &[(NodeId, Kind)]) -> Option<Vec<Wrapped>> {
    if !kinds.iter().any(|(_, k)| matches!(k, Kind::Content)) {
        return None;
    }
    let mut out = Vec::new();
    let mut run: Vec<Wrapped> = Vec::new();
    for (id, k) in kinds.iter().copied() {
        if k == Kind::Content || (k == Kind::Spaces && !run.is_empty()) {
            run.push(Wrapped::Keep(id));
        } else {
            if !run.is_empty() {
                out.push(Wrapped::Wrap("table-cell", std::mem::take(&mut run)));
            }
            out.push(Wrapped::Keep(id));
        }
    }
    if !run.is_empty() {
        out.push(Wrapped::Wrap("table-cell", run));
    }
    Some(out)
}

/// `items` made the children of `parent`: a wrapper created for each `Wrap`, named so that no
/// rule on a name reaches it, its `display` as inline style, which only a rule on `*` could
/// outweigh, in the document with its parent and children set; the kept nodes given their parent.
/// The wrappers made are listed in `wrappers`.
fn realized(doc: &mut BaseDocument, parent: NodeId, items: Vec<Wrapped>, wrappers: &mut Vec<NodeId>) -> Vec<NodeId> {
    use blitz_dom::node::NodeFlags;
    use blitz_dom::{ElementData, LocalName, NodeData, QualName, ns};
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let id = match item {
            Wrapped::Keep(id) => id,
            Wrapped::Wrap(display, inner) => {
                let name = QualName::new(None, ns!(html), LocalName::from(if display == "table-row" { "clv-row" } else { "clv-cell" }));
                let wrapper = doc.create_node(NodeData::Element(Box::new(ElementData::new(name, Vec::new()))));
                // Its one style, as a style attribute's declaration: an author rule on its name
                // can't exist, and one on `*` is the known difference.
                doc.set_style_property(wrapper, "display", display);
                let children = realized(doc, wrapper, inner, wrappers);
                if let Some(node) = doc.get_node_mut(wrapper) {
                    node.children = children.into_iter().collect();
                    node.flags.insert(NodeFlags::IS_IN_DOCUMENT);
                }
                wrappers.push(wrapper);
                wrapper
            }
        };
        if let Some(node) = doc.get_node_mut(id) {
            node.parent = Some(parent);
        }
        out.push(id);
    }
    out
}

/// `label` replayed through `to` into `size` pixels: premultiplied RGBA, row by row.
fn painted(label: &Recorded, to: [f64; 6], size: (u32, u32)) -> Option<Vec<u8>> {
    if size.0 == 0 || size.1 == 0 || to.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let rgba = render_to_buffer::<VelloCpuImageRenderer, _>(|scene| scene.append_scene(label.scene.clone(), Affine::new(to)), size.0, size.1);
    let expected = u64::from(size.0).saturating_mul(u64::from(size.1)).saturating_mul(4);
    (u64::try_from(rgba.len()).ok() == Some(expected)).then_some(rgba)
}

/// `command` moved by `by` each way.
fn moved(command: RenderCommand, by: f64) -> RenderCommand {
    let shift = |t: Affine| Affine::translate((by, by)) * t;
    match command {
        RenderCommand::PushLayer(mut c) => {
            c.transform = shift(c.transform);
            RenderCommand::PushLayer(c)
        }
        RenderCommand::PushClipLayer(mut c) => {
            c.transform = shift(c.transform);
            RenderCommand::PushClipLayer(c)
        }
        RenderCommand::Stroke(mut c) => {
            c.transform = shift(c.transform);
            RenderCommand::Stroke(c)
        }
        RenderCommand::Fill(mut c) => {
            c.transform = shift(c.transform);
            RenderCommand::Fill(c)
        }
        RenderCommand::GlyphRun(mut c) => {
            c.transform = shift(c.transform);
            RenderCommand::GlyphRun(c)
        }
        RenderCommand::BoxShadow(mut c) => {
            c.transform = shift(c.transform);
            RenderCommand::BoxShadow(c)
        }
        RenderCommand::PopLayer => RenderCommand::PopLayer,
    }
}

/// What each slot of `doc` paints over, in its slot's pixels, in order: its `<foreignObject>`'s
/// box (the first element of that name in the slot), and what overflows it.
fn boxes(doc: &BaseDocument) -> Vec<Rect> {
    let node = |id: NodeId| doc.get_node(id);
    let is_element = |id: &NodeId| node(*id).is_some_and(|n| n.element_data().is_some());
    let named = |id: NodeId, name: &str| node(id).and_then(|n| n.element_data()).is_some_and(|e| &*e.name.local == name);
    let html = doc.root_element();
    let Some(body) = html.children.iter().copied().find(|c| named(*c, "body")).and_then(node) else { return Vec::new() };
    let mut out = Vec::new();
    for slot in body.children.iter().copied().filter(is_element) {
        let mut walk = vec![slot];
        let mut found = None;
        while let Some(id) = walk.pop() {
            if named(id, "foreignObject") {
                found = Some(id);
                break;
            }
            if let Some(n) = node(id) {
                walk.extend(n.children.iter().rev().copied().filter(is_element));
            }
        }
        let region = found.and_then(node).map_or(Rect::ZERO, |fo| {
            let at = fo.absolute_position(0.0, 0.0);
            let size = fo.final_layout().size;
            let border = Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height));
            fo.scrollable_overflow().union(border) + kurbo::Vec2::new(f64::from(at.x), f64::from(at.y))
        });
        out.push(region);
    }
    out
}

// What the labels may fetch: `data:` URLs, a font or a picture each, sized from its header.

/// Fetches nothing but `data:` URLs (no network, no files), and of them only fonts and
/// pictures, within their budgets: anything else, or past them, arrives empty, so nothing waits on
/// it.
#[derive(Default)]
struct DataOnly {
    requests: AtomicUsize,
    fonts: AtomicU64,
    pictures: AtomicU64,
    links: Mutex<Vec<String>>,
}

impl NetProvider for DataOnly {
    fn fetch(&self, _doc: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.as_str().to_owned();
        let bytes = self.answer(&url).unwrap_or_default();
        handler.bytes(url, Bytes::from(bytes));
    }
}

impl DataOnly {
    /// What `url` is answered with, if anything.
    fn answer(&self, url: &str) -> Option<Vec<u8>> {
        let Some(data) = url.strip_prefix("data:") else {
            let mut links = self.links.lock().unwrap_or_else(PoisonError::into_inner);
            if links.len() < MAX_LINKS {
                // Its first LINK_BYTES, whole characters.
                let end = url.char_indices().map(|(i, c)| i + c.len_utf8()).take_while(|end| *end <= LINK_BYTES).last().unwrap_or(0);
                links.push(url.get(..end).unwrap_or_default().to_owned());
            }
            return None;
        };
        if self.requests.fetch_add(1, Ordering::Relaxed) >= MAX_REQUESTS {
            return None;
        }
        let bytes = data_url(data)?;
        // Counted in its total only when taken.
        let take = |total: &AtomicU64, size: u64, each: u64, all: u64| {
            size <= each && total.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |t| t.checked_add(size).filter(|n| *n <= all)).is_ok()
        };
        if let Some(size) = font_size(&bytes) {
            return take(&self.fonts, size, MAX_FONT, MAX_FONTS).then_some(bytes);
        }
        let picture = matches!(
            imagesize::image_type(&bytes),
            Ok(imagesize::ImageType::Png | imagesize::ImageType::Jpeg | imagesize::ImageType::Gif | imagesize::ImageType::Webp)
        );
        let size = imagesize::blob_size(&bytes).ok().filter(|_| picture)?;
        let decoded = u64::try_from(size.width).unwrap_or(u64::MAX).saturating_mul(u64::try_from(size.height).unwrap_or(u64::MAX)).saturating_mul(4);
        take(&self.pictures, decoded, MAX_PICTURE, MAX_PICTURES).then_some(bytes)
    }
}

/// What a `data:` URL's `data` (after `data:`) holds: base64, or percent-encoded.
fn data_url(data: &str) -> Option<Vec<u8>> {
    let (meta, payload) = data.split_once(',')?;
    if meta.ends_with(";base64") {
        let clean: Vec<u8> = percent_decoded(payload).into_iter().filter(|b| !b.is_ascii_whitespace()).collect();
        return base64::engine::general_purpose::STANDARD.decode(clean).ok();
    }
    Some(percent_decoded(payload))
}

/// `s` with its `%XX` escapes decoded.
fn percent_decoded(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = |at: usize| bytes.get(at).and_then(|c| char::from(*c).to_digit(16));
        match (b, hex(i.saturating_add(1)), hex(i.saturating_add(2))) {
            (b'%', Some(high), Some(low)) => {
                // Two hex digits: at most 255.
                out.push((high * 16 + low) as u8);
                i = i.saturating_add(3);
            }
            _ => {
                out.push(b);
                i = i.saturating_add(1);
            }
        }
    }
    out
}

/// What the font in `bytes` unpacks to, as its header says, or `None` when it isn't a font Blitz
/// reads: a WOFF the larger of its `totalSfntSize` and its tables' sizes, which its decoder (wuff)
/// sizes its output by, and larger than any where its tables would take the decoder past that; a
/// TrueType or OpenType font (or collection) its bytes.
fn font_size(bytes: &[u8]) -> Option<u64> {
    let u32_at = |at: usize| bytes.get(at..at.checked_add(4)?).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(|b| u64::from(u32::from_be_bytes(b)));
    let u16_at = |at: usize| bytes.get(at..at.checked_add(2)?).and_then(|b| <[u8; 2]>::try_from(b).ok()).map(|b| u64::from(u16::from_be_bytes(b)));
    let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    match bytes.get(..4)? {
        b"wOFF" => {
            // Each table's entry, 20 bytes from 44: its offset, its length stored and unpacked. Its
            // decoder copies a table stored no smaller than it unpacks as it is stored, so one
            // stored larger, or out of the file, or over another, is refused (as larger than any).
            let tables = u16_at(12)?;
            let mut ranges = Vec::new();
            let (mut sum, directory) = (0u64, 44u64.saturating_add(tables.saturating_mul(20)));
            for table in 0..tables {
                let at = usize::try_from(table.saturating_mul(20).saturating_add(44)).ok()?;
                let (offset, stored, unpacked) = (u32_at(at.checked_add(4)?)?, u32_at(at.checked_add(8)?)?, u32_at(at.checked_add(12)?)?);
                let end = offset.saturating_add(stored);
                if stored > unpacked || offset < directory || end > length {
                    return Some(u64::MAX);
                }
                ranges.push((offset, end));
                sum = sum.saturating_add(unpacked);
            }
            ranges.sort_unstable();
            if ranges.windows(2).any(|w| matches!(w, [a, b] if b.0 < a.1)) {
                return Some(u64::MAX);
            }
            Some(u32_at(16)?.max(sum))
        }
        b"wOF2" => {
            let (mut at, mut orig, mut stored) = (48usize, 0u64, 0u64);
            for _ in 0..u16_at(12)? {
                let flags = *bytes.get(at)?;
                at = at.checked_add(if flags & 0x3f == 0x3f { 5 } else { 1 })?;
                let (length, next) = base128(bytes, at)?;
                orig = orig.saturating_add(length);
                at = next;
                // glyf and loca are transformed at version 0, the others at any other.
                let version = flags >> 6;
                let transformed = if matches!(flags & 0x3f, 10 | 11) { version == 0 } else { version != 0 };
                if transformed {
                    let (length, next) = base128(bytes, at)?;
                    stored = stored.saturating_add(length);
                    at = next;
                } else {
                    stored = stored.saturating_add(length);
                }
            }
            Some(u32_at(16)?.max(orig).max(stored))
        }
        b"OTTO" | b"true" | b"ttcf" | [0, 1, 0, 0] => Some(length),
        _ => None,
    }
}

/// The UIntBase128 at `at` in `bytes` (WOFF 2's variable-length number), and where it ends.
fn base128(bytes: &[u8], at: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for i in 0..5usize {
        let b = *bytes.get(at.checked_add(i)?)?;
        if i == 0 && b == 0x80 {
            return None;
        }
        value = value.checked_mul(128)?.checked_add(u64::from(b & 0x7f))?;
        if b & 0x80 == 0 {
            return Some((value, at.checked_add(i)?.checked_add(1)?));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `data:` URL of `bytes`, in base64.
    fn data(bytes: &[u8]) -> String {
        format!("data:application/octet-stream;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    /// A WOFF header (and its table directory) saying it unpacks to `size` bytes.
    fn woff(size: u32) -> Vec<u8> {
        let mut out = b"wOFF".to_vec();
        out.extend(0x0001_0000u32.to_be_bytes());
        out.extend(64u32.to_be_bytes());
        out.extend(1u16.to_be_bytes());
        out.extend(0u16.to_be_bytes());
        out.extend(28u32.to_be_bytes());
        out.extend([0u8; 24]);
        out.extend(b"glyf");
        out.extend([64u32, 4, size, 0].iter().flat_map(|v| v.to_be_bytes()));
        out.extend([0u8; 4]);
        out
    }

    /// The header of a PNG `width` × `height`, as far as its size.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        out.extend(width.to_be_bytes());
        out.extend(height.to_be_bytes());
        out.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        out
    }

    /// A font is sized from its header before Blitz unpacks it: past 32 MiB it is refused, and
    /// past 64 MiB of a drawing's fonts in all.
    #[test]
    fn a_font_is_refused_past_what_its_header_says_it_unpacks_to() {
        assert_eq!(font_size(&woff(32 << 20)), Some(32 << 20));
        let net = DataOnly::default();
        assert!(net.answer(&data(&woff((32 << 20) + 1))).is_none());
        assert!(net.answer(&data(&woff(32 << 20))).is_some());
        assert!(net.answer(&data(&woff(32 << 20))).is_some());
        assert!(net.answer(&data(&woff(1))).is_none(), "past 64 MiB in all");
        // A WOFF 2's tables: one of 33 MiB, its length in UIntBase128.
        let mut woff2 = b"wOF2".to_vec();
        woff2.extend([0u8; 8]);
        woff2.extend(1u16.to_be_bytes());
        woff2.extend([0u8; 2]);
        woff2.extend(10u32.to_be_bytes());
        woff2.extend([0u8; 28]);
        woff2.push(0x3f);
        woff2.extend(b"abcd");
        woff2.extend([0x90, 0xc0, 0x80, 0x00]);
        assert_eq!(font_size(&woff2), Some(33 << 20));
        assert!(DataOnly::default().answer(&data(&woff2)).is_none());
        assert_eq!(font_size(b"OTTO and the rest"), Some(17));
        assert_eq!(font_size(b"wOFF"), None, "cut short");
    }

    /// A WOFF table its decoder would copy past what it sized its output by is refused: one stored
    /// larger than it unpacks, out of the file, or over another.
    #[test]
    fn a_woffs_inconsistent_tables_are_refused() {
        let tables = |entries: &[[u32; 3]], length: usize| {
            let mut out = b"wOFF".to_vec();
            out.extend(0x0001_0000u32.to_be_bytes());
            out.extend(u32::try_from(length).unwrap().to_be_bytes());
            out.extend(u16::try_from(entries.len()).unwrap().to_be_bytes());
            out.extend(0u16.to_be_bytes());
            out.extend(64u32.to_be_bytes());
            out.extend([0u8; 24]);
            for [offset, stored, unpacked] in entries {
                out.extend(b"glyf");
                out.extend([*offset, *stored, *unpacked, 0].iter().flat_map(|v| v.to_be_bytes()));
            }
            out.resize(length, 0);
            out
        };
        let directory = 44 + 2 * 20;
        assert_eq!(font_size(&tables(&[[directory, 8, 16], [directory + 8, 8, 16]], 100)), Some(64));
        assert_eq!(font_size(&tables(&[[directory, 16, 8]], 100)), Some(u64::MAX), "stored larger");
        assert_eq!(font_size(&tables(&[[directory, 8, 16], [directory + 4, 8, 16]], 100)), Some(u64::MAX), "overlapping");
        assert_eq!(font_size(&tables(&[[directory, 8, 16], [96, 8, 16]], 100)), Some(u64::MAX), "past the file");
        assert_eq!(font_size(&tables(&[[8, 8, 16], [directory, 8, 16]], 100)), Some(u64::MAX), "in the header");
    }

    /// A picture is sized from its header before it is decoded: past 64 MiB decoded it is
    /// refused, and past 256 MiB of a drawing's in all. Only PNG, JPEG, GIF and WebP are fetched:
    /// an SVG, a stylesheet, a document arrive empty.
    #[test]
    fn a_picture_is_refused_past_what_its_header_says_it_decodes_to() {
        let net = DataOnly::default();
        assert!(net.answer(&data(&png(4097, 4096))).is_none());
        for _ in 0..4 {
            assert!(net.answer(&data(&png(4096, 4096))).is_some());
        }
        assert!(net.answer(&data(&png(1, 1))).is_none(), "past 256 MiB in all");
        let net = DataOnly::default();
        for other in ["data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg'/>", "data:text/css,p{color:red}", "data:text/html,<p>hi</p>"] {
            assert!(net.answer(other).is_none(), "{other}");
        }
        assert_eq!(net.answer("data:,a%41b"), None, "not a font or a picture");
        assert_eq!(data_url(",a%41b%zz"), Some(b"aAb%zz".to_vec()));
    }

    /// `html` as Blitz has it at 240 × 200, its styles and layout resolved once. No fonts: the
    /// fixtures' coloured boxes need none.
    fn laid_out(html: &str) -> HtmlDocument {
        use parley::fontique::{Collection, CollectionOptions};
        let config = DocumentConfig {
            viewport: Some(Viewport::new(240, 200, 1.0, ColorScheme::Light)),
            font_ctx: Some(FontContext {
                collection: Collection::new(CollectionOptions { shared: false, system_fonts: false }),
                source_cache: Default::default(),
            }),
            ..DocumentConfig::default()
        };
        let mut doc = HtmlDocument::from_html(html, config);
        doc.handle_messages();
        doc.resolve(0.0);
        doc
    }

    /// `html` laid out and painted by Blitz at 240 × 200, with [`anonymous_cells`] or without:
    /// premultiplied RGBA.
    fn rendered(html: &str, workaround: bool) -> Vec<u8> {
        let (w, h) = (240u32, 200u32);
        let mut doc = laid_out(html);
        if workaround {
            anonymous_cells(&mut doc);
        }
        doc.resolve(0.0);
        let mut scene = Scene::new();
        blitz_paint::paint_scene(&mut scene, &mut doc, 1.0, w, h, 0, 0);
        render_to_buffer::<VelloCpuImageRenderer, _>(|s| s.append_scene(scene.clone(), Affine::IDENTITY), w, h)
    }

    /// How many red pixels each 48-pixel band of `html` rendered holds.
    fn red_by_band(html: &str, workaround: bool) -> [usize; 4] {
        let rgba = rendered(html, workaround);
        let red = |x: u32, y: u32| rgba.get(((y * 240 + x) * 4) as usize..).is_some_and(|p| p[0] > 150 && p[1] < 100);
        std::array::from_fn(|band| {
            (band as u32 * 48..(band as u32 + 1) * 48).flat_map(|y| (0..240).map(move |x| (x, y))).filter(|&(x, y)| red(x, y)).count()
        })
    }

    /// The box (x0, y0, x1, y1, exclusive) of the pixels of `rgba` (240 wide) that `is` says.
    fn box_of(rgba: &[u8], is: impl Fn(&[u8]) -> bool) -> Option<(u32, u32, u32, u32)> {
        let mut found: Option<(u32, u32, u32, u32)> = None;
        for (i, pixel) in rgba.chunks(4).enumerate() {
            if is(pixel) {
                let (x, y) = ((i % 240) as u32, (i / 240) as u32);
                found = Some(found.map_or((x, y, x + 1, y + 1), |(x0, y0, x1, y1)| (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1))));
            }
        }
        found
    }

    const RED: fn(&[u8]) -> bool = |p| p[0] > 150 && p[1] < 100 && p[2] < 100;
    const GREEN: fn(&[u8]) -> bool = |p| p[1] > 100 && p[0] < 100 && p[2] < 100;
    const BLUE: fn(&[u8]) -> bool = |p| p[2] > 150 && p[0] < 100 && p[1] < 100;

    /// Cells and the content between them share one anonymous row, the content alone in an
    /// anonymous cell (CSS 2.1 §17.2.1): a red cell, a blue inline box and a green cell come out
    /// side by side on one line, where Blitz alone drops the box and draws the cells together.
    #[test]
    fn cells_and_the_content_between_them_share_a_row() {
        let cell = |colour: &str| format!("<div style='display:table-cell;width:20px;height:20px;background:{colour}'></div>");
        let html = format!(
            "<html><body style='margin:0'><div style='display:table'>{}<span style='display:inline-block;width:20px;height:20px;background:blue'></span>{}</div></body></html>",
            cell("red"),
            cell("green")
        );
        let with = rendered(&html, true);
        assert_eq!(
            [box_of(&with, RED), box_of(&with, BLUE), box_of(&with, GREEN)],
            [Some((0, 0, 20, 20)), Some((20, 0, 40, 20)), Some((40, 0, 60, 20))],
            "one row of three"
        );
        let without = rendered(&html, false);
        assert_eq!(
            [box_of(&without, RED), box_of(&without, BLUE), box_of(&without, GREEN)],
            [Some((0, 0, 20, 20)), None, Some((20, 0, 40, 20))],
            "Blitz alone"
        );
    }

    /// The nodes moved into a wrapper keep the styles the rules gave them in the tree as written:
    /// a cell third among the table's children stays `#t > :nth-child(3)`'s, blue, though the two
    /// spans before it are wrapped in a row and a cell, and it with them; and a rule on `div`
    /// doesn't reach the wrappers.
    #[test]
    fn wrapping_leaves_the_tables_rules_as_they_matched() {
        let html = "<html><head><style>#t > :nth-child(3){background:blue} div{margin:0} .s{display:inline-block;width:10px;height:20px;background:red}</style></head>\
            <body style='margin:0'><div id='t' style='display:table'><span class='s'></span><span class='s'></span><div style='display:table-cell;width:20px;height:20px'></div></div></body></html>";
        let with = rendered(html, true);
        assert_eq!([box_of(&with, RED), box_of(&with, BLUE)], [Some((0, 0, 20, 20)), Some((20, 0, 40, 20))], "the spans, then the cell still blue");
        let without = rendered(html, false);
        assert_eq!([box_of(&without, RED), box_of(&without, BLUE)], [None, Some((0, 0, 20, 20))], "Blitz alone");
    }

    /// The wrapping costs what the box holds: 16,000 runs of three among 64,000 children are listed
    /// anew once, not taken out of the table one at a time, each a scan of the rest, which took
    /// 2.8 s here and grew with the square.
    #[test]
    fn anonymous_cells_cost_their_elements() {
        let html = format!(
            "<html><body><div style='display:table'>{}</div></body></html>",
            "<span>x</span><b>y</b><i>z</i><div style='display:table-row'></div>".repeat(16_000)
        );
        let mut doc = laid_out(&html);
        let started = std::time::Instant::now();
        assert!(anonymous_cells(&mut doc));
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "{:?}", started.elapsed());
    }

    /// Blitz 0.3.0-beta.4 draws nothing of inline content directly in a `display: table` box
    /// ([`anonymous_cells`] works around it): the first box of `tests/data/blitz-table.html`, a red
    /// square and text in such a box, comes out empty, while its three controls draw. Blitz's git
    /// main at 4647f4a2 (2026-11-10) does the same; Blitz tracks it as issue #1430. Of its open
    /// PRs, #781 draws the box but not the text, #783 (anonymous table boxes) and #1370 (Taffy's
    /// table layout) draw both, as measured on their branches that day. With the workaround, the
    /// first box draws as its controls do. dobladez/clview#1 tracks the workaround: close it when
    /// this test fails and the workaround comes out.
    #[test]
    fn blitz_still_drops_inline_content_in_a_table() {
        let html = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/blitz-table.html")).unwrap();
        let bands = red_by_band(&html, false);
        assert!(bands[1..].iter().all(|red| *red > 400), "the controls: {bands:?}");
        assert_eq!(
            bands[0], 0,
            "Blitz now draws inline content in display: table (its issue #1430; PR #783 or #1370 landed): remove the workaround (`anonymous_cells` in src/source/svg/html.rs) and this test, and close dobladez/clview#1"
        );
        let fixed = red_by_band(&html, true);
        assert!(fixed[0] > 400 && fixed[1..] == bands[1..], "with the workaround: {fixed:?}");
    }

    /// Only `data:` URLs are answered, at most 1,024 a drawing; what else a label asks for is
    /// told of, not fetched.
    #[test]
    fn only_so_many_data_urls_are_answered_and_nothing_else() {
        let net = DataOnly::default();
        for _ in 0..MAX_REQUESTS {
            assert!(net.answer(&data(&png(1, 1))).is_some());
        }
        assert!(net.answer(&data(&png(1, 1))).is_none());
        assert!(net.answer("https://example.com/font.woff2").is_none());
        assert!(net.answer("file:///etc/passwd").is_none());
        assert_eq!(*net.links.lock().unwrap(), ["https://example.com/font.woff2", "file:///etc/passwd"]);
    }
}
