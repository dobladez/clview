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

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Cursor, Write as _};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use super::head::{ZIP, mimetype};
use super::markup::{Piece, SKIPPED, StartTag, past_end_tag};
use super::worker::{self, Died, FAILED, Link as Worked, Session, WALL, Worker};
use super::zipped::Zipped;
use super::{
    Again, Anchor, Ask, Auto, BACKENDS, Backend, Bounded, Bounds, Deferred, Effect, Format, Frame, HEAD, Input, Job, Keys, Links, MAX_NAME, Named,
    Near, Numbers, Opened, Pool, Render, Reopenable, Request, Room, Setting, Source, Start, Target, Texts, Told, Value, Values,
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

/// The largest `META-INF/encryption.xml` read, which says which of the book's files are protected,
/// and the most files it may say are.
const MAX_ENCRYPTION: u64 = 1 << 20;
const MAX_PROTECTED: usize = 16_384;

/// The most fallbacks followed from a spine's item that is neither XHTML nor a picture, to one
/// that is: a chain that comes back on itself stops there.
const MAX_FALLBACKS: usize = 8;

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
    if left.shown == 0 && left.protected > 0 {
        return Err(error("it is protected (DRM)".into()));
    }
    let html = html.into_bytes();
    let head = html.get(..HEAD as usize).unwrap_or(&html).to_vec();
    let again: Again = {
        let (kept, plan) = (kept.clone(), plan.clone());
        Arc::new(move || Ok(assembled(&plan, &*kept.zipped()?).0.into_bytes()))
    };
    // The book's document is at its top: its pictures' paths are written from there.
    let document = Input::held(path.clone(), head, again).held_near(near(&kept, &plan, &path), String::new());
    // Not this one: what is assembled is never a book.
    let Some(backend) = BACKENDS.iter().filter(|b| b.name() != Epub.name()).find(|b| b.opens(&document.path, &document.head)) else {
        return Err(error("a book is shown as the HTML its chapters make, and no backend opens that".into()));
    };
    let shown = match room {
        Some(room) => backend.open_at(document, options, room)?,
        None => backend.open(document, options)?,
    };
    match shown {
        Opened::Pages(pages) => Ok(Opened::Pages(Box::new(Book { pages, notes: left.notes(plan.fixed).into() }))),
        Opened::Files(_) => Err(error("its chapters' HTML holds files".into())),
    }
}

/// What is beside the book's document ([`Near`]): the file at a path from the book's top, by its
/// very name in the zip, read from the book each time, as a held file of its own. Not a file the
/// book protects, whose bytes are no picture.
fn near(kept: &Kept, plan: &Arc<Plan>, book: &str) -> Near {
    let (kept, plan, book) = (kept.clone(), plan.clone(), book.to_owned());
    Arc::new(move |path| {
        let named = format!("{book}/{path}");
        let error = |e: String| format!("{named}: {e}");
        if plan.protects(path) {
            return Err(error("protected (DRM)".into()));
        }
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
#[derive(Debug, Default, PartialEq, Eq)]
struct Plan {
    /// The spine's items, in its order, each as the file that is shown for it.
    chapters: Vec<Chapter>,
    /// The cover picture the package names, by its path from the book's top.
    cover: Option<String>,
    /// Whether the package says the book is laid out page by page (`rendition:layout`
    /// `pre-paginated`), which it isn't here.
    fixed: bool,
    /// The files the book protects, by their paths from its top, sorted: encrypted by anything but
    /// a font's obfuscation.
    protected: Vec<String>,
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
    /// A picture or a drawing, shown as a page of it.
    Picture,
    /// Anything else.
    Other,
}

impl Kind {
    /// By a manifest item's media type: XHTML, as EPUB 3 names it, and as EPUB 2's OEBPS could; a
    /// picture, of any kind (`image/…`, SVG among them).
    fn of(media_type: &[u8]) -> Self {
        let media_type = media_type.trim_ascii();
        let starts = |start: &[u8]| media_type.get(..start.len()).is_some_and(|head| head.eq_ignore_ascii_case(start));
        let xhtml = [&b"application/xhtml+xml"[..], b"text/html", b"text/x-oeb1-document"];
        match xhtml.iter().any(|xhtml| media_type.eq_ignore_ascii_case(xhtml)) {
            true => Self::Xhtml,
            false if starts(b"image/") => Self::Picture,
            false => Self::Other,
        }
    }
}

impl Plan {
    /// Whether the book protects the file at `path`, from its top.
    fn protects(&self, path: &str) -> bool {
        self.protected.binary_search_by(|protected| protected.as_str().cmp(path)).is_ok()
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
    // Which of its files are protected, if it says: the worker's to read too.
    let encryption = match zipped.index_of(ENCRYPTION) {
        Some(_) => file(ENCRYPTION, MAX_ENCRYPTION, "list of protected files")?,
        None => Vec::new(),
    };
    let mut request = Vec::new();
    put(&mut request, opf_path.as_bytes());
    put(&mut request, &opf);
    put(&mut request, &encryption);
    match read(READ, request) {
        Ok((PLAN, answer)) => Plan::read(&answer).ok_or_else(|| UNREAD.to_owned()),
        Ok((REFUSED, why)) => Err(cut(&String::from_utf8_lossy(&why), MAX_WHY).to_owned()),
        Ok(_) => Err(UNREAD.into()),
        Err(Died::TooMany) => Err("too many files are being read at once to read its package".into()),
        Err(Died::Unstarted(why)) => Err(format!("the process that reads its package couldn't be started ({})", cut(&why, MAX_WHY))),
        Err(_) => Err(UNREAD.into()),
    }
}

/// Where a book says which of its files are encrypted, and how (EPUB 3.3's OCF, §4.2.6.3.5).
const ENCRYPTION: &str = "META-INF/encryption.xml";

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

/// What the worker reads of a package ([`READ`]'s answer), before the viewer checks it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Answer {
    /// Each item of the spine, in order: the path and the media type of the file shown for it; no
    /// path for an item the manifest doesn't have.
    chapters: Vec<(Option<String>, String)>,
    /// The cover picture's path.
    cover: Option<String>,
    fixed: bool,
    /// The files encrypted by anything but a font's obfuscation.
    protected: Vec<String>,
}

impl Answer {
    /// As the worker writes it: whether the book is laid out page by page, its cover's path, how
    /// many chapters and each chapter's path and media type, then how many protected files and
    /// each one's path.
    fn written(&self) -> Vec<u8> {
        let count = |count: usize| u32::try_from(count).unwrap_or(u32::MAX).to_le_bytes();
        let mut out = Vec::new();
        put(&mut out, &[u8::from(self.fixed)]);
        put(&mut out, self.cover.as_deref().unwrap_or_default().as_bytes());
        put(&mut out, &count(self.chapters.len()));
        for (path, media_type) in &self.chapters {
            put(&mut out, path.as_deref().unwrap_or_default().as_bytes());
            put(&mut out, cut(media_type, MAX_MEDIA_TYPE).as_bytes());
        }
        put(&mut out, &count(self.protected.len()));
        for path in &self.protected {
            put(&mut out, path.as_bytes());
        }
        out
    }
}

impl Plan {
    /// What the worker answered a [`READ`] with ([`Answer::written`]), checked as any input is:
    /// within the limits, its paths text, brought to paths from the book's top. `None` for an answer
    /// that isn't one.
    fn read(answer: &[u8]) -> Option<Self> {
        let mut fields = Fields(answer);
        let count = |field: &[u8]| usize::try_from(u32::from_le_bytes(*field.first_chunk::<4>()?)).ok();
        // A path within its limit, from the book's top; none for one that leads above it.
        let path = |field: &[u8]| (field.len() <= MAX_PATH).then(|| std::str::from_utf8(field).ok().and_then(|path| from_the_top("", path)));
        let fixed = matches!(fields.next()?, [1]);
        let cover = path(fields.next()?)?;
        let chapters = count(fields.next()?).filter(|count| *count <= MAX_CHAPTERS)?;
        let mut read = Vec::new();
        for _ in 0..chapters {
            let (file, media_type) = (fields.next()?, fields.next()?);
            read.push(Chapter { path: path(file)?, kind: Kind::of(media_type) });
        }
        let protected = count(fields.next()?).filter(|count| *count <= MAX_PROTECTED)?;
        let mut files = Vec::new();
        for _ in 0..protected {
            files.extend(path(fields.next()?)?);
        }
        files.sort_unstable();
        fields.next().is_none().then_some(Self { chapters: read, cover, fixed, protected: files })
    }
}

/// Why chapters were left out of a book assembled, and how many for each; and how many are shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Left {
    /// Not in the book: the manifest names a file the zip doesn't hold, or the spine an item the
    /// manifest doesn't.
    missing: usize,
    /// Read with an error, or not text this reads: neither UTF-8 nor UTF-16 with its byte-order mark.
    unreadable: usize,
    /// More than [`MAX_CHAPTER`].
    too_large: usize,
    /// Neither XHTML nor a picture, with no fallback that is.
    other: usize,
    /// Past what a book assembled may come to ([`MAX_WRITTEN`], [`MAX_MARKS`], [`MAX_TEXT`]).
    past: usize,
    /// Protected: encrypted by anything but a font's obfuscation.
    protected: usize,
    /// The chapters written into the book.
    shown: usize,
}

impl Left {
    /// What the first page says of the chapters left out, a line for each reason; and of a book
    /// made to be laid out page by page (`fixed`), which this isn't.
    fn notes(&self, fixed: bool) -> Vec<String> {
        let said = |count: usize, why: &str| match count {
            0 => None,
            1 => Some(format!("1 chapter is left out: {why}")),
            count => Some(format!("{count} chapters are left out: {why}")),
        };
        let protected = match self.protected {
            0 => None,
            1 => Some("1 chapter is protected (DRM)".to_owned()),
            count => Some(format!("{count} chapters are protected (DRM)")),
        };
        [
            protected,
            said(self.missing, "not in the book"),
            said(self.unreadable, "unreadable, or not text"),
            said(self.too_large, &format!("more than {} MiB", MAX_CHAPTER >> 20)),
            said(self.other, &format!("not a kind of chapter {} shows", crate::NAME)),
            said(self.past, &format!("the book is longer than {} lays out", crate::NAME)),
            fixed.then(|| "a fixed-layout book: its pages are laid out as its text flows, not as they were made".to_owned()),
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
/// `<div>` that starts a page, in the spine's order, a chapter that is a picture as a page of it;
/// before them the cover, a page of its own, when the package names one that the first chapter
/// shown doesn't show already. And how many chapters were left out, and why.
fn assembled(plan: &Plan, zipped: &Zipped) -> (String, Left) {
    let mut out = start();
    let (mut left, mut marks, mut text) = (Left::default(), 0usize, 0usize);
    // Each chapter's number in the reading order, from 1, by its file: where a link to it leads.
    let mut numbers: HashMap<&str, usize> = HashMap::new();
    for (number, chapter) in plan.chapters.iter().enumerate() {
        if let Some(path) = chapter.path.as_deref() {
            numbers.entry(path).or_insert(number.saturating_add(1));
        }
    }
    // The cover, as a picture the book holds and may show.
    let cover = plan.cover.as_deref().filter(|cover| zipped.index_of(cover).is_some() && !plan.protects(cover));
    for (number, chapter) in plan.chapters.iter().enumerate() {
        let Some(path) = chapter.path.as_deref() else {
            left.missing = left.missing.saturating_add(1);
            continue;
        };
        if chapter.kind == Kind::Other {
            left.other = left.other.saturating_add(1);
            continue;
        }
        let Some(index) = zipped.index_of(path) else {
            left.missing = left.missing.saturating_add(1);
            continue;
        };
        if plan.protects(path) {
            left.protected = left.protected.saturating_add(1);
            continue;
        }
        let number = number.saturating_add(1);
        let (body, body_text) = match chapter.kind {
            Kind::Picture => (page_of(&format!("c{number}"), path), 0),
            _ => {
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
                let directory = path.rsplit_once('/').map_or("", |(directory, _)| directory);
                let written = Written { chapter: number, directory, zipped, chapters: &numbers };
                written.body(&xhtml)
            }
        };
        // Before the first chapter shown, unless that one shows it.
        let first = match cover.filter(|cover| left.shown == 0 && !body.contains(&pictured_from_the_top(cover))) {
            Some(cover) => page_of("cover", cover),
            None => String::new(),
        };
        let body_marks = first.bytes().chain(body.bytes()).filter(|b| matches!(b, b'<' | b'\r' | b'\n')).count();
        let (marks_now, text_now) = (marks.saturating_add(body_marks), text.saturating_add(body_text));
        if out.len().saturating_add(first.len()).saturating_add(body.len()) > MAX_WRITTEN || marks_now > MAX_MARKS || text_now > MAX_TEXT {
            // With those after it: a book cut short reads to where it is cut.
            left.past = plan.chapters.len().saturating_add(1).saturating_sub(number);
            break;
        }
        (marks, text) = (marks_now, text_now);
        out.push_str(&first);
        out.push_str(&body);
        left.shown = left.shown.saturating_add(1);
    }
    (out, left)
}

/// A page of the picture at `path`, from the book's top, with the id `id`: a chapter that is a
/// picture, or the cover.
fn page_of(id: &str, path: &str) -> String {
    format!("<div id=\"{id}\" style=\"page-break-before: always\"><p><img src=\"{}\" alt=\"\"/></p></div>\n", pictured_from_the_top(path))
}

/// The picture at `path`, from the book's top, as the book's document refers to it: written as a
/// document's pictures are read (their `%XX` undone), so that every character that would mean
/// something there is escaped.
fn pictured_from_the_top(path: &str) -> String {
    format!("./{}", percent_encoded(path))
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

/// What marks a link's path, in the book assembled, as a file of the book that is no chapter, as
/// a file the book doesn't hold, and as a place whose name is too long to be one: written by
/// [`Written::linked`] for every link that leads to no place in the book, so that [`Book`] answers
/// each without reading the book, and none is ever looked for on disk.
const IN_THE_BOOK: &str = "clv-in-the-book/";
const NOT_IN_THE_BOOK: &str = "clv-not-in-the-book/";
const NO_SUCH_PLACE: &str = "clv-no-such-place/";

/// What is written before a table that is the first thing a chapter shows: a paragraph of one
/// zero-width space, a point high. A page break on the chapter's `<div>` reaches the first
/// paragraph in it, and a table is none, nor is an empty paragraph one (ADR-0038).
const BEFORE_A_TABLE: &str = "<p style=\"font-size: 1pt; margin: 0\">&#8203;</p>";

/// The elements that show something without any text in them: a chapter that starts with one has
/// started.
const SHOWN: [&str; 5] = ["img", "hr", "svg", "table", "br"];

/// HTML's void elements, which have no end tag: written short (`<br/>`), one is all there.
const VOID: [&str; 14] = ["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];

/// The picture an `<svg>` wraps, when that is all it holds, as a cover's page writes it
/// (`<svg><image xlink:href="cover.jpg"/></svg>`): its reference as written, and where the
/// `<svg>`, whose content starts at `at` in `xhtml`, ends. `None` for a drawing of anything else,
/// or of two pictures, which is left to be read as it is.
fn wrapped_picture(xhtml: &str, mut at: usize) -> Option<(&str, usize)> {
    let mut picture = None;
    while let Some((piece, next)) = Piece::at(xhtml, at) {
        at = next;
        match piece {
            Piece::Text(run) if run.trim().is_empty() => {}
            Piece::Other => {}
            Piece::End(name) if name.eq_ignore_ascii_case("svg") => return picture.map(|picture| (picture, at)),
            Piece::End(name) if name.eq_ignore_ascii_case("image") => {}
            // What says what the drawing is, to a reader that can't see it.
            Piece::Start(tag) if (tag.is("title") || tag.is("desc")) && !tag.self_closed => at = past_end_tag(xhtml, at, tag.name),
            Piece::Start(tag) if tag.is("image") && picture.is_none() => {
                let href = |name: &str| tag.attributes().find(|attribute| attribute.name.eq_ignore_ascii_case(name)).map(|attribute| attribute.value);
                picture = Some(href("xlink:href").or_else(|| href("href")).filter(|href| !href.trim().is_empty())?);
            }
            _ => return None,
        }
    }
    None
}

/// A chapter being written into the book.
struct Written<'a> {
    /// Its number in the reading order, from 1: what its ids start with.
    chapter: usize,
    /// The directory its file is in, from the book's top: what its paths start from.
    directory: &'a str,
    zipped: &'a Zipped,
    /// The book's chapters, each by its file's path from the book's top: its number in the reading
    /// order, from 1.
    chapters: &'a HashMap<&'a str, usize>,
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
                    // A drawing that only wraps a picture, as a cover's page does, is that picture.
                    match Some(&tag).filter(|tag| tag.is("svg") && !tag.self_closed).and_then(|_| wrapped_picture(xhtml, at)) {
                        Some((picture, past)) => {
                            let _ = write!(out, "<img src=\"{}\" alt=\"\"/>", self.pictured(picture));
                            at = past;
                        }
                        None => self.start_tag(&tag, &mut out),
                    }
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
                // The book's own: its chapter's number before it. One too long to be a place's
                // name once it has that is left out, and a link to it says so.
                name if name.eq_ignore_ascii_case("id") => match place(self.chapter, Some(attribute.value)) {
                    Some(id) => id,
                    None => continue,
                },
                name if name.eq_ignore_ascii_case("src") => self.pictured(attribute.value),
                name if name.eq_ignore_ascii_case("href") => self.linked(attribute.value),
                _ => attribute.value.to_owned(),
            };
            // As written, a value holds no quote of the kind around it; one written again may.
            let quote = if value.contains('"') { '\'' } else { '"' };
            let _ = write!(out, " {name}={quote}{}{quote}", value.replace('\'', if quote == '\'' { "&#39;" } else { "'" }));
        }
        // XHTML writes any element short when it is empty; read as HTML, only a void one ends
        // there, and any other would stay open over what follows it.
        match (tag.self_closed, VOID.iter().any(|name| tag.is(name))) {
            (true, false) => {
                let _ = write!(out, "></{}>", tag.name);
            }
            (true, true) => out.push_str("/>"),
            (false, _) => out.push('>'),
        }
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
            Some((Some(path), _)) => pictured_from_the_top(&path),
            Some((None, _)) => "../above-the-book".to_owned(),
            None => src.to_owned(),
        }
    }

    /// A link's `href` as the book's document writes it: a web address as it is; a link to a
    /// chapter, or to a place in one, as that place in the document (`#c3`, a chapter's start, or
    /// `#c3-s1`, the chapter's `id="s1"`), this chapter's for a `#name` alone; a link to any other
    /// file, marked as a file of the book or as none ([`IN_THE_BOOK`], [`NOT_IN_THE_BOOK`]) and
    /// written from the book's top, so that it is never a path on disk.
    fn linked(&self, href: &str) -> String {
        let reference = href.trim();
        let Some((path, rest)) = self.resolved(reference) else { return href.to_owned() };
        let fragment = rest.split_once('#').map(|(_, fragment)| fragment).filter(|fragment| !fragment.is_empty());
        // Its chapter: this one, for a place alone or a link to nowhere else.
        let own = reference.is_empty() || reference.starts_with(['#', '?']);
        let chapter = if own { Some(self.chapter) } else { path.as_deref().and_then(|path| self.chapters.get(path).copied()) };
        match (chapter, path) {
            (Some(chapter), _) => place(chapter, fragment).map_or_else(|| NO_SUCH_PLACE.to_owned(), |name| format!("#{name}")),
            (None, Some(path)) if self.zipped.index_of(&path).is_some() => format!("{IN_THE_BOOK}{}", percent_encoded(&path)),
            (None, path) => format!("{NOT_IN_THE_BOOK}{}", percent_encoded(path.as_deref().unwrap_or_default())),
        }
    }
}

/// The name, in the book's document, of a place in the chapter numbered `chapter`: the chapter's
/// start, or what the chapter calls `name` there, with the chapter's number before it, since two
/// chapters may call a place the same. `None` for a name longer, so made, than a place's may be
/// ([`MAX_NAME`]).
fn place(chapter: usize, name: Option<&str>) -> Option<String> {
    let place = match name {
        Some(name) => format!("c{chapter}-{name}"),
        None => format!("c{chapter}"),
    };
    (place.len() <= MAX_NAME).then_some(place)
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
    let (said, marker) = match path {
        path if path.starts_with(IN_THE_BOOK) => (format!("a file in the book, which {} doesn't open alone", crate::NAME), IN_THE_BOOK),
        path if path.starts_with(NO_SUCH_PLACE) => ("no such place in this book".to_owned(), NO_SUCH_PLACE),
        _ => ("not in this book".to_owned(), NOT_IN_THE_BOOK),
    };
    link.address = link.address.and_then(|address| match address.strip_prefix(marker) {
        Some("") => None,
        Some(path) => Some(path.to_owned()),
        None => Some(address),
    });
    link.target = Target::Other(said);
    link
}

// The worker's side.

/// A book's package, to read: the request. Its path in the book, its bytes, and the bytes of the
/// book's list of protected files, empty for a book without one ([`put`]).
const READ: u8 = 1;

/// Its reading order, its cover and its protected files ([`Answer::written`]): an answer.
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

/// The longest request: a package's path, the package and the book's list of protected files
/// ([`MAX_PATH`], [`MAX_OPF`], [`MAX_ENCRYPTION`]), each with its length.
const MAX_REQUEST: usize = MAX_PATH + (MAX_OPF as usize) + (MAX_ENCRYPTION as usize) + 12;

/// The longest media type kept of a manifest's item, in bytes.
const MAX_MEDIA_TYPE: usize = 255;

/// The longest answer: the cover's path, every chapter's path and media type, and the protected
/// files' paths, which together are no longer than the list they are read from; each with its
/// length, and their counts.
const MAX_ANSWER: usize = 32 + MAX_PATH + MAX_CHAPTERS * (MAX_PATH + MAX_MEDIA_TYPE + 8) + (MAX_ENCRYPTION as usize) + MAX_PROTECTED * 4;

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
        let (Some(path), Some(opf), Some(encryption), None) =
            (fields.next().and_then(|path| std::str::from_utf8(path).ok()), fields.next(), fields.next(), fields.next())
        else {
            return (FAILED, vec![MALFORMED]);
        };
        let read = std::panic::catch_unwind(|| read(path, opf, encryption));
        match read.unwrap_or_else(|_| Err(UNREAD.into())) {
            Ok(answer) => (PLAN, answer.written()),
            Err(why) => (REFUSED, cut(&why, MAX_WHY).as_bytes().to_vec()),
        }
    }
}

/// What rbook reads of the package `opf`, the file at `path` in its book: each item of the spine,
/// in order, as the path from the book's top and the media type of the file shown for it; the
/// cover picture it names; and whether it says the book is laid out page by page. With them, the
/// files `encryption`, the book's list of them, says are protected. Or why not. rbook is given a
/// zip holding the package alone, under a container that names it, and reads neither the table of
/// contents (`skip_toc`: one it can't read fails the whole package) nor any other file.
fn read(path: &str, opf: &[u8], encryption: &[u8]) -> Result<Answer, String> {
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
    let located = |item: &rbook::epub::manifest::EpubManifestEntry<'_>| -> Result<String, String> {
        let path = item.href().path().decode().into_owned();
        match path.len() > MAX_PATH {
            true => Err("a file's path is too long".into()),
            false => Ok(path),
        }
    };
    let mut chapters = Vec::new();
    for entry in spine.iter() {
        let Some(item) = entry.manifest_entry() else {
            chapters.push((None, String::new()));
            continue;
        };
        // What is shown for it: itself, or the first of its fallbacks that is XHTML or a picture.
        let mut shown = item;
        for _ in 0..MAX_FALLBACKS {
            if Kind::of(shown.media_type().as_bytes()) != Kind::Other {
                break;
            }
            let Some(fallback) = shown.fallback() else { break };
            shown = fallback;
        }
        let shown = if Kind::of(shown.media_type().as_bytes()) == Kind::Other { item } else { shown };
        chapters.push((Some(located(&shown)?), shown.media_type().to_owned()));
    }
    if chapters.is_empty() {
        return Err("its package lists no chapters to read".into());
    }
    let cover = epub.manifest().cover_image().map(|cover| located(&cover)).transpose()?;
    let fixed = epub.metadata().by_property("rendition:layout").any(|layout| layout.value().trim() == "pre-paginated");
    Ok(Answer { chapters, cover, fixed, protected: protected(&String::from_utf8_lossy(encryption))? })
}

/// The algorithms that only obfuscate a font, IDPF's and Adobe's: a file encrypted by one of them
/// is no protected file, and a document's fonts aren't used anyway.
const OBFUSCATIONS: [&str; 2] = ["http://www.idpf.org/2008/embedding", "http://ns.adobe.com/pdf/enc#RC"];

/// The files `xml`, a book's `META-INF/encryption.xml`, says are encrypted by anything but a font's
/// obfuscation, by their paths from the book's top: each `EncryptedData`'s `CipherReference`, unless
/// its `EncryptionMethod` is one of [`OBFUSCATIONS`]. rbook doesn't read this file: a scan of the
/// worker's own, which a hostile file can at most kill the worker on.
fn protected(xml: &str) -> Result<Vec<String>, String> {
    // An element's name without its namespace's prefix.
    let local = |name: &str| name.rsplit(':').next().unwrap_or(name).to_owned();
    let mut found = Vec::new();
    // The data being read: whether its method is a font's obfuscation, once said, and its file.
    let mut data: Option<(Option<bool>, Option<String>)> = None;
    let mut at = 0;
    while let Some((piece, next)) = Piece::at(xml, at) {
        at = next;
        match piece {
            Piece::Start(tag) if local(tag.name) == "EncryptedData" => data = Some((None, None)),
            Piece::Start(tag) => {
                let Some((method, file)) = &mut data else { continue };
                let attribute = |name: &str| tag.attributes().find(|attribute| attribute.name == name).map(|attribute| unescaped(attribute.value));
                match local(tag.name).as_str() {
                    // Its own, which comes first; a key's, inside it, comes after.
                    "EncryptionMethod" if method.is_none() => {
                        *method = Some(attribute("Algorithm").is_some_and(|algorithm| OBFUSCATIONS.contains(&algorithm.trim())))
                    }
                    "CipherReference" if file.is_none() => {
                        *file = attribute("URI").and_then(|uri| percent_decoded(uri.trim())).and_then(|path| from_the_top("", &path))
                    }
                    _ => {}
                }
            }
            Piece::End(name) if local(name) == "EncryptedData" => {
                if let Some((method, Some(file))) = data.take()
                    && method != Some(true)
                {
                    if found.len() >= MAX_PROTECTED {
                        return Err(format!("it protects more than {MAX_PROTECTED} files"));
                    }
                    found.push(file);
                }
            }
            _ => {}
        }
    }
    Ok(found)
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

    /// What the worker answers of a package whose spine is `chapters`, and nothing else.
    fn answered(chapters: Vec<(Option<String>, String)>) -> Vec<u8> {
        Answer { chapters, ..Answer::default() }.written()
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
            assert_eq!(kinds, [Kind::Xhtml, Kind::Xhtml, Kind::Xhtml, Kind::Other, Kind::Picture, Kind::Xhtml], "{version}");
            assert_eq!((plan.cover.as_deref(), plan.fixed, plan.protected.len()), (None, false, 0), "{version}");
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
            |_, _| Ok((PLAN, answered(vec![(Some("a".to_owned()), String::new()); MAX_CHAPTERS + 1]))),
            |_, _| {
                let mut answer = answered(vec![(Some("a".to_owned()), String::new())]);
                answer.extend_from_slice(&[0; 9]);
                Ok((PLAN, answer))
            },
            |_, _| Ok((PLAN, answered(vec![(Some("a".repeat(MAX_PATH + 1)), String::new())]))),
        ];
        for answer in answers {
            assert_eq!(planned(&book, answer).unwrap_err(), UNREAD);
        }
        assert_eq!(planned(&book, |_, _| Err(Died::TooMany)).unwrap_err(), "too many files are being read at once to read its package");
        assert!(planned(&book, |_, _| Err(Died::Unstarted("no such file".into()))).unwrap_err().ends_with("couldn't be started (no such file)"));
        assert_eq!(planned(&book, |_, _| Ok((REFUSED, "rbook's reason".into()))).unwrap_err(), "rbook's reason");
        // What the worker answers is checked, not trusted: a path above the top is no chapter.
        let above = planned(&book, |_, _| Ok((PLAN, answered(vec![(Some("../../etc/passwd".to_owned()), XHTML.to_owned())])))).unwrap();
        assert_eq!(paths(&above), [None]);
    }

    fn plan(chapters: &[(&str, Kind)]) -> Plan {
        let chapters = chapters.iter().map(|(path, kind)| Chapter { path: (!path.is_empty()).then(|| (*path).to_owned()), kind: *kind }).collect();
        Plan { chapters, ..Plan::default() }
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
        assert_eq!(left, Left { shown: 3, ..Left::default() });
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
        let written = Written { chapter: 1, directory: "", zipped: &book, chapters: &HashMap::new() };
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
        assert_eq!(left, Left { missing: 2, unreadable: 2, too_large: 1, other: 1, shown: 4, ..Left::default() });
        let shown: Vec<&str> = ["fine", "sixteen", "big", "marked", "caf"].into_iter().filter(|text| html.contains(&format!("<p>{text}"))).collect();
        assert_eq!(shown, ["fine", "sixteen", "big", "marked"]);
        assert!(html.contains(r#"<div id="c8""#) && !html.contains(r#"<div id="c9""#), "numbered by their place in the reading order");
        assert_eq!(
            left.notes(false),
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
        assert_eq!(cut_after(&[marks(MAX_MARKS / 2), marks(MAX_MARKS / 2), short(), short()]), (Left { past: 3, shown: 1, ..Left::default() }, 1));
        assert_eq!(cut_after(&[marks(MAX_MARKS / 4), marks(MAX_MARKS / 4), short()]).0.past, 0, "within them");
        let lines = |count: usize| chapter(&format!("<pre>first\n{}</pre>", "\n".repeat(count)));
        assert_eq!(cut_after(&[lines(MAX_MARKS / 2), lines(MAX_MARKS / 2), short()]).0.past, 2, "a line's end is a mark");
        let text = |bytes: usize| chapter(&format!("<p>first {}</p>", "a".repeat(bytes)));
        assert_eq!(cut_after(&[text(MAX_TEXT / 2), short(), text(MAX_TEXT / 2), short()]), (Left { past: 2, shown: 2, ..Left::default() }, 2));
        let attributes = |bytes: usize| chapter(&format!("<p title=\"{}\">first</p>", "a".repeat(bytes)));
        // Each within a chapter's own limit.
        let fifth = MAX_WRITTEN / 5;
        assert_eq!(
            cut_after(&[attributes(fifth), attributes(fifth), attributes(fifth), attributes(fifth), attributes(fifth), short()]).0.past,
            2,
            "by its bytes"
        );
        assert_eq!(Left { past: 2, ..Left::default() }.notes(false), ["2 chapters are left out: the book is longer than clv lays out"]);
    }

    /// A reference in a chapter is a path from the book's top, or none: resolved from the chapter's
    /// directory, `..` and `%XX` undone, never above the top; a web address is left as it is.
    #[test]
    fn a_chapters_references_are_paths_from_the_books_top() {
        let book = book(&[("OEBPS/images/a b.png", b"png"), ("OEBPS/text/two.xhtml", b"x"), ("cover.jpg", b"jpeg")]);
        let written = Written { chapter: 3, directory: "OEBPS/text", zipped: &book, chapters: &HashMap::new() };
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
    }

    /// A link to a chapter, or to a place in one, is that place in the book: by a path from the
    /// chapter that writes it, `..` and `%XX` undone, with its fragment or without; a `#name` alone
    /// is a place in its own chapter. A link to a file that is no chapter is marked as a file of
    /// the book, or as none; never left a path.
    #[test]
    fn links_between_chapters_are_places_in_the_book() {
        let book = book(&[
            ("OEBPS/front/one.xhtml", b"x"),
            ("OEBPS/text/chapter two.xhtml", b"x"),
            ("OEBPS/text/three.xhtml", b"x"),
            ("OEBPS/images/a b.png", b"png"),
            ("OEBPS/style.css", b"css"),
            ("cover.jpg", b"jpeg"),
        ]);
        // The fourth is in the reading order, and not in the book.
        let chapters = HashMap::from([
            ("OEBPS/front/one.xhtml", 1),
            ("OEBPS/text/chapter two.xhtml", 2),
            ("OEBPS/text/three.xhtml", 3),
            ("OEBPS/text/gone.xhtml", 4),
        ]);
        let written = Written { chapter: 3, directory: "OEBPS/text", zipped: &book, chapters: &chapters };
        for (href, linked) in [
            ("chapter%20two.xhtml", "#c2"),
            ("chapter%20two.xhtml#s3", "#c2-s3"),
            ("./chapter two.xhtml#", "#c2"),
            ("chapter%20two.xhtml?x=1#s3", "#c2-s3"),
            ("../front/one.xhtml#top", "#c1-top"),
            ("../../OEBPS/front/../front/one.xhtml", "#c1"),
            ("/OEBPS/front/one.xhtml#top", "#c1-top"),
            ("  ../front/one.xhtml#top ", "#c1-top"),
            ("#s3", "#c3-s3"),
            ("#", "#c3"),
            ("", "#c3"),
            ("three.xhtml#s3", "#c3-s3"),
            ("gone.xhtml#s1", "#c4-s1"),
            ("../images/a%20b.png", "clv-in-the-book/OEBPS/images/a%20b.png"),
            ("../style.css#x", "clv-in-the-book/OEBPS/style.css"),
            ("../../cover.jpg", "clv-in-the-book/cover.jpg"),
            ("missing.xhtml#s1", "clv-not-in-the-book/OEBPS/text/missing.xhtml"),
            ("../../../../etc/passwd", "clv-not-in-the-book/"),
            ("/etc/passwd", "clv-not-in-the-book/etc/passwd"),
            ("clv-in-the-book/x", "clv-not-in-the-book/OEBPS/text/clv-in-the-book/x"),
            ("https://example.com/a?b=c#d", "https://example.com/a?b=c#d"),
            ("mailto:a@example.com", "mailto:a@example.com"),
        ] {
            assert_eq!(written.linked(href), linked, "{href}");
        }
    }

    /// Two chapters' ids don't meet: each has its chapter's number before it, and so has every link
    /// to it, from its own chapter or from another. A chapter's start is a place too.
    #[test]
    fn links_lead_to_ids_made_the_books_own() {
        let one = chapter(
            r##"<h1 id="start">One</h1><p><a href="#start">here</a>, <a href="two.xhtml#start">there</a>, <a href="two.xhtml">its start</a></p>"##,
        );
        let two = chapter(r##"<h1 id="start">Two</h1><p><a href="one.xhtml#start">back</a></p>"##);
        let book = book(&[("text/one.xhtml", one.as_bytes()), ("text/two.xhtml", two.as_bytes())]);
        let (html, _) = assembled(&plan(&[("text/one.xhtml", Kind::Xhtml), ("text/two.xhtml", Kind::Xhtml)]), &book);
        assert_eq!(
            &html[HEAD as usize..],
            concat!(
                r##"<div id="c1" style="page-break-before: always"><h1 id="c1-start">One</h1><p><a href="#c1-start">here</a>, <a href="#c2-start">there</a>, <a href="#c2">its start</a></p></div>"##,
                "\n",
                r##"<div id="c2" style="page-break-before: always"><h1 id="c2-start">Two</h1><p><a href="#c1-start">back</a></p></div>"##,
                "\n",
            )
        );
    }

    /// An id that would be longer than a place's name may be, once it has its chapter's number, is
    /// left out, and a link to it says that no such place is in the book.
    #[test]
    fn links_to_an_id_too_long_once_made_the_books_own_say_so() {
        let book = book(&[]);
        let written = Written { chapter: 12, directory: "", zipped: &book, chapters: &HashMap::new() };
        let (fits, long) = ("a".repeat(MAX_NAME - 4), "a".repeat(MAX_NAME - 3));
        assert_eq!(written.linked(&format!("#{fits}")), format!("#c12-{fits}"));
        assert_eq!(written.linked(&format!("#{long}")), NO_SUCH_PLACE);
        let (body, _) = written.body(&chapter(&format!(r##"<p id="{fits}" class="a">one</p><p id="{long}" class="b">two</p>"##)));
        assert!(body.contains(&format!(r#"<p id="c12-{fits}" class="a">"#)) && body.contains(r#"<p class="b">two</p>"#), "{body}");
        let link = super::super::Link {
            page: 0,
            rects: Vec::new(),
            address: Some(NO_SUCH_PLACE.to_owned()),
            target: Target::File { path: NO_SUCH_PLACE.to_owned(), fragment: None, page: None },
        };
        let answered = in_the_book(link);
        assert_eq!((answered.address, answered.target), (None, Target::Other("no such place in this book".into())));
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

    /// A package as [`opf`] writes one, with `metadata` of its own after what it must have, and its
    /// manifest's `items` as written.
    fn package(version: &str, metadata: &str, items: &str, itemrefs: &str) -> String {
        opf(version, &[], itemrefs).replace("</metadata>", &format!("{metadata}</metadata>")).replace("<manifest>", &format!("<manifest>{items}"))
    }

    /// The plan of the book whose package is `package`, with `more` files, read by the worker.
    fn planned_of(package: &str, more: &[(&str, &[u8])]) -> Result<Plan, String> {
        let container = container("OEBPS/content.opf");
        let mut files = vec![("META-INF/container.xml", container.as_bytes()), ("OEBPS/content.opf", package.as_bytes())];
        files.extend_from_slice(more);
        planned(&book(&files), in_a_thread)
    }

    const CHAPTER_ITEM: &str = r#"<item id="one" href="text/one.xhtml" media-type="application/xhtml+xml"/>"#;

    /// The cover the package names, as EPUB 3 does and as EPUB 2 did, is a page of its own before
    /// the first chapter shown, unless that chapter shows it, as a picture or wrapped in a drawing:
    /// then it isn't shown twice. A cover the book doesn't hold is no page.
    #[test]
    fn the_cover_comes_first_unless_the_first_chapter_shows_it() {
        let epub3 = package(
            "3.0",
            "",
            &format!(r#"{CHAPTER_ITEM}<item id="c" href="images/cover%201.jpg" media-type="image/jpeg" properties="cover-image"/>"#),
            r#"<itemref idref="one"/>"#,
        );
        let epub2 = package(
            "2.0",
            r#"<meta name="cover" content="c"/>"#,
            &format!(r#"{CHAPTER_ITEM}<item id="c" href="images/cover%201.jpg" media-type="image/jpeg"/>"#),
            r#"<itemref idref="one"/>"#,
        );
        for (version, package) in [("3.0", &epub3), ("2.0", &epub2)] {
            assert_eq!(planned_of(package, &[]).unwrap().cover.as_deref(), Some("OEBPS/images/cover 1.jpg"), "{version}");
        }
        assert_eq!(planned_of(&package("3.0", "", CHAPTER_ITEM, r#"<itemref idref="one"/>"#), &[]).unwrap().cover, None);
        let plan = planned_of(&epub3, &[]).unwrap();
        let covered = |first: &str, held: bool| {
            let (one, two) = (chapter(first), chapter("<p>two</p>"));
            let mut files: Vec<(&str, &[u8])> = vec![("OEBPS/text/one.xhtml", one.as_bytes()), ("OEBPS/text/two.xhtml", two.as_bytes())];
            if held {
                files.push(("OEBPS/images/cover 1.jpg", b"jpeg"));
            }
            assembled(&plan, &book(&files)).0.split_off(HEAD as usize)
        };
        let cover = r#"<div id="cover" style="page-break-before: always"><p><img src="./OEBPS/images/cover%201.jpg" alt=""/></p></div>"#;
        let text = covered("<p>one</p>", true);
        assert!(text.starts_with(&format!("{cover}\n<div id=\"c1\"")) && text.matches("cover%201.jpg").count() == 1, "{text}");
        for shows in [
            r#"<p><img src="../images/cover%201.jpg" alt="Cover"/></p>"#,
            r#"<div><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 800 1104"><image width="800" height="1104" xlink:href="../images/cover%201.jpg"/></svg></div>"#,
        ] {
            let text = covered(shows, true);
            assert!(text.starts_with(r#"<div id="c1""#) && text.matches("./OEBPS/images/cover%201.jpg").count() == 1, "not twice: {text}");
        }
        assert!(covered("<p>one</p>", false).starts_with(r#"<div id="c1""#), "a cover the book doesn't hold");
        // Before the first chapter that is shown, not one left out.
        let later = Plan {
            chapters: plan.chapters.iter().cloned().chain([Chapter { path: Some("OEBPS/text/two.xhtml".into()), kind: Kind::Xhtml }]).collect(),
            cover: plan.cover.clone(),
            ..Plan::default()
        };
        let two = chapter("<p>two</p>");
        let (html, left) = assembled(&later, &book(&[("OEBPS/text/two.xhtml", two.as_bytes()), ("OEBPS/images/cover 1.jpg", b"jpeg")]));
        assert!(html[HEAD as usize..].starts_with(&format!("{cover}\n<div id=\"c2\"")) && (left.missing, left.shown) == (1, 1), "{html}");
    }

    /// A drawing that only wraps a picture is that picture, as a cover's page writes it; a drawing
    /// of anything else, or of two pictures, is left as it is.
    #[test]
    fn an_svg_that_only_wraps_a_picture_is_that_picture() {
        let book = book(&[]);
        let written = Written { chapter: 1, directory: "text", zipped: &book, chapters: &HashMap::new() };
        let body = |svg: &str| {
            let body = written.body(&chapter(svg)).0;
            body.strip_prefix(r#"<div id="c1" style="page-break-before: always">"#).and_then(|body| body.strip_suffix("</div>\n")).unwrap().to_owned()
        };
        for wraps in [
            r#"<svg viewBox="0 0 8 11"><image width="8" height="11" xlink:href="../images/c.jpg"/></svg>"#,
            r#"<svg><title>The cover</title><desc>A painting</desc> <image href="../images/c.jpg"></image> </svg>"#,
            "<svg>\n<!-- the cover -->\n<IMAGE HREF='../images/c.jpg'/>\n</svg>",
        ] {
            assert_eq!(body(wraps), r#"<img src="./images/c.jpg" alt=""/>"#, "{wraps}");
        }
        assert_eq!(
            body(r#"<p>before</p><svg><image href="c.jpg"/></svg><p>after</p>"#),
            r#"<p>before</p><img src="./text/c.jpg" alt=""/><p>after</p>"#
        );
        for drawn in [
            r#"<svg><image href="a.jpg"/><image href="b.jpg"/></svg>"#,
            r#"<svg><rect width="1" height="1"/><image href="a.jpg"/></svg>"#,
            r#"<svg><image href="a.jpg"/><text>words</text></svg>"#,
            r#"<svg><image href="a.jpg"/>words</svg>"#,
            r#"<svg><image/></svg>"#,
            r#"<svg></svg>"#,
            r#"<svg><image href="a.jpg"/>"#,
        ] {
            assert!(body(drawn).starts_with("<svg>"), "{drawn}: {}", body(drawn));
        }
    }

    /// XHTML writes an empty element short, whichever it is; HTML reads only a void one so. Each is
    /// written as HTML reads it empty: `<div/>` as `<div></div>`, `<br/>` as it is.
    #[test]
    fn an_element_written_short_is_read_as_empty() {
        let book = book(&[]);
        let written = Written { chapter: 2, directory: "", zipped: &book, chapters: &HashMap::new() };
        let (body, _) = written.body(&chapter(r#"<div class="gap"/><p>one<a id="note"/><br/>two<img src="a.png"/><span/></p><hr /><td/>"#));
        assert_eq!(
            body,
            concat!(
                r#"<div id="c2" style="page-break-before: always"><div class="gap"></div><p>one<a id="c2-note"></a><br/>two<img src="./a.png"/><span></span></p><hr/><td></td></div>"#,
                "\n"
            )
        );
    }

    /// A spine's item that is neither XHTML nor a picture is shown as the first of its fallbacks
    /// that is, at most eight along; a chain that comes back on itself, or is longer, leaves it
    /// what it was, to be left out. A picture or a drawing in the spine is a page of it.
    #[test]
    fn a_spines_item_is_shown_as_its_fallback_and_a_picture_as_its_page() {
        let thing = |id: &str, fallback: &str| format!(r#"<item id="{id}" href="{id}.bin" media-type="application/x-thing" fallback="{fallback}"/>"#);
        let chain = |length: usize| {
            (0..length).map(|n| thing(&format!("t{n}"), &if n + 1 == length { "one".to_owned() } else { format!("t{}", n + 1) })).collect::<String>()
        };
        let shown = |items: &str, idref: &str| {
            let plan = planned_of(&package("3.0", "", &format!("{CHAPTER_ITEM}{items}"), &format!(r#"<itemref idref="{idref}"/>"#)), &[]).unwrap();
            (plan.chapters[0].path.clone().unwrap(), plan.chapters[0].kind)
        };
        assert_eq!(shown(&chain(1), "t0"), ("OEBPS/text/one.xhtml".into(), Kind::Xhtml));
        assert_eq!(shown(&chain(MAX_FALLBACKS), "t0"), ("OEBPS/text/one.xhtml".into(), Kind::Xhtml), "eight along");
        assert_eq!(shown(&chain(MAX_FALLBACKS + 1), "t0"), ("OEBPS/t0.bin".into(), Kind::Other), "no further");
        assert_eq!(shown(&format!("{}{}", thing("a", "b"), thing("b", "a")), "a"), ("OEBPS/a.bin".into(), Kind::Other), "a loop");
        assert_eq!(shown(&thing("a", "gone"), "a"), ("OEBPS/a.bin".into(), Kind::Other), "a fallback the manifest hasn't");
        let picture = format!(r#"{}<item id="p" href="images/plate.svg" media-type="image/svg+xml"/>"#, thing("a", "p"));
        assert_eq!(shown(&picture, "a"), ("OEBPS/images/plate.svg".into(), Kind::Picture), "a picture is shown too");
        // A page of the picture, where it comes.
        let plan = plan(&[("images/plate one.png", Kind::Picture), ("gone.png", Kind::Picture)]);
        let (html, left) = assembled(&plan, &book(&[("images/plate one.png", b"png")]));
        assert_eq!(
            &html[HEAD as usize..],
            "<div id=\"c1\" style=\"page-break-before: always\"><p><img src=\"./images/plate%20one.png\" alt=\"\"/></p></div>\n"
        );
        assert_eq!((left.shown, left.missing), (1, 1));
    }

    /// What a book's list of encrypted files says is protected: every file encrypted by anything
    /// but a font's obfuscation, IDPF's or Adobe's, by its path from the book's top.
    #[test]
    fn a_file_encrypted_by_anything_but_a_fonts_obfuscation_is_protected() {
        let data = |algorithm: &str, uri: &str| {
            format!(
                r#"<enc:EncryptedData xmlns:enc="http://www.w3.org/2001/04/xmlenc#"><enc:EncryptionMethod Algorithm="{algorithm}"/><ds:KeyInfo xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><enc:EncryptedKey><enc:EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/></enc:EncryptedKey></ds:KeyInfo><enc:CipherData><enc:CipherReference URI="{uri}"/></enc:CipherData></enc:EncryptedData>"#
            )
        };
        const AES: &str = "http://www.w3.org/2001/04/xmlenc#aes256-cbc";
        let xml = format!(
            r#"<?xml version="1.0"?><encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><EncryptedKey><EncryptionMethod Algorithm="rsa"/><CipherData><CipherValue>AAAA</CipherValue></CipherData></EncryptedKey>{}{}{}{}{}<EncryptedData><CipherData><CipherReference URI="OEBPS/unsaid.xhtml"/></CipherData></EncryptedData></encryption>"#,
            data("http://www.idpf.org/2008/embedding", "OEBPS/fonts/a.otf"),
            data("http://ns.adobe.com/pdf/enc#RC", "OEBPS/fonts/b.otf"),
            data(AES, "OEBPS/text/chapter%202.xhtml"),
            data(AES, "OEBPS/images/../images/plate.jpg"),
            data(AES, "../../etc/passwd"),
        );
        assert_eq!(protected(&xml).unwrap(), ["OEBPS/text/chapter 2.xhtml", "OEBPS/images/plate.jpg", "OEBPS/unsaid.xhtml"]);
        assert_eq!(protected("").unwrap(), Vec::<String>::new());
        assert_eq!(protected("<EncryptedData><CipherReference URI='a'/>").unwrap(), Vec::<String>::new(), "one never ended");
        let many: String = (0..=MAX_PROTECTED).map(|n| format!("<EncryptedData><CipherReference URI='{n}'/></EncryptedData>")).collect();
        assert!(many.len() < MAX_ENCRYPTION as usize && protected(&many).is_err_and(|why| why == "it protects more than 16384 files"));
        // Through the worker, with the package; and a list past what is read of one refuses the book.
        let item = r#"<item id="two" href="text/chapter%202.xhtml" media-type="application/xhtml+xml"/>"#;
        let spine = r#"<itemref idref="one"/><itemref idref="two"/>"#;
        let book_of = package("3.0", "", &format!("{CHAPTER_ITEM}{item}"), spine);
        let plan = planned_of(&book_of, &[("META-INF/encryption.xml", xml.as_bytes())]).unwrap();
        assert_eq!(plan.protected, ["OEBPS/images/plate.jpg", "OEBPS/text/chapter 2.xhtml", "OEBPS/unsaid.xhtml"], "sorted, to be looked up");
        assert!(plan.protects("OEBPS/text/chapter 2.xhtml") && !plan.protects("OEBPS/text/one.xhtml") && !plan.protects("OEBPS/fonts/a.otf"));
        let large = vec![b' '; MAX_ENCRYPTION as usize + 1];
        assert_eq!(planned_of(&book_of, &[("META-INF/encryption.xml", &large)]).unwrap_err(), "its list of protected files is more than 1024 KiB");
    }

    /// A protected chapter is left out, and counted for the first page to say; a protected cover is
    /// no page; a protected picture is not read for a chapter; and a book says so when it is made
    /// to be laid out page by page.
    #[test]
    fn a_protected_chapter_is_left_out_and_a_protected_picture_not_read() {
        let (one, two) = (chapter(r#"<p>one <img src="plate.jpg"/></p>"#), chapter("<p>two</p>"));
        let files: [(&str, &[u8]); 4] =
            [("one.xhtml", one.as_bytes()), ("two.xhtml", b"\x00\x9f\x92 not text"), ("plate.jpg", b"jpeg"), ("cover.jpg", b"jpeg")];
        let chapters = [("one.xhtml", Kind::Xhtml), ("two.xhtml", Kind::Xhtml), ("plate.jpg", Kind::Picture)];
        let protecting = |protected: &[&str]| {
            let mut plan = plan(&chapters);
            plan.cover = Some("cover.jpg".to_owned());
            plan.protected = protected.iter().map(|path| (*path).to_owned()).collect();
            plan.protected.sort_unstable();
            plan
        };
        let (html, left) = assembled(&protecting(&["two.xhtml", "plate.jpg", "cover.jpg"]), &book(&files));
        assert_eq!((left.protected, left.shown, left.unreadable), (2, 1, 0));
        assert!(html[HEAD as usize..].starts_with(r#"<div id="c1""#) && !html.contains(r#"id="cover""#), "a protected cover is no page: {html}");
        assert_eq!(left.notes(false), ["2 chapters are protected (DRM)"]);
        assert_eq!(
            Left { protected: 1, ..Left::default() }.notes(true),
            ["1 chapter is protected (DRM)", "a fixed-layout book: its pages are laid out as its text flows, not as they were made"]
        );
        // Nothing of the book shown, and something protected: what `opened` refuses the book by.
        let (_, left) = assembled(&protecting(&["one.xhtml", "two.xhtml", "plate.jpg"]), &book(&files));
        assert_eq!((left.protected, left.shown), (3, 0));
        // A picture the book protects is refused to its chapter, as one it doesn't hold is.
        let kept = Kept::Open(Arc::new(book(&files)));
        let near = near(&kept, &Arc::new(protecting(&["plate.jpg"])), "book.epub");
        assert_eq!(near("plate.jpg").err().as_deref(), Some("book.epub/plate.jpg: protected (DRM)"));
        assert_eq!(near("gone.jpg").err().as_deref(), Some("book.epub/gone.jpg: not in the book"));
        assert_eq!(near("cover.jpg").unwrap().bytes().unwrap(), b"jpeg");
        let _ = two;
        // The package says how the book is laid out.
        let fixed = package("3.0", r#"<meta property="rendition:layout">pre-paginated</meta>"#, CHAPTER_ITEM, r#"<itemref idref="one"/>"#);
        let flowing = package("3.0", r#"<meta property="rendition:layout">reflowable</meta>"#, CHAPTER_ITEM, r#"<itemref idref="one"/>"#);
        assert!(planned_of(&fixed, &[]).unwrap().fixed && !planned_of(&flowing, &[]).unwrap().fixed);
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
            // Framed as an answer is, its counts and its paths whatever they are.
            let mut framed = Vec::new();
            put(&mut framed, &[u8::from(count % 2 == 1)]);
            put(&mut framed, paths.first().map_or(&b""[..], |path| path.as_bytes()));
            put(&mut framed, &count.to_le_bytes());
            for path in paths.iter().take(count as usize) {
                put(&mut framed, path.as_bytes());
                put(&mut framed, b"application/xhtml+xml");
            }
            put(&mut framed, &u32::try_from(paths.len()).unwrap().to_le_bytes());
            for path in &paths {
                put(&mut framed, path.as_bytes());
            }
            let within = |path: &str| !path.split('/').any(|part| part == ".." || part.is_empty());
            match Plan::read(&framed) {
                Some(plan) => {
                    prop_assert_eq!(plan.chapters.len(), count as usize);
                    prop_assert!(plan.chapters.iter().filter_map(|chapter| chapter.path.as_deref()).chain(plan.cover.as_deref()).all(within));
                    prop_assert!(plan.protected.iter().all(|path| within(path)) && plan.protected.is_sorted());
                }
                None => prop_assert!(paths.len() < count as usize, "an answer as the worker writes one is read"),
            }
        }
    }
}
