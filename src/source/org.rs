//! Org-mode documents, written as HTML and shown by whichever backend opens that HTML, the
//! documents' backend, never named here, as Markdown is shown: this one lays nothing out itself.
//! So an Org file has what a Markdown file has: pages the window's size, its settings, a search, its
//! pictures, its fenced blocks drawn, its links. The one file that names orgize
//! ([ADR-0040](../../docs/adr/0040-org-mode-through-orgize-in-a-worker-process.md)).
//!
//! An Org file is untrusted, and orgize, which parses it, takes time that grows as the square of
//! what a file nests or leaves open, and overflows a stack on what it nests deepest. So orgize
//! works in a worker process of clv's own ([`DOCUMENTS`]), capped on CPU time and heap, which is
//! given the Org text, held to the documents' limits first, and answers the HTML clv writes of it
//! ([`html_of`]): the parse and the writing both happen there. The HTML is clv's: what Org shows,
//! what it hides, and how its places become `id`s.
//!
//! The HTML is handed on as a file held here, read again (and so written again, by a worker, but
//! for what [`CACHE`] keeps) each time the documents' backend reads it, for a relayout; its
//! pictures are read beside the Org file: in its directory, or in the file holding it.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Read as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use orgize::rowan::ast::AstNode;
use orgize::rowan::{GreenNode, NodeOrToken};
use orgize::{SyntaxElement, SyntaxKind, SyntaxNode};

use super::worker::{self, Died, FAILED, Session, WALL, Worker};
use super::{
    Again, Auto, BACKENDS, Backend, Deferred, Effect, Format, HEAD, Input, Keys, Named, Near, Numbers, Opened, Reopenable, Room, Setting, Value,
    opener,
};
use crate::args::{OptionSpec, Parsed};

/// Org-mode documents, parsed by orgize.
pub(super) struct Org;

/// What it shows: text named `.org` or `.org_archive`, or whose first line says Org ([`opens`]);
/// a document laid out for the window, with its settings ([`SETTINGS`]).
const FORMAT: Format = Format::of("Org", &["org", "org_archive"]).having(SETTINGS);

/// An Org document's paper: as the documents' backend declares its own, under the same name,
/// values, key and option, so that it is one setting with theirs, which the viewer holds one value
/// of ([ADR-0033](../../docs/adr/0033-backends-share-a-setting-by-declaring-its-name.md)). The
/// document is laid out and drawn by the backend that opens its HTML, which reads the values
/// itself: these say only that an Org document has them, before it is opened.
const PAPER: Setting = Setting {
    name: "paper",
    named: &[Named { name: "light", says: "dark on light" }, Named { name: "dark", says: "light on dark" }],
    numbers: None,
    default: Value::Named("light"),
    zero: Value::Named("light"),
    auto: Some(Auto { light: "light", dark: "dark" }),
    keys: Keys::Cycle('D'),
    option: OptionSpec {
        name: "--theme",
        short: None,
        value: Some("auto|light|dark"),
        help: "an Org document light on dark or dark on light (auto: as the terminal is)",
        alias: None,
    },
    help: "light on dark, or dark on light",
    does: "go dark",
    effect: Effect::Redraw,
    sets_width: false,
};

/// How wide an Org document's line is: as [`PAPER`], the documents' own setting.
const WIDTH: Setting = Setting {
    name: "width",
    named: &[Named { name: "window", says: "width window" }],
    numbers: Some(Numbers { least: 20, most: 200, step: 4, unit: "" }),
    default: Value::Named("window"),
    zero: Value::Named("window"),
    auto: None,
    keys: Keys::Step('[', ']'),
    option: OptionSpec {
        name: "--width",
        short: None,
        value: Some("window|N"),
        help: "an Org document laid out to lines of N characters of code (20-200), or as wide as the window",
        alias: None,
    },
    help: "4 characters a line narrower, or wider; 72] sets 72, 0] the window's",
    does: "reflow",
    effect: Effect::Relayout,
    sets_width: true,
};

/// How large an Org document's text is: as [`PAPER`], the documents' own setting.
const FONT: Setting = Setting {
    name: "font",
    named: &[],
    numbers: Some(Numbers { least: 50, most: 400, step: 10, unit: "%" }),
    default: Value::Number(100),
    zero: Value::Number(100),
    auto: None,
    keys: Keys::Zoom,
    option: OptionSpec { name: "--font", short: None, value: Some("N"), help: "an Org document's text at N% of its size (50-400)", alias: None },
    help: "text larger, or smaller; 150+ sets 150%, 0+ 100%",
    does: "have a font size",
    effect: Effect::Relayout,
    sets_width: false,
};

/// How wide an Org document's side margins are: as [`PAPER`], the documents' own setting.
const MARGIN: Setting = Setting {
    name: "margin",
    named: &[],
    numbers: Some(Numbers { least: 0, most: 20, step: 1, unit: "" }),
    default: Value::Number(3),
    zero: Value::Number(3),
    auto: None,
    keys: Keys::Step('{', '}'),
    option: OptionSpec {
        name: "--margin",
        short: None,
        value: Some("N"),
        help: "an Org document's side margins of N characters of code (0-20)",
        alias: None,
    },
    help: "margins a character narrower, or wider; 5} sets 5, 0} back to 3",
    does: "have margins",
    effect: Effect::Relayout,
    sets_width: false,
};

/// Whether an Org document's source blocks are drawn: as [`PAPER`], the documents' own setting.
const FENCES: Setting = Setting {
    name: "fences",
    named: &[Named { name: "drawn", says: "blocks drawn" }, Named { name: "code", says: "blocks as code" }],
    numbers: None,
    default: Value::Named("drawn"),
    zero: Value::Named("drawn"),
    auto: None,
    keys: Keys::Cycle('s'),
    option: OptionSpec {
        name: "--fences",
        short: None,
        value: Some("drawn|code"),
        help: "an Org document's source blocks drawn as pictures (#+begin_src mermaid), or shown as code",
        alias: None,
    },
    help: "fenced blocks drawn, or as code",
    does: "draw fenced blocks",
    effect: Effect::Relayout,
    sets_width: false,
};

/// The settings an Org document has: a document's, laid out for the window.
const SETTINGS: &[&str] = &[PAPER.name, WIDTH.name, FONT.name, MARGIN.name, FENCES.name];

impl Backend for Org {
    fn name(&self) -> &'static str {
        "org"
    }

    fn shows(&self) -> &'static str {
        "Org documents"
    }

    fn formats(&self) -> Vec<Format> {
        vec![FORMAT]
    }

    fn credits(&self) -> &[&'static str] {
        &["orgize"]
    }

    fn options(&self) -> &'static [OptionSpec] {
        &[]
    }

    fn settings(&self) -> &'static [Setting] {
        &[PAPER, WIDTH, FONT, MARGIN, FENCES]
    }

    fn opens(&self, path: &str, head: &[u8]) -> bool {
        opens(path, head)
    }

    /// It parses the document in a worker and lays the whole of it out to open it: a list's tile
    /// opens it when wanted.
    fn deferred(&self, _path: &str, _head: &[u8]) -> Option<Deferred> {
        Some(Deferred { settings: SETTINGS })
    }

    /// Laid out for a room of the documents' backend's own: the viewer gives one
    /// ([`Backend::open_at`]).
    fn open(&self, input: Input, options: &Parsed<'_>) -> Result<Opened, String> {
        opened(input, options, None, convert)
    }

    fn open_at(&self, input: Input, options: &Parsed<'_>, room: Room) -> Result<Opened, String> {
        opened(input, options, Some(room), convert)
    }
}

/// Whether the file at `path`, which starts with `head`, is an Org document: text, named `.org` or
/// `.org_archive` in any case, or whose first line (its second, where the first starts with `#!`)
/// holds an Emacs mode line naming Org ([`says_org`]), whatever its name.
fn opens(path: &str, head: &[u8]) -> bool {
    is_text(head) && (FORMAT.named(path) || says_org(head))
}

/// The byte order marks of UTF-8, UTF-16 little-endian and big-endian.
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";
const UTF16_LE: &[u8] = b"\xFF\xFE";
const UTF16_BE: &[u8] = b"\xFE\xFF";

/// Whether `head`, a file's first bytes, is text: UTF-16 by its byte order mark; otherwise no NUL,
/// and no more than one in a hundred of the control characters text doesn't have (all but tab,
/// line feed, form feed, carriage return, `^Z` and escape), as the documents' backend tells it.
fn is_text(head: &[u8]) -> bool {
    if head.starts_with(UTF16_LE) || head.starts_with(UTF16_BE) {
        return true;
    }
    if head.contains(&0) {
        return false;
    }
    let odd = |b: &&u8| (**b < 0x20 && !matches!(**b, 0x09 | 0x0A | 0x0C | 0x0D | 0x1A | 0x1B)) || **b == 0x7F;
    head.iter().filter(odd).count().saturating_mul(100) <= head.len()
}

/// Whether `head` says it is Org on its first line, or its second where the first starts with `#!`
/// (a script's): an Emacs mode line, `-*- org -*-`, or `-*- mode: org; … -*-` among its
/// `;`-separated variables, in any case, as the Emacs manual's "Specifying File Variables" writes
/// them. Read as UTF-8 or as UTF-16 by its byte order mark, as far as `head` goes.
fn says_org(head: &[u8]) -> bool {
    let text = head_text(head);
    let mut lines = text.split('\n');
    let first = lines.next().unwrap_or_default();
    let line = if first.starts_with("#!") { lines.next().unwrap_or_default() } else { first };
    let Some((_, rest)) = line.split_once("-*-") else { return false };
    let Some((inside, _)) = rest.split_once("-*-") else { return false };
    if !inside.contains(':') {
        return inside.trim().eq_ignore_ascii_case("org");
    }
    inside
        .split(';')
        .filter_map(|variable| variable.split_once(':'))
        .any(|(name, value)| name.trim().eq_ignore_ascii_case("mode") && value.trim().eq_ignore_ascii_case("org"))
}

/// What of `head`, a file's first bytes, is text: UTF-16 by its byte order mark, else UTF-8 up to a
/// character cut by its end or one that isn't.
fn head_text(head: &[u8]) -> String {
    let units = |bytes: &[u8], le: bool| -> Vec<u16> {
        bytes.as_chunks::<2>().0.iter().map(|pair| if le { u16::from_le_bytes(*pair) } else { u16::from_be_bytes(*pair) }).collect()
    };
    if let Some(rest) = head.strip_prefix(UTF16_LE) {
        return char::decode_utf16(units(rest, true)).map_while(Result::ok).collect();
    }
    if let Some(rest) = head.strip_prefix(UTF16_BE) {
        return char::decode_utf16(units(rest, false)).map_while(Result::ok).collect();
    }
    let head = head.strip_prefix(UTF8_BOM).unwrap_or(head);
    match std::str::from_utf8(head) {
        Ok(text) => text.to_owned(),
        Err(e) => head.get(..e.valid_up_to()).and_then(|valid| std::str::from_utf8(valid).ok()).unwrap_or_default().to_owned(),
    }
}

/// `bytes`, an Org file's, as text: UTF-16 by its byte order mark, else UTF-8, a byte that isn't
/// made the replacement character; the byte order mark left out.
fn decoded(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], le: bool| {
        let units = rest.as_chunks::<2>().0.iter().map(|pair| if le { u16::from_le_bytes(*pair) } else { u16::from_be_bytes(*pair) });
        char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
    };
    if let Some(rest) = bytes.strip_prefix(UTF16_LE) {
        return utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(UTF16_BE) {
        return utf16(rest, false);
    }
    String::from_utf8_lossy(bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes)).into_owned()
}

/// The largest Org file read, and the most lines and marks it may have (each line's end and each
/// ASCII punctuation mark, which may start something of Org's): Markdown's, the documents' limits
/// ([ADR-0012](../../docs/adr/0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md)),
/// counted before it is parsed. They bound the HTML written too, which the documents' backend then
/// holds to HTML's.
const MAX_SOURCE: usize = 64 << 20;
const MAX_MARKS: usize = 500_000;

/// The most HTML written of an Org document: what the documents' backend reads of HTML.
const MAX_HTML: usize = 64 << 20;

/// The longest reason given for refusing a document, in bytes: cut there, at a character.
const MAX_WHY: usize = 256;

/// `bytes`, an Org file's, as the text the worker is given, if it is within the documents' limits.
fn within(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_SOURCE {
        return Err(format!("the document is more than {} MiB", MAX_SOURCE >> 20));
    }
    let text = decoded(bytes);
    if text.len() > MAX_SOURCE {
        return Err(format!("the document is more than {} MiB", MAX_SOURCE >> 20));
    }
    if text.bytes().filter(|b| b.is_ascii_punctuation() || matches!(b, b'\r' | b'\n')).count() > MAX_MARKS {
        return Err(format!("the document has more than {MAX_MARKS} lines and marks"));
    }
    Ok(text)
}

/// How an Org text is written as HTML: in a worker ([`convert`]), but in the tests.
type Convert = fn(Vec<u8>) -> Result<(u8, Vec<u8>), Died>;

/// Where an Org file's bytes are, to read them again.
#[derive(Clone)]
enum Kept {
    /// On disk, by its path.
    Disk(PathBuf),
    /// As they came through a pipe.
    Memory(Arc<Vec<u8>>),
    /// In another file (an archive), read whole from it each time.
    Held(Again),
}

impl Kept {
    /// The file's bytes, read again, within [`MAX_SOURCE`].
    fn bytes(&self) -> Result<Arc<Vec<u8>>, String> {
        let bytes = match self {
            Self::Disk(path) => {
                let file = std::fs::File::open(path).map_err(|e| format!("reading the file again: {e}"))?;
                let mut bytes = Vec::new();
                let most = u64::try_from(MAX_SOURCE).unwrap_or(u64::MAX).saturating_add(1);
                file.take(most).read_to_end(&mut bytes).map_err(|e| format!("reading the file again: {e}"))?;
                Arc::new(bytes)
            }
            Self::Memory(bytes) => bytes.clone(),
            Self::Held(again) => Arc::new(again()?),
        };
        match bytes.len() > MAX_SOURCE {
            true => Err(format!("the document is more than {} MiB", MAX_SOURCE >> 20)),
            false => Ok(bytes),
        }
    }

    /// The HTML of the file's text, as it reads now ([`html`]).
    fn html(&self, convert: Convert) -> Result<Arc<Vec<u8>>, String> {
        html(&self.bytes()?, convert, &CACHE)
    }
}

/// The Org document `input` is, given `options`, laid out for `room` if one is given: its text
/// written as HTML through `convert`, and that HTML opened by the backend that shows it, found as
/// any file's backend is, by how it starts, never by name.
fn opened(input: Input, options: &Parsed<'_>, room: Option<Room>, convert: Convert) -> Result<Opened, String> {
    let path = input.path.clone();
    let error = |e: String| format!("{path}: {e}");
    // What is beside it: in the file holding it, if it is held in one that says; else on disk.
    let beside = input.near().map(|beside| (beside.0.clone(), beside.1.clone())).or_else(|| on_disk(&input));
    let kept = match input.reopenable()? {
        Reopenable::Disk(_) => Kept::Disk(PathBuf::from(&path)),
        Reopenable::Bytes(bytes) => Kept::Memory(Arc::new(bytes)),
        Reopenable::Again(again) => Kept::Held(again),
    };
    let html = kept.html(convert).map_err(error)?;
    let head = html.get(..HEAD as usize).unwrap_or(&html).to_vec();
    let again: Again = Arc::new(move || kept.html(convert).map(|html| html.to_vec()));
    let mut document = Input::held(path.clone(), head, again);
    if let Some((near, from_top)) = beside {
        document = document.held_near(near, from_top);
    }
    // Not this one: what is written is never Org.
    let others: Vec<&dyn Backend> = BACKENDS.iter().copied().filter(|b| b.name() != Org.name()).collect();
    let Some(backend) = opener(&others, &document.path, &document.head) else {
        return Err(error("an Org document is shown as the HTML it is written as, and no backend opens that".into()));
    };
    let shown = match room {
        Some(room) => backend.open_at(document, options, room),
        None => backend.open(document, options),
    };
    match shown.map_err(|e| as_html(&path, &e))? {
        Opened::Pages(pages) => Ok(Opened::Pages(pages)),
        Opened::Files(_) => Err(error("its HTML holds files".into())),
    }
}

/// Why the HTML an Org document at `path` is written as wasn't shown, `why`, as the backend that
/// opens it said it: said to be of the HTML, so that a file within Org's limits and refused by
/// HTML's says which.
fn as_html(path: &str, why: &str) -> String {
    let why = why.strip_prefix(path).and_then(|rest| rest.strip_prefix(": ")).unwrap_or(why);
    format!("{path}: as the HTML it is written as, {why}")
}

/// What is beside the Org file `input` is, if it is a file on disk: any file, by its path from the
/// root of the file system, as a held file's are by theirs from its holder's top
/// ([ADR-0037](../../docs/adr/0037-what-is-beside-a-held-file-is-read-from-its-holder.md)), and
/// the document's own; so that a picture's path is read from the document's directory, `..`
/// included, as a document's on disk are.
fn on_disk(input: &Input) -> Option<(Near, String)> {
    let real = input.on_disk().ok()?;
    let from_top = real.to_str()?.strip_prefix('/')?.to_owned();
    let near: Near = Arc::new(|path: &str| Input::open(&format!("/{path}")));
    Some((near, from_top))
}

/// The HTML of the Org file `bytes`, from `cache` or written through `convert`; or why not.
fn html(bytes: &Arc<Vec<u8>>, convert: Convert, cache: &Cache) -> Result<Arc<Vec<u8>>, String> {
    if let Some(html) = cache.get(bytes) {
        return Ok(html);
    }
    let text = within(bytes)?;
    let html = match convert(text.into_bytes()) {
        Ok((WRITTEN, html)) if html.len() <= MAX_HTML => Arc::new(html),
        Ok((REFUSED, why)) if why.len() <= MAX_WHY => return Err(String::from_utf8_lossy(&why).into_owned()),
        Ok(_) => return Err(said(&Died::Malformed)),
        Err(died) => return Err(said(&died)),
    };
    cache.put(bytes, &html);
    Ok(html)
}

/// Why an Org document wasn't written, in the reader's words, as the worker went: `died`.
fn said(died: &Died) -> String {
    match died {
        Died::TooMany => format!("too many of {}'s worker processes at once: the document isn't read", crate::NAME),
        Died::Unstarted(why) => format!("no process to read the document in ({why})"),
        Died::TimedOut | Died::Cpu => "orgize took too long on the document".into(),
        Died::Memory => "orgize took too much memory on the document".into(),
        Died::Version => format!("{} was replaced while it ran: open the document again", crate::NAME),
        Died::Ended(_) | Died::Malformed => "orgize failed on the document".into(),
    }
}

// The worker's side.

/// An Org text, UTF-8, to write as HTML: the request.
const CONVERT: u8 = 1;

/// Its HTML: an answer.
const WRITTEN: u8 = 2;

/// Why it wasn't written: an answer.
const REFUSED: u8 = 3;

/// Why a worker answered [`FAILED`]: what it was asked isn't a request.
const MALFORMED: u8 = 1;

/// The CPU seconds a document may take in the worker, and the most its heap holds: many times
/// what the costliest real documents measured take, within the documents' limits (ADR-0040). The
/// viewer waits [`WALL`] times as long.
const CONVERT_CPU: u64 = 2;
const MAX_HEAP: usize = 512 << 20;

/// The worker that parses an Org document and writes it as HTML: `clv --worker org`.
pub(crate) struct Documents;

pub(crate) static DOCUMENTS: Documents = Documents;

impl Worker for Documents {
    fn name(&self) -> &'static str {
        "org"
    }

    fn takes(&self, kind: u8) -> Option<(u64, usize)> {
        (kind == CONVERT).then_some((CONVERT_CPU, MAX_SOURCE))
    }

    fn heap(&self) -> usize {
        MAX_HEAP
    }

    fn session(&self) -> Box<dyn Session> {
        Box::new(Writing)
    }
}

/// A worker's answers: it keeps nothing between them.
struct Writing;

impl Session for Writing {
    fn answer(&mut self, kind: u8, payload: Vec<u8>) -> (u8, Vec<u8>) {
        let (CONVERT, Ok(text)) = (kind, String::from_utf8(payload)) else { return (FAILED, vec![MALFORMED]) };
        let home = std::env::var("HOME").ok();
        let written = std::panic::catch_unwind(|| html_of(&text, home.as_deref()));
        match written.unwrap_or_else(|_| Err("orgize failed on the document".into())) {
            Ok(html) => (WRITTEN, html.into_bytes()),
            Err(why) => (REFUSED, cut(&why, MAX_WHY).as_bytes().to_vec()),
        }
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

/// One document written at a time: a list of them takes one worker's place among those alive, and
/// a document waits its turn here.
static TURN: Mutex<()> = Mutex::new(());

/// The worker's answer to `text`, an Org document's: a worker started for it, in its turn, and gone
/// once it has answered.
fn convert(text: Vec<u8>) -> Result<(u8, Vec<u8>), Died> {
    let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
    let mut link = worker::spawn(&DOCUMENTS)?;
    let wall = Duration::from_secs(CONVERT_CPU.saturating_mul(u64::from(WALL)));
    link.request(CONVERT, text, wall, MAX_HTML)
}

/// The most bytes [`CACHE`] keeps, Org files and their HTML together, and the most files.
const MAX_CACHED_BYTES: usize = 64 << 20;
const MAX_CACHED: usize = 64;

/// What the worker wrote of each Org file, by its bytes: so that a document read again, for a
/// relayout or a render, starts no worker.
static CACHE: Cache = Cache { kept: Mutex::new(Vec::new()), bytes: MAX_CACHED_BYTES, most: MAX_CACHED };

/// What was written of Org files: each file's hash, its bytes and its HTML, the most recently used
/// last; at most `bytes` of files and HTML and `most` of them, those used longest ago let go of
/// first.
struct Cache {
    kept: Mutex<Vec<Written>>,
    bytes: usize,
    most: usize,
}

impl Cache {
    /// The HTML written of `org`, if it is kept: found by its hash, taken by its bytes.
    fn get(&self, org: &[u8]) -> Option<Arc<Vec<u8>>> {
        let hash = hashed(org);
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        let at = kept.iter().position(|(h, bytes, _)| *h == hash && bytes.as_slice() == org)?;
        let entry = kept.remove(at);
        let html = entry.2.clone();
        kept.push(entry);
        Some(html)
    }

    /// `html` kept as what `org` is written as, those used longest ago let go of to make room. One
    /// larger than the whole cache isn't kept.
    fn put(&self, org: &Arc<Vec<u8>>, html: &Arc<Vec<u8>>) {
        let size = |org: &[u8], html: &[u8]| org.len().saturating_add(html.len());
        if size(org, html) > self.bytes {
            return;
        }
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        let hash = hashed(org);
        kept.retain(|(h, bytes, _)| !(*h == hash && bytes == org));
        let mut held = kept.iter().fold(0usize, |all, (_, org, html)| all.saturating_add(size(org, html)));
        while !kept.is_empty() && (kept.len() >= self.most || held.saturating_add(size(org, html)) > self.bytes) {
            let (_, old, written) = kept.remove(0);
            held = held.saturating_sub(size(&old, &written));
        }
        kept.push((hash, org.clone(), html.clone()));
    }
}

/// What [`Cache`] keeps of an Org file: its hash, its bytes and its HTML.
type Written = (u64, Arc<Vec<u8>>, Arc<Vec<u8>>);

/// `bytes`' hash, to find a file by.
fn hashed(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

// The HTML written of an Org document, in the worker.

/// How the HTML starts: [`HEAD`] bytes of clv's own, a doctype and a comment filling them, so that
/// it is HTML to the backend that opens it, and nothing the document writes is among the first
/// bytes a backend is chosen by.
fn start() -> String {
    const DOCTYPE: &str = "<!doctype html><!-- an Org document, written as HTML by clv ";
    const END: &str = "-->";
    let filled = (HEAD as usize).saturating_sub(DOCTYPE.len()).saturating_sub(END.len());
    format!("{DOCTYPE}{}{END}\n<html><head><meta charset=\"utf-8\"></head><body>\n", " ".repeat(filled))
}

/// How deep the HTML is written: a node deeper is written as its text. orgize's own tree, which
/// the worker's stack held, may go deeper.
const MAX_DEPTH: usize = 256;

/// Colours: a TODO keyword not done and done, and what is shown in grey (tags, planning lines).
const NOT_DONE: &str = "#CF222E";
const DONE: &str = "#1A7F37";
const GREY: &str = "#6E7781";

/// The extensions of a picture a link with no description shows, as Org's HTML export inlines it
/// (`org-html-inline-image-rules`).
const PICTURES: [&str; 7] = ["avif", "gif", "jpeg", "jpg", "png", "svg", "webp"];

/// The Org document `text` as HTML, clv's own of orgize's tree; or why not. `home`, the reader's
/// home directory, is where `~/` in a link's path leads.
fn html_of(text: &str, home: Option<&str>) -> Result<String, String> {
    let config = orgize::ParseConfig {
        todo_keywords: todo_keywords(text),
        // `_` and `^` as written: a document's `a_b` is never a subscript.
        use_sub_superscript: orgize::config::UseSubSuperscript::Nil,
        ..Default::default()
    };
    let org = config.clone().parse(text);
    let root = fixed(org.document().syntax(), &config);
    let mut writer = Writer { out: start(), home, notes: Notes::default(), definitions: definitions(&root) };
    writer.title_block(&root);
    writer.element(&root, 0);
    writer.footnotes();
    writer.out.push_str("</body></html>\n");
    match writer.out.len() > MAX_HTML {
        true => Err(format!("the HTML it makes is more than {} MiB", MAX_HTML >> 20)),
        false => Ok(writer.out),
    }
}

/// The TODO keywords `text` declares, not done and done, as Org reads its `#+TODO:`, `#+SEQ_TODO:`
/// and `#+TYP_TODO:` lines, in any case: the words before a `|` not done, those after it done; with
/// no `|`, the last done; `TODO(t)` the word before its key. Org's own, `TODO` and `DONE`, where it
/// declares none.
fn todo_keywords(text: &str) -> (Vec<String>, Vec<String>) {
    let (mut todo, mut done) = (Vec::new(), Vec::new());
    for line in text.lines() {
        let line = line.trim_start();
        let Some(rest) = ["#+todo:", "#+seq_todo:", "#+typ_todo:"]
            .iter()
            .find_map(|key| line.get(..key.len()).filter(|start| start.eq_ignore_ascii_case(key)).and_then(|_| line.get(key.len()..)))
        else {
            continue;
        };
        let words: Vec<String> = rest.split_whitespace().map(|word| word.split('(').next().unwrap_or(word).to_owned()).collect();
        match words.iter().position(|word| word == "|") {
            Some(bar) => {
                todo.extend(words.iter().take(bar).cloned());
                done.extend(words.iter().skip(bar.saturating_add(1)).cloned());
            }
            None => {
                if let Some((last, first)) = words.split_last() {
                    todo.extend(first.iter().cloned());
                    done.push(last.clone());
                }
            }
        }
    }
    if todo.is_empty() && done.is_empty() { (vec!["TODO".into()], vec!["DONE".into()]) } else { (todo, done) }
}

/// The text of `node`'s descendants, as written.
fn raw(node: &SyntaxNode) -> String {
    node.text().to_string()
}

/// `text` with every run of blank space one space, and none at either end.
fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `root` with two of orgize's mistakes mended, each by orgize parsing the text again (ADR-0040):
///
/// - a paragraph that starts with affiliated keywords (`#+NAME:`, `#+CAPTION:`) holds what should
///   have been the element they belong to, a table above all: what follows them is parsed again on
///   its own, and replaces the paragraph, the keywords kept as the first element's;
/// - a paragraph with a bracket link whose path goes on past a line's end (`[[*A heading⏎that
///   wraps]]`) has no link there: each such line's end is made a space, which keeps every offset,
///   and the paragraph parsed again.
///
/// Whatever doesn't come to something else when parsed again is left as it was.
fn fixed(root: &SyntaxNode, config: &orgize::ParseConfig) -> SyntaxNode {
    let paragraphs: Vec<SyntaxNode> = root.descendants().filter(|node| node.kind() == SyntaxKind::PARAGRAPH).collect();
    // From the last, so that what is replaced moves nothing before it.
    let mut green = root.green().into_owned();
    for paragraph in paragraphs.iter().rev() {
        let Some(replacement) = mended(paragraph, config) else { continue };
        let tree = SyntaxNode::new_root(green.clone());
        let Some(found) = tree.descendants().find(|node| node.kind() == SyntaxKind::PARAGRAPH && node.text_range() == paragraph.text_range()) else {
            continue;
        };
        let Some(parent) = found.parent() else { continue };
        let at = found.index();
        let new_parent = parent.green().splice_children(at..at.saturating_add(1), replacement);
        green = parent.replace_with(new_parent);
    }
    SyntaxNode::new_root(green)
}

/// What `paragraph` should have been parsed as ([`fixed`]): the elements to put in its place, or
/// `None` where it is a paragraph.
fn mended(paragraph: &SyntaxNode, config: &orgize::ParseConfig) -> Option<Vec<NodeOrToken<GreenNode, orgize::rowan::GreenToken>>> {
    let keywords: Vec<SyntaxNode> = paragraph.children().take_while(|child| child.kind() == SyntaxKind::AFFILIATED_KEYWORD).collect();
    if let Some(last) = keywords.last() {
        let start = usize::from(last.text_range().end()).saturating_sub(usize::from(paragraph.text_range().start()));
        let text = raw(paragraph);
        let rest = text.get(start..)?;
        // Keywords again would come to this paragraph again.
        if rest.trim_start().starts_with("#+") {
            return None;
        }
        let parsed = config.clone().parse(rest);
        let section = parsed.document().syntax().children().find(|child| child.kind() == SyntaxKind::SECTION)?;
        let elements: Vec<SyntaxNode> = section.children().collect();
        if elements.iter().all(|element| element.kind() == SyntaxKind::PARAGRAPH) {
            return None;
        }
        // The keywords as the first element's, as orgize keeps them for the elements it parses so.
        let mut out = Vec::new();
        for (n, element) in elements.iter().enumerate() {
            let green = element.green().into_owned();
            if n == 0 {
                let kept = keywords.iter().map(|keyword| NodeOrToken::Node(keyword.green().into_owned()));
                let children = kept.chain(green.children().map(|child| child.to_owned()));
                out.push(NodeOrToken::Node(GreenNode::new(green.kind(), children.collect::<Vec<_>>())));
            } else {
                out.push(NodeOrToken::Node(green));
            }
        }
        // What the section holds besides elements (blank lines), after them.
        out.extend(section.children_with_tokens().filter_map(|child| child.into_token()).map(|token| NodeOrToken::Token(token.green().to_owned())));
        return Some(out);
    }
    let text = raw(paragraph);
    let joined = links_joined(&text)?;
    let parsed = config.clone().parse(&joined);
    let section = parsed.document().syntax().children().find(|child| child.kind() == SyntaxKind::SECTION)?;
    let mut children = section.children();
    let (Some(only), None) = (children.next(), children.next()) else { return None };
    (only.kind() == SyntaxKind::PARAGRAPH && only.text_range().len() == paragraph.text_range().len())
        .then(|| vec![NodeOrToken::Node(only.green().into_owned())])
}

/// `text`, a paragraph's, with each line's end inside a bracket link's path (between a `[[` and
/// the next `]`, with no `[` between) made a space, as many bytes as it was; `None` where there is
/// none.
fn links_joined(text: &str) -> Option<String> {
    let mut out = text.as_bytes().to_vec();
    let mut changed = false;
    let mut at = 0;
    while let Some(found) = text.get(at..).and_then(|rest| rest.find("[[")) {
        let start = at.saturating_add(found).saturating_add(2);
        let Some(path) = text.get(start..) else { break };
        let end = path.find([']', '[']).unwrap_or(path.len());
        for (n, byte) in path.bytes().take(end).enumerate() {
            if matches!(byte, b'\n' | b'\r')
                && let Some(slot) = out.get_mut(start.saturating_add(n))
            {
                *slot = b' ';
                changed = true;
            }
        }
        at = start.saturating_add(end).max(at.saturating_add(1));
    }
    // Only line ends, each one byte, were made spaces: still UTF-8.
    changed.then(|| String::from_utf8(out).ok()).flatten()
}

/// Each footnote definition (`[fn:name] …` on a line of its own) in `root`, by its label: the first
/// of a label.
fn definitions(root: &SyntaxNode) -> HashMap<String, SyntaxNode> {
    let mut found = HashMap::new();
    for node in root.descendants().filter(|node| node.kind() == SyntaxKind::FN_DEF) {
        if let Some(label) = footnote_label(&node) {
            found.entry(label).or_insert(node);
        }
    }
    found
}

/// The label of a footnote's reference or definition, `[fn:label…`: `None` for one with none
/// (`[fn:: inline]`).
fn footnote_label(node: &SyntaxNode) -> Option<String> {
    let mut tokens = node.children_with_tokens().filter_map(|child| child.into_token());
    // `[`, `fn`, `:`, then the label.
    let label = tokens.nth(3)?;
    (label.kind() == SyntaxKind::TEXT && !label.text().is_empty()).then(|| label.text().to_owned())
}

/// The footnotes referred to, in the order they are first: each its id and what defines it.
#[derive(Default)]
struct Notes {
    /// Each note's number, from 1, by its label; an inline note without one has none.
    numbered: HashMap<String, usize>,
    /// Each note, in number order: its id, and the node it is defined in (a definition, or the
    /// reference that holds it inline).
    listed: Vec<(String, SyntaxNode)>,
}

/// The HTML being written.
struct Writer<'a> {
    out: String,
    home: Option<&'a str>,
    notes: Notes,
    definitions: HashMap<String, SyntaxNode>,
}

/// `text` with `&`, `<`, `>` and `"` written as HTML's entities.
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

impl Writer<'_> {
    fn push(&mut self, html: &str) {
        self.out.push_str(html);
    }

    /// Whether the HTML is past what is written: everything after is left out, and the document
    /// refused for it.
    fn full(&self) -> bool {
        self.out.len() > MAX_HTML
    }

    /// The title block, at the top: `#+TITLE`, `#+SUBTITLE`, `#+AUTHOR` and `#+DATE` wherever they
    /// are, each kind's lines joined.
    fn title_block(&mut self, root: &SyntaxNode) {
        let mut values: HashMap<String, Vec<String>> = HashMap::new();
        for keyword in root.descendants().filter_map(orgize::ast::Keyword::cast) {
            let key = keyword.key().to_ascii_uppercase();
            if matches!(key.as_str(), "TITLE" | "SUBTITLE" | "AUTHOR" | "DATE") {
                values.entry(key).or_default().push(collapsed(&keyword.value()));
            }
        }
        let joined = |key: &str| values.get(key).map(|lines| collapsed(&lines.join(" "))).filter(|value| !value.is_empty());
        if let Some(title) = joined("TITLE") {
            let _ = writeln!(self.out, "<h1 style=\"text-align:center\">{}</h1>", escaped(&title));
        }
        if let Some(subtitle) = joined("SUBTITLE") {
            let _ = writeln!(self.out, "<p style=\"text-align:center;font-size:14pt\">{}</p>", escaped(&subtitle));
        }
        let byline: Vec<String> = ["AUTHOR", "DATE"].iter().filter_map(|key| joined(key)).map(|value| escaped(&value)).collect();
        if !byline.is_empty() {
            let _ = writeln!(self.out, "<p style=\"text-align:center;color:{GREY}\">{}</p>", byline.join("<br>"));
        }
    }

    /// The element `node`, and what it holds, at `depth`.
    fn element(&mut self, node: &SyntaxNode, depth: usize) {
        use SyntaxKind as K;
        if self.full() {
            return;
        }
        if depth > MAX_DEPTH {
            let _ = write!(self.out, "<p>{}</p>", escaped(&raw(node)));
            return;
        }
        let deeper = depth.saturating_add(1);
        match node.kind() {
            K::DOCUMENT | K::SECTION | K::DRAWER_CONTENT => self.elements(node, deeper),
            K::HEADLINE => self.headline(node, deeper),
            K::PARAGRAPH => {
                self.push("<p>");
                self.objects(node.children_with_tokens(), deeper);
                self.push("</p>\n");
            }
            K::LIST => self.list(node, deeper),
            K::ORG_TABLE => self.table(node, deeper),
            K::TABLE_EL => self.pre(&raw(node), None),
            K::SOURCE_BLOCK => {
                let language = node
                    .descendants_with_tokens()
                    .filter_map(|e| e.into_token())
                    .find(|t| t.kind() == K::SRC_BLOCK_LANGUAGE)
                    .map(|t| t.text().to_owned());
                self.pre(&unescaped(&block_content(node)), language.as_deref());
            }
            K::EXAMPLE_BLOCK => self.pre(&unescaped(&block_content(node)), None),
            K::EXPORT_BLOCK => {
                let html = node
                    .descendants_with_tokens()
                    .filter_map(|e| e.into_token())
                    .any(|t| t.kind() == K::EXPORT_BLOCK_TYPE && t.text().eq_ignore_ascii_case("html"));
                if html {
                    // As Markdown's HTML is: read by the importer as it is.
                    self.push(&block_content(node));
                }
            }
            K::QUOTE_BLOCK => self.block(node, "<blockquote>", "</blockquote>\n", deeper),
            K::CENTER_BLOCK => self.block(node, "<center>", "</center>\n", deeper),
            K::SPECIAL_BLOCK => self.block(node, "<div>", "</div>\n", deeper),
            K::VERSE_BLOCK => {
                self.push("<p>");
                if let Some(content) = node.children().find(|child| child.kind() == K::BLOCK_CONTENT) {
                    self.verse(&content, deeper);
                }
                self.push("</p>\n");
            }
            K::DYN_BLOCK => {
                for child in node.children().filter(|child| !matches!(child.kind(), K::DYN_BLOCK_BEGIN | K::DYN_BLOCK_END)) {
                    self.element(&child, deeper);
                }
            }
            K::FIXED_WIDTH => {
                let lines: Vec<String> = raw(node)
                    .lines()
                    .filter(|line| !line.trim_start().starts_with("#+"))
                    .map(|line| {
                        let line = line.trim_start().strip_prefix(':').unwrap_or(line);
                        line.strip_prefix(' ').unwrap_or(line).to_owned()
                    })
                    .collect();
                self.pre(&lines.join("\n"), None);
            }
            K::LATEX_ENVIRONMENT => self.pre(raw(node).trim_end(), None),
            K::RULE => self.push("<hr>\n"),
            K::CLOCK => {
                let _ = writeln!(self.out, "<p style=\"font-size:9pt;color:{GREY}\">{}</p>", escaped(&collapsed(&raw(node))));
            }
            // Hidden: drawers, keywords, comments; a footnote's definition is written with the notes.
            K::DRAWER | K::PROPERTY_DRAWER | K::KEYWORD | K::BABEL_CALL | K::COMMENT | K::COMMENT_BLOCK | K::FN_DEF | K::AFFILIATED_KEYWORD => {}
            _ => {}
        }
    }

    /// The elements `node` holds.
    fn elements(&mut self, node: &SyntaxNode, depth: usize) {
        for child in node.children() {
            self.element(&child, depth);
        }
    }

    /// A greater block, `node`: what it holds, between `open` and `close`.
    fn block(&mut self, node: &SyntaxNode, open: &str, close: &str, depth: usize) {
        self.push(open);
        if let Some(content) = node.children().find(|child| child.kind() == SyntaxKind::BLOCK_CONTENT) {
            self.elements(&content, depth);
        }
        self.push(close);
    }

    /// A verse block's content, `node`: its objects, each line's end a line's end.
    fn verse(&mut self, node: &SyntaxNode, depth: usize) {
        for child in node.children_with_tokens() {
            match child {
                NodeOrToken::Node(node) if node.kind() == SyntaxKind::PARAGRAPH => self.verse(&node, depth),
                NodeOrToken::Node(node) => self.object(&node, depth),
                NodeOrToken::Token(token) => {
                    let lines: Vec<String> = token.text().split('\n').map(escaped).collect();
                    self.push(&lines.join("<br>\n"));
                }
            }
        }
    }

    /// Code, `text`, in a `<pre>`: in `language`'s class, which the documents' backend draws as a
    /// picture where a backend draws it.
    fn pre(&mut self, text: &str, language: Option<&str>) {
        let text = text.strip_suffix('\n').unwrap_or(text);
        let class = language.filter(|language| !language.is_empty()).map(|language| format!(" class=\"language-{}\"", escaped(language)));
        let _ = writeln!(self.out, "<pre><code{}>{}</code></pre>", class.unwrap_or_default(), escaped(text));
    }

    /// A headline, `node`: its heading (its TODO keyword in colour, its priority and its title, its
    /// tags in grey), its planning line, small and grey, then what is under it.
    fn headline(&mut self, node: &SyntaxNode, depth: usize) {
        use SyntaxKind as K;
        let stars = node.children_with_tokens().filter_map(|e| e.into_token()).find(|t| t.kind() == K::HEADLINE_STARS);
        let level = stars.map_or(1, |stars| stars.text().len()).clamp(1, 6);
        let _ = write!(self.out, "<h{level}>");
        for child in node.children_with_tokens() {
            match child {
                NodeOrToken::Token(token) if token.kind() == K::HEADLINE_KEYWORD_TODO => {
                    let _ = write!(self.out, "<span style=\"color:{NOT_DONE}\">{}</span> ", escaped(token.text()));
                }
                NodeOrToken::Token(token) if token.kind() == K::HEADLINE_KEYWORD_DONE => {
                    let _ = write!(self.out, "<span style=\"color:{DONE}\">{}</span> ", escaped(token.text()));
                }
                NodeOrToken::Node(child) if child.kind() == K::HEADLINE_PRIORITY => {
                    let _ = write!(self.out, "{} ", escaped(&raw(&child)));
                }
                NodeOrToken::Node(child) if child.kind() == K::HEADLINE_TITLE => {
                    self.objects(child.children_with_tokens(), depth);
                    // The space before its tags is theirs.
                    let end = self.out.trim_end_matches(' ').len();
                    self.out.truncate(end);
                }
                NodeOrToken::Node(child) if child.kind() == K::HEADLINE_TAGS => {
                    let _ = write!(self.out, " <span style=\"color:{GREY};font-weight:normal\">{}</span>", escaped(&raw(&child)));
                }
                _ => {}
            }
        }
        let _ = writeln!(self.out, "</h{level}>");
        for child in node.children() {
            match child.kind() {
                K::PLANNING => {
                    let _ = writeln!(self.out, "<p style=\"font-size:9pt;color:{GREY}\">{}</p>", escaped(&collapsed(&raw(&child))));
                }
                K::SECTION | K::HEADLINE => self.element(&child, depth),
                _ => {}
            }
        }
    }

    /// A list, `node`: ordered, unordered, or of descriptions; a checkbox as ☐, ☑ or ☒.
    fn list(&mut self, node: &SyntaxNode, depth: usize) {
        use SyntaxKind as K;
        let Some(list) = orgize::ast::List::cast(node.clone()) else { return };
        let (open, close) = match (list.is_ordered(), list.is_descriptive()) {
            (true, _) => ("<ol>", "</ol>\n"),
            (false, true) => ("<dl>", "</dl>\n"),
            (false, false) => ("<ul>", "</ul>\n"),
        };
        self.push(open);
        for item in node.children().filter(|child| child.kind() == K::LIST_ITEM) {
            let tag = item.children().find(|child| child.kind() == K::LIST_ITEM_TAG);
            let described = list.is_descriptive();
            match (&tag, described) {
                (Some(tag), true) => {
                    self.push("<dt><b>");
                    let shown = tag.children_with_tokens().filter(|e| e.kind() != K::COLON2);
                    self.objects(shown, depth);
                    self.push("</b></dt><dd>");
                }
                (_, true) => self.push("<dd>"),
                // A described item in a list that isn't one of descriptions: its term first.
                (Some(tag), false) => {
                    self.push("<li><b>");
                    let shown = tag.children_with_tokens().filter(|e| e.kind() != K::COLON2);
                    self.objects(shown, depth);
                    self.push("</b> — ");
                }
                (None, false) => self.push("<li>"),
            }
            if let Some(checkbox) = item.children().find(|child| child.kind() == K::LIST_ITEM_CHECK_BOX) {
                let mark = match raw(&checkbox).trim_matches(['[', ']']) {
                    "X" | "x" => "☑",
                    "-" => "☒",
                    _ => "☐",
                };
                let _ = write!(self.out, "{mark} ");
            }
            if let Some(content) = item.children().find(|child| child.kind() == K::LIST_ITEM_CONTENT) {
                // Its first paragraph on the item's line, as a tight list's is.
                let mut children = content.children().peekable();
                if let Some(first) = children.next_if(|child| child.kind() == K::PARAGRAPH) {
                    self.objects(first.children_with_tokens(), depth);
                }
                for child in children {
                    self.element(&child, depth);
                }
            }
            self.push(if described { "</dd>" } else { "</li>" });
        }
        self.push(close);
    }

    /// A table, `node`, in a grid: the rows before its first rule its header where rows follow the
    /// rule; rows of alignment and width cookies, and its formulas, left out.
    fn table(&mut self, node: &SyntaxNode, depth: usize) {
        use SyntaxKind as K;
        let rows: Vec<SyntaxNode> = node
            .children()
            .filter(|child| matches!(child.kind(), K::ORG_TABLE_STANDARD_ROW | K::ORG_TABLE_RULE_ROW))
            .filter(|row| !is_cookies(row))
            .collect();
        let first_rule = rows.iter().position(|row| row.kind() == K::ORG_TABLE_RULE_ROW);
        let header = first_rule.filter(|&rule| rule > 0 && rows.iter().skip(rule).any(|row| row.kind() == K::ORG_TABLE_STANDARD_ROW)).unwrap_or(0);
        self.push("<table border=\"1\">");
        if header > 0 {
            self.push("<thead>");
            for row in rows.iter().take(header) {
                self.row(row, "th", depth);
            }
            self.push("</thead>");
        }
        self.push("<tbody>");
        for row in rows.iter().skip(header).filter(|row| row.kind() == K::ORG_TABLE_STANDARD_ROW) {
            self.row(row, "td", depth);
        }
        self.push("</tbody></table>\n");
    }

    /// A table's row, `row`, its cells as `cell`s.
    fn row(&mut self, row: &SyntaxNode, cell: &str, depth: usize) {
        self.push("<tr>");
        for child in row.children().filter(|child| child.kind() == SyntaxKind::ORG_TABLE_CELL) {
            let _ = write!(self.out, "<{cell}>");
            self.objects(child.children_with_tokens(), depth);
            let _ = write!(self.out, "</{cell}>");
        }
        self.push("</tr>");
    }

    /// Objects: text, emphasis, links and the rest, as a paragraph or a title holds them.
    fn objects(&mut self, elements: impl Iterator<Item = SyntaxElement>, depth: usize) {
        for element in elements {
            match element {
                NodeOrToken::Node(node) => self.object(&node, depth),
                NodeOrToken::Token(token) => self.text(token.text()),
            }
        }
    }

    /// The object `node`.
    fn object(&mut self, node: &SyntaxNode, depth: usize) {
        use SyntaxKind as K;
        if self.full() {
            return;
        }
        if depth > MAX_DEPTH {
            self.push(&escaped(&raw(node)));
            return;
        }
        let deeper = depth.saturating_add(1);
        let inner = || {
            let children: Vec<SyntaxElement> = node.children_with_tokens().collect();
            let end = children.len().saturating_sub(1);
            children.into_iter().take(end).skip(1).collect::<Vec<_>>()
        };
        match node.kind() {
            K::BOLD => self.wrapped("b", inner(), deeper),
            K::ITALIC => self.wrapped("i", inner(), deeper),
            K::UNDERLINE => self.wrapped("u", inner(), deeper),
            K::STRIKE => self.wrapped("s", inner(), deeper),
            K::VERBATIM | K::CODE => {
                let text: String = inner().iter().map(|element| element.to_string()).collect();
                let _ = write!(self.out, "<code>{}</code>", escaped(&text));
            }
            K::LINE_BREAK => self.push("<br>\n"),
            K::LINK => self.link(node, deeper),
            K::FN_REF => self.reference(node),
            // A place, not shown: its `id` is written in the links' task.
            K::TARGET => {}
            K::RADIO_TARGET => {
                let text: String = inner().iter().map(|element| element.to_string()).collect();
                self.push(&escaped(&text));
            }
            K::ENTITY => match orgize::ast::Entity::cast(node.clone()) {
                Some(entity) if !entity.utf8().is_empty() => self.push(&escaped(entity.utf8())),
                _ => self.push(&escaped(&raw(node))),
            },
            K::SNIPPET => {
                if let Some(snippet) = orgize::ast::Snippet::cast(node.clone())
                    && snippet.backend().eq_ignore_ascii_case("html")
                {
                    self.push(&snippet.value());
                }
            }
            K::INLINE_SRC => {
                let text = raw(node);
                let body = text.split_once('{').and_then(|(_, body)| body.rsplit_once('}')).map_or(text.as_str(), |(body, _)| body);
                let _ = write!(self.out, "<code>{}</code>", escaped(body));
            }
            K::AFFILIATED_KEYWORD => {}
            // As written: timestamps, macros, LaTeX, cookies, calls, and anything else.
            _ => self.push(&escaped(&raw(node))),
        }
    }

    /// `children` in a `tag`.
    fn wrapped(&mut self, tag: &str, children: Vec<SyntaxElement>, depth: usize) {
        let _ = write!(self.out, "<{tag}>");
        self.objects(children.into_iter(), depth);
        let _ = write!(self.out, "</{tag}>");
    }

    /// Text, `text`, as it reads.
    fn text(&mut self, text: &str) {
        self.push(&escaped(text));
    }

    /// A link, `node`: a picture where it is one ([`picture`]), else its description, or its path
    /// where it has none, leading where its path does.
    fn link(&mut self, node: &SyntaxNode, depth: usize) {
        use SyntaxKind as K;
        let Some(link) = orgize::ast::Link::cast(node.clone()) else { return };
        let path = collapsed(&link.path());
        let described = link.has_description();
        if !described && let Some(src) = picture(&path) {
            let _ = write!(self.out, "<img src=\"{}\" alt=\"\">", escaped(&src));
            return;
        }
        let href = escaped(&self.address(&path));
        let _ = write!(self.out, "<a href=\"{href}\">");
        if described {
            match picture(&collapsed(&link.description_raw())) {
                Some(src) => {
                    let _ = write!(self.out, "<img src=\"{}\" alt=\"\">", escaped(&src));
                }
                None => {
                    let description: Vec<SyntaxElement> =
                        node.children_with_tokens().skip_while(|e| e.kind() != K::L_BRACKET).skip(1).filter(|e| e.kind() != K::R_BRACKET2).collect();
                    self.objects(description.into_iter(), depth);
                }
            }
        } else {
            self.text(path.strip_prefix('*').unwrap_or(&path));
        }
        self.push("</a>");
    }

    /// Where a link whose path is `path` leads, as an `href`.
    fn address(&self, path: &str) -> String {
        let _ = self.home;
        path.to_owned()
    }

    /// A footnote's reference, `node`: its number, as a superscript leading to the note; or as
    /// written, for a note defined nowhere.
    fn reference(&mut self, node: &SyntaxNode) {
        let label = footnote_label(node);
        // An inline definition: what comes after a second `:`.
        let colons = node.children_with_tokens().filter(|e| e.kind() == SyntaxKind::COLON).count();
        let inline = colons >= 2;
        let known = label.as_ref().and_then(|label| self.notes.numbered.get(label)).copied();
        let number = match known {
            Some(number) => number,
            None => {
                let defined = match (&label, inline) {
                    (_, true) => Some(node.clone()),
                    (Some(label), false) => self.definitions.get(label).cloned(),
                    (None, false) => None,
                };
                let Some(defined) = defined else {
                    self.push(&escaped(&raw(node)));
                    return;
                };
                self.notes.listed.push((format!("fn.{}", self.notes.listed.len().saturating_add(1)), defined));
                let number = self.notes.listed.len();
                if let Some(label) = label {
                    self.notes.numbered.insert(label, number);
                }
                number
            }
        };
        let id = self.notes.listed.get(number.saturating_sub(1)).map(|(id, _)| id.clone()).unwrap_or_default();
        let _ = write!(self.out, "<sup><a href=\"#{id}\">{number}</a></sup>");
    }

    /// The notes, after the document, in smaller text, each its number first: those referred to, in
    /// the order they are first, then those defined and referred to nowhere.
    fn footnotes(&mut self) {
        let mut unreferred: Vec<(String, SyntaxNode)> =
            self.definitions.iter().filter(|(label, _)| !self.notes.numbered.contains_key(*label)).map(|(l, n)| (l.clone(), n.clone())).collect();
        unreferred.sort_by_key(|(_, node)| node.text_range().start());
        if self.notes.listed.is_empty() && unreferred.is_empty() {
            return;
        }
        self.push("<hr>\n");
        let mut at = 0;
        loop {
            // A note may refer to another, which comes after it: listed as it is met.
            if at >= self.notes.listed.len() {
                let Some((label, node)) = unreferred.first().cloned() else { break };
                unreferred.remove(0);
                if self.notes.numbered.contains_key(&label) {
                    continue;
                }
                self.notes.listed.push((format!("fn.{}", self.notes.listed.len().saturating_add(1)), node));
                self.notes.numbered.insert(label, self.notes.listed.len());
            }
            let Some((id, node)) = self.notes.listed.get(at).cloned() else { break };
            at = at.saturating_add(1);
            let _ = write!(self.out, "<p id=\"{id}\" style=\"font-size:9.5pt\"><sup>{at}</sup> ");
            self.note(&node);
            self.push("</p>\n");
            if self.full() {
                break;
            }
        }
    }

    /// What `node` defines a note as: a definition's content, or an inline note's.
    fn note(&mut self, node: &SyntaxNode) {
        use SyntaxKind as K;
        let content: Vec<SyntaxElement> = match node.kind() {
            // After `[fn:label]`.
            K::FN_DEF => node.children_with_tokens().skip_while(|e| e.kind() != K::R_BRACKET).skip(1).collect(),
            // After `[fn:label:`, before the last `]`.
            _ => {
                let all: Vec<SyntaxElement> = node.children_with_tokens().collect();
                let end = all.len().saturating_sub(1);
                let mut colons = 0;
                all.into_iter()
                    .take(end)
                    .skip_while(|e| {
                        if e.kind() == K::COLON {
                            colons += 1;
                            return colons < 2;
                        }
                        true
                    })
                    .skip(1)
                    .collect()
            }
        };
        for element in content {
            match element {
                NodeOrToken::Node(child) if child.kind() == K::PARAGRAPH => {
                    self.objects(child.children_with_tokens(), 1);
                    self.push(" ");
                }
                NodeOrToken::Node(child) if !child.kind().is_object() => {
                    self.push("</p>\n");
                    self.element(&child, 1);
                    self.push("<p style=\"font-size:9.5pt\">");
                }
                NodeOrToken::Node(child) => self.object(&child, 1),
                NodeOrToken::Token(token) => self.text(token.text()),
            }
        }
    }
}

/// Whether `row`, a table's, holds nothing but alignment and width cookies (`<l>`, `<10>`, `<r5>`)
/// and empty cells, at least one cookie, or is a row of column groups (`/` first): Org's, never
/// shown.
fn is_cookies(row: &SyntaxNode) -> bool {
    let cells: Vec<String> =
        row.children().filter(|child| child.kind() == SyntaxKind::ORG_TABLE_CELL).map(|cell| raw(&cell).trim().to_owned()).collect();
    let cookie = |cell: &str| {
        cell.strip_prefix('<').and_then(|rest| rest.strip_suffix('>')).is_some_and(|inside| {
            let digits = inside.strip_prefix(['l', 'r', 'c']).unwrap_or(inside);
            !inside.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
        })
    };
    let groups = cells.first().is_some_and(|first| first == "/");
    groups || (cells.iter().any(|cell| cookie(cell)) && cells.iter().all(|cell| cell.is_empty() || cookie(cell)))
}

/// What a block, `node`, holds, as written.
fn block_content(node: &SyntaxNode) -> String {
    node.children().find(|child| child.kind() == SyntaxKind::BLOCK_CONTENT).map(|content| raw(&content)).unwrap_or_default()
}

/// `text`, a block's code, with the comma Org puts before a line that would otherwise start a
/// headline or a keyword (`,*`, `,#+`) taken off, as Org reads it.
fn unescaped(text: &str) -> String {
    text.split_inclusive('\n')
        .map(|line| {
            let indent = line.len().saturating_sub(line.trim_start_matches([' ', '\t']).len());
            let (lead, rest) = line.split_at_checked(indent).unwrap_or(("", line));
            match rest.strip_prefix(',') {
                Some(after) if after.trim_start_matches(',').starts_with('*') || after.trim_start_matches(',').starts_with("#+") => {
                    format!("{lead}{after}")
                }
                _ => line.to_owned(),
            }
        })
        .collect()
}

/// The picture a link with the path `path` shows, its path as written for the documents' backend
/// to read beside the document: a file's, `file:x` or `./x`, `../x`, `/x`, of an extension Org
/// inlines ([`PICTURES`]), with no search after `::`. Pictures on the web are links: nothing is
/// fetched.
fn picture(path: &str) -> Option<String> {
    let file = path
        .strip_prefix("file:")
        .or_else(|| path.strip_prefix("file+sys:"))
        .or_else(|| path.strip_prefix("file+emacs:"))
        .or_else(|| ["./", "../", "/"].iter().any(|start| path.starts_with(start)).then_some(path))?;
    if file.contains("::") || file.is_empty() {
        return None;
    }
    let extension = std::path::Path::new(file).extension().and_then(|e| e.to_str())?;
    PICTURES.iter().any(|picture| extension.eq_ignore_ascii_case(picture)).then(|| file.to_owned())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// The HTML of `org`, with no home.
    fn html(org: &str) -> String {
        html_of(org, None).unwrap()
    }

    /// The HTML of `org`'s body: past clv's own start, before its end.
    fn body(org: &str) -> String {
        let html = html(org);
        let start = html.find("<body>\n").unwrap() + "<body>\n".len();
        html[start..html.len() - "</body></html>\n".len()].to_owned()
    }

    #[test]
    fn org_is_told_by_its_name_and_its_mode_line_and_must_be_text() {
        for path in ["notes.org", "NOTES.ORG", "old.org_archive", "Old.Org_Archive"] {
            assert!(opens(path, b"* A heading\n"), "{path}");
        }
        assert!(!opens("notes.org", b"\x89PNG\r\n\x1a\n\0\0\0"), "a picture misnamed");
        assert!(!opens("notes.md", b"* A heading\n"), "Markdown named so");
        for head in [
            &b"-*- org -*-\n* A heading"[..],
            b"# -*- mode: org -*-\n",
            b"# -*- Mode: Org; coding: utf-8 -*-\n",
            b"# -*- coding: utf-8; mode: org; fill-column: 80 -*-\n",
            b"#!/usr/bin/env something\n# -*- mode: org -*-\n",
            b"\xEF\xBB\xBF-*- ORG -*-",
            b"\xFF\xFE-\0*\0-\0 \0o\0r\0g\0 \0-\0*\0-\0",
            b"-*- org -*-",
        ] {
            assert!(opens("notes.txt", head), "{:?}", String::from_utf8_lossy(head));
            assert!(opens("/dev/stdin", head), "through a pipe");
            assert!(opens("notes.md", head), "a .md that says Org");
        }
        for head in [
            &b"* A heading\n-*- org -*-\n"[..],
            b"-*- markdown -*-\n",
            b"-*- mode: markdown; org: yes -*-\n",
            b"-*- mode: organizer -*-\n",
            b"-*- org\n-*-\n",
            b"<!doctype html>\n<!-- -*- org -*- -->",
            b"#!/bin/sh\n\n# -*- mode: org -*-\n",
            b"-*- org -*-\0",
        ] {
            assert!(!opens("notes.txt", head), "{:?}", String::from_utf8_lossy(head));
        }
    }

    #[test]
    fn org_is_read_as_utf8_and_utf16_by_its_byte_order_mark() {
        assert_eq!(decoded(b"\xEF\xBB\xBF* caf\xC3\xA9"), "* café");
        assert_eq!(decoded(b"\xFF\xFE*\0 \0\xE9\0"), "* é");
        assert_eq!(decoded(b"\xFE\xFF\0*\0 \0\xE9"), "* é");
        assert_eq!(decoded(b"* \xFF"), "* \u{FFFD}");
    }

    /// The documents' limits, before the text reaches the worker: its size, and its lines and
    /// marks, counted as Markdown's are.
    #[test]
    fn an_org_file_past_the_documents_limits_is_refused_before_it_is_parsed() {
        assert!(within("*a ".repeat(MAX_MARKS / 2).as_bytes()).is_ok());
        let refused = within("*a".repeat(MAX_MARKS + 1).as_bytes()).unwrap_err();
        assert_eq!(refused, format!("the document has more than {MAX_MARKS} lines and marks"));
        assert!(within(&vec![b'a'; MAX_SOURCE + 1]).unwrap_err().contains("more than 64 MiB"));
    }

    #[test]
    fn a_headline_is_a_heading_with_its_keyword_in_colour_its_tags_in_grey_and_its_planning_small() {
        let html = body("* TODO [#A] First [1/3] :work:urgent:\nSCHEDULED: <2024-01-02 Tue>\n** DONE Second\n******* Seventh\n");
        assert!(html.contains(&format!("<h1><span style=\"color:{NOT_DONE}\">TODO</span> [#A] First [1/3] <span style=\"color:{GREY};font-weight:normal\">:work:urgent:</span></h1>")), "{html}");
        assert!(html.contains(&format!("<p style=\"font-size:9pt;color:{GREY}\">SCHEDULED: &lt;2024-01-02 Tue&gt;</p>")), "{html}");
        assert!(html.contains(&format!("<h2><span style=\"color:{DONE}\">DONE</span> Second</h2>")), "{html}");
        assert!(html.contains("<h6>Seventh</h6>"), "a level past 6 as 6: {html}");
    }

    #[test]
    fn the_todo_keywords_a_file_declares_are_honoured() {
        let html = body("#+TODO: WAIT(w) NEXT | GONE(g@)\n#+seq_todo: LATER\n* WAIT one\n* GONE two\n* LATER three\n* TODO four\n");
        assert!(html.contains(&format!("<span style=\"color:{NOT_DONE}\">WAIT</span> one")), "{html}");
        assert!(html.contains(&format!("<span style=\"color:{DONE}\">GONE</span> two")), "{html}");
        assert!(html.contains(&format!("<span style=\"color:{DONE}\">LATER</span> three")), "the last done, without a bar: {html}");
        assert!(html.contains("<h1>TODO four</h1>"), "Org's own no longer, once a file declares its own: {html}");
    }

    #[test]
    fn the_title_block_comes_first() {
        let html = body("Before.\n#+TITLE: My notes\n#+SUBTITLE: and more\n#+AUTHOR: Someone\n#+DATE: 2026-11-17\n#+title: & again\n");
        assert!(
            html.starts_with(
                "<h1 style=\"text-align:center\">My notes &amp; again</h1>\n<p style=\"text-align:center;font-size:14pt\">and more</p>\n"
            ),
            "{html}"
        );
        assert!(html.contains(&format!("<p style=\"text-align:center;color:{GREY}\">Someone<br>2026-11-17</p>")), "{html}");
        assert!(!html.contains("#+"), "{html}");
    }

    #[test]
    fn inline_markup_and_what_is_shown_as_written() {
        let html = body(
            "*bold* /italic/ _under_ +strike+ =verb <b>= ~code~ a\\\\\nnext a_b x^2 <2024-01-01 Mon> {{{macro}}} $x$ \\alpha src_sh{ls -l} [1/3]\n-----\n",
        );
        for shown in [
            "<b>bold</b>",
            "<i>italic</i>",
            "<u>under</u>",
            "<s>strike</s>",
            "<code>verb &lt;b&gt;</code>",
            "<code>code</code>",
            "a<br>\nnext",
            "a_b x^2",
            "&lt;2024-01-01 Mon&gt;",
            "{{{macro}}}",
            "$x$",
            "α",
            "<code>ls -l</code>",
            "[1/3]",
            "<hr>",
        ] {
            assert!(html.contains(shown), "{shown} in {html}");
        }
        assert!(!html.contains("<sub>") && !html.contains("<sup>"), "{html}");
    }

    #[test]
    fn lists_with_their_checkboxes_and_descriptions() {
        let html = body("- [ ] one\n- [X] two\n- [-] three\n  1. nested\n");
        assert_eq!(html, "<ul><li>☐ one\n</li><li>☑ two\n</li><li>☒ three\n<ol><li>nested\n</li></ol>\n</li></ul>\n");
        let html = body("- term :: described\n- other :: too\n");
        assert_eq!(html, "<dl><dt><b>term </b></dt><dd> described\n</dd><dt><b>other </b></dt><dd> too\n</dd></dl>\n");
        let html = body("1. first\n\n2. second\n");
        assert_eq!(html, "<ol><li>first\n</li><li>second\n</li></ol>\n");
        let mixed = body("- plain\n- term :: described\n");
        assert!(mixed.contains("<li><b>term </b> —  described"), "{mixed}");
    }

    #[test]
    fn a_table_in_a_grid_its_header_its_cookies_and_formulas_left_out() {
        let html = body("| a | b |\n|---+---|\n| <l> | <10> |\n| 1 | *2* |\n|---+---|\n| 3 | 4 |\n#+TBLFM: $2=$1\n");
        assert_eq!(
            html,
            "<table border=\"1\"><thead><tr><th>a</th><th>b</th></tr></thead><tbody><tr><td>1</td><td><b>2</b></td></tr><tr><td>3</td><td>4</td></tr></tbody></table>\n"
        );
        let headless = body("| a | b |\n| 1 | 2 |\n");
        assert!(headless.starts_with("<table border=\"1\"><tbody><tr><td>a</td>"), "no rule, no header: {headless}");
        let ruled_last = body("| a |\n|---|\n");
        assert!(!ruled_last.contains("<thead>"), "a rule with no row after it: {ruled_last}");
        let table_el = body("+---+---+\n| a | b |\n+---+---+\n");
        assert!(table_el.starts_with("<pre><code>+---+---+\n| a | b |\n+---+---+</code></pre>"), "{table_el}");
    }

    #[test]
    fn blocks_as_code_quotes_verse_centred_and_results() {
        let html = body(concat!(
            "#+begin_src mermaid :exports none\ngraph TD; A-->B\n,* not a heading\n#+end_src\n",
            "#+begin_src\nplain < code\n#+end_src\n",
            "#+begin_example\nexample\n#+end_example\n",
            ": fixed\n:   width\n",
            "#+begin_quote\nquoted\n#+end_quote\n",
            "#+begin_verse\nline one\n  line two\n#+end_verse\n",
            "#+begin_center\ncentred\n#+end_center\n",
            "#+begin_note\nspecial\n#+end_note\n",
            "#+RESULTS:\n: 42\n",
            "#+begin_export html\n<b>raw</b>\n#+end_export\n",
        ));
        for shown in [
            "<pre><code class=\"language-mermaid\">graph TD; A--&gt;B\n* not a heading</code></pre>",
            "<pre><code>plain &lt; code</code></pre>",
            "<pre><code>example</code></pre>",
            "<pre><code>fixed\n  width</code></pre>",
            "<blockquote><p>quoted\n</p>\n</blockquote>",
            "line one<br>\n  line two",
            "<center><p>centred\n</p>\n</center>",
            "<div><p>special\n</p>\n</div>",
            "<pre><code>42</code></pre>",
            "<b>raw</b>",
        ] {
            assert!(html.contains(shown), "{shown} in {html}");
        }
        assert!(!html.contains("RESULTS") && !html.contains("exports"), "{html}");
    }

    #[test]
    fn footnotes_are_numbered_as_referred_to_and_written_at_the_end() {
        let html = body(
            "A[fn:b] B[fn:: inline] C[fn:a] D[fn:n: named inline] E[fn:b] F[fn:none]\n\n[fn:a] Note a.\n[fn:b] Note b.\n[fn:c] Never referred to.\n",
        );
        assert!(
            html.contains("A<sup><a href=\"#fn.1\">1</a></sup> B<sup><a href=\"#fn.2\">2</a></sup> C<sup><a href=\"#fn.3\">3</a></sup>"),
            "{html}"
        );
        assert!(html.contains("D<sup><a href=\"#fn.4\">4</a></sup> E<sup><a href=\"#fn.1\">1</a></sup> F[fn:none]"), "{html}");
        let notes = &html[html.find("<hr>").unwrap()..];
        for (n, text) in [(1, " Note b."), (2, " inline"), (3, " Note a."), (4, " named inline"), (5, " Never referred to.")] {
            assert!(notes.contains(&format!("<p id=\"fn.{n}\" style=\"font-size:9.5pt\"><sup>{n}</sup> {text}")), "{n} in {notes}");
        }
        assert!(!html[..html.find("<hr>").unwrap()].contains("Note a."), "a definition isn't shown where it is: {html}");
    }

    #[test]
    fn a_link_to_a_picture_is_the_picture() {
        let html = body(
            "[[file:a.png]] [[./b.JPG]] [[../c.svg]] [[file:d.png][a description]] [[https://e.org/f.png]] [[file:g.png::3]] [[https://h.org][file:thumb.webp]]\n",
        );
        for shown in [
            "<img src=\"a.png\" alt=\"\">",
            "<img src=\"./b.JPG\" alt=\"\">",
            "<img src=\"../c.svg\" alt=\"\">",
            "<a href=\"file:d.png\">a description</a>",
            "<a href=\"https://e.org/f.png\">https://e.org/f.png</a>",
            "<a href=\"https://h.org\"><img src=\"thumb.webp\" alt=\"\"></a>",
        ] {
            assert!(html.contains(shown), "{shown} in {html}");
        }
        assert!(!html.contains("<img src=\"g.png"), "{html}");
    }

    #[test]
    fn what_org_hides_is_left_out() {
        let html = body(concat!(
            "#+STARTUP: overview\n* Heading\n:PROPERTIES:\n:ID: x\n:END:\n:LOGBOOK:\nlogged\n:END:\n",
            "# a comment\n#+begin_comment\nhidden\n#+end_comment\n#+INCLUDE: \"other.org\"\n",
            "#+begin_export latex\n\\LaTeX\n#+end_export\n#+CALL: run()\n#+NAME: named\n#+CAPTION: captioned\n| a |\n",
            "Shown @@html:<i>snippet</i>@@ @@latex:\\no@@.\n",
        ));
        for hidden in
            ["overview", "PROPERTIES", "logged", "a comment", "hidden", "INCLUDE", "other.org", "LaTeX", "run()", "named", "captioned", "\\no"]
        {
            assert!(!html.contains(hidden), "{hidden} in {html}");
        }
        assert!(html.contains("<p>Shown <i>snippet</i> .\n</p>"), "{html}");
    }

    /// What Org's export leaves out, and a viewer of one's own notes shows.
    #[test]
    fn a_comment_subtree_and_a_noexport_one_are_shown() {
        let html = body("* COMMENT Drafts\nA draft.\n* Private :noexport:\nKept.\n");
        assert!(html.contains("A draft.") && html.contains("Kept."), "{html}");
    }

    /// orgize reads a table after affiliated keywords as a paragraph; the converter parses what
    /// follows them again. This canary fails once orgize reads it as a table: then [`mended`]'s
    /// first case can go, with this.
    #[test]
    fn orgize_still_reads_a_table_after_keywords_as_a_paragraph() {
        let org = orgize::Org::parse("#+NAME: t\n| a | b |\n");
        let kinds: Vec<SyntaxKind> = org.document().syntax().descendants().map(|node| node.kind()).collect();
        assert!(
            kinds.contains(&SyntaxKind::PARAGRAPH) && !kinds.contains(&SyntaxKind::ORG_TABLE),
            "orgize reads a table after keywords now: take the first case out of `mended`, and this canary"
        );
    }

    /// orgize reads no bracket link whose path goes on past a line's end; the converter makes those
    /// line ends spaces and parses the paragraph again. This canary fails once orgize reads it: then
    /// [`mended`]'s second case and [`links_joined`] can go, with this.
    #[test]
    fn orgize_still_reads_no_link_whose_path_wraps() {
        let org = orgize::Org::parse("See [[*A heading\nthat wraps]].\n");
        let links = org.document().syntax().descendants().filter(|node| node.kind() == SyntaxKind::LINK).count();
        assert_eq!(links, 0, "orgize reads a link that wraps now: take the second case out of `mended`, `links_joined`, and this canary");
    }

    #[test]
    fn a_table_after_keywords_and_a_link_that_wraps_are_read_as_org_has_them() {
        let html = body("#+NAME: t\n#+CAPTION: c\n| a | b |\n|---+---|\n| 1 | 2 |\nAfter.\n");
        assert!(html.starts_with("<table border=\"1\"><thead><tr><th>a</th><th>b</th></tr></thead><tbody><tr><td>1</td><td>2</td></tr></tbody></table>\n<p>After.\n</p>"), "{html}");
        assert!(!html.contains("<s>"), "its rule isn't strike-through: {html}");
        let html = body("  - item\n    #+ATTR_HTML: :width 10\n    | x |\n");
        assert!(html.contains("<table border=\"1\"><tbody><tr><td>x</td></tr></tbody></table>"), "in a list: {html}");
        let html = body("See [[https://a.org/b][a link\nthat wraps]] and [[*A heading\n   that wraps][here]], [[c][d]].\n");
        assert!(html.contains("<a href=\"*A heading that wraps\">here</a>"), "{html}");
        assert!(html.contains("<a href=\"https://a.org/b\">a link\nthat wraps</a>"), "{html}");
        assert!(html.contains("<a href=\"c\">d</a>"), "{html}");
        let html = body("#+NAME: n\nJust a paragraph [[a\nb.\n");
        assert_eq!(html, "<p>Just a paragraph [[a\nb.\n</p>\n", "nothing else to read: as it was");
    }

    #[test]
    fn a_line_end_in_a_links_path_is_a_space_and_nothing_else_moves() {
        assert_eq!(links_joined("a [[b\nc]] [[d][e\nf]] [[g\r\nh"), Some("a [[b c]] [[d][e\nf]] [[g  h".into()));
        assert_eq!(links_joined("no link\nhere"), None);
        assert_eq!(links_joined("[[a]]\n[[b]]"), None);
        assert_eq!(links_joined("é[[\n"), Some("é[[ ".into()));
    }

    /// Past [`MAX_DEPTH`], what is nested is written as its text, and nothing recurses further.
    #[test]
    fn what_is_nested_past_the_depth_is_written_as_its_text() {
        // A list nested past the depth, each level an element deeper.
        let nested: String = (0..MAX_DEPTH + 10).map(|n| format!("{}- level {n}\n", "  ".repeat(n))).collect();
        let html = std::thread::Builder::new().stack_size(64 << 20).spawn(move || html_of(&nested, None)).unwrap().join().unwrap().unwrap();
        // The document is the first level.
        assert_eq!(html.matches("<ul>").count(), MAX_DEPTH - 1, "{html}");
        assert!(html.contains(&format!("level {}", MAX_DEPTH + 9)), "the rest as its text");
    }

    /// A worker that dies, takes too long or answers what isn't an answer costs the document, said.
    #[test]
    fn a_document_the_worker_fails_on_is_refused_and_said() {
        let cache = Cache { kept: Mutex::new(Vec::new()), bytes: 1 << 20, most: 4 };
        let bytes = Arc::new(b"* A heading".to_vec());
        let killed: Convert = |_| Err(Died::Cpu);
        assert_eq!(super::html(&bytes, killed, &cache).unwrap_err(), "orgize took too long on the document");
        let heap: Convert = |_| Err(Died::Memory);
        assert_eq!(super::html(&bytes, heap, &cache).unwrap_err(), "orgize took too much memory on the document");
        let strange: Convert = |_| Ok((9, Vec::new()));
        assert_eq!(super::html(&bytes, strange, &cache).unwrap_err(), "orgize failed on the document");
        let refused: Convert = |_| Ok((REFUSED, b"the HTML it makes is more than 64 MiB".to_vec()));
        assert_eq!(super::html(&bytes, refused, &cache).unwrap_err(), "the HTML it makes is more than 64 MiB");
        assert!(cache.get(&bytes).is_none(), "a failure isn't kept");
    }

    /// The worker's code on a thread, as a process's: a document written once, and kept.
    #[test]
    fn a_document_is_written_by_the_worker_once_and_kept() {
        let cache = Cache { kept: Mutex::new(Vec::new()), bytes: 1 << 20, most: 4 };
        let bytes = Arc::new(b"* TODO A heading".to_vec());
        let written = super::html(&bytes, convert, &cache).unwrap();
        assert!(String::from_utf8_lossy(&written).contains(">TODO</span> A heading</h1>"));
        let never: Convert = |_| panic!("asked again");
        assert_eq!(super::html(&bytes, never, &cache).unwrap(), written);
    }

    #[test]
    fn the_cache_lets_go_of_what_was_used_longest_ago() {
        let cache = Cache { kept: Mutex::new(Vec::new()), bytes: 100, most: 2 };
        let entry = |n: u8| (Arc::new(vec![n; 10]), Arc::new(vec![n; 10]));
        let (a, b, c) = (entry(1), entry(2), entry(3));
        cache.put(&a.0, &a.1);
        cache.put(&b.0, &b.1);
        assert!(cache.get(&a.0).is_some());
        cache.put(&c.0, &c.1);
        assert!(cache.get(&b.0).is_none() && cache.get(&a.0).is_some() && cache.get(&c.0).is_some());
        let large = (Arc::new(vec![9; 60]), Arc::new(vec![9; 60]));
        cache.put(&large.0, &large.1);
        assert!(cache.get(&large.0).is_none(), "larger than the whole cache");
    }

    /// How much larger the HTML is than the Org it is written from (ADR-0040): over the files
    /// `ORG_CORPUS` lists, a path a line, each's size, its HTML's, and their lines and marks, with
    /// the whole's and the largest share. `ORG_CORPUS=list cargo test -j 2 --bin clv
    /// org::tests::how_much_larger_the_html_is -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measure, over a corpus of the reader's"]
    fn how_much_larger_the_html_is() {
        let list = std::fs::read_to_string(std::env::var("ORG_CORPUS").unwrap()).unwrap();
        let marks = |text: &str| text.bytes().filter(|b| b.is_ascii_punctuation() || matches!(b, b'\r' | b'\n')).count();
        let html_marks = |text: &str| text.bytes().filter(|b| matches!(b, b'<' | b'\r' | b'\n')).count();
        let (mut org, mut html, mut org_marks, mut written_marks, mut most) = (0, 0, 0, 0, (0.0, String::new()));
        for path in list.lines().filter(|line| !line.is_empty()) {
            let text = decoded(&std::fs::read(path).unwrap());
            let written = html_of(&text, None).unwrap();
            // Past clv's own start, which every document has.
            let body = written.len() - start().len();
            let share = body as f64 / text.len().max(1) as f64;
            if share > most.0 && text.len() > 4096 {
                most = (share, path.to_owned());
            }
            (org, html, org_marks, written_marks) = (org + text.len(), html + body, org_marks + marks(&text), written_marks + html_marks(&written));
        }
        println!(
            "{org} bytes of Org, {html} of HTML: {:.2}x; {org_marks} lines and marks of Org, {written_marks} of HTML: {:.2}x; at most {:.2}x ({}), of files over 4 KiB",
            html as f64 / org as f64,
            written_marks as f64 / org_marks as f64,
            most.0,
            most.1
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Whatever an Org text holds, and wherever it is cut, it is written: nothing panics, and
        /// the HTML starts with clv's own [`HEAD`] bytes.
        #[test]
        fn any_org_text_is_written(
            text in r"(\* |\*\* |TODO |\[#A\] |:t: |#\+[a-zA-Z_]{0,8}:? ?|#\+begin_[a-z]{0,6} ?|#\+end_[a-z]{0,6}|\| ?|\|-|- |1\. |\[ \] |:[A-Z]{0,4}:|\[\[|\]\]|\]\[|\[fn:[a-z]{0,2}:?|<<|>>|[*/_+=~^]|\\\\|-----|\{\{\{|@@html:|src_x\{|[a-zé ]{1,4}|\n){0,40}",
            cut in any::<prop::sample::Index>(),
        ) {
            let end = cut.index(text.len() + 1);
            let end = (0..=end).rev().find(|at| text.is_char_boundary(*at)).unwrap_or(0);
            let html = html_of(&text[..end], Some("/home/x")).unwrap();
            prop_assert!(html.starts_with("<!doctype html><!--"));
            prop_assert!(html.len() >= HEAD as usize);
        }
    }
}
