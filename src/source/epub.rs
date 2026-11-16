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
    Again, Anchor, Ask, Auto, BACKENDS, Backend, Bounded, Bounds, Deferred, Effect, Format, Frame, HEAD, Input, Job, Keys, Landmark, Links, MAX_NAME,
    Named, Near, Numbers, Opened, Pool, Render, Reopenable, Request, Room, Setting, Source, Start, Target, Texts, Told, Value, Values,
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
    does: "draw fenced blocks",
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

/// The largest table of contents read (the nav document, or the NCX), and given to rbook; the most
/// entries kept of it, the longest label of one, in bytes, cut at a character, and how many levels
/// of them are shown, deeper ones at the last.
const MAX_TOC: u64 = 1 << 20;
const MAX_ENTRIES: usize = 4096;
const MAX_LABEL: usize = 256;
const MAX_LEVELS: u8 = 8;

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
        Opened::Pages(pages) => {
            let contents = left.contents.place().map(|name| Landmark { called: "Contents", name });
            Ok(Opened::Pages(Box::new(Book { pages, notes: left.notes(plan.fixed, &plan.contents).into(), contents })))
        }
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
    /// The file that holds the book's table of contents, by its path from the book's top: EPUB 3's
    /// nav document, else EPUB 2's NCX.
    toc: Option<String>,
    /// The table of contents read from it, for the book's Contents page.
    contents: Contents,
    /// The chapter the book marks as its own page of contents, by its path from the book's top: the
    /// nav document itself in the reading order, or the chapter the package's landmarks, or its
    /// guide, call its `toc`.
    own: Option<String>,
}

/// A book's table of contents, as its Contents page is made from it.
#[derive(Debug, Default, PartialEq, Eq)]
enum Contents {
    /// None to show: the book has none.
    #[default]
    None,
    /// Its entries, in order, and whether there were more than are kept.
    Listed { entries: Vec<Entry>, more: bool },
    /// It couldn't be read: the book has no Contents page, and its first page says so.
    Unread,
}

/// An entry of a book's table of contents.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    /// How deep it is nested, from 1.
    depth: u8,
    label: String,
    /// The file it leads to, by its path from the book's top, if it leads anywhere in the book.
    path: Option<String>,
    /// The place it names in that file, as the file calls it.
    fragment: Option<String>,
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
    let mut plan = match read(READ, request) {
        Ok((PLAN, answer)) => Plan::read(&answer).ok_or_else(|| UNREAD.to_owned()),
        Ok((REFUSED, why)) => Err(cut(&String::from_utf8_lossy(&why), MAX_WHY).to_owned()),
        Ok(_) => Err(UNREAD.into()),
        Err(Died::TooMany) => Err("too many files are being read at once to read its package".into()),
        Err(Died::Unstarted(why)) => Err(format!("the process that reads its package couldn't be started ({})", cut(&why, MAX_WHY))),
        Err(_) => Err(UNREAD.into()),
    }?;
    (plan.contents, plan.own) = contents(&plan, zipped, &opf_path, &opf, read);
    Ok(plan)
}

/// The table of contents of the book `plan` orders, for its Contents page: read by a second worker,
/// through `read`, which is given the package again (`path`, `opf`) with the file that holds it. None
/// where the book has none. With it, the chapter the book marks as its own page of contents, if it
/// has one: the nav document itself in the reading order, or the chapter the package's landmarks,
/// or its guide, call its `toc`. Whatever goes wrong costs the Contents page, never the book.
fn contents(plan: &Plan, zipped: &Zipped, path: &str, opf: &[u8], read: Read) -> (Contents, Option<String>) {
    let Some(toc) = plan.toc.as_deref() else { return (Contents::None, None) };
    let chapter = |file: &str| plan.chapters.iter().any(|chapter| chapter.path.as_deref() == Some(file));
    let in_order = chapter(toc).then(|| toc.to_owned());
    let held = zipped.index_of(toc).filter(|_| !plan.protects(toc)).and_then(|index| zipped.within(index, MAX_TOC).ok().flatten());
    let Some(held) = held else { return (Contents::Unread, in_order) };
    let mut request = Vec::new();
    for field in [path.as_bytes(), opf, toc.as_bytes(), &held] {
        put(&mut request, field);
    }
    let Ok((ENTRIES, answer)) = read(CONTENTS, request) else { return (Contents::Unread, in_order) };
    let Some(listed) = Listed::read(&answer) else { return (Contents::Unread, in_order) };
    let own = in_order.or(listed.own.filter(|own| chapter(own)));
    match listed.entries.is_empty() {
        true => (Contents::None, own),
        false => (Contents::Listed { entries: listed.entries, more: listed.more }, own),
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
        // Its end, among its first bytes alone: looked for in all the rest, a run of `&` with no
        // `;` after it took the square of its length to read.
        let end = rest.as_bytes().iter().take(11).position(|byte| *byte == b';');
        let entity = end.and_then(|end| Some((rest.get(1..end)?, end)));
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
/// (from the top, as [`from_the_top`] gives one: no `/` at its end, no empty name, no `.` or `..`)
/// writes it, names: each `.` and empty name dropped, each `..` taken with the name before it, and
/// one starting with `/` taken from the top. `None` for one that leads above the top, or names
/// nothing.
fn from_the_top(directory: &str, reference: &str) -> Option<String> {
    from_the_top_within(directory, reference, usize::MAX)
}

/// [`from_the_top`], of at most `most` bytes: `None` for a longer path too, which isn't put
/// together to be found so. The directory is taken as it is written, its end cut back for each
/// `..` that leads up it, and never apart: a reference costs what it is long, however deep the
/// directory, since a chapter may hold a million of them.
fn from_the_top_within(directory: &str, reference: &str, most: usize) -> Option<String> {
    let mut kept = if reference.starts_with('/') { "" } else { directory };
    let mut names: Vec<&str> = Vec::new();
    for name in reference.split('/') {
        match name {
            "" | "." => {}
            ".." if names.pop().is_some() => {}
            ".." if kept.is_empty() => return None,
            ".." => kept = kept.rsplit_once('/').map_or("", |(above, _)| above),
            name => names.push(name),
        }
    }
    let length = names.iter().fold(kept.len(), |length, name| length.saturating_add(name.len()).saturating_add(1));
    if (kept.is_empty() && names.is_empty()) || length.saturating_sub(usize::from(kept.is_empty())) > most {
        return None;
    }
    let mut path = String::with_capacity(length);
    path.push_str(kept);
    for name in names {
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(name);
    }
    Some(path)
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
    /// The file that holds the table of contents: the nav document's path, else the NCX's.
    toc: Option<String>,
}

/// `field` as text, if it is text of at most `most` bytes.
fn text_within(field: &[u8], most: usize) -> Option<&str> {
    std::str::from_utf8(field).ok().filter(|text| text.len() <= most)
}

/// What the worker reads of a table of contents ([`CONTENTS`]'s answer): its entries, whether
/// there were more than it kept, and the file the package's landmarks, or its guide, call the
/// book's own table of contents.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Listed {
    entries: Vec<Entry>,
    more: bool,
    own: Option<String>,
}

impl Listed {
    /// As the worker writes it: whether there were more entries, the book's own table of contents'
    /// path, how many entries, and each one's depth, label, path and place.
    fn written(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put(&mut out, &[u8::from(self.more)]);
        put(&mut out, self.own.as_deref().unwrap_or_default().as_bytes());
        put(&mut out, &u32::try_from(self.entries.len()).unwrap_or(u32::MAX).to_le_bytes());
        for entry in &self.entries {
            put(&mut out, &[entry.depth]);
            put(&mut out, cut(&entry.label, MAX_LABEL).as_bytes());
            put(&mut out, entry.path.as_deref().unwrap_or_default().as_bytes());
            put(&mut out, entry.fragment.as_deref().unwrap_or_default().as_bytes());
        }
        out
    }

    /// What the worker answered a [`CONTENTS`] with, checked as any input is: its entries within
    /// their count, their labels and places within their lengths, their depths from 1 to
    /// [`MAX_LEVELS`], their paths brought to paths from the book's top. `None` for an answer that
    /// isn't one.
    fn read(answer: &[u8]) -> Option<Self> {
        let mut fields = Fields(answer);
        let text = text_within;
        let path = |field: &[u8]| Some(text(field, MAX_PATH)?).map(|path| from_the_top("", path));
        let more = matches!(fields.next()?, [1]);
        let own = path(fields.next()?)?;
        let count = usize::try_from(u32::from_le_bytes(*fields.next()?.first_chunk::<4>()?)).ok().filter(|count| *count <= MAX_ENTRIES)?;
        let mut entries = Vec::new();
        for _ in 0..count {
            let [depth] = *fields.next()? else { return None };
            let label = text(fields.next()?, MAX_LABEL)?.to_owned();
            let file = path(fields.next()?)?;
            let fragment = Some(text(fields.next()?, MAX_NAME)?).filter(|fragment| !fragment.is_empty()).map(str::to_owned);
            entries.push(Entry { depth: depth.clamp(1, MAX_LEVELS), label, path: file, fragment });
        }
        fields.next().is_none().then_some(Self { entries, more, own })
    }
}

impl Answer {
    /// As the worker writes it: whether the book is laid out page by page, its cover's path, how
    /// many chapters and each chapter's path and media type, how many protected files and each
    /// one's path, then the path of the file that holds its table of contents.
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
        put(&mut out, self.toc.as_deref().unwrap_or_default().as_bytes());
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
        let toc = path(fields.next()?)?;
        fields.next().is_none().then_some(Self { chapters: read, cover, fixed, protected: files, toc, contents: Contents::None, own: None })
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
    /// Where the book's contents are, in it as assembled.
    contents: Leads,
}

/// Where a book's contents are, for the key that goes there ([`Source::contents`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Leads {
    /// Nowhere: the book has no table of contents, and no chapter that looks like one.
    #[default]
    Nowhere,
    /// The chapter of this number, the book's own page of contents, at its start.
    Chapter(usize),
    /// The chapter of this number, the book's own page of contents, at the first of its links to
    /// another chapter ([`first_link`]): a book's first chapter may be its title page, its
    /// contents and its preface, one after the other.
    Links(usize),
    /// The Contents page made here from the book's table of contents, its last page.
    Page,
}

impl Leads {
    /// The place's name in the book's document.
    fn place(self) -> Option<String> {
        match self {
            Self::Nowhere => None,
            Self::Chapter(chapter) => place(chapter, None),
            Self::Links(chapter) => Some(first_link(chapter)),
            Self::Page => Some(CONTENTS_PAGE.to_owned()),
        }
    }
}

/// The id of the Contents page made here: no chapter's, each of whose ids starts with its number
/// ([`place`]).
const CONTENTS_PAGE: &str = "contents";

/// The name of the place before the first link, in the chapter numbered `chapter`, that leads to
/// another chapter: where a page of contents starts. Written in the first [`GUESS_AMONG`] chapters.
fn first_link(chapter: usize) -> String {
    format!("contents-in-c{chapter}")
}

/// How many chapters, from the first of the reading order, may be guessed to be the book's own page
/// of contents, which its package doesn't mark: one is at a book's start. Of 22 books measured, the
/// page was the second or the third chapter of each of the 18 that have one (ADR-0038).
const GUESS_AMONG: usize = 8;
/// How many other chapters such a chapter's links lead to, at least: the pages of contents
/// measured had 12 links to other chapters or more, and no other chapter more than 2.
const GUESS_CHAPTERS: usize = 3;

/// What a chapter's links say of it, for the guess that it is the book's own page of contents:
/// counted as the chapter is written, and no more kept than the guess asks for, however many
/// links a chapter has.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Tally {
    /// Its links (`<a href>`), and those of them that lead to other chapters.
    links: usize,
    away: usize,
    /// The other chapters they lead to, each once, the first [`GUESS_CHAPTERS`] of them.
    chapters: Vec<usize>,
}

impl Tally {
    /// A link met, which leads to the chapter numbered `to`, if to any, from the one numbered
    /// `from`: whether it is the chapter's first to another.
    fn link(&mut self, from: usize, to: Option<usize>) -> bool {
        self.links = self.links.saturating_add(1);
        let Some(to) = to.filter(|to| *to != from) else { return false };
        self.away = self.away.saturating_add(1);
        if self.chapters.len() < GUESS_CHAPTERS && !self.chapters.contains(&to) {
            self.chapters.push(to);
        }
        self.away == 1
    }

    /// Whether the chapter looks like the book's own page of contents: its links lead to
    /// [`GUESS_CHAPTERS`] other chapters or more, and most of its links lead to another chapter
    /// (three in four did, or more, in the pages of contents measured).
    fn lists_the_book(&self) -> bool {
        self.chapters.len() >= GUESS_CHAPTERS && self.away.saturating_mul(2) > self.links
    }
}

impl Left {
    /// What the first page says of the chapters left out, a line for each reason; and of a book
    /// made to be laid out page by page (`fixed`), which this isn't.
    fn notes(&self, fixed: bool, contents: &Contents) -> Vec<String> {
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
            match contents {
                Contents::Unread => Some("its table of contents couldn't be read: it has no Contents page".to_owned()),
                Contents::Listed { more: true, .. } => {
                    Some(format!("its Contents page has the first {MAX_ENTRIES} entries of its table of contents"))
                }
                _ => None,
            },
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
/// shown doesn't show already; after them the Contents page made from the book's table of
/// contents, its last page, so that it moves none of the book's own. And how many chapters were
/// left out, and why, and where the book's contents are.
fn assembled(plan: &Plan, zipped: &Zipped) -> (String, Left) {
    let mut out = start();
    let mut left = Left::default();
    // Each chapter's number in the reading order, from 1, by its file: where a link to it leads.
    let mut numbers: HashMap<&str, usize> = HashMap::new();
    for (number, chapter) in plan.chapters.iter().enumerate() {
        if let Some(path) = chapter.path.as_deref() {
            numbers.entry(path).or_insert(number.saturating_add(1));
        }
    }
    // The cover, as a picture the book holds and may show.
    let cover = plan.cover.as_deref().filter(|cover| zipped.index_of(cover).is_some() && !plan.protects(cover));
    // The Contents page: an entry leads to its chapter where the book has that chapter to show.
    let contents = match &plan.contents {
        Contents::Listed { entries, .. } => {
            let shows = |path: &str| {
                let chapter = plan.chapters.iter().find(|chapter| chapter.path.as_deref() == Some(path));
                chapter.is_some_and(|chapter| chapter.kind != Kind::Other) && zipped.index_of(path).is_some() && !plan.protects(path)
            };
            let leads = |entry: &Entry| {
                let path = entry.path.as_deref().filter(|path| shows(path))?;
                place(numbers.get(path).copied()?, entry.fragment.as_deref())
            };
            contents_page(entries, leads)
        }
        Contents::None | Contents::Unread => String::new(),
    };
    // What the book comes to as it is written: the Contents page, which comes last, is counted
    // first, whole as text, so that the chapters leave it its room.
    let count = |html: &str| html.bytes().filter(|b| matches!(b, b'<' | b'\r' | b'\n')).count();
    let (mut marks, mut text) = (count(&contents), contents.len());
    // The chapter the book marks as its own page of contents, and the first that looks like one.
    let own = plan.own.as_deref().and_then(|own| numbers.get(own).copied());
    let (mut marked, mut guessed) = (None, None);
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
        let (body, body_text, tally) = match chapter.kind {
            Kind::Picture => (page_of(&format!("c{number}"), path), 0, Tally::default()),
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
                // What is left of the book's bytes, the Contents page's kept: the chapter stops
                // being written where it would pass them, however far it would have gone.
                let room = MAX_WRITTEN.saturating_sub(out.len()).saturating_sub(contents.len());
                match written.body(&xhtml, room) {
                    Ok(body) => body,
                    Err(_) => {
                        left.past = plan.chapters.len().saturating_add(1).saturating_sub(number);
                        break;
                    }
                }
            }
        };
        // Before the first chapter shown: the cover, unless that chapter shows it.
        let first = match (left.shown, cover.filter(|cover| !body.contains(&pictured_from_the_top(cover)))) {
            (0, Some(cover)) => page_of("cover", cover),
            _ => String::new(),
        };
        let (marks_now, text_now) = (marks.saturating_add(count(&first)).saturating_add(count(&body)), text.saturating_add(body_text));
        let written = out.len().saturating_add(first.len()).saturating_add(body.len()).saturating_add(contents.len());
        if written > MAX_WRITTEN || marks_now > MAX_MARKS || text_now > MAX_TEXT {
            // With those after it: a book cut short reads to where it is cut.
            left.past = plan.chapters.len().saturating_add(1).saturating_sub(number);
            break;
        }
        (marks, text) = (marks_now, text_now);
        out.push_str(&first);
        out.push_str(&body);
        left.shown = left.shown.saturating_add(1);
        // At its first link to another chapter, where it has one and the place was written.
        let at = if number <= GUESS_AMONG && tally.away > 0 { Leads::Links(number) } else { Leads::Chapter(number) };
        if own == Some(number) {
            marked = Some(at);
        }
        if guessed.is_none() && number <= GUESS_AMONG && tally.lists_the_book() {
            guessed = Some(at);
        }
    }
    // A book none of whose chapters shows has no Contents page either.
    let page = left.shown > 0 && !contents.is_empty();
    if page {
        out.push_str(&contents);
    }
    // The book's own page of contents, as it marks it, else as guessed; else the page made here.
    left.contents = marked.or(guessed).unwrap_or(if page { Leads::Page } else { Leads::Nowhere });
    (out, left)
}

/// The book's Contents page, of `entries`: a heading, a line saying who made the page, since the
/// book didn't, and nested lists as deep as the entries are, each entry its label, as a link to the
/// place `leads` says it leads to in the book's document, or alone where it leads to none.
fn contents_page(entries: &[Entry], leads: impl Fn(&Entry) -> Option<String>) -> String {
    let mut out = format!(
        "<div id=\"{CONTENTS_PAGE}\" style=\"page-break-before: always\"><h1>Contents</h1><p><em>Made by {} from the book's table of contents.</em></p>",
        crate::NAME
    );
    // How many lists are open, each with an item open in it.
    let mut open = 0u8;
    for entry in entries {
        // One level deeper than the entry before at most, so that every list is in an item.
        let depth = entry.depth.clamp(1, MAX_LEVELS).min(open.saturating_add(1));
        while open > depth {
            out.push_str("</li></ul>");
            open = open.saturating_sub(1);
        }
        if open == depth {
            out.push_str("</li>");
        }
        while open < depth {
            out.push_str("<ul>");
            open = open.saturating_add(1);
        }
        out.push_str("<li>");
        // A label is text: what would be markup in it is written so that it is none.
        let label = entry.label.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let label = if label.trim().is_empty() { "\u{2026}".to_owned() } else { label };
        match leads(entry) {
            Some(place) => {
                let _ = write!(out, "<a href=\"#{}\">{label}</a>", escaped(&place));
            }
            None => out.push_str(&label),
        }
    }
    for _ in 0..open {
        out.push_str("</li></ul>");
    }
    out.push_str("</div>\n");
    out
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

/// How far a drawing's `<title>` or `<desc>` is passed over, to see whether the drawing only wraps
/// a picture: what a cover says of itself is a line.
const MAX_LOOK: usize = 4 << 10;
/// How many bytes of a chapter's drawings are looked into for nothing, together: past them, no
/// drawing of the chapter is looked into. A drawing of anything else costs its first tag, so
/// this is thousands of them; only drawings made to be looked into at length spend it.
const MAX_LOOKED: usize = 1 << 20;

/// The picture an `<svg>` wraps, when that is all it holds, as a cover's page writes it
/// (`<svg><image xlink:href="cover.jpg"/></svg>`): its reference as written, and where the
/// `<svg>`, whose content starts at `at` in `xhtml`, ends. Or, for a drawing of anything else, or
/// of two pictures, which is left to be read as it is, how many bytes were looked at to find that
/// out: a `<title>` or a `<desc>` that doesn't end within [`MAX_LOOK`] is no cover's.
fn wrapped_picture(xhtml: &str, start: usize) -> Result<(&str, usize), usize> {
    let (mut picture, mut at) = (None, start);
    let looked = |at: usize| at.saturating_sub(start);
    while let Some((piece, next)) = Piece::at(xhtml, at) {
        at = next;
        match piece {
            Piece::Text(run) if run.trim().is_empty() => {}
            Piece::Other => {}
            Piece::End(name) if name.eq_ignore_ascii_case("svg") => return picture.map(|picture| (picture, at)).ok_or(looked(at)),
            Piece::End(name) if name.eq_ignore_ascii_case("image") => {}
            // What says what the drawing is, to a reader that can't see it: passed over, as far as
            // is looked. In all the rest of the chapter, a thousand drawings whose `<desc>` ends
            // nowhere would each be read to the chapter's end.
            Piece::Start(tag) if (tag.is("title") || tag.is("desc")) && !tag.self_closed => {
                let far = at.saturating_add(MAX_LOOK).min(xhtml.len());
                let Some(near) = (0..=far).rev().find(|end| xhtml.is_char_boundary(*end)).and_then(|end| xhtml.get(..end)) else {
                    return Err(looked(at));
                };
                let past = past_end_tag(near, at, tag.name);
                if past >= near.len() && near.len() < xhtml.len() {
                    return Err(looked(past));
                }
                at = past;
            }
            Piece::Start(tag) if tag.is("image") && picture.is_none() => {
                let href = |name: &str| tag.attributes().find(|attribute| attribute.name.eq_ignore_ascii_case(name)).map(|attribute| attribute.value);
                picture = Some(href("xlink:href").or_else(|| href("href")).filter(|href| !href.trim().is_empty()).ok_or(looked(at))?);
            }
            _ => return Err(looked(at)),
        }
    }
    Err(looked(at))
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
    /// none), in a `<div>` that starts a page; how many bytes of text that is; and what its links say
    /// of it. Within `room` bytes: written again, a chapter may come to many times its size (a
    /// short path is written from the book's top wherever it is referred to), so the writing stops
    /// at the piece that takes it past them, and gives how many bytes it had come to.
    fn body(&self, xhtml: &str, room: usize) -> Result<(String, usize, Tally), usize> {
        let mut out = format!("<div id=\"c{}\" style=\"page-break-before: always\">", self.chapter);
        let (mut text, mut at) = (0usize, 0usize);
        let mut tally = Tally::default();
        // How far its drawings have been looked into for a picture they didn't wrap.
        let mut looked = 0usize;
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
            if out.len() > room {
                return Err(out.len());
            }
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
                    let wrapped = match tag.is("svg") && !tag.self_closed && looked < MAX_LOOKED {
                        true => wrapped_picture(xhtml, at).map_err(|more| looked = looked.saturating_add(more)).ok(),
                        false => None,
                    };
                    match wrapped {
                        Some((picture, past)) => {
                            let _ = write!(out, "<img src=\"{}\" alt=\"\"/>", escaped(&self.pictured(&unescaped(picture))));
                            at = past;
                        }
                        // What it shows as text is text of the chapter's.
                        None => match self.start_tag(&tag, &mut out, &mut tally, room) {
                            Some(shown) => text = text.saturating_add(shown),
                            None => return Err(out.len()),
                        },
                    }
                }
            }
        }
        out.push_str("</div>\n");
        if out.len() > room {
            return Err(out.len());
        }
        Ok((out, text, tally))
    }

    /// The start tag `tag` written again: its ids the book's own, its paths from the book's top. A
    /// link (`<a href>`) is counted in `tally`, and before the first that leads to another chapter,
    /// in a chapter that may be guessed to be the book's page of contents, a place is written, for
    /// the key that goes to the contents ([`first_link`]). An id, a picture's path and a link are
    /// read as the text they are, their entities undone (`a&amp;b.png` is the file `a&b.png`), and
    /// written as text again ([`escaped`]). Gives how many bytes of text the tag shows: a
    /// picture's `alt`, which the importer writes where it has no picture, and no other attribute.
    /// Or nothing, for a tag that takes what is written past `room` bytes, where it stops: one
    /// tag may say an attribute any number of times, each written from the book's top.
    fn start_tag(&self, tag: &StartTag<'_>, out: &mut String, tally: &mut Tally, room: usize) -> Option<usize> {
        let from = out.len();
        let mut text = 0usize;
        out.push('<');
        out.push_str(tag.name);
        for attribute in tag.attributes() {
            if out.len() > room {
                return None;
            }
            let name = attribute.name;
            let is = |attribute: &str| name.eq_ignore_ascii_case(attribute);
            if is("alt") && tag.is("img") {
                text = text.saturating_add(attribute.value.len());
            }
            let rewritten = if is("id") {
                // The book's own: its chapter's number before it. One too long to be a place's
                // name once it has that is left out, and a link to it says so.
                match place(self.chapter, Some(&unescaped(attribute.value))) {
                    Some(id) => Some(id),
                    None => continue,
                }
            } else if is("src") {
                Some(self.pictured(&unescaped(attribute.value)))
            } else if is("href") {
                let (written, chapter) = self.linked(&unescaped(attribute.value));
                if tag.is("a") && tally.link(self.chapter, chapter) && self.chapter <= GUESS_AMONG {
                    out.insert_str(from, &format!("<a id=\"{}\"></a>", first_link(self.chapter)));
                }
                Some(written)
            } else {
                None
            };
            match rewritten {
                Some(value) => {
                    let _ = write!(out, " {name}=\"{}\"", escaped(&value));
                }
                // As the chapter wrote it, entities and all: a value holds no quote of the kind
                // around it, so one that holds the other kind keeps its own.
                None => {
                    let quote = if attribute.value.contains('"') { '\'' } else { '"' };
                    let value = attribute.value.replace('\'', if quote == '\'' { "&#39;" } else { "'" });
                    let _ = write!(out, " {name}={quote}{value}{quote}");
                }
            }
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
        Some(text)
    }

    /// The path `reference` names, from this chapter, as a path from the book's top, and what
    /// follows it (its `?query` and `#fragment`): `None` for what is no path in the book, a web
    /// address or anything else with a scheme, which is left as written. A path longer than any
    /// file's in a book ([`MAX_PATH`]) is none of the book's, as one that leads above its top.
    fn resolved<'r>(&self, reference: &'r str) -> Option<(Option<String>, &'r str)> {
        let reference = reference.trim();
        let end = reference.find(['?', '#']).unwrap_or(reference.len());
        let (path, rest) = reference.split_at_checked(end)?;
        let scheme = path.split_once(':').is_some_and(|(before, _)| !before.contains('/'));
        if scheme || path.starts_with("//") {
            return None;
        }
        Some((percent_decoded(path).and_then(|path| from_the_top_within(self.directory, &path, MAX_PATH)), rest))
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
    /// written from the book's top, so that it is never a path on disk. With it, the number of the
    /// chapter it leads to, if it leads to one.
    fn linked(&self, href: &str) -> (String, Option<usize>) {
        let reference = href.trim();
        let place_in = |chapter: usize, rest: &str| {
            let fragment = rest.split_once('#').map(|(_, fragment)| fragment).filter(|fragment| !fragment.is_empty());
            place(chapter, fragment).map_or_else(|| NO_SUCH_PLACE.to_owned(), |name| format!("#{name}"))
        };
        // A place alone, or a link to nowhere else, is in this chapter: no path is asked about.
        if reference.is_empty() || reference.starts_with(['#', '?']) {
            return (place_in(self.chapter, reference), Some(self.chapter));
        }
        let Some((path, rest)) = self.resolved(reference) else { return (href.to_owned(), None) };
        let chapter = path.as_deref().and_then(|path| self.chapters.get(path).copied());
        let written = match (chapter, path) {
            (Some(chapter), _) => place_in(chapter, rest),
            (None, Some(path)) if self.zipped.index_of(&path).is_some() => format!("{IN_THE_BOOK}{}", percent_encoded(&path)),
            (None, path) => format!("{NOT_IN_THE_BOOK}{}", percent_encoded(path.as_deref().unwrap_or_default())),
        };
        (written, chapter)
    }
}

/// `text` as an attribute's value between double quotes: what would end the value, or start an
/// entity or a tag, written as an entity.
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;")
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
    /// Where its contents are: its own page of them, or the one made here.
    contents: Option<Landmark>,
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
        let (job, notes, contents) = (self.pages.relayout(values, room)?, self.notes.clone(), self.contents.clone());
        Some(Box::new(move || job().map(|pages| Box::new(Book { pages, notes, contents }) as Box<dyn Source + Send>)))
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

    fn contents(&self) -> Option<Landmark> {
        self.contents.clone()
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

/// A book's table of contents, to read: a request too. The package's path and its bytes, then the
/// path and the bytes of the file that holds the table of contents.
const CONTENTS: u8 = 4;

/// Its entries ([`Listed::written`]): an answer.
const ENTRIES: u8 = 5;

/// Why rbook refused it, in its words: an answer.
const REFUSED: u8 = 3;

/// Why a worker answered [`FAILED`]: what it was asked isn't a request.
const MALFORMED: u8 = 1;

/// The CPU seconds a package may take in the worker, and the most its heap holds: what the
/// costliest packages measured take, many times over (ADR-0038). The viewer waits [`WALL`] times
/// as long.
const READ_CPU: u64 = 2;
const MAX_HEAP: usize = 256 << 20;

/// The longest request: a package's path and the package, with the book's list of protected files
/// or with a table of contents and its path ([`MAX_PATH`], [`MAX_OPF`], [`MAX_ENCRYPTION`],
/// [`MAX_TOC`], which is as much), each with its length.
const MAX_REQUEST: usize = 2 * MAX_PATH + (MAX_OPF as usize) + (MAX_ENCRYPTION as usize) + 16;

/// The longest media type kept of a manifest's item, in bytes.
const MAX_MEDIA_TYPE: usize = 255;

/// The longest answer: the cover's path, every chapter's path and media type, and the protected
/// files' paths, which together are no longer than the list they are read from; each with its
/// length, and their counts.
const MAX_ANSWER: usize = 40 + 2 * MAX_PATH + MAX_CHAPTERS * (MAX_PATH + MAX_MEDIA_TYPE + 8) + (MAX_ENCRYPTION as usize) + MAX_PROTECTED * 4;

/// The worker that reads a book's package: `clv --worker epub`.
pub(crate) struct Package;

pub(crate) static PACKAGE: Package = Package;

impl Worker for Package {
    fn name(&self) -> &'static str {
        "epub"
    }

    fn takes(&self, kind: u8) -> Option<(u64, usize)> {
        matches!(kind, READ | CONTENTS).then_some((READ_CPU, MAX_REQUEST))
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
        let mut fields = Fields(&payload);
        let text = |field| Option::and_then(field, |field: &[u8]| std::str::from_utf8(field).ok());
        let answered = match kind {
            READ => {
                let (Some(path), Some(opf), Some(encryption), None) = (text(fields.next()), fields.next(), fields.next(), fields.next()) else {
                    return (FAILED, vec![MALFORMED]);
                };
                std::panic::catch_unwind(|| read(path, opf, encryption).map(|answer| (PLAN, answer.written())))
            }
            CONTENTS => {
                let (Some(path), Some(opf), Some(toc), Some(held), None) =
                    (text(fields.next()), fields.next(), text(fields.next()), fields.next(), fields.next())
                else {
                    return (FAILED, vec![MALFORMED]);
                };
                std::panic::catch_unwind(|| listed(path, opf, toc, held).map(|listed| (ENTRIES, listed.written())))
            }
            _ => return (FAILED, vec![MALFORMED]),
        };
        match answered.unwrap_or_else(|_| Err(UNREAD.into())) {
            Ok(answer) => answer,
            Err(why) => (REFUSED, cut(&why, MAX_WHY).as_bytes().to_vec()),
        }
    }
}

/// A zip of `files`, by name and contents, stored as they are, under a container that names the
/// package at `path`: what rbook is given to read, in memory.
fn package_of(path: &str, files: &[(&str, &[u8])]) -> Result<Cursor<Vec<u8>>, String> {
    let escaped = path.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
    let container = format!(
        "<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile \
         full-path=\"{escaped}\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>"
    );
    let mut zip = ::zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = ::zip::write::SimpleFileOptions::default().compression_method(::zip::CompressionMethod::Stored);
    for (name, bytes) in [("mimetype", MIMETYPE), ("META-INF/container.xml", container.as_bytes())].iter().chain(files) {
        zip.start_file(*name, stored).map_err(|e| e.to_string())?;
        zip.write_all(bytes).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())
}

/// The path from the book's top that `href` leads to, within its limit.
fn located(href: rbook::ebook::element::Href<'_>) -> Result<String, String> {
    let path = href.path().decode().into_owned();
    match path.len() > MAX_PATH {
        true => Err("a file's path is too long".into()),
        false => Ok(path),
    }
}

/// What rbook reads of the table of contents `held`, the file at `toc` in the book whose package
/// is `opf`, at `path`: its entries, in order, each its depth, its label and where it leads, walked
/// with a stack of the worker's own (rbook's own counts of them recurse), as many as are kept; and
/// the file the package's landmarks, or its guide, call the book's table of contents. EPUB 3's nav
/// where the book has one, else EPUB 2's NCX. Or why not.
fn listed(path: &str, opf: &[u8], toc: &str, held: &[u8]) -> Result<Listed, String> {
    let epub = rbook::Epub::options().strict(false).read(package_of(path, &[(path, opf), (toc, held)])?).map_err(|e| e.to_string())?;
    let read = epub.toc();
    let is_toc = |kind: Option<&str>| kind.is_some_and(|kind| kind.split_ascii_whitespace().any(|kind| kind.eq_ignore_ascii_case("toc")));
    let own = read.landmarks().and_then(|landmarks| landmarks.iter().find(|landmark| is_toc(landmark.kind_raw())));
    let own = own.and_then(|landmark| landmark.href()).map(located).transpose()?;
    let Some(contents) = read.contents() else { return Err("it has no table of contents".into()) };
    let mut listed = Listed { own, ..Listed::default() };
    let mut levels = vec![contents.iter()];
    while let Some(level) = levels.last_mut() {
        let Some(entry) = level.next() else {
            levels.pop();
            continue;
        };
        if listed.entries.len() >= MAX_ENTRIES {
            listed.more = true;
            break;
        }
        let depth = u8::try_from(levels.len()).unwrap_or(u8::MAX).clamp(1, MAX_LEVELS);
        let href = entry.href();
        let fragment = href.and_then(|href| href.fragment()).filter(|fragment| !fragment.is_empty() && fragment.len() <= MAX_NAME);
        let path = href.map(located).transpose()?;
        listed.entries.push(Entry { depth, label: cut(entry.label().trim(), MAX_LABEL).to_owned(), path, fragment: fragment.map(str::to_owned) });
        levels.push(entry.iter());
    }
    Ok(listed)
}

/// What rbook reads of the package `opf`, the file at `path` in its book: each item of the spine,
/// in order, as the path from the book's top and the media type of the file shown for it; the
/// cover picture it names; and whether it says the book is laid out page by page. With them, the
/// files `encryption`, the book's list of them, says are protected. Or why not. rbook is given a
/// zip holding the package alone, under a container that names it, and reads neither the table of
/// contents (`skip_toc`: one it can't read fails the whole package) nor any other file.
fn read(path: &str, opf: &[u8], encryption: &[u8]) -> Result<Answer, String> {
    let epub = rbook::Epub::options().strict(false).skip_toc(true).read(package_of(path, &[(path, opf)])?).map_err(|e| e.to_string())?;
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
        chapters.push((Some(located(shown.href())?), shown.media_type().to_owned()));
    }
    if chapters.is_empty() {
        return Err("its package lists no chapters to read".into());
    }
    let manifest = epub.manifest();
    let cover = manifest.cover_image().map(|cover| located(cover.href())).transpose()?;
    let fixed = epub.metadata().by_property("rendition:layout").any(|layout| layout.value().trim() == "pre-paginated");
    // Where its table of contents is: the nav document, which EPUB 3 marks; else the NCX, by its kind.
    let nav = manifest.by_property("nav").next();
    let toc = nav.or_else(|| manifest.iter().find(|item| item.media_type().trim().eq_ignore_ascii_case("application/x-dtbncx+xml")));
    let toc = toc.map(|toc| located(toc.href())).transpose()?;
    Ok(Answer { chapters, cover, fixed, protected: protected(&String::from_utf8_lossy(encryption))?, toc })
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
        // In its turn, as a worker process is asked: the tests run side by side, and only so many
        // workers are alive at once.
        let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
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
        let starts = |body: &str| written.body(&chapter(body), MAX_WRITTEN).unwrap().0.contains(BEFORE_A_TABLE);
        assert!(starts("<table><tr><td>a</td></tr></table>") && starts("\n <div><section>\n<table><tr><td>a</td></tr></table></section></div>"));
        assert!(!starts("<p>first</p><table></table>") && !starts("<div><img src='a.png'/></div><table></table>") && !starts("text<table></table>"));
        assert_eq!(written.body(&chapter("<table></table><table></table>"), MAX_WRITTEN).unwrap().0.matches(BEFORE_A_TABLE).count(), 1);
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
            left.notes(false, &Contents::None),
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
        assert_eq!(
            Left { past: 2, ..Left::default() }.notes(false, &Contents::None),
            ["2 chapters are left out: the book is longer than clv lays out"]
        );
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
            assert_eq!(written.linked(href).0, linked, "{href}");
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
                r##"<div id="c1" style="page-break-before: always"><h1 id="c1-start">One</h1><p><a href="#c1-start">here</a>, <a id="contents-in-c1"></a><a href="#c2-start">there</a>, <a href="#c2">its start</a></p></div>"##,
                "\n",
                r##"<div id="c2" style="page-break-before: always"><h1 id="c2-start">Two</h1><p><a id="contents-in-c2"></a><a href="#c1-start">back</a></p></div>"##,
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
        assert_eq!(written.linked(&format!("#{fits}")).0, format!("#c12-{fits}"));
        assert_eq!(written.linked(&format!("#{long}")).0, NO_SUCH_PLACE);
        let (body, ..) =
            written.body(&chapter(&format!(r##"<p id="{fits}" class="a">one</p><p id="{long}" class="b">two</p>"##)), MAX_WRITTEN).unwrap();
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
            let body = written.body(&chapter(svg), MAX_WRITTEN).unwrap().0;
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
        let (body, ..) =
            written.body(&chapter(r#"<div class="gap"/><p>one<a id="note"/><br/>two<img src="a.png"/><span/></p><hr /><td/>"#), MAX_WRITTEN).unwrap();
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
        assert_eq!(left.notes(false, &Contents::None), ["2 chapters are protected (DRM)"]);
        assert_eq!(
            Left { protected: 1, ..Left::default() }.notes(true, &Contents::None),
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

    /// A nav document whose table of contents is `list`, with `more` beside it.
    fn nav(list: &str, more: &str) -> String {
        format!(
            r#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Contents</title></head><body><nav epub:type="toc"><h1>Contents</h1>{list}</nav>{more}</body></html>"#
        )
    }

    const NAV_ITEM: &str = r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#;
    const TWO_ITEM: &str = r#"<item id="two" href="text/two.xhtml" media-type="application/xhtml+xml"/>"#;
    const BOTH: &str = r#"<itemref idref="one"/><itemref idref="two"/>"#;

    fn entry(depth: u8, label: &str, path: Option<&str>, fragment: Option<&str>) -> Entry {
        Entry { depth, label: label.to_owned(), path: path.map(str::to_owned), fragment: fragment.map(str::to_owned) }
    }

    /// A book's table of contents is read by a second worker, which is given the file that holds
    /// it: EPUB 3's nav document where the book has one, else EPUB 2's NCX. Its entries come in
    /// order, each its depth, its label as text and where it leads, by a path from the book's top.
    #[test]
    fn contents_are_read_from_a_nav_and_from_an_ncx() {
        let list = r##"<ol><li><a href="text/one.xhtml">One &amp; <em>only</em></a><ol><li><a href="text/one.xhtml#s2">One, two</a><ol><li><a href="text/two.xhtml#s3">Deep</a></li></ol></li></ol></li><li><a href="text/gone.xhtml">Gone</a></li><li><span>No link</span></li></ol>"##;
        let epub3 = package("3.0", "", &format!("{CHAPTER_ITEM}{TWO_ITEM}{NAV_ITEM}"), BOTH);
        let plan = planned_of(&epub3, &[("OEBPS/nav.xhtml", nav(list, "").as_bytes())]).unwrap();
        assert_eq!(plan.toc.as_deref(), Some("OEBPS/nav.xhtml"));
        let read = [
            entry(1, "One & only", Some("OEBPS/text/one.xhtml"), None),
            entry(2, "One, two", Some("OEBPS/text/one.xhtml"), Some("s2")),
            entry(3, "Deep", Some("OEBPS/text/two.xhtml"), Some("s3")),
            entry(1, "Gone", Some("OEBPS/text/gone.xhtml"), None),
            entry(1, "No link", None, None),
        ];
        assert_eq!(plan.contents, Contents::Listed { entries: read.to_vec(), more: false });
        // EPUB 2: the NCX the spine names.
        let point = |label: &str, src: &str, inner: &str| {
            format!(r#"<navPoint><navLabel><text>{label}</text></navLabel><content src="{src}"/>{inner}</navPoint>"#)
        };
        let points = format!(
            "{}{}",
            point("One &amp; only", "text/one.xhtml", &point("One, two", "text/one.xhtml#s2", &point("Deep", "text/two.xhtml#s3", ""))),
            point("Gone", "text/gone.xhtml", "")
        );
        let ncx =
            format!(r#"<?xml version="1.0"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><navMap>{points}</navMap></ncx>"#);
        let ncx_item = r#"<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>"#;
        let epub2 = package("2.0", "", &format!("{CHAPTER_ITEM}{TWO_ITEM}{ncx_item}"), BOTH).replace("<spine>", r#"<spine toc="ncx">"#);
        let plan = planned_of(&epub2, &[("OEBPS/toc.ncx", ncx.as_bytes())]).unwrap();
        assert_eq!(plan.toc.as_deref(), Some("OEBPS/toc.ncx"));
        assert_eq!(plan.contents, Contents::Listed { entries: read[..4].to_vec(), more: false });
        // A book with both is read by its nav; one with neither has no table of contents.
        let both = package("3.0", "", &format!("{CHAPTER_ITEM}{TWO_ITEM}{ncx_item}{NAV_ITEM}"), BOTH);
        assert_eq!(planned_of(&both, &[("OEBPS/nav.xhtml", nav(list, "").as_bytes())]).unwrap().toc.as_deref(), Some("OEBPS/nav.xhtml"));
        let neither = planned_of(&package("3.0", "", &format!("{CHAPTER_ITEM}{TWO_ITEM}"), BOTH), &[]).unwrap();
        assert_eq!((neither.toc, neither.contents), (None, Contents::None));
    }

    /// Entries past the 4,096 kept are left out, and said; a level past the eighth is shown at the
    /// eighth; a label past 256 bytes is cut at a character.
    #[test]
    fn contents_are_kept_within_their_limits() {
        let long = "é".repeat(200);
        let many: String =
            (0..=MAX_ENTRIES).map(|n| format!(r#"<li><a href="text/one.xhtml#s{n}">{}</a></li>"#, if n == 0 { &long } else { "x" })).collect();
        let package = package("3.0", "", &format!("{CHAPTER_ITEM}{NAV_ITEM}"), r#"<itemref idref="one"/>"#);
        let plan = planned_of(&package, &[("OEBPS/nav.xhtml", nav(&format!("<ol>{many}</ol>"), "").as_bytes())]).unwrap();
        let Contents::Listed { entries, more } = &plan.contents else { panic!("{:?}", plan.contents) };
        assert_eq!((entries.len(), *more), (MAX_ENTRIES, true));
        assert_eq!((entries[0].label.len(), entries[0].label.chars().count()), (MAX_LABEL, MAX_LABEL / 2), "cut at a character");
        assert_eq!(Left::default().notes(false, &plan.contents), ["its Contents page has the first 4096 entries of its table of contents"]);
        let deep = format!("{}{}", r#"<ol><li><a href="text/one.xhtml">x</a>"#.repeat(12), "</li></ol>".repeat(12));
        let plan = planned_of(&package, &[("OEBPS/nav.xhtml", nav(&deep, "").as_bytes())]).unwrap();
        let Contents::Listed { entries, .. } = &plan.contents else { panic!("{:?}", plan.contents) };
        assert_eq!(entries.iter().map(|entry| entry.depth).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6, 7, 8, 8, 8, 8, 8]);
        // What a worker answers is checked: a depth, a count, a label or a place past its limit.
        let listed = |entries: Vec<Entry>| Listed { entries, more: false, own: None }.written();
        assert_eq!(
            Listed::read(&listed(vec![entry(200, "x", None, None), entry(0, "y", None, None)]))
                .unwrap()
                .entries
                .iter()
                .map(|e| e.depth)
                .collect::<Vec<_>>(),
            [8, 1]
        );
        assert!(Listed::read(&listed(vec![entry(1, "x", None, None); MAX_ENTRIES + 1])).is_none());
        assert!(Listed::read(&listed(vec![entry(1, "x", None, Some(&"a".repeat(MAX_NAME + 1)))])).is_none());
        assert!(Listed::read(&listed(vec![entry(1, "x", Some("../../etc/passwd"), None)])).is_some_and(|read| read.entries[0].path.is_none()));
        assert!(Listed::read(&[]).is_none() && Listed::read(&[0xFF; 40]).is_none());
    }

    /// The chapter a book marks as its own page of contents is known: its nav document in the
    /// reading order, the chapter its landmarks call its `toc`, or EPUB 2's guide. Its table of
    /// contents is read all the same, for the Contents page at the book's end.
    #[test]
    fn a_book_marks_its_own_page_of_contents() {
        let list = r#"<ol><li><a href="text/one.xhtml">One</a></li></ol>"#;
        let container = container("OEBPS/content.opf");
        let planned_with = |package: &str, nav: &str| {
            let files =
                [("META-INF/container.xml", container.as_bytes()), ("OEBPS/content.opf", package.as_bytes()), ("OEBPS/nav.xhtml", nav.as_bytes())];
            let plan = planned(&book(&files), in_a_thread).unwrap();
            assert!(matches!(plan.contents, Contents::Listed { .. }), "{:?}", plan.contents);
            plan.own
        };
        let in_spine = package("3.0", "", &format!("{CHAPTER_ITEM}{NAV_ITEM}"), r#"<itemref idref="nav"/><itemref idref="one"/>"#);
        assert_eq!(planned_with(&in_spine, &nav(list, "")).as_deref(), Some("OEBPS/nav.xhtml"));
        let apart = package("3.0", "", &format!("{CHAPTER_ITEM}{NAV_ITEM}"), r#"<itemref idref="one"/>"#);
        assert_eq!(planned_with(&apart, &nav(list, "")), None);
        let landmarks = |kind: &str, href: &str| {
            format!(r#"<nav epub:type="landmarks"><ol><li><a epub:type="{kind}" href="{href}">Table of contents</a></li></ol></nav>"#)
        };
        assert_eq!(planned_with(&apart, &nav(list, &landmarks("toc", "text/one.xhtml#contents"))).as_deref(), Some("OEBPS/text/one.xhtml"));
        assert_eq!(planned_with(&apart, &nav(list, &landmarks("bodymatter", "text/one.xhtml"))), None);
        assert_eq!(planned_with(&apart, &nav(list, &landmarks("toc", "text/not-a-chapter.xhtml"))), None, "no chapter of the book");
        // The nav document in the reading order comes before what the landmarks say.
        assert_eq!(planned_with(&in_spine, &nav(list, &landmarks("toc", "text/one.xhtml"))).as_deref(), Some("OEBPS/nav.xhtml"));
        // EPUB 2's guide says the same.
        let ncx = r#"<?xml version="1.0"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><navMap><navPoint><navLabel><text>One</text></navLabel><content src="text/one.xhtml"/></navPoint></navMap></ncx>"#;
        let ncx_item = r#"<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>"#;
        let guided = |guide: &str| {
            let package = package("2.0", "", &format!("{CHAPTER_ITEM}{ncx_item}"), r#"<itemref idref="one"/>"#)
                .replace("<spine>", r#"<spine toc="ncx">"#)
                .replace("</package>", &format!("{guide}</package>"));
            let plan = planned_of(&package, &[("OEBPS/toc.ncx", ncx.as_bytes())]).unwrap();
            assert!(matches!(plan.contents, Contents::Listed { .. }), "{:?}", plan.contents);
            plan.own
        };
        assert_eq!(
            guided(r#"<guide><reference type="toc" title="Contents" href="text/one.xhtml#contents"/></guide>"#).as_deref(),
            Some("OEBPS/text/one.xhtml")
        );
        assert_eq!(guided(r#"<guide><reference type="cover" title="Cover" href="text/one.xhtml"/></guide>"#), None);
        assert_eq!(guided(""), None);
    }

    /// A table of contents that can't be read costs the book its Contents page, never the book:
    /// the file missing, protected, or past what is read of one; one rbook refuses; a worker that
    /// dies on it, or answers what is no answer. The first page says so.
    #[test]
    fn contents_that_cant_be_read_leave_the_book_whole() {
        fn dies(kind: u8, payload: Vec<u8>) -> Result<(u8, Vec<u8>), Died> {
            if kind == CONTENTS { Err(Died::Memory) } else { in_a_thread(kind, payload) }
        }
        fn answers_otherwise(kind: u8, payload: Vec<u8>) -> Result<(u8, Vec<u8>), Died> {
            if kind == CONTENTS { Ok((ENTRIES, vec![0xFF; 64])) } else { in_a_thread(kind, payload) }
        }
        fn refuses(kind: u8, payload: Vec<u8>) -> Result<(u8, Vec<u8>), Died> {
            if kind == CONTENTS { Ok((REFUSED, b"rbook's reason".to_vec())) } else { in_a_thread(kind, payload) }
        }
        let package = package("3.0", "", &format!("{CHAPTER_ITEM}{NAV_ITEM}"), r#"<itemref idref="one"/>"#);
        let container = container("OEBPS/content.opf");
        let unread = |more: &[(&str, &[u8])], read: Read| {
            let mut files = vec![("META-INF/container.xml", container.as_bytes()), ("OEBPS/content.opf", package.as_bytes())];
            files.extend_from_slice(more);
            let plan = planned(&book(&files), read).expect("the book is read");
            assert_eq!(paths(&plan), [Some("OEBPS/text/one.xhtml")]);
            plan.contents
        };
        let good = nav(r#"<ol><li><a href="text/one.xhtml">One</a></li></ol>"#, "");
        let held: [(&str, &[u8]); 1] = [("OEBPS/nav.xhtml", good.as_bytes())];
        assert!(matches!(unread(&held, in_a_thread), Contents::Listed { .. }));
        for read in [dies as Read, answers_otherwise, refuses] {
            assert_eq!(unread(&held, read), Contents::Unread);
        }
        assert_eq!(unread(&[], in_a_thread), Contents::Unread, "the file it names isn't in the book");
        // One that could be read, but for its size: past what is read of one, by a byte.
        let large = format!("{good}{}", " ".repeat(MAX_TOC as usize + 1 - good.len()));
        assert_eq!(unread(&[("OEBPS/nav.xhtml", large.as_bytes())], in_a_thread), Contents::Unread);
        let within = format!("{good}{}", " ".repeat(MAX_TOC as usize - good.len()));
        assert!(matches!(unread(&[("OEBPS/nav.xhtml", within.as_bytes())], in_a_thread), Contents::Listed { .. }));
        assert_eq!(
            unread(&[("OEBPS/nav.xhtml", b"<html><body><nav epub:type=\"toc\"><ol><li>")], in_a_thread),
            Contents::Unread,
            "not a table of contents"
        );
        assert_eq!(unread(&[("OEBPS/nav.xhtml", b"<html><body><p>no nav here</p></body></html>")], in_a_thread), Contents::Unread);
        let protecting = br#"<encryption><EncryptedData><EncryptionMethod Algorithm="aes"/><CipherData><CipherReference URI="OEBPS/nav.xhtml"/></CipherData></EncryptedData></encryption>"#;
        assert_eq!(unread(&[held[0], ("META-INF/encryption.xml", protecting)], in_a_thread), Contents::Unread);
        assert_eq!(Left::default().notes(false, &Contents::Unread), ["its table of contents couldn't be read: it has no Contents page"]);
    }

    /// The Contents page is the book's last, after its chapters, so that it moves none of them: a
    /// heading, a line saying clv made it, and lists nested as the entries are, each its label,
    /// markup in it no markup, as a link to its place in the book; an entry that leads to no
    /// chapter the book shows is its label alone.
    #[test]
    fn contents_are_a_page_of_links_at_the_books_end() {
        let (one, two) = (chapter(r#"<h1 id="s1">One</h1>"#), chapter("<h1>Two</h1>"));
        let files: [(&str, &[u8]); 4] =
            [("text/one.xhtml", one.as_bytes()), ("text/two.xhtml", two.as_bytes()), ("cover.jpg", b"jpeg"), ("notes.txt", b"x")];
        let mut plan = plan(&[("text/one.xhtml", Kind::Xhtml), ("text/two.xhtml", Kind::Xhtml), ("text/gone.xhtml", Kind::Xhtml)]);
        plan.cover = Some("cover.jpg".to_owned());
        plan.contents = Contents::Listed {
            entries: vec![
                entry(1, "One <b>&</b> only", Some("text/one.xhtml"), None),
                entry(2, "Its start", Some("text/one.xhtml"), Some("s1")),
                entry(3, "Deeper", Some("text/two.xhtml"), Some("x")),
                entry(2, "Missing", Some("text/gone.xhtml"), None),
                entry(1, "Not a chapter", Some("notes.txt"), None),
                entry(1, "Nowhere", None, None),
                entry(1, " ", Some("text/two.xhtml"), None),
            ],
            more: false,
        };
        let (html, left) = assembled(&plan, &book(&files));
        let page = concat!(
            r##"<div id="contents" style="page-break-before: always"><h1>Contents</h1><p><em>Made by clv from the book's table of contents.</em></p><ul>"##,
            r##"<li><a href="#c1">One &lt;b&gt;&amp;&lt;/b&gt; only</a><ul><li><a href="#c1-s1">Its start</a><ul><li><a href="#c2-x">Deeper</a></li></ul></li><li>Missing</li></ul></li>"##,
            r##"<li>Not a chapter</li><li>Nowhere</li><li><a href="#c2">…</a></li></ul></div>"##,
            "\n",
        );
        let text = &html[HEAD as usize..];
        // The cover, the chapters, then the page, the book's last: none of the book's own is moved.
        let (cover, first) = (text.find(r#"<div id="cover""#).unwrap(), text.find(r#"<div id="c1""#).unwrap());
        let (second, contents) = (text.find(r#"<div id="c2""#).unwrap(), text.find(page).expect(text));
        assert!(cover == 0 && cover < first && first < second && second < contents && text.ends_with(page), "{text}");
        assert_eq!((left.shown, left.missing, left.contents), (2, 1, Leads::Page));
        assert_eq!(left.contents.place().as_deref(), Some("contents"));
        // A book with no chapter shown has none, and nowhere for the key to go.
        let (html, left) = assembled(&plan, &book(&[]));
        assert_eq!((html.len(), left.contents), (HEAD as usize, Leads::Nowhere));
        // The chapters leave the page its room: one that would fit without it is where the book is cut.
        let long = chapter(&format!("<p>{}</p>", "a".repeat(MAX_TEXT - page.len() / 2)));
        let filling: [(&str, &[u8]); 2] = [("text/one.xhtml", one.as_bytes()), ("text/two.xhtml", long.as_bytes())];
        let (html, left) = assembled(&plan, &book(&filling));
        assert!((left.shown, left.past) == (1, 2) && html.ends_with(page) && !html.contains("aaaa"), "{left:?}");
        // Nor has a book with no table of contents.
        plan.contents = Contents::None;
        let (html, left) = assembled(&plan, &book(&filling));
        assert!((left.shown, left.past) == (2, 0) && html.contains("aaaa"), "{left:?}");
        let (html, left) = assembled(&plan, &book(&files));
        assert!(!html.contains("Contents") && left.contents == Leads::Nowhere && left.contents.place().is_none());
        // Any depths, in any order, make lists that close.
        let odd = [entry(3, "a", None, None), entry(1, "b", None, None), entry(8, "c", None, None), entry(2, "d", None, None)];
        let page = contents_page(&odd, |_| None);
        assert!(page.ends_with("table of contents.</em></p><ul><li>a</li><li>b<ul><li>c</li><li>d</li></ul></li></ul></div>\n"), "{page}");
    }

    /// A book of `bodies`, a chapter each, named `1.xhtml` on: assembled, with `own` (a chapter's
    /// number) marked as its page of contents, and a table of contents if `listed`.
    fn led(bodies: &[String], own: Option<usize>, listed: bool) -> (String, Leads) {
        let named: Vec<String> = (1..=bodies.len()).map(|n| format!("{n}.xhtml")).collect();
        let chapters: Vec<String> = bodies.iter().map(|body| chapter(body)).collect();
        let files: Vec<(&str, &[u8])> = named.iter().zip(&chapters).map(|(name, chapter)| (name.as_str(), chapter.as_bytes())).collect();
        let order: Vec<(&str, Kind)> = named.iter().map(|name| (name.as_str(), Kind::Xhtml)).collect();
        let mut plan = plan(&order);
        plan.own = own.map(|number| format!("{number}.xhtml"));
        if listed {
            plan.contents = Contents::Listed { entries: vec![entry(1, "One", Some("1.xhtml"), None)], more: false };
        }
        let (html, left) = assembled(&plan, &book(&files));
        (html, left.contents)
    }

    /// Links to the chapters numbered `to`, a paragraph each.
    fn links_to(to: &[usize]) -> String {
        to.iter().map(|number| format!(r#"<p><a href="{number}.xhtml">Chapter {number}</a></p>"#)).collect()
    }

    /// The key that goes to the contents goes to the book's own page of them: the chapter it
    /// marks, or else one of its first chapters whose links are mostly to other chapters, three
    /// of them or more, at the first of those links. Only with neither, to the Contents page made
    /// here; and nowhere in a book with none of the three.
    #[test]
    fn the_contents_are_the_books_own_page_marked_or_guessed_else_the_page_made_here() {
        let text = |number: usize| format!("<p>Chapter {number}</p>");
        let plain: Vec<String> = (1..=10).map(text).collect();
        let with = |number: usize, body: String| {
            let mut bodies = plain.clone();
            bodies[number - 1] = body;
            bodies
        };
        // Neither marked nor guessed: the page made here, or nowhere.
        assert_eq!((led(&plain, None, true).1, led(&plain, None, false).1), (Leads::Page, Leads::Nowhere));
        // Marked: its start, or its first link to another chapter, where a place is written.
        assert_eq!(led(&plain, Some(2), true).1, Leads::Chapter(2));
        assert_eq!(Leads::Chapter(2).place().as_deref(), Some("c2"));
        let (html, leads) = led(&with(2, format!("<h1>A title</h1>{}", links_to(&[3]))), Some(2), true);
        assert_eq!((leads, leads.place().as_deref()), (Leads::Links(2), Some("contents-in-c2")));
        assert!(html.contains(r##"<h1>A title</h1><p><a id="contents-in-c2"></a><a href="#c3">Chapter 3</a></p>"##), "{html}");
        // Guessed, as Project Gutenberg's EPUB 3 books need: a title page, then the chapters listed.
        let lists = format!(r#"<h1>A title</h1><p><a href="https://example.org/">a library</a></p>{}"#, links_to(&[3, 4, 5]));
        let (html, leads) = led(&with(2, lists.clone()), None, true);
        assert_eq!(leads, Leads::Links(2));
        assert_eq!(html.matches(r#"<a id="contents-in-c2"></a>"#).count(), 1, "one place, at the first of them");
        assert_eq!(led(&with(2, lists.clone()), None, false).1, Leads::Links(2), "with no table of contents too");
        // The first that looks like one, and what the book marks before any guess.
        let both = with(3, links_to(&[1, 2, 4, 5, 6]));
        assert_eq!(led(&both, None, true).1, Leads::Links(3));
        let mut two = both.clone();
        two[4] = links_to(&[6, 7, 8]);
        assert_eq!(
            (led(&two, None, true).1, led(&two, Some(5), true).1, led(&two, Some(7), true).1),
            (Leads::Links(3), Leads::Links(5), Leads::Chapter(7))
        );
        // A few links between chapters make no page of contents: to two other chapters, to one
        // however often, or among more links that lead elsewhere.
        let few = with(2, links_to(&[3, 4]));
        let one = with(2, links_to(&[3, 3, 3, 3]));
        let elsewhere =
            format!("{}{}", links_to(&[3, 4, 5]), r##"<p><a href="https://example.org/">a</a> <a href="#x">b</a> <a href="2.xhtml">c</a></p>"##);
        for not in [few, one, with(2, elsewhere)] {
            assert_eq!(led(&not, None, true).1, Leads::Page, "{:?}", not[1]);
        }
        // Nor is one looked for past a book's first chapters, where no place is written either.
        let (html, leads) = led(&with(GUESS_AMONG + 1, links_to(&[1, 2, 3])), None, true);
        assert!(leads == Leads::Page && !html.contains("contents-in-c"), "{leads:?}");
        assert_eq!(led(&with(GUESS_AMONG + 1, links_to(&[1, 2, 3])), Some(GUESS_AMONG + 1), true).1, Leads::Chapter(GUESS_AMONG + 1));
        assert_eq!(led(&with(GUESS_AMONG, links_to(&[1, 2, 3])), None, true).1, Leads::Links(GUESS_AMONG));
        // A page of contents the book marks and doesn't show is none.
        let mut bodies = plain.clone();
        bodies.truncate(3);
        assert_eq!((led(&bodies, Some(9), true).1, led(&bodies, Some(9), false).1), (Leads::Page, Leads::Nowhere));
    }

    /// A chapter written again may come to many times its size: a path is written from the book's
    /// top wherever the chapter refers to it. It stops being written at the piece that takes it
    /// past the room the book has left, not once it is whole, and the book is cut there.
    #[test]
    fn a_chapter_stops_being_written_where_it_passes_the_books_room() {
        let (numbers, none) = (HashMap::new(), book(&[]));
        let deep = "d/".repeat(2_000);
        let written = Written { chapter: 1, directory: deep.trim_end_matches('/'), zipped: &none, chapters: &numbers };
        let pictures = chapter(&r#"<img src="x"/>"#.repeat(20_000));
        assert!(pictures.len() < 300 << 10);
        let whole = written.body(&pictures, usize::MAX).unwrap().0.len();
        assert!(whole > 80_000_000, "{whole}");
        let room = 1 << 20;
        let stopped = written.body(&pictures, room).unwrap_err();
        assert!(stopped > room && stopped <= room + 2 * MAX_PATH, "stopped at {stopped}, a piece past {room}");
        assert!(written.body(&pictures, whole).is_ok() && written.body(&pictures, whole - 1).is_err());
        // In a book: cut at that chapter, with the one after it, before the book is past its bytes.
        let (short, name) = (chapter("<p>short</p>"), format!("{deep}two.xhtml"));
        let files: [(&str, &[u8]); 3] = [("one.xhtml", short.as_bytes()), (&name, pictures.as_bytes()), ("three.xhtml", short.as_bytes())];
        let (html, left) = assembled(&plan(&[("one.xhtml", Kind::Xhtml), (&name, Kind::Xhtml), ("three.xhtml", Kind::Xhtml)]), &book(&files));
        assert_eq!(left, Left { shown: 1, past: 2, ..Left::default() });
        assert!(html.len() < 1 << 20 && html.contains("short"), "{}", html.len());
        // A path longer than any file's in a book is none of the book's, however it is escaped.
        let long = "a/".repeat(MAX_PATH);
        let top = Written { chapter: 1, directory: "", zipped: &none, chapters: &numbers };
        assert_eq!((top.pictured(&long).as_str(), top.linked(&long).0.as_str()), ("../above-the-book", NOT_IN_THE_BOOK));
        let within = format!("{}b", "a/".repeat(MAX_PATH / 2 - 1));
        assert_eq!((within.len(), top.pictured(&within)), (MAX_PATH - 1, format!("./{within}")));
    }

    /// A picture's `alt` is text of its chapter, which the importer writes where it has no picture:
    /// counted with the chapter's text, so that a book whose alt text would pass what a document
    /// may hold is cut at that chapter, where it would be refused whole. No other attribute is.
    #[test]
    fn a_pictures_alt_is_text_of_its_chapter() {
        let (numbers, none) = (HashMap::new(), book(&[]));
        let written = Written { chapter: 1, directory: "", zipped: &none, chapters: &numbers };
        let text = |body: &str| written.body(&chapter(body), MAX_WRITTEN).unwrap().1;
        let plain = text(r#"<p>ab<img src="a.png"/></p>"#);
        assert_eq!(text(r#"<p>ab<img src="a.png" alt="four"/></p>"#), plain + 4);
        assert_eq!(text(r#"<p>ab<img src="a.png" ALT="four"/><IMG alt='two' src="b.png"></p>"#), plain + 7);
        assert_eq!(text(r#"<p title="none">ab<img src="a.png" title="none"/><input alt="none"/><area alt="none"/></p>"#), plain);
        let (short, long) = (chapter("<p>short</p>"), chapter(&format!(r#"<p><img src="a.png" alt="{}"/></p>"#, "a".repeat(MAX_TEXT))));
        let files: [(&str, &[u8]); 3] = [("one.xhtml", short.as_bytes()), ("two.xhtml", long.as_bytes()), ("three.xhtml", short.as_bytes())];
        let (html, left) = assembled(&plan(&[("one.xhtml", Kind::Xhtml), ("two.xhtml", Kind::Xhtml), ("three.xhtml", Kind::Xhtml)]), &book(&files));
        assert_eq!(left, Left { shown: 1, past: 2, ..Left::default() });
        assert!(html.contains("short") && !html.contains("aaaa"));
    }

    /// An id, a picture's path and a link are read as the text they are, their entities undone,
    /// whichever way each was written, and written as text again: `a&amp;b.png` is the file
    /// `a&b.png`, and what would end an attribute or start an entity is an entity. What is no
    /// entity stays as it is, and any other attribute is as the chapter wrote it.
    #[test]
    fn references_are_read_as_their_text_and_written_as_text_again() {
        let numbers: HashMap<&str, usize> = [("text/one.xhtml", 1), ("text/two.xhtml", 2)].into();
        let book = book(&[("text/a&b.png", b"png")]);
        let written = Written { chapter: 1, directory: "text", zipped: &book, chapters: &numbers };
        let (body, ..) = written
            .body(
                &chapter(concat!(
                    r##"<p id="x&amp;y"><img src="a&amp;b.png"/><img src="a&#38;b.png"/><img src="a&#x26;b.png"/>"##,
                    r##"<a href="two.xhtml#p&amp;q">a</a><a href="#x&#x26;y">b</a>"##,
                    r##"<a href="https://example.org/?a=1&amp;b=&quot;2&quot;&lt;3">c</a>"##,
                    r##"<img src="a&bogus;b&amp.png"/><a title="kept &amp; as &quot;written&quot;" class='its "own" quotes' href="x&amp;y.txt">d</a>"##,
                    r##"<a href="&amp;amp;.xhtml">e</a></p>"##,
                )),
                MAX_WRITTEN,
            )
            .unwrap();
        for part in [
            r##"<p id="c1-x&amp;y">"##,
            r##"<img src="./text/a%26b.png"/><img src="./text/a%26b.png"/><img src="./text/a%26b.png"/>"##,
            r##"<a href="#c2-p&amp;q">a</a>"##,
            r##"<a href="#c1-x&amp;y">b</a>"##,
            r##"<a href="https://example.org/?a=1&amp;b=&quot;2&quot;&lt;3">c</a>"##,
            r##"<img src="./text/a%26bogus;b%26amp.png"/>"##,
            r##"<a title="kept &amp; as &quot;written&quot;" class='its "own" quotes' href="clv-not-in-the-book/text/x%26y.txt">d</a>"##,
            r##"<a href="clv-not-in-the-book/text/%26amp;.xhtml">e</a>"##,
        ] {
            assert!(body.contains(part), "{part} in {body}");
        }
        // A cover's drawing that wraps a picture, and an entry of the Contents page, the same.
        let (cover, ..) = written.body(&chapter(r#"<svg><image xlink:href="a&amp;b.png"/></svg>"#), MAX_WRITTEN).unwrap();
        assert!(cover.contains(r#"<img src="./text/a%26b.png" alt=""/>"#), "{cover}");
        let page = contents_page(&[entry(1, "One", Some("text/one.xhtml"), Some("x\"y<z&"))], |entry| place(1, entry.fragment.as_deref()));
        assert!(page.contains(r##"<a href="#c1-x&quot;y&lt;z&amp;">One</a>"##), "{page}");
    }

    /// One tag stops being written where it passes the book's room, as a chapter does: an
    /// attribute said 20,000 times in one tag, each written from the book's top, is no more whole
    /// than 20,000 tags are.
    #[test]
    fn a_tag_stops_being_written_where_it_passes_the_books_room() {
        let (numbers, none) = (HashMap::new(), book(&[]));
        let deep = "d/".repeat(2_000);
        let written = Written { chapter: 1, directory: deep.trim_end_matches('/'), zipped: &none, chapters: &numbers };
        let tag = chapter(&format!("<img {}/>", r#"src="x" "#.repeat(20_000)));
        assert!(tag.len() < 200 << 10);
        let room = 1 << 20;
        let stopped = written.body(&tag, room).unwrap_err();
        assert!(stopped > room && stopped <= room + 2 * MAX_PATH, "stopped at {stopped}, an attribute past {room}");
        let links = chapter(&format!("<a {}>x</a>", r#"href="y.xhtml" "#.repeat(20_000)));
        let stopped = written.body(&links, room).unwrap_err();
        assert!(stopped > room && stopped <= room + 4 * MAX_PATH, "stopped at {stopped}");
    }

    /// What is read of a chapter takes time as its length, however it is made: entities that end
    /// nowhere, and drawings that each say what they are at a length that ends nowhere near.
    #[test]
    fn a_chapter_made_to_be_read_slowly_isnt() {
        // Read as the square of their length, each of these took tens of seconds.
        let within = |what: &str, since: std::time::Instant| assert!(since.elapsed() < Duration::from_secs(3), "{what}: {:?}", since.elapsed());
        let since = std::time::Instant::now();
        let ampersands = "&".repeat(1 << 20);
        assert_eq!(unescaped(&ampersands), ampersands);
        within("entities that end nowhere", since);
        let far = format!("{}&amp;{}&#x41;&é;&ééééé;", "&".repeat(1 << 16), "&#".repeat(1 << 16));
        assert_eq!(unescaped(&far).len(), (3 << 16) + 2 + 4 + 12);
        // An entity's end is looked for in bytes, which may stop in the middle of a character.
        assert_eq!(unescaped("&aéééééé;&lt;"), "&aéééééé;<");
        let (numbers, none) = (HashMap::new(), book(&[]));
        let written = Written { chapter: 1, directory: "", zipped: &none, chapters: &numbers };
        // A drawing's `<desc>` is passed over to see whether the drawing only wraps a picture: each
        // of these ends at the chapter's end, 80,000 times over.
        let drawings = chapter(&format!("{}</desc>", "<svg><desc>".repeat(80_000)));
        let since = std::time::Instant::now();
        let (body, ..) = written.body(&drawings, MAX_WRITTEN).unwrap();
        within("drawings that say what they are to the chapter's end", since);
        assert_eq!(body.matches("<svg>").count(), 80_000);
    }

    /// A reference costs what it is long, not what its chapter's directory is deep: a place in the
    /// chapter itself (`#x`) asks nothing of any path, and a path is found from the directory as it
    /// is written, with nothing of it taken apart for each. Under a directory 2,000 deep, 800,000
    /// of each in one tag are read in the time they take at the book's top; they took 20 s.
    #[test]
    fn a_reference_costs_nothing_for_its_chapters_depth() {
        let deep = "d/".repeat(2_000);
        let (here, there) = (format!("{deep}one.xhtml"), format!("{deep}two.xhtml"));
        let numbers: HashMap<&str, usize> = [(here.as_str(), 1), (there.as_str(), 2)].into();
        let none = book(&[]);
        let written = Written { chapter: 1, directory: deep.trim_end_matches('/'), zipped: &none, chapters: &numbers };
        let many = |attribute: &str| chapter(&format!("<a {}>x</a>", attribute.repeat(800_000)));
        for (attribute, to, most) in
            [(r##"href="#x" "##, r##" href="#c1-x""##, 3), ("href=\"\" ", r##" href="#c1""##, 3), (r#"href="two.xhtml" "#, r##" href="#c2""##, 3)]
        {
            let links = many(attribute);
            let since = std::time::Instant::now();
            let (body, ..) = written.body(&links, MAX_WRITTEN).unwrap();
            let took = since.elapsed();
            assert_eq!(body.matches(to).count(), 800_000, "{attribute}");
            assert!(took < Duration::from_secs(most), "{attribute}: {took:?}");
        }
        // What a path is found to be is what it was: from the directory, up it, from the top.
        let from = |directory: &str, reference: &str| from_the_top(directory, reference);
        assert_eq!(from("a/b", "c/./d//e").as_deref(), Some("a/b/c/d/e"));
        assert_eq!((from("a/b", "../c").as_deref(), from("a/b", "../../c").as_deref(), from("a/b", "../../../c")), (Some("a/c"), Some("c"), None));
        assert_eq!(
            (from("a/b", "c/../../d").as_deref(), from("a/b", "/c").as_deref(), from("a/b", "..").as_deref()),
            (Some("a/d"), Some("c"), Some("a"))
        );
        assert_eq!((from("a/b", "").as_deref(), from("", ""), from("a", ".."), from("", "x/..")), (Some("a/b"), None, None, None));
    }

    /// A drawing is looked into only so far for the picture it may wrap, and a chapter's drawings
    /// only so far together: past that, a drawing that wraps a picture stays a drawing. Drawings
    /// of anything else cost a glance each, and any number of them leave the next one its look.
    #[test]
    fn a_drawing_is_looked_into_only_so_far_for_its_picture() {
        let (numbers, none) = (HashMap::new(), book(&[]));
        let written = Written { chapter: 1, directory: "", zipped: &none, chapters: &numbers };
        let wrapped = r#"<svg><title>A cover</title><image xlink:href="cover.jpg"/></svg>"#;
        let picture = r#"<img src="./cover.jpg" alt=""/>"#;
        let body = |xhtml: &str| written.body(&chapter(xhtml), MAX_WRITTEN).unwrap().0;
        assert_eq!(body(&wrapped.repeat(300)).matches(picture).count(), 300, "each of a chapter's wrapped pictures");
        let drawn = r#"<svg><path d="M0 0"/></svg>"#.repeat(50_000);
        assert_eq!(body(&format!("{drawn}{wrapped}")).matches(picture).count(), 1, "after any number of drawings");
        // One that says what it is at more length than is looked into stays a drawing.
        let long = format!(r#"<svg><desc>{}</desc><image xlink:href="cover.jpg"/></svg>"#, "a".repeat(MAX_LOOK));
        assert_eq!(body(&long).matches(picture).count(), 0);
        // And once a chapter's drawings have been looked into that far together, none is.
        let spent = format!("{}{}</desc>{wrapped}", "<svg><desc>".repeat(MAX_LOOKED / MAX_LOOK + 1), "a".repeat(MAX_LOOK));
        assert_eq!(body(&spent).matches(picture).count(), 0);
    }

    /// The guess costs a chapter nothing that grows with it: its links are counted as they are
    /// written, the chapters they lead to kept only as far as the guess asks, and one place written.
    #[test]
    fn a_chapter_of_links_costs_the_guess_no_more_than_a_few_of_them() {
        let names: Vec<String> = (1..=MAX_CHAPTERS).map(|n| format!("{n}.xhtml")).collect();
        let numbers: HashMap<&str, usize> = names.iter().enumerate().map(|(n, name)| (name.as_str(), n + 1)).collect();
        let book = book(&[]);
        let written = Written { chapter: 1, directory: "", zipped: &book, chapters: &numbers };
        let links: String = (0..100_000).map(|n| format!(r#"<a href="{}.xhtml">x</a>"#, n % MAX_CHAPTERS + 1)).collect();
        let (body, _, tally) = written.body(&chapter(&links), MAX_WRITTEN).unwrap();
        // One in 4096 leads to this chapter itself.
        let own = 100_000usize.div_ceil(MAX_CHAPTERS);
        assert_eq!(tally, Tally { links: 100_000, away: 100_000 - own, chapters: vec![2, 3, 4] });
        assert!(tally.lists_the_book() && tally.chapters.capacity() <= 8);
        assert_eq!(body.matches("contents-in-c1").count(), 1);
        assert!(body.contains(r##"<a href="#c1">x</a><a id="contents-in-c1"></a><a href="#c2">x</a>"##));
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

    /// A path from the top as it is plainly found: the directory's names and the reference's, one
    /// by one, as [`from_the_top`] found it before it stopped taking the directory apart.
    fn plainly_from_the_top(directory: &str, reference: &str) -> Option<String> {
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

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// Whatever the directory and the reference, the path found is the one found plainly, name
        /// by name: `..` up the directory and above the top, `.` and empty names, a path from the
        /// top, names past ASCII, and a `/` or a `.` written `%2F` or `%2E`, which is one once the
        /// reference is read. Within a length, it is that path where it is no longer; and a
        /// chapter's reference is found the same.
        #[test]
        fn a_path_from_the_top_is_the_one_found_name_by_name(
            directory in prop::collection::vec("[a-cé]{1,3}", 0..6),
            reference in r"(/|//|\.\./|\./|\.\.|\.|[a-cé]{1,2}|%2F|%2f|%2E|%C3%A9|%){0,12}",
            most in 0usize..24,
        ) {
            let directory = directory.join("/");
            for reference in [reference.clone(), percent_decoded(&reference).unwrap_or_default()] {
                let plainly = plainly_from_the_top(&directory, &reference);
                prop_assert_eq!(from_the_top(&directory, &reference), plainly.clone(), "{} {}", directory, reference);
                prop_assert_eq!(from_the_top_within(&directory, &reference, most), plainly.filter(|path| path.len() <= most), "{} {}", directory, reference);
            }
            let (numbers, none) = (HashMap::new(), book(&[]));
            let written = Written { chapter: 1, directory: &directory, zipped: &none, chapters: &numbers };
            let read = percent_decoded(&reference).and_then(|read| plainly_from_the_top(&directory, &read));
            let found = if reference.starts_with("//") { None } else { Some((read, "")) };
            prop_assert_eq!(written.resolved(&reference), found, "{} {}", directory, reference);
        }
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
            put(&mut framed, paths.last().map_or(&b""[..], |path| path.as_bytes()));
            let within = |path: &str| !path.split('/').any(|part| part == ".." || part.is_empty());
            match Plan::read(&framed) {
                Some(plan) => {
                    prop_assert_eq!(plan.chapters.len(), count as usize);
                    prop_assert!(plan.chapters.iter().filter_map(|chapter| chapter.path.as_deref()).chain(plan.cover.as_deref()).chain(plan.toc.as_deref()).all(within));
                    prop_assert!(plan.protected.iter().all(|path| within(path)) && plan.protected.is_sorted());
                }
                None => prop_assert!(paths.len() < count as usize, "an answer as the worker writes one is read"),
            }
        }
    }
}
