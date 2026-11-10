//! SVG drawings, rendered in Rust by resvg (usvg parses them, tiny-skia paints): one page each,
//! drawn at whatever scale the viewer asks, from the drawing itself, so it stays sharp. svgz
//! (gzipped SVG) too. Opening parses the drawing once, to know it opens and its size, and keeps
//! only where its bytes are: a renderer, or a pool, parses it again on its first render and keeps
//! the tree for the renders after, until it goes, as an image keeps its pixels.
//!
//! A drawing is untrusted, and usvg and resvg bound little of what one costs, so this backend
//! does (ADR-0003):
//! - It reads nothing but the file: a link to a picture, a stylesheet or a font in another file
//!   or on the network is not followed, and said. Pictures embedded as `data:` URLs are drawn,
//!   up to [`MAX_PICTURE`] each decoded, and SVGs embedded in them up to [`MAX_NESTED`] deep.
//! - Before usvg parses it, entities, `use`s, markers and the clip paths, masks, filters and
//!   paint a shape refers to are counted the way usvg repeats them ([`Scan`]), within
//!   [`MAX_EXPANDED`] bytes and [`MAX_ELEMENTS`] elements, [`MAX_DEPTH`] deep.
//! - Before resvg renders it, what the render allocates at once and how many pixels it paints
//!   are estimated the way resvg 0.49 renders ([`Estimate`]), within [`MAX_RENDER`] and
//!   [`MAX_WORK`].
//! - Text is drawn with the system's fonts, found once, when a drawing first has text.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{self, fontdb, roxmltree};

use super::threads::Threads;
use super::{Again, Backend, Deferred, Format, Frame, HEAD, Input, Opened, Pixels, Pool, Render, Reopenable, Request, Source, guarded};
use crate::args::{OptionSpec, Parsed};

#[cfg(feature = "svg-html")]
mod html;

/// The worker that lays a drawing's labels out, in a process of its own.
#[cfg(feature = "svg-html")]
pub(super) use html::LABELS;

/// SVG drawings, rendered by resvg.
pub(super) struct Svg;

/// What it shows: a drawing, and one gzipped, each named so when its first bytes end before its
/// root element ([`named_svg`]).
const FORMATS: &[Format] = &[Format { name: "SVG", extensions: &["svg"] }, Format { name: "svgz", extensions: &["svgz"] }];

impl Backend for Svg {
    fn name(&self) -> &'static str {
        "resvg"
    }

    fn shows(&self) -> &'static str {
        "SVG drawings"
    }

    fn formats(&self) -> Vec<Format> {
        FORMATS.to_vec()
    }

    fn credits(&self) -> &[&'static str] {
        #[cfg(feature = "svg-html")]
        return &["resvg", html::CREDIT];
        #[cfg(not(feature = "svg-html"))]
        &["resvg"]
    }

    fn options(&self) -> &'static [OptionSpec] {
        &[]
    }

    fn opens(&self, path: &str, head: &[u8]) -> bool {
        is_svg(path, head)
    }

    fn draws_pictures(&self) -> bool {
        true
    }

    /// It parses the whole drawing to open it.
    fn deferred(&self, _path: &str, _head: &[u8]) -> Option<Deferred> {
        Some(Deferred { settings: &[] })
    }

    fn open(&self, input: Input, _options: &Parsed<'_>) -> Result<Opened, String> {
        let path = input.path.clone();
        let error = |e: String| format!("{path}: {e}");
        let (encoded, drawn) = match input.reopenable()? {
            Reopenable::Disk(file) => {
                let bytes = read(file).map_err(error)?;
                (Encoded::Disk(PathBuf::from(&path)), guarded_open(&bytes))
            }
            Reopenable::Bytes(bytes) => {
                let drawn = guarded_open(&bytes);
                (Encoded::Memory(bytes), drawn)
            }
            Reopenable::Again(again) => {
                let bytes = again().and_then(within).map_err(error)?;
                (Encoded::Held(again), guarded_open(&bytes))
            }
        };
        // Only the size is kept: the tree goes, and a render parses the drawing again.
        let size = drawn.map_err(error)?.size;
        Ok(Opened::Pages(Box::new(Drawing(Arc::new(Document { encoded, size })))))
    }
}

/// The largest drawing this opens, as text (an svgz unpacked): 64 MiB.
const MAX_FILE: u64 = 64 << 20;

/// The most bytes of content (attribute values and text) a drawing may come to once usvg has
/// repeated what its `use`s, markers and references repeat: 128 MiB. A drawing that repeats
/// nothing comes to its own size.
const MAX_EXPANDED: u64 = 128 << 20;

/// The most XML nodes read: elements, text, comments.
const MAX_NODES: u32 = 4_000_000;

/// The most elements, each `use` counting what it repeats again: usvg itself stops at a million.
const MAX_ELEMENTS: u64 = 1_000_000;

/// The deepest elements nest, each `use` and reference one level more: usvg's own limit.
const MAX_DEPTH: usize = 1024;

/// What matching its stylesheets costs: their rules, times its elements. usvg tries every rule on
/// every element; ten million take a quarter of a second (ADR-0003).
const MAX_STYLE: u64 = 50_000_000;

/// The most characters of text drawn, `use`s counted: usvg shapes them and turns every glyph
/// into a path as it parses, 200,000 in about 0.4 seconds. A drawing with more is drawn without
/// its text, which is said.
const MAX_TEXT: u64 = 200_000;

/// What laying out its text may cost: usvg's layout of a `<text>` element takes time as the
/// square of its characters, and as its spans times its characters, about 170 times as much per
/// unit (ADR-0003). This is about three seconds a parse on an M3 Max: a `<text>` of 70,000
/// characters, or of 20,000 in 1,300 `<tspan>`s. A drawing past it is drawn without its text,
/// which is said.
const MAX_LAYOUT: u64 = 5_000_000_000;

/// A span's cost in [`MAX_LAYOUT`], per character of its `<text>`.
const SPAN_COST: u64 = 170;

/// The most bytes a picture embedded in a drawing may take decoded: 64 MiB, 16 megapixels. A
/// larger one is left out, which is said.
const MAX_PICTURE: u64 = 64 << 20;

/// How deep SVGs embedded in a drawing may nest, each in the one before.
const MAX_NESTED: usize = 4;

/// The most times a drawing may look through every font for one that has a character its own
/// fonts lack: each look reads every font's character map.
const MAX_FALLBACKS: usize = 64;

/// The most a render may allocate at once, as estimated: its canvas, and the layers, masks and
/// filter buffers resvg makes on the way. resvg bounds a layer only to 25 times the canvas.
const MAX_RENDER: u64 = 1 << 30;

/// The most pixels a render may paint, as estimated: resvg paints about 550 million a second on
/// an M3 Max, so this is about seven seconds there (ADR-0003).
const MAX_WORK: u64 = 4_000_000_000;

/// The most nodes the estimate visits: a pattern is drawn again for every shape it fills.
const MAX_STEPS: u64 = 10_000_000;

/// The largest side of a render, in pixels. The viewer asks for at most 8192.
const MAX_SIDE: u64 = 16_384;

/// The SVG namespace, which the root element must be in.
const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// Whether the file at `path`, which starts with `head`, is an SVG drawing: XML (or gzipped XML)
/// whose root element is `svg`, after whatever may come first (a declaration, comments, a
/// doctype). Other XML is not one. When `head` ends before the root element does (a long
/// comment first), the name decides: `.svg` or `.svgz`.
fn is_svg(path: &str, head: &[u8]) -> bool {
    let unpacked;
    let text = if head.starts_with(&[0x1f, 0x8b]) {
        unpacked = unpacked_head(head);
        if unpacked.is_empty() {
            return false;
        }
        &unpacked[..]
    } else {
        head
    };
    // UTF-16 isn't read, but a file named as an SVG is still one: it is refused when opened.
    if text.starts_with(&[0xFF, 0xFE]) || text.starts_with(&[0xFE, 0xFF]) {
        return named_svg(path);
    }
    match prolog(text) {
        Root::Svg => true,
        Root::Other => false,
        Root::Unseen => named_svg(path),
    }
}

/// What the start of an XML document says its root element is.
#[derive(Debug, PartialEq)]
enum Root {
    Svg,
    /// Another element, or not XML at all.
    Other,
    /// The text ends first.
    Unseen,
}

/// The root element of the XML document `text` starts: what comes before it is skipped, as XML
/// allows it (whitespace, a declaration, comments, processing instructions, a doctype).
fn prolog(text: &[u8]) -> Root {
    let mut s = text.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(text);
    loop {
        s = s.trim_ascii_start();
        if s.is_empty() {
            return Root::Unseen;
        }
        let skipped = if s.starts_with(b"<?") {
            after(s, b"?>")
        } else if s.starts_with(b"<!--") {
            after(s, b"-->")
        } else if s.starts_with(b"<!DOCTYPE") {
            after_doctype(s)
        } else if let Some(tag) = s.strip_prefix(b"<") {
            let end = tag.iter().position(|&b| b.is_ascii_whitespace() || b == b'>' || b == b'/');
            let Some(end) = end else { return Root::Unseen };
            let name = tag.get(..end).unwrap_or_default();
            // `svg`, or `svg` after a prefix (`svg:svg`).
            return if name == b"svg" || (name.ends_with(b":svg") && name.len() > 4) { Root::Svg } else { Root::Other };
        } else {
            return Root::Other;
        };
        match skipped {
            Some(rest) => s = rest,
            None => return Root::Unseen,
        }
    }
}

/// What follows the first `end` in `s`, or `None` when there is none.
fn after<'a>(s: &'a [u8], end: &[u8]) -> Option<&'a [u8]> {
    let at = s.windows(end.len()).position(|w| w == end)?;
    s.get(at.checked_add(end.len())?..)
}

/// What follows the doctype `s` starts with: its name, ids in quotes, and an internal subset in
/// brackets, whose declarations may hold `>` and `]` in quotes or comments.
fn after_doctype(s: &[u8]) -> Option<&[u8]> {
    let mut rest = s.get(b"<!DOCTYPE".len()..)?;
    let mut subset = false;
    loop {
        let &b = rest.first()?;
        rest = match b {
            b'"' | b'\'' => {
                let quoted = rest.get(1..)?;
                // Within `quoted`, so one more is at most its length.
                quoted.get(quoted.iter().position(|&c| c == b)? + 1..)?
            }
            b'<' if rest.starts_with(b"<!--") => after(rest, b"-->")?,
            b'>' if !subset => return rest.get(1..),
            _ => {
                subset = match b {
                    b'[' => true,
                    b']' => false,
                    _ => subset,
                };
                rest.get(1..)?
            }
        };
    }
}

/// Whether `path` is named as an SVG is, or an svgz: either name goes for either.
fn named_svg(path: &str) -> bool {
    FORMATS.iter().any(|format| format.named(path))
}

/// The start of what the gzipped `head` unpacks to: as much as its bytes give, up to 8 KiB.
fn unpacked_head(head: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    // `head` is cut short, so this ends in an error, after what it could unpack.
    let _ = flate2::read::GzDecoder::new(head.get(..HEAD as usize).unwrap_or(head)).take(8 << 10).read_to_end(&mut out);
    out
}

/// An open drawing.
struct Drawing(Arc<Document>);

struct Document {
    encoded: Encoded,
    /// As shown, in CSS pixels.
    size: (f32, f32),
}

/// Where a drawing's bytes are.
enum Encoded {
    /// In a file on disk, read again to render.
    Disk(PathBuf),
    /// As they came through a pipe, which can't be read again.
    Memory(Vec<u8>),
    /// In another file (an archive), read from it again to render.
    Held(Again),
}

/// A drawing parsed, by the first render of a renderer or a pool, each of which has its own:
/// the parser's error, or a panic's, is kept too, so a drawing that fails isn't parsed again for
/// every render.
type Cache = OnceLock<Result<Arc<Drawn>, String>>;

impl Source for Drawing {
    fn pages(&self) -> usize {
        1
    }

    fn size(&self, page: usize) -> Option<(f32, f32)> {
        (page == 0).then_some(self.0.size)
    }

    /// A drawing scales: fitted to the window, it fills it, as a PDF's page does.
    fn natural(&self, _page: usize) -> bool {
        false
    }

    fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
        Ok(Box::new(Renderer { document: self.0.clone(), parsed: Cache::new() }))
    }

    fn pool(&self, threads: usize) -> Box<dyn Pool> {
        let (document, parsed) = (self.0.clone(), Arc::new(Cache::new()));
        Box::new(Threads::new(threads, Arc::new(move |request| document.render(request, &parsed))))
    }
}

struct Renderer {
    document: Arc<Document>,
    parsed: Cache,
}

impl Render for Renderer {
    fn render(&mut self, request: Request) -> Frame {
        guarded(request, || self.document.render(request, &self.parsed))
    }
}

impl Document {
    fn render(&self, request: Request, parsed: &Cache) -> Frame {
        self.rendered(request, parsed).unwrap_or_else(|e| Frame::failed(request, e))
    }

    fn rendered(&self, request: Request, parsed: &Cache) -> Result<Frame, String> {
        if request.page != 0 {
            return Err(format!("an SVG drawing has one page, not {}", request.page.saturating_add(1)));
        }
        let drawn = parsed.get_or_init(|| self.bytes().and_then(|bytes| guarded_parse(&bytes)).map(Arc::new)).clone()?;
        drawn.render(request)
    }

    /// Its bytes: the ones kept, or the file's again.
    fn bytes(&self) -> Result<Cow<'_, [u8]>, String> {
        let again = match &self.encoded {
            Encoded::Memory(bytes) => return Ok(Cow::Borrowed(&bytes[..])),
            Encoded::Disk(path) => File::open(path).map_err(|e| e.to_string()).and_then(read),
            Encoded::Held(again) => again().and_then(within),
        };
        again.map(Cow::Owned).map_err(|e| format!("reading the file again: {e}"))
    }
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

/// A drawing parsed: its tree, its size as shown, and what the reader should know about it.
struct Drawn {
    tree: usvg::Tree,
    size: (f32, f32),
    notes: Notes,
    /// Its HTML labels, found, and laid out by its first render that isn't a list's tile's.
    #[cfg(feature = "svg-html")]
    labels: Option<Labelled>,
}

/// A drawing's HTML labels: found and written out, then laid out and recorded in a worker of
/// their own, once, each with where it goes on the drawing and its opacity.
#[cfg(feature = "svg-html")]
struct Labelled {
    found: Found,
    prepared: OnceLock<Option<Prepared>>,
    /// Why the worker went while painting them, once it has: they aren't drawn again.
    failed: OnceLock<String>,
}

/// A drawing's labels laid out, each with where it goes on the drawing and its opacity.
#[cfg(feature = "svg-html")]
type Prepared = (Vec<(Transform, f32)>, html::Painted);

/// [`parse`], a parser's panic made an error: the drawing to render, its labels prepared.
fn guarded_parse(bytes: &[u8]) -> Result<Drawn, String> {
    std::panic::catch_unwind(|| parse(bytes, true)).unwrap_or_else(|_| Err("the SVG parser failed on this drawing".into()))
}

/// [`guarded_parse`], for a drawing being opened, which keeps only its size: its labels aren't
/// kept.
fn guarded_open(bytes: &[u8]) -> Result<Drawn, String> {
    std::panic::catch_unwind(|| parse(bytes, false)).unwrap_or_else(|_| Err("the SVG parser failed on this drawing".into()))
}

/// The drawing in `bytes`, an SVG or an svgz, at its size: its own, or its `viewBox`'s. One that
/// says neither (no `viewBox`, and a `width` or a `height` missing or a percentage) is as large
/// as what it draws reaches from its origin, as usvg sizes it, and 100 × 100 if it draws nothing.
/// With `labels`, its HTML labels are kept, to be laid out by its first render that draws them.
fn parse(bytes: &[u8], labels: bool) -> Result<Drawn, String> {
    let notes = Notes::default();
    let (tree, found) = parse_at(bytes, 0, &notes)?;
    let size = (tree.size().width(), tree.size().height());
    if !(size.0.is_finite() && size.1.is_finite() && size.0 > 0.0 && size.1 > 0.0) {
        return Err(format!("the drawing has no size to show ({} × {})", size.0, size.1));
    }
    #[cfg(feature = "svg-html")]
    let labels = (labels && !found.labels.is_empty()).then(|| Labelled { found, prepared: OnceLock::new(), failed: OnceLock::new() });
    #[cfg(not(feature = "svg-html"))]
    let (Found {}, _) = (found, labels);
    Ok(Drawn {
        tree,
        size,
        notes,
        #[cfg(feature = "svg-html")]
        labels,
    })
}

/// The labels `found`, laid out and recorded in a worker of their own, with where each goes; or
/// none, with why told to `notes`.
#[cfg(feature = "svg-html")]
fn prepared(found: &Found, notes: &Notes) -> Option<Prepared> {
    let leave_out = |why: &str| {
        for _ in &found.labels {
            notes.left_out_label(why);
        }
    };
    if found.style_work > MAX_STYLE {
        leave_out("its stylesheets' rules are too many for its labels' elements");
        return None;
    }
    match html::prepare(&found.css, &found.labels) {
        Ok(painted) => {
            notes.linked(painted.links.iter().map(String::as_str));
            for index in 0..found.labels.len() {
                if painted.region(index).is_none() {
                    notes.left_out_label(&format!("laid out past {} pixels a side", html::MAX_LAYOUT_SIDE));
                }
            }
            Some((found.labels.iter().map(|l| (l.place, l.opacity)).collect(), painted))
        }
        Err(why) => {
            leave_out(&why);
            None
        }
    }
}

/// Refuses `text` if its entities could expand it past [`MAX_EXPANDED`]: checked before every
/// parse.
fn bounded(text: &str) -> Result<(), String> {
    match entities_bound(text) > MAX_EXPANDED {
        true => Err(format!("its entities expand to more than {} MiB", MAX_EXPANDED >> 20)),
        false => Ok(()),
    }
}

/// `text`, parsed as XML within [`MAX_NODES`], once [`bounded`].
fn xml(text: &str) -> Result<roxmltree::Document<'_>, String> {
    bounded(text)?;
    let options = roxmltree::ParsingOptions { allow_dtd: true, nodes_limit: MAX_NODES, entity_resolver: None };
    roxmltree::Document::parse_with_options(text, options).map_err(|e| format!("not a well-formed SVG: {e}"))
}

/// `bytes` parsed by usvg, `depth` SVGs down from the file's own: once checked within this
/// backend's limits, with `notes` told what was left out. With the HTML labels to draw over it,
/// found in the file's own drawing only: in a build that draws them, what follows the child a
/// browser takes in a `<switch>` is taken out before usvg parses it.
fn parse_at(bytes: &[u8], depth: usize, notes: &Notes) -> Result<(usvg::Tree, Found), String> {
    let unpacked = unpacked(bytes)?;
    let text = std::str::from_utf8(&unpacked).map_err(|_| "the drawing isn't UTF-8 text (only UTF-8 SVGs are read)".to_string())?;
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let doc = xml(text)?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" || root.tag_name().namespace() != Some(SVG_NS) {
        return Err("not an SVG drawing: its root element isn't <svg> in the SVG namespace".into());
    }
    let scan = Scan::of(&doc)?;
    notes.linked(scan.links.iter().copied());
    let fonts = if !scan.has_text {
        Arc::new(fontdb::Database::new())
    } else if scan.text() > MAX_TEXT {
        notes.left_out_text(format!("more than {MAX_TEXT} characters"));
        Arc::new(fontdb::Database::new())
    } else if scan.layout > MAX_LAYOUT {
        notes.left_out_text("too long to lay out, in one <text> or in many spans".into());
        Arc::new(fontdb::Database::new())
    } else {
        let fonts = system_fonts();
        if fonts.is_empty() {
            notes.left_out_text("no fonts were found on this system".into());
        }
        fonts
    };
    // What matching the stylesheets costs, as the scan counted it: the labels' elements add to it.
    #[cfg(feature = "svg-html")]
    let style_work = scan.visited.saturating_mul(scan.rules);
    drop(scan);
    let options = options_for(notes, depth, fonts);
    #[cfg(feature = "svg-html")]
    if depth == 0 {
        // Percentages are of the root's viewport: its own, or, where it says none, the size usvg
        // gives the drawing from what it draws, known once its tree is made.
        let cut = without_fallbacks(&doc, text);
        let parsed = |doc: &roxmltree::Document| usvg::Tree::from_xmltree(doc, &options).map_err(|e| e.to_string());
        let (tree, mut found) = match root_viewport(doc.root_element()) {
            Some(viewport) => {
                let found = found_labels(&doc, notes, style_work, viewport);
                let tree = match cut {
                    Some(cut) => {
                        drop(doc);
                        parsed(&xml(&cut)?)?
                    }
                    None => parsed(&doc)?,
                };
                (tree, found)
            }
            None => {
                let tree = match &cut {
                    Some(cut) => parsed(&xml(cut)?)?,
                    None => parsed(&doc)?,
                };
                let found = found_labels(&doc, notes, style_work, (tree.size().width(), tree.size().height()));
                (tree, found)
            }
        };
        found.place(&tree);
        return Ok((tree, found));
    }
    notes.undrawn.fetch_add(label_count(&doc), Ordering::Relaxed);
    let tree = usvg::Tree::from_xmltree(&doc, &options).map_err(|e| e.to_string())?;
    Ok((tree, Found::default()))
}

/// `bytes`, unpacked if they are gzipped (an svgz), to at most [`MAX_FILE`].
fn unpacked(bytes: &[u8]) -> Result<Cow<'_, [u8]>, String> {
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(Cow::Borrowed(bytes));
    }
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes).take(MAX_FILE + 1).read_to_end(&mut out).map_err(|e| format!("its gzip is broken: {e}"))?;
    if u64::try_from(out.len()).unwrap_or(u64::MAX) > MAX_FILE {
        return Err(format!("the drawing is more than {} MiB unpacked", MAX_FILE >> 20));
    }
    Ok(Cow::Owned(out))
}

/// The most bytes `text` can come to once roxmltree expands its entities: each `&` can stand for
/// the longest entity, itself expanded, down to roxmltree's own limit of 10 levels. A document
/// that declares none comes to its own length.
fn entities_bound(text: &str) -> u64 {
    let length = u64::try_from(text.len()).unwrap_or(u64::MAX);
    if !text.contains("<!ENTITY") {
        return length;
    }
    // Every declaration, wherever it is, by its text: one missed would bound nothing.
    let (mut longest, mut most_refs) = (0u64, 0u64);
    for declaration in text.split("<!ENTITY").skip(1) {
        let value = entity_value(declaration).unwrap_or(declaration);
        longest = longest.max(u64::try_from(value.len()).unwrap_or(u64::MAX));
        most_refs = most_refs.max(u64::try_from(value.matches('&').count()).unwrap_or(u64::MAX));
    }
    let mut expanded = longest;
    for _ in 0..10 {
        expanded = longest.saturating_add(most_refs.saturating_mul(expanded));
    }
    let refs = u64::try_from(text.matches('&').count()).unwrap_or(u64::MAX);
    length.saturating_add(refs.saturating_mul(expanded))
}

/// The quoted value of the entity declaration `declaration` starts (after `<!ENTITY`).
fn entity_value(declaration: &str) -> Option<&str> {
    let start = declaration.find(['"', '\''])?;
    let quote = declaration.get(start..)?.chars().next()?;
    let value = declaration.get(start + 1..)?;
    value.get(..value.find(quote)?)
}

// HTML labels (Spec-0004): the `<foreignObject>`s a browser would draw, found, placed and written
// out for the painter in `svg/html.rs`, which draws them over what resvg draws.

/// The XHTML namespace, a label's HTML's.
#[cfg(feature = "svg-html")]
const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";

/// The most HTML labels drawn: painting each one walks every label's box (ADR-0024).
#[cfg(feature = "svg-html")]
const MAX_LABELS: usize = 1_024;

/// The most elements all of a drawing's labels hold, within its [`MAX_ELEMENTS`]: the painter
/// holds some 1.7 KB an element (ADR-0024).
#[cfg(feature = "svg-html")]
const MAX_LABEL_ELEMENTS: u64 = 100_000;

/// The most markup written out for a drawing's labels, its stylesheets included.
#[cfg(feature = "svg-html")]
const MAX_LABEL_MARKUP: usize = 8 << 20;

/// The most text in a drawing's labels: 1 MiB lays out in 0.4 seconds (ADR-0024).
#[cfg(feature = "svg-html")]
const MAX_LABEL_TEXT: usize = 1 << 20;

/// The deepest a label's elements nest, from its `<foreignObject>` down, as they are written out:
/// the writer recurses once a level, on a render thread, and the drawing's own bound on its
/// depth ([`MAX_DEPTH`]) doesn't reach every element a label holds, since its scan skips some by
/// name (`<metadata>`, `<defs>`). As deep as the drawing may nest: the worker lays them out on a
/// stack of 64 MiB, which holds 1,024 nested grids in a debug build (ADR-0025).
#[cfg(feature = "svg-html")]
const MAX_LABEL_DEPTH: usize = 1_024;

/// The XLink namespace, of SVG 1.1's `xlink:href`.
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

/// The languages usvg takes a `systemLanguage` to ask for: its default.
const LANGUAGES: &[&str] = &["en"];

/// Elements whose content isn't drawn where it is, only when something refers to it.
const UNSHOWN: &[&str] = &["defs", "symbol", "clipPath", "mask", "pattern", "marker"];

/// Whether `node` is an SVG element named `name`.
fn svg_element(node: roxmltree::Node, name: &str) -> bool {
    node.has_tag_name((SVG_NS, name))
}

/// The `<foreignObject>`s of `doc` a browser draws, in document order, and its drawn `<use>`s,
/// found in one walk from its root by [`shown_parts`]' rule: none in a definition (`defs`,
/// `symbol`, …) or in another label; none that is, or whose ancestor is, hidden by its attributes
/// or its `style`; and in each `<switch>`, only in the child it takes.
fn drawn<'a, 'input>(doc: &'a roxmltree::Document<'input>) -> (Vec<roxmltree::Node<'a, 'input>>, Vec<roxmltree::Node<'a, 'input>>) {
    let (mut labels, mut uses, mut walk) = (Vec::new(), Vec::new(), vec![doc.root_element()]);
    while let Some(node) = walk.pop() {
        if hidden(node) {
            continue;
        }
        if svg_element(node, "foreignObject") {
            labels.push(node);
            continue;
        }
        if svg_element(node, "use") {
            uses.push(node);
            continue;
        }
        let next = walk.len();
        walk.extend(shown_parts(node));
        // Walked in document order.
        if let Some(children) = walk.get_mut(next..) {
            children.reverse();
        }
    }
    (labels, uses)
}

/// What a shown element `node` (neither a label nor a `<use>`) shows of its children: a
/// `<switch>` the child it takes, any other its element children; none of them a definition.
fn shown_parts<'a, 'input>(node: roxmltree::Node<'a, 'input>) -> impl Iterator<Item = roxmltree::Node<'a, 'input>> {
    let switch = svg_element(node, "switch");
    let taken = if switch { taken(node) } else { None };
    let children = node.children().filter(move |_| !switch).filter(roxmltree::Node::is_element);
    taken.into_iter().chain(children).filter(|n| !UNSHOWN.iter().any(|name| svg_element(*n, name)))
}

/// Whether `node` is hidden by its own attributes or `style`: `display: none`, `visibility:
/// hidden` or `collapse`. A stylesheet's rules aren't read.
fn hidden(node: roxmltree::Node) -> bool {
    let says = |property: &str, value: &str| match property.trim() {
        "display" => value.trim().eq_ignore_ascii_case("none"),
        "visibility" => ["hidden", "collapse"].iter().any(|v| value.trim().eq_ignore_ascii_case(v)),
        _ => false,
    };
    let by_attribute = ["display", "visibility"].iter().any(|p| node.attribute(*p).is_some_and(|v| says(p, v)));
    by_attribute || declarations(node.attribute("style").unwrap_or_default()).any(|(p, v)| says(p, v.trim_end_matches("!important")))
}

/// The declarations of the `style` attribute `style`, each a property and its value.
fn declarations(style: &str) -> impl Iterator<Item = (&str, &str)> {
    style.split(';').filter_map(|d| d.split_once(':'))
}

/// The child of `switch` a browser draws: its first element that asks for no extension
/// (`requiredExtensions`) and a language usvg takes (`systemLanguage`), as usvg evaluates them;
/// what it asks of `requiredFeatures` is taken as met, as browsers do.
fn taken<'a, 'input>(switch: roxmltree::Node<'a, 'input>) -> Option<roxmltree::Node<'a, 'input>> {
    switch.children().filter(roxmltree::Node::is_element).find(|child| {
        let language = child.attribute("systemLanguage").is_none_or(|languages| {
            languages.split(',').map(str::trim).any(|l| LANGUAGES.iter().any(|ours| l == *ours || l.split_once('-').is_some_and(|(p, _)| p == *ours)))
        });
        !child.has_attribute("requiredExtensions") && language
    })
}

/// How many labels of `doc` a browser would draw: those [`drawn`] finds, and those its `<use>`s
/// show.
fn label_count(doc: &roxmltree::Document) -> usize {
    let (labels, uses) = drawn(doc);
    labels.len().saturating_add(used_labels(doc, &uses))
}

/// The labels the `<use>`s `uses` of `doc` show, each through what it refers to, by the rule
/// [`drawn`] follows, nested `<use>`s too: usvg copies a `<use>`, not a label, so they are left
/// out. What each element shows is counted once and remembered, however many uses (or targets
/// holding one another) reach it, so counting visits each element at most twice; a `<use>` that
/// refers back to one being counted shows nothing more.
fn used_labels(doc: &roxmltree::Document, uses: &[roxmltree::Node]) -> usize {
    if uses.is_empty() {
        return 0;
    }
    let ids: HashMap<&str, roxmltree::Node> = doc.descendants().filter_map(|n| Some((n.attribute("id")?, n))).collect();
    // What each element shows, once counted; and those being counted.
    let mut shown: HashMap<roxmltree::NodeId, usize> = HashMap::new();
    let mut counting: HashSet<roxmltree::NodeId> = HashSet::new();
    // What a node shows through: a `<use>` its target (a `<symbol>` too, which it shows, but no
    // other definition), any other its [`shown_parts`].
    let parts = |node: roxmltree::Node<'_, '_>| -> Vec<roxmltree::NodeId> {
        if hidden(node) || svg_element(node, "foreignObject") {
            return Vec::new();
        }
        if svg_element(node, "use") {
            let href = node.attribute("href").or_else(|| node.attribute((XLINK_NS, "href"))).and_then(|h| h.trim().strip_prefix('#'));
            let target = href.and_then(|id| ids.get(id)).filter(|t| svg_element(**t, "symbol") || !UNSHOWN.iter().any(|name| svg_element(**t, name)));
            return target.map(roxmltree::Node::id).into_iter().collect();
        }
        shown_parts(node).map(|n| n.id()).collect()
    };
    let mut total = 0usize;
    for start in uses {
        // Depth first, each node left once its parts are counted.
        let mut walk = vec![(start.id(), false)];
        while let Some((id, counted)) = walk.pop() {
            let Some(node) = doc.get_node(id) else { continue };
            if counted {
                let own = usize::from(svg_element(node, "foreignObject") && !hidden(node));
                let sum = parts(node).iter().filter_map(|p| shown.get(p)).fold(own, |sum, n| sum.saturating_add(*n));
                counting.remove(&id);
                shown.insert(id, sum);
            } else if !shown.contains_key(&id) && counting.insert(id) {
                walk.push((id, true));
                walk.extend(parts(node).into_iter().filter(|p| !shown.contains_key(p) && !counting.contains(p)).map(|p| (p, false)));
            }
        }
        total = total.saturating_add(shown.get(&start.id()).copied().unwrap_or(0));
    }
    total
}

/// A drawing's labels to draw: its stylesheets, and each label written out.
#[derive(Default)]
struct Found {
    #[cfg(feature = "svg-html")]
    css: String,
    #[cfg(feature = "svg-html")]
    labels: Vec<Label>,
    /// The root's `viewBox` and `preserveAspectRatio`, which usvg applies and doesn't say.
    #[cfg(feature = "svg-html")]
    view: Option<(svgtypes::ViewBox, svgtypes::AspectRatio)>,
    /// What matching the stylesheets (the labels' hints too) costs, the labels' elements with
    /// the drawing's, counted as ADR-0003 counts them: rules times elements.
    #[cfg(feature = "svg-html")]
    style_work: u64,
}

/// A label, written out for the painter.
#[cfg(feature = "svg-html")]
struct Label {
    /// XHTML: copies of its ancestors (their ids, classes, styles, and font and colour
    /// attributes as styles, `display: contents`), then of the `<foreignObject>` made its box,
    /// `width` × `height` CSS pixels at its top left, the containing block of what it holds.
    markup: String,
    width: f32,
    height: f32,
    /// Whether what it holds is drawn past its box (`overflow: visible`).
    visible: bool,
    /// From its box (its CSS pixels, from its top left) to the drawing's size as shown.
    place: Transform,
    /// Its ancestors' opacity and its own, multiplied: the label is drawn at it as a whole.
    opacity: f32,
}

#[cfg(feature = "svg-html")]
impl Found {
    /// Each label placed on the drawing, at the size usvg gave `tree`.
    fn place(&mut self, tree: &usvg::Tree) {
        let size = (tree.size().width(), tree.size().height());
        let view = self.view.map_or(Transform::identity(), |(view_box, aspect)| fitted(view_box, aspect, size));
        for label in &mut self.labels {
            label.place = view.pre_concat(label.place);
        }
    }
}

/// What the labels of a drawing have cost so far, as they are written out.
#[cfg(feature = "svg-html")]
#[derive(Clone, Copy, Default)]
struct Spent {
    markup: usize,
    text: usize,
    elements: u64,
}

/// Why a label wasn't written out.
#[cfg(feature = "svg-html")]
enum Past {
    /// It nests too deep: it alone is left out.
    Nesting,
    /// The labels have spent a budget: it and the rest are left out.
    Budget(String),
}

/// `doc`'s labels a browser would draw ([`drawn`]), placed and written out within their
/// budgets; what is left out is told to `notes`.
#[cfg(feature = "svg-html")]
fn found_labels(doc: &roxmltree::Document, notes: &Notes, style_work: u64, viewport: (f32, f32)) -> Found {
    let root = doc.root_element();
    let view_box = root.attribute("viewBox").and_then(|v| v.parse::<svgtypes::ViewBox>().ok()).filter(|v| v.w > 0.0 && v.h > 0.0);
    let aspect = root.attribute("preserveAspectRatio").and_then(|v| v.parse().ok()).unwrap_or_default();
    // The `<font>` hints first, of no specificity, so the drawing's own rules win. A label's own
    // `<style>`s join the drawing's, as a browser takes them: the painter is given no element
    // that loads a stylesheet ([`LOADING`]).
    let styles: Vec<&str> = doc
        .descendants()
        .filter(|n| svg_element(*n, "style") || (n.has_tag_name((XHTML_NS, "style")) && n.ancestors().any(|a| svg_element(a, "foreignObject"))))
        .filter_map(|n| n.text())
        .collect();
    let css = format!("{}\n{}", font_hints(doc), styles.join("\n"));
    let mut spent = Spent { markup: css.len(), ..Spent::default() };
    let mut found = Found { css, labels: Vec::new(), view: view_box.map(|v| (v, aspect)), style_work };
    let mut spent_out: Option<String> = (spent.markup > MAX_LABEL_MARKUP).then(|| format!("past {} MiB of markup", MAX_LABEL_MARKUP >> 20));
    let (labels, uses) = drawn(doc);
    for fo in labels {
        if spent_out.is_none() && found.labels.len() >= MAX_LABELS {
            spent_out = Some(format!("past {MAX_LABELS} of them"));
        }
        if let Some(why) = &spent_out {
            notes.left_out_label(why);
            continue;
        }
        let Some((place, width, height)) = placed(fo, viewport) else { continue };
        let before = spent;
        match written_label(fo, width, height, &mut spent) {
            Ok((markup, visible)) => found.labels.push(Label { markup, width, height, visible, place, opacity: opacity(fo) }),
            Err(Past::Nesting) => {
                spent = before;
                notes.left_out_label(&format!("nested more than {MAX_LABEL_DEPTH} deep"));
            }
            Err(Past::Budget(why)) => {
                notes.left_out_label(&why);
                spent_out = Some(why);
            }
        }
    }
    notes.undrawn.fetch_add(used_labels(doc, &uses), Ordering::Relaxed);
    // The labels' elements are matched against every rule too, as their document's: the copies of
    // their context, and for each label a slot and its box.
    let rules = u64::try_from(found.css.matches('{').count()).unwrap_or(u64::MAX).saturating_add(1);
    let labels = u64::try_from(found.labels.len()).unwrap_or(u64::MAX);
    let elements = spent.elements.saturating_add(labels.saturating_mul(2)).saturating_add(4);
    found.style_work = found.style_work.saturating_add(rules.saturating_mul(elements));
    found
}

/// The opacity the label `node` is drawn at: its ancestors' (`opacity`, by attribute or `style`, a
/// number or a percentage, within 0 to 1), multiplied, and its own attribute's. Its own `style`'s
/// the painter applies, its copy keeping it ([`copy_of`]), so it isn't counted twice.
#[cfg(feature = "svg-html")]
fn opacity(node: roxmltree::Node) -> f32 {
    let parse = |v: &str| {
        let v = v.trim().trim_end_matches("!important").trim();
        let n = match v.strip_suffix('%') {
            Some(percent) => percent.trim().parse::<f32>().ok().map(|p| p / 100.0),
            None => v.parse::<f32>().ok(),
        };
        n.filter(|n| n.is_finite()).map(|n| n.clamp(0.0, 1.0))
    };
    node.ancestors()
        .filter(roxmltree::Node::is_element)
        .map(|a| {
            let styled =
                declarations(a.attribute("style").unwrap_or_default()).filter(|(p, _)| p.trim() == "opacity").filter_map(|(_, v)| parse(v)).last();
            match styled {
                Some(_) if a == node => 1.0,
                Some(styled) => styled,
                None => a.attribute("opacity").and_then(parse).unwrap_or(1.0),
            }
        })
        .product()
}

/// The most `<font>` hints written as rules.
#[cfg(feature = "svg-html")]
const MAX_FONT_HINTS: usize = 1_024;

/// Rules for the hints of the `<font>` elements in `doc` (`face`, `color`, `size`), which a browser
/// takes and the painter doesn't (draw.io sets its labels' fonts with them): of no specificity
/// (`:where(…)`), so any rule of the drawing's overrides them, as it overrides a hint. Each value
/// is quoted or parsed, never pasted, so a drawing can't write rules of its own here; one that
/// doesn't parse is left out.
#[cfg(feature = "svg-html")]
fn font_hints(doc: &roxmltree::Document) -> String {
    const SIZES: [&str; 7] = ["x-small", "small", "medium", "large", "x-large", "xx-large", "xxx-large"];
    let mut rules = std::collections::BTreeSet::new();
    for font in doc.descendants().filter(|n| n.has_tag_name((XHTML_NS, "font"))) {
        if rules.len() >= MAX_FONT_HINTS {
            break;
        }
        if let Some(face) = font.attribute("face").filter(|f| f.len() <= 4096) {
            let names: Vec<String> = face.split(',').map(str::trim).filter(|n| !n.is_empty()).map(css_string).collect();
            if !names.is_empty() {
                rules.insert(format!(":where(font[face={}]){{font-family:{}}}", css_string(face), names.join(",")));
            }
        }
        if let Some((value, c)) = font.attribute("color").and_then(|v| Some((v, v.trim().parse::<svgtypes::Color>().ok()?))) {
            let colour = format!("rgba({},{},{},{})", c.red, c.green, c.blue, f32::from(c.alpha) / 255.0);
            rules.insert(format!(":where(font[color={}]){{color:{colour}}}", css_string(value)));
        }
        if let Some((value, size)) = font.attribute("size").and_then(|v| Some((v, font_size(v)?))) {
            rules.insert(format!(":where(font[size={}]){{font-size:{}}}", css_string(value), SIZES.get(size.saturating_sub(1)).unwrap_or(&"medium")));
        }
    }
    rules.into_iter().collect()
}

/// A `<font size>`, 1 to 7: a number, or `+n` or `-n` from 3, clamped; `None` when it isn't one.
#[cfg(feature = "svg-html")]
fn font_size(value: &str) -> Option<usize> {
    let v = value.trim();
    let (relative, digits) = match v.as_bytes().first()? {
        b'+' => (Some(1i64), v.get(1..)?),
        b'-' => (Some(-1i64), v.get(1..)?),
        _ => (None, v),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // At most 7 after clamping, so it fits usize.
    let n = digits.parse::<i64>().unwrap_or(i64::MAX).min(1_000);
    let size = match relative {
        Some(sign) => 3i64.saturating_add(sign.saturating_mul(n)),
        None => n,
    };
    usize::try_from(size.clamp(1, 7)).ok()
}

/// The root's viewport, its percentages' measure, as it says it: its `viewBox`, else its own
/// `width` and `height`; `None` when it says neither, usvg then sizing the drawing by what it draws.
#[cfg(feature = "svg-html")]
fn root_viewport(root: roxmltree::Node) -> Option<(f32, f32)> {
    let view_box = root.attribute("viewBox").and_then(|v| v.parse::<svgtypes::ViewBox>().ok()).filter(|v| v.w > 0.0 && v.h > 0.0);
    // Within f32: a viewBox's numbers are a drawing's.
    if let Some(v) = view_box {
        return Some((v.w as f32, v.h as f32));
    }
    let absolute = |name: &str| {
        let l = root.attribute(name)?.parse::<svgtypes::Length>().ok()?;
        (l.unit != svgtypes::LengthUnit::Percent).then(|| length(root, name, 0.0)).flatten().filter(|n| *n > 0.0)
    };
    Some((absolute("width")?, absolute("height")?))
}

/// `node`'s length `name` in user units, a percentage of `of` (the side of the viewport it
/// measures), in the units usvg takes (96 pixels an inch, `em` its nearest `font-size`); `None`
/// when it has none or it doesn't parse.
#[cfg(feature = "svg-html")]
fn length(node: roxmltree::Node, name: &str, of: f32) -> Option<f32> {
    let l = node.attribute(name)?.parse::<svgtypes::Length>().ok()?;
    // A length's number is an f64 for svgtypes, an f32 for usvg.
    let n = l.number as f32;
    use svgtypes::LengthUnit as U;
    let font_size = || node.ancestors().find_map(|a| a.attribute("font-size")?.parse::<svgtypes::Length>().ok()).map_or(12.0, absolute);
    let user = match l.unit {
        U::Percent => n * of / 100.0,
        U::Em => n * font_size(),
        U::Ex => n * font_size() / 2.0,
        _ => absolute(l),
    };
    user.is_finite().then_some(user)
}

/// An absolute length in user units (a relative one as if it were pixels).
#[cfg(feature = "svg-html")]
fn absolute(l: svgtypes::Length) -> f32 {
    use svgtypes::LengthUnit as U;
    let n = l.number as f32;
    match l.unit {
        U::In => n * 96.0,
        U::Cm => n * 96.0 / 2.54,
        U::Mm => n * 96.0 / 25.4,
        U::Pt => n * 96.0 / 72.0,
        U::Pc => n * 96.0 / 6.0,
        _ => n,
    }
}

/// `node`'s `transform`, or none when it doesn't parse.
#[cfg(feature = "svg-html")]
fn transform_of(node: roxmltree::Node) -> Transform {
    match node.attribute("transform").map(str::parse::<svgtypes::Transform>) {
        Some(Ok(t)) => Transform::from_row(t.a as f32, t.b as f32, t.c as f32, t.d as f32, t.e as f32, t.f as f32),
        _ => Transform::identity(),
    }
}

/// The transform from `view_box`'s user units to `size`, by `aspect`, as SVG fits a viewport.
#[cfg(feature = "svg-html")]
fn fitted(view_box: svgtypes::ViewBox, aspect: svgtypes::AspectRatio, size: (f32, f32)) -> Transform {
    use svgtypes::Align;
    let (x, y, w, h) = (view_box.x as f32, view_box.y as f32, view_box.w as f32, view_box.h as f32);
    if !(w > 0.0 && h > 0.0) {
        return Transform::identity();
    }
    let (sx, sy) = (size.0 / w, size.1 / h);
    let (ax, ay) = match aspect.align {
        Align::None => return Transform::from_row(sx, 0.0, 0.0, sy, -x * sx, -y * sy),
        Align::XMinYMin => (0.0, 0.0),
        Align::XMidYMin => (0.5, 0.0),
        Align::XMaxYMin => (1.0, 0.0),
        Align::XMinYMid => (0.0, 0.5),
        Align::XMidYMid => (0.5, 0.5),
        Align::XMaxYMid => (1.0, 0.5),
        Align::XMinYMax => (0.0, 1.0),
        Align::XMidYMax => (0.5, 1.0),
        Align::XMaxYMax => (1.0, 1.0),
    };
    let s = if aspect.slice { sx.max(sy) } else { sx.min(sy) };
    Transform::from_row(s, 0.0, 0.0, s, -x * s + (size.0 - w * s) * ax, -y * s + (size.1 - h * s) * ay)
}

/// Where the label `fo` is: the transform from its box to the root's user units, and its box's
/// width and height. Through a stack of viewports, as SVG places it: each nested `<svg>`'s `x`,
/// `y`, `width` and `height` (`100%` when left out), `viewBox` and `preserveAspectRatio`, and
/// between them the ancestors' `transform`s, the root's own left out as usvg leaves it out.
/// Lengths in percent are of the nearest viewport, `root` (the root's) at first. `None` for a box
/// with nothing inside, or one that can't be placed.
#[cfg(feature = "svg-html")]
fn placed(fo: roxmltree::Node, root: (f32, f32)) -> Option<(Transform, f32, f32)> {
    let mut chain: Vec<roxmltree::Node> = fo.ancestors().skip(1).filter(roxmltree::Node::is_element).collect();
    chain.reverse();
    let (mut place, mut viewport) = (Transform::identity(), root);
    for svg in chain.iter().skip(1) {
        place = place.pre_concat(transform_of(*svg));
        if !svg_element(*svg, "svg") {
            continue;
        }
        let (x, y) = (length(*svg, "x", viewport.0).unwrap_or(0.0), length(*svg, "y", viewport.1).unwrap_or(0.0));
        let (w, h) = (length(*svg, "width", viewport.0).unwrap_or(viewport.0), length(*svg, "height", viewport.1).unwrap_or(viewport.1));
        place = place.pre_translate(x, y);
        let view_box = svg.attribute("viewBox").and_then(|v| v.parse::<svgtypes::ViewBox>().ok()).filter(|v| v.w > 0.0 && v.h > 0.0);
        viewport = match view_box {
            Some(view_box) => {
                let aspect = svg.attribute("preserveAspectRatio").and_then(|v| v.parse().ok()).unwrap_or_default();
                place = place.pre_concat(fitted(view_box, aspect, (w, h)));
                (view_box.w as f32, view_box.h as f32)
            }
            None => (w, h),
        };
    }
    place = place.pre_concat(transform_of(fo));
    let (x, y) = (length(fo, "x", viewport.0).unwrap_or(0.0), length(fo, "y", viewport.1).unwrap_or(0.0));
    let (w, h) = (length(fo, "width", viewport.0)?, length(fo, "height", viewport.1)?);
    let place = place.pre_translate(x, y);
    let finite = [place.sx, place.kx, place.ky, place.sy, place.tx, place.ty].iter().all(|v| v.is_finite());
    (finite && w > 0.0 && h > 0.0).then_some((place, w, h))
}

/// `s`, escaped for XML text or an attribute value in double quotes, onto `out`.
#[cfg(feature = "svg-html")]
fn escaped(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

/// `s` as a CSS string, quoted: nothing in it ends the string or the declaration.
#[cfg(feature = "svg-html")]
fn css_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => out.push_str(&format!("\\{:x} ", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `node`'s font and colour attributes (`font-family`, `font-size`, `font-weight`,
/// `font-style`, `color`), as CSS declarations: each parsed or quoted, never pasted, so a
/// drawing can't write declarations of its own here. One that doesn't parse is left out.
#[cfg(feature = "svg-html")]
fn presentation(node: roxmltree::Node) -> String {
    let mut out = String::new();
    // A list longer than this is no font's: left out, rather than written out at its length.
    let families = node.attribute("font-family").filter(|v| v.len() <= 4096).and_then(|v| svgtypes::parse_font_families(v).ok());
    if let Some(families) = families {
        let names: Vec<String> = families
            .iter()
            .map(|f| match f {
                svgtypes::FontFamily::Named(name) => css_string(name),
                generic => generic.to_string(),
            })
            .collect();
        if !names.is_empty() {
            out.push_str(&format!("font-family:{};", names.join(",")));
        }
    }
    if let Some(size) = node.attribute("font-size").and_then(|v| v.parse::<svgtypes::Length>().ok()) {
        let n = size.number;
        if n.is_finite() && n >= 0.0 {
            match size.unit {
                svgtypes::LengthUnit::Percent => out.push_str(&format!("font-size:{n}%;")),
                svgtypes::LengthUnit::Em => out.push_str(&format!("font-size:{n}em;")),
                svgtypes::LengthUnit::Ex => out.push_str(&format!("font-size:{n}ex;")),
                _ => out.push_str(&format!("font-size:{}px;", absolute(size))),
            }
        }
    }
    const WEIGHTS: &[&str] = &["normal", "bold", "bolder", "lighter", "100", "200", "300", "400", "500", "600", "700", "800", "900"];
    if let Some(weight) = node.attribute("font-weight").map(str::trim).filter(|w| WEIGHTS.contains(w)) {
        out.push_str(&format!("font-weight:{weight};"));
    }
    if let Some(style) = node.attribute("font-style").map(str::trim).filter(|s| ["normal", "italic", "oblique"].contains(s)) {
        out.push_str(&format!("font-style:{style};"));
    }
    if let Some(c) = node.attribute("color").and_then(|v| v.trim().parse::<svgtypes::Color>().ok()) {
        out.push_str(&format!("color:rgba({},{},{},{});", c.red, c.green, c.blue, f32::from(c.alpha) / 255.0));
    }
    out
}

/// The copy of the element `node` that opens around a label, named `name`, its `style` before
/// `extra`, onto `out`.
#[cfg(feature = "svg-html")]
fn copy_of(node: roxmltree::Node, name: &str, extra: &str, out: &mut Markup) -> Result<(), Past> {
    out.raw("<")?;
    out.raw(name)?;
    for attribute in ["id", "class"] {
        if let Some(value) = node.attribute(attribute) {
            out.raw(" ")?;
            out.raw(attribute)?;
            out.raw("=\"")?;
            out.escaped(value)?;
            out.raw("\"")?;
        }
    }
    out.raw(" style=\"")?;
    out.escaped(&presentation(node))?;
    out.escaped(node.attribute("style").unwrap_or_default())?;
    out.raw(";")?;
    out.raw(extra)?;
    out.raw("\">")
}

/// A label's markup as it is written out, in what is left of the labels' markup budget: what would
/// take it past is refused before it is written, so the markup never holds more than the budget.
#[cfg(feature = "svg-html")]
struct Markup {
    out: String,
    room: usize,
}

#[cfg(feature = "svg-html")]
impl Markup {
    /// Whether `more` bytes still fit.
    fn fits(&self, more: usize) -> Result<(), Past> {
        match self.out.len().saturating_add(more) > self.room {
            true => Err(Past::Budget(format!("past {} MiB of markup", MAX_LABEL_MARKUP >> 20))),
            false => Ok(()),
        }
    }

    fn raw(&mut self, s: &str) -> Result<(), Past> {
        self.fits(s.len())?;
        self.out.push_str(s);
        Ok(())
    }

    /// `s`, escaped as [`escaped`] does, its length known before it is written.
    fn escaped(&mut self, s: &str) -> Result<(), Past> {
        let length = s.bytes().map(|b| match b {
            b'&' => 5,
            b'<' | b'>' => 4,
            b'"' => 6,
            _ => 1,
        });
        self.fits(length.fold(0usize, usize::saturating_add))?;
        escaped(s, &mut self.out);
        Ok(())
    }
}

/// The label `fo`, `width` × `height`, written out, within what is left of `spent`'s budgets, and
/// counted in it: its markup, and whether its overflow is visible.
#[cfg(feature = "svg-html")]
fn written_label(fo: roxmltree::Node, width: f32, height: f32, spent: &mut Spent) -> Result<(String, bool), Past> {
    let visible = fo.attribute("overflow").is_some_and(|v| v.trim() == "visible")
        || declarations(fo.attribute("style").unwrap_or_default()).any(|(p, v)| p.trim() == "overflow" && v.trim().starts_with("visible"));
    let mut chain: Vec<roxmltree::Node> = fo.ancestors().skip(1).filter(roxmltree::Node::is_element).collect();
    chain.reverse();
    let mut label =
        Spent { markup: spent.markup, text: spent.text, elements: spent.elements.saturating_add(u64::try_from(chain.len()).unwrap_or(u64::MAX)) };
    affordable(&label)?;
    let mut out = Markup { out: String::new(), room: MAX_LABEL_MARKUP.saturating_sub(spent.markup) };
    // An `<svg>`'s copy is a `div`: an element named `svg` would be taken for a drawing.
    fn name<'a>(node: &roxmltree::Node<'a, '_>) -> &'a str {
        if svg_element(*node, "svg") { "div" } else { node.tag_name().name() }
    }
    for ancestor in &chain {
        copy_of(*ancestor, name(ancestor), "display:contents !important", &mut out)?;
    }
    let overflow = if visible { "visible" } else { "hidden" };
    let box_style = format!(
        "display:block !important;position:absolute !important;left:0 !important;top:0 !important;width:{width}px !important;\
         height:{height}px !important;margin:0 !important;box-sizing:border-box !important;overflow:{overflow} !important"
    );
    copy_of(fo, "foreignObject", &box_style, &mut out)?;
    for child in fo.children() {
        written(child, Some(XHTML_NS), 1, &mut label, &mut out)?;
    }
    out.raw("</foreignObject>")?;
    for ancestor in chain.iter().rev() {
        out.raw("</")?;
        out.raw(name(ancestor))?;
        out.raw(">")?;
    }
    *spent = Spent { markup: spent.markup.saturating_add(out.out.len()), ..label };
    Ok((out.out, visible))
}

/// Whether the text and the elements `spent` counts are within the labels' budgets; their markup
/// [`Markup`] keeps within its own.
#[cfg(feature = "svg-html")]
fn affordable(spent: &Spent) -> Result<(), Past> {
    if spent.text > MAX_LABEL_TEXT {
        return Err(Past::Budget(format!("past {} MiB of text", MAX_LABEL_TEXT >> 20)));
    }
    if spent.elements > MAX_LABEL_ELEMENTS {
        return Err(Past::Budget(format!("past {MAX_LABEL_ELEMENTS} elements")));
    }
    Ok(())
}

/// The elements left out of a label's markup, in any namespace, since the painter would load a
/// stylesheet or a document for them, and asks for them without saying which they are: only fonts
/// and pictures are answered ([`html`]), as data. A `<style>`'s text joins the drawing's
/// stylesheets instead, read for the painter without `@import`.
#[cfg(feature = "svg-html")]
const LOADING: &[&str] = &["style", "link", "iframe", "frame", "frameset", "object", "embed", "base", "portal"];

/// `node`, `depth` levels below its label's `<foreignObject>`, inside an element in the
/// namespace `parent`, written out from the parsed tree onto `out`, counted in `spent`: elements
/// in the namespaces the drawing binds, declared where they change; attributes without a
/// namespace; text, its entities expanded; each escaped. Comments and processing instructions
/// are left out, and the elements [`LOADING`] names.
#[cfg(feature = "svg-html")]
fn written(node: roxmltree::Node, parent: Option<&str>, depth: usize, spent: &mut Spent, out: &mut Markup) -> Result<(), Past> {
    if node.is_text() {
        let text = node.text().unwrap_or_default();
        spent.text = spent.text.saturating_add(text.len());
        affordable(spent)?;
        return out.escaped(text);
    }
    if !node.is_element() {
        return Ok(());
    }
    if depth > MAX_LABEL_DEPTH {
        return Err(Past::Nesting);
    }
    if LOADING.contains(&node.tag_name().name()) {
        return Ok(());
    }
    spent.elements = spent.elements.saturating_add(1);
    affordable(spent)?;
    let (name, namespace) = (node.tag_name().name(), node.tag_name().namespace());
    out.raw("<")?;
    out.raw(name)?;
    if namespace != parent {
        out.raw(" xmlns=\"")?;
        out.escaped(namespace.unwrap_or_default())?;
        out.raw("\"")?;
    }
    for attribute in node.attributes().filter(|a| a.namespace().is_none()) {
        out.raw(" ")?;
        out.raw(attribute.name())?;
        out.raw("=\"")?;
        out.escaped(attribute.value())?;
        out.raw("\"")?;
    }
    out.raw(">")?;
    for child in node.children() {
        written(child, namespace, depth.saturating_add(1), spent, out)?;
    }
    out.raw("</")?;
    out.raw(name)?;
    out.raw(">")
}

/// The text of `doc` without what follows the child a browser takes in each `<switch>` (draw.io's
/// "[Not supported by viewer]" after its label, and its "Text is not SVG - cannot display" after
/// an empty `requiredFeatures` group), or `None` when there is none: usvg would draw them. The
/// child taken loses its `requiredFeatures`, which usvg would refuse it for. A `<switch>` whose
/// child taken or fallback comes from an entity keeps its fallbacks: its text is the entity's
/// declaration, which other elements may come from too.
#[cfg(feature = "svg-html")]
fn without_fallbacks(doc: &roxmltree::Document, text: &str) -> Option<String> {
    let body = doc.root_element().range();
    let in_body = |r: &std::ops::Range<usize>| body.start <= r.start && r.end <= body.end;
    let mut cut: Vec<std::ops::Range<usize>> = Vec::new();
    for switch in doc.descendants().filter(|n| svg_element(*n, "switch")) {
        let Some(child) = taken(switch) else { continue };
        // `next_siblings` starts at the node itself.
        let fallbacks: Vec<std::ops::Range<usize>> = child.next_siblings().skip(1).filter(roxmltree::Node::is_element).map(|n| n.range()).collect();
        let features = child.attribute_node("requiredFeatures").map(|a| a.range());
        let ranges: Vec<std::ops::Range<usize>> = fallbacks.into_iter().chain(features).collect();
        if !in_body(&child.range()) || !ranges.iter().all(|r| in_body(r) && r.start < r.end) {
            continue;
        }
        cut.extend(ranges);
    }
    if cut.is_empty() {
        return None;
    }
    cut.sort_by_key(|r| r.start);
    let (mut kept, mut at) = (String::with_capacity(text.len()), 0);
    for range in cut {
        // One inside another already cut.
        if range.start < at {
            continue;
        }
        kept.push_str(text.get(at..range.start)?);
        at = range.end;
    }
    kept.push_str(text.get(at..)?);
    Some(kept)
}

/// What usvg will repeat of a document as it turns it into a tree, counted before it does, from
/// the root down the way usvg goes: what each `use` repeats is counted again; so are a clip path,
/// a mask, a filter or a paint server for each element that refers to one (usvg converts one
/// anew for each element when its units are the element's bounding box, the default for masks
/// and filters), and a marker for each vertex it marks. Resources are walked once and their
/// weight remembered, so the walk itself costs at most [`MAX_ELEMENTS`] visits.
struct Scan<'a, 'input> {
    ids: HashMap<&'a str, roxmltree::Node<'a, 'input>>,
    /// The rules of the drawing's stylesheets that refer to something, and the rules in all.
    css: Vec<CssRef<'a>>,
    rules: u64,
    /// Each resource's weight, once walked.
    weights: HashMap<roxmltree::NodeId, Weight>,
    /// The uses and resources being walked: one that refers back to itself repeats nothing, as
    /// usvg skips it.
    walking: Vec<roxmltree::NodeId>,
    visited: u64,
    /// Content counted, repeats included.
    weight: Weight,
    has_text: bool,
    /// What laying out its `<text>`s costs, in [`MAX_LAYOUT`]'s units.
    layout: u64,
    /// Links to other files or the network, which usvg doesn't follow.
    links: HashSet<&'a str>,
}

/// Bytes of content, and characters of text among them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Weight {
    bytes: u64,
    text: u64,
}

/// What an element inherits that refers to something: its paint and its markers.
#[derive(Clone, Copy, Default)]
struct Inherited<'a> {
    fill: Option<&'a str>,
    stroke: Option<&'a str>,
    markers: [Option<&'a str>; 3],
}

/// The properties an element sets that refer to something: the inherited ones (`None` leaves
/// them as inherited, `Some(None)` sets them to no reference), and those that aren't.
#[derive(Default)]
struct Properties<'a> {
    fill: Option<Option<&'a str>>,
    stroke: Option<Option<&'a str>>,
    markers: [Option<Option<&'a str>>; 3],
    once: Vec<&'a str>,
}

/// A rule of a stylesheet that sets a property to a reference.
struct CssRef<'a> {
    /// The classes it applies to, or `None` for any element (a selector that isn't a list of
    /// classes).
    classes: Option<Vec<&'a str>>,
    property: &'a str,
    value: &'a str,
}

/// Why a scan refused a document.
type Refused = String;

/// Elements that aren't drawn where they are, only when something refers to them.
const UNDRAWN: &[&str] = &[
    "clipPath",
    "defs",
    "desc",
    "filter",
    "linearGradient",
    "marker",
    "mask",
    "metadata",
    "pattern",
    "radialGradient",
    "script",
    "style",
    "symbol",
    "title",
];

impl<'a, 'input> Scan<'a, 'input> {
    fn of(doc: &'a roxmltree::Document<'input>) -> Result<Self, Refused> {
        let root = doc.root_element();
        let mut scan = Scan {
            ids: HashMap::new(),
            css: Vec::new(),
            rules: 0,
            weights: HashMap::new(),
            walking: Vec::new(),
            visited: 0,
            weight: Weight::default(),
            has_text: false,
            layout: 0,
            links: HashSet::new(),
        };
        for node in doc.descendants() {
            if let Some(pi) = node.pi()
                && pi.target == "xml-stylesheet"
            {
                scan.links.insert(pi.value.unwrap_or("xml-stylesheet"));
            }
            if !node.is_element() {
                continue;
            }
            if let Some(id) = node.attribute("id") {
                scan.ids.entry(id).or_insert(node);
            }
            if node.tag_name().name() == "style"
                && let Some(text) = node.text()
            {
                scan.stylesheet(text);
            }
        }
        // Each reference a stylesheet sets is matched on every element, as a rule is.
        scan.rules = scan.rules.max(u64::try_from(scan.css.len()).unwrap_or(u64::MAX));
        scan.element(root, Inherited::default(), false, 0)?;
        Ok(scan)
    }

    /// The rules of the stylesheet `text`, as far as they refer to something: every `url(…)`
    /// in it, with the property it sets and the selector of its rule.
    fn stylesheet(&mut self, text: &'a str) {
        self.rules = self.rules.saturating_add(u64::try_from(text.matches('{').count()).unwrap_or(u64::MAX));
        // A font face is a link usvg doesn't follow; where labels are drawn they fetch its `data:`
        // URLs, and its others are counted as any URL is.
        let rules: &[&str] = if cfg!(feature = "svg-html") { &["@import"] } else { &["@import", "@font-face"] };
        for rule in rules {
            if text.contains(rule) {
                self.links.insert(rule);
            }
        }
        let mut from = 0;
        while let Some(at) = text.get(from..).and_then(|rest| rest.find("url(")).and_then(|at| at.checked_add(from)) {
            from = at.saturating_add(4);
            let before = text.get(..at).unwrap_or_default();
            let declaration_start = before.rfind(['{', ';']).map_or(0, |i| i.saturating_add(1));
            let declaration = before.get(declaration_start..).unwrap_or_default();
            let Some((property, _)) = declaration.split_once(':') else { continue };
            let end = text.get(at..).and_then(|rest| rest.find([';', '}'])).map_or(text.len(), |e| at.saturating_add(e));
            let value = text.get(at..end).unwrap_or_default();
            let block = before.rfind('{').unwrap_or(0);
            let selector_start = text.get(..block).and_then(|b| b.rfind(['}', '{'])).map_or(0, |i| i.saturating_add(1));
            let selector = text.get(selector_start..block).unwrap_or_default();
            self.css.push(CssRef { classes: classes(selector), property: property.trim(), value });
        }
    }

    /// Walks `node`, an element drawn with what it `inherits`, counting it and what it repeats.
    fn element(&mut self, node: roxmltree::Node<'a, 'input>, inherits: Inherited<'a>, in_text: bool, depth: usize) -> Result<(), Refused> {
        self.visited = self.visited.saturating_add(1);
        if self.visited > MAX_ELEMENTS {
            return Err(format!("it has more than {MAX_ELEMENTS} elements, counting what its uses repeat"));
        }
        if self.visited.saturating_mul(self.rules) > MAX_STYLE {
            return Err(format!("its stylesheets' {} rules are too many for its {} elements", self.rules, self.visited));
        }
        if depth > MAX_DEPTH {
            return Err(format!("it nests more than {MAX_DEPTH} deep, counting its uses and references"));
        }
        let name = node.tag_name().name();
        let own = node.attributes().fold(1u64, |sum, a| sum.saturating_add(u64::try_from(a.value().len()).unwrap_or(u64::MAX)));
        self.add(Weight { bytes: own, text: 0 }, 1)?;
        // Presentation attributes, then the stylesheets' rules, then the style attribute.
        let mut set = Properties::default();
        for attribute in node.attributes() {
            self.property(&mut set, attribute.name(), attribute.value(), false);
        }
        if !self.css.is_empty() {
            let class = node.attribute("class").unwrap_or_default();
            let matched: Vec<(&'a str, &'a str)> = self
                .css
                .iter()
                .filter(|r| r.classes.as_ref().is_none_or(|cs| class.split_ascii_whitespace().any(|c| cs.contains(&c))))
                .map(|r| (r.property, r.value))
                .collect();
            for (property, value) in matched {
                self.property(&mut set, property, value, true);
            }
        }
        if let Some(style) = node.attribute("style") {
            for declaration in style.split(';') {
                if let Some((property, value)) = declaration.split_once(':') {
                    self.property(&mut set, property.trim(), value, false);
                }
            }
        }
        let here = Inherited {
            fill: set.fill.unwrap_or(inherits.fill),
            stroke: set.stroke.unwrap_or(inherits.stroke),
            markers: [0, 1, 2].map(|i| set.markers[i].unwrap_or(inherits.markers[i])),
        };
        if matches!(name, "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" | "text" | "tspan" | "textPath") {
            for id in [here.fill, here.stroke].into_iter().flatten() {
                self.refer(id, 1, depth)?;
            }
        }
        if matches!(name, "path" | "line" | "polyline" | "polygon") {
            let vertices = vertices(node);
            for id in here.markers.into_iter().flatten() {
                self.refer(id, vertices, depth)?;
            }
        }
        for id in set.once {
            self.refer(id, 1, depth)?;
        }
        let href = node.attribute("href");
        match (name, href.map(|h| h.trim().strip_prefix('#'))) {
            ("use", Some(Some(id))) => {
                if let Some(target) = self.ids.get(id).copied()
                    && !self.walking.contains(&target.id())
                {
                    self.walking.push(target.id());
                    let walked = self.element(target, here, in_text, depth.saturating_add(1));
                    self.walking.pop();
                    walked?;
                }
            }
            ("tref", Some(Some(id))) => {
                let length = self.ids.get(id).map_or(0, |target| text_length(*target));
                self.add(Weight { bytes: length, text: length }, 1)?;
            }
            // A picture's link is followed by usvg's resolver, which counts the ones it refuses.
            ("image" | "a" | "script", _) => {}
            (_, Some(Some(id))) => self.refer(id, 1, depth)?,
            ("use" | "feImage" | "tref" | "textPath", Some(None)) => {
                self.links.extend(href);
            }
            _ => {}
        }
        if name == "text" && !in_text {
            self.layout = self.layout.saturating_add(layout(node, &self.ids));
        }
        let in_text = in_text || name == "text";
        self.has_text |= in_text;
        for child in node.children() {
            if child.is_text() {
                let length = u64::try_from(child.text().map_or(0, str::len)).unwrap_or(u64::MAX);
                self.add(Weight { bytes: length, text: if in_text { length } else { 0 } }, 1)?;
            } else if child.is_element() && !UNDRAWN.contains(&child.tag_name().name()) {
                self.element(child, here, in_text, depth.saturating_add(1))?;
            }
        }
        Ok(())
    }

    /// Notes in `set` what `property: value` refers to, if anything.
    /// `stylesheet` says it comes from a `<style>`, whose `data:` URLs the labels fetch, where
    /// they are drawn: those aren't links not followed.
    fn property(&mut self, set: &mut Properties<'a>, property: &str, value: &'a str, stylesheet: bool) {
        if !value.contains("url(") {
            // A colour or `none` ends a reference inherited; `inherit` keeps it.
            let cleared = if value.trim() == "inherit" { None } else { Some(None) };
            match property {
                "fill" => set.fill = cleared.or(set.fill),
                "stroke" => set.stroke = cleared.or(set.stroke),
                "marker" => set.markers = [cleared.or(set.markers[0]), cleared.or(set.markers[1]), cleared.or(set.markers[2])],
                "marker-start" => set.markers[0] = cleared.or(set.markers[0]),
                "marker-mid" => set.markers[1] = cleared.or(set.markers[1]),
                "marker-end" => set.markers[2] = cleared.or(set.markers[2]),
                _ => {}
            }
            return;
        }
        let mut ids = Vec::new();
        for url in value.split("url(").skip(1) {
            let url = url.trim_start().trim_start_matches(['"', '\'']);
            let url = url.get(..url.find(['"', '\'', ')']).unwrap_or(url.len())).unwrap_or(url);
            match url.strip_prefix('#') {
                Some(id) => ids.push(id.trim_end()),
                None if cfg!(feature = "svg-html") && stylesheet && url.trim_start().starts_with("data:") => {}
                None => {
                    self.links.insert(url);
                }
            }
        }
        let first = ids.first().copied();
        match property {
            "fill" => set.fill = Some(first),
            "stroke" => set.stroke = Some(first),
            "marker" => set.markers = [Some(first); 3],
            "marker-start" => set.markers[0] = Some(first),
            "marker-mid" => set.markers[1] = Some(first),
            "marker-end" => set.markers[2] = Some(first),
            _ => set.once.extend(ids),
        }
    }

    /// Counts the resource `id` refers to, converted `times`, from an element `depth` deep.
    fn refer(&mut self, id: &'a str, times: u64, depth: usize) -> Result<(), Refused> {
        let Some(node) = self.ids.get(id).copied() else { return Ok(()) };
        if let Some(weight) = self.weights.get(&node.id()).copied() {
            return self.add(weight, times);
        }
        if self.walking.contains(&node.id()) {
            return Ok(());
        }
        // Walked once, counted once; the other times by its weight.
        let before = self.weight;
        self.walking.push(node.id());
        let walked = self.element(node, Inherited::default(), false, depth.saturating_add(1));
        self.walking.pop();
        walked?;
        let weight = Weight { bytes: self.weight.bytes.saturating_sub(before.bytes), text: self.weight.text.saturating_sub(before.text) };
        self.weights.insert(node.id(), weight);
        self.add(weight, times.saturating_sub(1))
    }

    /// Adds `weight`, `times`.
    fn add(&mut self, weight: Weight, times: u64) -> Result<(), Refused> {
        self.weight.bytes = self.weight.bytes.saturating_add(weight.bytes.saturating_mul(times));
        self.weight.text = self.weight.text.saturating_add(weight.text.saturating_mul(times));
        if self.weight.bytes > MAX_EXPANDED {
            return Err(format!("it comes to more than {} MiB once what its uses, markers and references repeat is repeated", MAX_EXPANDED >> 20));
        }
        Ok(())
    }

    /// Characters of text, repeats included.
    fn text(&self) -> u64 {
        self.weight.text
    }
}

/// The classes `selector` lists, or `None` when it is anything else (which may then match any
/// element).
fn classes(selector: &str) -> Option<Vec<&str>> {
    let mut out = Vec::new();
    for part in selector.split(',') {
        let class = part.trim().strip_prefix('.')?;
        if class.is_empty() || !class.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return None;
        }
        out.push(class);
    }
    Some(out)
}

/// At most how many vertices the shape `node` has, so how many markers it takes: a path's and a
/// polyline's data need at least two bytes a vertex.
fn vertices(node: roxmltree::Node) -> u64 {
    let data = match node.tag_name().name() {
        "path" => node.attribute("d"),
        "polyline" | "polygon" => node.attribute("points"),
        _ => return 2,
    };
    u64::try_from(data.map_or(0, str::len) / 2).unwrap_or(u64::MAX).saturating_add(1)
}

/// What laying out the `<text>` element `text` costs, in [`MAX_LAYOUT`]'s units: its characters
/// (a `tref`'s too, found in `ids`) squared, and its spans (the elements in it) times its
/// characters, times [`SPAN_COST`].
fn layout(text: roxmltree::Node, ids: &HashMap<&str, roxmltree::Node>) -> u64 {
    let (mut characters, mut spans) = (0u64, 1u64);
    for node in text.descendants().skip(1) {
        if node.is_text() {
            characters = characters.saturating_add(u64::try_from(node.text().map_or(0, str::len)).unwrap_or(u64::MAX));
        } else if node.is_element() {
            spans = spans.saturating_add(1);
            if node.tag_name().name() == "tref"
                && let Some(target) = node.attribute("href").and_then(|h| ids.get(h.trim().trim_start_matches('#')))
            {
                characters = characters.saturating_add(text_length(*target));
            }
        }
    }
    characters.saturating_mul(characters).saturating_add(spans.saturating_mul(characters).saturating_mul(SPAN_COST))
}

/// The length of the text in `node`.
fn text_length(node: roxmltree::Node) -> u64 {
    node.descendants().filter(|n| n.is_text()).map(|n| u64::try_from(n.text().map_or(0, str::len)).unwrap_or(u64::MAX)).fold(0, u64::saturating_add)
}

/// The most links told apart, to say how many weren't followed.
const MAX_LINKS: usize = 10_000;

/// What was left out of a drawing and its embedded ones as they parsed, said with its pages.
#[derive(Default)]
struct Notes {
    /// Links to other files or the network, by a hash of each, up to [`MAX_LINKS`].
    links: Mutex<HashSet<u64>>,
    /// Embedded pictures larger than [`MAX_PICTURE`].
    pictures: AtomicUsize,
    /// Embedded drawings broken, or nested too deep.
    drawings: AtomicUsize,
    /// Fallback fonts looked for.
    fallbacks: AtomicUsize,
    /// Why the text was left out, if it was.
    text: Mutex<Option<String>>,
    /// HTML labels a browser would draw that this build doesn't, or couldn't.
    undrawn: AtomicUsize,
    /// HTML labels left out past a limit, by why.
    left_out_labels: Mutex<Vec<(String, usize)>>,
}

impl Notes {
    /// Notes `links` as not followed.
    fn linked<'l>(&self, links: impl IntoIterator<Item = &'l str>) {
        let mut seen = self.links.lock().unwrap_or_else(PoisonError::into_inner);
        for link in links {
            if seen.len() >= MAX_LINKS {
                return;
            }
            let mut hasher = std::hash::DefaultHasher::new();
            std::hash::Hash::hash(link, &mut hasher);
            seen.insert(std::hash::Hasher::finish(&hasher));
        }
    }

    /// Notes an HTML label as left out, for `why`.
    #[cfg(feature = "svg-html")]
    fn left_out_label(&self, why: &str) {
        let mut left_out = self.left_out_labels.lock().unwrap_or_else(PoisonError::into_inner);
        match left_out.iter_mut().find(|(w, _)| w == why) {
            Some((_, n)) => *n = n.saturating_add(1),
            None => left_out.push((why.to_string(), 1)),
        }
    }

    fn left_out_text(&self, why: String) {
        self.text.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert(why);
    }

    /// What the reader should know, in words; with `labels`, of the HTML labels too, which a
    /// list's tile doesn't draw.
    fn warnings(&self, labels: bool) -> Vec<String> {
        let count = |n: &AtomicUsize| n.load(Ordering::Relaxed);
        let plural = |n: usize, one: &str, many: &str| if n == 1 { format!("1 {one} was") } else { format!("{n} {many} were") };
        let mut out = Vec::new();
        if let Some(why) = self.text.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            out.push(format!("its text was left out: {why}"));
        }
        match self.links.lock().unwrap_or_else(PoisonError::into_inner).len() {
            0 => {}
            n => out.push(format!("{} not followed", plural(n, "link to another file or the network", "links to other files or the network"))),
        }
        match count(&self.pictures) {
            0 => {}
            n => out.push(format!(
                "{} left out",
                plural(n, &format!("picture of more than {} MiB", MAX_PICTURE >> 20), &format!("pictures of more than {} MiB", MAX_PICTURE >> 20))
            )),
        }
        match count(&self.drawings) {
            0 => {}
            n => out.push(format!(
                "{} left out (broken, or more than {MAX_NESTED} deep)",
                plural(n, "drawing embedded in it", "drawings embedded in it")
            )),
        }
        match count(&self.undrawn) {
            n if n == 0 || !labels => {}
            n => out.push(format!("{} not drawn", plural(n, "HTML label (<foreignObject>)", "HTML labels (<foreignObject>)"))),
        }
        for (why, n) in self.left_out_labels.lock().unwrap_or_else(PoisonError::into_inner).iter().filter(|_| labels) {
            out.push(format!("{} left out, {why}", plural(*n, "HTML label (<foreignObject>)", "HTML labels (<foreignObject>)")));
        }
        if count(&self.fallbacks) > MAX_FALLBACKS {
            out.push(format!(
                "fonts were looked through for missing characters {MAX_FALLBACKS} times, then no more: some characters may show as boxes"
            ));
        }
        out
    }
}

/// usvg's options for a drawing `depth` SVGs down, with `fonts` for its text: nothing is read
/// from files or the network, and what is left out is told to `notes`.
fn options_for(notes: &Notes, depth: usize, fonts: Arc<fontdb::Database>) -> usvg::Options<'_> {
    let fallback = usvg::FontResolver::default_fallback_selector();
    usvg::Options {
        resources_dir: None,
        fontdb: fonts,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(move |mime, data, _| embedded(mime, data, depth, notes)),
            resolve_string: Box::new(move |href, _| {
                notes.linked([href]);
                None
            }),
        },
        font_resolver: usvg::FontResolver {
            select_font: usvg::FontResolver::default_font_selector(),
            select_fallback: Box::new(move |c, used, db| match notes.fallbacks.fetch_add(1, Ordering::Relaxed) < MAX_FALLBACKS {
                true => fallback(c, used, db),
                false => None,
            }),
        },
        ..usvg::Options::default()
    }
}

/// A picture embedded as a `data:` URL, of type `mime`, known by its signature: a raster one up
/// to [`MAX_PICTURE`] decoded, or an SVG, parsed as the file's own is, up to [`MAX_NESTED`] deep.
fn embedded(mime: &str, data: Arc<Vec<u8>>, depth: usize, notes: &Notes) -> Option<usvg::ImageKind> {
    let kind = match imagesize::image_type(&data) {
        Ok(imagesize::ImageType::Png) => usvg::ImageKind::PNG,
        Ok(imagesize::ImageType::Jpeg) => usvg::ImageKind::JPEG,
        Ok(imagesize::ImageType::Gif) => usvg::ImageKind::GIF,
        Ok(imagesize::ImageType::Webp) => usvg::ImageKind::WEBP,
        _ if matches!(mime, "image/svg+xml" | "text/plain") => {
            let nested = depth.saturating_add(1);
            let tree = (nested <= MAX_NESTED).then(|| parse_at(&data, nested, notes).ok()).flatten().map(|(tree, _)| tree);
            if tree.is_none() {
                notes.drawings.fetch_add(1, Ordering::Relaxed);
            }
            return tree.map(usvg::ImageKind::SVG);
        }
        _ => return None,
    };
    let size = imagesize::blob_size(&data).ok()?;
    let bytes = u64::try_from(size.width).unwrap_or(u64::MAX).saturating_mul(u64::try_from(size.height).unwrap_or(u64::MAX)).saturating_mul(4);
    if bytes > MAX_PICTURE {
        notes.pictures.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    Some(kind(data))
}

/// The system's fonts, found once, by the first drawing with text: fontdb reads the names in
/// every font file, about 15 ms for macOS's (ADR-0002). The generic families (serif,
/// sans-serif…) are set to fonts this system has.
fn system_fonts() -> Arc<fontdb::Database> {
    static FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();
            let has = |db: &fontdb::Database, family: &str| db.faces().any(|f| f.families.iter().any(|(name, _)| name == family));
            let first = |db: &fontdb::Database, families: &[&str]| families.iter().find(|f| has(db, f)).map(|f| f.to_string());
            let any = db.faces().next().and_then(|f| f.families.first()).map(|(name, _)| name.clone());
            let sans = first(&db, &["Arial", "Helvetica", "Noto Sans", "DejaVu Sans", "Liberation Sans", "FreeSans"]).or(any);
            let serif = first(&db, &["Times New Roman", "Times", "Noto Serif", "DejaVu Serif", "Liberation Serif", "FreeSerif"]).or(sans.clone());
            let mono = first(&db, &["Courier New", "Menlo", "Noto Sans Mono", "DejaVu Sans Mono", "Liberation Mono", "FreeMono"]).or(sans.clone());
            let cursive = first(&db, &["Comic Sans MS", "Apple Chancery", "URW Chancery L"]).or(sans.clone());
            let fantasy = first(&db, &["Impact", "Papyrus"]).or(sans.clone());
            if let Some(f) = serif {
                db.set_serif_family(f);
            }
            if let Some(f) = sans {
                db.set_sans_serif_family(f);
            }
            if let Some(f) = mono {
                db.set_monospace_family(f);
            }
            if let Some(f) = cursive {
                db.set_cursive_family(f);
            }
            if let Some(f) = fantasy {
                db.set_fantasy_family(f);
            }
            Arc::new(db)
        })
        .clone()
}

impl Drawn {
    fn render(&self, request: Request) -> Result<Frame, String> {
        let (width, height) = self.size;
        let canvas = Dims { w: scaled(width, request.scale)?, h: scaled(height, request.scale)? };
        if canvas.w > MAX_SIDE || canvas.h > MAX_SIDE {
            return Err(format!("{} × {} pixels is too large to render", canvas.w, canvas.h));
        }
        // At most MAX_SIDE, so exact as f32 and as u32.
        let (w, h) = (canvas.w as f32, canvas.h as f32);
        let transform = Transform::from_scale(w / width, h / height);
        #[allow(unused_mut, reason = "its labels add to it, in a build that draws them")]
        let mut cost = Estimate::of(&self.tree, transform, canvas)?;
        // A list's tile is drawn without its labels: no worker is started for it.
        #[cfg(feature = "svg-html")]
        let labels = if request.tile { None } else { self.prepared() };
        #[cfg(feature = "svg-html")]
        let parts = labels.map(|labels| label_parts(labels, transform, canvas)).unwrap_or_default();
        // Each label's part painted, then drawn over the page; and as work, the part's pixels.
        #[cfg(feature = "svg-html")]
        for (_, _, size) in parts.iter().flatten() {
            let part = Dims { w: u64::from(size.0), h: u64::from(size.1) };
            cost.peak = cost.peak.saturating_add(part.bytes().saturating_mul(2));
            cost.work = cost.work.saturating_add(part.px());
        }
        if cost.peak > MAX_RENDER {
            return Err(format!("drawing it at {} × {} would take more than {} MiB", canvas.w, canvas.h, MAX_RENDER >> 20));
        }
        if cost.work > MAX_WORK {
            return Err(format!("drawing it at {} × {} would take too long ({} million pixels painted)", canvas.w, canvas.h, cost.work / 1_000_000));
        }
        let (cw, ch) = (canvas.w as u32, canvas.h as u32);
        let mut pixmap = Pixmap::new(cw, ch).ok_or_else(|| format!("no canvas of {cw} × {ch} pixels"))?;
        resvg::render(&self.tree, transform, &mut pixmap.as_mut());
        #[cfg(feature = "svg-html")]
        if let Some(labels) = labels {
            self.draw_labels(labels, &mut pixmap, &parts);
        }
        let warnings = self.notes.warnings(!request.tile);
        // tiny-skia's pixels are premultiplied RGBA, row by row, as a frame's are.
        Ok(Frame { request, width: cw, height: ch, rgba: Pixels::new(pixmap.take()), error: None, warnings, again: false })
    }
}

/// Where a label is drawn on a page: the part of the page it lands on (its top left), the transform
/// from its box's CSS pixels to that part, and the part's size.
#[cfg(feature = "svg-html")]
type Part = ((i32, i32), [f64; 6], (u32, u32));

#[cfg(feature = "svg-html")]
impl Drawn {
    /// Its labels laid out, by the first render that asks, in a worker of their own; `None` when
    /// it has none, they were left out, or their worker went.
    fn prepared(&self) -> Option<&Prepared> {
        let labelled = self.labels.as_ref()?;
        if labelled.failed.get().is_some() {
            return None;
        }
        labelled.prepared.get_or_init(|| prepared(&labelled.found, &self.notes)).as_ref()
    }

    /// Its `labels`, each replayed into its part of the page and drawn over `pixmap` at its
    /// opacity: over the whole drawing, not among its shapes. If their worker goes, none of them
    /// is drawn, now or again, and why is told.
    fn draw_labels(&self, labels: &Prepared, pixmap: &mut Pixmap, parts: &[Option<Part>]) {
        use resvg::tiny_skia::{IntSize, PixmapPaint};
        let (placed, painted) = labels;
        for (index, part) in parts.iter().enumerate() {
            let (Some(((x, y), to, size)), Some((_, opacity))) = (part, placed.get(index)) else { continue };
            let rgba = match painted.paint(index, *to, *size) {
                Ok(Some(rgba)) => rgba,
                Ok(None) => continue,
                Err(why) => {
                    if let Some(labelled) = &self.labels {
                        labelled.failed.get_or_init(|| {
                            for _ in placed {
                                self.notes.left_out_label(&why);
                            }
                            why
                        });
                    }
                    return;
                }
            };
            let Some(drawn) = IntSize::from_wh(size.0, size.1).and_then(|size| Pixmap::from_vec(rgba, size)) else { continue };
            let paint = PixmapPaint { opacity: *opacity, ..PixmapPaint::default() };
            pixmap.draw_pixmap(*x, *y, drawn.as_ref(), &paint, Transform::identity(), None);
        }
    }
}

/// The part of a `canvas` drawn through `transform` each of `labels` lands on: `None` for a label
/// left out, or landing nowhere on it. Only that part is painted, so no label costs more pixels
/// than the page, however large it is shown.
#[cfg(feature = "svg-html")]
fn label_parts(labels: &Prepared, transform: Transform, canvas: Dims) -> Vec<Option<Part>> {
    let (placed, painted) = labels;
    placed
        .iter()
        .enumerate()
        .map(|(index, (place, _))| {
            let [x0, y0, x1, y1] = painted.region(index)?;
            let to_page = transform.pre_concat(*place);
            // Within f32: a region is at most some 25,000 pixels a side.
            let mut corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)].map(|(x, y)| resvg::tiny_skia::Point::from_xy(x as f32, y as f32));
            to_page.map_points(&mut corners);
            let (lx, ly) = corners.iter().fold((f32::INFINITY, f32::INFINITY), |(x, y), p| (x.min(p.x), y.min(p.y)));
            let (hx, hy) = corners.iter().fold((f32::NEG_INFINITY, f32::NEG_INFINITY), |(x, y), p| (x.max(p.x), y.max(p.y)));
            // At most MAX_SIDE, so exact as f32.
            let (lx, ly, hx, hy) = (lx.floor().max(0.0), ly.floor().max(0.0), hx.ceil().min(canvas.w as f32), hy.ceil().min(canvas.h as f32));
            if !(lx.is_finite() && ly.is_finite() && hx > lx && hy > ly) {
                return None;
            }
            let to = to_page.post_translate(-lx, -ly);
            // Within the canvas, at most MAX_SIDE: exact as i32 and u32.
            Some(((lx as i32, ly as i32), [to.sx, to.ky, to.kx, to.sy, to.tx, to.ty].map(f64::from), ((hx - lx) as u32, (hy - ly) as u32)))
        })
        .collect()
}

/// `size` at `scale`, rounded, at least one pixel.
fn scaled(size: f32, scale: f32) -> Result<u64, String> {
    let scaled = (f64::from(size) * f64::from(scale)).round();
    if !(scale > 0.0 && scaled.is_finite() && scaled <= 1e15) {
        return Err(format!("can't render at scale {scale}"));
    }
    // In range: checked just above.
    Ok((scaled as u64).max(1))
}

/// Pixels across and down.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Dims {
    w: u64,
    h: u64,
}

impl Dims {
    /// A rectangle `w` × `h`, `margin` more each way, in whole pixels.
    fn of(w: f32, h: f32, margin: u64) -> Self {
        // `as` saturates, and makes NaN 0.
        Self { w: (w.ceil() as u64).saturating_add(margin), h: (h.ceil() as u64).saturating_add(margin) }
    }

    fn px(self) -> u64 {
        self.w.saturating_mul(self.h)
    }

    fn bytes(self) -> u64 {
        self.px().saturating_mul(4)
    }

    fn min(self, other: Self) -> Self {
        Self { w: self.w.min(other.w), h: self.h.min(other.h) }
    }

    fn times(self, n: u64) -> Self {
        Self { w: self.w.saturating_mul(n), h: self.h.saturating_mul(n) }
    }

    /// The bounds of this rectangle once `transform` turns it.
    fn turned(self, transform: Transform) -> Self {
        let (w, h) = (self.w as f32, self.h as f32);
        Self::of(transform.sx.abs() * w + transform.kx.abs() * h, transform.ky.abs() * w + transform.sy.abs() * h, 1)
    }
}

/// What a render will cost: the most bytes it holds at once, and the pixels it paints.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Cost {
    peak: u64,
    work: u64,
}

/// A render's [`Cost`], estimated from the tree the way resvg 0.49 renders it: a pixmap the
/// canvas's size, a layer for each group drawn apart (one with opacity, a clip path, a mask, a
/// filter or a blend mode), the size of what it holds but within five times the canvas each way;
/// for a clip path or a mask, pixmaps the layer's size; for a filter, the layer again for each
/// of its primitives; for a pattern, a tile of its own size, however large; for an embedded
/// SVG, a pixmap the size of the one it is drawn into. A layer is held while what is in it is
/// drawn, so nested ones add up.
#[derive(Default)]
struct Estimate {
    work: u64,
    steps: u64,
    nested: usize,
}

impl Estimate {
    fn of(tree: &usvg::Tree, transform: Transform, canvas: Dims) -> Result<Cost, String> {
        let mut estimate = Estimate::default();
        let drawn = estimate.children(tree.root(), transform, canvas, canvas.times(5));
        if estimate.steps > MAX_STEPS {
            return Err(format!("the drawing is too complex to draw: more than {MAX_STEPS} shapes, patterns repeated"));
        }
        estimate.work = estimate.work.saturating_add(canvas.px());
        Ok(Cost { peak: canvas.bytes().saturating_add(drawn), work: estimate.work })
    }

    /// Counts a node visited: `false` once there have been too many, to stop.
    fn step(&mut self) -> bool {
        self.steps = self.steps.saturating_add(1);
        self.steps <= MAX_STEPS
    }

    fn paint(&mut self, pixels: u64) {
        self.work = self.work.saturating_add(pixels);
    }

    /// The most `group`'s children hold at once, drawn with `transform` into a pixmap `target`
    /// big, layers bounded to `region`.
    fn children(&mut self, group: &usvg::Group, transform: Transform, target: Dims, region: Dims) -> u64 {
        let mut most = 0;
        for node in group.children() {
            let held = match node {
                usvg::Node::Group(g) => self.group(g, transform, target, region),
                usvg::Node::Path(p) => self.path(p, transform, target, region),
                usvg::Node::Image(i) => self.image(i, transform, target),
                usvg::Node::Text(t) => self.group(t.flattened(), transform, target, region),
            };
            most = most.max(held);
        }
        most
    }

    fn group(&mut self, group: &usvg::Group, transform: Transform, target: Dims, region: Dims) -> u64 {
        if !self.step() {
            return 0;
        }
        let transform = transform.pre_concat(group.transform());
        if !group.should_isolate() {
            return self.children(group, transform, target, region);
        }
        // A filtered group's layer is drawn unturned, and turned as it is drawn back.
        let (transform, region, margin) = if group.filters().is_empty() {
            (transform, region, 4)
        } else {
            let (sx, sy) = (transform.sx.hypot(transform.ky), transform.kx.hypot(transform.sy));
            let rest = Transform::from_row(transform.sx / sx, transform.ky / sx, transform.kx / sy, transform.sy / sy, 0.0, 0.0);
            let Some(unturn) = rest.invert() else { return 0 };
            (unturn.pre_concat(transform), region.turned(unturn), 0)
        };
        let Some(bounds) = group.layer_bounding_box().transform(transform) else { return 0 };
        let layer = Dims::of(bounds.width(), bounds.height(), margin).min(region);
        self.paint(layer.px().saturating_mul(2));
        let mut most = self.children(group, transform, layer, region);
        for filter in group.filters() {
            most = most.max(self.filter(filter, transform, layer));
        }
        if let Some(clip) = group.clip_path() {
            most = most.max(self.clip(clip, transform, layer));
        }
        if let Some(mask) = group.mask() {
            most = most.max(self.mask(mask, transform, layer, region));
        }
        layer.bytes().saturating_add(most)
    }

    fn path(&mut self, path: &usvg::Path, transform: Transform, target: Dims, region: Dims) -> u64 {
        if !self.step() || !path.is_visible() {
            return 0;
        }
        let covered = path.stroke_bounding_box().transform(transform).map_or(0, |r| Dims::of(r.width(), r.height(), 2).min(target).px());
        self.paint(covered.saturating_add(u64::try_from(path.data().points().len()).unwrap_or(u64::MAX)));
        if let Some(stroke) = path.stroke()
            && let Some(dashes) = stroke.dasharray()
        {
            // tiny-skia makes at most a million dashes of a path; each is a path of its own.
            let interval: f32 = dashes.iter().sum();
            let bounds = path.data().bounds();
            let length = 3.0 * bounds.width().hypot(bounds.height()) * path.data().len() as f32;
            let count = if interval > 0.0 { (length / interval * dashes.len() as f32).min(1e6) } else { 0.0 };
            self.paint((count as u64).saturating_mul(16));
        }
        let fill = path.fill().map_or(0, |f| self.pattern(f.paint(), transform, region));
        let stroke = path.stroke().map_or(0, |s| self.pattern(s.paint(), transform, region));
        fill.max(stroke)
    }

    /// What painting with `paint` holds: for a pattern, its tile and what is drawn in it.
    fn pattern(&mut self, paint: &usvg::Paint, transform: Transform, region: Dims) -> u64 {
        let usvg::Paint::Pattern(pattern) = paint else { return 0 };
        let (sx, sy) = transform.pre_concat(pattern.transform()).get_scale();
        let rect = pattern.rect();
        let tile = Dims { w: (rect.width() * sx).round() as u64, h: (rect.height() * sy).round() as u64 };
        if tile.px() == 0 {
            return 0;
        }
        self.paint(tile.px());
        tile.bytes().saturating_add(self.children(pattern.root(), Transform::from_scale(sx, sy), tile, region))
    }

    fn image(&mut self, image: &usvg::Image, transform: Transform, target: Dims) -> u64 {
        if !self.step() || !image.is_visible() {
            return 0;
        }
        match image.kind() {
            usvg::ImageKind::SVG(tree) => {
                if self.nested > MAX_NESTED {
                    return 0;
                }
                self.paint(target.px().saturating_mul(2));
                self.nested += 1;
                let inside = self.children(tree.root(), transform, target, target.times(5));
                self.nested -= 1;
                target.bytes().saturating_add(inside)
            }
            _ => {
                // Decoded at its own size, then drawn: the decoder's buffer and the pixmap.
                let size = image.size();
                let decoded = Dims::of(size.width(), size.height(), 0);
                let drawn = usvg::Rect::from_xywh(0.0, 0.0, size.width(), size.height())
                    .and_then(|r| r.transform(transform))
                    .map_or(0, |r| Dims::of(r.width(), r.height(), 2).min(target).px());
                self.paint(decoded.px().saturating_mul(4).saturating_add(drawn));
                decoded.bytes().saturating_mul(2)
            }
        }
    }

    /// What a clip path holds, applied to a layer `layer` big: a pixmap that size, what is drawn
    /// in it (another for each clipped group in it), the clip path's own clip path, and a mask.
    fn clip(&mut self, clip: &usvg::ClipPath, transform: Transform, layer: Dims) -> u64 {
        if !self.step() {
            return 0;
        }
        self.paint(layer.px().saturating_mul(3));
        let drawn = self.clip_children(clip.root(), transform.pre_concat(clip.transform()), layer);
        let inner = clip.clip_path().map_or(0, |c| self.clip(c, transform, layer));
        layer.bytes().saturating_add(drawn.max(inner).max(layer.px()))
    }

    fn clip_children(&mut self, group: &usvg::Group, transform: Transform, layer: Dims) -> u64 {
        let mut most = 0;
        for node in group.children() {
            let held = match node {
                // Drawn with a region of one pixel: whatever a pattern holds stays small.
                usvg::Node::Path(p) => self.path(p, transform, layer, Dims { w: 1, h: 1 }),
                usvg::Node::Text(t) => self.clip_children(t.flattened(), transform, layer),
                usvg::Node::Group(g) => {
                    if !self.step() {
                        return most;
                    }
                    let transform = transform.pre_concat(g.transform());
                    match g.clip_path() {
                        Some(clip) => {
                            self.paint(layer.px());
                            let drawn = self.clip_children(g, transform, layer);
                            let clipped = self.clip(clip, transform, layer);
                            layer.bytes().saturating_add(drawn.max(clipped))
                        }
                        None => self.clip_children(g, transform, layer),
                    }
                }
                usvg::Node::Image(_) => 0,
            };
            most = most.max(held);
        }
        most
    }

    /// What a mask holds, applied to a layer `layer` big: a pixmap that size and a mask of it,
    /// what is drawn in it, and the mask's own mask.
    fn mask(&mut self, mask: &usvg::Mask, transform: Transform, layer: Dims, region: Dims) -> u64 {
        if !self.step() || mask.root().children().is_empty() {
            return 0;
        }
        self.paint(layer.px().saturating_mul(4));
        let drawn = self.children(mask.root(), transform, layer, region);
        let inner = mask.mask().map_or(0, |m| self.mask(m, transform, layer, region));
        layer.bytes().saturating_add(layer.px()).saturating_add(drawn.max(inner).max(layer.px()))
    }

    /// What a filter holds, applied to a layer `layer` big: the layer cropped to its region, a
    /// result that big for each primitive, a primitive's inputs and buffers, and an `feImage`'s
    /// drawing. Its work is each primitive's over its region: a convolution's kernel, a
    /// morphology's radius and turbulence's octaves times as much.
    fn filter(&mut self, filter: &usvg::filter::Filter, transform: Transform, layer: Dims) -> u64 {
        if !self.step() {
            return 0;
        }
        let region = filter.rect().transform(transform).map_or(layer, |r| Dims::of(r.width(), r.height(), 1).min(layer));
        let (sx, sy) = transform.get_scale();
        let mut drawn = 0u64;
        for primitive in filter.primitives() {
            if !self.step() {
                break;
            }
            use usvg::filter::Kind;
            let factor = match primitive.kind() {
                Kind::ConvolveMatrix(fe) => u64::from(fe.matrix().columns()).saturating_mul(u64::from(fe.matrix().rows())),
                Kind::Morphology(fe) => {
                    let side = |r: f32, s: f32, most: u64| ((r * s).round().max(0.0) as u64).min(most).saturating_mul(2).saturating_add(1);
                    side(fe.radius_x().get(), sx, region.w).saturating_mul(side(fe.radius_y().get(), sy, region.h))
                }
                // Measured: an octave over a pixel takes as long as 20 pixels filled.
                Kind::Turbulence(fe) => u64::from(fe.num_octaves()).max(1).saturating_mul(20),
                Kind::GaussianBlur(_) | Kind::DropShadow(_) | Kind::DiffuseLighting(_) | Kind::SpecularLighting(_) => 16,
                Kind::Merge(fe) => u64::try_from(fe.inputs().len()).unwrap_or(u64::MAX).saturating_add(1),
                Kind::Image(fe) => {
                    drawn = drawn.max(self.children(fe.root(), Transform::from_scale(sx, sy), region, region));
                    2
                }
                _ => 4,
            };
            self.paint(region.px().saturating_mul(factor));
        }
        let primitives = u64::try_from(filter.primitives().len()).unwrap_or(u64::MAX);
        region.bytes().saturating_mul(primitives.saturating_add(6)).saturating_add(drawn)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// A drawing `width` × `height` px, of `body`.
    fn svg(width: u32, height: u32, body: &str) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{width}" height="{height}">{body}</svg>"#
        )
    }

    fn gzipped(bytes: &[u8]) -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(bytes).unwrap();
        gz.finish().unwrap()
    }

    fn open(bytes: &[u8]) -> Result<Drawing, String> {
        let drawn = guarded_parse(bytes)?;
        Ok(Drawing(Arc::new(Document { encoded: Encoded::Memory(bytes.to_vec()), size: drawn.size })))
    }

    fn render(drawing: &Drawing, scale: f32) -> Frame {
        drawing.renderer().unwrap().render(Request { page: 0, scale, tag: 7, ..Default::default() })
    }

    /// The pixel at `x`, `y` of `frame`.
    fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * frame.width + x) * 4) as usize;
        frame.rgba[at..at + 4].try_into().unwrap()
    }

    /// Which labels are drawn is found in one walk of the drawing, whatever its shape: 50,000
    /// labels a `<switch>` doesn't take, each asking which child it takes, would be 2.5 billion
    /// steps. Labels a drawn `<use>` repeats are counted, each target once; those in a definition,
    /// not drawn, are not.
    #[test]
    fn labels_are_found_in_one_walk() {
        let not_taken = "<foreignObject width='1' height='1' requiredExtensions='x'/>".repeat(50_000);
        let group = format!("<defs><g id='g'>{}</g><use href='#g'/></defs>", "<foreignObject width='1' height='1'/>".repeat(1_000));
        let used = "<use xlink:href='#g'/>".repeat(500);
        let text = svg(10, 10, &format!("<switch>{not_taken}<rect width='1' height='1'/></switch>{group}{used}"));
        let started = std::time::Instant::now();
        let warnings = guarded_parse(text.as_bytes()).unwrap().notes.warnings(true);
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
        assert_eq!(warnings, ["500000 HTML labels (<foreignObject>) were not drawn"]);
    }

    /// The labels `<use>`s show are counted once an element, however its targets nest: 4,000
    /// targets each holding the next inside a `defs`, with 100 elements beside it, each used, were
    /// 800 million visits counted afresh for each target.
    #[test]
    fn labels_through_uses_are_counted_once_an_element() {
        let filler = "<rect/>".repeat(100);
        let (mut nested, mut used) = (String::from("<foreignObject width='1' height='1'/>"), String::new());
        for i in (0..4000).rev() {
            nested = format!("<g id='t{i}'><defs>{filler}{nested}</defs></g>");
            used.push_str(&format!("<use href='#t{i}'/>"));
        }
        let text = svg(10, 10, &format!("<defs>{nested}</defs>{used}"));
        // Counted before usvg parses it (which then refuses it, copying every use).
        let doc = xml(&text).unwrap();
        assert!(Scan::of(&doc).is_ok());
        let started = std::time::Instant::now();
        let counted = label_count(&doc);
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "{:?}", started.elapsed());
        // Each target shows nothing: what it holds is in a `defs`.
        assert_eq!(counted, 0);
    }

    /// A label's markup is refused before it is written past its budget: an attribute escaping to
    /// more than is left is never appended.
    #[cfg(feature = "svg-html")]
    #[test]
    fn markup_past_its_budget_is_refused_before_it_is_written() {
        let mut out = Markup { out: String::from("<i"), room: 10 };
        assert!(out.escaped(r#"a"b"#).is_ok());
        assert_eq!(out.out, "<ia&quot;b");
        assert!(out.escaped("\"").is_err() && out.raw("xy").is_err());
        assert_eq!((out.out.as_str(), out.out.len()), ("<ia&quot;b", 10));
        let mut out = Markup { out: String::new(), room: MAX_LABEL_MARKUP };
        assert!(out.escaped(&"\"".repeat((MAX_LABEL_MARKUP / 6) + 1)).is_err());
        assert_eq!(out.out.capacity(), 0);
    }

    /// `<font>`'s hints are rules of no specificity, each value quoted or parsed: a face is
    /// a list of names, quoted, so nothing in one ends its rule; a size 1 to 7, or from 3.
    #[cfg(feature = "svg-html")]
    #[test]
    fn a_fonts_hints_are_rules_that_write_nothing_of_their_own() {
        let text = svg(
            10,
            10,
            r#"<foreignObject width="1" height="1"><div xmlns="http://www.w3.org/1999/xhtml"><font face='Courier New, "x"}*{color:red'/><font size="+9"/><font size="-1"/><font size="x"/></div></foreignObject>"#,
        );
        let hints = font_hints(&xml(&text).unwrap());
        let face = r#":where(font[face="Courier New, \"x\"}*{color:red"]){font-family:"Courier New","\"x\"}*{color:red"}"#;
        assert!(hints.contains(face), "{hints}");
        assert!(
            hints.contains(r#":where(font[size="+9"]){font-size:xxx-large}"#) && hints.contains(r#":where(font[size="-1"]){font-size:small}"#),
            "{hints}"
        );
        assert!(!hints.contains(r#"size="x""#), "{hints}");
        assert_eq!(
            [font_size("3"), font_size("+1"), font_size("-9"), font_size("12"), font_size(""), font_size("+")],
            [Some(3), Some(4), Some(1), Some(7), None, None]
        );
    }

    /// A label stays on a page however large it is drawn, only its part of the page painted: the
    /// spike lost a label as wide as the drawing past some 8,192 pixels of label.
    #[cfg(feature = "svg-html")]
    #[test]
    fn a_label_is_drawn_on_a_page_over_six_thousand_pixels_wide() {
        // Its overflow visible, and a box of nothing reaching 4,000 units past the drawing each way:
        // what it paints over is 1.3 million pixels across, of which the page is 6,400.
        let wide = r#"<foreignObject width="100%" height="100%" overflow="visible"><div xmlns="http://www.w3.org/1999/xhtml" style="position:relative">
            <div style="position:absolute;left:-4000px;top:-4000px;width:8040px;height:8020px"/><div style="margin-left:38px;width:2px;height:20px;background:blue"/></div></foreignObject>"#;
        let frame = render(&open(svg(40, 20, wide).as_bytes()).unwrap(), 160.0);
        assert_eq!((frame.width, frame.error.as_deref()), (6400, None));
        assert_eq!([pixel(&frame, 6300, 100), pixel(&frame, 6000, 100)], [[0, 0, 255, 255], [0, 0, 0, 0]]);
    }

    /// A list's tile is drawn without its labels (its fallbacks still cut), warns of nothing about
    /// them, and starts no worker; the same drawing rendered as a page draws them, in a worker of
    /// its own.
    #[cfg(feature = "svg-html")]
    #[test]
    fn a_tile_draws_no_labels_and_starts_no_worker() {
        let fallback = "<rect width='40' height='10' fill='green'/>";
        let label = r#"<foreignObject width="40" height="10"><div xmlns="http://www.w3.org/1999/xhtml" style="width:100%;height:100%;background:blue"/></foreignObject>"#;
        let drawing = open(svg(40, 20, &format!("<rect width='40' height='20' fill='red'/><switch>{label}{fallback}</switch>")).as_bytes()).unwrap();
        let mut renderer = Renderer { document: drawing.0.clone(), parsed: Cache::new() };
        let mut rendered = |tile: bool| renderer.render(Request { page: 0, scale: 1.0, tag: 7, tile, ..Default::default() });
        let tile = rendered(true);
        assert_eq!((pixel(&tile, 5, 5), pixel(&tile, 5, 15), tile.warnings.as_slice()), ([255, 0, 0, 255], [255, 0, 0, 255], &[][..]));
        // Its labels found, but not laid out: no worker.
        let prepared = |renderer: &Renderer| renderer.parsed.get().unwrap().as_ref().unwrap().labels.as_ref().unwrap().prepared.get().is_some();
        assert!(!prepared(&renderer), "a worker started for a tile");
        let mut rendered = |tile: bool| renderer.render(Request { page: 0, scale: 1.0, tag: 7, tile, ..Default::default() });
        let page = rendered(false);
        assert_eq!((pixel(&page, 5, 5), pixel(&page, 5, 15)), ([0, 0, 255, 255], [255, 0, 0, 255]), "the label, and no fallback");
        assert!(prepared(&renderer), "no worker started for a page");
    }

    /// A label's part of the page is in the page's estimate, painted and then drawn over the page:
    /// a label as large as the page doubles what a 16,000 × 8,000 page holds, past 1 GiB.
    #[cfg(feature = "svg-html")]
    #[test]
    fn a_labels_part_of_the_page_is_in_its_estimate() {
        let labelled =
            open(svg(40, 20, r#"<foreignObject width="40" height="20"><div xmlns="http://www.w3.org/1999/xhtml"/></foreignObject>"#).as_bytes())
                .unwrap();
        let plain = open(svg(40, 20, "<rect width='40' height='20'/>").as_bytes()).unwrap();
        assert_eq!(render(&plain, 400.0).error, None);
        let error = render(&labelled, 400.0).error.unwrap_or_default();
        assert!(error.contains("would take more than 1024 MiB"), "{error}");
        assert_eq!(render(&labelled, 200.0).error, None);
    }

    /// A percentage of a drawing that says no size of its own is of the size usvg gives it from what
    /// it draws: a rect at (10, 20), 300 × 50, makes it 310 × 70.
    #[cfg(feature = "svg-html")]
    #[test]
    fn a_percentage_of_a_drawing_sized_by_what_it_draws_is_of_that_size() {
        let text = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect x="10" y="20" width="300" height="50"/><foreignObject width="100%" height="100%"/></svg>"#;
        let found = labels(text);
        assert_eq!((found[0].width, found[0].height), (310.0, 70.0));
    }

    /// The labels found in `text`, placed on the drawing.
    #[cfg(feature = "svg-html")]
    fn labels(text: &str) -> Vec<Label> {
        parse_at(text.as_bytes(), 0, &Notes::default()).unwrap().1.labels
    }

    /// A label is written out from the parsed tree, never copied from the file: its namespaces
    /// as the drawing binds them (a prefix declared on the root), its entities expanded, its
    /// text and attributes escaped, comments left out.
    #[cfg(feature = "svg-html")]
    #[test]
    fn a_label_is_written_out_from_the_tree() {
        let text = r#"<!DOCTYPE svg [<!ENTITY who "World &amp; all">]>
            <svg xmlns="http://www.w3.org/2000/svg" xmlns:h="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML" width="10" height="10">
            <foreignObject width="10" height="10"><h:div title='a "b" &lt;c&gt;'>Hello &who;<!-- not this --><![CDATA[ <i>raw</i> ]]><m:math/></h:div></foreignObject></svg>"#;
        let found = labels(text);
        let markup = &found[0].markup;
        let inner = r#"<div title="a &quot;b&quot; &lt;c&gt;">Hello World &amp; all &lt;i&gt;raw&lt;/i&gt; <math xmlns="http://www.w3.org/1998/Math/MathML"></math></div></foreignObject>"#;
        assert!(markup.contains(inner), "{markup}");
        assert!(!markup.contains("not this") && !markup.contains("<h:"), "{markup}");
    }

    /// A label is written inside copies of its context: its ancestors (the root's an `svg`
    /// made a `div`) with their ids, classes, styles and font and colour attributes as styles,
    /// shown as their contents; then the `<foreignObject>` itself, made its box. Each attribute is
    /// parsed or quoted: none writes a declaration of its own.
    #[cfg(feature = "svg-html")]
    #[test]
    fn a_label_is_written_inside_its_context() {
        let text = svg(
            40,
            20,
            r##"<g id="n1" class="node a" style="opacity: 1" font-family="'Courier New', monospace" font-size="2mm" font-weight="bold" font-style="italic" color="#f00">
            <foreignObject class="label" style="overflow: visible" x="1" y="2" width="20" height="10" font-family="'x;color:blue'" color="red;background:blue"><div xmlns="http://www.w3.org/1999/xhtml">a</div></foreignObject></g>"##,
        );
        let found = labels(&text);
        let label = &found[0];
        let g = r#"<g id="n1" class="node a" style="font-family:&quot;Courier New&quot;,monospace;font-size:7.5590553px;font-weight:bold;font-style:italic;color:rgba(255,0,0,1);opacity: 1;display:contents !important">"#;
        assert!(label.markup.starts_with(&format!(r#"<div style=";display:contents !important">{g}<foreignObject class="label" style="font-family:&quot;x;color:blue&quot;;overflow: visible;display:block !important;"#)), "{}", label.markup);
        assert!(
            label.markup.contains("width:20px !important;height:10px !important;") && label.markup.contains("overflow:visible !important"),
            "{}",
            label.markup
        );
        assert!(label.markup.ends_with("</foreignObject></g></div>"), "{}", label.markup);
        assert!(label.visible && (label.width, label.height) == (20.0, 10.0));
    }

    /// A label is placed through the stack of viewports: the root's `viewBox` fitted to its size,
    /// each nested `<svg>`'s `x`, `y`, `width`, `height`, `viewBox` and `preserveAspectRatio`, and
    /// the ancestors' transforms between them, but the root's own; percentages of the nearest
    /// viewport, and units at 96 pixels an inch.
    #[cfg(feature = "svg-html")]
    #[test]
    fn a_label_is_placed_through_the_viewports_it_is_in() {
        let at = |body: &str| {
            let text =
                format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 100 50" transform="scale(3)">{body}</svg>"#);
            let found = labels(&text);
            let l = &found[0];
            let mut corners = [resvg::tiny_skia::Point::from_xy(0.0, 0.0), resvg::tiny_skia::Point::from_xy(l.width, l.height)];
            l.place.map_points(&mut corners);
            (l.width, l.height, [corners[0].x, corners[0].y, corners[1].x, corners[1].y].map(|v| (v * 1000.0).round() / 1000.0))
        };
        let fo = r#"<foreignObject x="1" y="2" width="3" height="4"/>"#;
        // The root's viewBox doubles it.
        assert_eq!(at(fo), (3.0, 4.0, [2.0, 4.0, 8.0, 12.0]));
        assert_eq!(at(&format!(r#"<g transform="translate(10 5)">{fo}</g>"#)), (3.0, 4.0, [22.0, 14.0, 28.0, 22.0]));
        // A nested svg at (10, 10), 50 × 50, showing 10 × 10: five times larger.
        let nested = format!(r#"<svg x="10" y="10" width="50" height="50" viewBox="0 0 10 10">{fo}</svg>"#);
        assert_eq!(at(&nested), (3.0, 4.0, [30.0, 40.0, 60.0, 80.0]));
        // Showing 10 × 20 in 50 × 50, centred: 2.5 times larger, 12.5 in from the left.
        let centred = format!(r#"<svg width="50" height="50" viewBox="0 0 10 20">{fo}</svg>"#);
        assert_eq!(at(&centred), (3.0, 4.0, [30.0, 10.0, 45.0, 30.0]));
        let stretched = format!(r#"<svg width="50" height="50" viewBox="0 0 10 20" preserveAspectRatio="none">{fo}</svg>"#);
        assert_eq!(at(&stretched), (3.0, 4.0, [10.0, 10.0, 40.0, 30.0]));
        // Percentages: of the root's viewBox, then of a nested svg's own size.
        assert_eq!(at(r#"<foreignObject x="10%" width="100%" height="100%"/>"#), (100.0, 50.0, [20.0, 0.0, 220.0, 100.0]));
        assert_eq!(at(r#"<svg width="40" height="20"><foreignObject width="50%" height="50%"/></svg>"#), (20.0, 10.0, [0.0, 0.0, 40.0, 20.0]));
        assert_eq!(at(r#"<foreignObject x="1in" width="10mm" height="1pt"/>"#).0, 37.795277);
        assert!(labels(&svg(10, 10, r#"<foreignObject width="0" height="4"/><foreignObject width="4"/>"#)).is_empty());
    }

    #[test]
    fn an_svg_is_known_by_its_root_element() {
        let yes: &[&[u8]] = &[
            b"<svg xmlns='http://www.w3.org/2000/svg'/>",
            b"\xEF\xBB\xBF<svg>",
            b"  \n<?xml version='1.0' encoding='UTF-8'?>\n<!-- Created with Inkscape -->\n<svg\n width='1'>",
            b"<?xml version='1.0'?><!DOCTYPE svg PUBLIC '-//W3C//DTD SVG 1.1//EN' 'http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd'><svg>",
            b"<!DOCTYPE svg [ <!ENTITY ns_svg \"http://www.w3.org/2000/svg\"> <!-- a ]> in a comment --> <!ENTITY q '>]'> ]>\n<svg>",
            b"<?xml-stylesheet href='a.css'?><svg:svg xmlns:svg='http://www.w3.org/2000/svg'>",
            b"<svg/>",
        ];
        for head in yes {
            assert!(is_svg("x", head), "{}", String::from_utf8_lossy(head));
        }
        let no: &[&[u8]] = &[
            b"<html><body><svg></svg></body></html>",
            b"<!DOCTYPE html><html>",
            b"<?xml version='1.0'?><plist version='1.0'>",
            b"<svgfoo>",
            b"<:svg>",
            b"%PDF-1.7 <svg>",
            b"hello <svg>",
            b"\x89PNG\r\n\x1a\n",
            b"<rss><svg/></rss>",
        ];
        for head in no {
            assert!(!is_svg("x.svg", head), "{}", String::from_utf8_lossy(head));
        }
    }

    /// A head that ends before the root element does is an SVG if it is named as one.
    #[test]
    fn a_prolog_longer_than_the_head_goes_by_the_name() {
        let long = format!("<?xml version='1.0'?><!-- {} -->", "licence text ".repeat(100));
        let head = &long.as_bytes()[..HEAD as usize];
        assert_eq!(prolog(head), Root::Unseen);
        assert!(is_svg("a.svg", head) && is_svg("a.SVGZ", head) && !is_svg("a.xml", head) && !is_svg("a", head));
        for cut in [&b"<sv"[..], b"<?xml", b"<!-- open", b"<!DOCTYPE svg [ <!ENTITY a 'x'>", b""] {
            assert_eq!(prolog(cut), Root::Unseen, "{}", String::from_utf8_lossy(cut));
        }
        // UTF-16, which isn't read, by its name too.
        assert!(is_svg("a.svg", b"\xFF\xFE<\0s\0v\0g\0") && !is_svg("a.txt", b"\xFF\xFE<\0"));
    }

    #[test]
    fn an_svgz_is_known_by_what_it_unpacks_to() {
        let drawing = svg(10, 10, "<rect width='10' height='10'/>");
        assert!(is_svg("x", &gzipped(drawing.as_bytes())));
        assert!(!is_svg("x.svgz", &gzipped(b"<html><svg/></html>")));
        assert!(!is_svg("x.svgz", b"\x1f\x8b not gzip at all"));
        // Only the head of a large one: what it unpacks to is enough.
        let large = gzipped(svg(10, 10, &"<rect width='1' height='1'/>".repeat(10_000)).as_bytes());
        assert!(large.len() > HEAD as usize && is_svg("x", &large[..HEAD as usize]));
        let opened = open(&large).unwrap();
        assert_eq!(opened.size(0), Some((10.0, 10.0)));
    }

    #[test]
    fn a_drawing_is_one_page_rendered_at_its_scale() {
        // Red on the left half, blue on the right.
        let drawing =
            open(svg(40, 20, "<rect width='20' height='20' fill='red'/><rect x='20' width='20' height='20' fill='blue'/>").as_bytes()).unwrap();
        assert_eq!((drawing.pages(), drawing.size(0), drawing.size(1), drawing.natural(0)), (1, Some((40.0, 20.0)), None, false));
        for (scale, w, h) in [(1.0, 40, 20), (2.5, 100, 50), (0.1, 4, 2), (0.001, 1, 1)] {
            let frame = render(&drawing, scale);
            assert_eq!((frame.error.as_deref(), frame.width, frame.height, frame.rgba.len()), (None, w, h, (w * h * 4) as usize), "{scale}");
        }
        // Drawn again at each scale, not scaled: the edge between the halves stays sharp.
        let frame = render(&drawing, 10.0);
        assert_eq!((pixel(&frame, 199, 100), pixel(&frame, 200, 100)), ([255, 0, 0, 255], [0, 0, 255, 255]));
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(render(&drawing, scale).error.is_some_and(|e| e.contains("can't render at scale")), "{scale}");
        }
        let second = drawing.renderer().unwrap().render(Request { page: 1, scale: 1.0, tag: 7, ..Default::default() });
        assert!(second.error.is_some_and(|e| e.contains("one page")));
    }

    #[test]
    fn transparent_pixels_are_premultiplied() {
        let drawing = open(svg(1, 1, "<rect width='1' height='1' fill='#c86432' fill-opacity='0.5'/>").as_bytes()).unwrap();
        let [r, g, b, a] = pixel(&render(&drawing, 1.0), 0, 0);
        assert!(a.abs_diff(128) <= 1 && r.abs_diff(100) <= 1 && g.abs_diff(50) <= 1 && b.abs_diff(25) <= 1, "{:?}", [r, g, b, a]);
    }

    /// The size of a drawing that says none and draws nothing: usvg's default, in CSS pixels.
    const EMPTY: (f32, f32) = (100.0, 100.0);

    /// The size a drawing is shown at: its own, its viewBox's, or what it draws.
    #[test]
    fn a_drawing_without_a_size_is_as_large_as_what_it_draws() {
        let size = |text: &str| open(text.as_bytes()).map(|d| d.size(0).unwrap());
        let ns = "xmlns='http://www.w3.org/2000/svg'";
        assert_eq!(size(&format!("<svg {ns} width='3in' height='2cm'/>")).map(|(w, h)| (w.round(), h.round())), Ok((288.0, 76.0)));
        assert_eq!(size(&format!("<svg {ns} viewBox='0 0 24 12'/>")), Ok((24.0, 12.0)));
        assert_eq!(size(&format!("<svg {ns} width='48' viewBox='0 0 24 12'/>")), Ok((48.0, 24.0)));
        // Neither: from its origin to as far as it draws, both ways, as usvg sizes it.
        assert_eq!(size(&format!("<svg {ns}><rect x='10' y='20' width='300' height='50'/></svg>")), Ok((310.0, 70.0)));
        assert_eq!(size(&format!("<svg {ns} width='100%'><circle cx='50' cy='60' r='10'/></svg>")), Ok((60.0, 70.0)));
        assert_eq!(size(&format!("<svg {ns} width='40'><rect width='300' height='50'/></svg>")), Ok((300.0, 50.0)));
        assert_eq!(size(&format!("<svg {ns}/>")), Ok(EMPTY));
        let default = usvg::Options::default().default_size;
        assert_eq!((default.width(), default.height()), EMPTY, "usvg's own, for a relative size");
        // Drawn wholly at negative coordinates: nothing reaches past its origin.
        assert_eq!(size(&format!("<svg {ns}><rect x='-50' y='-50' width='10' height='10'/></svg>")), Ok(EMPTY));
        assert!(size(&format!("<svg {ns} width='0' height='10'/>")).is_err());
    }

    #[test]
    fn what_isnt_an_svg_drawing_is_refused_with_why() {
        let refused = |bytes: &[u8]| open(bytes).err().unwrap_or_default();
        assert!(refused(b"<html/>").contains("root element isn't <svg>"), "{}", refused(b"<html/>"));
        assert!(refused(b"<svg/>").contains("SVG namespace"));
        assert!(refused(b"<svg xmlns='http://www.w3.org/2000/svg'>").contains("not a well-formed SVG"));
        assert!(refused(b"\xFF\xFE<\0s\0").contains("UTF-8"));
        assert!(refused(b"\x1f\x8b\x08\0\0\0\0\0").contains("gzip is broken"));
        assert!(refused(b"").contains("not a well-formed SVG"));
    }

    /// A drawing on disk keeps no tree in its source: each renderer reads the file when it first
    /// renders, and keeps what it parsed for itself.
    #[test]
    fn a_drawing_on_disk_is_read_by_each_renderer_and_kept_by_it() {
        let path = std::env::temp_dir().join(format!("clview-svg-disk-{}.svg", std::process::id()));
        let filled = |colour: &str| svg(2, 2, &format!("<rect width='2' height='2' fill='{colour}'/>"));
        std::fs::write(&path, filled("#0a0a0a")).unwrap();
        let input = Input::open(path.to_str().unwrap()).unwrap();
        let Opened::Pages(drawing) = Svg.open(input, &Parsed::default()).unwrap() else { panic!("pages") };
        let mut first = drawing.renderer().unwrap();
        let red = |frame: Frame| frame.rgba.first().copied();
        assert_eq!(red(first.render(Request { page: 0, scale: 1.0, tag: 1, ..Default::default() })), Some(10));
        std::fs::write(&path, filled("#c8c8c8")).unwrap();
        assert_eq!(red(first.render(Request { page: 0, scale: 2.0, tag: 2, ..Default::default() })), Some(10));
        assert_eq!(red(drawing.renderer().unwrap().render(Request { page: 0, scale: 1.0, tag: 3, ..Default::default() })), Some(200));
        std::fs::remove_file(&path).unwrap();
        let gone = drawing.renderer().unwrap().render(Request { page: 0, scale: 1.0, tag: 4, ..Default::default() });
        assert!(gone.error.is_some_and(|e| e.starts_with("reading the file again: ")));
    }

    /// A 16 × 16 PNG, as a data URL.
    fn png_url(width: u32, height: u32) -> String {
        use base64::Engine;
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, width, height);
            encoder.set_color(png::ColorType::Rgba);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&vec![255; (width * height * 4) as usize]).unwrap();
        }
        format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png))
    }

    /// Nothing outside the file is read: a picture, a stylesheet, another drawing linked by a path
    /// or a URL is left out, and said once with how many.
    #[test]
    fn links_to_other_files_or_the_network_are_not_followed() {
        let dir = std::env::temp_dir().join(format!("clview-svg-links-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let red = dir.join("red.svg");
        std::fs::write(&red, svg(10, 10, "<rect width='10' height='10' fill='red'/>")).unwrap();
        let body = format!(
            "<image width='10' height='10' href='{red}'/><image x='0' width='10' height='10' xlink:href='file://{red}'/>\
             <image width='10' height='10' href='https://example.com/a.png'/><use href='{red}#r'/>\
             <rect x='10' width='10' height='10' fill='url({red}#p) blue'/>",
            red = red.display()
        );
        let text = format!("<?xml-stylesheet href='{}'?>{}", dir.join("a.css").display(), svg(20, 10, &body));
        let drawing = open(text.as_bytes()).unwrap();
        let frame = render(&drawing, 1.0);
        assert_eq!(frame.error, None);
        assert_eq!(pixel(&frame, 5, 5), [0, 0, 0, 0], "no picture read");
        assert_ne!(pixel(&frame, 15, 5), [255, 0, 0, 255], "not the linked pattern");
        assert_eq!(frame.warnings, ["6 links to other files or the network were not followed"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Pictures embedded in the drawing are drawn, up to their cap; a larger one is left out.
    #[test]
    fn an_embedded_picture_is_drawn_and_one_too_large_left_out() {
        let small = svg(16, 16, &format!("<image width='16' height='16' href='{}'/>", png_url(16, 16)));
        let frame = render(&open(small.as_bytes()).unwrap(), 1.0);
        assert_eq!((frame.error.as_deref(), pixel(&frame, 8, 8), frame.warnings.len()), (None, [255; 4], 0));
        // A PNG header that claims 5000 × 5000 pixels, 100 MB decoded.
        let large = svg(16, 16, &format!("<image width='16' height='16' href='{}'/>", png_url(5000, 5000)));
        let frame = render(&open(large.as_bytes()).unwrap(), 1.0);
        assert_eq!((frame.error.as_deref(), pixel(&frame, 8, 8)), (None, [0; 4]));
        assert_eq!(frame.warnings, ["1 picture of more than 64 MiB was left out"]);
    }

    /// An SVG embedded in an SVG is drawn, down to MAX_NESTED; deeper ones are left out.
    #[test]
    fn embedded_drawings_nest_only_so_deep() {
        use base64::Engine;
        let nest = |levels: usize| {
            let mut text = svg(8, 8, "<rect width='8' height='8' fill='lime'/>");
            for _ in 0..levels {
                let url = format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(&text));
                text = svg(8, 8, &format!("<image width='8' height='8' href='{url}'/>"));
            }
            render(&open(text.as_bytes()).unwrap(), 1.0)
        };
        let frame = nest(MAX_NESTED);
        assert_eq!((frame.error.as_deref(), pixel(&frame, 4, 4), frame.warnings.len()), (None, [0, 255, 0, 255], 0));
        let frame = nest(MAX_NESTED + 1);
        assert_eq!((frame.error.as_deref(), pixel(&frame, 4, 4)), (None, [0; 4]));
        assert_eq!(frame.warnings, ["1 drawing embedded in it was left out (broken, or more than 4 deep)"]);
    }

    /// Entities, the way to make a little text a great deal (billion laughs), are bounded before
    /// the XML parser expands them; a doctype that declares a few short ones is fine.
    #[test]
    fn entities_that_expand_too_far_are_refused() {
        // A long entity referred to many times at the top level, where roxmltree counts none:
        // 200 MB of text from 300 KB.
        let wide = format!(
            "<!DOCTYPE svg [<!ENTITY a '{}'>]><svg xmlns='http://www.w3.org/2000/svg'><desc>{}</desc></svg>",
            "x".repeat(100_000),
            "&a;".repeat(2000)
        );
        assert!(open(wide.as_bytes()).err().is_some_and(|e| e.contains("entities expand")));
        // Entities of entities, which roxmltree refuses too.
        let laughs =
            (1..10).fold("<!ENTITY lol0 'lol'>".to_string(), |d, i| format!("{d}<!ENTITY lol{i} '{}'>", format!("&lol{};", i - 1).repeat(10)));
        let text = format!("<!DOCTYPE svg [{laughs}]><svg xmlns='http://www.w3.org/2000/svg'><text>&lol9;</text></svg>");
        assert!(open(text.as_bytes()).err().is_some_and(|e| e.contains("entities expand")));
        let illustrator = "<!DOCTYPE svg PUBLIC '-//W3C//DTD SVG 1.1//EN' 'http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd' [\
            <!ENTITY ns_svg 'http://www.w3.org/2000/svg'> <!ENTITY ns_xlink 'http://www.w3.org/1999/xlink'>]>\
            <svg xmlns='&ns_svg;' xmlns:xlink='&ns_xlink;' width='4' height='4'/>";
        assert_eq!(open(illustrator.as_bytes()).map(|d| d.size(0)), Ok(Some((4.0, 4.0))));
        // An external entity is never read.
        let external = format!("<!DOCTYPE svg [<!ENTITY e SYSTEM 'file:///etc/passwd'>]>{}", svg(4, 4, "<text>&e;</text>"));
        assert!(open(external.as_bytes()).is_err());
    }

    /// `use`s that repeat what repeats (each level twice the one below) are counted as usvg
    /// would expand them, and refused before it does.
    #[test]
    fn uses_that_repeat_too_much_are_refused_before_parsing() {
        let doubling = |levels: usize, leaf: &str| {
            let mut body = format!("<defs><g id='l0'>{leaf}</g>");
            for i in 1..=levels {
                body.push_str(&format!("<g id='l{i}'><use href='#l{p}'/><use href='#l{p}'/></g>", p = i - 1));
            }
            body.push_str(&format!("</defs><use href='#l{levels}'/>"));
            svg(10, 10, &body)
        };
        // Few elements, each with much in it, within usvg's million: the content is bounded.
        let path = format!("<path d='M0 0{}'/>", " h1".repeat(100_000));
        assert!(open(doubling(9, &path).as_bytes()).err().is_some_and(|e| e.contains("MiB once what its uses")), "512 copies of 300 KB");
        assert!(open(doubling(8, "<rect width='1' height='1'/>").as_bytes()).is_ok(), "256 rects");
        // Past usvg's million, which it refuses too.
        assert!(open(doubling(30, "<rect width='1' height='1'/>").as_bytes()).err().is_some_and(|e| e.contains("more than 1000000 elements")));
        // A use of itself, or of its own ancestor, repeats nothing: usvg skips it.
        assert!(open(svg(10, 10, "<g id='a'><use href='#a'/><g id='b'><use href='#a'/></g></g>").as_bytes()).is_ok());
    }

    /// A marker is converted again for every vertex it marks, and a mask, a filter or a pattern
    /// for every element that refers to it.
    #[test]
    fn markers_and_references_are_counted_as_often_as_usvg_repeats_them() {
        let marker = format!("<marker id='m'><path d='M0 0{}'/></marker>", " l1 1".repeat(2_000));
        let vertices = format!("M0 0{}", " h1".repeat(15_000));
        let text = svg(10, 10, &format!("<defs>{marker}</defs><path d='{vertices}' marker-mid='url(#m)'/>"));
        assert!(open(text.as_bytes()).err().is_some_and(|e| e.contains("markers and references")), "15,000 marks of 10 KB");
        // The same marker inherited from a group, or set by a stylesheet's class.
        let inherited = svg(10, 10, &format!("<defs>{marker}</defs><g marker-mid='url(#m)'><path d='{vertices}'/></g>"));
        assert!(open(inherited.as_bytes()).is_err());
        let styled = svg(10, 10, &format!("<style>.a {{ marker-mid: url(#m) }}</style><defs>{marker}</defs><path class='a' d='{vertices}'/>"));
        assert!(open(styled.as_bytes()).is_err());
        let other_class = svg(10, 10, &format!("<style>.a {{ marker-mid: url(#m) }}</style><defs>{marker}</defs><path class='b' d='{vertices}'/>"));
        assert!(open(other_class.as_bytes()).is_ok(), "a rule for another class");
        // A mask of 100 KB, on 1500 rects.
        let mask = format!("<mask id='k'><path d='M0 0{}'/></mask>", " l1 1".repeat(20_000));
        let masked = svg(10, 10, &format!("<defs>{mask}</defs>{}", "<rect width='1' height='1' mask='url(#k)'/>".repeat(1500)));
        assert!(open(masked.as_bytes()).is_err());
        let once = svg(10, 10, &format!("<defs>{mask}</defs><g mask='url(#k)'>{}</g>", "<rect width='1' height='1'/>".repeat(1500)));
        assert!(open(once.as_bytes()).is_ok(), "a mask on the group, converted once");
    }

    #[test]
    fn text_is_counted_through_uses_and_too_much_is_left_out() {
        let line = format!("<text y='5' font-size='1'>{}</text>", "abcd ".repeat(20));
        let once = line.replacen("<text", "<text id='t'", 1);
        // In 2100 lines of 100, or one line used 2100 times: the rest is drawn, the text left out,
        // and said.
        for body in [line.repeat(2100), format!("<defs>{once}</defs>{}", "<use href='#t'/>".repeat(2100))] {
            let long = svg(10, 10, &format!("<rect width='10' height='10' fill='red'/>{body}"));
            let frame = render(&open(long.as_bytes()).unwrap(), 1.0);
            assert_eq!((frame.error.as_deref(), pixel(&frame, 5, 8)), (None, [255, 0, 0, 255]));
            assert_eq!(frame.warnings, ["its text was left out: more than 200000 characters"]);
        }
    }

    /// usvg lays out a `<text>` in time that grows as the square of its characters, and as its
    /// spans times its characters: text that would take too long is left out, and said.
    #[test]
    fn text_too_long_to_lay_out_is_left_out() {
        // 20,000 characters in 6,667 spans: half a minute of layout.
        let spans = svg(10, 10, &format!("<rect width='10' height='10' fill='red'/><text y='5'>{}</text>", "<tspan>ab </tspan>".repeat(6_667)));
        let frame = render(&open(spans.as_bytes()).unwrap(), 1.0);
        assert_eq!((frame.error.as_deref(), pixel(&frame, 5, 8)), (None, [255, 0, 0, 255]));
        assert_eq!(frame.warnings, ["its text was left out: too long to lay out, in one <text> or in many spans"]);
        // 10,000 characters in 100 spans are laid out.
        let ordinary = svg(10, 10, &format!("<text>{}</text>", format!("<tspan>{}</tspan>", "abcd ".repeat(20)).repeat(100)));
        let frame = render(&open(ordinary.as_bytes()).unwrap(), 1.0);
        assert!(!frame.warnings.iter().any(|warning| warning.contains("too long")), "{:?}", frame.warnings);
    }

    #[test]
    fn nesting_too_deep_is_refused_before_parsing() {
        let nested = |depth: usize| svg(10, 10, &format!("{}<rect width='1' height='1'/>{}", "<g>".repeat(depth), "</g>".repeat(depth)));
        assert!(open(nested(1000).as_bytes()).is_ok());
        assert!(open(nested(1100).as_bytes()).err().is_some_and(|e| e.contains("nests more than 1024 deep")));
        // A chain of uses, each of the next.
        let chain: String = (0..1100).map(|i| format!("<use id='u{i}' href='#u{}'/>", i + 1)).collect();
        assert!(open(svg(10, 10, &format!("<defs>{chain}<rect id='u1100'/></defs><use href='#u0'/>")).as_bytes()).is_err());
    }

    /// Deeply nested drawings parse and render on a render thread's stack (2 MiB, Rust's default).
    #[test]
    fn the_deepest_drawing_parses_and_renders_on_a_render_threads_stack() {
        let depth = MAX_DEPTH - 2;
        let body = format!("{}<rect width='10' height='10' fill='blue'/>{}", "<g opacity='0.99'>".repeat(depth), "</g>".repeat(depth));
        let text = svg(10, 10, &body);
        let frame =
            std::thread::Builder::new().stack_size(2 << 20).spawn(move || render(&open(text.as_bytes()).unwrap(), 0.2)).unwrap().join().unwrap();
        assert_eq!((frame.error.as_deref(), frame.width), (None, 2));
    }

    /// The error of rendering `body`, in a 100 × 100 drawing, at `scale`: empty if it renders.
    fn refused(body: &str, scale: f32) -> String {
        render(&open(svg(100, 100, body).as_bytes()).unwrap(), scale).error.unwrap_or_default()
    }

    /// What a render would hold at once is estimated before it starts, and more than MAX_RENDER
    /// refused: smaller renders of the same drawing still come. (Each case would hold about
    /// 1.2 GB if rendered.)
    #[test]
    fn a_render_that_would_hold_too_much_is_refused() {
        // Layers in layers, each as large as resvg allows: five times the canvas each way. At
        // 1200 × 1200, eight layers of 6000 × 6000.
        let layers = format!("{}<rect x='-1e6' y='-1e6' width='2e6' height='2e6'/>{}", "<g opacity='0.5'>".repeat(8), "</g>".repeat(8));
        assert_eq!(refused(&layers, 12.0), "drawing it at 1200 × 1200 would take more than 1024 MiB");
        assert_eq!(refused(&layers, 0.5), "", "a tile of it");
        // A pattern's tile is its own size, however large.
        let tile = "<defs><pattern id='p' width='2e4' height='2e4' patternUnits='userSpaceOnUse'><rect width='1' height='1'/></pattern></defs>\
                    <rect width='10' height='10' fill='url(#p)'/>";
        assert!(refused(tile, 1.0).contains("MiB"));
        // A filter keeps a buffer for each of its primitives.
        let primitives = format!("<filter id='f'>{}</filter>", "<feFlood flood-color='red'/>".repeat(100));
        let filtered = format!("<defs>{primitives}</defs><rect width='100' height='100' filter='url(#f)'/>");
        assert!(refused(&filtered, 14.0).contains("MiB"));
        assert_eq!(refused(&filtered, 1.0), "");
    }

    /// What a render would paint is estimated too, and more than MAX_WORK refused. (Each case
    /// would take about eight seconds if rendered.)
    #[test]
    fn a_render_that_would_take_too_long_is_refused() {
        // Shapes over the whole canvas: 2000 of 1500 × 1500.
        let many = "<rect width='100' height='100' fill-opacity='0.01'/>".repeat(2000);
        assert_eq!(refused(&many, 15.0), "drawing it at 1500 × 1500 would take too long (4502 million pixels painted)");
        assert_eq!(refused(&many, 1.0), "");
        // Turbulence of a thousand octaves, a morphology of a large radius.
        let octaves = "<defs><filter id='t'><feTurbulence baseFrequency='0.1' numOctaves='1000'/></filter></defs>\
                       <rect width='100' height='100' filter='url(#t)'/>";
        // Refused before resvg starts: past some 30 octaves, its turbulence overflows an integer
        // (a panic where overflow is checked, as in the tests' build; the page fails, and only it).
        assert!(refused(octaves, 4.0).contains("too long"), "{}", refused(octaves, 4.0));
        let morphology = "<defs><filter id='m'><feMorphology radius='17'/></filter></defs><rect width='100' height='100' filter='url(#m)'/>";
        assert!(refused(morphology, 4.0).contains("too long"));
    }

    /// The estimate holds what resvg's render of an ordinary drawing takes: a drawing with a drop
    /// shadow over the page renders at the largest size the viewer asks for.
    #[test]
    fn an_ordinary_drawing_renders_at_the_largest_size() {
        let shadow = "<defs><filter id='s'><feGaussianBlur in='SourceAlpha' stdDeviation='3'/><feOffset dx='2' dy='2'/>\
                      <feMerge><feMergeNode/><feMergeNode in='SourceGraphic'/></feMerge></filter></defs>\
                      <g filter='url(#s)' opacity='0.9'><rect x='5' y='5' width='90' height='90' fill='teal' clip-path='url(#c)'/></g>\
                      <clipPath id='c'><circle cx='50' cy='50' r='45'/></clipPath>";
        let drawing = open(svg(100, 100, shadow).as_bytes()).unwrap();
        // 4000 × 4000, the viewer's largest area (16 MP).
        let frame = render(&drawing, 40.0);
        assert_eq!((frame.error.as_deref(), frame.width), (None, 4000));
        assert_eq!(pixel(&frame, 2000, 2000)[3], 230, "the group's opacity");
    }

    /// Fonts are found only for a drawing with text, which is then drawn.
    #[test]
    fn text_is_drawn_with_the_systems_fonts() {
        let drawing = open(svg(200, 40, "<text x='0' y='30' font-size='30' font-family='sans-serif'>Hello</text>").as_bytes()).unwrap();
        let frame = render(&drawing, 1.0);
        assert_eq!(frame.error, None);
        let inked = frame.rgba.chunks(4).filter(|p| p[3] > 0).count();
        if system_fonts().is_empty() {
            assert_eq!(frame.warnings, ["its text was left out: no fonts were found on this system"]);
        } else {
            assert!(inked > 100, "{inked} pixels of text");
            assert!(frame.warnings.is_empty(), "{:?}", frame.warnings);
        }
        // Nothing shows whether fonts were looked for, which takes a while, but this decides it.
        let plain = svg(10, 10, "<title>not drawn</title><rect width='1' height='1'/>");
        assert!(!Scan::of(&roxmltree::Document::parse(&plain).unwrap()).unwrap().has_text);
    }

    #[test]
    fn a_drawing_too_large_is_refused() {
        assert!(within(vec![b' '; MAX_FILE as usize + 1]).is_err_and(|e| e.contains("more than 64 MiB")));
        let bomb = gzipped(&vec![b' '; (MAX_FILE + 10) as usize]);
        assert!(open(&bomb).err().is_some_and(|e| e.contains("more than 64 MiB unpacked")));
        let drawing = open(svg(100, 100, "").as_bytes()).unwrap();
        assert!(render(&drawing, 200.0).error.is_some_and(|e| e == "20000 × 20000 pixels is too large to render"));
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

        /// Whatever the bytes, detecting, opening and rendering them never panics.
        #[test]
        fn any_bytes_are_refused_or_drawn_without_panicking(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..512)) {
            let _ = is_svg("x.svg", &bytes);
            let _ = prolog(&bytes);
            let _ = entities_bound(&String::from_utf8_lossy(&bytes));
            if let Ok(drawing) = std::panic::catch_unwind(|| open(&bytes)).unwrap() {
                let _ = render(&drawing, 1.0);
            }
        }

        /// A drawing cut anywhere, or with a byte changed, is refused or drawn, never a panic.
        #[test]
        fn a_drawing_broken_anywhere_never_panics(cut in 0usize..2000, at in 0usize..2000, byte in proptest::prelude::any::<u8>()) {
            let text = format!(
                "<?xml version='1.0'?><!DOCTYPE svg [<!ENTITY e 'x'>]>{}",
                svg(20, 20, "<defs><linearGradient id='g'><stop offset='0' stop-color='red'/></linearGradient>\
                    <pattern id='p' width='4' height='4' patternUnits='userSpaceOnUse'><circle r='2'/></pattern>\
                    <filter id='f'><feGaussianBlur stdDeviation='1'/></filter><marker id='m'><path d='M0 0 l2 2'/></marker></defs>\
                    <g opacity='0.5' filter='url(#f)'><rect width='10' height='10' fill='url(#g)'/></g>\
                    <path d='M0 0 L10 10 L20 0' stroke='url(#p)' marker-mid='url(#m)'/><use href='#m'/><text>&e;</text>")
            );
            let mut bytes = text.into_bytes();
            if let Some(b) = bytes.get_mut(at) {
                *b = byte;
            }
            bytes.truncate(cut);
            if let Ok(drawing) = open(&bytes) {
                let _ = render(&drawing, 1.5);
            }
        }
    }
}
