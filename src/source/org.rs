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

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Read as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use orgize::rowan::ast::AstNode;
use orgize::rowan::{GreenNode, NodeOrToken};
use orgize::{SyntaxElement, SyntaxKind, SyntaxNode, TextSize};

use super::slug::slugged;
use super::worker::{self, Died, FAILED, Session, WALL, Worker};
use super::{
    Again, Anchor, Ask, Auto, BACKENDS, Backend, Bounded, Bounds, Deferred, Effect, Format, HEAD, Input, Job, Keys, Landmark, Links, Named, Near,
    Numbers, Opened, Pool, Render, Room, Setting, Source, Target, Texts, Told, Value, Values, opener,
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
const THEME: Setting = Setting {
    name: "theme",
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

/// How wide an Org document's line is: as [`THEME`], the documents' own setting.
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
    help: "4 characters a line narrower, or wider; 72{increase-width} sets 72, 0{increase-width} the window's",
    does: "reflow",
    effect: Effect::Relayout,
    sets_width: true,
};

/// How large an Org document's text is: as [`THEME`], the documents' own setting.
const FONT: Setting = Setting {
    name: "font",
    named: &[],
    numbers: Some(Numbers { least: 50, most: 400, step: 10, unit: "%" }),
    default: Value::Number(100),
    zero: Value::Number(100),
    auto: None,
    keys: Keys::Zoom,
    option: OptionSpec { name: "--font", short: None, value: Some("N"), help: "an Org document's text at N% of its size (50-400)", alias: None },
    help: "text larger, or smaller; 150{zoom-in} sets 150%, 0{zoom-in} 100%",
    does: "have a font size",
    effect: Effect::Relayout,
    sets_width: false,
};

/// How wide an Org document's side margins are: as [`THEME`], the documents' own setting.
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
    help: "margins a character narrower, or wider; 5{increase-margin} sets 5, 0{increase-margin} back to 3",
    does: "have margins",
    effect: Effect::Relayout,
    sets_width: false,
};

/// Whether an Org document's source blocks are drawn: as [`THEME`], the documents' own setting.
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
const SETTINGS: &[&str] = &[THEME.name, WIDTH.name, FONT.name, MARGIN.name, FENCES.name];

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
        &[THEME, WIDTH, FONT, MARGIN, FENCES]
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
    /// In another file (an archive), read from it each time: to a bound where the holder can stop
    /// early, else whole, within the holder's limits.
    Held(Again, Option<Bounded>),
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
            Self::Held(_, Some(bounded)) => {
                let most = u64::try_from(MAX_SOURCE).unwrap_or(u64::MAX);
                Arc::new(bounded(most)?.ok_or_else(|| format!("the document is more than {} MiB", MAX_SOURCE >> 20))?)
            }
            Self::Held(again, None) => Arc::new(again()?),
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
    // Read again from where it is, never further than the documents' limit: a pipe is read to a
    // byte past it, and kept; a file held in another to a bound, where its holder can stop early.
    let kept = match input.holder() {
        Some((again, bounded)) => Kept::Held(again, bounded),
        None if input.is_a_file() => Kept::Disk(PathBuf::from(&path)),
        None => match input.at_most(MAX_SOURCE as u64)? {
            Some(bytes) => Kept::Memory(Arc::new(bytes)),
            None => return Err(error(format!("the document is more than {} MiB", MAX_SOURCE >> 20))),
        },
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
        Opened::Pages(pages) => Ok(Opened::Pages(Box::new(Shown(pages)))),
        Opened::Files(_) => Err(error("its HTML holds files".into())),
    }
}

/// An Org document, as the backend that opens its HTML shows it: everything is that backend's, but
/// what the links it marks as not followed ([`NOT_FOLLOWED`]) say.
struct Shown(Box<dyn Source + Send>);

impl Source for Shown {
    fn pages(&self) -> usize {
        self.0.pages()
    }

    fn size(&self, page: usize) -> Option<(f32, f32)> {
        self.0.size(page)
    }

    fn natural(&self, page: usize) -> bool {
        self.0.natural(page)
    }

    fn settings(&self) -> Vec<&'static str> {
        self.0.settings()
    }

    fn dark_form(&self) -> bool {
        self.0.dark_form()
    }

    fn settle(&self, page: usize) {
        self.0.settle(page);
    }

    fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
        self.0.renderer()
    }

    fn pool(&self, threads: usize) -> Box<dyn Pool> {
        self.0.pool(threads)
    }

    fn laid_at(&self) -> (Values, Option<Room>) {
        self.0.laid_at()
    }

    /// The document laid out again is shown so again.
    fn relayout(&self, values: Values, room: Room) -> Option<Job> {
        let job = self.0.relayout(values, room)?;
        Some(Box::new(move || job().map(|pages| Box::new(Shown(pages)) as Box<dyn Source + Send>)))
    }

    fn largest_scale(&self, page: usize) -> Option<f32> {
        self.0.largest_scale(page)
    }

    fn bounds(&self, setting: &str, room: Room, values: Values) -> Option<Bounds> {
        self.0.bounds(setting, room, values)
    }

    fn anchor(&self, page: usize, y: f32) -> Option<Anchor> {
        self.0.anchor(page, y)
    }

    fn find(&self, anchor: &Anchor) -> Option<(usize, f32)> {
        self.0.find(anchor)
    }

    fn texts(&self) -> Option<Box<dyn Texts>> {
        self.0.texts()
    }

    fn links(&self) -> Option<Box<dyn Links>> {
        Some(Box::new(Said(self.0.links()?)))
    }

    fn contents(&self) -> Option<Landmark> {
        self.0.contents()
    }
}

/// An Org document's links, as the backend that shows it finds them, those marked as not followed
/// made what they are.
struct Said(Box<dyn Links>);

impl Links for Said {
    fn set_queue(&self, wanted: Vec<Ask>) {
        self.0.set_queue(wanted);
    }

    fn try_recv(&self) -> Option<Told> {
        Some(match self.0.try_recv()? {
            Told::Page { page, links, more } => Told::Page { page, links: links.map(|links| links.into_iter().map(told).collect()), more },
            told @ Told::Name { .. } => told,
        })
    }
}

/// Where an Org link that clv doesn't follow is written to lead: a path, which the documents'
/// backend's importer keeps where it drops an `href` with any scheme but the web's, and which
/// [`told`] makes what the link is.
const NOT_FOLLOWED: &str = "clv-org-not-followed/";

/// `link`, of an Org document, with a target marked [`NOT_FOLLOWED`] made the address the document
/// wrote, for the viewer, which follows only the web's, to say what it is (`an elisp: link, which
/// isn't followed`), as it says of any document's.
fn told(mut link: super::Link) -> super::Link {
    let Target::File { path, .. } = &link.target else { return link };
    let Some(written) = path.strip_prefix(NOT_FOLLOWED).map(str::to_owned) else { return link };
    link.address = Some(written.clone()).filter(|written| written.len() <= super::MAX_ADDRESS);
    link.target = Target::Web(written);
    link
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
    let mut config = orgize::ParseConfig {
        // `_` and `^` as written: a document's `a_b` is never a subscript.
        use_sub_superscript: orgize::config::UseSubSuperscript::Nil,
        ..Default::default()
    };
    let mut org = config.clone().parse(text);
    // The TODO keywords the file declares, which its headlines are parsed by: parsed again where
    // they aren't Org's own.
    let declared = todo_keywords(org.document().syntax());
    if declared != config.todo_keywords {
        config.todo_keywords = declared;
        org = config.clone().parse(text);
    }
    let root = fixed(org.document().syntax(), &config);
    let mut writer = Writer {
        out: start(),
        home,
        notes: Notes::default(),
        definitions: definitions(&root),
        places: places(&root),
        abbreviations: abbreviations(&root),
        in_link: false,
    };
    writer.title_block(&root);
    writer.element(&root, 0);
    writer.footnotes();
    writer.out.push_str("</body></html>\n");
    match writer.out.len() > MAX_HTML {
        true => Err(format!("the HTML it makes is more than {} MiB", MAX_HTML >> 20)),
        false => Ok(writer.out),
    }
}

/// The TODO keywords the document `root` declares, not done and done, as Org reads its `#+TODO:`,
/// `#+SEQ_TODO:` and `#+TYP_TODO:` keywords, in any case: the words before a `|` not done, those
/// after it done; with no `|`, the last done; `TODO(t)` the word before its key. Only keywords
/// orgize parses as such count, as only those Emacs reads as keywords do: never a line in a block.
/// Org's own, `TODO` and `DONE`, where it declares none.
fn todo_keywords(root: &SyntaxNode) -> (Vec<String>, Vec<String>) {
    let (mut todo, mut done) = (Vec::new(), Vec::new());
    for keyword in root.descendants().filter_map(orgize::ast::Keyword::cast) {
        let key = keyword.key();
        if !["TODO", "SEQ_TODO", "TYP_TODO"].iter().any(|name| key.eq_ignore_ascii_case(name)) {
            continue;
        }
        let rest = keyword.value();
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
    fixed_counted(root, config).0
}

/// [`fixed`], and how many times the tree was built again: once a round that mends anything.
fn fixed_counted(root: &SyntaxNode, config: &orgize::ParseConfig) -> (SyntaxNode, usize) {
    let mut tree = root.clone();
    let mut built: usize = 0;
    // What is parsed again keeps every offset, so a paragraph is known by its range from one round
    // to the next: those left as they were aren't parsed again.
    let mut kept: HashSet<orgize::TextRange> = HashSet::new();
    // Round after round, since what follows a name is parsed again whole, and may hold another
    // element named, read as a paragraph again; until a round mends nothing. Each round mends what
    // it finds all at once: the tree built again once, never a paragraph at a time.
    loop {
        let mut mending: HashMap<orgize::TextRange, Vec<Green>> = HashMap::new();
        let paragraphs: Vec<SyntaxNode> =
            tree.descendants().filter(|node| node.kind() == SyntaxKind::PARAGRAPH && !kept.contains(&node.text_range())).collect();
        for paragraph in paragraphs {
            // As deep as the tree is built again; one deeper stays as orgize read it.
            let deep = paragraph.ancestors().count() > MAX_DEPTH;
            match mended(&paragraph, config).filter(|_| !deep) {
                Some(replacement) => {
                    mending.insert(paragraph.text_range(), replacement);
                }
                None => {
                    kept.insert(paragraph.text_range());
                }
            }
        }
        if mending.is_empty() {
            return (tree, built);
        }
        let mut starts: Vec<TextSize> = mending.keys().map(|range| range.start()).collect();
        starts.sort_unstable();
        tree = SyntaxNode::new_root(rebuilt(&tree, &mending, &starts));
        built = built.saturating_add(1);
    }
}

/// `node`'s tree with each paragraph `mending` has, by its range, given way to what it has: built
/// again down the nodes that hold one (`starts`, the ranges' starts in order), the rest kept as
/// they are.
fn rebuilt(node: &SyntaxNode, mending: &HashMap<orgize::TextRange, Vec<Green>>, starts: &[TextSize]) -> GreenNode {
    let holds = |node: &SyntaxNode| {
        let range = node.text_range();
        let at = starts.partition_point(|start| *start < range.start());
        starts.get(at).is_some_and(|start| *start < range.end())
    };
    let mut children: Vec<Green> = Vec::new();
    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(token) => children.push(NodeOrToken::Token(token.green().to_owned())),
            NodeOrToken::Node(child) => match mending.get(&child.text_range()).filter(|_| child.kind() == SyntaxKind::PARAGRAPH) {
                Some(replacement) => children.extend(replacement.iter().cloned()),
                // No deeper than [`fixed_counted`] took a paragraph from.
                None if holds(&child) => children.push(NodeOrToken::Node(rebuilt(&child, mending, starts))),
                None => children.push(NodeOrToken::Node(child.green().into_owned())),
            },
        }
    }
    GreenNode::new(node.green().kind(), children)
}

/// What `paragraph` should have been parsed as ([`fixed`]): the elements to put in its place, or
/// `None` where it is a paragraph.
fn mended(paragraph: &SyntaxNode, config: &orgize::ParseConfig) -> Option<Vec<Green>> {
    // A paragraph that stays one after its keywords may still hold a link that wraps.
    if let Some(elements) = named(paragraph, config) {
        return Some(elements);
    }
    let text = raw(paragraph);
    let joined = links_joined(&text)?;
    let parsed = config.clone().parse(&joined);
    let section = parsed.document().syntax().children().find(|child| child.kind() == SyntaxKind::SECTION)?;
    let mut children = section.children();
    let (Some(only), None) = (children.next(), children.next()) else { return None };
    // Every mark in a link's path, where [`unwrapped`] reads it: none left in the text.
    let marked = |token: &orgize::SyntaxToken| token.kind() != SyntaxKind::LINK_PATH && token.text().contains(WRAPPED);
    let stray = only.descendants_with_tokens().filter_map(|e| e.into_token()).any(|token| marked(&token));
    (only.kind() == SyntaxKind::PARAGRAPH && only.text_range().len() == paragraph.text_range().len() && !stray)
        .then(|| vec![NodeOrToken::Node(only.green().into_owned())])
}

/// What `paragraph`, which starts with affiliated keywords, should have been parsed as: the
/// elements what follows its keywords is ([`mended`]'s first case), or `None` where that is a
/// paragraph still.
fn named(paragraph: &SyntaxNode, config: &orgize::ParseConfig) -> Option<Vec<Green>> {
    let keywords: Vec<SyntaxNode> = paragraph.children().take_while(|child| child.kind() == SyntaxKind::AFFILIATED_KEYWORD).collect();
    let last = keywords.last()?;
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
    Some(out)
}

/// What [`mended`] puts in a paragraph's place: elements and the blank lines after them.
type Green = NodeOrToken<GreenNode, orgize::rowan::GreenToken>;

/// `text`, a paragraph's, with each line's end inside a bracket link's path (between a `[[` and
/// the next `]`, with no `[` between) made a space, as many bytes as it was; `None` where there is
/// none.
fn links_joined(text: &str) -> Option<String> {
    let mut out = text.as_bytes().to_vec();
    let mut changed = false;
    let mut at: usize = 0;
    while let Some(found) = text.get(at..).and_then(|rest| rest.find("[[")) {
        let start = at.saturating_add(found).saturating_add(2);
        let Some(path) = text.get(start..) else { break };
        let end = path.find([']', '[']).unwrap_or(path.len());
        // Only a path a `]` closes: anything else is no link, and its line ends are the text's.
        let closed = path.get(end..).is_some_and(|rest| rest.starts_with(']'));
        for (n, byte) in path.bytes().take(if closed { end } else { 0 }).enumerate() {
            if matches!(byte, b'\n' | b'\r')
                && let Some(slot) = out.get_mut(start.saturating_add(n))
            {
                *slot = WRAPPED as u8;
                changed = true;
            }
        }
        at = start.saturating_add(end).max(at.saturating_add(1));
    }
    // Only line ends, each one byte, were made another byte below 128: still UTF-8.
    changed.then(|| String::from_utf8(out).ok()).flatten()
}

/// What a line's end in a link's path is made by [`links_joined`]: a control of ASCII's, one byte as
/// a line's end is, which orgize reads in a path where it reads no line's end, and [`unwrapped`]
/// makes a space with the blanks around it.
const WRAPPED: char = '\u{0B}';

/// Each footnote definition (`[fn:name] …` on a line of its own) in `root`, by its label: the first
/// of a label.
fn definitions(root: &SyntaxNode) -> HashMap<String, SyntaxNode> {
    let mut found = HashMap::new();
    for node in root.descendants().filter(|node| matches!(node.kind(), SyntaxKind::FN_DEF | SyntaxKind::FN_REF)) {
        let inline = node.kind() == SyntaxKind::FN_REF && is_inline(&node);
        if (node.kind() == SyntaxKind::FN_DEF || inline)
            && !hidden(&node)
            && let Some(label) = footnote_label(&node)
        {
            found.entry(label).or_insert(node);
        }
    }
    found
}

/// Whether a footnote's reference, `node`, defines its note inline: what comes after a second `:`.
fn is_inline(node: &SyntaxNode) -> bool {
    node.children_with_tokens().filter(|e| e.kind() == SyntaxKind::COLON).count() >= 2
}

/// Whether `node` is under what isn't shown, or is it: a drawer, a comment, a keyword, an export
/// block but HTML's. What is there is neither a note nor a place.
fn hidden(node: &SyntaxNode) -> bool {
    use SyntaxKind as K;
    node.ancestors().any(|above| match above.kind() {
        K::DRAWER | K::PROPERTY_DRAWER | K::COMMENT_BLOCK | K::COMMENT | K::KEYWORD | K::BABEL_CALL => true,
        K::EXPORT_BLOCK => !is_html(&above),
        _ => false,
    })
}

/// Whether an export block, `node`, is HTML's (`#+begin_export html`), which is shown.
fn is_html(node: &SyntaxNode) -> bool {
    node.descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .any(|t| t.kind() == SyntaxKind::EXPORT_BLOCK_TYPE && t.text().eq_ignore_ascii_case("html"))
}

/// The label of a footnote's reference or definition, `[fn:label…`: `None` for one with none
/// (`[fn:: inline]`).
fn footnote_label(node: &SyntaxNode) -> Option<String> {
    let mut tokens = node.children_with_tokens().filter_map(|child| child.into_token());
    // `[`, `fn`, `:`, then the label.
    let label = tokens.nth(3)?;
    (label.kind() == SyntaxKind::TEXT && !label.text().is_empty()).then(|| label.text().to_owned())
}

/// The longest name a place is given an `id` by: what the documents' backend finds a place by.
const MAX_ID: usize = 256;

/// The places of a document ([`places`]), each written as one `id` or more where it is, and what
/// a link finds each by.
#[derive(Default)]
struct Places {
    /// Every id given, so that none is given twice.
    used: HashSet<String>,
    /// The ids written where each element or object starts, by its start and its kind.
    at: HashMap<(TextSize, SyntaxKind), Vec<String>>,
    /// The id a heading's custom id (`:CUSTOM_ID:`) and its id (`:ID:`) lead to, as written.
    custom: HashMap<String, String>,
    ids: HashMap<String, String>,
    /// The id a target's (`<<target>>`), a name's (`#+NAME:`) and a heading's title lead to,
    /// [folded](folded): the first of each.
    targets: HashMap<String, String>,
    names: HashMap<String, String>,
    titles: HashMap<String, String>,
}

impl Places {
    /// `name` as an id, if it can be one ([`MAX_ID`]) and none has it, in any case: the documents'
    /// backend finds a place by its name in any case too.
    fn claim(&mut self, name: &str) -> Option<String> {
        let folded = name.to_lowercase();
        if name.is_empty() || name.len() > MAX_ID || self.used.contains(&folded) {
            return None;
        }
        self.used.insert(folded);
        Some(name.to_owned())
    }

    /// `base`, short, as an id none has: as it is, else with `-1`, `-2`, … after it. Among as many
    /// as are given, plus one, one is free.
    fn unique(&mut self, base: &str) -> String {
        let tries = self.used.len().saturating_add(1);
        std::iter::once(base.to_owned()).chain((1..=tries).map(|n| format!("{base}-{n}"))).find_map(|name| self.claim(&name)).unwrap_or_default()
    }

    /// `id` written where `node` starts.
    fn mark(&mut self, node: &SyntaxNode, id: String) {
        self.at.entry((node.text_range().start(), node.kind())).or_default().push(id);
    }

    /// The ids written where `node` starts.
    fn of(&self, node: &SyntaxNode) -> &[String] {
        self.at.get(&(node.text_range().start(), node.kind())).map_or(&[], Vec::as_slice)
    }
}

/// `text` as a name is matched: its blank space collapsed, in lower case.
fn folded(text: &str) -> String {
    collapsed(text).to_lowercase()
}

/// The places in `root` and their ids: the names the document gives itself first, in its order, a
/// heading's custom id and id, a target and a name, each written as it is; then each heading, by
/// its title as Org matches `[[*Heading]]` ([`title_of`]) and by GitHub's slug of what it shows
/// ([`slugged`]), so that a Markdown file's `notes.org#set-up` finds it, where no name has them
/// already; a heading with neither, by an id of its own, so that a link finds it still. A name the
/// document gives itself so wins over a heading's.
fn places(root: &SyntaxNode) -> Places {
    use SyntaxKind as K;
    let mut places = Places::default();
    for node in root.descendants().filter(|node| !hidden(node)) {
        match node.kind() {
            K::HEADLINE => {
                for (key, value) in properties(&node) {
                    let value = value.trim().to_owned();
                    let custom = key.eq_ignore_ascii_case("CUSTOM_ID");
                    if !custom && !key.eq_ignore_ascii_case("ID") {
                        continue;
                    }
                    // The heading's own id of that name, given for its other property, leads to it too.
                    let own = places.of(&node).iter().find(|id| id.eq_ignore_ascii_case(&value)).cloned();
                    let claimed = places.claim(&value);
                    let Some(id) = claimed.clone().or(own) else { continue };
                    let map = if custom { &mut places.custom } else { &mut places.ids };
                    map.entry(value).or_insert_with(|| id.clone());
                    if claimed.is_some() {
                        places.mark(&node, id);
                    }
                }
            }
            K::TARGET | K::RADIO_TARGET => {
                let name = collapsed(raw(&node).trim_start_matches('<').trim_end_matches('>'));
                if let Some(id) = places.claim(&name) {
                    places.targets.entry(folded(&name)).or_insert_with(|| id.clone());
                    places.mark(&node, id);
                }
            }
            K::AFFILIATED_KEYWORD => {
                let Some(keyword) = orgize::ast::AffiliatedKeyword::cast(node.clone()) else { continue };
                let (true, Some(name), Some(element)) = (keyword.key().eq_ignore_ascii_case("NAME"), keyword.value(), node.parent()) else {
                    continue;
                };
                if element.kind() == K::FN_DEF {
                    continue;
                }
                let name = collapsed(&name);
                if let Some(id) = places.claim(&name) {
                    places.names.entry(folded(&name)).or_insert_with(|| id.clone());
                    places.mark(&element, id);
                }
            }
            _ => {}
        }
    }
    let mut slugs = HashMap::new();
    for node in root.descendants().filter(|node| node.kind() == K::HEADLINE) {
        let title = title_of(&node);
        let shown = shown_title(&node);
        let slug = slugged(&shown, &mut slugs);
        // And `*` before its title, as written and as it shows, which only a heading is found by:
        // another file's `[[file:x.org::*Heading]]` leads to `x.org#*Heading`, never to a target
        // of that name.
        let searched = [format!("*{title}"), format!("*{shown}")];
        for name in [title.clone(), slug].into_iter().chain(searched) {
            if let Some(id) = places.claim(&name) {
                places.mark(&node, id);
            }
        }
        if places.of(&node).is_empty() {
            let id = places.unique("heading");
            places.mark(&node, id);
        }
        if let Some(anchor) = places.of(&node).first().cloned() {
            places.titles.entry(folded(&title)).or_insert_with(|| anchor.clone());
            places.titles.entry(folded(&shown)).or_insert(anchor);
        }
    }
    places
}

/// The properties of a headline, `node`, in its drawer: each its name and its value.
fn properties(node: &SyntaxNode) -> Vec<(String, String)> {
    let Some(drawer) = node.children().find_map(orgize::ast::PropertyDrawer::cast) else { return Vec::new() };
    drawer.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect()
}

/// A headline's title, `node`'s: as Org matches `[[*Heading]]`, as written without its statistics
/// cookies (`[1/3]`), and without `COMMENT`, its TODO keyword, priority and tags (which orgize
/// keeps out of it). Its blank space collapsed. What it shows is [`shown_title`].
fn title_of(node: &SyntaxNode) -> String {
    use SyntaxKind as K;
    let Some(title) = node.children().find(|child| child.kind() == K::HEADLINE_TITLE) else { return String::new() };
    let mut text = String::new();
    let mut stack: Vec<SyntaxElement> = title.children_with_tokens().collect();
    stack.reverse();
    while let Some(element) = stack.pop() {
        match element {
            NodeOrToken::Node(node) if node.kind() == K::COOKIE => {}
            NodeOrToken::Node(node) => stack.extend(node.children_with_tokens().collect::<Vec<_>>().into_iter().rev()),
            NodeOrToken::Token(token) => text.push_str(token.text()),
        }
    }
    let text = collapsed(&text);
    match text.strip_prefix("COMMENT") {
        Some(rest) if rest.is_empty() || rest.starts_with(' ') => rest.trim().to_owned(),
        _ => text,
    }
}

/// A headline's title, `node`'s, as it shows, which GitHub's slug is made from: its text as
/// [`Writer`] writes it, an entity the character it names, a link its description or its address,
/// code and verbatim their text, emphasis without its marks; without its statistics cookies, a
/// footnote's number and a target, as its title as Org matches it ([`title_of`]) is. Its blank
/// space collapsed.
fn shown_title(node: &SyntaxNode) -> String {
    let Some(title) = node.children().find(|child| child.kind() == SyntaxKind::HEADLINE_TITLE) else { return String::new() };
    let mut text = String::new();
    shown(title.children_with_tokens().collect(), &mut text, 0);
    collapsed(&text)
}

/// `elements`, objects, as they show, onto `text`: [`shown_title`]'s, `depth` down.
fn shown(elements: Vec<SyntaxElement>, text: &mut String, depth: usize) {
    use SyntaxKind as K;
    for element in elements {
        let node = match element {
            NodeOrToken::Token(token) => {
                text.push_str(token.text());
                continue;
            }
            NodeOrToken::Node(node) => node,
        };
        if depth > MAX_DEPTH {
            text.push_str(&raw(&node));
            continue;
        }
        let inner = || {
            let children: Vec<SyntaxElement> = node.children_with_tokens().collect();
            let end = children.len().saturating_sub(1);
            children.into_iter().take(end).skip(1)
        };
        match node.kind() {
            K::BOLD | K::ITALIC | K::UNDERLINE | K::STRIKE | K::VERBATIM | K::CODE | K::RADIO_TARGET => {
                shown(inner().collect(), text, depth.saturating_add(1))
            }
            K::COOKIE | K::FN_REF | K::TARGET | K::SNIPPET => {}
            K::ENTITY => match orgize::ast::Entity::cast(node.clone()) {
                Some(entity) if !entity.utf8().is_empty() => text.push_str(entity.utf8()),
                _ => text.push_str(&raw(&node)),
            },
            K::LINE_BREAK => text.push(' '),
            K::INLINE_SRC => {
                let source = raw(&node);
                text.push_str(source.split_once('{').and_then(|(_, body)| body.rsplit_once('}')).map_or(source.as_str(), |(body, _)| body));
            }
            K::LINK => match orgize::ast::Link::cast(node.clone()) {
                Some(link) if link.has_description() => {
                    let description =
                        node.children_with_tokens().skip_while(|e| e.kind() != K::L_BRACKET).skip(1).filter(|e| e.kind() != K::R_BRACKET2);
                    shown(description.collect(), text, depth.saturating_add(1));
                }
                Some(link) => {
                    let path = unwrapped(&link.path());
                    if picture(&path).is_none() {
                        text.push_str(path.strip_prefix('*').unwrap_or(&path));
                    }
                }
                None => {}
            },
            _ => text.push_str(&raw(&node)),
        }
    }
}

/// `path`, a link's as written, with each line's end in it and the blanks around it one space, as Org
/// reads a link that wraps; the rest of its spaces as they are. [`links_joined`] marks a line's
/// end so ([`WRAPPED`]).
fn unwrapped(path: &str) -> String {
    let ends = |c: char| matches!(c, '\n' | '\r' | WRAPPED);
    if !path.contains(ends) {
        return path.to_owned();
    }
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if !ends(c) {
            out.push(c);
            continue;
        }
        let kept = out.trim_end_matches([' ', '\t']).len();
        out.truncate(kept);
        while chars.next_if(|c| ends(*c) || matches!(c, ' ' | '\t')).is_some() {}
        out.push(' ');
    }
    out
}

/// The link abbreviations `root` declares (`#+LINK: word https://…/%s`): each its word and what it
/// stands for.
fn abbreviations(root: &SyntaxNode) -> Vec<(String, String)> {
    root.descendants()
        .filter_map(orgize::ast::Keyword::cast)
        .filter(|keyword| keyword.key().eq_ignore_ascii_case("LINK"))
        .filter_map(|keyword| {
            let value = keyword.value().trim().to_owned();
            let (word, url) = value.split_once(char::is_whitespace)?;
            Some((word.to_owned(), url.trim().to_owned()))
        })
        .collect()
}

/// The longest address an abbreviation is expanded to: longer than the viewer follows.
const MAX_EXPANDED: usize = 8 << 10;

/// `text` with every character a URL can't hold as it is (a space, a control, `"`, `#`, `%`, `<`,
/// `>`, and the rest RFC 3986 leaves out) percent-encoded, as its UTF-8 bytes: what is read back
/// from an `href` as it was.
fn encoded(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() || matches!(c, ' ' | '"' | '#' | '%' | '?' | '<' | '>' | '\\' | '^' | '`' | '{' | '|' | '}') {
            let mut bytes = [0; 4];
            for byte in c.encode_utf8(&mut bytes).bytes() {
                let _ = write!(out, "%{byte:02X}");
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `text` with every byte but a letter, a digit, `-`, `.`, `_` and `~` percent-encoded, as Org's
/// `%h` writes an abbreviation's tag.
fn hexified(text: &str) -> String {
    text.bytes().fold(String::new(), |mut out, byte| {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
        out
    })
}

/// `address`, a link's with a scheme: as it is, a web address the viewer follows (`http`, `https`,
/// `mailto`), or else marked [`NOT_FOLLOWED`], for what it is to be said.
fn web_or_not(address: String) -> String {
    let scheme = address.split_once(':').map(|(scheme, _)| scheme.to_ascii_lowercase());
    match scheme.as_deref() {
        Some("http" | "https" | "mailto") => address,
        _ => format!("{NOT_FOLLOWED}{}", encoded(&address)),
    }
}

/// `path`, a file's, as a path the documents' backend can't read as a URI: a relative one whose first
/// name holds a colon (`notes:2026.org`, a scheme and its path to a URL's reader) after `./`.
fn relative(path: &str) -> std::borrow::Cow<'_, str> {
    let first = path.split('/').next().unwrap_or_default();
    match !path.starts_with('/') && first.contains(':') {
        true => format!("./{path}").into(),
        false => path.into(),
    }
}

/// Whether `text` starts with a URI's scheme and a colon (`https:`, `elisp:`).
fn has_scheme(text: &str) -> bool {
    let Some((scheme, _)) = text.split_once(':') else { return false };
    scheme.starts_with(|c: char| c.is_ascii_alphabetic()) && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// The file a link's path names, if it names one: after `file:`, `file+sys:` or `file+emacs:`, or a
/// path that starts as one does, `./`, `../`, `/` or `~`.
fn file_of(path: &str) -> Option<&str> {
    for prefix in ["file:", "file+sys:", "file+emacs:"] {
        if path.get(..prefix.len()).is_some_and(|start| start.eq_ignore_ascii_case(prefix)) {
            return path.get(prefix.len()..);
        }
    }
    ["./", "../", "/", "~"].iter().any(|start| path.starts_with(start)).then_some(path)
}

/// The first link `text` writes plainly, as Org finds one in text: an angle link, `<type:path>`, or
/// a bare `http://`, `https://` or `mailto:` address, from a word's start to a space or a bracket,
/// the punctuation at its end left out. What is before it, its path, and what is after it.
fn autolink(text: &str) -> Option<(&str, &str, &str)> {
    let mut at: usize = 0;
    loop {
        let start = at.saturating_add(text.get(at..)?.find(['<', 'h', 'H', 'm', 'M'])?);
        let here = text.get(start..)?;
        if let Some(inside) = here.strip_prefix('<').and_then(|rest| rest.split_once('>')).map(|(inside, _)| inside)
            && has_scheme(inside)
            && !inside.contains(['<', '\n'])
        {
            return Some((text.get(..start)?, inside, here.get(inside.len().saturating_add(2)..)?));
        }
        let at_a_word = text.get(..start)?.chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let scheme =
            ["http://", "https://", "mailto:"].into_iter().find(|scheme| here.get(..scheme.len()).is_some_and(|s| s.eq_ignore_ascii_case(scheme)));
        if let (true, Some(scheme)) = (at_a_word, scheme) {
            let end = url_end(here);
            let link = here.get(..end)?.trim_end_matches(['.', ',', ';', ':', '!', '?']);
            if link.len() > scheme.len() {
                return Some((text.get(..start)?, link, here.get(link.len()..)?));
            }
        }
        at = start.saturating_add(here.chars().next()?.len_utf8());
    }
}

/// Where a bare address that starts `text` ends, as Org's plain links do: at a space or a bracket,
/// a quote, `<` or `>`; at a `)` only where it closes no `(` of the address's own, so that
/// `…/Org-mode_(disambiguation)` keeps its parentheses and `(see https://x.org)` leaves its own.
fn url_end(text: &str) -> usize {
    let mut open: usize = 0;
    for (at, c) in text.char_indices() {
        match c {
            '(' => open = open.saturating_add(1),
            ')' if open == 0 => return at,
            ')' => open = open.saturating_sub(1),
            c if c.is_whitespace() || "<>[]\"'".contains(c) => return at,
            _ => {}
        }
    }
    text.len()
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
    places: Places,
    abbreviations: Vec<(String, String)>,
    /// Whether what is written is a link's description, where no address is a link of its own.
    in_link: bool,
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
                let id = self.id_of(node);
                let _ = write!(self.out, "<p{id}>");
                self.objects(node.children_with_tokens(), deeper);
                self.push("</p>\n");
            }
            K::LIST => self.list(node, deeper),
            K::ORG_TABLE => self.table(node, deeper),
            K::TABLE_EL => self.pre(&raw(node), None, node),
            K::SOURCE_BLOCK => {
                let language = node
                    .descendants_with_tokens()
                    .filter_map(|e| e.into_token())
                    .find(|t| t.kind() == K::SRC_BLOCK_LANGUAGE)
                    .map(|t| t.text().to_owned());
                self.pre(&unescaped(&block_content(node)), language.as_deref(), node);
            }
            K::EXAMPLE_BLOCK => self.pre(&unescaped(&block_content(node)), None, node),
            // As Markdown's HTML is: read by the importer as it is.
            K::EXPORT_BLOCK if is_html(node) => {
                let id = self.id_of(node);
                let _ = writeln!(self.out, "<div{id}>{}</div>", block_content(node));
            }
            K::QUOTE_BLOCK => self.block(node, "<blockquote>", "</blockquote>\n", deeper),
            K::CENTER_BLOCK => self.block(node, "<center>", "</center>\n", deeper),
            K::SPECIAL_BLOCK => self.block(node, "<div>", "</div>\n", deeper),
            K::VERSE_BLOCK => {
                let id = self.id_of(node);
                let _ = write!(self.out, "<p{id}>");
                if let Some(content) = node.children().find(|child| child.kind() == K::BLOCK_CONTENT) {
                    self.verse(&content, deeper);
                }
                self.push("</p>\n");
            }
            K::DYN_BLOCK => {
                let id = self.id_of(node);
                let _ = write!(self.out, "<div{id}>");
                for child in node.children().filter(|child| !matches!(child.kind(), K::DYN_BLOCK_BEGIN | K::DYN_BLOCK_END)) {
                    self.element(&child, deeper);
                }
                self.push("</div>\n");
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
                self.pre(&lines.join("\n"), None, node);
            }
            K::LATEX_ENVIRONMENT => self.pre(raw(node).trim_end(), None, node),
            K::RULE => {
                let id = self.id_of(node);
                let _ = writeln!(self.out, "<hr{id}>");
            }
            K::CLOCK => {
                let id = self.id_of(node);
                let _ = writeln!(self.out, "<p{id} style=\"font-size:9pt;color:{GREY}\">{}</p>", escaped(&collapsed(&raw(node))));
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
        let id = self.id_of(node);
        let _ = write!(self.out, "{}{id}>", open.trim_end_matches('>'));
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
    fn pre(&mut self, text: &str, language: Option<&str>, node: &SyntaxNode) {
        let id = self.id_of(node);
        let text = text.strip_suffix('\n').unwrap_or(text);
        let class = language.filter(|language| !language.is_empty()).map(|language| format!(" class=\"language-{}\"", escaped(language)));
        let _ = writeln!(self.out, "<pre{id}><code{}>{}</code></pre>", class.unwrap_or_default(), escaped(text));
    }

    /// A headline, `node`: its heading (its TODO keyword in colour, its priority and its title, its
    /// tags in grey), its planning line, small and grey, then what is under it.
    fn headline(&mut self, node: &SyntaxNode, depth: usize) {
        use SyntaxKind as K;
        let stars = node.children_with_tokens().filter_map(|e| e.into_token()).find(|t| t.kind() == K::HEADLINE_STARS);
        let level = stars.map_or(1, |stars| stars.text().len()).clamp(1, 6);
        let _ = write!(self.out, "<h{level}>");
        self.anchors(node);
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
        let id = self.id_of(node);
        let _ = write!(self.out, "{}{id}>", open.trim_end_matches('>'));
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
        let id = self.id_of(node);
        let _ = write!(self.out, "<table border=\"1\"{id}>");
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
            // A place, not shown.
            K::TARGET => self.anchors(node),
            K::RADIO_TARGET => {
                self.anchors(node);
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
        let mut rest = text;
        while !self.in_link
            && let Some((before, path, after)) = autolink(rest)
        {
            let href = escaped(&self.address(path));
            let _ = write!(self.out, "{}<a href=\"{href}\">{}</a>", escaped(before), escaped(path));
            rest = after;
        }
        self.push(&escaped(rest));
    }

    /// The `id` attribute of the element `node`, for the place it is, if it is one: a name's.
    fn id_of(&self, node: &SyntaxNode) -> String {
        self.places.of(node).first().map(|id| format!(" id=\"{}\"", escaped(id))).unwrap_or_default()
    }

    /// An anchor for each place `node` is, where it starts: a heading's, a target's.
    fn anchors(&mut self, node: &SyntaxNode) {
        let anchors: String = self.places.of(node).iter().map(|id| format!("<a id=\"{}\"></a>", escaped(id))).collect();
        self.push(&anchors);
    }

    /// A link, `node`: a picture where it is one ([`picture`]), else its description, or its path
    /// where it has none, leading where its path does.
    fn link(&mut self, node: &SyntaxNode, depth: usize) {
        use SyntaxKind as K;
        let Some(link) = orgize::ast::Link::cast(node.clone()) else { return };
        let path = unwrapped(&link.path());
        let described = link.has_description();
        // An abbreviation expanded first: `[[pic:one]]` is a picture where `pic` stands for one.
        if !described && let Some(src) = picture(&self.expanded(&path)) {
            let _ = write!(self.out, "<img src=\"{}\" alt=\"\">", escaped(&encoded(&relative(&src))));
            return;
        }
        let href = escaped(&self.address(&path));
        let _ = write!(self.out, "<a href=\"{href}\">");
        if described {
            match picture(&self.expanded(unwrapped(&link.description_raw()).trim())) {
                Some(src) => {
                    let _ = write!(self.out, "<img src=\"{}\" alt=\"\">", escaped(&encoded(&relative(&src))));
                }
                None => {
                    let description: Vec<SyntaxElement> =
                        node.children_with_tokens().skip_while(|e| e.kind() != K::L_BRACKET).skip(1).filter(|e| e.kind() != K::R_BRACKET2).collect();
                    self.in_link = true;
                    self.objects(description.into_iter(), depth);
                    self.in_link = false;
                }
            }
        } else {
            self.push(&escaped(path.strip_prefix('*').unwrap_or(&path)));
        }
        self.push("</a>");
    }

    /// Where a link whose path is `path` leads, as an `href`, the place resolved here, as Org has it:
    /// `*Heading` to the heading of that title, `#custom-id` and `id:X` to the heading that has it;
    /// a path with no type (`[[target]]`) to a target, else a name, else a heading's title; a file,
    /// with what follows its `::` as the place in it; `doi:X` to `https://doi.org/X`; anything else
    /// with a type as it is, which the viewer follows or says what it is. A place not found is an
    /// `href` to a place of that name, which the viewer says isn't in the document. An abbreviation
    /// is expanded first ([`Writer::expanded`]).
    fn address(&self, path: &str) -> String {
        let path = self.expanded(path);
        let to = |id: Option<&String>, said: &str| format!("#{}", encoded(id.map_or(said, String::as_str)));
        if let Some(title) = path.strip_prefix('*') {
            return to(self.places.titles.get(&folded(title)), &path);
        }
        if let Some(custom) = path.strip_prefix('#') {
            return to(self.places.custom.get(custom.trim()), custom.trim());
        }
        if let Some(id) = path.strip_prefix("id:") {
            return to(self.places.ids.get(id.trim()), &path);
        }
        if let Some(doi) = path.strip_prefix("doi:") {
            return format!("https://doi.org/{}", doi.trim());
        }
        if let Some(file) = file_of(&path) {
            return self.file(file);
        }
        if has_scheme(&path) {
            return web_or_not(path);
        }
        let name = folded(&path);
        let places = &self.places;
        to(places.targets.get(&name).or_else(|| places.names.get(&name)).or_else(|| places.titles.get(&name)), &path)
    }

    /// Where a link to the file `link` leads (`other.org::*Heading`): its path, `~` the reader's home
    /// (another's, `~user`, refused, as a `file:` link the viewer doesn't follow), and after `::` the
    /// place in it, a heading's title, a custom id or a target; a line's number or a `/regex/` none,
    /// the file opened at its top.
    fn file(&self, link: &str) -> String {
        let (file, search) = link.split_once("::").map_or((link, None), |(file, search)| (file, Some(search.trim())));
        let file = match file.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => match self.home {
                Some(home) => format!("{home}{rest}"),
                None => return web_or_not(format!("file:{link}")),
            },
            Some(_) => return web_or_not(format!("file:{link}")),
            None => file.to_owned(),
        };
        let place = search.and_then(|search| {
            let regex = search.len() > 1 && search.starts_with('/') && search.ends_with('/');
            if search.is_empty() || regex || search.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            // `*Heading` as `*` and the title, which only a heading is found by.
            Some(match search.strip_prefix('*') {
                Some(title) => format!("*{}", collapsed(title)),
                None => collapsed(search.strip_prefix('#').unwrap_or(search)),
            })
        });
        match place.filter(|place| !place.is_empty()) {
            Some(place) => format!("{}#{}", encoded(&relative(&file)), encoded(&place)),
            None => encoded(&relative(&file)),
        }
    }

    /// `path` with an abbreviation the document declares (`#+LINK:`) expanded: `word:tag` as what
    /// `word` stands for, its `%s` the tag and its `%h` the tag percent-encoded, or the tag after it
    /// where it has neither; as it is where that would be longer than [`MAX_EXPANDED`].
    fn expanded(&self, path: &str) -> String {
        let (word, tag) = path.split_once(':').unwrap_or((path, ""));
        let Some((_, url)) = self.abbreviations.iter().find(|(abbreviation, _)| abbreviation == word) else { return path.to_owned() };
        let marks = url.matches("%s").count().saturating_add(url.matches("%h").count());
        if url.len().saturating_add(marks.max(1).saturating_mul(tag.len().saturating_mul(3))) > MAX_EXPANDED {
            return path.to_owned();
        }
        if marks == 0 {
            return format!("{url}{tag}");
        }
        // The template's own `%s` and `%h`, in one pass: what the tag holds is never read for them.
        let mut out = String::with_capacity(url.len().saturating_add(tag.len()));
        let mut rest = url.as_str();
        while let Some(at) = rest.find('%') {
            out.push_str(rest.get(..at).unwrap_or_default());
            let after = rest.get(at.saturating_add(1)..).unwrap_or_default();
            match after.chars().next() {
                Some('s') => out.push_str(tag),
                Some('h') => out.push_str(&hexified(tag)),
                _ => {
                    out.push('%');
                    rest = after;
                    continue;
                }
            }
            rest = after.get(1..).unwrap_or_default();
        }
        out.push_str(rest);
        out
    }

    /// A footnote's reference, `node`: its number, as a superscript leading to the note; or as
    /// written, for a note defined nowhere.
    fn reference(&mut self, node: &SyntaxNode) {
        let label = footnote_label(node);
        // An inline definition: what comes after a second `:`.
        let inline = is_inline(node);
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
                let id = self.places.unique(&format!("fn.{}", self.notes.listed.len().saturating_add(1)));
                self.notes.listed.push((id, defined));
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
                let id = self.places.unique(&format!("fn.{}", self.notes.listed.len().saturating_add(1)));
                self.notes.listed.push((id, node));
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
        assert!(html.contains(&format!("<h1><a id=\"First\"></a><a id=\"*First\"></a><span style=\"color:{NOT_DONE}\">TODO</span> [#A] First [1/3] <span style=\"color:{GREY};font-weight:normal\">:work:urgent:</span></h1>")), "{html}");
        assert!(html.contains(&format!("<p style=\"font-size:9pt;color:{GREY}\">SCHEDULED: &lt;2024-01-02 Tue&gt;</p>")), "{html}");
        assert!(
            html.contains(&format!("<h2><a id=\"Second\"></a><a id=\"*Second\"></a><span style=\"color:{DONE}\">DONE</span> Second</h2>")),
            "{html}"
        );
        assert!(html.contains("<h6><a id=\"Seventh\"></a><a id=\"*Seventh\"></a>Seventh</h6>"), "a level past 6 as 6: {html}");
    }

    #[test]
    fn the_todo_keywords_a_file_declares_are_honoured() {
        let html = body("#+TODO: WAIT(w) NEXT | GONE(g@)\n#+seq_todo: LATER\n* WAIT one\n* GONE two\n* LATER three\n* TODO four\n");
        assert!(html.contains(&format!("<span style=\"color:{NOT_DONE}\">WAIT</span> one")), "{html}");
        assert!(html.contains(&format!("<span style=\"color:{DONE}\">GONE</span> two")), "{html}");
        assert!(html.contains(&format!("<span style=\"color:{DONE}\">LATER</span> three")), "the last done, without a bar: {html}");
        assert!(
            html.contains("<h1><a id=\"TODO four\"></a><a id=\"todo-four\"></a><a id=\"*TODO four\"></a>TODO four</h1>"),
            "Org's own no longer, once a file declares its own: {html}"
        );
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
            "<a href=\"d.png\">a description</a>",
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
            ["overview", "PROPERTIES", "logged", "a comment", "hidden", "INCLUDE", "other.org", "LaTeX", "run()", "#+NAME", "captioned", "\\no"]
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
    /// follows them again. This canary fails once orgize reads it as a table: then [`named`], and
    /// its call in [`mended`], can go, with this.
    #[test]
    fn orgize_still_reads_a_table_after_keywords_as_a_paragraph() {
        let org = orgize::Org::parse("#+NAME: t\n| a | b |\n");
        let kinds: Vec<SyntaxKind> = org.document().syntax().descendants().map(|node| node.kind()).collect();
        assert!(
            kinds.contains(&SyntaxKind::PARAGRAPH) && !kinds.contains(&SyntaxKind::ORG_TABLE),
            "orgize reads a table after keywords now: take `named` out, and its call in `mended`, and this canary"
        );
    }

    /// orgize reads no bracket link whose path goes on past a line's end; the converter makes those
    /// line ends spaces and parses the paragraph again. This canary fails once orgize reads it,
    /// alone or after affiliated keywords: then the rest of [`mended`], [`links_joined`] and
    /// [`unwrapped`]'s marks can go, with this.
    #[test]
    fn orgize_still_reads_no_link_whose_path_wraps() {
        for org in ["See [[*A heading\nthat wraps]].\n", "#+NAME: p\nSee [[*A heading\nthat wraps]].\n"] {
            let org = orgize::Org::parse(org);
            let links = org.document().syntax().descendants().filter(|node| node.kind() == SyntaxKind::LINK).count();
            assert_eq!(links, 0, "orgize reads a link that wraps now: take the rest of `mended` out, `links_joined`, and this canary");
        }
    }

    #[test]
    fn a_table_after_keywords_and_a_link_that_wraps_are_read_as_org_has_them() {
        let html = body("#+NAME: t\n#+CAPTION: c\n| a | b |\n|---+---|\n| 1 | 2 |\nAfter.\n");
        assert!(html.starts_with("<table border=\"1\" id=\"t\"><thead><tr><th>a</th><th>b</th></tr></thead><tbody><tr><td>1</td><td>2</td></tr></tbody></table>\n<p>After.\n</p>"), "{html}");
        assert!(!html.contains("<s>"), "its rule isn't strike-through: {html}");
        let html = body("  - item\n    #+ATTR_HTML: :width 10\n    | x |\n");
        assert!(html.contains("<table border=\"1\"><tbody><tr><td>x</td></tr></tbody></table>"), "in a list: {html}");
        let html = body("See [[https://a.org/b][a link\nthat wraps]] and [[*A heading\n   that wraps][here]], [[c][d]].\n");
        assert!(html.contains("<a href=\"#*A%20heading%20that%20wraps\">here</a>"), "a heading this document hasn't: {html}");
        assert!(html.contains("<a href=\"https://a.org/b\">a link\nthat wraps</a>"), "{html}");
        assert!(html.contains("<a href=\"#c\">d</a>"), "{html}");
        let html = body("#+NAME: n\nJust a paragraph [[a\nb.\n");
        assert_eq!(html, "<p id=\"n\">Just a paragraph [[a\nb.\n</p>\n", "nothing else to read: as it was, named");
    }

    #[test]
    fn a_line_end_in_a_links_path_is_a_space_and_nothing_else_moves() {
        assert_eq!(links_joined("a [[b\nc]] [[d][e\nf]] [[g\r\nh]]"), Some("a [[b\u{b}c]] [[d][e\nf]] [[g\u{b}\u{b}h]]".into()));
        // A `[[` no `]` closes is no link: its line ends are the text's.
        assert_eq!(links_joined("a [[g\nh [[i\nj"), None);
        // Nor one a single `]` closes, which orgize reads as text: no mark is left in it.
        assert_eq!(body("A [[a\nb] c.\n"), "<p>A [[a\nb] c.\n</p>\n");
        assert_eq!(links_joined("no link\nhere"), None);
        assert_eq!(links_joined("[[a]]\n[[b]]"), None);
        assert_eq!(links_joined("é[[\n]]"), Some("é[[\u{b}]]".into()));
        // And read back: a line's end and the blanks around it one space, the rest as written.
        assert_eq!(unwrapped("*A \u{b}   wrapped  heading"), "*A wrapped  heading");
        assert_eq!(unwrapped("a\u{b}\u{b}b"), "a b");
        assert_eq!(unwrapped("one  two"), "one  two");
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

    /// The `href`s of `org`'s links, in order, as written in its HTML.
    fn hrefs(org: &str) -> Vec<String> {
        let html = body(org);
        html.split("<a href=\"").skip(1).map(|rest| rest[..rest.find('"').unwrap()].to_owned()).collect()
    }

    /// Every link form Org has, each where Org has it lead: a place in the document, resolved here,
    /// as one `href`; a file and the place in it; an abbreviation expanded; anything with a type as
    /// it is, for the viewer to follow or say what it is.
    #[test]
    fn links_lead_where_org_has_them() {
        let org = concat!(
            "#+LINK: gh https://github.com/%s\n#+LINK: q https://q.org/?s=%h\n#+LINK: plain https://p.org/\n",
            "[[https://a.org/x][web]] <https://angle.org/y> https://bare.org/z. mailto:me@x.org =https://code.org= [[doi:10.1000/182]]\n",
            "[[file:other.org]] [[file+sys:other.org::*Some   Heading]] [[file+emacs:../up.org::#custom]] [[./here.org::a target]]\n",
            "[[/abs/x.org::42]] [[~/notes.org::/re/]] [[~someone/x.org]] [[file:my notes#1.org]]\n",
            "[[*Some Heading]] [[*another ONE]] [[#custom]] [[id:abc-123]] [[id:elsewhere]] [[a target]] [[named]] [[Another one]] [[nowhere]]\n",
            "[[gh:someone/repo]] [[q:a b]] [[plain:path]] [[elisp:(message \"x\")]] [[shell:ls]] [[help:org-mode]] [fn:1]\n",
            "#+NAME: named\n| a |\n",
            "* Some Heading [1/2] :tag:\n:PROPERTIES:\n:CUSTOM_ID: custom\n:ID: abc-123\n:END:\n",
            "* TODO Another one\nA <<a target>> here.\n\n[fn:1] A note.\n",
        );
        let expected = [
            "https://a.org/x",
            "https://angle.org/y",
            "https://bare.org/z",
            "mailto:me@x.org",
            "https://doi.org/10.1000/182",
            "other.org",
            "other.org#*Some%20Heading",
            "../up.org#custom",
            "./here.org#a%20target",
            "/abs/x.org",
            "/home/x/notes.org",
            "clv-org-not-followed/file:~someone/x.org",
            "my%20notes%231.org",
            "#custom",
            "#Another%20one",
            "#custom",
            "#abc-123",
            "#id:elsewhere",
            "#a%20target",
            "#named",
            "#Another%20one",
            "#nowhere",
            "https://github.com/someone/repo",
            "https://q.org/?s=a%20b",
            "https://p.org/path",
            "clv-org-not-followed/elisp:(message%20%22x%22)",
            "clv-org-not-followed/shell:ls",
            "clv-org-not-followed/help:org-mode",
            "#fn.1",
        ];
        let html = html_of(org, Some("/home/x")).unwrap();
        let start = html.find("<body>").unwrap();
        let found: Vec<String> = html[start..].split("<a href=\"").skip(1).map(|rest| rest[..rest.find('"').unwrap()].to_owned()).collect();
        assert_eq!(found, expected);
        assert!(html.contains("<code>https://code.org</code>"), "no link in code: {html}");
        assert!(html.contains("<a href=\"https://bare.org/z\">https://bare.org/z</a>."), "the full stop isn't the address's: {html}");
        assert_eq!(hrefs("[[~/x.org]]"), ["clv-org-not-followed/file:~/x.org"], "no home to lead to");
    }

    /// A link's description is never a link of its own, an address in it as it reads.
    #[test]
    fn an_address_in_a_links_description_is_its_text() {
        assert_eq!(hrefs("[[https://a.org][see https://b.org]]"), ["https://a.org"]);
    }

    /// The places a document has, each one `id` or more where it is: a heading by its custom id and
    /// its id, its title as Org matches it and GitHub's slug of what it shows; a target; a named
    /// element. A name the document gives itself wins over a heading's, in any case, and a heading
    /// left without either still gets an id a link can lead to.
    #[test]
    fn places_are_written_as_ids_the_documents_own_names_first() {
        let html = body(concat!(
            "* TODO [#A] Some =Code= Heading [2/3] :tag:\n:PROPERTIES:\n:CUSTOM_ID: custom\n:ID: abc\n:END:\n",
            "* Same\n* Same\n",
            "* Clash\nA <<clash>> target, <<*clash>> and <<<radio>>> one.\n",
            "#+NAME: Table\n| a |\n#+NAME: code\n#+begin_src sh\nls\n#+end_src\n",
            "* COMMENT Drafts\n",
        ));
        for (anchors, what) in [
            (
                "<h1><a id=\"custom\"></a><a id=\"abc\"></a><a id=\"Some =Code= Heading\"></a><a id=\"some-code-heading\"></a><a id=\"*Some =Code= Heading\"></a><a id=\"*Some Code Heading\"></a>",
                "a heading's: its title as written, its slug, and `*` before its title as written and as it shows",
            ),
            ("<h1><a id=\"Same\"></a><a id=\"*Same\"></a>Same</h1>", "the first Same, its slug the same name in another case"),
            ("<h1><a id=\"same-1\"></a>Same</h1>", "the second, by its slug"),
            ("<h1><a id=\"heading\"></a>Clash</h1>", "a heading whose every name a target has"),
            ("A <a id=\"clash\"></a> target, <a id=\"*clash\"></a> and <a id=\"radio\"></a>radio one.", "targets"),
            ("<table border=\"1\" id=\"Table\">", "a named table"),
            ("<pre id=\"code\"><code class=\"language-sh\">", "a named block"),
            ("<a id=\"Drafts\"></a><a id=\"comment-drafts\"></a>", "a COMMENT heading, by its title without it"),
        ] {
            assert!(html.contains(anchors), "{what}: {html}");
        }
        assert_eq!(
            html.matches(" id=\"").count(),
            html.match_indices(" id=\"").map(|(at, _)| &html[at..at + 20]).collect::<HashSet<_>>().len(),
            "no id twice: {html}"
        );
    }

    /// A link the viewer doesn't follow says what it is, its address as the document wrote it; any
    /// other is left as it is.
    #[test]
    fn a_link_clv_doesnt_follow_says_what_it_is() {
        let link = |path: &str| super::super::Link {
            page: 0,
            rects: Vec::new(),
            address: None,
            target: Target::File { path: path.into(), fragment: None, page: None },
        };
        let told = |path: &str| {
            let link = told(link(path));
            (link.target, link.address)
        };
        assert_eq!(told("clv-org-not-followed/elisp:(beep)"), (Target::Web("elisp:(beep)".into()), Some("elisp:(beep)".into())));
        assert_eq!(told("clv-org-not-followed/file:~someone/x.org").0, Target::Web("file:~someone/x.org".into()));
        assert_eq!(told("other.org").0, Target::File { path: "other.org".into(), fragment: None, page: None });
    }

    #[test]
    fn a_url_and_an_id_are_written_as_read_back() {
        assert_eq!(encoded("a b\"#%<>é\u{7}"), "a%20b%22%23%25%3C%3Eé%07");
        assert_eq!(hexified("a b/é"), "a%20b%2F%C3%A9");
        assert!(has_scheme("https://x") && has_scheme("elisp:(x)") && has_scheme("file+sys:x"));
        assert!(!has_scheme("no scheme") && !has_scheme(":x") && !has_scheme("1x:y") && !has_scheme("a b:c"));
    }

    #[test]
    fn an_abbreviation_too_long_once_expanded_is_left_as_it_is() {
        let tag = "x".repeat(4000);
        assert_eq!(hrefs(&format!("#+LINK: w https://w.org/%s%s%s\n[[w:{tag}]]\n")), [format!("clv-org-not-followed/w:{tag}")]);
    }

    /// A footnote's definition under what is hidden (a drawer, a comment) is no note: what it says
    /// isn't shown, nor searched, and a reference to it is as written.
    #[test]
    fn a_footnote_defined_where_it_is_hidden_is_no_note() {
        let html = body(
            "* H\n:LOGBOOK:\n[fn:private] Secret.\n:END:\n#+begin_comment\n[fn:other] Hidden.\n#+end_comment\nText[fn:private] and[fn:other].\n",
        );
        assert!(!html.contains("Secret") && !html.contains("Hidden"), "{html}");
        assert!(html.contains("Text[fn:private] and[fn:other]."), "{html}");
    }

    /// Only the TODO keywords Org reads as keywords are the file's: not a `#+TODO:` line in a
    /// block, which is the block's text.
    #[test]
    fn a_todo_line_in_a_block_declares_nothing() {
        for block in ["src org", "example", "comment"] {
            let end = block.split(' ').next().unwrap();
            let html = body(&format!("#+begin_{block}\n#+TODO: LATER | GONE\n#+end_{end}\n* TODO Still Org's own\n* LATER not one\n"));
            assert!(html.contains(&format!("<span style=\"color:{NOT_DONE}\">TODO</span> Still")), "{block}: {html}");
            assert!(!html.contains(">LATER</span>"), "{block}: {html}");
        }
    }

    /// A picture's path and a file's are written as URLs, so that a `#`, a `?` or a `%` in a name is
    /// read back as that name.
    #[test]
    fn a_pictures_path_and_a_files_are_written_as_urls() {
        let html = body("[[file:plot#1.png]] [[./a%20b.png]] [[https://x.org][file:thumb?.png]] [[file:a#b.org]]\n");
        for written in ["<img src=\"plot%231.png\"", "<img src=\"./a%2520b.png\"", "<img src=\"thumb%3F.png\"", "<a href=\"a%23b.org\">"] {
            assert!(html.contains(written), "{written} in {html}");
        }
    }

    /// A path's own spaces are kept; only a line's end and the blanks around it, where a link
    /// wraps, are one space.
    #[test]
    fn a_links_spaces_are_kept_but_where_it_wraps() {
        assert_eq!(hrefs("[[file:one  two.org]] [[file:a\n    b.org]]\n"), ["one%20%20two.org", "a%20b.org"]);
        let html = body("[[*Some\n   heading][x]] [[file:one  two.org]]\n* Some heading\n");
        assert!(html.contains("<a href=\"#Some%20heading\">x</a>") && html.contains(">file:one  two.org</a>"), "{html}");
    }

    /// A note defined inline after a reference to it by its name is that reference's note too: one
    /// number, one note.
    #[test]
    fn a_note_defined_inline_after_its_first_reference_is_its_note() {
        let html = body("First[fn:n]. Later[fn:n: The note.] and again[fn:n].\n");
        assert_eq!(html.matches("<sup><a href=\"#fn.1\">1</a></sup>").count(), 3, "{html}");
        assert!(html.contains("<p id=\"fn.1\" style=\"font-size:9.5pt\"><sup>1</sup>  The note.</p>") && !html.contains("[fn:n]"), "{html}");
    }

    /// Every place the document has is written as an `id`, whatever element it is: no name a link
    /// leads to that the document doesn't have.
    #[test]
    fn every_place_is_written_where_it_is() {
        let org = concat!(
            "* Heading\n:PROPERTIES:\n:CUSTOM_ID: custom\n:ID: an-id\n:END:\nA <<target>> and <<<radio>>>.\n",
            "#+NAME: para\nA paragraph.\n#+NAME: verse\n#+begin_verse\nA verse.\n#+end_verse\n#+NAME: rule\n-----\n",
            "#+NAME: table\n| a |\n#+NAME: list\n- an item\n#+NAME: src\n#+begin_src sh\nls\n#+end_src\n",
            "#+NAME: example\n#+begin_example\nx\n#+end_example\n#+NAME: fixed\n: fixed\n#+NAME: quote\n#+begin_quote\nq\n#+end_quote\n",
            "#+NAME: centre\n#+begin_center\nc\n#+end_center\n#+NAME: special\n#+begin_note\nn\n#+end_note\n",
            "#+NAME: latex\n\\begin{equation}\nx\n\\end{equation}\n",
            "| a <<in a cell>> |\n- an <<in an item>> item\n",
            "#+NAME: html\n#+begin_export html\n<b>raw</b>\n#+end_export\n#+NAME: table.el\n+---+\n| a |\n+---+\n",
            ":LOGBOOK:\n<<in a drawer>>\n:END:\n",
        );
        let config = orgize::ParseConfig { use_sub_superscript: orgize::config::UseSubSuperscript::Nil, ..Default::default() };
        let root = fixed(config.clone().parse(org).document().syntax(), &config);
        let places = places(&root);
        let html = html(org);
        let ids: Vec<&String> = places.at.values().flatten().collect();
        assert_eq!(ids.len(), 22, "every place named here: {ids:?}");
        for id in ids {
            assert!(html.contains(&format!("id=\"{}\"", escaped(id))), "{id} isn't written: {html}");
        }
        assert!(!places.targets.contains_key("in a drawer"), "a target hidden is no place");
    }

    /// A heading's slug is made from its title as it shows: an entity the character it names, a link
    /// with no description its address; and `[[*…]]` finds it by what it shows too.
    #[test]
    fn a_headings_slug_is_made_from_what_it_shows() {
        let html = body("[[*Greek α]]\n* Greek \\alpha\n* See [[https://x.org/a]]\n* A =code= [[*x][link]] \\to there\n");
        for id in ["greek-α", "see-httpsxorga", "a-code-link--there"] {
            assert!(html.contains(&format!("<a id=\"{id}\"></a>")), "{id} in {html}");
        }
        assert!(html.contains("<a href=\"#Greek%20%5Calpha\">"), "found by what it shows: {html}");
    }

    /// An abbreviation's template is expanded once: a `%h` in the tag a `%s` puts in it stays.
    #[test]
    fn an_abbreviation_is_expanded_once() {
        assert_eq!(hrefs("#+LINK: w https://w.org/?q=%s&h=%h\n[[w:%h%s]]\n"), ["https://w.org/?q=%h%s&amp;h=%25h%25s"]);
        assert_eq!(hrefs("#+LINK: w https://w.org/%x/%s\n[[w:t]]\n"), ["https://w.org/%x/t"]);
    }

    /// A file held in another is read again only to a byte past the documents' limit, where its
    /// holder can stop early, never whole.
    #[test]
    fn a_held_file_is_read_again_to_the_limit_and_no_further() {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let again: Again = Arc::new(|| panic!("read whole"));
        let bounded: Bounded = {
            let asked = asked.clone();
            Arc::new(move |most| {
                asked.lock().unwrap().push(most);
                Ok(None)
            })
        };
        let kept = Kept::Held(again, Some(bounded));
        assert_eq!(kept.bytes().unwrap_err(), "the document is more than 64 MiB");
        assert_eq!(*asked.lock().unwrap(), [MAX_SOURCE as u64]);
    }

    /// A bare address keeps its own balanced parentheses, and leaves those and the punctuation of
    /// the prose around it.
    #[test]
    fn a_bare_address_keeps_its_parentheses() {
        assert_eq!(
            hrefs("See https://en.wikipedia.org/wiki/Org-mode_(disambiguation). (Or https://x.org/a.) And https://y.org/(a(b)c)d), too.\n"),
            ["https://en.wikipedia.org/wiki/Org-mode_(disambiguation)", "https://x.org/a", "https://y.org/(a(b)c)d"]
        );
    }

    /// A named paragraph's link that wraps a line is a link, as any paragraph's.
    #[test]
    fn a_named_paragraphs_link_that_wraps_is_a_link() {
        let html = body("#+NAME: p\nSee [[*A\n   heading][there]].\n* A heading\n");
        assert!(html.contains("<p id=\"p\">See <a href=\"#A%20heading\">there</a>."), "{html}");
    }

    /// A file's relative path whose first name holds a colon is written after `./`, so that no
    /// reader of the `href` takes it for a URI's scheme; a picture's too.
    #[test]
    fn a_relative_path_with_a_colon_is_no_scheme() {
        assert_eq!(
            hrefs("[[file:notes:2026.org]] [[file:notes:2026.org::*X]] [[file:/a:b.org]] [[file:dir/a:b.org]]\n"),
            ["./notes:2026.org", "./notes:2026.org#*X", "/a:b.org", "dir/a:b.org"]
        );
        assert!(body("[[file:plot:1.png]]\n").contains("<img src=\"./plot:1.png\""));
    }

    /// A heading whose custom id and id are the same name is found by either.
    #[test]
    fn a_custom_id_and_an_id_alike_both_lead_to_their_heading() {
        for properties in [":CUSTOM_ID: same\n:ID: same\n", ":ID: same\n:CUSTOM_ID: same\n"] {
            let html = body(&format!("[[#same]] [[id:same]]\n* H\n:PROPERTIES:\n{properties}:END:\n"));
            assert_eq!(hrefs(&format!("[[#same]] [[id:same]]\n* H\n:PROPERTIES:\n{properties}:END:\n")), ["#same", "#same"], "{html}");
            assert_eq!(html.matches("<a id=\"same\"></a>").count(), 1, "{html}");
        }
    }

    /// Each round of mending builds the tree again once, however many paragraphs it mends: a document
    /// of many paragraphs with links that wrap costs a pass, not a pass a paragraph.
    #[test]
    fn the_tree_is_built_again_once_a_round() {
        let org = "See [[*A\nheading]].\n\n".repeat(3000);
        let config = orgize::ParseConfig { use_sub_superscript: orgize::config::UseSubSuperscript::Nil, ..Default::default() };
        let (root, built) = fixed_counted(config.clone().parse(&org).document().syntax(), &config);
        assert_eq!(built, 1);
        assert_eq!(root.descendants().filter(|node| node.kind() == SyntaxKind::LINK).count(), 3000);
        // Named ones, a round each for what follows a name: still once a round.
        let org = "#+NAME: a\n#+NAME: b\n| x |\n\n".repeat(300);
        let (root, built) = fixed_counted(config.clone().parse(&org).document().syntax(), &config);
        assert_eq!(built, 1);
        assert_eq!(root.descendants().filter(|node| node.kind() == SyntaxKind::ORG_TABLE).count(), 300);
    }

    /// `[[*Heading]]` goes to the heading of that title, in this file and in another
    /// (`file:x.org::*Heading`, by the `*Heading` every heading is also), never to a target of the
    /// same name, which `[[target]]` goes to.
    #[test]
    fn a_heading_search_finds_the_heading_never_a_target_of_its_name() {
        let html = body("[[*clash]] [[clash]] [[file:other.org::*clash]] [[file:other.org::clash]]\nA <<clash>>.\n* clash\n");
        assert_eq!(
            hrefs("[[*clash]] [[clash]] [[file:other.org::*clash]] [[file:other.org::clash]]\nA <<clash>>.\n* clash\n"),
            ["#*clash", "#clash", "other.org#*clash", "other.org#clash"]
        );
        assert!(html.contains("<h1><a id=\"*clash\"></a>clash</h1>"), "{html}");
    }

    /// An abbreviation is expanded before a link is told a picture: `[[pic:one]]` is the picture
    /// `pic` stands for, as in Org; so is a description written so.
    #[test]
    fn an_abbreviated_link_to_a_picture_is_the_picture() {
        let html = body("#+LINK: pic file:assets/%s.png\n[[pic:one]] [[https://x.org][pic:two]] [[pic:three][a description]]\n");
        for shown in [
            "<img src=\"assets/one.png\" alt=\"\">",
            "<a href=\"https://x.org\"><img src=\"assets/two.png\" alt=\"\"></a>",
            "<a href=\"assets/three.png\">a description</a>",
        ] {
            assert!(html.contains(shown), "{shown} in {html}");
        }
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
