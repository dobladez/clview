//! Mermaid diagrams (`.mmd`, `.mermaid`), turned into SVG by merman, a pure-Rust Mermaid (its
//! parser, its Cytoscape layout and its SVG), and shown by whichever backend opens that SVG, the
//! SVG backend, never named here: this one draws nothing itself. The one file that names merman
//! ([ADR-0030](../../docs/adr/0030-mermaid-diagrams-through-merman-in-a-worker-process.md)).
//!
//! A diagram is untrusted, and what laying it out costs can't be bounded from its size, so merman
//! works in a worker process of clv's own ([`DIAGRAMS`]), capped on CPU time and heap, after its own
//! limits (its `Constrained` profile, Mermaid's 500 edges and a deadline). One worker serves the
//! backend's diagrams in turn ([`Service`]): started when one is first wanted, started again after
//! it dies, killed once idle, and not started for a while after a few killed in a row. What it
//! answers is kept ([`Cache`]), by the diagram's bytes, so a diagram is turned into SVG once
//! however often it is opened: a document's picture is opened to size it and again to draw it.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use merman::model::MermaidConfig;
use merman::model::resources::{InputResourcePolicy, ResourceProfile};
use merman::{OperationControl, RenderOutput, RenderRequest, Renderer, SemanticParser, SvgRequest};

use super::worker::{self, Died, FAILED, Link, Session, WALL, Worker};
use super::{Again, BACKENDS, Backend, Format, HEAD, Input, Opened};
use crate::args::{OptionSpec, Parsed};

/// Mermaid diagrams, turned into SVG by merman.
pub(super) struct Mermaid;

/// What it shows, by name alone: a `.mermaid` that is text, and a `.mmd` that starts as a diagram
/// ([`Kind::of`]).
const FORMAT: Format = Format { name: "Mermaid", extensions: &["mmd", "mermaid"] };

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

    fn open(&self, input: Input, options: &Parsed<'_>) -> Result<Opened, String> {
        let path = input.path.clone();
        let error = |e: String| format!("{path}: {e}");
        let Some(bytes) = input.at_most(MAX_SOURCE)? else {
            return Err(error(format!("the diagram is more than {} MiB", MAX_SOURCE >> 20)));
        };
        let svg = drawn(&SERVICE, &CACHE, bytes).map_err(error)?;
        let head = svg.get(..HEAD as usize).unwrap_or(&svg).to_vec();
        let again: Again = Arc::new(move || Ok(svg.to_vec()));
        let drawing = Input::held(path, head, again);
        // Not this one: what merman draws is never a diagram.
        let Some(backend) = BACKENDS.iter().filter(|b| b.name() != self.name()).find(|b| b.opens(&drawing.path, &drawing.head)) else {
            return Err(format!("{}: a diagram is shown as the SVG merman draws, and no backend opens that", drawing.path));
        };
        backend.open(drawing, options)
    }
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
    let head = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    let text = match std::str::from_utf8(head) {
        Ok(text) => text,
        // What is UTF-8 of it, up to a character cut by its end or one that isn't.
        Err(e) => head.get(..e.valid_up_to()).and_then(|valid| std::str::from_utf8(valid).ok()).unwrap_or_default(),
    };
    // A file shorter than the head ends with its last line; otherwise that line may go on.
    let whole = head.len() < HEAD as usize && text.len() == head.len();
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

/// Its SVG: an answer.
const DRAWN: u8 = 2;

/// Why merman refused it, in its words: an answer.
const REFUSED: u8 = 3;

/// Why a worker answered [`FAILED`]: what it was asked isn't a request.
const MALFORMED: u8 = 1;

/// The CPU seconds a diagram may take in the worker: about ten times the costliest diagram merman
/// accepts (ADR-0030). The viewer waits [`WALL`] times as long.
const DRAW_CPU: u64 = 2;

/// How long merman works on a diagram before it stops itself, with its own message: before the
/// kernel stops the worker at [`DRAW_CPU`].
const DEADLINE: Duration = Duration::from_millis(1_500);

/// The most the worker's heap holds: twice the costliest diagram merman accepts (ADR-0030).
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
        (kind == DRAW).then_some((DRAW_CPU, MAX_SOURCE as usize))
    }

    fn heap(&self) -> usize {
        MAX_HEAP
    }

    fn session(&self) -> Box<dyn Session> {
        Box::new(Drawing(None))
    }
}

/// What the worker keeps between diagrams: merman's renderer, made for the first (some 40 ms).
struct Drawing(Option<Renderer>);

impl Session for Drawing {
    fn answer(&mut self, kind: u8, payload: Vec<u8>) -> (u8, Vec<u8>) {
        let Ok(text) = String::from_utf8(payload) else { return (FAILED, vec![MALFORMED]) };
        if kind != DRAW {
            return (FAILED, vec![MALFORMED]);
        }
        let renderer = self.0.get_or_insert_with(renderer);
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
/// edges among them); within merman's `Constrained` limits.
fn renderer() -> Renderer {
    let config = serde_json::json!({
        "htmlLabels": false,
        "flowchart": { "htmlLabels": false },
        "maxEdges": MAX_EDGES,
        "secure": ["secure", "securityLevel", "startOnLoad", "maxTextSize", "suppressErrorRendering", "maxEdges", "htmlLabels"],
    });
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

/// The SVG `source` is drawn to, from `cache` or from `service`'s worker; or why not, for the reader.
fn drawn(service: &'static Service, cache: &Cache, source: Vec<u8>) -> Result<Arc<Vec<u8>>, String> {
    if std::str::from_utf8(&source).is_err() {
        return Err("the diagram isn't UTF-8 text".into());
    }
    if let Some(kept) = cache.get(&source) {
        return kept;
    }
    // Its turn: diagrams opened at once wait for it here, and one drawn meanwhile is in the cache.
    let mut turn = service.turn();
    if let Some(kept) = cache.get(&source) {
        return kept;
    }
    let source = Arc::new(source);
    match service.ask(&mut turn, source.clone()) {
        Ok(svg) => {
            let svg = Arc::new(on_white(svg).into_bytes());
            cache.put(&source, Ok(svg.clone()));
            Ok(svg)
        }
        Err(Failed::Kept(why)) => {
            cache.put(&source, Err(why.clone()));
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

    /// The worker's SVG for `source`, a diagram's UTF-8 text, in `state`, the turn held; a worker
    /// started first if there is none.
    fn ask(&'static self, state: &mut State, source: Arc<Vec<u8>>) -> Result<String, Failed> {
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
        let asked = link.request_shared(DRAW, source, wall, MAX_SVG);
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

/// What the worker answered for each diagram, by its bytes: its SVG, or why it failed. At most
/// `bytes` of diagrams and SVG and `most` diagrams, those used longest ago let go of first.
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

    /// What `source` came to, if it is kept: found by its hash, taken by its bytes.
    fn get(&self, source: &[u8]) -> Option<Result<Arc<Vec<u8>>, String>> {
        let hash = hashed(source);
        let mut kept = self.lock();
        let at = kept.entries.iter().position(|entry| entry.hash == hash && entry.source.as_slice() == source)?;
        let entry = kept.entries.remove(at);
        let drawn = entry.drawn.clone();
        kept.entries.push(entry);
        Some(drawn)
    }

    /// `source` kept as having come to `drawn`, those used longest ago let go of to make room. One
    /// larger than the whole cache isn't kept.
    fn put(&self, source: &Arc<Vec<u8>>, drawn: Result<Arc<Vec<u8>>, String>) {
        let entry = Entry { hash: hashed(source), source: source.clone(), drawn };
        let size = entry.bytes();
        if size > self.bytes {
            return;
        }
        let mut kept = self.lock();
        if let Some(at) = kept.entries.iter().position(|e| e.hash == entry.hash && e.source == entry.source) {
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
        cache.put(&a, svg(10));
        cache.put(&b, Err("refused".into()));
        cache.put(&c, svg(10));
        assert_eq!(cache.get(&b), Some(Err("refused".into())), "a failure kept");
        assert!(cache.get(&a).is_some(), "a used: b and c are older");
        cache.put(&d, svg(10));
        assert!(cache.get(&b).is_some() && cache.get(&c).is_none(), "three at most: c went, used longest ago");
        cache.put(&c, svg(85));
        assert!(cache.get(&c).is_some() && cache.get(&d).is_none() && cache.get(&a).is_none(), "100 bytes at most");
        assert_eq!(cache.lock().bytes, 86 + 8);
        cache.put(&a, svg(100));
        assert!(cache.get(&a).is_none() && cache.get(&c).is_some(), "larger than the cache: not kept");
        cache.put(&c, svg(1));
        assert_eq!(cache.get(&c).map(|kept| kept.map(|svg| svg.len())), Some(Ok(1)), "replaced");
        assert_eq!(cache.lock().bytes, 2 + 8);
        // Found by its bytes, not its hash alone.
        let e = source("e");
        cache.lock().entries.push(Entry { hash: hashed(&e), source: source("f"), drawn: svg(1) });
        assert!(cache.get(&e).is_none());
    }

    /// A diagram is drawn once: asked again, it is the cache's. merman's refusal is kept too, but
    /// not a worker that couldn't be started, which may be later.
    #[test]
    fn what_the_worker_answers_is_kept_but_for_no_worker() {
        static STARTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        static SERVICE: Service = Service::new(|| {
            STARTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            worker::spawn(&DIAGRAMS)
        });
        let cache = Cache::new(MAX_CACHED_BYTES, MAX_CACHED);
        let flow = b"flowchart LR\n  a --> b\n".to_vec();
        let svg = drawn(&SERVICE, &cache, flow.clone()).unwrap();
        assert!(svg.starts_with(b"<svg"), "{:?}", String::from_utf8_lossy(&svg[..40]));
        SERVICE.turn().link = None;
        assert!(Arc::ptr_eq(&drawn(&SERVICE, &cache, flow).unwrap(), &svg), "kept");
        assert_eq!(STARTS.load(std::sync::atomic::Ordering::SeqCst), 1, "no worker asked");
        let broken = b"flowchart TD\n  A[[[ (((\n".to_vec();
        let why = drawn(&SERVICE, &cache, broken.clone()).unwrap_err();
        assert!(why.starts_with("merman: ") && why.len() <= MAX_WHY + 8, "{why}");
        assert_eq!(cache.get(&broken), Some(Err(why)));
        assert_eq!(drawn(&SERVICE, &cache, b"\xff".to_vec()), Err("the diagram isn't UTF-8 text".into()));
        static FULL: Service = Service::new(|| Err(Died::TooMany));
        let pie = b"pie\n  \"a\" : 1\n".to_vec();
        let why = drawn(&FULL, &cache, pie.clone()).unwrap_err();
        assert!(why.starts_with("too many of clv's worker processes at once"), "{why}");
        assert_eq!(cache.get(&pie), None, "not kept");
    }

    /// The worker is killed once idle, 2 s after its last answer, and not before; the next diagram
    /// starts another.
    #[test]
    fn the_worker_is_gone_two_seconds_after_its_answer() {
        static SERVICE: Service = Service::new(|| worker::spawn(&DIAGRAMS));
        let cache = Cache::new(MAX_CACHED_BYTES, MAX_CACHED);
        let alive = || SERVICE.turn().link.is_some();
        drawn(&SERVICE, &cache, b"flowchart LR\n  a --> b\n".to_vec()).unwrap();
        let answered = Instant::now();
        assert!(alive());
        std::thread::sleep(Duration::from_millis(1_500));
        assert!(alive(), "still alive 1.5 s after its answer");
        while alive() {
            assert!(answered.elapsed() < Duration::from_secs(4), "still alive");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(answered.elapsed() >= IDLE, "{:?}", answered.elapsed());
        drawn(&SERVICE, &cache, b"flowchart LR\n  c --> d\n".to_vec()).unwrap();
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
            assert_eq!(drawn(&SERVICE, &cache, source.clone()), Err("merman failed on the diagram".into()));
            assert_eq!(cache.get(&source), Some(Err("merman failed on the diagram".into())), "kept");
        }
        assert_eq!(STARTS.load(std::sync::atomic::Ordering::SeqCst), 3, "one a diagram");
        let why = drawn(&SERVICE, &cache, b"pie\n".to_vec()).unwrap_err();
        assert!(why.contains("none is drawn for a minute"), "{why}");
        assert_eq!(STARTS.load(std::sync::atomic::Ordering::SeqCst), 3, "paused");
        assert_eq!(cache.get(b"pie\n"), None, "not kept: it may pass");
        // Past the pause, a worker is started again.
        SERVICE.turn().paused = Instant::now().checked_sub(PAUSE);
        assert_eq!(drawn(&SERVICE, &cache, b"pie\n".to_vec()), Err("merman failed on the diagram".into()));
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

    #[test]
    fn a_reason_is_cut_at_a_character() {
        assert_eq!(cut("abc", 2), "ab");
        assert_eq!(cut("aé", 2), "a");
        assert_eq!(cut("é", 0), "");
        assert_eq!(cut("abc", 10), "abc");
    }
}
