//! Ebooks (epub) as books: the chapters of a book joined, in its reading order, into one HTML
//! document, each on a page of its own, and shown by whichever backend opens that HTML, the
//! documents' backend, never named here: this one lays nothing out itself. So a book has what a
//! document has: a page the window's size, its settings, a search across its chapters, its links.
//! The one file that names rbook
//! ([ADR-0038](../../docs/adr/0038-an-epub-as-one-document-its-package-read-by-rbook-in-a-worker.md)).
//!
//! A book's package (its OPF: the manifest, the spine, the cover) is untrusted, and rbook, which
//! reads it, can panic, recurse without bound and read without a cap on a hostile one. So rbook
//! works in a worker process of clv's own ([`PACKAGE`]), capped on CPU time and heap, which is
//! given only the package's files, read from the book within their limits, never the book; one
//! worker a book, gone once it has answered. What it answers, the reading order ([`Plan`]), is
//! checked and kept with the book: assembling the book again, for a render or a relayout, asks the
//! worker nothing.
//!
//! The book is read with the zip crate ([`Zipped`]) and assembled by a pass of clv's own over each
//! chapter ([`assembled`]), cut into tags as the documents' importer cuts it: ids made the book's
//! own, pictures' and links' paths resolved from the book's top, stylesheets dropped. It is handed
//! on as a file held here, whose pictures are read from the book ([`Near`]).

use std::fmt::Write as _;
use std::io::{Cursor, Write as _};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use super::head::{ZIP, mimetype};
use super::markup::{Piece, SKIPPED, StartTag, past_end_tag};
use super::worker::{self, Died, FAILED, Link as Worked, Session, WALL, Worker};
use super::zipped::Zipped;
use super::{
    Again, Anchor, Ask, Auto, BACKENDS, Backend, Bounded, Bounds, Deferred, Effect, Format, Frame, HEAD, Input, Job, Keys, Links, Named, Near,
    Numbers, Opened, Pool, Render, Reopenable, Request, Room, Setting, Source, Start, Target, Texts, Told, Value, Values,
};
use crate::args::{OptionSpec, Parsed};

/// Ebooks, their packages read by rbook.
pub(super) struct Epub;

/// What it shows: a zip that says it is an epub, or is named as one ([`opens`]); a book has the
/// settings a document laid out for the window has ([`SETTINGS`]).
const FORMAT: Format = Format::of("EPUB", &["epub"]).having(SETTINGS);

/// What a zip's first file, `mimetype`, says of an epub (EPUB 3.3's OCF, §4.3).
const MIMETYPE: &[u8] = b"application/epub+zip";

/// A book's paper: as the documents' backend declares its own, under the same name, values, key and
/// option, so that it is one setting with theirs, which the viewer holds one value of
/// ([ADR-0033](../../docs/adr/0033-backends-share-a-setting-by-declaring-its-name.md)). The book is
/// laid out and drawn by the backend that opens its HTML, which reads the values itself: these say
/// only that a book has them, before it is opened.
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
        help: "a book light on dark or dark on light (auto: as the terminal is)",
        alias: None,
    },
    help: "light on dark, or dark on light",
    does: "go dark",
    effect: Effect::Redraw,
    sets_width: false,
};

/// How wide a book's line is: as [`PAPER`], the documents' own setting.
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
        help: "a book laid out to lines of N characters of code (20-200), or as wide as the window",
        alias: None,
    },
    help: "4 characters a line narrower, or wider; 72] sets 72, 0] the window's",
    does: "reflow",
    effect: Effect::Relayout,
    sets_width: true,
};

/// How large a book's text is: as [`PAPER`], the documents' own setting.
const FONT: Setting = Setting {
    name: "font",
    named: &[],
    numbers: Some(Numbers { least: 50, most: 400, step: 10, unit: "%" }),
    default: Value::Number(100),
    zero: Value::Number(100),
    auto: None,
    keys: Keys::Zoom,
    option: OptionSpec { name: "--font", short: None, value: Some("N"), help: "a book's text at N% of its size (50-400)", alias: None },
    help: "text larger, or smaller; 150+ sets 150%, 0+ 100%",
    does: "have a font size",
    effect: Effect::Relayout,
    sets_width: false,
};

/// How wide a book's side margins are: as [`PAPER`], the documents' own setting.
const MARGIN: Setting = Setting {
    name: "margin",
    named: &[],
    numbers: Some(Numbers { least: 0, most: 20, step: 1, unit: "" }),
    default: Value::Number(3),
    zero: Value::Number(3),
    auto: None,
    keys: Keys::Step('{', '}'),
    option: OptionSpec { name: "--margin", short: None, value: Some("N"), help: "a book's side margins of N characters of code (0-20)", alias: None },
    help: "margins a character narrower, or wider; 5} sets 5, 0} back to 3",
    does: "have margins",
    effect: Effect::Relayout,
    sets_width: false,
};

/// Whether a book's fenced blocks are drawn: as [`PAPER`], the documents' own setting.
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
        help: "a book's fenced blocks drawn as pictures, or shown as code",
        alias: None,
    },
    help: "fenced blocks drawn, or as code",
    does: "draw their blocks",
    effect: Effect::Relayout,
    sets_width: false,
};

/// The settings a book has: a document's, laid out for the window.
const SETTINGS: &[&str] = &[PAPER.name, WIDTH.name, FONT.name, MARGIN.name, FENCES.name];

impl Backend for Epub {
    fn name(&self) -> &'static str {
        "epub"
    }

    fn shows(&self) -> &'static str {
        "ebooks"
    }

    fn formats(&self) -> Vec<Format> {
        vec![FORMAT]
    }

    /// rbook reads the package, and the zip crate the book.
    fn credits(&self) -> &[&'static str] {
        &["rbook", "zip"]
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

    /// It reads the package in a worker and lays the whole book out to open it: a list's tile opens
    /// it when wanted.
    fn deferred(&self, _path: &str, _head: &[u8]) -> Option<Deferred> {
        Some(Deferred { settings: SETTINGS })
    }

    /// Laid out for a room of the documents' backend's own: the viewer gives one
    /// ([`Backend::open_at`]).
    fn open(&self, input: Input, options: &Parsed<'_>) -> Result<Opened, String> {
        opened(input, options, None, package)
    }

    fn open_at(&self, input: Input, options: &Parsed<'_>, room: Room) -> Result<Opened, String> {
        opened(input, options, Some(room), package)
    }
}

/// Whether the file at `path`, which starts with `head`, is a book: a zip whose first file is
/// `mimetype`, stored as it is, saying it is an epub, whatever its name; or a zip named `.epub`
/// whose first bytes don't say what it is (a `mimetype` that isn't first, or is compressed, as some
/// tools write it).
fn opens(path: &str, head: &[u8]) -> bool {
    if !head.starts_with(ZIP) {
        return false;
    }
    // The first file's compression, in its local header: 0 is stored.
    let stored = head.get(8..10) == Some(&[0, 0]);
    match mimetype(head).filter(|_| stored) {
        Some(says) => says.trim_ascii() == MIMETYPE,
        None => FORMAT.named(path),
    }
}

/// The largest book kept whole, come through a pipe, or read whole from the file holding it each
/// time it is read: 256 MiB.
const MAX_BOOK: usize = 256 << 20;

/// The largest `META-INF/container.xml` read: it holds one path.
const MAX_CONTAINER: u64 = 64 << 10;

/// The largest package (OPF) read, and given to rbook.
const MAX_OPF: u64 = 4 << 20;

/// The most chapters a book's reading order has, and the longest path of a file in it, in bytes:
/// past either, the book is refused.
const MAX_CHAPTERS: usize = 4096;
const MAX_PATH: usize = 4 << 10;

/// The largest chapter read: one larger is left out.
const MAX_CHAPTER: u64 = 16 << 20;

/// What the book assembled may come to, below what the documents' backend refuses a file past (64
/// MiB, half a million lines and marks, 8 MiB of text,
/// [ADR-0012](../../docs/adr/0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md)):
/// its bytes, its marks (each `<` and line's end, as that backend counts them) and its text (the
/// bytes between its tags, which is no less than the text it is read as). A chapter that would pass
/// one is left out with those after it, so that a long book shows its start rather than nothing.
const MAX_WRITTEN: usize = 48 << 20;
const MAX_MARKS: usize = 400_000;
const MAX_TEXT: usize = 6 << 20;

/// The longest reason given for refusing a book, in bytes: cut there, at a character.
const MAX_WHY: usize = 256;

/// Why a book whose package's reading killed the worker, or made rbook panic, is refused.
const UNREAD: &str = "its package couldn't be read";

/// Where a book's bytes are.
#[derive(Clone)]
enum Kept {
    /// In a file on disk, read from it each time, or as they came through a pipe: its central
    /// directory, parsed once.
    Open(Arc<Zipped>),
    /// In another file (an archive), read whole from it each time the book is read.
    Held(Again),
}

impl Kept {
    /// The book as a zip to read from.
    fn zipped(&self) -> Result<Arc<Zipped>, String> {
        match self {
            Self::Open(zipped) => Ok(zipped.clone()),
            Self::Held(again) => again().and_then(within).and_then(Zipped::in_memory).map(Arc::new),
        }
    }
}

/// `bytes`, a book's, as long as they are no more than [`MAX_BOOK`].
fn within(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    match bytes.len() > MAX_BOOK {
        true => Err(format!("the book is more than {} MiB", MAX_BOOK >> 20)),
        false => Ok(bytes),
    }
}

/// How a book's package is read: [`package`], but in the tests.
type Read = fn(u8, Vec<u8>) -> Result<(u8, Vec<u8>), Died>;

/// The book `input` is, given `options`, laid out for `room` if one is given: its package read
/// through `read`, its chapters assembled, and the document they make opened by the backend that
/// shows it, found as any file's backend is, by how it starts, never by name.
fn opened(input: Input, options: &Parsed<'_>, room: Option<Room>, read: Read) -> Result<Opened, String> {
    let path = input.path.clone();
    let error = |e: String| format!("{path}: {e}");
    let kept = match input.reopenable()? {
        Reopenable::Disk(file) => Kept::Open(Arc::new(Zipped::on_disk(&path, file).map_err(error)?)),
        Reopenable::Bytes(bytes) => Kept::Open(Arc::new(within(bytes).and_then(Zipped::in_memory).map_err(error)?)),
        Reopenable::Again(again) => Kept::Held(again),
    };
    let zipped = kept.zipped().map_err(error)?;
    let plan = Arc::new(planned(&zipped, read).map_err(error)?);
    let (html, left) = assembled(&plan, &zipped);
    drop(zipped);
    let html = html.into_bytes();
    let head = html.get(..HEAD as usize).unwrap_or(&html).to_vec();
    let again: Again = {
        let (kept, plan) = (kept.clone(), plan.clone());
        Arc::new(move || Ok(assembled(&plan, &*kept.zipped()?).0.into_bytes()))
    };
    // The book's document is at its top: its pictures' paths are written from there.
    let document = Input::held(path.clone(), head, again).held_near(near(&kept, &path), String::new());
    // Not this one: what is assembled is never a book.
    let Some(backend) = BACKENDS.iter().filter(|b| b.name() != Epub.name()).find(|b| b.opens(&document.path, &document.head)) else {
        return Err(error("a book is shown as the HTML its chapters make, and no backend opens that".into()));
    };
    let shown = match room {
        Some(room) => backend.open_at(document, options, room)?,
        None => backend.open(document, options)?,
    };
    match shown {
        Opened::Pages(pages) => Ok(Opened::Pages(Box::new(Book { pages, notes: left.notes().into() }))),
        Opened::Files(_) => Err(error("its chapters' HTML holds files".into())),
    }
}

/// What is beside the book's document ([`Near`]): the file at a path from the book's top, by its
/// very name in the zip, read from the book each time, as a held file of its own.
fn near(kept: &Kept, book: &str) -> Near {
    let (kept, book) = (kept.clone(), book.to_owned());
    Arc::new(move |path| {
        let named = format!("{book}/{path}");
        let error = |e: String| format!("{named}: {e}");
        let zipped = kept.zipped().map_err(error)?;
        let index = zipped.index_of(path).ok_or_else(|| error("not in the book".into()))?;
        let head = zipped.first(index, HEAD).map_err(error)?;
        // Each reads the book again from where it is kept: on disk, by the directory kept.
        let from = |kept: &Kept| kept.zipped();
        let (whole, bound, first) = (kept.clone(), kept.clone(), kept.clone());
        let again: Again = Arc::new(move || from(&whole)?.whole(index, super::zipped::MAX_BYTES));
        let bounded: Bounded = Arc::new(move |most| from(&bound)?.within(index, most));
        let start: Start = Arc::new(move |most| from(&first)?.first(index, most));
        Ok(Input::held_bounded(named, head, again, bounded).starting(start))
    })
}

/// A book's reading order, as its package gives it: what the worker answered, checked.
#[derive(Debug, PartialEq, Eq)]
struct Plan {
    /// The spine's items, in its order.
    chapters: Vec<Chapter>,
}

/// An item of a book's spine.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Chapter {
    /// Its file's path from the book's top, as the zip names it; `None` for an item the manifest
    /// doesn't have, or whose path leads above the top.
    path: Option<String>,
    /// What its file is, by the manifest's media type.
    kind: Kind,
}

/// What a chapter's file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// XHTML, a chapter to read.
    Xhtml,
    /// Anything else.
    Other,
}

impl Kind {
    /// By a manifest item's media type: XHTML, as EPUB 3 names it, and as EPUB 2's OEBPS could.
    fn of(media_type: &[u8]) -> Self {
        match [&b"application/xhtml+xml"[..], b"text/html", b"text/x-oeb1-document"]
            .iter()
            .any(|xhtml| media_type.trim_ascii().eq_ignore_ascii_case(xhtml))
        {
            true => Self::Xhtml,
            false => Self::Other,
        }
    }
}

/// The reading order of the book `zipped` holds: its package's files read within their limits and
/// given to the worker through `read`, and what it answers checked.
fn planned(zipped: &Zipped, read: Read) -> Result<Plan, String> {
    let file = |name: &str, most: u64, what: &str| -> Result<Vec<u8>, String> {
        let index = zipped.index_of(name).ok_or_else(|| format!("it has no {what} ({name})"))?;
        zipped.within(index, most).map_err(|e| format!("{name}: {e}"))?.ok_or_else(|| format!("its {what} is more than {} KiB", most >> 10))
    };
    let container = file("META-INF/container.xml", MAX_CONTAINER, "container")?;
    let opf_path = package_path(&String::from_utf8_lossy(&container)).ok_or("its container names no package")?;
    if opf_path.len() > MAX_PATH {
        return Err("its package's path is too long".into());
    }
    let opf = file(&opf_path, MAX_OPF, "package")?;
    let mut request = Vec::new();
    put(&mut request, opf_path.as_bytes());
    put(&mut request, &opf);
    match read(READ, request) {
        Ok((PLAN, answer)) => Plan::read(&answer).ok_or_else(|| UNREAD.to_owned()),
        Ok((REFUSED, why)) => Err(cut(&String::from_utf8_lossy(&why), MAX_WHY).to_owned()),
        Ok(_) => Err(UNREAD.into()),
        Err(Died::TooMany) => Err("too many files are being read at once to read its package".into()),
        Err(Died::Unstarted(why)) => Err(format!("the process that reads its package couldn't be started ({})", cut(&why, MAX_WHY))),
        Err(_) => Err(UNREAD.into()),
    }
}

/// The path of the package (OPF) the container `xml` names first: the `full-path` of its first
/// `rootfile` that says it is a package, or says nothing of what it is. A look of clv's own, for one
/// attribute of a small file.
fn package_path(xml: &str) -> Option<String> {
    let mut at = 0;
    while let Some((piece, next)) = Piece::at(xml, at) {
        at = next;
        let Piece::Start(tag) = piece else { continue };
        if !tag.name.rsplit(':').next().is_some_and(|name| name.eq_ignore_ascii_case("rootfile")) {
            continue;
        }
        let attribute =
            |name: &str| tag.attributes().find(|attribute| attribute.name.eq_ignore_ascii_case(name)).map(|attribute| unescaped(attribute.value));
        let package = attribute("media-type").is_none_or(|kind| kind.trim().eq_ignore_ascii_case("application/oebps-package+xml"));
        if let Some(path) = attribute("full-path").filter(|path| package && !path.is_empty()) {
            return from_the_top("", &path);
        }
    }
    None
}

/// `text` with XML's five named entities, and its numbered ones, decoded; any other left as written.
fn unescaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(rest.get(..at).unwrap_or_default());
        rest = rest.get(at..).unwrap_or_default();
        let entity = rest.find(';').filter(|end| *end <= 10).and_then(|end| Some((rest.get(1..end)?, end)));
        let decoded = entity.and_then(|(name, end)| {
            let character = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => {
                    let number = name.strip_prefix('#')?;
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => number.parse().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((character, end))
        });
        match decoded {
            Some((character, end)) => {
                out.push(character);
                rest = rest.get(end.saturating_add(1)..).unwrap_or_default();
            }
            None => {
                out.push('&');
                rest = rest.get(1..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    out
}

/// The path from the book's top of what `reference`, a path as a file in the directory `directory`
/// (from the top, with no `/` at its end) writes it, names: each `.` and empty name dropped, each
/// `..` taken with the name before it, and one starting with `/` taken from the top. `None` for one
/// that leads above the top, or names nothing.
fn from_the_top(directory: &str, reference: &str) -> Option<String> {
    let mut parts: Vec<&str> = match reference.starts_with('/') {
        true => Vec::new(),
        false => directory.split('/').filter(|part| !part.is_empty()).collect(),
    };
    for part in reference.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// One field more in `out`: its length, four bytes, little-endian, and its bytes. A field too long
/// to say is written empty.
fn put(out: &mut Vec<u8>, field: &[u8]) {
    let field = if u32::try_from(field.len()).is_ok() { field } else { &[] };
    out.extend_from_slice(&u32::try_from(field.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(field);
}

/// The fields of a request or an answer, one after another ([`put`]): none past what is there.
struct Fields<'a>(&'a [u8]);

impl<'a> Iterator for Fields<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        let (length, rest) = self.0.split_first_chunk::<4>()?;
        let length = usize::try_from(u32::from_le_bytes(*length)).ok()?;
        let (field, rest) = rest.split_at_checked(length)?;
        self.0 = rest;
        Some(field)
    }
}

impl Plan {
    /// The answer to a [`READ`]: how many chapters, then each chapter's path and its media type.
    fn written(chapters: &[(Option<String>, String)]) -> Vec<u8> {
        let mut out = Vec::new();
        put(&mut out, &u32::try_from(chapters.len()).unwrap_or(u32::MAX).to_le_bytes());
        for (path, media_type) in chapters {
            put(&mut out, path.as_deref().unwrap_or_default().as_bytes());
            put(&mut out, cut(media_type, MAX_MEDIA_TYPE).as_bytes());
        }
        out
    }

    /// What the worker answered a [`READ`] with, checked as any input is: within the limits, its
    /// paths text, brought to paths from the book's top. `None` for an answer that isn't one.
    fn read(answer: &[u8]) -> Option<Self> {
        let mut fields = Fields(answer);
        let count = usize::try_from(u32::from_le_bytes(*fields.next()?.first_chunk::<4>()?)).ok()?;
        if count > MAX_CHAPTERS {
            return None;
        }
        let mut chapters = Vec::new();
        for _ in 0..count {
            let (path, media_type) = (fields.next()?, fields.next()?);
            if path.len() > MAX_PATH {
                return None;
            }
            let path = std::str::from_utf8(path).ok().and_then(|path| from_the_top("", path));
            chapters.push(Chapter { path, kind: Kind::of(media_type) });
        }
        fields.next().is_none().then_some(Self { chapters })
    }
}

/// Why chapters were left out of a book assembled, and how many for each.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Left {
    /// Not in the book: the manifest names a file the zip doesn't hold, or the spine an item the
    /// manifest doesn't.
    missing: usize,
    /// Read with an error, or not text this reads: neither UTF-8 nor UTF-16 with its byte-order mark.
    unreadable: usize,
    /// More than [`MAX_CHAPTER`].
    too_large: usize,
    /// Not XHTML.
    other: usize,
    /// Past what a book assembled may come to ([`MAX_WRITTEN`], [`MAX_MARKS`], [`MAX_TEXT`]).
    past: usize,
}

impl Left {
    /// What the first page says of the chapters left out, a line for each reason.
    fn notes(&self) -> Vec<String> {
        let said = |count: usize, why: &str| match count {
            0 => None,
            1 => Some(format!("1 chapter is left out: {why}")),
            count => Some(format!("{count} chapters are left out: {why}")),
        };
        [
            said(self.missing, "not in the book"),
            said(self.unreadable, "unreadable, or not text"),
            said(self.too_large, &format!("more than {} MiB", MAX_CHAPTER >> 20)),
            said(self.other, &format!("not a kind of chapter {} shows", crate::NAME)),
            said(self.past, &format!("the book is longer than {} lays out", crate::NAME)),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

/// How the book assembled starts: [`HEAD`] bytes of clv's own, a doctype and a comment filling
/// them, so that nothing a book writes is among the first bytes a backend is chosen by (a tar's
/// header is looked for at byte 257).
fn start() -> String {
    const DOCTYPE: &str = "<!doctype html><!-- a book, its chapters joined by clv ";
    const END: &str = "-->";
    let filled = (HEAD as usize).saturating_sub(DOCTYPE.len()).saturating_sub(END.len());
    format!("{DOCTYPE}{}{END}", " ".repeat(filled))
}

/// The book `plan` orders, read from `zipped`, as one HTML document: each chapter's body in a
/// `<div>` that starts a page, in the spine's order; and how many chapters were left out, and why.
fn assembled(plan: &Plan, zipped: &Zipped) -> (String, Left) {
    let mut out = start();
    let (mut left, mut marks, mut text) = (Left::default(), 0usize, 0usize);
    for (number, chapter) in plan.chapters.iter().enumerate() {
        let Some(path) = chapter.path.as_deref() else {
            left.missing = left.missing.saturating_add(1);
            continue;
        };
        if chapter.kind != Kind::Xhtml {
            left.other = left.other.saturating_add(1);
            continue;
        }
        let Some(index) = zipped.index_of(path) else {
            left.missing = left.missing.saturating_add(1);
            continue;
        };
        let bytes = match zipped.within(index, MAX_CHAPTER) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                left.too_large = left.too_large.saturating_add(1);
                continue;
            }
            Err(_) => {
                left.unreadable = left.unreadable.saturating_add(1);
                continue;
            }
        };
        let Some(xhtml) = decoded(bytes) else {
            left.unreadable = left.unreadable.saturating_add(1);
            continue;
        };
        let written = Written { chapter: number.saturating_add(1), directory: path.rsplit_once('/').map_or("", |(directory, _)| directory), zipped };
        let (body, body_text) = written.body(&xhtml);
        let body_marks = body.bytes().filter(|b| matches!(b, b'<' | b'\r' | b'\n')).count();
        let (marks_now, text_now) = (marks.saturating_add(body_marks), text.saturating_add(body_text));
        if out.len().saturating_add(body.len()) > MAX_WRITTEN || marks_now > MAX_MARKS || text_now > MAX_TEXT {
            // With those after it: a book cut short reads to where it is cut.
            left.past = plan.chapters.len().saturating_sub(number);
            break;
        }
        (marks, text) = (marks_now, text_now);
        out.push_str(&body);
    }
    (out, left)
}

/// A chapter's text: UTF-8, or UTF-16 with its byte-order mark, without that mark. `None` for any
/// other.
fn decoded(bytes: Vec<u8>) -> Option<String> {
    let utf16 = |bytes: &[u8], unit: fn([u8; 2]) -> u16| {
        let units = bytes.as_chunks::<2>().0.iter().map(|pair| unit(*pair));
        bytes.len().is_multiple_of(2).then(|| char::decode_utf16(units).collect::<Result<String, _>>().ok()).flatten()
    };
    match bytes.as_slice() {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        [0xEF, 0xBB, 0xBF, rest @ ..] => std::str::from_utf8(rest).ok().map(str::to_owned),
        _ => String::from_utf8(bytes).ok(),
    }
}

/// The elements of a chapter left out with all they hold, beyond those the importer reads no text
/// of ([`SKIPPED`]): what is about the chapter, not of it.
const DROPPED: [&str; 2] = ["head", "title"];

/// The elements whose tags are left out, what they hold kept: a chapter's own frame, which the
/// book has one of, and what would load a stylesheet or say where paths start from.
const UNWRAPPED: [&str; 5] = ["html", "body", "link", "meta", "base"];

/// What marks a link's path, in the book assembled, as a file of the book that is no chapter, and
/// as a file the book doesn't hold: written by [`Written::linked`] for every link to a file, so
/// that [`Book`] answers each without reading the book, and none is ever looked for on disk.
const IN_THE_BOOK: &str = "clv-in-the-book/";
const NOT_IN_THE_BOOK: &str = "clv-not-in-the-book/";

/// What is written before a table that is the first thing a chapter shows: a paragraph of one
/// zero-width space, a point high. A page break on the chapter's `<div>` reaches the first
/// paragraph in it, and a table is none, nor is an empty paragraph one (ADR-0038).
const BEFORE_A_TABLE: &str = "<p style=\"font-size: 1pt; margin: 0\">&#8203;</p>";

/// The elements that show something without any text in them: a chapter that starts with one has
/// started.
const SHOWN: [&str; 5] = ["img", "hr", "svg", "table", "br"];

/// A chapter being written into the book.
struct Written<'a> {
    /// Its number in the reading order, from 1: what its ids start with.
    chapter: usize,
    /// The directory its file is in, from the book's top: what its paths start from.
    directory: &'a str,
    zipped: &'a Zipped,
}

impl Written<'_> {
    /// The chapter `xhtml` as its part of the book: what its `<body>` holds (all of it, where it has
    /// none), in a `<div>` that starts a page; and how many bytes of text that is.
    fn body(&self, xhtml: &str) -> (String, usize) {
        let mut out = format!("<div id=\"c{}\" style=\"page-break-before: always\">", self.chapter);
        let (mut text, mut at) = (0usize, 0usize);
        // Whether anything of the chapter shows yet: text, or an element that shows without any.
        let mut shown = false;
        // From its `<body>`, where it has one.
        let mut scan = 0;
        while let Some((piece, next)) = Piece::at(xhtml, scan) {
            scan = next;
            if matches!(&piece, Piece::Start(tag) if tag.is("body")) {
                at = next;
                break;
            }
        }
        while let Some((piece, next)) = Piece::at(xhtml, at) {
            at = next;
            match piece {
                // A `<` that starts no tag is written so that it starts none once the tags around
                // it are written again.
                Piece::Text(run) if run.starts_with('<') => {
                    out.push_str("&lt;");
                    out.push_str(run.get(1..).unwrap_or_default());
                    text = text.saturating_add(run.len());
                    shown = true;
                }
                Piece::Text(run) => {
                    out.push_str(run);
                    text = text.saturating_add(run.len());
                    shown |= !run.trim().is_empty();
                }
                Piece::Other => {}
                Piece::End(name) if name.eq_ignore_ascii_case("body") => break,
                Piece::End(name) if UNWRAPPED.iter().any(|unwrapped| name.eq_ignore_ascii_case(unwrapped)) => {}
                Piece::End(name) => {
                    let _ = write!(out, "</{name}>");
                }
                Piece::Start(tag) if SKIPPED.iter().chain(&DROPPED).any(|name| tag.is(name)) => {
                    if !tag.self_closed {
                        at = past_end_tag(xhtml, at, tag.name);
                    }
                }
                Piece::Start(tag) if UNWRAPPED.iter().any(|name| tag.is(name)) => {}
                Piece::Start(tag) => {
                    if !shown && tag.is("table") {
                        out.push_str(BEFORE_A_TABLE);
                    }
                    shown |= SHOWN.iter().any(|name| tag.is(name));
                    self.start_tag(&tag, &mut out);
                }
            }
        }
        out.push_str("</div>\n");
        (out, text)
    }

    /// The start tag `tag` written again: its ids the book's own, its paths from the book's top.
    fn start_tag(&self, tag: &StartTag<'_>, out: &mut String) {
        out.push('<');
        out.push_str(tag.name);
        for attribute in tag.attributes() {
            let name = attribute.name;
            let value = match name {
                name if name.eq_ignore_ascii_case("id") => format!("c{}-{}", self.chapter, attribute.value),
                name if name.eq_ignore_ascii_case("src") => self.pictured(attribute.value),
                name if name.eq_ignore_ascii_case("href") => self.linked(attribute.value),
                _ => attribute.value.to_owned(),
            };
            // As written, a value holds no quote of the kind around it; one written again may.
            let quote = if value.contains('"') { '\'' } else { '"' };
            let _ = write!(out, " {name}={quote}{}{quote}", value.replace('\'', if quote == '\'' { "&#39;" } else { "'" }));
        }
        out.push_str(if tag.self_closed { "/>" } else { ">" });
    }

    /// The path `reference` names, from this chapter, as a path from the book's top, and what
    /// follows it (its `?query` and `#fragment`): `None` for what is no path in the book, a web
    /// address or anything else with a scheme, which is left as written.
    fn resolved<'r>(&self, reference: &'r str) -> Option<(Option<String>, &'r str)> {
        let reference = reference.trim();
        let end = reference.find(['?', '#']).unwrap_or(reference.len());
        let (path, rest) = reference.split_at_checked(end)?;
        let scheme = path.split_once(':').is_some_and(|(before, _)| !before.contains('/'));
        if scheme || path.starts_with("//") {
            return None;
        }
        Some((percent_decoded(path).and_then(|path| from_the_top(self.directory, &path)), rest))
    }

    /// A picture's `src` as the book's document writes it: its path from the book's top, written
    /// as a document's pictures are read (their `%XX` undone), so that every character that would
    /// mean something there is escaped. One that leads above the book's top is left to say so.
    fn pictured(&self, src: &str) -> String {
        match self.resolved(src) {
            Some((Some(path), _)) => format!("./{}", percent_encoded(&path)),
            Some((None, _)) => "../above-the-book".to_owned(),
            None => src.to_owned(),
        }
    }

    /// A link's `href` as the book's document writes it: a web address as it is; a path, marked as
    /// a file of the book or as none ([`IN_THE_BOOK`], [`NOT_IN_THE_BOOK`]) and written from the
    /// book's top, so that it is never a path on disk.
    fn linked(&self, href: &str) -> String {
        match self.resolved(href) {
            None => href.to_owned(),
            // To a place in this chapter.
            Some((_, rest)) if href.trim().starts_with('#') => rest.to_owned(),
            Some((Some(path), _)) if self.zipped.index_of(&path).is_some() => format!("{IN_THE_BOOK}{}", percent_encoded(&path)),
            Some((path, _)) => format!("{NOT_IN_THE_BOOK}{}", percent_encoded(path.as_deref().unwrap_or_default())),
        }
    }
}

/// `text` with its `%XX` escapes undone, if that leaves text.
fn percent_decoded(text: &str) -> Option<String> {
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

/// `path` with what would mean something in a document's reference escaped as `%XX`: the `%` itself,
/// what ends a path there (`?`, `#`), what a tag is written with, and blank space.
fn percent_encoded(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for character in path.chars() {
        match character {
            '%' | '?' | '#' | '"' | '\'' | '<' | '>' | '&' | ':' | '\\' => {
                let _ = write!(out, "%{:02X}", u32::from(character));
            }
            character if character.is_whitespace() || character.is_control() => {
                let mut utf8 = [0; 4];
                for byte in character.encode_utf8(&mut utf8).bytes() {
                    let _ = write!(out, "%{byte:02X}");
                }
            }
            character => out.push(character),
        }
    }
    out
}

/// A book, open: its document, as the backend that opens it shows it, with what was left out of it
/// said with its first page; and its links answered so that none leads to a file.
struct Book {
    pages: Box<dyn Source + Send>,
    notes: Arc<[String]>,
}

/// Everything is the document's, as its backend shows it; but the first page's notes, and where
/// its links lead.
impl Source for Book {
    fn pages(&self) -> usize {
        self.pages.pages()
    }

    fn size(&self, page: usize) -> Option<(f32, f32)> {
        self.pages.size(page)
    }

    fn natural(&self, page: usize) -> bool {
        self.pages.natural(page)
    }

    fn settings(&self) -> Vec<&'static str> {
        self.pages.settings()
    }

    fn dark_form(&self) -> bool {
        self.pages.dark_form()
    }

    fn settle(&self, page: usize) {
        self.pages.settle(page);
    }

    fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
        Ok(Box::new(Noted { render: self.pages.renderer()?, notes: self.notes.clone() }))
    }

    fn pool(&self, threads: usize) -> Box<dyn Pool> {
        Box::new(NotedPool { pool: self.pages.pool(threads), notes: self.notes.clone() })
    }

    fn laid_at(&self) -> (Values, Option<Room>) {
        self.pages.laid_at()
    }

    /// The document laid out again is a book again.
    fn relayout(&self, values: Values, room: Room) -> Option<Job> {
        let (job, notes) = (self.pages.relayout(values, room)?, self.notes.clone());
        Some(Box::new(move || job().map(|pages| Box::new(Book { pages, notes }) as Box<dyn Source + Send>)))
    }

    fn largest_scale(&self, page: usize) -> Option<f32> {
        self.pages.largest_scale(page)
    }

    fn bounds(&self, setting: &str, room: Room, values: Values) -> Option<Bounds> {
        self.pages.bounds(setting, room, values)
    }

    fn anchor(&self, page: usize, y: f32) -> Option<Anchor> {
        self.pages.anchor(page, y)
    }

    fn find(&self, anchor: &Anchor) -> Option<(usize, f32)> {
        self.pages.find(anchor)
    }

    fn texts(&self) -> Option<Box<dyn Texts>> {
        self.pages.texts()
    }

    fn links(&self) -> Option<Box<dyn Links>> {
        Some(Box::new(InTheBook(self.pages.links()?)))
    }
}

struct Noted<'a> {
    render: Box<dyn Render + 'a>,
    notes: Arc<[String]>,
}

impl Render for Noted<'_> {
    fn render(&mut self, request: Request) -> Frame {
        noted(self.render.render(request), &self.notes)
    }
}

struct NotedPool {
    pool: Box<dyn Pool>,
    notes: Arc<[String]>,
}

impl Pool for NotedPool {
    fn set_queue(&self, requests: Vec<Request>) {
        self.pool.set_queue(requests);
    }

    fn try_recv(&self) -> Option<Frame> {
        self.pool.try_recv().map(|frame| noted(frame, &self.notes))
    }
}

/// `frame`, with `notes` first among its warnings if it is the first page.
fn noted(mut frame: Frame, notes: &[String]) -> Frame {
    if frame.request.page == 0 && frame.error.is_none() {
        frame.warnings.splice(0..0, notes.iter().cloned());
    }
    frame
}

/// A book's links, as its document's backend finds them, each that would lead to a file answered
/// here: a book's links are never looked for beside it on disk.
struct InTheBook(Box<dyn Links>);

impl Links for InTheBook {
    fn set_queue(&self, wanted: Vec<Ask>) {
        self.0.set_queue(wanted);
    }

    fn try_recv(&self) -> Option<Told> {
        Some(match self.0.try_recv()? {
            Told::Page { page, links, more } => {
                let links = links.map(|links| links.into_iter().map(in_the_book).collect());
                Told::Page { page, links, more }
            }
            told @ Told::Name { .. } => told,
        })
    }
}

/// `link`, of a book's document, with a target that is a file made a message, whichever file it
/// is, and its address as the chapter wrote the file's path, from the book's top.
fn in_the_book(mut link: super::Link) -> super::Link {
    let Target::File { path, .. } = &link.target else { return link };
    let (said, marker) = match path.starts_with(IN_THE_BOOK) {
        true => (format!("a file in the book, which {} doesn't open alone", crate::NAME), IN_THE_BOOK),
        false => ("not in this book".to_owned(), NOT_IN_THE_BOOK),
    };
    link.address = link.address.map(|address| match address.strip_prefix(marker) {
        Some(path) => path.to_owned(),
        None => address,
    });
    link.target = Target::Other(said);
    link
}

// The worker's side.

/// A book's package, to read: the request. Its path in the book, then its bytes ([`put`]).
const READ: u8 = 1;

/// Its reading order ([`Plan::written`]): an answer.
const PLAN: u8 = 2;

/// Why rbook refused it, in its words: an answer.
const REFUSED: u8 = 3;

/// Why a worker answered [`FAILED`]: what it was asked isn't a request.
const MALFORMED: u8 = 1;

/// The CPU seconds a package may take in the worker, and the most its heap holds: what the
/// costliest packages measured take, many times over (ADR-0038). The viewer waits [`WALL`] times
/// as long.
const READ_CPU: u64 = 2;
const MAX_HEAP: usize = 256 << 20;

/// The longest request: a package's path and the package ([`MAX_PATH`], [`MAX_OPF`]), each with
/// its length.
const MAX_REQUEST: usize = MAX_PATH + (MAX_OPF as usize) + 8;

/// The longest media type kept of a manifest's item, in bytes.
const MAX_MEDIA_TYPE: usize = 255;

/// The longest answer: every chapter's path and media type, each with its length, and their count.
const MAX_ANSWER: usize = 8 + MAX_CHAPTERS * (MAX_PATH + MAX_MEDIA_TYPE + 8);

/// The worker that reads a book's package: `clv --worker epub`.
pub(crate) struct Package;

pub(crate) static PACKAGE: Package = Package;

impl Worker for Package {
    fn name(&self) -> &'static str {
        "epub"
    }

    fn takes(&self, kind: u8) -> Option<(u64, usize)> {
        (kind == READ).then_some((READ_CPU, MAX_REQUEST))
    }

    fn heap(&self) -> usize {
        MAX_HEAP
    }

    fn session(&self) -> Box<dyn Session> {
        Box::new(Reading)
    }
}

/// A worker's answers: it keeps nothing between them.
struct Reading;

impl Session for Reading {
    fn answer(&mut self, kind: u8, payload: Vec<u8>) -> (u8, Vec<u8>) {
        if kind != READ {
            return (FAILED, vec![MALFORMED]);
        }
        let mut fields = Fields(&payload);
        let (Some(path), Some(opf), None) = (fields.next().and_then(|path| std::str::from_utf8(path).ok()), fields.next(), fields.next()) else {
            return (FAILED, vec![MALFORMED]);
        };
        let read = std::panic::catch_unwind(|| read(path, opf));
        match read.unwrap_or_else(|_| Err(UNREAD.into())) {
            Ok(chapters) => (PLAN, Plan::written(&chapters)),
            Err(why) => (REFUSED, cut(&why, MAX_WHY).as_bytes().to_vec()),
        }
    }
}

/// The reading order of the package `opf`, the file at `path` in its book, as rbook reads it: each
/// item of the spine, in order, as its file's path from the book's top and its media type; or why
/// not. rbook is given a zip holding the package alone, under a container that names it, and reads
/// neither the table of contents (`skip_toc`: one it can't read fails the whole package) nor any
/// other file.
fn read(path: &str, opf: &[u8]) -> Result<Vec<(Option<String>, String)>, String> {
    let escaped = path.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
    let container = format!(
        "<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile \
         full-path=\"{escaped}\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>"
    );
    let mut zip = ::zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = ::zip::write::SimpleFileOptions::default().compression_method(::zip::CompressionMethod::Stored);
    for (name, bytes) in [("mimetype", MIMETYPE), ("META-INF/container.xml", container.as_bytes()), (path, opf)] {
        zip.start_file(name, stored).map_err(|e| e.to_string())?;
        zip.write_all(bytes).map_err(|e| e.to_string())?;
    }
    let package = zip.finish().map_err(|e| e.to_string())?;
    let epub = rbook::Epub::options().strict(false).skip_toc(true).read(package).map_err(|e| e.to_string())?;
    let spine = epub.spine();
    if spine.len() > MAX_CHAPTERS {
        return Err(format!("its reading order has more than {MAX_CHAPTERS} chapters"));
    }
    let mut chapters = Vec::new();
    for entry in spine.iter() {
        let Some(item) = entry.manifest_entry() else {
            chapters.push((None, String::new()));
            continue;
        };
        let path = item.href().path().decode().into_owned();
        if path.len() > MAX_PATH {
            return Err("a chapter's path is too long".into());
        }
        chapters.push((Some(path), item.media_type().to_owned()));
    }
    if chapters.is_empty() {
        return Err("its package lists no chapters to read".into());
    }
    Ok(chapters)
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

/// One package read at a time: a list of books takes one worker's place among those alive, and a
/// book waits its turn here.
static TURN: Mutex<()> = Mutex::new(());

/// The worker's answer to a request of `kind`, its `payload`: a worker started for it, in its turn,
/// and gone once it has answered.
fn package(kind: u8, payload: Vec<u8>) -> Result<(u8, Vec<u8>), Died> {
    let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
    asked(|| worker::spawn(&PACKAGE), kind, payload)
}

/// [`package`], of a worker `spawn` starts.
fn asked(spawn: impl FnOnce() -> Result<Worked, Died>, kind: u8, payload: Vec<u8>) -> Result<(u8, Vec<u8>), Died> {
    let mut link = spawn()?;
    let wall = Duration::from_secs(READ_CPU.saturating_mul(u64::from(WALL)));
    link.request(kind, payload, wall, MAX_ANSWER)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// A book of `files`, by name and contents, deflated, `mimetype` first and stored.
    pub(super) fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = ::zip::ZipWriter::new(Cursor::new(Vec::new()));
        let stored = ::zip::write::SimpleFileOptions::default().compression_method(::zip::CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(MIMETYPE).unwrap();
        for (name, contents) in files {
            zip.start_file(*name, ::zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(contents).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn book(files: &[(&str, &[u8])]) -> Zipped {
        Zipped::in_memory(zip_of(files)).unwrap()
    }

    /// The container that names the package at `path`.
    fn container(path: &str) -> String {
        format!(
            r#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="{path}" media-type="application/oebps-package+xml"/></rootfiles></container>"#
        )
    }

    /// A package of `version`: its manifest's `items` (id, href, media type) and its spine's
    /// `itemrefs`, as written.
    fn opf(version: &str, items: &[(&str, &str, &str)], itemrefs: &str) -> String {
        let items: String = items.iter().map(|(id, href, kind)| format!(r#"<item id="{id}" href="{href}" media-type="{kind}"/>"#)).collect();
        format!(
            r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="{version}" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">x</dc:identifier><dc:title>A book</dc:title><dc:language>en</dc:language></metadata><manifest>{items}</manifest><spine>{itemrefs}</spine></package>"#
        )
    }

    const XHTML: &str = "application/xhtml+xml";

    /// A chapter whose body is `body`.
    fn chapter(body: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>A chapter</title><link rel=\"stylesheet\" href=\"style.css\"/><style>p {{ color: red }}</style></head><body class=\"calibre\">{body}</body></html>"
        )
    }

    /// The worker's code, on a thread of the tests, as [`package`] asks a worker process.
    fn in_a_thread(kind: u8, payload: Vec<u8>) -> Result<(u8, Vec<u8>), Died> {
        asked(|| worker::spawn(&PACKAGE), kind, payload)
    }

    fn paths(plan: &Plan) -> Vec<Option<&str>> {
        plan.chapters.iter().map(|chapter| chapter.path.as_deref()).collect()
    }

    /// What rbook answers of a package, EPUB 2 and EPUB 3: the spine's items in its order, a
    /// `linear="no"` one where it comes, each as its file's path from the book's top, whatever
    /// directory the package is in and however its hrefs are written (`..`, `%20`); an item the
    /// manifest doesn't have, and one that isn't XHTML, each in its place.
    #[test]
    fn the_worker_answers_a_packages_reading_order() {
        let items = [
            ("one", "text/one.xhtml", XHTML),
            ("two", "text/chapter%20two.xhtml", XHTML),
            ("three", "../outside/three.xhtml", XHTML),
            ("notes", "text/notes.xhtml", "text/html"),
            ("picture", "images/plate.jpg", "image/jpeg"),
            ("ncx", "toc.ncx", "application/x-dtbncx+xml"),
        ];
        let spine = r#"<itemref idref="one"/><itemref idref="notes" linear="no"/><itemref idref="two"/><itemref idref="gone"/><itemref idref="picture"/><itemref idref="three"/>"#;
        for version in ["2.0", "3.0"] {
            let package = opf(version, &items, spine);
            let book = book(&[("META-INF/container.xml", container("OEBPS/content.opf").as_bytes()), ("OEBPS/content.opf", package.as_bytes())]);
            let plan = planned(&book, in_a_thread).unwrap_or_else(|e| panic!("{version}: {e}"));
            assert_eq!(
                paths(&plan),
                [
                    Some("OEBPS/text/one.xhtml"),
                    Some("OEBPS/text/notes.xhtml"),
                    Some("OEBPS/text/chapter two.xhtml"),
                    None,
                    Some("OEBPS/images/plate.jpg"),
                    Some("outside/three.xhtml")
                ],
                "{version}"
            );
            let kinds: Vec<Kind> = plan.chapters.iter().map(|chapter| chapter.kind).collect();
            assert_eq!(kinds, [Kind::Xhtml, Kind::Xhtml, Kind::Xhtml, Kind::Other, Kind::Other, Kind::Xhtml], "{version}");
        }
        // A package at the book's top, named with a space, as the container writes it.
        let package = opf("3.0", &[("one", "one.xhtml", XHTML)], r#"<itemref idref="one"/>"#);
        let book = book(&[("META-INF/container.xml", container("my book.opf").as_bytes()), ("my book.opf", package.as_bytes())]);
        assert_eq!(paths(&planned(&book, in_a_thread).unwrap()), [Some("one.xhtml")]);
    }

    /// A book that can't be read as one is refused with why: without its container, a container
    /// that names no package or one the book doesn't hold, a package with no spine, one that isn't
    /// XML, and those past their limits, which the worker is never given.
    #[test]
    fn a_book_whose_package_cant_be_read_is_refused_with_why() {
        let never: Read = |_, _| panic!("the worker is asked");
        let refused = |files: &[(&str, &[u8])], read: Read| planned(&book(files), read).unwrap_err();
        assert_eq!(refused(&[("a.xhtml", b"x")], never), "it has no container (META-INF/container.xml)");
        let names_none = br#"<container><rootfiles><rootfile media-type="text/plain" full-path="a.txt"/></rootfiles></container>"#;
        assert_eq!(refused(&[("META-INF/container.xml", names_none)], never), "its container names no package");
        let container = container("content.opf");
        assert_eq!(refused(&[("META-INF/container.xml", container.as_bytes())], never), "it has no package (content.opf)");
        let large = vec![b' '; MAX_CONTAINER as usize + 1];
        assert_eq!(refused(&[("META-INF/container.xml", &large)], never), "its container is more than 64 KiB");
        let large = vec![b' '; MAX_OPF as usize + 1];
        assert_eq!(refused(&[("META-INF/container.xml", container.as_bytes()), ("content.opf", &large)], never), "its package is more than 4096 KiB");
        let with = |package: &str| refused(&[("META-INF/container.xml", container.as_bytes()), ("content.opf", package.as_bytes())], in_a_thread);
        assert_eq!(with(&opf("3.0", &[("one", "one.xhtml", XHTML)], "")), "its package lists no chapters to read");
        let no_spine = opf("3.0", &[], "").replace("<spine></spine>", "");
        assert!(!with(&no_spine).is_empty(), "no spine");
        let why = with("<package");
        assert!(!why.is_empty() && why.len() <= MAX_WHY, "{why}");
        let long = format!("<{}", "é".repeat(400));
        assert!(with(&long).len() <= MAX_WHY, "cut at a character");
        let many: String = (0..=MAX_CHAPTERS).map(|_| r#"<itemref idref="one"/>"#).collect();
        assert_eq!(with(&opf("3.0", &[("one", "one.xhtml", XHTML)], &many)), "its reading order has more than 4096 chapters");
    }

    /// A worker that dies, or answers what is no answer, costs the book, with one message; one
    /// that couldn't be started says so.
    #[test]
    fn a_worker_that_dies_or_answers_too_much_costs_the_book() {
        let package = opf("3.0", &[("one", "one.xhtml", XHTML)], r#"<itemref idref="one"/>"#);
        let container = container("content.opf");
        let book = book(&[("META-INF/container.xml", container.as_bytes()), ("content.opf", package.as_bytes())]);
        let deaths: [Read; 6] = [
            |_, _| Err(Died::Cpu),
            |_, _| Err(Died::Memory),
            |_, _| Err(Died::TimedOut),
            |_, _| Err(Died::Ended("signal 6".into())),
            |_, _| Err(Died::Malformed),
            |_, _| Ok((FAILED, vec![MALFORMED])),
        ];
        for death in deaths {
            assert_eq!(planned(&book, death).unwrap_err(), UNREAD);
        }
        let answers: [Read; 5] = [
            |_, _| Ok((PLAN, Vec::new())),
            |_, _| Ok((PLAN, vec![0xFF; 64])),
            // More chapters than a book has, said and unsaid; and a path past its limit.
            |_, _| Ok((PLAN, Plan::written(&vec![(Some("a".to_owned()), String::new()); MAX_CHAPTERS + 1]))),
            |_, _| {
                let mut answer = Plan::written(&[(Some("a".to_owned()), String::new())]);
                answer.extend_from_slice(&[0; 9]);
                Ok((PLAN, answer))
            },
            |_, _| Ok((PLAN, Plan::written(&[(Some("a".repeat(MAX_PATH + 1)), String::new())]))),
        ];
        for answer in answers {
            assert_eq!(planned(&book, answer).unwrap_err(), UNREAD);
        }
        assert_eq!(planned(&book, |_, _| Err(Died::TooMany)).unwrap_err(), "too many files are being read at once to read its package");
        assert!(planned(&book, |_, _| Err(Died::Unstarted("no such file".into()))).unwrap_err().ends_with("couldn't be started (no such file)"));
        assert_eq!(planned(&book, |_, _| Ok((REFUSED, "rbook's reason".into()))).unwrap_err(), "rbook's reason");
        // What the worker answers is checked, not trusted: a path above the top is no chapter.
        let above = planned(&book, |_, _| Ok((PLAN, Plan::written(&[(Some("../../etc/passwd".to_owned()), XHTML.to_owned())])))).unwrap();
        assert_eq!(paths(&above), [None]);
    }

    fn plan(chapters: &[(&str, Kind)]) -> Plan {
        Plan { chapters: chapters.iter().map(|(path, kind)| Chapter { path: (!path.is_empty()).then(|| (*path).to_owned()), kind: *kind }).collect() }
    }

    /// A book is its chapters' bodies, in the spine's order, each in a `<div>` that starts a page,
    /// after a kilobyte of clv's own; a chapter's head, its stylesheets and its scripts are left
    /// out, its ids made the book's own, and its pictures' paths written from the book's top.
    #[test]
    fn chapters_are_assembled_in_order_each_on_a_page() {
        let one = chapter(
            r#"<h1 id="start">One</h1><p style="text-indent: 0">first <img src="../images/a%20b.png" alt="a"/> <img src="https://example.com/x.png"/></p><style>h1 { color: red }</style><script>alert(1)</script>"#,
        );
        let two = chapter(r#"<h1 id="start">Two</h1><p>second, 1 < 2</p><!-- a comment -->"#);
        let fragment = "<p>no body of its own</p>";
        let book = book(&[
            ("OEBPS/text/one.xhtml", one.as_bytes()),
            ("OEBPS/text/two.xhtml", two.as_bytes()),
            ("OEBPS/three.xhtml", fragment.as_bytes()),
            ("OEBPS/images/a b.png", b"png"),
        ]);
        let plan = plan(&[("OEBPS/text/two.xhtml", Kind::Xhtml), ("OEBPS/text/one.xhtml", Kind::Xhtml), ("OEBPS/three.xhtml", Kind::Xhtml)]);
        let (html, left) = assembled(&plan, &book);
        assert_eq!(left, Left::default());
        let (head, chapters) = html.split_at(HEAD as usize);
        assert!(head.starts_with("<!doctype html><!--") && head.ends_with("-->") && !head[19..head.len() - 3].contains("--"), "{head}");
        assert_eq!(
            chapters,
            concat!(
                r#"<div id="c1" style="page-break-before: always"><h1 id="c1-start">Two</h1><p>second, 1 &lt; 2</p></div>"#,
                "\n",
                r#"<div id="c2" style="page-break-before: always"><h1 id="c2-start">One</h1><p style="text-indent: 0">first <img src="./OEBPS/images/a%20b.png" alt="a"/> <img src="https://example.com/x.png"/></p></div>"#,
                "\n",
                r#"<div id="c3" style="page-break-before: always"><p>no body of its own</p></div>"#,
                "\n",
            )
        );
        // The same again, whenever it is asked for.
        assert_eq!(assembled(&plan, &book).0, html);
        // A chapter that starts with a table starts its page with a paragraph, which takes the
        // break; a table after anything else needs none.
        let written = Written { chapter: 1, directory: "", zipped: &book };
        let starts = |body: &str| written.body(&chapter(body)).0.contains(BEFORE_A_TABLE);
        assert!(starts("<table><tr><td>a</td></tr></table>") && starts("\n <div><section>\n<table><tr><td>a</td></tr></table></section></div>"));
        assert!(!starts("<p>first</p><table></table>") && !starts("<div><img src='a.png'/></div><table></table>") && !starts("text<table></table>"));
        assert_eq!(written.body(&chapter("<table></table><table></table>")).0.matches(BEFORE_A_TABLE).count(), 1);
    }

    /// A chapter that is missing, unreadable, too large, or not XHTML is left out, and counted for
    /// the first page to say; the rest of the book is there.
    #[test]
    fn a_chapter_that_cant_be_read_is_left_out_and_counted() {
        let fine = chapter("<p>fine</p>");
        let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain("<p>sixteen</p>".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let big_endian: Vec<u8> = [0xFE, 0xFF].into_iter().chain("<p>big</p>".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        let large = vec![b' '; MAX_CHAPTER as usize + 1];
        let book = book(&[
            ("fine.xhtml", fine.as_bytes()),
            ("latin.xhtml", b"<p>caf\xE9</p>"),
            ("utf16.xhtml", &utf16),
            ("big.xhtml", &big_endian),
            ("odd.xhtml", &[0xFF, 0xFE, 0x41]),
            ("bom.xhtml", b"\xEF\xBB\xBF<p>marked</p>"),
            ("large.xhtml", &large),
            ("plate.jpg", b"jpeg"),
        ]);
        let order = [
            ("fine.xhtml", Kind::Xhtml),
            ("gone.xhtml", Kind::Xhtml),
            ("", Kind::Xhtml),
            ("latin.xhtml", Kind::Xhtml),
            ("utf16.xhtml", Kind::Xhtml),
            ("big.xhtml", Kind::Xhtml),
            ("odd.xhtml", Kind::Xhtml),
            ("bom.xhtml", Kind::Xhtml),
            ("large.xhtml", Kind::Xhtml),
            ("plate.jpg", Kind::Other),
        ];
        let (html, left) = assembled(&plan(&order), &book);
        assert_eq!(left, Left { missing: 2, unreadable: 2, too_large: 1, other: 1, past: 0 });
        let shown: Vec<&str> = ["fine", "sixteen", "big", "marked", "caf"].into_iter().filter(|text| html.contains(&format!("<p>{text}"))).collect();
        assert_eq!(shown, ["fine", "sixteen", "big", "marked"]);
        assert!(html.contains(r#"<div id="c8""#) && !html.contains(r#"<div id="c9""#), "numbered by their place in the reading order");
        assert_eq!(
            left.notes(),
            [
                "2 chapters are left out: not in the book",
                "2 chapters are left out: unreadable, or not text",
                "1 chapter is left out: more than 16 MiB",
                "1 chapter is left out: not a kind of chapter clv shows",
            ]
        );
    }

    /// A book longer than is laid out is cut at the chapter that would pass a bound, with those
    /// after it, however short they are: by its marks, by its text and by its bytes.
    #[test]
    fn a_book_is_cut_at_the_chapter_that_would_pass_its_bounds() {
        let cut_after = |chapters: &[String]| {
            let named: Vec<String> = (0..chapters.len()).map(|n| format!("{n}.xhtml")).collect();
            let files: Vec<(&str, &[u8])> = named.iter().zip(chapters).map(|(name, chapter)| (name.as_str(), chapter.as_bytes())).collect();
            let order: Vec<(&str, Kind)> = named.iter().map(|name| (name.as_str(), Kind::Xhtml)).collect();
            let (html, left) = assembled(&plan(&order), &book(&files));
            assert!(html.len() <= MAX_WRITTEN && html.contains("first"), "what is before the cut is there");
            (left, html.matches("<div id=").count())
        };
        let short = || chapter("<p>short</p>");
        let marks = |count: usize| chapter(&format!("<p>first</p>{}", "<br/>".repeat(count)));
        assert_eq!(cut_after(&[marks(MAX_MARKS / 2), marks(MAX_MARKS / 2), short(), short()]), (Left { past: 3, ..Left::default() }, 1));
        assert_eq!(cut_after(&[marks(MAX_MARKS / 4), marks(MAX_MARKS / 4), short()]).0, Left::default(), "within them");
        let lines = |count: usize| chapter(&format!("<pre>first\n{}</pre>", "\n".repeat(count)));
        assert_eq!(cut_after(&[lines(MAX_MARKS / 2), lines(MAX_MARKS / 2), short()]).0.past, 2, "a line's end is a mark");
        let text = |bytes: usize| chapter(&format!("<p>first {}</p>", "a".repeat(bytes)));
        assert_eq!(cut_after(&[text(MAX_TEXT / 2), short(), text(MAX_TEXT / 2), short()]), (Left { past: 2, ..Left::default() }, 2));
        let attributes = |bytes: usize| chapter(&format!("<p title=\"{}\">first</p>", "a".repeat(bytes)));
        // Each within a chapter's own limit.
        let fifth = MAX_WRITTEN / 5;
        assert_eq!(
            cut_after(&[attributes(fifth), attributes(fifth), attributes(fifth), attributes(fifth), attributes(fifth), short()]).0.past,
            2,
            "by its bytes"
        );
        assert_eq!(Left { past: 2, ..Left::default() }.notes(), ["2 chapters are left out: the book is longer than clv lays out"]);
    }

    /// A reference in a chapter is a path from the book's top, or none: resolved from the chapter's
    /// directory, `..` and `%XX` undone, never above the top; a web address is left as it is.
    #[test]
    fn a_chapters_references_are_paths_from_the_books_top() {
        let book = book(&[("OEBPS/images/a b.png", b"png"), ("OEBPS/text/two.xhtml", b"x"), ("cover.jpg", b"jpeg")]);
        let written = Written { chapter: 3, directory: "OEBPS/text", zipped: &book };
        for (src, pictured) in [
            ("../images/a%20b.png", "./OEBPS/images/a%20b.png"),
            ("../images/a b.png", "./OEBPS/images/a%20b.png"),
            ("plate.png?v=2#x", "./OEBPS/text/plate.png"),
            ("../../cover.jpg", "./cover.jpg"),
            ("/cover.jpg", "./cover.jpg"),
            ("../../../cover.jpg", "../above-the-book"),
            ("100%25.png", "./OEBPS/text/100%25.png"),
            ("what%3F.png", "./OEBPS/text/what%3F.png"),
            ("a:b.png", "a:b.png"),
            ("./a:b.png", "./OEBPS/text/a%3Ab.png"),
            ("https://example.com/a.png", "https://example.com/a.png"),
            ("//example.com/a.png", "//example.com/a.png"),
            ("data:image/png;base64,AAAA", "data:image/png;base64,AAAA"),
        ] {
            assert_eq!(written.pictured(src), pictured, "{src}");
        }
        for (href, linked) in [
            ("../images/a%20b.png", "clv-in-the-book/OEBPS/images/a%20b.png"),
            ("two.xhtml#s3", "clv-in-the-book/OEBPS/text/two.xhtml"),
            ("../../cover.jpg", "clv-in-the-book/cover.jpg"),
            ("three.xhtml", "clv-not-in-the-book/OEBPS/text/three.xhtml"),
            ("../../../../etc/passwd", "clv-not-in-the-book/"),
            ("/etc/passwd", "clv-not-in-the-book/etc/passwd"),
            ("clv-in-the-book/x", "clv-not-in-the-book/OEBPS/text/clv-in-the-book/x"),
            ("#s3", "#s3"),
            ("https://example.com/a?b=c#d", "https://example.com/a?b=c#d"),
            ("mailto:a@example.com", "mailto:a@example.com"),
        ] {
            assert_eq!(written.linked(href), linked, "{href}");
        }
    }

    /// A link the book's document reads as one to a file is answered here, as a file of the book
    /// or as none, and never left for the viewer to look for; any other link is as it was.
    #[test]
    fn a_books_link_to_a_file_is_a_message_never_a_file() {
        let link = |address: &str, target: Target| super::super::Link { page: 0, rects: Vec::new(), address: Some(address.to_owned()), target };
        let file = |path: &str| Target::File { path: path.to_owned(), fragment: Some("s3".into()), page: None };
        let held = in_the_book(link("clv-in-the-book/OEBPS/a%20b.png", file("clv-in-the-book/OEBPS/a b.png")));
        assert_eq!(
            (held.address.as_deref(), held.target),
            (Some("OEBPS/a%20b.png"), Target::Other("a file in the book, which clv doesn't open alone".into()))
        );
        let missing = in_the_book(link("clv-not-in-the-book/three.xhtml", file("clv-not-in-the-book/three.xhtml")));
        assert_eq!((missing.address.as_deref(), missing.target), (Some("three.xhtml"), Target::Other("not in this book".into())));
        // Whatever path it has: a link is never a file's.
        assert_eq!(in_the_book(link("x", file("../../etc/passwd"))).target, Target::Other("not in this book".into()));
        for target in [Target::Place { page: 2, y: 10.0 }, Target::Web("https://example.com".into()), Target::Other("no #x in this document".into())]
        {
            assert_eq!(in_the_book(link("as written", target.clone())).target, target);
        }
    }

    /// Which zips are books: one that says so in its first file, stored, whatever its name; one
    /// named `.epub` whose first bytes don't say what it is; and no other.
    #[test]
    fn a_book_is_a_zip_that_says_so_or_is_named_so() {
        use super::super::head::zip_head;
        let says = zip_head(&[("mimetype", MIMETYPE), ("META-INF/container.xml", b"x")]);
        assert!(opens("book.epub", &says) && opens("book.zip", &says) && opens("book", &says));
        assert!(opens("book.zip", &zip_head(&[("mimetype", b"application/epub+zip\r\n")])), "with a line's end");
        let other = zip_head(&[("mimetype", b"application/vnd.oasis.opendocument.text")]);
        assert!(!opens("book.epub", &other) && !opens("book.zip", &other), "it says it is something else");
        let unsaid = zip_head(&[("META-INF/container.xml", b"x"), ("mimetype", MIMETYPE)]);
        assert!(opens("book.epub", &unsaid) && opens("BOOK.EPUB", &unsaid) && !opens("book.zip", &unsaid), "its mimetype isn't first");
        // Compressed, as some tools write it: its first bytes say nothing that can be read.
        let mut packed = says.clone();
        packed[8] = 8;
        assert!(opens("book.epub", &packed) && !opens("book.zip", &packed));
        assert!(!opens("book.epub", b"<!doctype html>") && !opens("book.epub", b""), "not a zip");
        assert!(Epub.formats().iter().any(|format| format.named("a.epub") && format.names == ["epub"]));
    }

    #[test]
    fn a_container_names_its_package() {
        assert_eq!(package_path(&container("OEBPS/content.opf")).as_deref(), Some("OEBPS/content.opf"));
        assert_eq!(package_path(&container("a &amp; b/c&#233;.opf")).as_deref(), Some("a & b/cé.opf"));
        assert_eq!(package_path(&container("/OEBPS/./x/../content.opf")).as_deref(), Some("OEBPS/content.opf"));
        let two = r#"<c:rootfile full-path='first.opf'/><rootfile full-path="second.opf" media-type="application/oebps-package+xml"/>"#;
        assert_eq!(package_path(two).as_deref(), Some("first.opf"), "the first, in any namespace, with either quote");
        for none in [container("../above.opf"), container(""), "<rootfile/>".into(), "<rootfiles full-path='x'/>".into(), String::new()] {
            assert_eq!(package_path(&none), None, "{none}");
        }
        assert_eq!(unescaped("&lt;&gt;&quot;&apos;&#x41;&unknown;&;&#xZZ;& &amp"), "<>\"'A&unknown;&;&#xZZ;& &amp");
    }

    #[test]
    fn fields_are_read_as_they_were_written_and_no_further() {
        let mut written = Vec::new();
        for field in [&b"one"[..], b"", b"three"] {
            put(&mut written, field);
        }
        assert_eq!(Fields(&written).collect::<Vec<_>>(), [&b"one"[..], b"", b"three"]);
        assert_eq!(Fields(&written[..written.len() - 1]).collect::<Vec<_>>(), [&b"one"[..], b""], "one cut short is none");
        assert_eq!(Fields(&[0xFF, 0xFF, 0xFF, 0xFF, 1]).count(), 0);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        /// Whatever a chapter holds, and wherever it is cut, it is written into the book or left
        /// out: nothing panics, and what is written starts no page of another chapter's.
        #[test]
        fn any_chapter_is_assembled_or_left_out(
            body in r#"[<>/="' a-c!?&;#%.:é\n-]{0,96}"#,
            flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 0..8),
            cut in any::<prop::sample::Index>(),
        ) {
            let mut bytes = chapter(&format!(r#"<p id="x"><a href="{body}">a</a><img src="{body}"/>{body}</p>"#)).into_bytes();
            for (at, byte) in flips {
                let at = at.index(bytes.len());
                bytes[at] ^= byte;
            }
            bytes.truncate(cut.index(bytes.len() + 1));
            let book = book(&[("text/a.xhtml", &bytes), ("text/b.xhtml", chapter("<p>after</p>").as_bytes())]);
            let (html, left) = assembled(&plan(&[("text/a.xhtml", Kind::Xhtml), ("text/b.xhtml", Kind::Xhtml)]), &book);
            prop_assert!(html.len() >= HEAD as usize && html.ends_with("<p>after</p></div>\n"));
            prop_assert!(left.missing + left.too_large + left.other + left.past == 0 && left.unreadable <= 1);
        }

        /// Whatever the worker answers, it is a reading order within the limits or none: nothing
        /// panics, and no path leads above the book's top.
        #[test]
        fn any_answer_is_a_plan_or_none(answer in prop::collection::vec(any::<u8>(), 0..256), count in 0u32..8, paths in prop::collection::vec("[a-c/.%]{0,12}", 0..8)) {
            let _ = Plan::read(&answer);
            let mut framed = Vec::new();
            put(&mut framed, &count.to_le_bytes());
            for path in &paths {
                put(&mut framed, path.as_bytes());
                put(&mut framed, b"application/xhtml+xml");
            }
            if let Some(plan) = Plan::read(&framed) {
                prop_assert_eq!(plan.chapters.len(), count as usize);
                for chapter in &plan.chapters {
                    prop_assert!(chapter.path.as_deref().is_none_or(|path| !path.split('/').any(|part| part == ".." || part.is_empty())));
                }
            }
        }
    }
}
