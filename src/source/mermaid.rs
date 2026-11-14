//! Mermaid diagrams (`.mmd`, `.mermaid`), turned into SVG by merman, a pure-Rust Mermaid (its
//! parser, its Cytoscape layout and its SVG), and shown by whichever backend opens that SVG, the
//! SVG backend, never named here: this one draws nothing itself. The one file that names merman
//! ([ADR-0031](../../docs/adr/0031-mermaid-diagrams-through-merman-in-a-worker-process.md)).
//!
//! A diagram is untrusted, and what laying it out costs can't be bounded from its size, so merman
//! works in a worker process of clv's own ([`DIAGRAMS`]), capped on CPU time and heap, after its own
//! limits (its `Constrained` profile, Mermaid's 500 edges and a deadline). One worker serves the
//! backend's diagrams in turn ([`Service`]): started when one is first wanted, started again after
//! it dies, killed once idle, and not started for a while after a few killed in a row. What it
//! answers is kept ([`Cache`]), by the diagram's bytes, so a diagram is turned into SVG once
//! however often it is opened: a document's picture is opened to size it and again to draw it.
//!
//! A diagram a document shows on dark paper ([`Request::on_dark`], a fenced block's) is drawn again
//! in Mermaid's dark theme, with nothing behind it ([`Theme`]); the cache keeps each theme's SVG.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use merman::model::MermaidConfig;
use merman::model::resources::{InputResourcePolicy, ResourceProfile};
use merman::{OperationControl, RenderOutput, RenderRequest, Renderer, SemanticParser, SvgRequest};

use super::worker::{self, Died, FAILED, Link, Session, WALL, Worker};
use super::{
    Again, Anchor, BACKENDS, Backend, Bounds, Deferred, Format, Frame, HEAD, Input, Job, Opened, Pixels, Pool, Render, Request, Room, Source, Texts,
    Values,
};
use crate::args::{OptionSpec, Parsed};

/// Mermaid diagrams, turned into SVG by merman.
pub(super) struct Mermaid;

/// What it shows, by name alone: a `.mermaid` that is text, and a `.mmd` that starts as a diagram
/// ([`Kind::of`]).
const FORMAT: Format = Format::of("Mermaid", &["mmd", "mermaid"]).fenced(&["mermaid"]);

impl Backend for Mermaid {
    fn name(&self) -> &'static str {
        "merman"
    }

    fn shows(&self) -> &'static str {
        "Mermaid diagrams"
    }

    fn formats(&self) -> Vec<Format> {
        vec![FORMAT]
    }

    fn credits(&self) -> &[&'static str] {
        &["merman"]
    }

    fn options(&self) -> &'static [OptionSpec] {
        &[]
    }

    fn opens(&self, path: &str, head: &[u8]) -> bool {
        opens(path, head)
    }

    /// A document may show a diagram as a picture: what merman draws holds no file and refers to no
    /// picture, and is drawn once however often the document opens it ([`Cache`]).
    fn draws_pictures(&self) -> bool {
        true
    }

    /// It turns the whole diagram into SVG, in a worker, to open it: a list's tile opens it when wanted.
    fn deferred(&self, _path: &str, _head: &[u8]) -> Option<Deferred> {
        Some(Deferred { settings: &[] })
    }

    fn open(&self, input: Input, options: &Parsed<'_>) -> Result<Opened, String> {
        let path = input.path.clone();
        let error = |e: String| format!("{path}: {e}");
        let Some(bytes) = input.at_most(MAX_SOURCE)? else {
            return Err(error(format!("the diagram is more than {} MiB", MAX_SOURCE >> 20)));
        };
        let text = Arc::new(bytes);
        let svg = drawn(&SERVICE, &CACHE, &text, Theme::Light).map_err(error)?;
        Ok(match shown(&path, svg, options)? {
            Opened::Pages(light) => Opened::Pages(Box::new(Diagram { light, text, path })),
            files => files,
        })
    }
}

/// `svg`, what merman drew of the diagram at `path`, opened by the backend that shows it, with
/// `options`: found as any file's backend is, by how it starts, never by name.
fn shown(path: &str, svg: Arc<Vec<u8>>, options: &Parsed<'_>) -> Result<Opened, String> {
    let head = svg.get(..HEAD as usize).unwrap_or(&svg).to_vec();
    let again: Again = Arc::new(move || Ok(svg.to_vec()));
    let drawing = Input::held(path.to_owned(), head, again);
    // Not this one: what merman draws is never a diagram.
    let Some(backend) = BACKENDS.iter().filter(|b| b.name() != Mermaid.name()).find(|b| b.opens(&drawing.path, &drawing.head)) else {
        return Err(format!("{}: a diagram is shown as the SVG merman draws, and no backend opens that", drawing.path));
    };
    backend.open(drawing, options)
}

/// How a diagram is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Theme {
    /// In Mermaid's default theme, on white: a diagram shown by itself, as a picture by path, and as
    /// a document's block on light paper.
    Light,
    /// In Mermaid's dark theme, with nothing behind it: a document's block on dark paper, which shows
    /// through. A diagram's own `%%{init}%%` theme and colours still apply, as they do on a web page
    /// whose theme is dark.
    Dark,
}

/// A diagram, open: its SVG in Mermaid's default theme, opened by the backend that shows it, which
/// shows it wherever it is shown by itself; and its text, for a document that shows it on dark paper
/// to have it drawn again in the dark theme.
struct Diagram {
    light: Box<dyn Source + Send>,
    text: Arc<Vec<u8>>,
    path: String,
}

impl Diagram {
    /// The diagram in the dark theme, opened by the backend that shows its SVG, or why not.
    fn dark(&self) -> Result<Box<dyn Source + Send>, String> {
        let error = |e: String| format!("{}: {e}", self.path);
        let svg = drawn(&SERVICE, &CACHE, &self.text, Theme::Dark).map_err(error)?;
        match shown(&self.path, svg, &Parsed::default())? {
            Opened::Pages(dark) => Ok(dark),
            Opened::Files(_) => Err(error("its SVG holds files".into())),
        }
    }
}

/// Everything is the light drawing's, as its backend shows it; but a render for dark paper.
impl Source for Diagram {
    fn pages(&self) -> usize {
        self.light.pages()
    }

    fn size(&self, page: usize) -> Option<(f32, f32)> {
        self.light.size(page)
    }

    fn natural(&self, page: usize) -> bool {
        self.light.natural(page)
    }

    fn settings(&self) -> Vec<&'static str> {
        self.light.settings()
    }

    fn settle(&self, page: usize) {
        self.light.settle(page);
    }

    /// The light drawing's renderer, and for a request on dark paper the dark drawing's, drawn when
    /// the first such request comes.
    fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
        Ok(Box::new(OnPaper { diagram: self, light: self.light.renderer()?, dark: None }))
    }

    fn pool(&self, threads: usize) -> Box<dyn Pool> {
        self.light.pool(threads)
    }

    fn laid_at(&self) -> (Values, Option<Room>) {
        self.light.laid_at()
    }

    fn relayout(&self, values: Values, room: Room) -> Option<Job> {
        self.light.relayout(values, room)
    }

    fn largest_scale(&self, page: usize) -> Option<f32> {
        self.light.largest_scale(page)
    }

    fn bounds(&self, setting: &str, room: Room, values: Values) -> Option<Bounds> {
        self.light.bounds(setting, room, values)
    }

    fn anchor(&self, page: usize, y: f32) -> Option<Anchor> {
        self.light.anchor(page, y)
    }

    fn find(&self, anchor: &Anchor) -> Option<(usize, f32)> {
        self.light.find(anchor)
    }

    fn texts(&self) -> Option<Box<dyn Texts>> {
        self.light.texts()
    }
}

/// A diagram's renderer: the light drawing's, and the dark one's for a request on dark paper.
struct OnPaper<'a> {
    diagram: &'a Diagram,
    light: Box<dyn Render + 'a>,
    /// The dark drawing, once asked for, or why there is none.
    dark: Option<Result<Box<dyn Source + Send>, String>>,
}

impl Render for OnPaper<'_> {
    fn render(&mut self, request: Request) -> Frame {
        if !request.on_dark {
            return self.light.render(request);
        }
        let dark = self.dark.get_or_insert_with(|| self.diagram.dark());
        let room = self.diagram.light.size(request.page);
        let rendered = dark.as_ref().map_err(String::clone).and_then(|dark| in_the_room_of(room, &**dark, request));
        rendered.unwrap_or_else(|why| Frame::failed(request, why))
    }
}

/// The most pixels of the frame a dark drawing is put in the middle of: 64 million, 256 MiB.
const MAX_FRAME: u64 = 64_000_000;

/// `dark`, a diagram's dark drawing, rendered for `request` in the room its light drawing takes, of
/// the size `room`: a document is laid out around the light one, and Mermaid's dark theme lays some
/// diagrams out a little larger (a class diagram a tenth wider, ADR-0032). So it is drawn at the
/// request's scale where it is no larger than that room, and smaller where it is, its shape kept;
/// then put in the middle of a frame the light one's size, with nothing around it.
fn in_the_room_of(room: Option<(f32, f32)>, dark: &dyn Source, request: Request) -> Result<Frame, String> {
    let mut renderer = dark.renderer()?;
    let (Some((room_w, room_h)), Some((w, h))) = (room, dark.size(request.page)) else { return Ok(renderer.render(request)) };
    let fit = Some((room_w / w).min(room_h / h).min(1.0)).filter(|fit| fit.is_finite() && *fit > 0.0).unwrap_or(1.0);
    let frame = renderer.render(Request { scale: request.scale * fit, ..request });
    let frame = Frame { request, ..frame };
    if frame.error.is_some() || frame.again {
        return Ok(frame);
    }
    // The light drawing's frame, in whole pixels, and never smaller than what was drawn. The cast
    // is of a number between 1 and 2^31.
    let side = |points: f32, drawn: u32| ((f64::from(points) * f64::from(request.scale)).ceil().clamp(1.0, 2_147_483_648.0) as u32).max(drawn);
    let (width, height) = (side(room_w, frame.width), side(room_h, frame.height));
    if (width, height) == (frame.width, frame.height) || u64::from(width).saturating_mul(u64::from(height)) > MAX_FRAME {
        return Ok(frame);
    }
    Ok(centred(frame, width, height))
}

/// `frame` in the middle of one `width` by `height` pixels, each at least its own, with nothing
/// around it; as it is where its pixels aren't what it says they are.
fn centred(frame: Frame, width: u32, height: u32) -> Frame {
    // Each a `u32`, and their product within `MAX_FRAME`, which the caller checked.
    let (row, drawn_row) = (width as usize * 4, frame.width as usize * 4);
    let (left, top) = ((width.saturating_sub(frame.width) / 2) as usize * 4, (height.saturating_sub(frame.height) / 2) as usize);
    if drawn_row == 0 || frame.rgba.len() != drawn_row.saturating_mul(frame.height as usize) {
        return frame;
    }
    let mut pixels = vec![0u8; row.saturating_mul(height as usize)];
    for (into, from) in pixels.chunks_exact_mut(row).skip(top).zip(frame.rgba.chunks_exact(drawn_row)) {
        if let Some(into) = into.get_mut(left..left.saturating_add(drawn_row)) {
            into.copy_from_slice(from);
        }
    }
    Frame { width, height, rgba: Pixels::new(pixels), ..frame }
}

/// The largest diagram read: 1 MiB, merman's own limit on a diagram's text.
const MAX_SOURCE: u64 = 1 << 20;

/// The largest SVG a diagram is drawn to: 64 MiB, the most the SVG backend opens (the largest seen,
/// 22 MB, from a label of 1 MB).
const MAX_SVG: usize = 64 << 20;

/// The longest reason merman gives for refusing a diagram, in bytes: cut there, at a character.
const MAX_WHY: usize = 256;

/// Whether the file at `path`, which starts with `head`, is a diagram this opens: text named
/// `.mermaid`, or text named `.mmd` that starts as a diagram ([`starts_as_diagram`]).
fn opens(path: &str, head: &[u8]) -> bool {
    if !FORMAT.named(path) || head.contains(&0) {
        return false;
    }
    let mmd = std::path::Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("mmd"));
    !mmd || starts_as_diagram(head)
}

/// Whether `head`, a file's first [`HEAD`] bytes or all of a shorter one, starts as a Mermaid
/// diagram: its first line past a byte order mark, blank lines, `%%` comments and front matter
/// (`---` to `---`) is an `%%{init: …}%%` directive, or starts with a kind of diagram merman draws
/// ([`kinds`]), followed by a space, a `-` or the line's end. `.mmd` is MultiMarkdown's too, which
/// starts otherwise. Only what `head` holds whole counts: a line cut by its end, or by a character
/// cut there, is no line, so a diagram that starts past it is declined.
fn starts_as_diagram(head: &[u8]) -> bool {
    // A file shorter than the head ends with its last line; otherwise that line may go on. Told by
    // the head as read, its byte order mark included.
    let short = head.len() < HEAD as usize;
    let head = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    let text = match std::str::from_utf8(head) {
        Ok(text) => text,
        // What is UTF-8 of it, up to a character cut by its end or one that isn't.
        Err(e) => head.get(..e.valid_up_to()).and_then(|valid| std::str::from_utf8(valid).ok()).unwrap_or_default(),
    };
    let whole = short && text.len() == head.len();
    let mut lines = text.split('\n').peekable();
    let mut front = false;
    let mut first = true;
    while let Some(line) = lines.next() {
        let ended = lines.peek().is_some() || whole;
        let trimmed = line.trim();
        if !ended {
            return false;
        }
        if front {
            front = trimmed != "---";
            continue;
        }
        if first && trimmed == "---" {
            front = true;
            first = false;
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        first = false;
        if let Some(directive) = trimmed.strip_prefix("%%{") {
            return directive.trim_start().starts_with("init");
        }
        if trimmed.starts_with("%%") {
            continue;
        }
        return kinds().any(|kind| {
            trimmed
                .strip_prefix(kind)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('-') || rest.starts_with(|c: char| c.is_ascii_whitespace()))
        });
    }
    false
}

/// The kinds of diagram merman draws, as a diagram's first line names them: the first word of each
/// of its headers (`flowchart`, `graph`, `sequenceDiagram`, `stateDiagram-v2`…), matched in their
/// case, as Mermaid matches them.
fn kinds() -> impl Iterator<Item = &'static str> {
    merman::diagram_header_facts().iter().filter_map(|fact| fact.label.split(' ').next()).filter(|kind| !kind.is_empty())
}

// The worker's side.

/// A diagram's text, to draw: the request.
const DRAW: u8 = 1;

/// A diagram's text, to draw in the dark theme: a request too.
const DRAW_DARK: u8 = 4;

/// Its SVG: an answer.
const DRAWN: u8 = 2;

/// Why merman refused it, in its words: an answer.
const REFUSED: u8 = 3;

/// Why a worker answered [`FAILED`]: what it was asked isn't a request.
const MALFORMED: u8 = 1;

/// The CPU seconds a diagram may take in the worker: about ten times the costliest diagram merman
/// accepts (ADR-0031). The viewer waits [`WALL`] times as long.
const DRAW_CPU: u64 = 2;

/// How long merman works on a diagram before it stops itself, with its own message: before the
/// kernel stops the worker at [`DRAW_CPU`].
const DEADLINE: Duration = Duration::from_millis(1_500);

/// The most the worker's heap holds: twice the costliest diagram merman accepts (ADR-0031).
const MAX_HEAP: usize = 512 << 20;

/// The most edges a diagram may have: Mermaid's own limit.
const MAX_EDGES: u32 = 500;

/// The worker that turns a diagram into SVG: `clv --worker mermaid`.
pub(crate) struct Diagrams;

pub(crate) static DIAGRAMS: Diagrams = Diagrams;

impl Worker for Diagrams {
    fn name(&self) -> &'static str {
        "mermaid"
    }

    fn takes(&self, kind: u8) -> Option<(u64, usize)> {
        // Within MAX_SOURCE, which is 1 MiB.
        matches!(kind, DRAW | DRAW_DARK).then_some((DRAW_CPU, MAX_SOURCE as usize))
    }

    fn heap(&self) -> usize {
        MAX_HEAP
    }

    fn session(&self) -> Box<dyn Session> {
        Box::new(Drawing { light: None, dark: None })
    }
}

/// What the worker keeps between diagrams: merman's renderer for each theme, made for the first
/// diagram drawn in it (some 40 ms).
struct Drawing {
    light: Option<Renderer>,
    dark: Option<Renderer>,
}

impl Session for Drawing {
    fn answer(&mut self, kind: u8, payload: Vec<u8>) -> (u8, Vec<u8>) {
        let Ok(text) = String::from_utf8(payload) else { return (FAILED, vec![MALFORMED]) };
        let renderer = match kind {
            DRAW => self.light.get_or_insert_with(|| renderer(Theme::Light)),
            DRAW_DARK => self.dark.get_or_insert_with(|| renderer(Theme::Dark)),
            _ => return (FAILED, vec![MALFORMED]),
        };
        let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| svg(renderer, &text)));
        match drawn.unwrap_or_else(|_| Err("merman failed on it".into())) {
            Ok(svg) if svg.len() > MAX_SVG => (REFUSED, format!("its SVG is more than {} MiB", MAX_SVG >> 20).into_bytes()),
            Ok(svg) => (DRAWN, svg.into_bytes()),
            Err(why) => (REFUSED, cut(&why, MAX_WHY).as_bytes().to_vec()),
        }
    }
}

/// merman's renderer, as clv draws with it: labels as SVG `<text>`, which a diagram's own
/// configuration can't turn back into HTML, nor raise Mermaid's other secure settings (its limit on
/// edges among them); within merman's `Constrained` limits. In `theme`: Mermaid's default, or its
/// dark one as the site's, which a diagram's own configuration may still change, as on a web page.
fn renderer(theme: Theme) -> Renderer {
    let mut config = serde_json::json!({
        "htmlLabels": false,
        "flowchart": { "htmlLabels": false },
        "maxEdges": MAX_EDGES,
        "secure": ["secure", "securityLevel", "startOnLoad", "maxTextSize", "suppressErrorRendering", "maxEdges", "htmlLabels"],
    });
    if let (Theme::Dark, Some(config)) = (theme, config.as_object_mut()) {
        config.insert("theme".into(), "dark".into());
    }
    let parser = SemanticParser::new().with_site_config(MermaidConfig::from_value(config));
    Renderer::new().with_semantic_parser(parser).with_resource_policy(InputResourcePolicy::for_profile(ResourceProfile::Constrained))
}

/// The diagram `text` as SVG, or merman's reason why not.
fn svg(renderer: &Renderer, text: &str) -> Result<String, String> {
    let control = OperationControl::new().with_deadline(DEADLINE);
    match renderer.render(RenderRequest::svg(text, control, SvgRequest::default())) {
        Ok(RenderOutput::Svg(Some(svg))) => Ok(svg.svg().to_owned()),
        Ok(_) => Err("merman drew nothing of it".into()),
        Err(e) => Err(e.to_string()),
    }
}

/// `text` cut to at most `most` bytes, at a character.
fn cut(text: &str, most: usize) -> &str {
    let mut end = most.min(text.len());
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text.get(..end).unwrap_or_default()
}

// The viewer's side.

/// How long the worker is kept once idle: killed this long after its last answer.
const IDLE: Duration = Duration::from_secs(2);

/// How many workers killed in a row pause the service: three cost at most 6 s of CPU.
const MAX_KILLS: u32 = 3;

/// How long no worker is started after [`MAX_KILLS`] killed in a row.
const PAUSE: Duration = Duration::from_secs(60);

/// The diagrams' worker, and what the viewer keeps of what it answered.
static SERVICE: Service = Service::new(|| worker::spawn(&DIAGRAMS));
static CACHE: Cache = Cache::new(MAX_CACHED_BYTES, MAX_CACHED);

/// Why a diagram wasn't drawn, as the worker's service saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Failed {
    /// merman refused it, in its words, or the worker died on it: kept, as it would fail again.
    Kept(String),
    /// No worker for it, for now: not kept, as it may pass.
    Passing(String),
}

/// The SVG `source` is drawn to in `theme`, from `cache` or from `service`'s worker; or why not, for
/// the reader. In the light theme it is put on white; in the dark one it is left on nothing.
fn drawn(service: &'static Service, cache: &Cache, source: &Arc<Vec<u8>>, theme: Theme) -> Result<Arc<Vec<u8>>, String> {
    if std::str::from_utf8(source).is_err() {
        return Err("the diagram isn't UTF-8 text".into());
    }
    if let Some(kept) = cache.get(source, theme) {
        return kept;
    }
    // Its turn: diagrams opened at once wait for it here, and one drawn meanwhile is in the cache.
    let mut turn = service.turn();
    if let Some(kept) = cache.get(source, theme) {
        return kept;
    }
    match service.ask(&mut turn, source.clone(), theme) {
        Ok(svg) => {
            let svg = match theme {
                Theme::Light => on_white(svg),
                Theme::Dark => svg,
            };
            let svg = Arc::new(svg.into_bytes());
            cache.put(source, theme, Ok(svg.clone()));
            Ok(svg)
        }
        Err(Failed::Kept(why)) => {
            cache.put(source, theme, Err(why.clone()));
            Err(why)
        }
        Err(Failed::Passing(why)) => Err(why),
    }
}

/// The most of an SVG's start that is looked through for the end of its `<svg>` tag: merman's is a
/// few hundred bytes.
const MAX_START_TAG: usize = 4096;

/// `svg`, as merman drew it, on white: a white rectangle over its `viewBox` (or its whole viewport,
/// without one) as the first thing it draws. Mermaid's ink is made for white, as `mmdc -b white`
/// draws it, where an SVG drawing is shown on the terminal's background. Only the `<svg>` start tag
/// is read, within [`MAX_START_TAG`]: an SVG that doesn't start so is left as it is, for the SVG
/// backend to judge.
fn on_white(svg: String) -> String {
    let Some(end) = start_tag_end(svg.as_bytes()) else { return svg };
    let Some(tag) = svg.get(..end) else { return svg };
    if tag.ends_with('/') {
        return svg;
    }
    let rect = match view_box(tag) {
        Some([x, y, width, height]) => format!(r#"<rect x="{x}" y="{y}" width="{width}" height="{height}" fill="white"/>"#),
        None => r#"<rect width="100%" height="100%" fill="white"/>"#.to_owned(),
    };
    let mut out = String::new();
    if out.try_reserve_exact(svg.len().saturating_add(rect.len())).is_err() {
        return svg;
    }
    out.push_str(tag);
    out.push('>');
    out.push_str(&rect);
    out.push_str(svg.get(end.saturating_add(1)..).unwrap_or_default());
    out
}

/// Where the `>` that ends `svg`'s `<svg` start tag is, outside its attributes' quotes, within
/// [`MAX_START_TAG`] bytes; `None` for an SVG that doesn't start with one there.
fn start_tag_end(svg: &[u8]) -> Option<usize> {
    let start = svg.get(..MAX_START_TAG.min(svg.len()))?;
    if !start.starts_with(b"<svg") || !start.get(4).is_some_and(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/') {
        return None;
    }
    let mut quote = None;
    for (at, &b) in start.iter().enumerate() {
        match (quote, b) {
            (None, b'"' | b'\'') => quote = Some(b),
            (Some(q), b) if b == q => quote = None,
            (None, b'>') => return Some(at),
            _ => {}
        }
    }
    None
}

/// The `viewBox` of the start tag `tag`: its x, y, width and height, finite, its sides positive.
fn view_box(tag: &str) -> Option<[f64; 4]> {
    let at = tag.find(" viewBox=").or_else(|| tag.find("\nviewBox="))?;
    let rest = tag.get(at.saturating_add(" viewBox=".len())..)?;
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let value = rest.get(1..)?.split(quote).next()?;
    let mut numbers = value.split(|c: char| c.is_ascii_whitespace() || c == ',').filter(|n| !n.is_empty()).map(str::parse::<f64>);
    let mut out = [0.0; 4];
    for v in &mut out {
        *v = numbers.next()?.ok().filter(|v| v.is_finite())?;
    }
    let [_, _, width, height] = out;
    (numbers.next().is_none() && width > 0.0 && height > 0.0).then_some(out)
}

/// The one worker that draws the backend's diagrams, in turn: held behind a lock, started when a
/// diagram is first wanted, started again after it dies, and killed once idle ([`IDLE`]) by a
/// timer of the service's own. After [`MAX_KILLS`] killed in a row, none is started for [`PAUSE`].
struct Service {
    state: Mutex<State>,
    /// How a worker is started: [`DIAGRAMS`], but in the tests.
    spawn: fn() -> Result<Link, Died>,
}

struct State {
    link: Option<Link>,
    /// When the worker last answered, or was started.
    last: Option<Instant>,
    /// Workers killed on a diagram since one last drew one.
    kills: u32,
    /// Since when none is started, after [`MAX_KILLS`] killed in a row.
    paused: Option<Instant>,
    /// How many workers were started: the idle timer serves the one it was started with.
    started: u64,
    /// Whether an idle timer serves the worker.
    timed: bool,
}

impl Service {
    const fn new(spawn: fn() -> Result<Link, Died>) -> Self {
        Self { state: Mutex::new(State { link: None, last: None, kills: 0, paused: None, started: 0, timed: false }), spawn }
    }

    /// The service to this caller alone, while it holds it: a diagram's turn.
    fn turn(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The worker's SVG for `source`, a diagram's UTF-8 text, drawn in `theme`, in `state`, the turn
    /// held; a worker started first if there is none.
    fn ask(&'static self, state: &mut State, source: Arc<Vec<u8>>, theme: Theme) -> Result<String, Failed> {
        if let Some(since) = state.paused {
            if since.elapsed() < PAUSE {
                let why = format!("{MAX_KILLS} diagrams in a row were too much for merman: none is drawn for a minute after");
                return Err(Failed::Passing(why));
            }
            state.paused = None;
            state.kills = 0;
        }
        if state.link.is_none() {
            let link = (self.spawn)().map_err(|died| Failed::Passing(said(&died)))?;
            state.started = state.started.wrapping_add(1);
            state.last = Some(Instant::now());
            let (service, started) = (self, state.started);
            // Without a timer, the worker is let go of after this diagram instead.
            state.timed = std::thread::Builder::new().name("mermaid idle".into()).spawn(move || service.idle(started)).is_ok();
            state.link = Some(link);
        }
        let Some(link) = state.link.as_mut() else { return Err(Failed::Passing(said(&Died::Unstarted("none started".into())))) };
        let wall = Duration::from_secs(DRAW_CPU.saturating_mul(u64::from(WALL)));
        let kind = match theme {
            Theme::Light => DRAW,
            Theme::Dark => DRAW_DARK,
        };
        let asked = link.request_shared(kind, source, wall, MAX_SVG);
        let answer = match asked {
            Ok((DRAWN, svg)) => String::from_utf8(svg).map_err(|_| link.refuse()),
            Ok((REFUSED, why)) if why.len() <= MAX_WHY => match String::from_utf8(why) {
                Ok(why) => return self.answered(state, Err(Failed::Kept(format!("merman: {why}")))),
                Err(_) => Err(link.refuse()),
            },
            Ok(_) => Err(link.refuse()),
            Err(died) => Err(died),
        };
        match answer {
            Ok(svg) => self.answered(state, Ok(svg)),
            Err(died) => {
                state.link = None;
                state.kills = state.kills.saturating_add(1);
                if state.kills >= MAX_KILLS {
                    state.paused = Some(Instant::now());
                }
                Err(match died {
                    Died::TooMany | Died::Unstarted(_) | Died::Version => Failed::Passing(said(&died)),
                    _ => Failed::Kept(said(&died)),
                })
            }
        }
    }

    /// `answer`, as the worker gave it: alive, its killed counted from none again, and idle from now.
    fn answered(&self, state: &mut State, answer: Result<String, Failed>) -> Result<String, Failed> {
        state.kills = 0;
        state.last = Some(Instant::now());
        if !state.timed {
            state.link = None;
        }
        answer
    }

    /// The idle timer of the worker started `started`th: it kills it [`IDLE`] after its last answer,
    /// and ends with it.
    fn idle(&self, started: u64) {
        loop {
            let wait = {
                let mut state = self.turn();
                if state.started != started || state.link.is_none() {
                    return;
                }
                let idle = state.last.map_or(IDLE, |last| last.elapsed());
                if idle >= IDLE {
                    state.link = None;
                    return;
                }
                IDLE.saturating_sub(idle)
            };
            std::thread::sleep(wait);
        }
    }
}

/// Why a diagram wasn't drawn, in the reader's words, as the worker went: `died`.
fn said(died: &Died) -> String {
    match died {
        Died::TooMany => format!("too many of {}'s worker processes at once: the diagram isn't drawn", crate::NAME),
        Died::Unstarted(why) => format!("no process to draw the diagram in ({why})"),
        Died::TimedOut | Died::Cpu => "merman took too long on the diagram".into(),
        Died::Memory => "merman took too much memory on the diagram".into(),
        Died::Version => format!("{} was replaced while it ran: open the diagram again", crate::NAME),
        Died::Ended(_) | Died::Malformed => "merman failed on the diagram".into(),
    }
}

/// The most bytes the cache keeps, diagrams and their SVG together.
const MAX_CACHED_BYTES: usize = 64 << 20;

/// The most diagrams the cache keeps.
const MAX_CACHED: usize = 4_096;

/// What the worker answered for each diagram in each theme, by its bytes: its SVG, or why it failed.
/// At most `bytes` of diagrams and SVG and `most` of them, those used longest ago let go of first.
struct Cache {
    kept: Mutex<Kept>,
    bytes: usize,
    most: usize,
}

#[derive(Default)]
struct Kept {
    /// Each diagram's hash, its bytes, what it came to, and when it was last used; the most recently
    /// used last.
    entries: Vec<Entry>,
    /// The bytes of `entries`, diagrams and SVG.
    bytes: usize,
}

struct Entry {
    hash: u64,
    theme: Theme,
    source: Arc<Vec<u8>>,
    drawn: Result<Arc<Vec<u8>>, String>,
}

impl Entry {
    fn bytes(&self) -> usize {
        let drawn = self.drawn.as_ref().map_or_else(String::len, |svg| svg.len());
        self.source.len().saturating_add(drawn)
    }
}

impl Cache {
    const fn new(bytes: usize, most: usize) -> Self {
        Self { kept: Mutex::new(Kept { entries: Vec::new(), bytes: 0 }), bytes, most }
    }

    fn lock(&self) -> MutexGuard<'_, Kept> {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What `source` came to in `theme`, if it is kept: found by its hash, taken by its bytes.
    fn get(&self, source: &[u8], theme: Theme) -> Option<Result<Arc<Vec<u8>>, String>> {
        let hash = hashed(source);
        let mut kept = self.lock();
        let at = kept.entries.iter().position(|entry| entry.hash == hash && entry.theme == theme && entry.source.as_slice() == source)?;
        let entry = kept.entries.remove(at);
        let drawn = entry.drawn.clone();
        kept.entries.push(entry);
        Some(drawn)
    }

    /// `source` kept as having come to `drawn` in `theme`, those used longest ago let go of to make
    /// room. One larger than the whole cache isn't kept.
    fn put(&self, source: &Arc<Vec<u8>>, theme: Theme, drawn: Result<Arc<Vec<u8>>, String>) {
        let entry = Entry { hash: hashed(source), theme, source: source.clone(), drawn };
        let size = entry.bytes();
        if size > self.bytes {
            return;
        }
        let mut kept = self.lock();
        if let Some(at) = kept.entries.iter().position(|e| e.hash == entry.hash && e.theme == theme && e.source == entry.source) {
            let old = kept.entries.remove(at);
            kept.bytes = kept.bytes.saturating_sub(old.bytes());
        }
        while !kept.entries.is_empty() && (kept.entries.len() >= self.most || kept.bytes.saturating_add(size) > self.bytes) {
            let old = kept.entries.remove(0);
            kept.bytes = kept.bytes.saturating_sub(old.bytes());
        }
        kept.bytes = kept.bytes.saturating_add(size);
        kept.entries.push(entry);
    }
}

/// `bytes`' hash, to find a diagram by.
fn hashed(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`drawn`], in the light theme.
    fn light(service: &'static Service, cache: &Cache, source: Vec<u8>) -> Result<Arc<Vec<u8>>, String> {
        drawn(service, cache, &Arc::new(source), Theme::Light)
    }

    impl Cache {
        /// [`Cache::get`] and [`Cache::put`], in the light theme.
        fn got(&self, source: &[u8]) -> Option<Result<Arc<Vec<u8>>, String>> {
            self.get(source, Theme::Light)
        }

        fn keep(&self, source: &Arc<Vec<u8>>, drawn: Result<Arc<Vec<u8>>, String>) {
            self.put(source, Theme::Light, drawn);
        }
    }

    /// A `.mmd` is a diagram by its first line, past what may come before it, and only by merman's
    /// kinds, in their case; a `.mermaid` by being text.
    #[test]
    fn a_mmd_starts_as_a_diagram_merman_draws() {
        assert_eq!(kinds().count(), 47);
        assert!(kinds().any(|k| k == "flowchart") && kinds().any(|k| k == "stateDiagram-v2") && kinds().any(|k| k == "flowchart-elk"));
        for head in [
            "flowchart LR\n",
            "graph",
            "  sequenceDiagram\n",
            "stateDiagram-v2\n",
            "sankey-beta\n",
            "pie title Pets\n",
            "\u{feff}%% a comment\n\nclassDiagram\n",
            "---\ntitle: x\nflowchart: y\n---\nerDiagram\n",
            "%%{init: {'theme': 'dark'}}%%\nwhatever\n",
            "%%{ initialize: {} }%%\n",
        ] {
            assert!(opens("a.mmd", head.as_bytes()), "{head:?}");
        }
        for head in ["", "Flowchart LR\n", "flowcharts\n", "Title: Notes\n\nflowchart LR\n", "%%{wrap}%%\nflowchart\n", "---\nflowchart LR\n", "%%\n"]
        {
            assert!(!opens("a.mmd", head.as_bytes()), "{head:?}");
        }
        assert!(opens("a.MERMAID", b"anything at all") && opens("a.mermaid", b""));
        assert!(!opens("a.mermaid", b"flowchart\0") && !opens("a.md", b"flowchart LR\n") && !opens("mmd", b"flowchart LR\n"));
        // A line cut by the head's end is no line.
        let full = |text: &str| -> Vec<u8> { format!("{text}{}", "x".repeat(HEAD as usize)).into_bytes()[..HEAD as usize].to_vec() };
        assert!(opens("a.mmd", &full("flowchart LR\n%% ")) && !opens("a.mmd", &full(&"\n".repeat(1023))));
        // A head of 1024 bytes with a byte order mark is as cut as one without: a file whose first
        // line, after blank ones, has its 1024th byte just after `graph`, in "graphical notes", is no
        // diagram.
        let graphical = |bom: &str| -> Vec<u8> {
            let text = format!("{bom}{}graphical notes", "\n".repeat(HEAD as usize - bom.len() - 5));
            text.into_bytes()[..HEAD as usize].to_vec()
        };
        assert!(graphical("\u{feff}").ends_with(b"\ngraph") && graphical("").ends_with(b"\ngraph"));
        assert!(!opens("a.mmd", &graphical("\u{feff}")) && !opens("a.mmd", &graphical("")));
        assert!(opens("a.mmd", b"\xEF\xBB\xBFgraph"), "a short file with a byte order mark ends with its line");
        let mut cut = full("pie\n%% ");
        cut[HEAD as usize - 1] = 0xC3;
        assert!(opens("a.mmd", &cut));
    }

    /// The cache keeps at most its bytes and its diagrams, those used longest ago let go of first,
    /// and finds a diagram by its bytes, its failure too.
    #[test]
    fn the_cache_is_bounded_and_lets_go_of_the_least_used() {
        let cache = Cache::new(100, 3);
        let source = |s: &str| Arc::new(s.as_bytes().to_vec());
        let svg = |n: usize| Ok(Arc::new(vec![b'x'; n]));
        let (a, b, c, d) = (source("a"), source("b"), source("c"), source("d"));
        cache.keep(&a, svg(10));
        cache.keep(&b, Err("refused".into()));
        cache.keep(&c, svg(10));
        assert_eq!(cache.got(&b), Some(Err("refused".into())), "a failure kept");
        assert!(cache.got(&a).is_some(), "a used: b and c are older");
        cache.keep(&d, svg(10));
        assert!(cache.got(&b).is_some() && cache.got(&c).is_none(), "three at most: c went, used longest ago");
        cache.keep(&c, svg(85));
        assert!(cache.got(&c).is_some() && cache.got(&d).is_none() && cache.got(&a).is_none(), "100 bytes at most");
        assert_eq!(cache.lock().bytes, 86 + 8);
        cache.keep(&a, svg(100));
        assert!(cache.got(&a).is_none() && cache.got(&c).is_some(), "larger than the cache: not kept");
        cache.keep(&c, svg(1));
        assert_eq!(cache.got(&c).map(|kept| kept.map(|svg| svg.len())), Some(Ok(1)), "replaced");
        assert_eq!(cache.lock().bytes, 2 + 8);
        // Found by its bytes, not its hash alone.
        let e = source("e");
        cache.lock().entries.push(Entry { hash: hashed(&e), theme: Theme::Light, source: source("f"), drawn: svg(1) });
        assert!(cache.got(&e).is_none());
    }

    /// The tests that draw several diagrams through a real worker take turns, and let go of their
    /// worker when they are done: in the tests a worker is a thread of this process, and every test's
    /// share the six a viewer may have at once, the SVG backend's among them.
    static TURN: Mutex<()> = Mutex::new(());

    /// A diagram is drawn once: asked again, it is the cache's. merman's refusal is kept too, but
    /// not a worker that couldn't be started, which may be later.
    #[test]
    fn what_the_worker_answers_is_kept_but_for_no_worker() {
        let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
        static STARTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        static SERVICE: Service = Service::new(|| {
            STARTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            worker::spawn(&DIAGRAMS)
        });
        let cache = Cache::new(MAX_CACHED_BYTES, MAX_CACHED);
        let flow = b"flowchart LR\n  a --> b\n".to_vec();
        let svg = light(&SERVICE, &cache, flow.clone()).unwrap();
        assert!(svg.starts_with(b"<svg"), "{:?}", String::from_utf8_lossy(&svg[..40]));
        SERVICE.turn().link = None;
        assert!(Arc::ptr_eq(&light(&SERVICE, &cache, flow).unwrap(), &svg), "kept");
        assert_eq!(STARTS.load(std::sync::atomic::Ordering::SeqCst), 1, "no worker asked");
        let broken = b"flowchart TD\n  A[[[ (((\n".to_vec();
        let why = light(&SERVICE, &cache, broken.clone()).unwrap_err();
        assert!(why.starts_with("merman: ") && why.len() <= MAX_WHY + 8, "{why}");
        assert_eq!(cache.got(&broken), Some(Err(why)));
        assert_eq!(light(&SERVICE, &cache, b"\xff".to_vec()), Err("the diagram isn't UTF-8 text".into()));
        static FULL: Service = Service::new(|| Err(Died::TooMany));
        let pie = b"pie\n  \"a\" : 1\n".to_vec();
        let why = light(&FULL, &cache, pie.clone()).unwrap_err();
        assert!(why.starts_with("too many of clv's worker processes at once"), "{why}");
        assert_eq!(cache.got(&pie), None, "not kept");
        SERVICE.turn().link = None;
    }

    /// The worker is killed once idle, 2 s after its last answer, and not before; the next diagram
    /// starts another.
    #[test]
    fn the_worker_is_gone_two_seconds_after_its_answer() {
        static SERVICE: Service = Service::new(|| worker::spawn(&DIAGRAMS));
        let cache = Cache::new(MAX_CACHED_BYTES, MAX_CACHED);
        let alive = || SERVICE.turn().link.is_some();
        light(&SERVICE, &cache, b"flowchart LR\n  a --> b\n".to_vec()).unwrap();
        let answered = Instant::now();
        assert!(alive());
        std::thread::sleep(Duration::from_millis(1_500));
        assert!(alive(), "still alive 1.5 s after its answer");
        while alive() {
            assert!(answered.elapsed() < Duration::from_secs(4), "still alive");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(answered.elapsed() >= IDLE, "{:?}", answered.elapsed());
        light(&SERVICE, &cache, b"flowchart LR\n  c --> d\n".to_vec()).unwrap();
        assert_eq!(SERVICE.turn().started, 2);
    }

    /// A worker that dies on a diagram fails that one alone, which is kept, and the next starts
    /// another; three in a row, and none is started for a while, the diagrams wanting one failing.
    #[test]
    fn three_workers_killed_in_a_row_pause_the_service() {
        /// A worker that dies on every diagram.
        struct Dying;
        impl Worker for Dying {
            fn name(&self) -> &'static str {
                "dying"
            }
            fn takes(&self, kind: u8) -> Option<(u64, usize)> {
                DIAGRAMS.takes(kind)
            }
            fn heap(&self) -> usize {
                MAX_HEAP
            }
            fn session(&self) -> Box<dyn Session> {
                struct Dies;
                impl Session for Dies {
                    fn answer(&mut self, _kind: u8, _payload: Vec<u8>) -> (u8, Vec<u8>) {
                        // The worker's thread ends, and its pipes with it.
                        std::panic::resume_unwind(Box::new("dies"))
                    }
                }
                Box::new(Dies)
            }
        }
        static DYING: Dying = Dying;
        static STARTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        static SERVICE: Service = Service::new(|| {
            STARTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            worker::spawn(&DYING)
        });
        let cache = Cache::new(MAX_CACHED_BYTES, MAX_CACHED);
        for n in 0..3 {
            let source = format!("pie\n  \"{n}\" : 1\n").into_bytes();
            assert_eq!(light(&SERVICE, &cache, source.clone()), Err("merman failed on the diagram".into()));
            assert_eq!(cache.got(&source), Some(Err("merman failed on the diagram".into())), "kept");
        }
        assert_eq!(STARTS.load(std::sync::atomic::Ordering::SeqCst), 3, "one a diagram");
        let why = light(&SERVICE, &cache, b"pie\n".to_vec()).unwrap_err();
        assert!(why.contains("none is drawn for a minute"), "{why}");
        assert_eq!(STARTS.load(std::sync::atomic::Ordering::SeqCst), 3, "paused");
        assert_eq!(cache.got(b"pie\n"), None, "not kept: it may pass");
        // Past the pause, a worker is started again.
        SERVICE.turn().paused = Instant::now().checked_sub(PAUSE);
        assert_eq!(light(&SERVICE, &cache, b"pie\n".to_vec()), Err("merman failed on the diagram".into()));
        assert_eq!(STARTS.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    /// A diagram is put on white: a rectangle over its viewBox, wherever that starts, as the first
    /// thing the SVG draws; over its viewport without one; and nothing for what doesn't start as an
    /// SVG, or ends its start tag past where it is looked for.
    #[test]
    fn a_diagram_is_put_on_white_over_its_view_box() {
        let svg = r#"<svg id="m" style="a>b" viewBox="-8 -8.5 200 100.25" xmlns="http://www.w3.org/2000/svg"><g/></svg>"#;
        assert_eq!(
            on_white(svg.into()),
            r#"<svg id="m" style="a>b" viewBox="-8 -8.5 200 100.25" xmlns="http://www.w3.org/2000/svg"><rect x="-8" y="-8.5" width="200" height="100.25" fill="white"/><g/></svg>"#
        );
        assert_eq!(on_white("<svg width='3'><g/></svg>".into()), r#"<svg width='3'><rect width="100%" height="100%" fill="white"/><g/></svg>"#);
        assert_eq!(on_white("<svg viewBox='0,0,4,2'>".into()), r#"<svg viewBox='0,0,4,2'><rect x="0" y="0" width="4" height="2" fill="white"/>"#);
        for unchanged in ["<svg/>", "<svgx>", "<?xml?><svg>", "<svg viewBox='0 0 1 1'", "", "<g><svg>"] {
            assert_eq!(on_white(unchanged.into()), unchanged);
        }
        for odd in ["0 0 -1 1", "0 0 1", "0 0 1 1 1", "0 0 inf 1", "a b c d"] {
            assert_eq!(view_box(&format!("<svg viewBox='{odd}'")), None, "{odd}");
        }
        let late = format!("<svg {}>", "x".repeat(MAX_START_TAG));
        assert_eq!(on_white(late.clone()), late);
    }

    /// Each theme's SVG is kept apart: a diagram drawn for dark paper is no answer for light, nor
    /// the other way, and each is counted in what the cache holds.
    #[test]
    fn the_cache_keeps_each_themes_svg_apart() {
        let cache = Cache::new(100, 3);
        let a = Arc::new(b"a".to_vec());
        cache.put(&a, Theme::Light, Ok(Arc::new(vec![b'l'; 10])));
        assert_eq!(cache.get(&a, Theme::Dark), None);
        cache.put(&a, Theme::Dark, Err("refused in the dark".into()));
        assert_eq!(cache.get(&a, Theme::Light).map(|kept| kept.map(|svg| svg.len())), Some(Ok(10)));
        assert_eq!(cache.get(&a, Theme::Dark), Some(Err("refused in the dark".into())));
        let held = || {
            let kept = cache.lock();
            (kept.entries.len(), kept.bytes)
        };
        assert_eq!(held(), (2, 11 + 20));
        cache.put(&a, Theme::Dark, Ok(Arc::new(vec![b'd'; 5])));
        assert_eq!(cache.get(&a, Theme::Dark).map(|kept| kept.map(|svg| svg.len())), Some(Ok(5)), "replaced, the light one as it was");
        assert_eq!(held(), (2, 11 + 6));
    }

    /// A diagram drawn for dark paper is merman's in Mermaid's dark theme, with nothing behind it,
    /// where the light one is on white; and kept, each theme drawn once. It is about as large as the
    /// light one, never smaller and at most a seventh larger, whatever the kind of diagram: the dark
    /// theme lays some out a little larger (a class diagram a tenth wider), which is why a dark
    /// drawing is fitted into the light one's room ([`in_the_room_of`]).
    ///
    /// Opened, a diagram is the light drawing wherever it is shown by itself; a render asked for dark
    /// paper is the dark one, with nothing behind it, in a frame the light one's size: the class
    /// diagram is drawn smaller in the middle of it, its shape kept, so the frame's top and bottom
    /// are clear and its width is used.
    #[test]
    fn a_diagram_for_dark_paper_is_in_the_dark_theme_on_nothing_in_the_light_ones_room() {
        let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
        static STARTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        static SERVICE: Service = Service::new(|| {
            STARTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            worker::spawn(&DIAGRAMS)
        });
        let cache = Cache::new(MAX_CACHED_BYTES, MAX_CACHED);
        let size = |svg: &[u8]| {
            let svg = std::str::from_utf8(svg).unwrap();
            view_box(&svg[..start_tag_end(svg.as_bytes()).unwrap()]).map(|[_, _, width, height]| (width, height))
        };
        const FLOW: &str = "flowchart LR\n  a[Start] --> b{Choose}\n  b -->|yes| c[Finish]\n";
        const CLASS: &str = "classDiagram\n  class Animal {\n    +name string\n    +speak()\n  }\n  Animal <|-- Dog\n";
        let mut larger = false;
        for diagram in [
            FLOW,
            "sequenceDiagram\n  Alice->>Bob: Hello\n  Note over Bob: A note\n  Bob-->>Alice: Hi\n",
            CLASS,
            "stateDiagram-v2\n  [*] --> Still\n  Still --> Moving\n",
            "pie title Pets\n  \"Dogs\" : 3\n  \"Cats\" : 1\n",
            "gantt\n  dateFormat YYYY-MM-DD\n  section A\n  One :a, 2026-01-01, 3d\n",
            "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n",
        ] {
            let source = Arc::new(diagram.as_bytes().to_vec());
            let light = drawn(&SERVICE, &cache, &source, Theme::Light).unwrap();
            let dark = drawn(&SERVICE, &cache, &source, Theme::Dark).unwrap();
            let white = b"fill=\"white\"/>";
            assert!(light.windows(white.len()).any(|w| w == white) && !dark.windows(white.len()).any(|w| w == white), "{diagram}");
            assert_ne!(light, dark, "{diagram}");
            // The dark theme's own background for a node, which the default theme never uses.
            let fills = |svg: &[u8]| String::from_utf8_lossy(svg).to_lowercase().contains("#1f2020");
            assert!(!fills(&light) && (fills(&dark) || !diagram.starts_with("flowchart")), "{diagram}");
            let ((light_w, light_h), (dark_w, dark_h)) = (size(&light).unwrap(), size(&dark).unwrap());
            let about = |light: f64, dark: f64| dark >= light && dark <= light * 8.0 / 7.0;
            assert!(about(light_w, dark_w) && about(light_h, dark_h), "{diagram}: {light_w} × {light_h} and {dark_w} × {dark_h}");
            larger |= (dark_w, dark_h) != (light_w, light_h);
            assert!(Arc::ptr_eq(&drawn(&SERVICE, &cache, &source, Theme::Dark).unwrap(), &dark), "kept");
            assert!(Arc::ptr_eq(&drawn(&SERVICE, &cache, &source, Theme::Light).unwrap(), &light), "kept");
        }
        assert!(larger, "none is larger in the dark theme: `in_the_room_of` has nothing left to fit");
        assert_eq!(STARTS.load(std::sync::atomic::Ordering::SeqCst), 1, "one worker for both themes");
        // A diagram's own theme is its own on dark paper too, as on a web page whose theme is dark.
        let own = Arc::new(b"%%{init: {\"theme\": \"forest\"}}%%\nflowchart LR\n  a --> b\n".to_vec());
        let plain = Arc::new(b"flowchart LR\n  a --> b\n".to_vec());
        let dark = |source: &Arc<Vec<u8>>| drawn(&SERVICE, &cache, source, Theme::Dark).unwrap();
        assert_ne!(dark(&own), dark(&plain));

        // Opened as the backend opens it, from what this test's worker drew: put where `open` looks,
        // so that no other worker is started for it.
        let frames = |diagram: &'static str| {
            let source = Arc::new(diagram.as_bytes().to_vec());
            for theme in [Theme::Light, Theme::Dark] {
                CACHE.put(&source, theme, drawn(&SERVICE, &cache, &source, theme));
            }
            let again: Again = Arc::new(move || Ok(diagram.as_bytes().to_vec()));
            let input = Input::held("a.mermaid".into(), Vec::new(), again);
            let Ok(Opened::Pages(diagram)) = Mermaid.open(input, &Parsed::default()) else { panic!("not pages") };
            assert_eq!(diagram.pages(), 1);
            let mut renderer = diagram.renderer().unwrap();
            let light = renderer.render(Request { scale: 2.0, tag: 7, ..Request::default() });
            let dark = renderer.render(Request { scale: 2.0, tag: 8, on_dark: true, dark: [30, 30, 30], ..Request::default() });
            assert!(light.error.is_none() && dark.error.is_none(), "{:?} {:?}", light.error, dark.error);
            assert_eq!((light.request.tag, dark.request.tag, dark.request.scale), (7, 8, 2.0), "the request as it was asked");
            let sizes = ((light.width, light.height), (dark.width, dark.height));
            assert!(light.width.abs_diff(dark.width) <= 1 && light.height.abs_diff(dark.height) <= 1, "{sizes:?}");
            (light, dark)
        };
        let pixels = |frame: &Frame| frame.rgba.as_chunks::<4>().0.to_vec();
        let (light, dark) = frames(FLOW);
        assert_eq!((pixels(&light).first(), pixels(&dark).first()), (Some(&[255; 4]), Some(&[0; 4])), "white behind one, nothing behind the other");
        assert!(pixels(&dark).iter().all(|pixel| *pixel != [255; 4]), "no white in the dark theme");
        assert!(pixels(&dark).iter().any(|pixel| pixel[3] == 255 && pixel[0] > 150), "its ink is light");
        let (light, dark) = frames(CLASS);
        let (width, height) = (dark.width as usize, dark.height as usize);
        let dark = pixels(&dark);
        let column = |x: usize| (0..height).filter_map(|y| dark.get(y * width + x)).any(|pixel| pixel[3] != 0);
        let row = |y: usize| (0..width).filter_map(|x| dark.get(y * width + x)).any(|pixel| pixel[3] != 0);
        // A tenth wider and a thirty-third taller: fitted to the width, some 4% of the height clear at
        // the top and the bottom, and its ink reaching the sides as the light one's does.
        assert!(!row(0) && !row(height / 50) && !row(height - 1 - height / 50), "clear above and below");
        assert!((0..height / 8).any(row) && (height - height / 8..height).any(row), "its height used");
        assert!((0..width / 8).any(column) && (width - width / 8..width).any(column), "its width used");
        assert!(pixels(&light).iter().all(|pixel| pixel[3] == 255), "the light one fills its own");
        assert_eq!(STARTS.load(std::sync::atomic::Ordering::SeqCst), 1, "no worker but this test's");
        SERVICE.turn().link = None;
    }

    /// A frame is put in the middle of a larger one, nothing around it; and left as it is where its
    /// pixels aren't what its size says.
    #[test]
    fn a_frame_is_centred_in_a_larger_one() {
        let frame = |width, height, rgba: Vec<u8>| Frame {
            request: Request::default(),
            width,
            height,
            rgba: Pixels::new(rgba),
            error: None,
            warnings: vec!["kept".into()],
            again: false,
        };
        let centred = super::centred(frame(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]), 4, 3);
        assert_eq!((centred.width, centred.height, centred.warnings.len()), (4, 3, 1));
        let expected = [[0u8; 16].as_slice(), &[0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0], &[0; 16]].concat();
        assert_eq!(&*centred.rgba, expected.as_slice());
        let odd = super::centred(frame(2, 1, vec![1, 2, 3]), 4, 3);
        assert_eq!((odd.width, odd.height, odd.rgba.len()), (2, 1, 3));
        let none = super::centred(frame(0, 0, Vec::new()), 4, 3);
        assert_eq!((none.width, none.height), (0, 0));
    }

    #[test]
    fn a_reason_is_cut_at_a_character() {
        assert_eq!(cut("abc", 2), "ab");
        assert_eq!(cut("aé", 2), "a");
        assert_eq!(cut("é", 0), "");
        assert_eq!(cut("abc", 10), "abc");
    }
}
