//! What the viewer shows: a source of pages, and the backends that open and render them. The
//! pager and the printer in `view` see only the traits here; each backend brings its own renderer
//! and its own threads, so one can cancel or time out work the way it knows how.

mod dir;
mod folder;
#[cfg(feature = "sheet")]
mod gridcraft;
#[cfg(any(feature = "word", feature = "sheet"))]
mod head;
#[cfg(feature = "image")]
mod image;
mod joined;
mod later;
#[cfg(feature = "mermaid")]
mod mermaid;
#[cfg(feature = "pdf")]
mod pdfcraft;
#[cfg(feature = "word")]
mod picture;
#[cfg(feature = "svg")]
mod svg;
#[cfg(feature = "tar")]
mod tar;
#[cfg(any(feature = "pdf", feature = "word"))]
mod texts;
mod threads;
#[cfg(any(feature = "zip", feature = "tar"))]
mod tree;
#[cfg(feature = "word")]
mod wordcraft;
mod worker;
#[cfg(feature = "zip")]
mod zip;

use std::fs::File;
use std::io::{Read, Seek};
use std::ops::{Deref, Range};
use std::rc::Rc;
use std::sync::Arc;

use crate::args::{OptionSpec, Parsed};
pub(crate) use dir::Directory;
pub(crate) use folder::{Badge, Folder};
pub(crate) use joined::{Joined, Part};
pub(crate) use later::{Handle, Later, Openers};
pub(crate) use worker::{Worker, serve};

/// The backends this build has, in the order they are asked whether they open a file. SVG
/// first: it goes by how a file starts, where a PDF's header may be anywhere in the first
/// 1024 bytes, which a drawing's text may hold. Mermaid diagrams next, for the same reason, and
/// tar, for the same reason: a tar's first file starts 512 bytes in. Then documents, for the same reason (an RTF's text), and spreadsheets,
/// both before zip, since a docx and an xlsx are zips.
pub(crate) const BACKENDS: &[&dyn Backend] = &[
    #[cfg(feature = "svg")]
    &svg::Svg,
    #[cfg(feature = "mermaid")]
    &mermaid::Mermaid,
    #[cfg(feature = "tar")]
    &tar::Tar,
    #[cfg(feature = "word")]
    &wordcraft::Wordcraft,
    #[cfg(feature = "sheet")]
    &gridcraft::Gridcraft,
    #[cfg(feature = "pdf")]
    &pdfcraft::Pdfcraft,
    #[cfg(feature = "image")]
    &image::Images,
    #[cfg(feature = "zip")]
    &zip::Zip,
];

/// The worker processes this build has, by name: the work a backend does in a process of its own
/// ([`Worker`]), clv run again as `clv --worker <name>`.
pub(crate) const WORKERS: &[&'static dyn Worker] = &[
    #[cfg(feature = "svg-html")]
    &svg::LABELS,
    #[cfg(feature = "pdf")]
    &pdfcraft::TEXT,
    #[cfg(feature = "mermaid")]
    &mermaid::DIAGRAMS,
];

/// How much of a file a backend sees to decide whether it opens it: a PDF's header may come
/// anywhere in its first 1024 bytes.
const HEAD: u64 = 1024;

/// A kind of file the viewer shows, and how it opens one. On any thread: a list's files are opened
/// off the thread that shows them ([`Later`]).
pub(crate) trait Backend: Sync {
    /// Its name, as messages give it.
    fn name(&self) -> &'static str;
    /// What it shows, in a word or two, for `--help` and for a file no backend opens: "PDFs",
    /// "documents". Which formats, [`Backend::formats`] says.
    fn shows(&self) -> &'static str;
    /// The formats it shows, for `--help`: taken from the tables [`Backend::opens`] goes by, so
    /// that the two can't disagree.
    fn formats(&self) -> Vec<Format>;
    /// The libraries it reads and draws its files through, by their crates' names, for `--help` to
    /// credit. None for a backend that does it all itself.
    fn credits(&self) -> &[&'static str];
    /// The options it takes, beyond the viewer's own.
    fn options(&self) -> &'static [OptionSpec];
    /// The settings it declares: what the reader may change while its files are shown, with keys of
    /// their own, and which [`Source::settings`] says a file has. None by default.
    fn settings(&self) -> &'static [Setting] {
        &[]
    }
    /// Whether it opens the file at `path`, going by `head`: its first [`HEAD`] bytes, or all of
    /// a shorter file.
    fn opens(&self, path: &str, head: &[u8]) -> bool;
    /// `input` opened, with the values `options` gives its own options: its pages, or for a file
    /// that holds files (an archive), a container of them. A backend that lays a file out for the
    /// room it is shown in ([`Backend::open_at`]) takes a room of its own here.
    fn open(&self, input: Input, options: &Parsed<'_>) -> Result<Opened, String>;
    /// [`Backend::open`], for a file shown in `room`: the viewer opens every file through this, and
    /// only a backend that lays a file out for the room it is shown in (a document with no page of
    /// its own) looks at it; the rest open as they do without one.
    fn open_at(&self, input: Input, options: &Parsed<'_>, _room: Room) -> Result<Opened, String> {
        self.open(input, options)
    }
    /// What a list may know of the file at `path`, which starts with `head`, without opening it: for
    /// a backend whose [`Backend::open`] costs more than a list of hundreds should spend on files
    /// the reader may never come to, and which then opens to pages, never to a container. `None`
    /// opens it as the list is listed: it costs little, or it may hold files.
    fn deferred(&self, _path: &str, _head: &[u8]) -> Option<Deferred> {
        None
    }
    /// Whether a document may show what it opens as a picture: its first page, drawn by this backend.
    /// Not what may hold other files or refer to pictures of its own, so that a document can't come to
    /// draw itself.
    #[cfg_attr(not(feature = "word"), expect(dead_code, reason = "only documents refer to pictures"))]
    fn draws_pictures(&self) -> bool {
        false
    }
}

/// What a page is laid out for, when a document has no page of its own: the pages area of the
/// window, all of it the viewer's to say and none of it a document's. The area's columns and rows
/// (the window less the status line's row), a cell's width and height in pixels, exactly the cell
/// the pages are fitted with (for text cells a sampling cell, 1.25 × 2 for a 10 × 16 terminal cell:
/// rounding it would change its shape), the cells between two pages side by side, and how many pages
/// the reader asked to have to a row of it (`--per-row`, `,` and `.`), as the grid draws them: `None`
/// at `auto`, where the document decides (one, or two as a book). A room is input like any other: a
/// backend checks what it computes from one. Rooms are only ever compared through [`Room::same`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Room {
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    pub(crate) cell_w: f32,
    pub(crate) cell_h: f32,
    pub(crate) gap: u16,
    pub(crate) per_row: Option<u16>,
}

/// How far two cells may differ, as a share of either side, and still be the same room: a window
/// dragged by a few pixels changes a cell's size by a fraction, and lays nothing out again.
const SAME_CELL: f32 = 0.02;

impl Room {
    /// Whether `other` is the same room: the same columns, rows, gap and pages to a row, and a cell within
    /// [`SAME_CELL`] of this one's, width and height each. A font change in the terminal, which
    /// changes the cell by more and the columns and rows with it, isn't.
    pub(crate) fn same(&self, other: &Self) -> bool {
        let near = |a: f32, b: f32| (a - b).abs() <= a.abs().max(b.abs()) * SAME_CELL;
        self.cols == other.cols
            && self.rows == other.rows
            && self.gap == other.gap
            && self.per_row == other.per_row
            && near(self.cell_w, other.cell_w)
            && near(self.cell_h, other.cell_h)
    }
}

/// Where a [`Setting`]'s values sit for a document in a room ([`Source::bounds`]): both for that
/// room and the values the document is laid out with, so they change when either does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Bounds {
    /// Where the setting's named value sits among its numbers (a page as wide as the window, at
    /// the characters its line holds), so that stepping up from it goes on to the next number and
    /// the steps pass through it. `None` for a named value that sits nowhere.
    pub(crate) named_at: Option<u32>,
    /// The most a number is laid out at here (the longest line the room holds): a number past it
    /// is kept, and laid out at it.
    pub(crate) most: Option<u32>,
}

/// What a list knows of a file it hasn't opened ([`Backend::deferred`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Deferred {
    /// The settings it has ([`Source::settings`]).
    pub(crate) settings: &'static [&'static str],
}

/// Something the reader may change while a file is shown, which its backend declares
/// ([`Backend::settings`]), as it declares its options: Markdown on dark paper. The viewer holds one
/// value of it for the whole run, steps it by its keys, and sends it with every render
/// ([`Request::values`]); the backend decides what it draws from it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Setting {
    /// What it is called, in [`Values`] and in what the reader is told (`width 72`).
    pub(crate) name: &'static str,
    /// Its named values, in the order its [`Keys::Cycle`] goes through them.
    pub(crate) named: &'static [Named],
    /// The numbers it takes too, if any.
    pub(crate) numbers: Option<Numbers>,
    /// Where it starts, unless its option says otherwise; and with `auto`, where a terminal that
    /// doesn't say its background starts.
    pub(crate) default: Value,
    /// Where a count of 0 before one of its keys goes (`0]`): back to the window's width, to 100%.
    /// Only a setting with numbers takes a count, so only there does it matter.
    pub(crate) zero: Value,
    /// Where it starts by the terminal's background, if it does: then its option takes `auto` too.
    pub(crate) auto: Option<Auto>,
    /// Its keys in the pager.
    pub(crate) keys: Keys,
    /// The command-line option that sets it, taken whatever the files are.
    pub(crate) option: OptionSpec,
    /// What its keys do, as `?` lists them.
    pub(crate) help: &'static str,
    /// What its keys say on a file that hasn't it.
    pub(crate) missing: &'static str,
    /// What a change of its value takes.
    pub(crate) effect: Effect,
    /// Whether its numbers say how wide a page is, as a count of pages to a row does on a document laid
    /// out for the room ([`Room::per_row`]): then the last of the two the reader sets wins. A count sets
    /// it back to its default, and a number of it, typed or stepped to, sets the count back to `auto`.
    pub(crate) sets_width: bool,
}

/// What a change of a [`Setting`]'s value takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    /// The pages drawn again with it, wherever they are shown, a list's tiles too: a request
    /// carries it ([`Request::values`]).
    Redraw,
    /// The document on screen laid out again at it, as a new source ([`Source::relayout`]); a
    /// list's tiles keep the layout they were opened with, and a request's value does nothing.
    Relayout,
}

/// A named value of a [`Setting`], and what the reader is told on going to it (`light on dark`).
/// Where it sits among the setting's numbers, if it does, depends on the room a document is shown
/// in, so the document says ([`Source::bounds`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Named {
    pub(crate) name: &'static str,
    pub(crate) says: &'static str,
}

/// The numbers a [`Setting`] takes: from `least` to `most`, its keys stepping to the multiples of
/// `step`, each said with `unit` after it (`%`; nothing for a count of characters).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Numbers {
    pub(crate) least: u32,
    pub(crate) most: u32,
    pub(crate) step: u32,
    pub(crate) unit: &'static str,
}

/// Which of a [`Setting`]'s named values it starts at by the terminal's background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Auto {
    pub(crate) light: &'static str,
    pub(crate) dark: &'static str,
}

/// A [`Setting`]'s keys in the pager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Keys {
    /// One, going to the next named value, after the last the first.
    #[cfg_attr(not(any(feature = "word", test)), expect(dead_code, reason = "only documents have a setting"))]
    Cycle(char),
    /// Two, going a step down and a step up among the numbers, and the named value set among them
    /// ([`Bounds::named_at`]).
    #[cfg_attr(not(any(feature = "word", test)), expect(dead_code, reason = "only documents have a setting with numbers"))]
    Step(char, char),
    /// The viewer's zoom keys, `+` and `=` up, `-` and `_` down, stepping among the numbers as
    /// [`Keys::Step`] does, with a count setting the number: a font size. They go to the setting only
    /// on a document that has it, shown by itself without `--pages`; everywhere else they zoom, as
    /// they always did. The one place a setting takes the viewer's keys
    /// ([ADR-0023](../docs/adr/0023-the-window-is-the-page-of-markdown-and-html.md)). Named by the
    /// viewer, which asks a setting whether it is on these keys, so live in every build.
    Zoom,
}

impl Keys {
    /// The keys, in the order they are pressed down and up.
    pub(crate) fn chars(self) -> Vec<char> {
        match self {
            Self::Cycle(key) => vec![key],
            Self::Step(down, up) => vec![down, up],
            Self::Zoom => vec!['-', '_', '+', '='],
        }
    }

    /// Whether `key` steps up (`true`), down (`false`), or isn't one of these keys.
    pub(crate) fn up(self, key: char) -> Option<bool> {
        match self {
            Self::Cycle(_) => None,
            Self::Step(down, up) => (key == up).then_some(true).or((key == down).then_some(false)),
            Self::Zoom => matches!(key, '+' | '=').then_some(true).or(matches!(key, '-' | '_').then_some(false)),
        }
    }
}

/// A [`Setting`]'s value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    Named(&'static str),
    Number(u32),
}

/// The most settings a build may declare: [`Values`] holds as many, to be `Copy` as a request is.
pub(crate) const MAX_SETTINGS: usize = 8;

/// The value of each setting, by its name, as a render is asked for with them. One not in it is at
/// its default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Values([Option<(&'static str, Value)>; MAX_SETTINGS]);

impl Values {
    /// Setting `name`'s value, if it has one here.
    pub(crate) fn get(&self, name: &str) -> Option<Value> {
        self.0.iter().flatten().find(|(named, _)| *named == name).map(|(_, value)| *value)
    }

    /// Setting `name` to `value`: whether there was room, [`MAX_SETTINGS`] in all.
    pub(crate) fn set(&mut self, name: &'static str, value: Value) -> bool {
        let slot = match self.0.iter().position(|slot| slot.is_some_and(|(named, _)| named == name)) {
            Some(at) => self.0.get_mut(at),
            None => self.0.iter_mut().find(|slot| slot.is_none()),
        };
        slot.map(|slot| *slot = Some((name, value))).is_some()
    }
}

/// A format a backend shows, as `--help` lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Format {
    /// What it is called: "PDF", "Word".
    pub(crate) name: &'static str,
    /// The extensions that tell a file of it where its first bytes can't (a TGA has no
    /// signature; a compound file may as well be a workbook), in lowercase, without the dot.
    /// Empty for a format known by how a file starts alone.
    pub(crate) extensions: &'static [&'static str],
    /// The languages a document's fenced block of it is written in, in lowercase: ` ```mermaid ` in
    /// Markdown, `<pre><code class="language-mermaid">` in HTML, which a document then shows as a
    /// picture of the block rather than its code ([`fenced`]). Matched without regard to case against
    /// the first word of the block's info string (` ```Mermaid title="x" ` is `mermaid`). Listed,
    /// never taken from [`extensions`](Format::extensions) nor from how the text starts: a fence's
    /// name may differ from an extension (` ```dot `, `.gv`), and a block of a format that lists none
    /// stays code (` ```svg `). Empty for most formats: only a backend that
    /// [draws pictures](Backend::draws_pictures) lists any, and no two list the same.
    ///
    /// A backend that lists one is opened with the block's text straight through [`Backend::open`],
    /// never asked whether it [opens](Backend::opens) it, under a name ending in its format's first
    /// extension. It is opened every time the document is read: to size the block, again for the
    /// first render, again for every relayout and every time the picture is let go of and drawn
    /// again. So it keeps what it made of a text, as Mermaid's backend keeps each diagram's SVG, and
    /// answers the same text again from that.
    pub(crate) fences: &'static [&'static str],
}

impl Format {
    /// The format called `name`, which goes by `extensions`, and lists no fence.
    #[cfg_attr(
        not(any(
            feature = "pdf",
            feature = "image",
            feature = "svg",
            feature = "mermaid",
            feature = "zip",
            feature = "tar",
            feature = "word",
            feature = "sheet",
            test
        )),
        expect(dead_code, reason = "only a backend lists formats")
    )]
    pub(crate) const fn of(name: &'static str, extensions: &'static [&'static str]) -> Self {
        Self { name, extensions, fences: &[] }
    }

    /// This format, a document's fenced blocks in the languages `fences` drawn as pictures of it
    /// ([`Format::fences`]).
    #[cfg_attr(not(any(feature = "mermaid", test)), expect(dead_code, reason = "only diagrams are written in fenced blocks"))]
    pub(crate) const fn fenced(self, fences: &'static [&'static str]) -> Self {
        Self { fences, ..self }
    }

    /// Whether `path` ends in one of its [`extensions`](Format::extensions), in any case.
    #[cfg_attr(
        not(any(feature = "svg", feature = "word", feature = "sheet", test)),
        expect(dead_code, reason = "only drawings, documents and spreadsheets go by a name")
    )]
    pub(crate) fn named(&self, path: &str) -> bool {
        let extension = std::path::Path::new(path).extension().and_then(|e| e.to_str());
        extension.is_some_and(|e| self.extensions.iter().any(|x| e.eq_ignore_ascii_case(x)))
    }
}

/// What a backend opened a file to.
pub(crate) enum Opened {
    #[cfg_attr(
        not(any(feature = "pdf", feature = "image", feature = "svg", feature = "word", feature = "sheet", test)),
        expect(dead_code, reason = "only PDFs, images, drawings, documents and spreadsheets have pages")
    )]
    Pages(Box<dyn Source + Send>),
    #[cfg_attr(not(any(feature = "zip", feature = "tar", test)), expect(dead_code, reason = "only archives hold files"))]
    Files(Rc<dyn Container>),
}

/// A file being opened: its first bytes, read to choose a backend, and the rest still to read. A
/// file is read once, so a pipe (`/dev/stdin`) opens too.
pub(crate) struct Input {
    pub(crate) path: String,
    head: Vec<u8>,
    rest: Rest,
}

/// The rest of a file being opened.
enum Rest {
    /// A file of its own, read on from its head.
    File(File),
    /// A file held in another (an archive's), read whole, again, from there; and read only so far, if
    /// the holder can stop early ([`Bounded`]).
    Held(Again, Option<Bounded>),
}

/// A file held in another, read whole from it, every time it is called: within the holder's
/// limits, and on any thread, since a render reads it.
pub(crate) type Again = Arc<dyn Fn() -> Result<Vec<u8>, String> + Send + Sync>;

/// A file held in another, read from it to a bound of the reader's own: the whole file if it is no
/// more than that many bytes, else `None`, having unpacked no more than a byte past them, so a reader
/// that wants a small file doesn't unpack a large one whole. Within the holder's limits too.
pub(crate) type Bounded = Arc<dyn Fn(u64) -> Result<Option<Vec<u8>>, String> + Send + Sync>;

impl Input {
    fn open(path: &str) -> Result<Self, String> {
        let error = |e: std::io::Error| format!("{path}: {e}");
        let mut rest = File::open(path).map_err(error)?;
        let mut head = Vec::new();
        Read::by_ref(&mut rest).take(HEAD).read_to_end(&mut head).map_err(error)?;
        Ok(Self { path: path.to_owned(), head, rest: Rest::File(rest) })
    }

    /// The file at `path` in another, which starts with `head` (its first [`HEAD`] bytes, or all
    /// of a shorter file) and which `again` reads whole.
    fn held(path: String, head: Vec<u8>, again: Again) -> Self {
        Self { path, head, rest: Rest::Held(again, None) }
    }

    /// [`Input::held`], for a holder that can stop reading the file at a bound ([`Bounded`]).
    #[cfg_attr(
        not(any(feature = "zip", feature = "tar", feature = "word", test)),
        expect(dead_code, reason = "only archives, and a document's fenced blocks, stop early")
    )]
    fn held_bounded(path: String, head: Vec<u8>, again: Again, bounded: Bounded) -> Self {
        Self { path, head, rest: Rest::Held(again, Some(bounded)) }
    }

    /// The whole file.
    pub(crate) fn bytes(self) -> Result<Vec<u8>, String> {
        match self.rest {
            Rest::File(mut rest) => {
                let mut head = self.head;
                rest.read_to_end(&mut head).map_err(|e| format!("{}: {e}", self.path))?;
                Ok(head)
            }
            Rest::Held(again, _) => again().map_err(|e| format!("{}: {e}", self.path)),
        }
    }

    /// The whole file, if it is no more than `most` bytes, else `None`: reading stops past them, so
    /// a pipe or a device is never read whole, nor an archive's file unpacked whole where its holder
    /// can stop early ([`Bounded`]); otherwise, a file held in another is read within the holder's
    /// limits.
    #[cfg_attr(not(any(feature = "mermaid", test)), expect(dead_code, reason = "only diagrams are read so"))]
    pub(crate) fn at_most(self, most: u64) -> Result<Option<Vec<u8>>, String> {
        let bytes = match self.rest {
            Rest::File(rest) => {
                let mut bytes = self.head;
                let left = most.saturating_add(1).saturating_sub(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
                rest.take(left).read_to_end(&mut bytes).map_err(|e| format!("{}: {e}", self.path))?;
                bytes
            }
            Rest::Held(_, Some(bounded)) => match bounded(most).map_err(|e| format!("{}: {e}", self.path))? {
                Some(bytes) => bytes,
                None => return Ok(None),
            },
            Rest::Held(again, None) => again().map_err(|e| format!("{}: {e}", self.path))?,
        };
        Ok((u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= most).then_some(bytes))
    }

    /// The file for a backend that reads it again when it renders, and so keeps only what it
    /// needs meanwhile: on disk, from its start; held in another, how to read it from there;
    /// otherwise the whole of it, read now.
    #[cfg_attr(
        all(
            not(feature = "image"),
            not(feature = "svg"),
            not(feature = "zip"),
            not(feature = "tar"),
            not(feature = "word"),
            not(feature = "sheet"),
            not(test)
        ),
        expect(dead_code, reason = "only images, drawings, documents and archives read their file again")
    )]
    pub(crate) fn reopenable(self) -> Result<Reopenable, String> {
        let Self { path, head, rest } = self;
        match rest {
            Rest::File(mut file) if file.metadata().is_ok_and(|m| m.is_file()) => {
                file.rewind().map_err(|e| format!("{path}: {e}"))?;
                Ok(Reopenable::Disk(file))
            }
            Rest::File(file) => Self { path, head, rest: Rest::File(file) }.bytes().map(Reopenable::Bytes),
            Rest::Held(again, _) => Ok(Reopenable::Again(again)),
        }
    }
}

/// A file as [`Input::reopenable`] gives it.
#[cfg_attr(
    not(any(feature = "image", feature = "svg", feature = "zip", feature = "tar", feature = "word", feature = "sheet")),
    expect(dead_code, reason = "only images, drawings, documents and archives read their file again")
)]
pub(crate) enum Reopenable {
    /// On disk, from its start: its path opens it again.
    Disk(File),
    /// A pipe or a device, which can't be read again: all of it.
    Bytes(Vec<u8>),
    /// Held in another: how to read it whole from there.
    Again(Again),
}

/// `path` opened, with the first of `backends` that opens it.
pub(crate) fn choose<'b>(path: &str, backends: &[&'b dyn Backend]) -> Result<(&'b dyn Backend, Input), String> {
    if backends.is_empty() {
        return Err(format!("{path}: this {} was built without backends (cargo features such as `pdf`), so it shows nothing", crate::NAME));
    }
    let input = Input::open(path)?;
    Ok((pick(backends, path, &input.head)?, input))
}

/// `input`, with the first of `backends` that opens it, or `None` when none does: for a
/// container's files, where what nothing shows is no error.
pub(crate) fn shown_by<'b>(input: Input, backends: &[&'b dyn Backend]) -> Option<(&'b dyn Backend, Input)> {
    let backend = backends.iter().find(|backend| backend.opens(&input.path, &input.head)).copied();
    backend.map(|backend| (backend, input))
}

/// The first of `backends` that draws a fenced block written in `language` as a picture, and the
/// format of its that lists it ([`Format::fences`]), in any case; `None` for a language none lists,
/// whose blocks stay code. Only a backend that [draws pictures](Backend::draws_pictures) is asked.
#[cfg_attr(not(any(feature = "word", test)), expect(dead_code, reason = "only documents have fenced blocks"))]
pub(crate) fn fenced<'b>(language: &str, backends: &[&'b dyn Backend]) -> Option<(&'b dyn Backend, Format)> {
    backends.iter().filter(|backend| backend.draws_pictures()).find_map(|backend| {
        let lists = |format: &Format| format.fences.iter().any(|fence| fence.eq_ignore_ascii_case(language));
        backend.formats().into_iter().find(lists).map(|format| (*backend, format))
    })
}

/// Files held together, shown as a list: a directory's. Each file is opened from it, and each
/// container it holds is listed from that one, when the reader goes into it.
pub(crate) trait Container {
    /// Where it is, as messages give it: a directory's path.
    fn path(&self) -> &str;
    /// What it holds to show, in order: its files and the containers in it, or with `recurse`,
    /// those containers' files at their place instead.
    fn list(&self, recurse: bool) -> Result<Listing, String>;
    /// The file it listed as `name`, opened.
    fn open(&self, name: &str) -> Result<Input, String>;
    /// What its folder carries, saying what kind of container it is: nothing, for a directory.
    fn badge(&self) -> Badge {
        Badge::None
    }
    /// Whether it lists without waiting, or how far it has got reading itself first: a compressed
    /// tar is read through, start to end, to find its files. The first call starts that, on a
    /// thread of its own; [`Container::release`] stops it, and [`Container::list`] waits for it.
    fn ready(&self) -> Ready {
        Ready::Now
    }
    /// Let go of all that reading it keeps (a compressed tar's unpacked copy and decompressor), a
    /// reading of itself under way stopped: the reader left it, or stopped going into it. Asked
    /// again, it reads itself again.
    fn release(&self) {}
}

/// How far a container has got reading itself before it lists ([`Container::ready`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ready {
    /// It lists without waiting.
    Now,
    /// Still reading: `read` bytes of `of`, when it knows how many it has.
    #[cfg_attr(not(feature = "tar"), expect(dead_code, reason = "only compressed tars read themselves first"))]
    Reading { read: u64, of: Option<u64> },
}

impl std::fmt::Debug for dyn Container + '_ {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Container({:?})", self.path())
    }
}

/// What a container holds to show.
#[derive(Debug, Default)]
pub(crate) struct Listing {
    /// Each file, and each container in it unless recursing, in order.
    pub(crate) entries: Vec<Listed>,
    /// What couldn't be read, by its name in the container, and why: a subdirectory, a name that
    /// isn't UTF-8.
    pub(crate) unread: Vec<(String, String)>,
    /// Whether the listing stopped at one of the container's limits before the end.
    pub(crate) cut: bool,
    /// The order its entries are quickest opened in, by their place in `entries`, when that isn't
    /// the order they are listed in: a compressed tar's, as its files come in it. Empty otherwise.
    pub(crate) quickest: Vec<usize>,
}

/// A file or a container listed.
#[derive(Debug)]
pub(crate) struct Listed {
    /// In the container listed: `2024/beach.jpg`, recursing. A file opens by it.
    pub(crate) name: String,
    /// A container's own, to list; `None` for a file.
    pub(crate) container: Option<Rc<dyn Container>>,
}

/// The first of `backends` that opens the file at `path`, which starts with `head`.
fn pick<'b>(backends: &[&'b dyn Backend], path: &str, head: &[u8]) -> Result<&'b dyn Backend, String> {
    if let Some(backend) = backends.iter().find(|backend| backend.opens(path, head)) {
        return Ok(*backend);
    }
    let mut shown: Vec<&str> = Vec::new();
    for backend in backends {
        if !shown.contains(&backend.shows()) {
            shown.push(backend.shows());
        }
    }
    let list = match shown.split_last() {
        Some((last, [])) => (*last).to_string(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => "nothing".into(),
    };
    Err(format!("{path}: not a file {} shows (it shows {list})", crate::NAME))
}

/// A render asked of a source: `page` (0-based) at `scale` output pixels per unit of its
/// [`Source::size`], with the settings at `values`. `tag` tells the result apart from others of the
/// same page.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Request {
    pub(crate) page: usize,
    pub(crate) scale: f32,
    pub(crate) tag: u64,
    /// The settings' values: a backend draws by those it declares, for a file that has them.
    pub(crate) values: Values,
    /// The colour of dark paper here: the terminal's own background where it is dark.
    pub(crate) dark: [u8; 3],
    /// For a list's tile, or its copy: drawn as a tile, which may leave out what costs more than a
    /// list's tiles should (an SVG's HTML labels). Only the pager sets it.
    pub(crate) tile: bool,
    /// For a picture a document shows on dark paper, of the colour [`Request::dark`]: a backend with a
    /// dark form of what it draws (a diagram's dark theme) draws that, with nothing behind it, so that
    /// the paper shows through, and at the size of its light form, since the document is laid out
    /// around the picture once. A backend with no dark form draws as ever. Asked of a
    /// [`Source::renderer`] alone, never of a pool, and only for a document's fenced blocks
    /// ([`Format::fences`]): a file shown by itself, and a picture by path, are never asked so.
    pub(crate) on_dark: bool,
}

/// Premultiplied RGBA, row by row, top to bottom: a backend's own buffer, read as bytes, so its
/// pixels reach the terminal without a copy.
pub(crate) struct Pixels(Box<dyn AsRef<[u8]> + Send>);

impl Pixels {
    pub(crate) fn new(pixels: impl AsRef<[u8]> + Send + 'static) -> Self {
        Self(Box::new(pixels))
    }
}

impl Deref for Pixels {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        (*self.0).as_ref()
    }
}

/// A rendered page, or why there is none.
pub(crate) struct Frame {
    pub(crate) request: Request,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Pixels,
    pub(crate) error: Option<String>,
    /// What the reader should know about the page, each once: content left out, a colour that
    /// had to be guessed. In words for the reader, without the page number.
    pub(crate) warnings: Vec<String>,
    /// No page, as the request was made for a size that turned out otherwise ([`Later`]): the viewer
    /// is to ask the source its size again, and ask again for the page at that.
    pub(crate) again: bool,
}

impl Frame {
    /// No page for `request`, and why.
    pub(crate) fn failed(request: Request, error: String) -> Self {
        Self { request, width: 0, height: 0, rgba: Pixels::new(Vec::new()), error: Some(error), warnings: Vec::new(), again: false }
    }

    /// No page for `request`, to be asked for again ([`Frame::again`](field@Frame::again)).
    pub(crate) fn asked_again(request: Request) -> Self {
        Self { request, width: 0, height: 0, rgba: Pixels::new(Vec::new()), error: None, warnings: Vec::new(), again: true }
    }
}

/// What `render` returns, or a failed page if it panics: a backend's bug costs the page, not the
/// viewer.
pub(crate) fn guarded(request: Request, render: impl FnOnce() -> Frame) -> Frame {
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(render));
    caught.unwrap_or_else(|_| Frame::failed(request, "the renderer failed on this page".into()))
}

/// Something with pages to show: a PDF's pages, or an image as one page.
pub(crate) trait Source {
    /// How many pages: at least one, since a source with none fails to open.
    fn pages(&self) -> usize;
    /// Page `page`'s size as shown, in the source's own unit (points for a PDF), or `None` past
    /// the last page. A render at scale `s` is about `s` times as many pixels on each side.
    fn size(&self, page: usize) -> Option<(f32, f32)>;
    /// Whether page `page`'s size is its natural size, in pixels of its own (an image's): fitted
    /// to the window, it is never drawn larger, unless the reader zooms to the width. A PDF's
    /// points are a unit for any scale.
    fn natural(&self, page: usize) -> bool;
    /// The settings it has, by name, of those its backend declares ([`Backend::settings`]): dark
    /// paper for a page that is just text with no paper of its own, never for a PDF or a Word
    /// document, a page as printed, white in its design. None by default.
    fn settings(&self) -> Vec<&'static str> {
        Vec::new()
    }
    /// Find out page `page`'s true size, for a caller that may wait for it: printing. The pager never
    /// waits: until it is known, [`Source::size`] may be a stand-in ([`Later`]).
    fn settle(&self, _page: usize) {}
    /// A renderer on this thread, for pages printed one after another.
    fn renderer(&self) -> Result<Box<dyn Render + '_>, String>;
    /// Renders on threads of their own, for the pager: keys still work while pages render, and
    /// a page that takes too long is given up on. The pool holds what it needs of the source,
    /// so the pager can keep both.
    fn pool(&self, threads: usize) -> Box<dyn Pool>;
    /// The values of the relayout settings ([`Effect::Relayout`]) it was laid out at, a document's
    /// width, and the room it was laid out for, both exactly as asked ([`Backend::open_at`],
    /// [`Source::relayout`]), never what the source made of them: the viewer compares them with what
    /// it wants, and a source that answered otherwise would be laid out again on every pass. No
    /// values and no room for what is laid out once and for all.
    fn laid_at(&self) -> (Values, Option<Room>) {
        (Values::default(), None)
    }
    /// A job that lays this out again at `values`, for `room`, for a source laid out at others: it
    /// owns what it needs (the bytes the source keeps, the values, the room), nothing of the viewer's,
    /// and gives a new source laid out at them, or why not. It may run on any thread, and a panic in it
    /// costs the relayout alone. `None` when the layout is at `values` in that room already, or
    /// nothing here lays out again.
    fn relayout(&self, _values: Values, _room: Room) -> Option<Job> {
        None
    }
    /// The largest scale (pixels per unit of [`Source::size`]) page `page` is shown at, fitted to the
    /// window: for a page laid out for a room, the scale at which it is as tall as that room, so that
    /// its text is never larger than its font size. `None` for a page shown at any scale.
    fn largest_scale(&self, _page: usize) -> Option<f32> {
        None
    }
    /// Where setting `setting`'s values sit for this source shown in `room`, laid out with `values`:
    /// where its named value sits among its numbers, and the most a number is laid out at there.
    /// `None` for a setting this source has no say over.
    fn bounds(&self, _setting: &str, _room: Room, _values: Values) -> Option<Bounds> {
        None
    }
    /// Where the reader is, for a source laid out again to find it: the paragraph at `y` points down
    /// page `page`, or the page's last if `y` is below them all. `None` where there is no paragraph to
    /// go by (a page of pictures, a source that doesn't lay out again).
    fn anchor(&self, _page: usize, _y: f32) -> Option<Anchor> {
        None
    }
    /// Where `anchor`, from this source or one it was laid out again from, is in this layout: the page
    /// it starts on, and how far down it, in points. `None` for one this source doesn't know.
    fn find(&self, _anchor: &Anchor) -> Option<(usize, f32)> {
        None
    }
    /// Its text, for the reader to search (`/`): read on threads or in a process of the source's
    /// own from here on, so asked for only when a search starts, and let go of when it ends.
    /// `None` for a source with no text to search (a picture, a drawing), the default.
    fn texts(&self) -> Option<Box<dyn Texts>> {
        None
    }
}

/// A source's text, for the viewer to search ([`Source::texts`]): in chunks, in the document's
/// order, each whatever the backend's text comes in (a PDF's page, a document's paragraph). The
/// viewer folds case, matches and tints, the same for every backend; a match never runs from one
/// chunk onto the next.
///
/// A chunk's text is the text as it should be searched: what the end of a line, a hyphen there or
/// a ligature becomes is the backend's to decide, and the viewer never corrects it. A place in it
/// is a count of its characters (`char`s) from its start.
///
/// The work keeps [`Pool`]'s contract: [`Texts::set_queue`] replaces what is wanted,
/// [`Texts::try_recv`] never blocks, a result arrives only while it is wanted, and dropping this
/// ends its work without waiting for it. Whatever it gives is input like any other: the viewer
/// checks it.
pub(crate) trait Texts {
    /// How many chunks the text comes in.
    fn chunks(&self) -> usize;
    /// The chunks with text on page `page` (0-based), first to last: none for a page without
    /// text. Two pages share a chunk that runs from one onto the other, so a page's first may be
    /// the last of the page before; neither ever comes before an earlier page's. Where chunks
    /// aren't in the pages' order (a table's cells, side by side on a page and one after another
    /// in the text), the range may take in chunks with no text on that page.
    fn chunks_on(&self, page: usize) -> Range<usize>;
    /// What the text was read from, as a number the same for the same text and, near enough,
    /// never for another: a search of a document laid out again keeps what it read only if the new
    /// text's is the same. `None` where the source can't say, the default: a PDF's, which is never
    /// laid out again, and whose search would start over.
    fn fingerprint(&self) -> Option<u64> {
        None
    }
    /// Replace what is wanted, the most urgent first. One still waited for is listed again every
    /// time; one given isn't given again while it stays listed.
    fn set_queue(&self, wanted: Vec<Want>);
    /// Something wanted, if it is ready. Never blocks.
    fn try_recv(&self) -> Option<Got>;
}

/// What the viewer wants of a source's text ([`Texts::set_queue`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Want {
    /// A chunk's text.
    Text(usize),
    /// Where the characters `from..to` of a chunk's text are on its pages.
    Rects { chunk: usize, from: usize, to: usize },
}

/// What a source gives of its text ([`Texts::try_recv`]), or why not, in words for the reader.
#[cfg_attr(not(any(feature = "pdf", feature = "word", test)), expect(dead_code, reason = "only PDFs and documents give their text"))]
pub(crate) enum Got {
    /// A chunk's text. Why not costs the search that chunk alone.
    Text(usize, Result<String, String>),
    /// The rectangles a [`Want::Rects`] covers, one for each line: the page (0-based) each is on,
    /// and its left, top, right and bottom there, in the page's unit from its top left, y down,
    /// as [`Source::size`] measures it.
    Rects(Want, Result<Vec<(usize, [f32; 4])>, String>),
}

/// A place in a document as its source knows it ([`Source::anchor`]), meaning nothing to the viewer,
/// which only hands it back ([`Source::find`]) to a source laid out again from the same document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Anchor(pub(crate) u64);

/// A source laid out again ([`Source::relayout`]), once run: the new source, or why there is none.
pub(crate) type Job = Box<dyn FnOnce() -> Result<Box<dyn Source + Send>, String> + Send>;

/// Renders on the caller's thread.
pub(crate) trait Render {
    fn render(&mut self, request: Request) -> Frame;
}

/// Renders on threads of the pool's own, the most urgent first.
pub(crate) trait Pool {
    /// Replace the renders wanted, the most urgent first. A result is delivered only while its
    /// request is in the latest queue: one still waited for must be listed again every time.
    fn set_queue(&self, requests: Vec<Request>);
    /// A finished render, if one is ready. Never blocks.
    fn try_recv(&self) -> Option<Frame>;
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The core never depends on an extension (CONTRIBUTING's rule): nothing in `src/view/`,
    /// `src/main.rs`, `src/args.rs` or `src/encode.rs` names a module of `src/source/`, a backend
    /// or the threads lent to one. The viewer knows sources only through the items here, and takes
    /// the backends built from `main`, the one file that names `source::BACKENDS`. (What a backend
    /// renders with is named only by the backend and its own tests, which check it.)
    #[test]
    fn the_viewer_names_no_backend() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let modules: Vec<String> = std::fs::read_dir(root.join("src/source"))
            .unwrap()
            .filter_map(|e| e.unwrap().path().file_stem().map(|s| s.to_string_lossy().into_owned()))
            .collect();
        assert!(modules.len() >= 3, "{modules:?}");
        let mut core: Vec<std::path::PathBuf> = ["src/main.rs", "src/args.rs", "src/encode.rs"].iter().map(|f| root.join(f)).collect();
        let dir = root.join("src/view");
        core.extend(std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "rs")));
        assert!(core.len() >= 12, "read {} files, {} of them in {}", core.len(), core.len() - 3, dir.display());
        for path in &core {
            let text = std::fs::read_to_string(path).unwrap();
            for module in &modules {
                assert!(!text.contains(&format!("source::{module}")), "{} names source::{module}", path.display());
            }
            if !path.ends_with("main.rs") {
                assert!(!text.contains("source::BACKENDS"), "{} names the backends built: main's to hand over", path.display());
                assert!(!text.contains("source::WORKERS"), "{} names the workers built: main's to hand over", path.display());
            }
        }
    }

    /// A backend that opens files starting with `magic`: to a page (a folder's), or if it
    /// `holds` files, to the directory beside the file, named as it is with `.d` after. It lists
    /// one format, and credits one library, named as it is.
    pub(crate) struct Fake {
        pub(crate) name: &'static str,
        pub(crate) shows: &'static str,
        pub(crate) magic: &'static [u8],
        pub(crate) options: &'static [OptionSpec],
        pub(crate) holds: bool,
    }

    impl Backend for Fake {
        fn name(&self) -> &'static str {
            self.name
        }
        fn shows(&self) -> &'static str {
            self.shows
        }
        fn formats(&self) -> Vec<Format> {
            vec![Format::of(self.name, &[])]
        }
        fn credits(&self) -> &[&'static str] {
            std::slice::from_ref(&self.name)
        }
        fn options(&self) -> &'static [OptionSpec] {
            self.options
        }
        fn opens(&self, _path: &str, head: &[u8]) -> bool {
            head.starts_with(self.magic)
        }
        fn open(&self, input: Input, _options: &Parsed<'_>) -> Result<Opened, String> {
            Ok(match self.holds {
                true => Opened::Files(Rc::new(Directory::new(&format!("{}.d", input.path)))),
                false => Opened::Pages(Box::new(Folder(Badge::None))),
            })
        }
    }

    pub(crate) const PDF: Fake = Fake { name: "pdf-one", shows: "PDFs", magic: b"%PDF-", options: &[], holds: false };
    const PDF_TOO: Fake = Fake { name: "pdf-two", shows: "PDFs", magic: b"%PDF-", options: &[], holds: false };
    const PNG: Fake = Fake { name: "png", shows: "images", magic: b"\x89PNG", options: &[], holds: false };
    const DIR: Fake = Fake { name: "dir", shows: "directories", magic: b"\0dir", options: &[], holds: false };
    /// Files that hold files.
    pub(crate) const ARCHIVE: Fake = Fake { name: "archive", shows: "archives", magic: b"\0arc", options: &[], holds: true };

    #[test]
    fn the_first_backend_that_opens_a_file_opens_it() {
        let backends: [&dyn Backend; 3] = [&PNG, &PDF, &PDF_TOO];
        assert_eq!(pick(&backends, "a.pdf", b"%PDF-1.7\n").map(|b| b.name()), Ok("pdf-one"));
        assert_eq!(pick(&backends, "a.png", b"\x89PNG\r\n").map(|b| b.name()), Ok("png"));
        assert_eq!(pick(&[&PDF_TOO, &PDF], "a.pdf", b"%PDF-2.0").map(|b| b.name()), Ok("pdf-two"));
    }

    #[test]
    fn a_file_no_backend_opens_is_refused_with_what_would_be_shown() {
        let refused = |backends: &[&dyn Backend]| pick(backends, "notes.txt", b"hello").err();
        let said = |shows: &str| Some(format!("notes.txt: not a file {} shows (it shows {shows})", crate::NAME));
        assert_eq!(refused(&[&PDF, &PDF_TOO]), said("PDFs"));
        assert_eq!(refused(&[&PDF, &PNG]), said("PDFs and images"));
        assert_eq!(refused(&[&PDF, &PNG, &PDF_TOO, &DIR]), said("PDFs, images and directories"));
        assert!(pick(&[&PDF], "empty", b"").is_err());
    }

    #[test]
    fn a_file_is_read_once_its_head_and_then_the_rest() {
        let path = std::env::temp_dir().join(format!("clview-input-{}", std::process::id()));
        let bytes: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        let path = path.to_str().unwrap();
        let input = Input::open(path).unwrap();
        assert_eq!(input.head, bytes[..1024]);
        assert_eq!(input.bytes().unwrap(), bytes);
        // On disk, a backend gets the file back from its start.
        let Ok(Reopenable::Disk(mut file)) = Input::open(path).unwrap().reopenable() else { panic!("not on disk") };
        let mut again = Vec::new();
        file.read_to_end(&mut again).unwrap();
        assert_eq!(again, bytes);
        std::fs::write(path, b"%PDF").unwrap();
        assert_eq!(Input::open(path).unwrap().head, b"%PDF");
        std::fs::remove_file(path).unwrap();
        assert!(Input::open(path).is_err_and(|e| e.starts_with(path)));
    }

    /// A file read to a cap is read whole within it, and found larger past it, without reading on:
    /// on disk, through a pipe, held in another (by a holder that can stop early, only so far).
    #[test]
    fn a_file_is_read_whole_only_within_a_cap() {
        let path = std::env::temp_dir().join(format!("clview-at-most-{}", std::process::id()));
        std::fs::write(&path, vec![b'x'; 3000]).unwrap();
        let path = path.to_str().unwrap();
        assert_eq!(Input::open(path).unwrap().at_most(3000).unwrap().map(|b| b.len()), Some(3000));
        assert_eq!(Input::open(path).unwrap().at_most(2999).unwrap().map(|b| b.len()), None);
        assert_eq!(Input::open(path).unwrap().at_most(10).unwrap().map(|b| b.len()), None, "within the head");
        std::fs::remove_file(path).unwrap();
        // `yes` writes without end: only a little past the cap is read.
        let mut yes = std::process::Command::new("yes").stdout(std::process::Stdio::piped()).spawn().unwrap();
        let stdout = yes.stdout.take().unwrap();
        let mut head = Vec::new();
        let mut rest = File::from(std::os::fd::OwnedFd::from(stdout));
        Read::by_ref(&mut rest).take(HEAD).read_to_end(&mut head).unwrap();
        let piped = Input { path: "yes".into(), head, rest: Rest::File(rest) };
        assert_eq!(piped.at_most(1 << 20).unwrap().map(|b| b.len()), None);
        yes.kill().unwrap();
        yes.wait().unwrap();
        // A holder that can stop early is asked to, never for the whole file.
        let whole: Again = Arc::new(|| panic!("read whole"));
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let told = asked.clone();
        let bounded: Bounded = Arc::new(move |most| {
            told.lock().unwrap().push(most);
            Ok((most >= 100).then(|| vec![b'x'; 100]))
        });
        let input = |most| Input::held_bounded("a.zip/b".into(), Vec::new(), whole.clone(), bounded.clone()).at_most(most);
        assert_eq!((input(99).unwrap().map(|b| b.len()), input(100).unwrap().map(|b| b.len())), (None, Some(100)));
        assert_eq!(*asked.lock().unwrap(), [99, 100]);
        let again: Again = Arc::new(|| Ok(vec![b'x'; 100]));
        assert_eq!(Input::held("a.zip/b".into(), Vec::new(), again.clone()).at_most(100).unwrap().map(|b| b.len()), Some(100));
        assert_eq!(Input::held("a.zip/b".into(), Vec::new(), again).at_most(99).unwrap().map(|b| b.len()), None);
    }

    #[test]
    fn a_file_held_in_another_is_read_from_it_again_each_time() {
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = reads.clone();
        let again: Again = Arc::new(move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(b"%PDF-1.7 whole".to_vec())
        });
        let input = Input::held("a.zip/b.pdf".into(), b"%PDF-1.7".to_vec(), again.clone());
        assert_eq!(pick(&[&PNG, &PDF], &input.path, &input.head).map(|b| b.name()), Ok("pdf-one"));
        assert_eq!(input.bytes().unwrap(), b"%PDF-1.7 whole");
        let Ok(Reopenable::Again(again)) = Input::held("a.zip/b.pdf".into(), Vec::new(), again).reopenable() else { panic!("held") };
        assert_eq!(again().unwrap(), b"%PDF-1.7 whole");
        assert_eq!(reads.load(std::sync::atomic::Ordering::Relaxed), 2, "read when asked, not before");
        let failing: Again = Arc::new(|| Err("gone".into()));
        assert_eq!(Input::held("a.zip/c.pdf".into(), Vec::new(), failing).bytes().err().as_deref(), Some("a.zip/c.pdf: gone"));
    }

    #[test]
    fn a_name_is_matched_by_its_extension_in_any_case() {
        let format = |extensions| Format::of("a format", extensions);
        assert!(format(&["docx"]).named("a/b.DocX") && format(&["csv", "xlsx"]).named("b.tar.xlsx"));
        assert!(!format(&["docx"]).named("docx") && !format(&["docx"]).named("a.docx.gz") && !format(&[]).named("a.docx"));
    }

    /// The extensions a backend lists are the names it opens a file by: each opens one whose first
    /// bytes leave it unsaid, in any case, and no other name does, of those listed by any backend
    /// and others files are often named with. Tried on the first bytes of each kind of file that
    /// goes by its name: none (a TGA, an SVG whose root element comes later), text (a CSV), a zip
    /// of nothing it knows, a compound file, a Mermaid diagram.
    #[test]
    fn the_extensions_a_backend_lists_are_the_names_it_opens_by() {
        let header = [&b"PK\x03\x04\x14\0"[..], &[0; 12], &4u32.to_le_bytes(), &4u32.to_le_bytes(), &9u16.to_le_bytes(), &[0, 0]].concat();
        let zip = [&header[..], b"notes.txt", b"text"].concat();
        let compound = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0, 0, 0];
        let heads: [&[u8]; 5] = [b"", b"date,item\n2026-03-01,widget\n", &zip, &compound, b"%% a flow\nflowchart LR\n  a --> b\n"];
        let extensions = |backend: &dyn Backend| -> Vec<&str> { backend.formats().iter().flat_map(|f| f.extensions.iter().copied()).collect() };
        let listed: Vec<&str> = BACKENDS.iter().flat_map(|b| extensions(*b)).collect();
        let others = [
            "pdf", "ps", "eps", "xps", "djvu", "png", "jpg", "jpeg", "gif", "webp", "tif", "tiff", "bmp", "ico", "tga", "icb", "vda", "vst", "dds",
            "pcx", "avif", "heic", "jxl", "exr", "hdr", "ppm", "qoi", "ff", "psd", "svg", "svgz", "txt", "md", "xml", "html", "json", "rtf", "doc",
            "docx", "odt", "fodt", "xls", "xlsx", "ods", "fods", "csv", "tsv", "ppt", "pptx", "odp", "pages", "numbers", "key", "zip", "jar", "cbz",
            "epub", "tar", "gz", "tgz", "bz2", "xz", "zst", "7z", "rar",
        ];
        for backend in BACKENDS {
            let own = extensions(*backend);
            assert!(!backend.formats().is_empty(), "{} lists no format", backend.name());
            let tells = |name: &str| heads.iter().any(|head| backend.opens(&format!("a.{name}"), head) != backend.opens("a", head));
            for name in listed.iter().chain(&others) {
                assert_eq!(tells(name), own.contains(name), "{}: .{name}", backend.name());
            }
            for name in own {
                assert!(tells(&name.to_uppercase()), "{}: .{}", backend.name(), name.to_uppercase());
            }
        }
    }

    /// A backend that lists the fences `fences`, and draws pictures or not: a block whose text is two
    /// numbers, `40 20`, opens to one page that size, drawn opaque red, or opaque blue for dark paper
    /// ([`Request::on_dark`]); any other text is refused.
    pub(crate) struct Sketches {
        pub(crate) fences: &'static [&'static str],
        pub(crate) pictures: bool,
    }

    impl Backend for Sketches {
        fn name(&self) -> &'static str {
            "sketches"
        }
        fn shows(&self) -> &'static str {
            "sketches"
        }
        fn formats(&self) -> Vec<Format> {
            vec![Format::of("Plain", &["txt"]), Format::of("Sketch", &["sk", "sketch"]).fenced(self.fences)]
        }
        fn credits(&self) -> &[&'static str] {
            &[]
        }
        fn options(&self) -> &'static [OptionSpec] {
            &[]
        }
        fn opens(&self, _path: &str, _head: &[u8]) -> bool {
            false
        }
        fn open(&self, input: Input, _options: &Parsed<'_>) -> Result<Opened, String> {
            let path = input.path.clone();
            let text = String::from_utf8(input.bytes()?).map_err(|e| e.to_string())?;
            let mut sides = text.split_whitespace().map(str::parse::<f32>);
            match (sides.next(), sides.next(), sides.next()) {
                (Some(Ok(width)), Some(Ok(height)), None) => Ok(Opened::Pages(Box::new(Sketch(width, height)))),
                _ => Err(format!("{path}: not a sketch")),
            }
        }
        fn draws_pictures(&self) -> bool {
            self.pictures
        }
    }

    /// What [`Sketches`] opens: a page of a size, drawn opaque red, or blue for dark paper.
    struct Sketch(f32, f32);

    impl Source for Sketch {
        fn pages(&self) -> usize {
            1
        }
        fn size(&self, page: usize) -> Option<(f32, f32)> {
            (page == 0).then_some((self.0, self.1))
        }
        fn natural(&self, _page: usize) -> bool {
            false
        }
        fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
            Ok(Box::new(Sketch(self.0, self.1)))
        }
        fn pool(&self, _threads: usize) -> Box<dyn Pool> {
            Box::new(Sketch(self.0, self.1))
        }
    }

    impl Render for Sketch {
        fn render(&mut self, request: Request) -> Frame {
            let side = |points: f32| ((points * request.scale).ceil() as u32).clamp(1, 4096);
            let (width, height) = (side(self.0), side(self.1));
            let colour = if request.on_dark { [0, 0, 255, 255] } else { [255, 0, 0, 255] };
            let rgba = colour.repeat(width as usize * height as usize);
            Frame { request, width, height, rgba: Pixels::new(rgba), error: None, warnings: Vec::new(), again: false }
        }
    }

    /// A pool no test asks.
    impl Pool for Sketch {
        fn set_queue(&self, _requests: Vec<Request>) {}
        fn try_recv(&self) -> Option<Frame> {
            None
        }
    }

    /// What is wrong with the fences `backends` list: one listed by a backend that draws no pictures,
    /// one that isn't a lowercase word, or one listed twice.
    fn wrong_fences(backends: &[&dyn Backend]) -> Vec<String> {
        let mut seen: Vec<&str> = Vec::new();
        let mut wrong = Vec::new();
        for backend in backends {
            for fence in backend.formats().iter().flat_map(|format| format.fences.iter().copied()) {
                if !backend.draws_pictures() {
                    wrong.push(format!("{} lists {fence}, and draws no pictures", backend.name()));
                }
                if fence.is_empty() || fence.chars().any(|c| c.is_whitespace() || c.is_uppercase()) {
                    wrong.push(format!("{}: {fence:?} is no lowercase word", backend.name()));
                }
                match seen.iter().any(|other| other.eq_ignore_ascii_case(fence)) {
                    true => wrong.push(format!("{fence} is listed twice")),
                    false => seen.push(fence),
                }
            }
        }
        wrong
    }

    /// Only a backend that draws pictures lists fences, each a lowercase word, and no language is
    /// listed twice among the backends built: a block is drawn by one backend, and a document can't
    /// come to draw itself.
    #[test]
    fn a_fence_is_listed_once_and_only_by_a_backend_that_draws_pictures() {
        assert_eq!(wrong_fences(BACKENDS), Vec::<String>::new());
        // What it would say.
        static DRAWS: Sketches = Sketches { fences: &["sketch", "sk"], pictures: true };
        static HOLDS: Sketches = Sketches { fences: &["sketch"], pictures: false };
        static SHOUTS: Sketches = Sketches { fences: &["Sketch"], pictures: true };
        static WORDS: Sketches = Sketches { fences: &["a sketch", ""], pictures: true };
        static NONE: Sketches = Sketches { fences: &[], pictures: false };
        assert!(wrong_fences(&[&DRAWS, &NONE, &PDF]).is_empty());
        assert_eq!(wrong_fences(&[&HOLDS]).len(), 1, "it draws no pictures");
        assert_eq!(wrong_fences(&[&DRAWS, &DRAWS]).len(), 2, "each of the two, twice");
        assert_eq!(wrong_fences(&[&DRAWS, &SHOUTS]).len(), 2, "twice whatever the case, and not lowercase");
        assert_eq!(wrong_fences(&[&WORDS]).len(), 2, "two words, and none");
    }

    /// A fenced block's language finds the backend that lists it and the format it is of, in any
    /// case; a language none lists finds none, nor one listed by a backend that draws no pictures.
    #[test]
    fn a_fences_language_finds_the_backend_that_draws_it() {
        static DRAWS: Sketches = Sketches { fences: &["sketch", "sk"], pictures: true };
        static HOLDS: Sketches = Sketches { fences: &["held"], pictures: false };
        let backends: [&dyn Backend; 3] = [&PDF, &HOLDS, &DRAWS];
        let found =
            |language: &str| fenced(language, &backends).map(|(backend, format)| (backend.name(), format.name, format.extensions.first().copied()));
        assert_eq!(found("sketch"), Some(("sketches", "Sketch", Some("sk"))));
        assert_eq!(found("SK"), Some(("sketches", "Sketch", Some("sk"))), "in any case");
        for language in ["", "sketches", "ske", "txt", "plain", "pdf-one", "held", "sketch "] {
            assert_eq!(found(language), None, "{language:?}");
        }
        assert!(fenced("sketch", &[]).is_none());
    }

    /// A backend that declares `settings`, and opens what [`PDF`] does.
    pub(crate) struct Declaring(pub(crate) &'static [Setting]);

    impl Backend for Declaring {
        fn name(&self) -> &'static str {
            "declaring"
        }
        fn shows(&self) -> &'static str {
            PDF.shows()
        }
        fn formats(&self) -> Vec<Format> {
            PDF.formats()
        }
        fn credits(&self) -> &[&'static str] {
            &[]
        }
        fn options(&self) -> &'static [OptionSpec] {
            &[]
        }
        fn settings(&self) -> &'static [Setting] {
            self.0
        }
        fn opens(&self, path: &str, head: &[u8]) -> bool {
            PDF.opens(path, head)
        }
        fn open(&self, input: Input, options: &Parsed<'_>) -> Result<Opened, String> {
            PDF.open(input, options)
        }
    }

    /// A setting the tests declare, cycled by `key`, its option `option`.
    pub(crate) const fn cycled(name: &'static str, key: char, option: &'static str) -> Setting {
        Setting {
            name,
            named: &[Named { name: "light", says: "dark on light" }, Named { name: "dark", says: "light on dark" }],
            numbers: None,
            default: Value::Named("light"),
            zero: Value::Named("light"),
            auto: Some(Auto { light: "light", dark: "dark" }),
            keys: Keys::Cycle(key),
            option: OptionSpec { name: option, short: None, value: Some("auto|light|dark"), help: "", alias: None },
            help: "",
            missing: "not this one",
            effect: Effect::Redraw,
            sets_width: false,
        }
    }

    /// What is wrong with the keys `backends` give their settings: one that is the viewer's, one that
    /// means two settings, or a setting given two sets of keys.
    fn wrong_keys(backends: &[&dyn Backend]) -> Vec<String> {
        let mut seen: Vec<(char, &str)> = Vec::new();
        let mut keys: Vec<(&str, Keys)> = Vec::new();
        let mut wrong = Vec::new();
        for backend in backends {
            for setting in backend.settings() {
                match keys.iter().find(|(name, _)| *name == setting.name) {
                    Some((_, known)) if *known != setting.keys => wrong.push(format!("{}: {} has other keys", backend.name(), setting.name)),
                    Some(_) => {}
                    None => keys.push((setting.name, setting.keys)),
                }
                for key in setting.keys.chars() {
                    // The zoom keys alone may be a setting's: they are the viewer's where the setting isn't.
                    if crate::view::VIEWER_KEYS.contains(key) && setting.keys != Keys::Zoom {
                        wrong.push(format!("{}: {key:?}, {}'s key, is the viewer's", backend.name(), setting.name));
                    }
                    match seen.iter().find(|(seen, _)| *seen == key) {
                        Some((_, other)) if *other != setting.name => wrong.push(format!("{key:?} is {other}'s and {}'s", setting.name)),
                        Some(_) => {}
                        None => seen.push((key, setting.name)),
                    }
                }
            }
        }
        wrong
    }

    /// No backend's setting has one of the viewer's keys, a key means the same setting in every
    /// backend that declares it, and the build declares no more settings than a request carries.
    #[test]
    fn a_settings_keys_are_its_own_in_every_backend() {
        assert_eq!(wrong_keys(BACKENDS), Vec::<String>::new());
        let mut names: Vec<&str> = BACKENDS.iter().flat_map(|b| b.settings()).map(|s| s.name).collect();
        names.sort_unstable();
        names.dedup();
        assert!(names.len() <= MAX_SETTINGS, "{names:?}");
        // What it would say.
        static PAPER: Declaring = Declaring(&[cycled("paper", 'd', "--theme")]);
        static QUITS: Declaring = Declaring(&[cycled("paper", 'q', "--theme")]);
        static OTHER: Declaring = Declaring(&[cycled("tint", 'd', "--tint")]);
        static STEPS: Declaring = Declaring(&[Setting { keys: Keys::Step('[', ']'), ..cycled("paper", 'd', "--theme") }]);
        static ZOOMS: Declaring = Declaring(&[Setting { keys: Keys::Zoom, ..cycled("font", '+', "--font") }]);
        static PLUS: Declaring = Declaring(&[Setting { keys: Keys::Step('-', '+'), ..cycled("font", '+', "--font") }]);
        assert!(wrong_keys(&[&PAPER, &PAPER]).is_empty(), "one setting in two backends");
        assert_eq!(wrong_keys(&[&QUITS]).len(), 1, "the viewer's");
        assert!(wrong_keys(&[&PAPER, &ZOOMS]).is_empty(), "the zoom keys, as Keys::Zoom alone");
        assert_eq!(wrong_keys(&[&PLUS]).len(), 2, "the zoom keys as a Step are the viewer's");
        assert_eq!((Keys::Zoom.up('+'), Keys::Zoom.up('='), Keys::Zoom.up('_'), Keys::Zoom.up('x')), (Some(true), Some(true), Some(false), None));
        assert_eq!((Keys::Step('[', ']').up(']'), Keys::Step('[', ']').up('['), Keys::Cycle('d').up('d')), (Some(true), Some(false), None));
        assert_eq!(wrong_keys(&[&PAPER, &OTHER]).len(), 1, "two settings");
        assert_eq!(wrong_keys(&[&PAPER, &STEPS]).len(), 1, "two sets of keys");
    }

    #[test]
    fn a_value_is_held_by_its_settings_name_and_values_hold_so_many() {
        let mut values = Values::default();
        assert_eq!(values.get("paper"), None);
        assert!(values.set("paper", Value::Named("dark")) && values.set("width", Value::Number(72)));
        assert!(values.set("paper", Value::Named("light")));
        assert_eq!((values.get("paper"), values.get("width")), (Some(Value::Named("light")), Some(Value::Number(72))));
        let names = ["a", "b", "c", "d", "e", "f"];
        assert!(names.iter().all(|name| values.set(name, Value::Number(1))));
        assert!(!values.set("g", Value::Number(1)), "past {MAX_SETTINGS}");
        assert_eq!(values.get("g"), None);
    }

    #[test]
    fn without_backends_nothing_opens() {
        assert!(choose("a.pdf", &[]).is_err_and(|e| e.contains("built without backends")));
    }

    /// Two rooms are the same with the same columns, rows and gap and a cell within 2% each way: a
    /// window dragged a few pixels is the same room, a font change isn't, and a text cell's sampling
    /// cell is kept as it is, never rounded to a whole pixel.
    #[test]
    fn a_room_is_the_same_within_a_little_of_its_cell() {
        let room = Room { cols: 100, rows: 40, cell_w: 10.0, cell_h: 20.0, gap: 2, per_row: None };
        assert!(room.same(&room));
        assert!(room.same(&Room { cell_w: 10.1, cell_h: 19.8, ..room }), "1% off");
        assert!(!room.same(&Room { cell_w: 10.3, ..room }) && !room.same(&Room { cell_h: 20.6, ..room }), "3% off");
        assert!(!room.same(&Room { cols: 101, ..room }) && !room.same(&Room { rows: 39, ..room }) && !room.same(&Room { gap: 1, ..room }));
        assert!(
            !room.same(&Room { per_row: Some(3), ..room })
                && Room { per_row: Some(3), ..room }.same(&Room { per_row: Some(3), cell_w: 10.1, ..room })
        );
        let sampling = Room { cell_w: 1.25, cell_h: 2.0, ..room };
        assert!(sampling.same(&sampling) && !sampling.same(&Room { cell_w: 1.0, ..sampling }));
        let odd = Room { cell_w: f32::NAN, cell_h: 0.0, ..room };
        assert!(!odd.same(&odd) && Room { cell_w: 0.0, cell_h: 0.0, ..room }.same(&Room { cell_w: 0.0, cell_h: 0.0, ..room }));
    }
}
