//! Word documents (docx, doc), OpenDocument text, RTF, Markdown and HTML, read, laid out into pages
//! and drawn by wordcraft, as its app prints them: headers and footers at full strength, no comment
//! balloons, no proofing marks. A password opens a protected Word document.
//!
//! Opening reads and lays out the document once, to know it opens and its pages' sizes, and keeps
//! only where its bytes are: a renderer, or a pool, reads and lays it out again on its first
//! render and keeps that for the renders after, until it goes, as a drawing keeps its tree. A
//! search reads its text, a body paragraph a chunk, from the pool's layout ([`Paragraphs`]).
//!
//! A docx and an odt are zips: this backend is asked before the zip backend, and knows one by the
//! names of the files that start it, or a Word document by its own name.
//!
//! Markdown is read by pulldown-cmark, into the HTML wordcraft's importer reads, the one that can be
//! given pictures. The pictures both refer to are drawn by the backends that show them, through
//! `source/picture.rs`, when a page they are on first renders. A fenced block written in a language
//! a backend lists (` ```mermaid `) is such a picture, of its text
//! ([ADR-0032](../../docs/adr/0032-a-fenced-block-is-drawn-by-the-backend-that-lists-its-language.md)).

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock, Weak};

use pulldown_cmark::{BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use wordcraft_doc::para::{COLUMN_BREAK, InlineObject, NB_HYPHEN, OBJ, SOFT_HYPHEN, Wrap};
use wordcraft_doc::props::{Border, BorderStyle, Borders, LineSpacing};
use wordcraft_doc::{Block, Paragraph, Pos, Rgb, StoryRef, TextColor};
use wordcraft_layout::{DocLayout, LayoutCache, LayoutOptions, Page, Placed, ViewMode};
use wordcraft_render::RenderOptions;

use super::head::{COMPOUND, Names, ZIP, mimetype};
use super::picture::{Pictures, Promised};
use super::texts::{Answer, Queue};
use super::threads::Threads;
use super::{
    Again, Anchor, Auto, Backend, Bounds, Deferred, Effect, Format, Frame, Got, Input, Job, Keys, Named, Numbers, Opened, Pixels, Pool, Render,
    Reopenable, Request, Room, Setting, Source, Texts, Value, Values, Want, guarded,
};
use crate::args::{OptionSpec, Parsed};

/// Word documents, OpenDocument text, RTF, Markdown and HTML, laid out and drawn by wordcraft.
pub(super) struct Wordcraft;

const OPTIONS: &[OptionSpec] =
    &[OptionSpec { name: "--password", short: None, value: Some("PW"), help: "the password of a protected Word document", alias: None }];

/// Markdown and HTML on dark paper, light on dark, or as printed, dark on light: they have no paper
/// of their own ([ADR-0015](../../docs/adr/0015-markdown-and-html-on-dark-paper-the-terminals-own-colour.md)).
const PAPER: Setting = Setting {
    name: "paper",
    named: &[Named { name: LIGHT, says: "dark on light" }, Named { name: DARK, says: "light on dark" }],
    numbers: None,
    default: Value::Named(LIGHT),
    zero: Value::Named(LIGHT),
    auto: Some(Auto { light: LIGHT, dark: DARK }),
    keys: Keys::Cycle('d'),
    option: OptionSpec {
        name: "--theme",
        short: None,
        value: Some("auto|light|dark"),
        help: "Markdown and HTML light on dark or dark on light (auto: as the terminal is)",
        alias: None,
    },
    help: "Markdown and HTML light on dark, or dark on light",
    missing: "only Markdown and HTML go dark",
    effect: Effect::Redraw,
    sets_width: false,
};
const LIGHT: &str = "light";
const DARK: &str = "dark";

/// How wide a line of Markdown or HTML is: as wide as the window, or a number of characters of the
/// code face, from 20 to 200, which lays the document out again
/// ([ADR-0022](../../docs/adr/0022-markdown-and-html-reflow-to-a-width-in-characters-as-a-new-level.md),
/// [ADR-0023](../../docs/adr/0023-the-window-is-the-page-of-markdown-and-html.md)).
const WIDTH: Setting = Setting {
    name: "width",
    named: &[Named { name: WINDOW, says: "width window" }],
    numbers: Some(Numbers { least: 20, most: 200, step: 4, unit: "" }),
    default: Value::Named(WINDOW),
    zero: Value::Named(WINDOW),
    auto: None,
    keys: Keys::Step('[', ']'),
    option: OptionSpec {
        name: "--width",
        short: None,
        value: Some("window|N"),
        help: "Markdown and HTML laid out to lines of N characters of code (20-200), or as wide as the window",
        alias: None,
    },
    help: "Markdown and HTML 4 characters a line narrower, or wider; 72] sets 72, 0] the window's",
    missing: "only Markdown and HTML reflow",
    effect: Effect::Relayout,
    sets_width: true,
};
const WINDOW: &str = "window";

/// How large the text of Markdown and HTML is, as a share of its size, at which a character of the
/// code face is a cell of the window: 50% to 400% in steps of 10, on the zoom keys, which lays the
/// document out again ([ADR-0023](../../docs/adr/0023-the-window-is-the-page-of-markdown-and-html.md)).
const FONT: Setting = Setting {
    name: "font",
    named: &[],
    numbers: Some(Numbers { least: 50, most: 400, step: 10, unit: "%" }),
    default: Value::Number(FONT_DEFAULT),
    zero: Value::Number(FONT_DEFAULT),
    auto: None,
    keys: Keys::Zoom,
    option: OptionSpec { name: "--font", short: None, value: Some("N"), help: "Markdown and HTML text at N% of its size (50-400)", alias: None },
    help: "Markdown and HTML text larger, or smaller; 150+ sets 150%, 0+ 100%",
    missing: "only Markdown and HTML have a font size",
    effect: Effect::Relayout,
    sets_width: false,
};
const FONT_DEFAULT: u32 = 100;

/// How wide the side margins of Markdown and HTML are, in characters of the code face at the font size,
/// at each side: 0 to 20, which lays the document out again. At `window` they come out of the line; at a
/// width, the page grows by them, up to the window.
const MARGIN: Setting = Setting {
    name: "margin",
    named: &[],
    numbers: Some(Numbers { least: 0, most: 20, step: 1, unit: "" }),
    default: Value::Number(MARGIN_DEFAULT),
    zero: Value::Number(MARGIN_DEFAULT),
    auto: None,
    keys: Keys::Step('{', '}'),
    option: OptionSpec {
        name: "--margin",
        short: None,
        value: Some("N"),
        help: "Markdown and HTML side margins of N characters of code (0-20)",
        alias: None,
    },
    help: "Markdown and HTML margins a character narrower, or wider; 5} sets 5, 0} back to 3",
    missing: "only Markdown and HTML have margins",
    effect: Effect::Relayout,
    sets_width: false,
};
const MARGIN_DEFAULT: u32 = 3;

/// The margin `values` ask for, in characters: [`MARGIN`]'s, within its range, or the default where
/// they say nothing.
fn margin_wanted(values: &Values) -> u32 {
    match values.get(MARGIN.name) {
        Some(Value::Number(margin)) => margin_within(margin),
        _ => MARGIN_DEFAULT,
    }
}

/// The margin `--margin` gives, if given: the viewer refuses one out of the range before any file
/// opens, and anything else is the default.
fn margin_given(option: Option<&str>) -> u32 {
    option.and_then(|given| given.parse().ok()).map_or(MARGIN_DEFAULT, margin_within)
}

/// `margin`, within [`MARGIN`]'s range.
fn margin_within(margin: u32) -> u32 {
    let Some(numbers) = MARGIN.numbers else { return MARGIN_DEFAULT };
    margin.clamp(numbers.least, numbers.most.max(numbers.least))
}

/// The font size `values` ask for, as a percentage: [`FONT`]'s, within its range, or 100 where they say
/// nothing.
fn font_wanted(values: &Values) -> u32 {
    match values.get(FONT.name) {
        Some(Value::Number(percent)) => font_within(percent),
        _ => FONT_DEFAULT,
    }
}

/// The font size `--font` gives, if given: the viewer refuses one out of the range before any file
/// opens, and anything else is 100.
fn font_given(option: Option<&str>) -> u32 {
    option.and_then(|given| given.parse().ok()).map_or(FONT_DEFAULT, font_within)
}

/// `percent`, within [`FONT`]'s range.
fn font_within(percent: u32) -> u32 {
    let Some(numbers) = FONT.numbers else { return FONT_DEFAULT };
    percent.clamp(numbers.least, numbers.most.max(numbers.least))
}

/// The settings a document of `kind` has: paper, width, font size and margins, for Markdown and HTML.
fn settings_of(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Markdown | Kind::Html => &[PAPER.name, WIDTH.name, FONT.name, MARGIN.name],
        _ => &[],
    }
}

/// How wide Markdown and HTML are laid out ([`WIDTH`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Width {
    /// As wide as the window, or where it holds two pages of [`BOOK_LEAST`] characters, half of it.
    Window,
    /// Lines of so many characters of the code face, within [`WIDTH`]'s numbers, never wider than
    /// the window.
    Columns(u32),
}

impl Width {
    /// The width `values` ask for: [`WIDTH`]'s, or `window` where they say nothing.
    fn wanted(values: &Values) -> Self {
        match values.get(WIDTH.name) {
            Some(Value::Number(columns)) => Self::columns(columns),
            _ => Self::Window,
        }
    }

    /// The width `--width` gives, if given: the viewer refuses one out of the range before any file
    /// opens, and anything else is `window`.
    fn given(option: Option<&str>) -> Self {
        option.and_then(|given| given.parse().ok()).map_or(Self::Window, Self::columns)
    }

    /// `columns` characters, within the range.
    fn columns(columns: u32) -> Self {
        let Some(numbers) = WIDTH.numbers else { return Self::Window };
        Self::Columns(columns.clamp(numbers.least, numbers.most.max(numbers.least)))
    }

    /// As a setting's value.
    fn value(self) -> Value {
        match self {
            Self::Window => Value::Named(WINDOW),
            Self::Columns(columns) => Value::Number(columns),
        }
    }
}

/// A character of the code face, in points: a column, and at 100% a cell of the room on screen.
const CODE_WIDTH: f32 = CODE_SIZE * CODE_ADVANCE;

/// A line of body text, in points: [`TEXT_SIZE`] at the line spacing [`styled`] gives it.
const LINE: f32 = TEXT_SIZE * 1.15;

/// The least a page holds: so many characters of code a line, and so many lines of text. In a room
/// too small for them the page grows past the room, and is shown smaller than its font size.
const LEAST_COLUMNS: u32 = 20;
const LEAST_LINES: u32 = 10;

/// The characters a page holds at least for `window` to be two pages side by side, as an open book.
const BOOK_LEAST: u32 = 60;

/// The margin above and below, in rows of the room at 100%: the sides are [`MARGIN`]'s.
const TOP_MARGIN: f32 = 1.0;

/// The largest a page is laid out, in points, on a side: 200 inches, what a PDF allows.
const MAX_PAGE: f32 = 14_400.0;

/// The room a document is laid out for when it is opened with none ([`Backend::open`]): 100 × 40
/// cells of 10 × 20 pixels. The viewer always gives one ([`Backend::open_at`]).
const ROOMLESS: Room = Room { cols: 100, rows: 40, cell_w: 10.0, cell_h: 20.0, gap: 2, per_row: None };

/// A cell of `room` in points at `font` percent: a column wide at 100%, half of one at 200%, and as
/// tall as the cell's shape makes it, taken within reason since the room is input like any other.
fn cell_points(room: &Room, font: u32) -> (f32, f32) {
    let shape = room.cell_h / room.cell_w;
    let shape = if shape.is_finite() && shape > 0.0 { shape.clamp(0.25, 8.0) } else { 2.0 };
    // Within the setting's range, so the division is sound.
    let cell = CODE_WIDTH * 100.0 / font_within(font) as f32;
    (cell, cell * shape)
}

/// The columns of text a page `cols` cells of `cw` points wide holds between side margins of `side`,
/// in points.
fn text_columns(cols: u16, cw: f32, side: f32) -> f32 {
    (f32::from(cols) * cw).min(MAX_PAGE) - 2.0 * side
}

/// The margin above and below at `room`'s cell shape, [`TOP_MARGIN`] rows, in points, at the text's
/// size whatever the font size.
fn top_margin(room: &Room) -> f32 {
    let (_, ch) = cell_points(room, FONT_DEFAULT);
    TOP_MARGIN * ch
}

/// A side margin of `margin` characters of the code face, in points: at any font size, a character is
/// [`CODE_WIDTH`] points, `font`/100 of a cell on screen.
fn side(margin: u32) -> f32 {
    // At most 20, so exact.
    margin_within(margin) as f32 * CODE_WIDTH
}

/// The cells a page of `room` is wide: the room's, or with `per_row` pages to a row of it, its share
/// of the room's columns less the gaps between them, a cell at least.
fn page_cols(room: &Room) -> u16 {
    let Some(n) = room.per_row.filter(|n| *n > 1) else { return room.cols };
    let gaps = room.gap.checked_mul(n.saturating_sub(1));
    gaps.and_then(|gaps| room.cols.checked_sub(gaps)).and_then(|cols| cols.checked_div(n)).unwrap_or(0).max(1)
}

/// The cells a page laid out at `window` is wide in `room` with margins of `margin` characters: as
/// wide as the room, or half of it less the gap where each half holds [`BOOK_LEAST`] characters of
/// text between its margins at `font` percent, or with a count to a row ([`Room::per_row`]) its share
/// ([`page_cols`]), the book rule off.
fn window_cols(room: &Room, font: u32, margin: u32) -> u16 {
    if room.per_row.is_some() {
        return page_cols(room);
    }
    let half = room.cols.saturating_sub(room.gap) / 2;
    let (cw, _) = cell_points(room, font);
    if text_columns(half, cw, side(margin)) >= BOOK_LEAST as f32 * CODE_WIDTH { half } else { room.cols }
}

/// The page `width` is laid out on in `room` at `font` percent with side margins of `margin`
/// characters, in points such that a character of the code face is `font`/100 of a cell of the room
/// when the page is shown as tall as it: always as tall as the room; at `window`, as wide as it, or as
/// [`window_cols`] says, the margins coming out of the line; at a number, the characters between the
/// margins, the page growing by them, never wider than the room's share ([`page_cols`]). The bounds, in
/// order, each from the values as asked: the page's cells; `font`'s most there ([`font_most`]),
/// whatever the margin, a font past it laid out at it; `margin`'s most at the font laid out at
/// ([`margin_most`]), a margin past it laid out at it: the margin yields to the font. Where even no
/// margin leaves 20 characters a line, the margin is `min(margin, 1)` and the page grows past the room
/// to [`LEAST_COLUMNS`] × [`LEAST_LINES`] of text (and is shown smaller).
fn page_in(width: Width, room: &Room, font: u32, margin: u32) -> wordcraft_doc::SectionProps {
    let margin = margin_within(margin);
    let font = font_within(font).min(font_most(room));
    let laid = margin_most(room, font).map_or(margin.min(1), |most| margin.min(most));
    let (cw, ch) = cell_points(room, font);
    let (side, top) = (side(laid), top_margin(room));
    let (least_w, least_h) = (LEAST_COLUMNS as f32 * CODE_WIDTH, LEAST_LINES as f32 * LINE);
    let text_w = match width {
        Width::Window => text_columns(window_cols(room, font, laid), cw, side),
        // At most 200, so exact.
        Width::Columns(columns) => (columns as f32 * CODE_WIDTH).min(text_columns(page_cols(room), cw, side)),
    };
    let text_w = text_w.max(least_w);
    let text_h = ((f32::from(room.rows) * ch).min(MAX_PAGE) - 2.0 * top).max(least_h);
    wordcraft_doc::SectionProps {
        page_w: text_w + 2.0 * side,
        page_h: text_h + 2.0 * top,
        margin_left: side,
        margin_right: side,
        margin_top: top,
        margin_bottom: top,
        ..Default::default()
    }
}

/// Whether `room` holds a page of [`LEAST_COLUMNS`] × [`LEAST_LINES`] of text at `font` percent, a page
/// as wide as its share of the room ([`page_cols`]), the side margins down to a column whatever the
/// margin asked, which yields to the font: past the font size where it doesn't, the page grows past the
/// room.
fn fits(room: &Room, font: u32) -> bool {
    let (cw, ch) = cell_points(room, font);
    text_columns(page_cols(room), cw, side(1)) >= LEAST_COLUMNS as f32 * CODE_WIDTH
        && f32::from(room.rows) * ch - 2.0 * top_margin(room) >= LEAST_LINES as f32 * LINE
}

/// The largest font size, among [`FONT`]'s steps, at which `room` holds a page ([`fits`]), whatever the
/// margin: the least where none does.
fn font_most(room: &Room) -> u32 {
    let Some(numbers) = FONT.numbers else { return FONT_DEFAULT };
    let mut steps = (numbers.least..=numbers.most).rev().filter(|font| font % numbers.step.max(1) == 0 || *font == numbers.least);
    steps.find(|font| fits(room, *font)).unwrap_or(numbers.least)
}

/// The largest margin, in characters, at which a line of a page of `room` at `font` percent holds
/// [`LEAST_COLUMNS`] characters: `None` where even none leaves that many.
fn margin_most(room: &Room, font: u32) -> Option<u32> {
    let numbers = MARGIN.numbers?;
    let (cw, _) = cell_points(room, font);
    (numbers.least..=numbers.most).rev().find(|margin| text_columns(page_cols(room), cw, side(*margin)) >= LEAST_COLUMNS as f32 * CODE_WIDTH)
}

/// The characters of the code face a line of `page` holds.
fn columns_of(page: &wordcraft_doc::SectionProps) -> u32 {
    // Within the page's bounds, so the cast is exact.
    (page.text_width() / CODE_WIDTH + 0.001).floor().clamp(0.0, 1.0e9) as u32
}

impl Backend for Wordcraft {
    fn name(&self) -> &'static str {
        "wordcraft"
    }

    fn shows(&self) -> &'static str {
        "documents"
    }

    fn formats(&self) -> Vec<Format> {
        vec![WORD, ODT, RTF, MARKDOWN, HTML]
    }

    fn credits(&self) -> &[&'static str] {
        &["wordcraft", "pulldown-cmark"]
    }

    fn options(&self) -> &'static [OptionSpec] {
        OPTIONS
    }

    fn settings(&self) -> &'static [Setting] {
        &[PAPER, WIDTH, FONT, MARGIN]
    }

    fn opens(&self, path: &str, head: &[u8]) -> bool {
        Kind::of(path, head).is_some()
    }

    /// It lays out the whole document to open it.
    fn deferred(&self, path: &str, head: &[u8]) -> Option<Deferred> {
        Some(Deferred { settings: settings_of(Kind::of(path, head)?) })
    }

    /// Laid out for a room of this backend's own ([`ROOMLESS`]): the viewer gives one
    /// ([`Backend::open_at`]).
    fn open(&self, input: Input, options: &Parsed<'_>) -> Result<Opened, String> {
        self.open_at(input, options, ROOMLESS)
    }

    /// Markdown and HTML are laid out for `room`, the window's pages area; the rest have pages of their
    /// own.
    fn open_at(&self, input: Input, options: &Parsed<'_>, room: Room) -> Result<Opened, String> {
        opened(input, options, room).map(|paged| Opened::Pages(Box::new(paged)))
    }
}

/// The document `input` is, given `options`, laid out for `room` ([`Backend::open_at`]).
fn opened(input: Input, options: &Parsed<'_>, room: Room) -> Result<Paged, String> {
    let path = input.path.clone();
    let error = |e: String| format!("{path}: {e}");
    let kind = Kind::of(&path, &input.head).ok_or_else(|| error("not a document this reads".into()))?;
    let password = options.value("--password").map(String::from);
    let width = Width::given(options.value(WIDTH.option.name));
    let font = font_given(options.value(FONT.option.name));
    let margin = margin_given(options.value(MARGIN.option.name));
    let at = At { width, font, margin };
    let read_as = |bytes: &[u8], beside: Option<&Path>| {
        (caught(|| laid_out(bytes, kind, password.as_deref(), &path, beside, at, &room)), fingerprint_of(bytes))
    };
    let (stored, (laid, fingerprint)) = match input.reopenable()? {
        Reopenable::Disk(file) => {
            let bytes = read(file).map_err(error)?;
            (Stored::Disk(PathBuf::from(&path)), read_as(&bytes, Path::new(&path).parent()))
        }
        Reopenable::Bytes(bytes) => {
            let bytes = within(bytes).map_err(error)?;
            let laid = read_as(&bytes, None);
            (Stored::Memory(bytes), laid)
        }
        Reopenable::Again(again) => {
            let bytes = again().and_then(within).map_err(error)?;
            (Stored::Held(again), read_as(&bytes, None))
        }
    };
    // Only the pages' sizes are kept, and where their paragraphs are: the document goes, and a render
    // reads it again.
    let laid = laid.map_err(error)?;
    let (sizes, paras) = (sizes_of(&laid), paras_of(&laid, kind));
    let mut document = Document::new(stored, kind, password, path, sizes, (at, room), paras);
    document.fingerprint = fingerprint;
    Ok(Paged(Arc::new(document)))
}

/// A hash of a document's bytes, the same for the same bytes in this build ([`Texts::fingerprint`]).
fn fingerprint_of(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// The largest file this reads: 256 MiB.
const MAX_FILE: u64 = 256 << 20;

/// The largest Markdown or HTML file this reads: 64 MiB. A page may hold far more than it lays out:
/// a single-file page's scripts and pictures in `data:` URLs cost next to nothing, and what wordcraft
/// reads of its styles, it bounds ([ADR-0017](../../docs/adr/0017-htmls-style-rules-apply-as-far-as-wordcraft-reads-them.md)).
const MAX_MARKED_UP: usize = 64 << 20;

/// The most marks a Markdown or HTML file may have ([`marked_up`]): what it costs goes by them, up
/// to 9 KB each between importing and laying out.
const MAX_MARKS: usize = 500_000;

/// The most text of a Markdown or HTML file that is laid out: 8 MiB, some 5,000 pages, which takes
/// 20 seconds to open and 1.6 GB.
const MAX_TEXT: usize = 8 << 20;

/// The most pixels a page is drawn with: 64 million, 256 MiB.
const MAX_PIXELS: u64 = 64_000_000;

/// A Word document, by what it is named where its first bytes can't tell: a document, a
/// template, with macros or without, in either format, since Word opens each by what it holds.
const WORD: Format = Format::of("Word", &["docx", "docm", "dotx", "dotm", "doc", "dot"]);

/// OpenDocument text, by its name where its first file doesn't say: a document or a template.
const ODT: Format = Format::of("OpenDocument text", &["odt", "ott"]);

/// RTF, by its first bytes alone.
const RTF: Format = Format::of("RTF", &[]);

/// Markdown, by its name alone, since nothing in how it starts says so.
const MARKDOWN: Format = Format::of("Markdown", &["md", "markdown", "mdown", "mkd"]);

/// HTML, by how it starts, or by its name where that doesn't say (a fragment of a page).
const HTML: Format = Format::of("HTML", &["html", "htm", "xhtml"]);

/// What an OpenDocument text says it is, in its first file: a document or a template.
const ODT_TYPE: &[u8] = b"application/vnd.oasis.opendocument.text";

/// What kind of document a file is, which says how it is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A Word package (docx and the rest), a zip.
    Package,
    /// A compound file: a Word 97-2003 document, or a Word package protected with a password.
    Compound,
    /// OpenDocument text, a zip whose first file says so.
    Odt,
    Rtf,
    Markdown,
    Html,
}

impl Kind {
    /// The kind of the file at `path`, which starts with `head`, if this backend reads it. An RTF
    /// says so in its first bytes whatever its name (one is often saved as `.doc`), an odt in its
    /// first file. A Word package is a zip with a file in `word/` among the first, or named as a
    /// Word document; a compound file only when named so, since spreadsheets and slides are too.
    /// HTML says so by how it starts; and text may be Markdown or HTML when named so, which a PDF
    /// or a picture misnamed isn't.
    fn of(path: &str, head: &[u8]) -> Option<Self> {
        if head.starts_with(b"{\\rtf") {
            return Some(Self::Rtf);
        }
        if head.starts_with(ZIP) {
            if mimetype(head).is_some_and(|m| m.starts_with(ODT_TYPE)) || ODT.named(path) {
                return Some(Self::Odt);
            }
            let word = Names(head).any(|name| name.starts_with(b"word/"));
            return (word || WORD.named(path)).then_some(Self::Package);
        }
        if head.starts_with(COMPOUND) {
            return WORD.named(path).then_some(Self::Compound);
        }
        if starts_as_html(head) {
            return Some(Self::Html);
        }
        if !wordcraft_formats::txt::is_text(head) {
            return None;
        }
        MARKDOWN.named(path).then_some(Self::Markdown).or(HTML.named(path).then_some(Self::Html))
    }
}

/// Whether a file starting with `head` says it is HTML: a doctype or an `html` element, in any case,
/// after whatever may come first (a byte order mark, blank space, comments, an XML declaration).
fn starts_as_html(head: &[u8]) -> bool {
    let mut rest = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head).trim_ascii_start();
    loop {
        let end: &[u8] = match rest {
            [b'<', b'!', b'-', b'-', ..] => b"-->",
            [b'<', b'?', ..] => b"?>",
            _ => break,
        };
        let Some(at) = rest.windows(end.len()).position(|bytes| bytes == end) else { return false };
        rest = rest.get(at.saturating_add(end.len())..).unwrap_or_default().trim_ascii_start();
    }
    [&b"<!doctype html"[..], b"<html"].iter().any(|tag| {
        let named = rest.get(..tag.len()).is_some_and(|start| start.eq_ignore_ascii_case(tag));
        named && rest.get(tag.len()).is_none_or(|next| next.is_ascii_whitespace() || *next == b'>')
    })
}

/// An open document.
struct Paged(Arc<Document>);

impl Paged {
    /// Whether this lays out again, for a width and a room: Markdown and HTML do, having no page of
    /// their own.
    fn reflows(&self) -> bool {
        settings_of(self.0.kind).contains(&WIDTH.name)
    }
}

struct Document {
    stored: Stored,
    kind: Kind,
    password: Option<String>,
    /// As given, for the fields that show the file's name.
    path: String,
    /// Each page's, in points.
    sizes: Vec<(f32, f32)>,
    /// What it is laid out at, and the room it is laid out for, exactly as given; Markdown and HTML
    /// lay out again at another ([`Source::relayout`]). The rest, with pages of their own, keep them
    /// only to say what they were opened with.
    at: At,
    room: Room,
    /// Each page's paragraphs, for Markdown and HTML, which lay out again: where the reader is, found
    /// again in the new layout ([`Source::anchor`]). Empty for the rest.
    paragraphs: Vec<Vec<Para>>,
    /// Of every kind, each page's body paragraphs, first to last by their order in the document, and
    /// how many it has: a search's chunks ([`Paragraphs`]).
    on: Vec<Range<usize>>,
    count: usize,
    /// A hash of the bytes it was laid out from, here and in a relayout: what its text was read from
    /// ([`Texts::fingerprint`]).
    fingerprint: u64,
    /// The layout a pool renders with and a search reads, while either holds it: laid out once for
    /// both, and let go of with the last ([`Document::shared`]).
    shared: Mutex<Weak<Cache>>,
    /// How many times it was laid out to render or to search.
    #[cfg(test)]
    layouts: std::sync::atomic::AtomicUsize,
}

/// What Markdown and HTML are laid out at, as asked: the width, the font size (a percentage, [`FONT`])
/// and the side margins (characters, [`MARGIN`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct At {
    width: Width,
    font: u32,
    margin: u32,
}

impl At {
    /// What `values` ask for.
    fn wanted(values: &Values) -> Self {
        Self { width: Width::wanted(values), font: font_wanted(values), margin: margin_wanted(values) }
    }
}

/// A paragraph on a page: which, in the document's order, and where.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Para {
    index: u32,
    /// Its top on the page, and its height there, in points.
    y: f32,
    height: f32,
}

/// Each page's size, in points, of a document laid out.
fn sizes_of(laid: &Laid) -> Vec<(f32, f32)> {
    laid.layout.pages.iter().map(|page| (page.w, page.h)).collect()
}

/// What a document laid out says of its body's paragraphs: where each is on each page, for Markdown
/// and HTML, which lay out again; each page's, first to last; and how many there are.
struct Paras {
    paragraphs: Vec<Vec<Para>>,
    on: Vec<Range<usize>>,
    count: usize,
}

/// What `laid`, a document of `kind`, says of its body's paragraphs ([`Paras`]).
fn paras_of(laid: &Laid, kind: Kind) -> Paras {
    let (paragraphs, count) = paragraphs_of(laid);
    let indices: Vec<Vec<usize>> =
        paragraphs.iter().map(|page| page.iter().map(|para| usize::try_from(para.index).unwrap_or(usize::MAX)).collect()).collect();
    let on = ranges_of(&indices);
    let paragraphs = if settings_of(kind).contains(&WIDTH.name) { paragraphs } else { Vec::new() };
    Paras { paragraphs, on, count }
}

/// Each page's chunks for a search ([`Texts::chunks_on`]), from the paragraphs on it, by their order
/// in the document (`pages`): in order, each from the least paragraph on it or any page after to the
/// most on it or any page before. That is each page's own, but for a table row over several pages,
/// whose cells' paragraphs are side by side on each and one after another in the document: a page
/// it is on takes in the row's paragraphs on its neighbours too, so that no page's chunks start or
/// end before an earlier page's. A page with no paragraph has none.
fn ranges_of(pages: &[Vec<usize>]) -> Vec<Range<usize>> {
    let (mut least, mut most) = (vec![usize::MAX; pages.len()], vec![0usize; pages.len()]);
    let mut after = usize::MAX;
    for (page, slot) in pages.iter().zip(least.iter_mut()).rev() {
        after = after.min(page.iter().copied().min().unwrap_or(usize::MAX));
        *slot = after;
    }
    let mut before = None;
    for (page, slot) in pages.iter().zip(most.iter_mut()) {
        before = before.max(page.iter().copied().max().map(|last| last.saturating_add(1)));
        *slot = before.unwrap_or(0);
    }
    pages.iter().zip(least.into_iter().zip(most)).map(|(page, (first, end))| if page.is_empty() { 0..0 } else { first..end }).collect()
}

/// Each page's paragraphs of `laid`, in the order they are placed: the body's, by their order in the
/// document, with the lines each has on the page; and how many the body has.
fn paragraphs_of(laid: &Laid) -> (Vec<Vec<Para>>, usize) {
    let document = laid.document.read().unwrap_or_else(PoisonError::into_inner);
    let paths = document.para_paths(StoryRef::Body);
    let count = paths.len();
    let order: HashMap<_, u32> = paths.into_iter().zip(0u32..).collect();
    let placed = |item: &Placed| match item {
        Placed::Lines { story: StoryRef::Body, path, para, l0, l1, y, .. } => {
            let height = para.lines.get(*l0..*l1).unwrap_or_default().iter().map(|line| line.height).sum();
            order.get(path).map(|index| Para { index: *index, y: *y, height })
        }
        _ => None,
    };
    (laid.layout.pages.iter().map(|page| page.items.iter().filter_map(placed).collect()).collect(), count)
}

/// Where a document's bytes are.
#[derive(Clone)]
enum Stored {
    /// In a file on disk, read again to render.
    Disk(PathBuf),
    /// As they came through a pipe, which can't be read again.
    Memory(Vec<u8>),
    /// In another file (an archive), read from it again to render.
    Held(Again),
}

/// A document read and laid out.
struct Laid {
    /// Written to only to hold a picture a page wants, or let go of one: pages render side by side.
    document: RwLock<wordcraft_doc::Document>,
    layout: DocLayout,
    pictures: Promised,
}

impl Laid {
    /// `page`'s pictures drawn, for it to render (as a list's `tile`, or a page), and what the
    /// reader should know of them.
    fn drawn_pictures(&self, page: &Page, first: bool, tile: bool) -> Vec<String> {
        let mut notes = Vec::new();
        if first && self.pictures.left_out() > 0 {
            notes.push(format!("{} pictures it refers to are left out: missing, or not what {} shows", self.pictures.left_out(), crate::NAME));
        }
        match self.pictures.undrawn() {
            _ if !first => {}
            0 => {}
            1 => notes.push("one of its blocks could not be drawn, and is shown as its code".into()),
            undrawn => notes.push(format!("{undrawn} of its blocks could not be drawn, and are shown as their code")),
        }
        if self.pictures.is_empty() {
            return notes;
        }
        let on_page: Vec<String> = {
            let document = self.document.read().unwrap_or_else(PoisonError::into_inner);
            pictures_on(&document, page).into_iter().map(str::to_owned).collect()
        };
        let on_page: Vec<&str> = on_page.iter().map(String::as_str).collect();
        // Nothing locked while they are drawn, which may take a while; then, under one lock, held and
        // in the document at once, or another page could find one drawn and render its stand-in.
        let drawn = self.pictures.draw(&on_page, tile);
        if !drawn.is_empty() {
            let mut document = self.document.write().unwrap_or_else(PoisonError::into_inner);
            let changes = self.pictures.hold(drawn, &on_page, tile);
            document.media.extend(changes);
        }
        if self.pictures.failed(&on_page) {
            notes.push("a picture it refers to could not be drawn".into());
        }
        notes
    }
}

/// What `document` holds for the pictures on `page`, by key: of each paragraph on it, even one that
/// goes on to another page.
fn pictures_on<'a>(document: &'a wordcraft_doc::Document, page: &Page) -> Vec<&'a str> {
    let paragraph = |item: &Placed| match item {
        Placed::Lines { story, path, .. } => document.para(*story, path),
        _ => None,
    };
    let picture = |object: &'a InlineObject| match object {
        InlineObject::Image { media, .. } => Some(media.as_str()),
        _ => None,
    };
    page.items.iter().filter_map(paragraph).flat_map(|para| para.objects.iter()).filter_map(picture).collect()
}

/// A document laid out, by the first render of a renderer or a pool, or a search's first read: an
/// error, or a panic's, is kept too, so a document that fails isn't read again for every render. A
/// renderer (a list's tile) has its own; a pool and a search share one ([`Document::shared`]).
type Cache = OnceLock<Result<Arc<Laid>, String>>;

impl Source for Paged {
    fn pages(&self) -> usize {
        self.0.sizes.len()
    }

    fn size(&self, page: usize) -> Option<(f32, f32)> {
        self.0.sizes.get(page).copied()
    }

    /// A page scales: fitted to the window, it fills it, as a PDF's does.
    fn natural(&self, _page: usize) -> bool {
        false
    }

    /// Markdown and HTML have no paper of their own, unlike what is laid out to be printed.
    fn settings(&self) -> Vec<&'static str> {
        settings_of(self.0.kind).to_vec()
    }

    fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
        Ok(Box::new(Renderer { document: self.0.clone(), laid: Cache::new() }))
    }

    fn pool(&self, threads: usize) -> Box<dyn Pool> {
        let (document, laid) = (self.0.clone(), self.0.shared());
        Box::new(Threads::new(threads, Arc::new(move |request| document.render(request, &laid))))
    }

    /// Markdown and HTML say their width and their room; the rest have a page of their own.
    fn laid_at(&self) -> (Values, Option<Room>) {
        let mut values = Values::default();
        if !self.reflows() {
            return (values, None);
        }
        // There is room: nothing else is in it.
        let _ = values.set(WIDTH.name, self.0.at.width.value());
        let _ = values.set(FONT.name, Value::Number(self.0.at.font));
        let _ = values.set(MARGIN.name, Value::Number(self.0.at.margin));
        (values, Some(self.0.room))
    }

    /// Markdown and HTML at another width, font size or margin, or for another room: read again from
    /// where their bytes are, and laid out at it.
    fn relayout(&self, values: Values, room: Room) -> Option<Job> {
        let at = At::wanted(&values);
        if !self.reflows() || (at == self.0.at && room.same(&self.0.room)) {
            return None;
        }
        let document = self.0.clone();
        Some(Box::new(move || document.at(at, room).map(|paged| Box::new(paged) as Box<dyn Source + Send>)))
    }

    /// A page laid out for a room is as tall as that room at most: there a character of the code face
    /// is a cell, at the font size. A page of a document's own is shown at any scale.
    fn largest_scale(&self, page: usize) -> Option<f32> {
        if !self.reflows() {
            return None;
        }
        let (_, h) = self.0.sizes.get(page)?;
        let scale = f32::from(self.0.room.rows) * self.0.room.cell_h / h;
        (scale.is_finite() && scale > 0.0).then_some(scale)
    }

    /// Where `window` sits among the widths in `room` at the font size `values` want, at the characters
    /// its line holds there, and the most a width is laid out at, the longest line the room holds; and
    /// the most the font size goes to in `room`, the largest at which it holds a page.
    fn bounds(&self, setting: &str, room: Room, values: Values) -> Option<Bounds> {
        if !self.reflows() {
            return None;
        }
        // In order, each from the values as asked: the page's cells, the font's most there whatever the
        // margin, the margin's most at the font laid out at.
        let At { font, margin, .. } = At::wanted(&values);
        let font_most = font_most(&room);
        if setting == FONT.name {
            return Some(Bounds { named_at: None, most: Some(font_most) });
        }
        if setting == MARGIN.name {
            // Where even no margin leaves a line its 20 characters, it is laid out at one at most.
            return Some(Bounds { named_at: None, most: Some(margin_most(&room, font.min(font_most)).unwrap_or(1)) });
        }
        if setting != WIDTH.name {
            return None;
        }
        let columns = |width| columns_of(&page_in(width, &room, font, margin));
        Some(Bounds { named_at: Some(columns(Width::Window)), most: Some(columns(Width::Columns(u32::MAX))) })
    }

    /// The paragraph at `y` down `page`: the first that reaches below it, or the page's last.
    fn anchor(&self, page: usize, y: f32) -> Option<Anchor> {
        let paragraphs = self.0.paragraphs.get(page)?;
        let at = paragraphs.iter().find(|para| para.y + para.height > y).or(paragraphs.last())?;
        Some(Anchor(u64::from(at.index)))
    }

    /// The first page `anchor`'s paragraph is on, and its top there.
    fn find(&self, anchor: &Anchor) -> Option<(usize, f32)> {
        let index = u32::try_from(anchor.0).ok()?;
        self.0
            .paragraphs
            .iter()
            .enumerate()
            .find_map(|(page, paragraphs)| paragraphs.iter().find(|para| para.index == index).map(|para| (page, para.y.max(0.0))))
    }

    /// Its body's text, a paragraph a chunk, read from the layout the pool renders with.
    fn texts(&self) -> Option<Box<dyn Texts>> {
        let queue = Queue::new();
        queue.start("document text", Reader { document: self.0.clone(), laid: self.0.shared(), paths: None });
        Some(Box::new(Paragraphs { queue, document: self.0.clone() }))
    }
}

/// The most bytes of a paragraph's text a search reads: a longer one is left unread, as the viewer
/// leaves a longer chunk.
const MAX_PARAGRAPH: usize = 1 << 20;

/// The most rectangles a match is given, one a line.
const MAX_LINES: usize = 64;

/// A document's text for a search ([`Texts`]): a body paragraph a chunk, tables' cells included, read
/// on a queue's thread from the layout the pool renders with ([`Document::shared`]): no second layout.
struct Paragraphs {
    queue: Queue,
    document: Arc<Document>,
}

impl Texts for Paragraphs {
    fn chunks(&self) -> usize {
        self.document.count
    }

    fn chunks_on(&self, page: usize) -> Range<usize> {
        self.document.on.get(page).cloned().unwrap_or_default()
    }

    /// The bytes it was laid out from, hashed: a file changed on disk since is another text.
    fn fingerprint(&self) -> Option<u64> {
        Some(self.document.fingerprint)
    }

    fn set_queue(&self, wanted: Vec<Want>) {
        self.queue.set_queue(wanted);
    }

    fn try_recv(&self) -> Option<Got> {
        self.queue.try_recv()
    }
}

/// The queue's side of [`Paragraphs`]: the layout, laid out by the first to want it, a render or
/// this; and its body's paragraphs, in order, once read.
struct Reader {
    document: Arc<Document>,
    laid: Arc<Cache>,
    paths: Option<Vec<wordcraft_doc::Path>>,
}

/// Each piece of a paragraph's text as it is searched, with the bytes of [`Paragraph::text`] it
/// stands for: a character as it is, an inline object as its text ([`object_text`]) or nothing, a
/// soft hyphen as nothing, a non-breaking one as a hyphen, a column break as a space.
fn pieces(para: &Paragraph) -> impl Iterator<Item = (Cow<'_, str>, usize, usize)> {
    let mut objects = para.objects.iter();
    para.text.char_indices().map(move |(at, c)| {
        let end = at.saturating_add(c.len_utf8());
        let piece = match c {
            OBJ => objects.next().map_or(Cow::Borrowed(""), object_text),
            SOFT_HYPHEN => Cow::Borrowed(""),
            NB_HYPHEN => Cow::Borrowed("-"),
            COLUMN_BREAK => Cow::Borrowed(" "),
            _ => Cow::Borrowed(para.text.get(at..end).unwrap_or("")),
        };
        (piece, at, end)
    })
}

/// What stands for a field left out of a search: a character no query typed holds, so that no
/// match runs across the field, and that maps to the field's bytes as any object does.
const LEFT_OUT: &str = "\u{FFFC}";

/// An inline object's text as it is searched: its `plain_text()`, but a field as the page shows it,
/// asked as wordcraft's layout asks: shown as its code, the code in braces; a legacy drop-down, its
/// entry shown; then, through `field_text_on`, its saved result, or nothing for `ASK` and `SET`. A
/// field whose text the page doesn't show as text, or that the layout computes, is left out
/// ([`LEFT_OUT`]): a legacy check box, an overstrike (`EQ \o`), one that depends on where it lands
/// (`PAGE`, `NUMPAGES`, `SECTION`, …), and one with nothing saved, whose text (`TITLE`, `FILENAME`)
/// comes from the document. A search never finds text the page doesn't show.
fn object_text(object: &InlineObject) -> Cow<'_, str> {
    use wordcraft_doc::legacy_form::LegacyForm;
    let InlineObject::Field { instr, result, code, .. } = object else { return Cow::Borrowed(object.plain_text()) };
    // Printed, fields show their results ([`LayoutOptions::field_codes`]): one marked otherwise, its code.
    if *code {
        return Cow::Owned(wordcraft_layout::fields::field_code_text(instr));
    }
    match object.legacy_form() {
        Some(LegacyForm::CheckBox { .. }) => return Cow::Borrowed(LEFT_OUT),
        Some(form) => {
            if let Some(shown) = form.shown_text() {
                return Cow::Owned(shown.to_owned());
            }
        }
        None => {}
    }
    if wordcraft_doc::eq::parse_overstrike(instr).is_some() {
        return Cow::Borrowed(LEFT_OUT);
    }
    let shown = caught(|| Ok(wordcraft_layout::fields::field_text_on(instr, result, &wordcraft_layout::FieldCtx::default(), 1)));
    match shown {
        Ok((shown, false)) if !result.is_empty() => Cow::Owned(shown),
        _ => Cow::Borrowed(LEFT_OUT),
    }
}

/// The bytes of `para`'s text that the characters `from..to` of its text as searched ([`pieces`])
/// stand for: from where the piece the first is in starts to where the one the last is in ends.
/// `None` for a range of none of them.
fn bytes_of(para: &Paragraph, from: usize, to: usize) -> Option<(usize, usize)> {
    let (mut at, mut start, mut end) = (0usize, None, None);
    for (piece, first, past) in pieces(para) {
        if at >= to {
            break;
        }
        let next = at.saturating_add(piece.chars().count());
        if next > at && next > from {
            start.get_or_insert(first);
            end = Some(past);
        }
        at = next;
    }
    start.zip(end)
}

impl Reader {
    /// The layout, laid out now if nothing has yet, and the body's paragraphs, in order.
    fn laid(&mut self) -> Result<(Arc<Laid>, &[wordcraft_doc::Path]), String> {
        let laid = self.document.laid(&self.laid)?;
        if self.paths.is_none() {
            let document = laid.document.read().unwrap_or_else(PoisonError::into_inner);
            self.paths = Some(caught(|| Ok(document.para_paths(StoryRef::Body)))?);
        }
        Ok((laid, self.paths.as_deref().unwrap_or_default()))
    }

    /// The first page paragraph `chunk` is on (0-based).
    fn page_of(&self, chunk: usize) -> usize {
        self.document.on.iter().position(|on| on.contains(&chunk)).unwrap_or(0)
    }
}

impl Answer for Reader {
    fn text(&mut self, chunk: usize) -> Result<String, String> {
        let (laid, paths) = self.laid()?;
        let document = laid.document.read().unwrap_or_else(PoisonError::into_inner);
        let mut text = String::new();
        let mut long = false;
        if let Some(para) = paths.get(chunk).and_then(|path| document.para(StoryRef::Body, path)) {
            for (piece, ..) in pieces(para) {
                long |= text.len().saturating_add(piece.len()) > MAX_PARAGRAPH;
                if long {
                    break;
                }
                text.push_str(&piece);
            }
        } else {
            let page = self.page_of(chunk).saturating_add(1);
            return Err(format!("a paragraph on page {page} is no longer in the file: open it again to search it"));
        }
        if long {
            let page = self.page_of(chunk).saturating_add(1);
            return Err(format!("a paragraph on page {page} has more text than a search keeps of one"));
        }
        Ok(text)
    }

    /// Wordcraft's own selection of the characters `from..to`: a rectangle for each line, or each
    /// piece of a bidirectional one, the hyphen drawn at a line's end included, on each page it runs
    /// over, in points from the page's top left.
    fn rects(&mut self, chunk: usize, from: usize, to: usize) -> Result<Vec<(usize, [f32; 4])>, String> {
        let hint = self.page_of(chunk);
        let (laid, paths) = self.laid()?;
        let document = laid.document.read().unwrap_or_else(PoisonError::into_inner);
        let Some(path) = paths.get(chunk) else { return Ok(Vec::new()) };
        let Some((start, end)) = document.para(StoryRef::Body, path).and_then(|para| bytes_of(para, from, to)) else { return Ok(Vec::new()) };
        let (a, b) = (Pos::new(StoryRef::Body, path.clone(), start), Pos::new(StoryRef::Body, path.clone(), end));
        let rects = caught(|| Ok(laid.layout.selection_rects(&document, &a, &b, hint)))?;
        Ok(rects.into_iter().take(MAX_LINES).map(|(page, r)| (page, [r.x, r.y, r.x + r.w, r.y + r.h])).collect())
    }
}

struct Renderer {
    document: Arc<Document>,
    laid: Cache,
}

impl Render for Renderer {
    fn render(&mut self, request: Request) -> Frame {
        guarded(request, || self.document.render(request, &self.laid))
    }
}

impl Document {
    /// What `stored` holds, of `kind`, laid out `at` for a room into pages of `sizes`, with `paras`.
    fn new(stored: Stored, kind: Kind, password: Option<String>, path: String, sizes: Vec<(f32, f32)>, (at, room): (At, Room), paras: Paras) -> Self {
        let Paras { paragraphs, on, count } = paras;
        Self {
            stored,
            kind,
            password,
            path,
            sizes,
            at,
            room,
            paragraphs,
            on,
            count,
            fingerprint: 0,
            shared: Mutex::new(Weak::new()),
            #[cfg(test)]
            layouts: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// The layout a pool and a search share: the one either holds, or a new one, not yet laid out.
    fn shared(&self) -> Arc<Cache> {
        let mut shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        shared.upgrade().unwrap_or_else(|| {
            let cache = Arc::new(Cache::new());
            *shared = Arc::downgrade(&cache);
            cache
        })
    }

    /// It laid out, once for `laid`: by whoever asks first, the others waiting for it.
    fn laid(&self, laid: &Cache) -> Result<Arc<Laid>, String> {
        laid.get_or_init(|| {
            #[cfg(test)]
            self.layouts.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let bytes = self.bytes()?;
            caught(|| laid_out(&bytes, self.kind, self.password.as_deref(), &self.path, self.beside(), self.at, &self.room)).map(Arc::new)
        })
        .clone()
    }

    fn render(&self, request: Request, laid: &Cache) -> Frame {
        self.rendered(request, laid).unwrap_or_else(|e| Frame::failed(request, e))
    }

    fn rendered(&self, request: Request, laid: &Cache) -> Result<Frame, String> {
        let laid = self.laid(laid)?;
        let page = laid.layout.pages.get(request.page).ok_or_else(|| format!("the document has no page {}", request.page.saturating_add(1)))?;
        let (w, h) = (pixels(page.w, request.scale)?, pixels(page.h, request.scale)?);
        let side = u64::from(wordcraft_render::MAX_SIDE);
        if w > side || h > side || w.saturating_mul(h) > MAX_PIXELS {
            return Err(format!("{w} × {h} pixels is too large to render"));
        }
        let warnings = laid.drawn_pictures(page, request.page == 0, request.tile);
        let mut options = RenderOptions::default();
        // A printed page: headers and footers dimmed only while the body is being edited.
        options.display.dim_header = false;
        let dark = settings_of(self.kind).contains(&PAPER.name) && request.values.get(PAPER.name) == Some(Value::Named(DARK));
        let dark = dark.then_some(request.dark);
        if let Some(paper) = dark {
            options.dark = true;
            options.dark_paper = grey_of(paper);
        }
        let document = laid.document.read().unwrap_or_else(PoisonError::into_inner);
        let mut drawn = wordcraft_render::render_page(&document, page, request.scale, &options);
        if let Some(paper) = dark {
            on_paper(&mut drawn.pixels, paper);
        }
        // Premultiplied RGBA, row by row, as a frame's are.
        Ok(Frame { request, width: drawn.width, height: drawn.height, rgba: Pixels::new(drawn.pixels), error: None, warnings, again: false })
    }

    /// This document laid out again `at` a width, a font size and margins, for `room`, from its bytes: a
    /// new one, this one as it is.
    fn at(&self, at: At, room: Room) -> Result<Paged, String> {
        let bytes = self.bytes()?;
        let laid = caught(|| laid_out(&bytes, self.kind, self.password.as_deref(), &self.path, self.beside(), at, &room))?;
        let Self { stored, kind, password, path, .. } = self;
        let (stored, password, path) = (stored.clone(), password.clone(), path.clone());
        let (sizes, paras) = (sizes_of(&laid), paras_of(&laid, *kind));
        let mut document = Document::new(stored, *kind, password, path, sizes, (at, room), paras);
        document.fingerprint = fingerprint_of(&bytes);
        Ok(Paged(Arc::new(document)))
    }

    /// The directory it is in, if it is a file on disk: what the pictures it refers to are near.
    fn beside(&self) -> Option<&Path> {
        match &self.stored {
            Stored::Disk(path) => path.parent(),
            Stored::Memory(_) | Stored::Held(_) => None,
        }
    }

    /// Its bytes: the ones kept, or the file's again.
    fn bytes(&self) -> Result<Cow<'_, [u8]>, String> {
        let again = match &self.stored {
            Stored::Memory(bytes) => return Ok(Cow::Borrowed(&bytes[..])),
            Stored::Disk(path) => File::open(path).map_err(|e| e.to_string()).and_then(read),
            Stored::Held(again) => again().and_then(within),
        };
        again.map(Cow::Owned).map_err(|e| format!("reading the file again: {e}"))
    }
}

/// The grey as bright as `colour`: what wordcraft's dark page can be.
fn grey_of([r, g, b]: [u8; 3]) -> u8 {
    let luma = (2126 * u32::from(r) + 7152 * u32::from(g) + 722 * u32::from(b)) / 10000;
    u8::try_from(luma).unwrap_or(u8::MAX)
}

/// `pixels`, a dark page on the grey of `paper`, on `paper` itself, so the page is exactly the colour
/// behind it: each channel stretched to keep its white and meet that colour at the grey. A picture on
/// the page shifts with it, as little as the colour is from grey.
fn on_paper(pixels: &mut [u8], paper: [u8; 3]) {
    let grey = grey_of(paper);
    if paper == [grey; 3] {
        return;
    }
    let stretch = |to: u8| -> [u8; 256] {
        let (grey, to) = (u32::from(grey), u32::from(to));
        std::array::from_fn(|from| {
            // Within a byte: `from` is below 256, and each stays between its two ends.
            let from = from as u32;
            let at = if from <= grey { from * to / grey.max(1) } else { to + (from - grey) * (255 - to) / (255 - grey).max(1) };
            at.min(255) as u8
        })
    };
    let [r, g, b] = paper.map(stretch);
    for pixel in pixels.as_chunks_mut::<4>().0 {
        // Each channel a byte, so each index is within its table.
        *pixel = [r[usize::from(pixel[0])], g[usize::from(pixel[1])], b[usize::from(pixel[2])], pixel[3]];
    }
}

/// `size` points at `scale`, in whole pixels, as wordcraft rounds them.
fn pixels(size: f32, scale: f32) -> Result<u64, String> {
    let scaled = (f64::from(size) * f64::from(scale)).ceil();
    if !(scale > 0.0 && scaled.is_finite() && scaled <= 1e15) {
        return Err(format!("can't render at scale {scale}"));
    }
    // In range: checked just above.
    Ok((scaled as u64).max(1))
}

/// The document in `bytes`, of `kind`, read with `password` if it is protected, and laid out into
/// pages: Markdown and HTML `at` a width, a font size and margins on the page `room` gives them ([`page_in`]),
/// the rest on pages of their own. `path` is the file's, for the fields that show it, and `beside` the
/// directory it is in, for the pictures it refers to.
fn laid_out(bytes: &[u8], kind: Kind, password: Option<&str>, path: &str, beside: Option<&Path>, at: At, room: &Room) -> Result<Laid, String> {
    let pictures = Pictures::beside(beside);
    let read = match kind {
        Kind::Package => wordcraft_docx::read_with_password(bytes, password).map_err(|e| e.to_string()),
        Kind::Compound if wordcraft_docx::is_encrypted(bytes) => wordcraft_docx::read_with_password(bytes, password).map_err(|e| e.to_string()),
        Kind::Compound => wordcraft_docbin::read_with_password(bytes, password).map_err(|e| e.to_string()),
        Kind::Odt => imported("odt", bytes),
        Kind::Rtf => imported("rtf", bytes),
        Kind::Markdown => markdown(bytes, &pictures, page_in(at.width, room, at.font, at.margin)),
        Kind::Html => html(bytes, &pictures, page_in(at.width, room, at.font, at.margin)),
    };
    let mut document = read.map_err(|e| match e == wordcraft_docx::PASSWORD_REQUIRED {
        true => format!("{e}: give it with --password"),
        false => e,
    })?;
    document.ensure_nonempty();
    let name = Path::new(path).file_name().map_or_else(|| path.into(), |name| name.to_string_lossy());
    let options = LayoutOptions { view: ViewMode::Print, filename: Some(Arc::from(&*name)), filepath: Some(Arc::from(path)), ..Default::default() };
    let layout = wordcraft_layout::layout(&document, &mut LayoutCache::new(), &options);
    if layout.pages.is_empty() {
        return Err("the document has no pages to show".into());
    }
    let pictures = pictures.promised_in(&document.media);
    Ok(Laid { document: RwLock::new(document), layout, pictures })
}

/// The document in `bytes`, in the format `extension` names, through wordcraft's importers.
fn imported(extension: &str, bytes: &[u8]) -> Result<wordcraft_doc::Document, String> {
    let page = wordcraft_doc::SectionProps::default();
    wordcraft_formats::import_on(extension, bytes, &page).unwrap_or_else(|| Err(format!("wordcraft reads no .{extension}")))
}

/// The Markdown in `bytes`, where any punctuation or line may start a run, a cell or a paragraph, on
/// `page`.
fn markdown(bytes: &[u8], pictures: &Pictures<'_>, page: wordcraft_doc::SectionProps) -> Result<wordcraft_doc::Document, String> {
    let fit = (page.text_width(), page.text_height());
    let as_html = |bytes: &[u8]| markdown_as_html(&wordcraft_formats::txt::decode(bytes), pictures, fit);
    marked_up(|b| b.is_ascii_punctuation() || matches!(b, b'\r' | b'\n'), bytes, as_html, pictures, page)
}

/// The HTML in `bytes`, where a tag or, in a `pre`, a line may, on `page`.
fn html(bytes: &[u8], pictures: &Pictures<'_>, page: wordcraft_doc::SectionProps) -> Result<wordcraft_doc::Document, String> {
    marked_up(|b| matches!(b, b'<' | b'\r' | b'\n'), bytes, wordcraft_formats::html::decode, pictures, page)
}

/// `text`, Markdown, as HTML for wordcraft's importer to read
/// ([ADR-0014](../../docs/adr/0014-markdown-through-pulldown-cmark-into-wordcrafts-html-importer.md)):
/// GitHub's tables, strikethrough, task lists, footnotes and alerts, with what it would leave out
/// written otherwise, a table's grid, a task's checkbox and what an alert is, and front matter (`---`)
/// left out. A fenced block written in a language a backend draws (` ```mermaid `) is a picture of
/// its text among `pictures`, to fit in `fit`, or else the code it would have been: which, is known
/// here, before any HTML of it is written.
fn markdown_as_html(text: &str, pictures: &Pictures<'_>, fit: (f32, f32)) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS;
    let mut alert = None;
    // The fenced block being read, if a backend draws its language: its info string, and its text so far.
    let mut fence: Option<(pulldown_cmark::CowStr<'_>, String)> = None;
    let events = Parser::new_ext(text, options).flat_map(|event| {
        // What an alert is goes on the line above its first paragraph's text, or else alone.
        let (before, after) = match (alert.take(), &event) {
            (Some(kind), Event::Start(Tag::Paragraph)) => (None, Some(Event::InlineHtml(format!("{}<br>", alert_label(kind)).into()))),
            (Some(kind), _) => (Some(Event::Html(format!("<p>{}</p>", alert_label(kind)).into())), None),
            (None, _) => (None, None),
        };
        if let Event::Start(Tag::BlockQuote(Some(kind))) = &event {
            alert = Some(*kind);
        }
        // A block to draw is gathered whole, then written as its picture, or as the code it is.
        let [event, text, end] = match (fence.take(), event) {
            (None, Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))))
                if language_of(&info).is_some_and(|language| pictures.draws(language)) =>
            {
                fence = Some((info, String::new()));
                [None, None, None]
            }
            (Some((info, mut text)), Event::Text(more)) => {
                text.push_str(&more);
                fence = Some((info, text));
                [None, None, None]
            }
            (Some((info, text)), Event::End(TagEnd::CodeBlock)) => {
                match language_of(&info).and_then(|language| pictures.fenced(language, &text, fit)) {
                    Some(key) => [Some(Event::Html(format!("<p><img src=\"{key}\" alt=\"\"></p>\n").into())), None, None],
                    None => [
                        Some(Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))),
                        Some(Event::Text(text.into())),
                        Some(Event::End(TagEnd::CodeBlock)),
                    ],
                }
            }
            // Nothing but text comes in a code block: anything else would be passed on, the block's
            // text so far left out.
            (_, Event::TaskListMarker(true)) => [Some(Event::Text("☑ ".into())), None, None],
            (_, Event::TaskListMarker(false)) => [Some(Event::Text("☐ ".into())), None, None],
            (_, other) => [Some(other), None, None],
        };
        [before, event, text, end, after].into_iter().flatten()
    });
    let mut out = String::with_capacity(text.len());
    pulldown_cmark::html::push_html(&mut out, events);
    numbers_in_their_footnotes(&out.replace("<table>", "<table border=\"1\">"))
}

/// The language a fenced block is written in: the first word of its info string (` ```mermaid title="x" `
/// is `mermaid`), if it has one.
fn language_of(info: &str) -> Option<&str> {
    info.split_whitespace().next()
}

/// What an alert (`> [!NOTE]`) says it is, and its colour.
fn alert(kind: BlockQuoteKind) -> (&'static str, Rgb) {
    match kind {
        BlockQuoteKind::Note => ("Note", LINK),
        BlockQuoteKind::Tip => ("Tip", Rgb(0x1A, 0x7F, 0x37)),
        BlockQuoteKind::Important => ("Important", Rgb(0x4F, 0x2D, 0x94)),
        BlockQuoteKind::Warning => ("Warning", Rgb(0x9A, 0x67, 0x00)),
        BlockQuoteKind::Caution => ("Caution", Rgb(0xA4, 0x0E, 0x26)),
    }
}

/// What an alert is, in its colour, for the top of its quote.
fn alert_label(kind: BlockQuoteKind) -> String {
    let (name, Rgb(r, g, b)) = alert(kind);
    format!("<strong style=\"color:#{r:02X}{g:02X}{b:02X}\">{name}</strong>")
}

/// `html` with each footnote's number in its first paragraph, in smaller text, rather than on a line of
/// its own above it. What is written by hand to look like one, but isn't, stays as it is. Only what comes
/// right after each number is looked at, so a file of them takes no longer than a file of anything.
fn numbers_in_their_footnotes(html: &str) -> String {
    const NUMBER: &str = "<sup class=\"footnote-definition-label\">";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some((before, after)) = rest.split_once(NUMBER) {
        out.push_str(before);
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        // ASCII digits, so that's a char boundary.
        let (number, text) = after.split_at(digits);
        match text.strip_prefix("</sup>\n<p>") {
            Some(text) => {
                out.push_str("<p style=\"font-size:9.5pt\"><sup>");
                out.push_str(number);
                out.push_str("</sup> ");
                rest = text;
            }
            None => {
                out.push_str(NUMBER);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The Markdown or HTML in `bytes`, unless it costs too much to read and lay out
/// ([ADR-0012](../../docs/adr/0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md)).
/// `mark` says which bytes may start something of its own, counted before it is imported, since
/// importing them is what costs; its text, what laying it out costs, can only be told after, since
/// a page's scripts and styles aren't text. Laid out on `page`.
fn marked_up(
    mark: impl Fn(&u8) -> bool,
    bytes: &[u8],
    as_html: impl FnOnce(&[u8]) -> String,
    pictures: &Pictures<'_>,
    page: wordcraft_doc::SectionProps,
) -> Result<wordcraft_doc::Document, String> {
    if bytes.len() > MAX_MARKED_UP {
        return Err(format!("the document is more than {} MiB", MAX_MARKED_UP >> 20));
    }
    if bytes.iter().filter(|b| mark(b)).count() > MAX_MARKS {
        return Err(format!("the document has more than {MAX_MARKS} lines and marks"));
    }
    let fit = (page.text_width(), page.text_height());
    // As a fragment, which takes the document's own formatting, here `styled`'s: `import_on` reads a web
    // page, as Word opens one, Times New Roman and no code style
    // ([ADR-0017](../../docs/adr/0017-htmls-style-rules-apply-as-far-as-wordcraft-reads-them.md)). The
    // page goes into the conversion, since tables size to its width there; `import_with` would give them
    // the default page's. (What that entry point does first, `begin_import`, only starts wordcraft's log of
    // what an import dropped afresh, and is the crate's own.)
    let flow = wordcraft_formats::html::parse_with(&as_html(bytes), &|source: &str| pictures.measured(source, fit));
    let mut document = wordcraft_formats::model::to_doc_in(&flow, page);
    let measured: HashMap<&str, Measured> = document
        .media
        .iter()
        .filter_map(|(key, held)| {
            let shown = pictures.shown(held)?;
            let read = wordcraft_formats::model::make_img(held.clone(), None, None, "")?;
            Some((key.as_str(), Measured { read: (read.w, read.h), shown }))
        })
        .collect();
    fitted(&mut document.body, &measured, fit);
    styled(&mut document);
    let body = StoryRef::Body;
    let paragraphs = document.para_paths(body);
    let text = paragraphs.iter().filter_map(|path| document.para(body, path)).fold(0, |all: usize, para| all.saturating_add(para.text.len()));
    if text > MAX_TEXT {
        return Err(format!("the document has more than {} MiB of text", MAX_TEXT >> 20));
    }
    Ok(document)
}

/// The faces of a Markdown or HTML page's text and code, which come with wordcraft, so that a page
/// looks the same on any machine.
const TEXT_FACE: &str = "Source Sans 3";
const CODE_FACE: &str = "JetBrains Mono";

/// The size of its text, and of a code block's, at which 80 columns fit on a line of the block.
const TEXT_SIZE: f32 = 11.0;
const CODE_SIZE: f32 = 9.4;

/// How wide a character of the code face is, in ems: a column is this much of [`CODE_SIZE`].
const CODE_ADVANCE: f32 = 0.6;

/// The room below a paragraph of text.
const TEXT_AFTER: f32 = 9.0;

/// The size of code among text, for the size of that text: a monospaced face looks the larger.
const CODE_AMONG_TEXT: f32 = 0.88;

/// Its colours, for white paper: wordcraft's dark page turns each into what suits dark paper.
const INK: Rgb = Rgb(0x1F, 0x23, 0x28);
const QUIET_INK: Rgb = Rgb(0x59, 0x63, 0x6E);
const HAIRLINE: Rgb = Rgb(0xD0, 0xD7, 0xDE);
const WASH: Rgb = Rgb(0xF6, 0xF8, 0xFA);
const LINK: Rgb = Rgb(0x09, 0x69, 0xDA);

/// Each heading's size, the space before and after it, and whether a line runs under it.
const HEADINGS: [(f32, f32, f32, bool); 6] = [
    (23.0, 18.0, 11.0, true),
    (17.5, 18.0, 9.0, true),
    (14.0, 15.0, 6.0, false),
    (12.0, 13.0, 4.0, false),
    (11.0, 11.0, 4.0, false),
    (10.0, 11.0, 4.0, false),
];

/// How `document`, Markdown or HTML just imported, looks: what the importer gives is a word processor's
/// defaults, and such a file says little of its own
/// ([ADR-0016](../../docs/adr/0016-how-markdown-and-html-look.md)). What its own CSS says stays,
/// being on the text itself.
fn styled(document: &mut wordcraft_doc::Document) {
    let colour = |rgb| Some(TextColor::Rgb(rgb));
    let line = |width, color, space| Border { style: BorderStyle::Single, width, color: Some(color), space };
    let styles = &mut document.styles;
    styles.default_chr.font = Some(TEXT_FACE.into());
    styles.default_chr.size = Some(TEXT_SIZE);
    styles.default_chr.color = colour(INK);
    styles.default_para.line_spacing = Some(LineSpacing::Multiple(1.15));
    styles.default_para.space_after = Some(TEXT_AFTER);
    for (level, (size, before, after, ruled)) in HEADINGS.into_iter().enumerate() {
        if let Some(heading) = styles.get_mut(&format!("Heading{}", level + 1)) {
            heading.para.space_before = Some(before);
            heading.para.space_after = Some(after);
            heading.para.line_spacing = Some(LineSpacing::Multiple(1.05));
            heading.para.borders = ruled.then(|| Borders { bottom: Some(line(0.5, HAIRLINE, 3.0)), ..Default::default() });
            heading.chr.font = Some(TEXT_FACE.into());
            heading.chr.size = Some(size);
            heading.chr.bold = Some(true);
            heading.chr.italic = None;
            heading.chr.color = colour(if level == 5 { QUIET_INK } else { INK });
        }
    }
    if let Some(quote) = styles.get_mut("Quote") {
        quote.para.align = Some(wordcraft_doc::Align::Left);
        quote.para.space_before = Some(0.0);
        // The bar on the left edge of the text, from its distance and half its width.
        quote.para.indent_left = Some(11.25);
        quote.para.borders = Some(Borders { left: Some(line(2.5, HAIRLINE, 10.0)), ..Default::default() });
        quote.chr.italic = None;
        quote.chr.color = colour(QUIET_INK);
    }
    // Each line of a code block is a paragraph: alike, they are one box, its edges the colour of its
    // wash for the room they keep around the code, which is indented by that room to keep it in the column.
    if let Some(code) = styles.get_mut("Code") {
        code.para.space_before = Some(4.0);
        code.para.space_after = Some(14.0);
        code.para.contextual_spacing = Some(true);
        code.para.line_spacing = Some(LineSpacing::Multiple(1.05));
        code.para.indent_left = Some(8.0);
        code.para.indent_right = Some(8.0);
        code.para.shading = Some(WASH);
        code.para.borders = Some(Borders::all(line(0.5, WASH, 8.0)));
        code.chr.font = Some(CODE_FACE.into());
        code.chr.size = Some(CODE_SIZE);
    }
    if let Some(item) = styles.get_mut("ListParagraph") {
        item.para.contextual_spacing = Some(false);
        item.para.space_after = Some(2.0);
    }
    if let Some(parts) = styles.get_mut("TableGrid").and_then(|grid| grid.table.as_mut()) {
        parts.borders = Some(Borders::all(line(0.5, HAIRLINE, 0.0)));
        parts.cell_margins = Some([3.5, 6.0, 3.5, 6.0]);
        parts.header_fill = Some(WASH);
        parts.header_chr.bold = Some(true);
    }
    for level in document.numbering.abstracts.iter_mut().flat_map(|list| list.levels.iter_mut().enumerate()) {
        let (depth, level) = level;
        level.indent = 20.0 + 18.0 * depth as f32;
        level.hanging = 14.0;
        level.tab = None;
    }
    if let Some(Block::Para(first)) = document.body.first_mut().map(Arc::make_mut) {
        first.props.space_before = Some(0.0);
    }
    styled_blocks(&mut document.body, &document.numbering);
}

/// What a style can't say, in `blocks`: what depends on the blocks beside one, or on the text in it.
fn styled_blocks(blocks: &mut [Arc<Block>], numbering: &wordcraft_doc::Numbering) {
    let list_of = |block: Option<&Arc<Block>>| match block.map(|block| &**block) {
        Some(Block::Para(para)) => para.props.numbering.map(|list| list.num),
        _ => None,
    };
    for at in 0..blocks.len() {
        let next_list = list_of(blocks.get(at + 1));
        let after_table = at.checked_sub(1).and_then(|before| blocks.get(before)).is_some_and(|block| matches!(**block, Block::Table(_)));
        let Some(block) = blocks.get_mut(at) else { break };
        match Arc::make_mut(block) {
            Block::Para(para) => {
                styled_para(para, next_list, after_table, numbering);
                para.touch();
            }
            Block::Table(table) => {
                for cell in table.rows.iter_mut().flat_map(|row| row.cells.iter_mut()) {
                    styled_blocks(&mut cell.blocks, numbering);
                }
            }
        }
    }
}

/// `para`, before what may be a block of the list `next_list`, and maybe after a table.
fn styled_para(para: &mut wordcraft_doc::Paragraph, next_list: Option<u32>, after_table: bool, numbering: &wordcraft_doc::Numbering) {
    let heading = para.props.style.as_deref().and_then(|style| style.strip_prefix("Heading")).and_then(|level| level.parse::<usize>().ok());
    let text_size = heading.and_then(|level| HEADINGS.get(level.wrapping_sub(1))).map_or(TEXT_SIZE, |heading| heading.0);
    for run in &mut para.runs {
        // Code among text is what the importer gives this font. Its face alone says so: a shading
        // would show a seam between each two letters (ADR-0016).
        if run.props.font.as_deref() == Some(wordcraft_formats::model::MONO_FONT) {
            run.props.font = Some(CODE_FACE.into());
            run.props.size = Some((run.props.size.unwrap_or(text_size) * CODE_AMONG_TEXT).max(1.0));
        }
        if run.props.link.is_some() && run.props.color.is_none() {
            run.props.color = Some(TextColor::Rgb(LINK));
        }
    }
    let list = para.props.numbering.map(|list| list.num);
    if list.is_some() && list != next_list {
        para.props.space_after = Some(TEXT_AFTER);
    }
    // A task's box takes its bullet's place, and its text the text's.
    if ["☑ ", "☐ "].iter().any(|task| para.text.starts_with(task)) {
        let level = para.props.numbering.and_then(|list| numbering.abstract_of(list.num)?.levels.get(usize::from(list.level)));
        if let Some(level) = level.filter(|level| level.format == wordcraft_doc::section::NumFormat::Bullet) {
            para.props.indent_left = Some(level.indent);
            para.props.indent_first = Some(-level.hanging);
            para.props.numbering = None;
            // A box is three bytes, and its space is the fourth, as just checked. The same length, so
            // the runs stay as they are.
            para.text.replace_range(3..4, "\t");
        }
    }
    // A rule is a paragraph of nothing with a line under it.
    if para.text.is_empty()
        && let Some(bottom) = para.props.borders.as_mut().and_then(|borders| borders.bottom.as_mut())
    {
        bottom.color = Some(HAIRLINE);
        para.props.space_before = Some(6.0);
        para.props.space_after = Some(14.0);
    }
    if after_table && heading.is_none() {
        para.props.space_before = Some(14.0);
    }
}

/// A picture a document refers to, as [`Pictures::measured`] sized it: what wordcraft's reader makes
/// of its stand-in when the document gives it no size, and the size, in points, it was measured to be
/// shown at.
struct Measured {
    read: (f32, f32),
    shown: (f32, f32),
}

/// Every picture within the column and the page, `fit` (the text's width and height), keeping its shape.
/// wordcraft's reader holds a picture to a US Letter column, 468 points, whatever the page, and takes
/// its size from its stand-in's whole pixels: one the document gives no size to gets back the size it
/// was measured for (`measured`, by key), which is within both already. Any other, one the document
/// gives a size to or holds itself, is shrunk to fit: laid out as it is, it would be cut off at the
/// side of the column or the foot of the page.
fn fitted(blocks: &mut [Arc<Block>], measured: &HashMap<&str, Measured>, fit: (f32, f32)) {
    for block in blocks {
        match Arc::make_mut(block) {
            Block::Para(para) => {
                let mut changed = false;
                for object in &mut para.objects {
                    let InlineObject::Image { media, w, h, float, .. } = object else { continue };
                    if float.wrap != Wrap::Inline {
                        continue;
                    }
                    let (width, height) = match measured.get(media.as_str()) {
                        Some(picture) if (picture.read.0 - *w).abs() < 0.01 && (picture.read.1 - *h).abs() < 0.01 => picture.shown,
                        _ => (*w, *h),
                    };
                    let shrink = (fit.0 / width).min(fit.1 / height).min(1.0);
                    if !(shrink.is_finite() && shrink > 0.0) {
                        continue;
                    }
                    let size = ((width * shrink).max(1.0), (height * shrink).max(1.0));
                    if size != (*w, *h) {
                        (*w, *h) = size;
                        changed = true;
                    }
                }
                if changed {
                    para.touch();
                }
            }
            Block::Table(table) => {
                for cell in table.rows.iter_mut().flat_map(|row| row.cells.iter_mut()) {
                    fitted(&mut cell.blocks, measured, fit);
                }
            }
        }
    }
}

/// What `read` returns, or an error if wordcraft panics: its bug costs the document, not the
/// viewer.
fn caught<T>(read: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|_| Err("wordcraft failed on this document".into()))
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
        true => Err(format!("the document is more than {} MiB", MAX_FILE >> 20)),
        false => Ok(bytes),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::super::head::zip_head;
    use super::*;

    const COMPOUND_HEAD: [u8; 16] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0, 0, 0, 0, 0, 0, 0];

    #[test]
    fn an_rtf_is_known_by_its_first_bytes_whatever_its_name() {
        for path in ["notes.rtf", "report.doc", "untitled"] {
            assert_eq!(Kind::of(path, b"{\\rtf1\\ansi Hello}"), Some(Kind::Rtf), "{path}");
        }
        assert_eq!(Kind::of("notes.rtf", b"plain text"), None, "named so, it isn't one");
    }

    #[test]
    fn a_word_package_is_a_zip_with_word_files_first_or_named_so() {
        let package =
            zip_head(&[("[Content_Types].xml", b"<Types/>"), ("_rels/.rels", b"<Relationships/>"), ("word/document.xml", b"<w:document/>")]);
        for path in ["a.docx", "a.zip", "a"] {
            assert_eq!(Kind::of(path, &package), Some(Kind::Package), "{path}");
        }
        let other = zip_head(&[("notes.txt", b"text")]);
        for path in ["a.docx", "a.DOCM", "a.dotx", "a.dotm", "a.doc", "a.dot"] {
            assert_eq!(Kind::of(path, &other), Some(Kind::Package), "{path}: named as one");
        }
        for path in ["a.zip", "a.xlsx", "a.epub", "docx"] {
            assert_eq!(Kind::of(path, &other), None, "{path}");
        }
    }

    #[test]
    fn an_odt_is_a_zip_whose_first_file_says_so() {
        let odt = zip_head(&[("mimetype", ODT_TYPE), ("content.xml", b"<office:document-content/>")]);
        assert_eq!((Kind::of("a.odt", &odt), Kind::of("a.zip", &odt)), (Some(Kind::Odt), Some(Kind::Odt)));
        let template = zip_head(&[("mimetype", b"application/vnd.oasis.opendocument.text-template")]);
        assert_eq!(Kind::of("a.ott", &template), Some(Kind::Odt));
        let sheet = zip_head(&[("mimetype", b"application/vnd.oasis.opendocument.spreadsheet")]);
        assert_eq!(Kind::of("a.ods", &sheet), None);
        let epub = zip_head(&[("mimetype", b"application/epub+zip")]);
        assert_eq!(Kind::of("a.epub", &epub), None);
    }

    #[test]
    fn a_compound_file_is_a_word_document_only_when_named_so() {
        for path in ["a.doc", "a.DOT", "a.docx"] {
            assert_eq!(Kind::of(path, &COMPOUND_HEAD), Some(Kind::Compound), "{path}");
        }
        for path in ["a.xls", "a.ppt", "a.msg", "a"] {
            assert_eq!(Kind::of(path, &COMPOUND_HEAD), None, "{path}");
        }
    }

    #[test]
    fn markdown_is_text_named_so() {
        for path in ["a.md", "a.MD", "a.markdown", "a.mdown", "a.mkd"] {
            assert_eq!(Kind::of(path, b"# Title\n"), Some(Kind::Markdown), "{path}");
        }
        assert_eq!(Kind::of("empty.md", b""), Some(Kind::Markdown));
        assert_eq!(Kind::of("a.md", &[0xFF, 0xFE, b'#', 0, b' ', 0]), Some(Kind::Markdown), "UTF-16, by its byte order mark");
        for path in ["a.txt", "md", "a.md.txt"] {
            assert_eq!(Kind::of(path, b"# Title\n"), None, "{path}");
        }
    }

    /// A PDF, a picture or a zip given a text name is what it is, for the backend that shows it.
    #[test]
    fn what_is_not_text_is_not_markdown_or_html_by_its_name() {
        for path in ["a.md", "a.html"] {
            for head in [&b"%PDF-1.7\n"[..], b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR", b"PK\x03\x04", &COMPOUND_HEAD] {
                assert_eq!(Kind::of(path, head), None, "{path}: {head:?}");
            }
        }
    }

    #[test]
    fn html_is_known_by_how_it_starts_whatever_its_name() {
        for head in [
            &b"<!DOCTYPE html>"[..],
            b"<!doctype HTML PUBLIC \"-//W3C//DTD HTML 4.01//EN\">",
            b"<html lang=en>",
            b"<HTML>",
            b"<html",
            b"\xEF\xBB\xBF\n  <html>",
            b"<?xml version=\"1.0\"?>\n<!-- made by hand -->\n<!DOCTYPE html>",
        ] {
            for path in ["a.html", "a.txt", "untitled"] {
                assert_eq!(Kind::of(path, head), Some(Kind::Html), "{path}: {head:?}");
            }
        }
        for head in [&b"<htmlish>"[..], b"<!doctype xml>", b"<!-- never ends <html>", b"<p>a fragment</p>", b"<svg/>", b"text <html>"] {
            assert_eq!(Kind::of("untitled", head), None, "{head:?}");
        }
    }

    #[test]
    fn html_that_doesnt_say_so_is_text_named_so() {
        for path in ["a.html", "a.HTM", "a.xhtml"] {
            assert_eq!(Kind::of(path, b"<p>a fragment</p>"), Some(Kind::Html), "{path}");
        }
    }

    /// What a limit is for: the marks and the text of what is read, not the bytes of the file.
    #[test]
    fn markdown_and_html_are_refused_past_their_limits() {
        let nowhere = Pictures::beside(None);
        let markdown = |bytes: &[u8]| markdown(bytes, &nowhere, page_in(Width::Window, &ROOMLESS, 100, MARGIN_DEFAULT));
        let html = |bytes: &[u8]| html(bytes, &nowhere, page_in(Width::Window, &ROOMLESS, 100, MARGIN_DEFAULT));
        let refused = |what: &str, read: &dyn Fn(&[u8]) -> Result<wordcraft_doc::Document, String>, bytes: &[u8]| {
            assert!(read(bytes).is_err_and(|e| e.contains(what)), "{what}");
        };
        refused("lines and marks", &markdown, "*a".repeat(MAX_MARKS + 1).as_bytes());
        refused("lines and marks", &markdown, "\r".repeat(MAX_MARKS + 1).as_bytes());
        refused("lines and marks", &html, "<b>".repeat(MAX_MARKS + 1).as_bytes());
        refused("lines and marks", &html, format!("<pre>{}", "\n".repeat(MAX_MARKS + 1)).as_bytes());

        let text = "a".repeat(MAX_TEXT + 1);
        refused("of text", &markdown, text.as_bytes());
        refused("of text", &html, format!("<p>{text}</p>").as_bytes());
        assert!(markdown("a".repeat(MAX_TEXT).as_bytes()).is_ok(), "as much text as it may have");

        let hidden = format!("<script>{text}</script><style>{text}</style><img src=\"data:image/png;base64,{text}\"><p>Shown</p>");
        assert!(html(hidden.as_bytes()).is_ok(), "a page's text is what counts, not its scripts, styles and pictures");
        refused(" MiB", &html, format!("<script>{}</script>", "a".repeat(MAX_MARKED_UP)).as_bytes());
    }

    #[test]
    fn a_dark_page_is_exactly_the_paper_asked_for_and_keeps_its_white() {
        assert_eq!(grey_of([0x1E; 3]), 0x1E);
        assert_eq!(grey_of([255; 3]), 255);
        for paper in [[0x1E, 0x2E, 0x3E], [0, 0, 0], [0x28, 0x2C, 0x34], [0x00, 0x2B, 0x36], [0x1E, 0x1E, 0x1E]] {
            let grey = grey_of(paper);
            let mut pixels = [grey, grey, grey, 255, 255, 255, 255, 255, 40, 200, 90, 77];
            on_paper(&mut pixels, paper);
            assert_eq!(pixels[..4], [paper[0], paper[1], paper[2], 255], "the page: {paper:?}");
            assert_eq!(pixels[4..8], [255; 4], "white: {paper:?}");
            assert_eq!(pixels[11], 77, "what is seen through stays: {paper:?}");
        }
        let mut grey = [0x1E, 0x1E, 0x1E, 255, 80, 90, 100, 255];
        on_paper(&mut grey, [0x1E; 3]);
        assert_eq!(grey, [0x1E, 0x1E, 0x1E, 255, 80, 90, 100, 255], "a grey terminal: nothing to do");
    }

    /// What a page is asked on, in the tests: the paper setting's value, and dark paper's colour.
    #[derive(Clone, Copy, Debug)]
    enum Paper {
        Printed,
        Dark([u8; 3]),
    }

    impl Paper {
        /// Page `page` at `scale`, on this paper.
        fn asked(self, page: usize, scale: f32) -> Request {
            let mut values = crate::source::Values::default();
            let (value, dark) = match self {
                Self::Printed => (LIGHT, [0x1E; 3]),
                Self::Dark(dark) => (DARK, dark),
            };
            assert!(values.set(PAPER.name, Value::Named(value)));
            Request { page, scale, tag: 0, values, dark, tile: false }
        }
    }

    /// The page's corner, drawn through `backend` on `paper`, from a file of `contents` named `name`.
    fn corner(name: &str, contents: &[u8], paper: Paper) -> (bool, [u8; 3]) {
        let dir = std::env::temp_dir().join(format!("clview-word-corner-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        let input = crate::source::Input::open(path.to_str().unwrap()).unwrap();
        let Ok(Opened::Pages(source)) = Wordcraft.open(input, &Parsed::default()) else { panic!("not pages: {name}") };
        let frame = source.renderer().unwrap().render(paper.asked(0, 1.0));
        assert_eq!(frame.error, None);
        (source.settings().contains(&PAPER.name), [frame.rgba[0], frame.rgba[1], frame.rgba[2]])
    }

    /// Markdown and HTML go on the paper asked for; what is laid out to be printed stays as printed.
    #[test]
    fn only_markdown_and_html_have_a_dark_form() {
        const PAPER: [u8; 3] = [0x1E, 0x2E, 0x3E];
        for (name, contents) in [("a.md", &b"# One"[..]), ("a.html", b"<p>One</p>")] {
            assert_eq!(corner(name, contents, Paper::Dark(PAPER)), (true, PAPER), "{name}");
            assert_eq!(corner(name, contents, Paper::Printed), (true, [255; 3]), "{name}");
        }
        assert_eq!(corner("a.rtf", b"{\\rtf1\\ansi One}", Paper::Dark(PAPER)), (false, [255; 3]));
    }

    /// How many pixels to a point the styling's tests look at a page.
    const SCALE: f32 = 2.0;
    /// The page's left margin (three columns, 16.92 pt), and its width ([`ROOMLESS`]'s 100 columns,
    /// 564 pt), in those pixels.
    const SIDE_PX: u32 = 34;
    const PAGE_WIDTH: u32 = 1128;

    /// The first page of this Markdown on `paper`, as wide as the room it is opened with none.
    fn looked_at(markdown: &str, paper: Paper) -> Frame {
        let frame = looked_at_on(markdown, paper, None);
        assert_eq!(frame.width, PAGE_WIDTH);
        frame
    }

    /// The first page of this Markdown on `paper`, laid out at `width` if given (`--width`'s value).
    fn looked_at_on(markdown: &str, paper: Paper, width: Option<&str>) -> Frame {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let at = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("clview-word-looks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{at}.md"));
        std::fs::write(&path, markdown).unwrap();
        let input = crate::source::Input::open(path.to_str().unwrap()).unwrap();
        let Ok(Opened::Pages(source)) = Wordcraft.open(input, &given(width)) else { panic!("not pages") };
        let frame = source.renderer().unwrap().render(paper.asked(0, SCALE));
        assert_eq!(frame.error, None);
        std::fs::remove_file(&path).unwrap();
        frame
    }

    /// The options, with `--width` at `width` if given.
    fn given(width: Option<&str>) -> Parsed<'static> {
        let args: &'static [String] = width.map_or(&[], |width| Box::leak(vec!["--width".to_owned(), width.to_owned()].into_boxed_slice()));
        Parsed::parse(args, &[WIDTH.option]).unwrap()
    }

    /// A room of `cols` × `rows` cells of `cell_w` × `cell_h` pixels, two cells between pages.
    const fn room(cols: u16, rows: u16, cell_w: f32, cell_h: f32) -> Room {
        Room { cols, rows, cell_w, cell_h, gap: 2, per_row: None }
    }

    /// Within a hundredth of a point.
    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    /// The page is the room, in points such that a character of the code face is a cell when the page
    /// is as tall as the room: at `window` as wide as it, with margins of three columns and a row; at a
    /// number, that many characters between the same margins, never wider than the room; its height
    /// always the room's. Where the room holds two pages of 60 characters between their margins,
    /// `window` is a book's page, half the room less the gap.
    #[test]
    fn the_page_is_the_room_and_its_width_so_many_characters_of_the_code_face() {
        // 80 × 23 cells of 10 × 20 px: 451.2 × 259.44 pt, a column 5.64 pt and a row 11.28.
        let small = room(80, 23, 10.0, 20.0);
        let page = page_in(Width::Window, &small, 100, MARGIN_DEFAULT);
        assert!(near(page.page_w, 80.0 * CODE_WIDTH) && near(page.page_h, 23.0 * 2.0 * CODE_WIDTH), "{} × {}", page.page_w, page.page_h);
        assert!(near(page.margin_left, 3.0 * CODE_WIDTH) && near(page.margin_right, 3.0 * CODE_WIDTH), "{}", page.margin_left);
        assert!(near(page.margin_top, 2.0 * CODE_WIDTH) && near(page.margin_bottom, 2.0 * CODE_WIDTH), "{}", page.margin_top);
        assert!(near(page.text_width(), 74.0 * CODE_WIDTH) && columns_of(&page) == 74);
        // The cell's shape is the page's: 10 × 16 cells make a shorter page, and a text cell's sampling
        // cell (1.25 × 2) the same one.
        let squat = page_in(Width::Window, &room(80, 23, 10.0, 16.0), 100, MARGIN_DEFAULT);
        assert!(near(squat.page_w, page.page_w) && near(squat.page_h, 23.0 * 1.6 * CODE_WIDTH), "{}", squat.page_h);
        let sampled = page_in(Width::Window, &room(80, 23, 1.25, 2.0), 100, MARGIN_DEFAULT);
        assert!(near(sampled.page_w, squat.page_w) && near(sampled.page_h, squat.page_h));
        // A number: that many characters between the margins, as tall; wider than the room, the room's.
        let narrow = page_in(Width::Columns(40), &small, 100, MARGIN_DEFAULT);
        assert!(near(narrow.text_width(), 40.0 * CODE_WIDTH) && near(narrow.page_w, 46.0 * CODE_WIDTH) && near(narrow.page_h, page.page_h));
        assert!(near(page_in(Width::Columns(120), &small, 100, MARGIN_DEFAULT).page_w, page.page_w), "never wider than the room");
        assert_eq!(columns_of(&page_in(Width::Columns(120), &small, 100, MARGIN_DEFAULT)), 74);
        // A book: 134 columns hold two pages of (134 − 2) / 2 − 6 = 60 characters; 132 don't.
        let wide = room(134, 40, 10.0, 20.0);
        let book = page_in(Width::Window, &wide, 100, MARGIN_DEFAULT);
        assert!(near(book.page_w, 66.0 * CODE_WIDTH) && near(book.page_h, 40.0 * 2.0 * CODE_WIDTH), "{}", book.page_w);
        assert_eq!(columns_of(&book), 60);
        assert_eq!(columns_of(&page_in(Width::Window, &room(132, 40, 10.0, 20.0), 100, MARGIN_DEFAULT)), 126, "one page, as wide as the room");
        // A tiny room holds no page at any size, so it is laid out at the least, 50%, where 23 columns are 46
        // characters, 40 between the margins. At 50% 10 columns are 20 characters with no margin, so the
        // margin is none; 9 aren't 20 even then, so the margin is a column and the page grows to 20
        // characters. Either way, 10 lines of text.
        let tiny = page_in(Width::Window, &room(23, 5, 10.0, 20.0), 100, MARGIN_DEFAULT);
        assert!(near(tiny.margin_left, 3.0 * CODE_WIDTH) && columns_of(&tiny) == 40 && near(tiny.text_height(), 10.0 * LINE), "{tiny:?}");
        let tinier = page_in(Width::Window, &room(10, 5, 10.0, 20.0), 100, MARGIN_DEFAULT);
        assert!(near(tinier.margin_left, 0.0) && near(tinier.text_width(), 20.0 * CODE_WIDTH), "{tinier:?}");
        let tiniest = page_in(Width::Window, &room(9, 5, 10.0, 20.0), 100, MARGIN_DEFAULT);
        assert!(
            near(tiniest.margin_left, CODE_WIDTH) && near(tiniest.text_width(), 20.0 * CODE_WIDTH) && near(tiniest.text_height(), 10.0 * LINE),
            "{tiniest:?}"
        );
        for odd in [room(0, 0, 10.0, 20.0), room(1, 1, 0.0, 0.0), room(5, 5, f32::NAN, f32::INFINITY), room(u16::MAX, u16::MAX, 1.0, 1.0e30)] {
            let page = page_in(Width::Window, &odd, 100, MARGIN_DEFAULT);
            assert!(
                page.page_w.is_finite() && page.page_h.is_finite() && page.page_w <= MAX_PAGE + 100.0 && page.page_h <= MAX_PAGE + 100.0,
                "{odd:?}"
            );
            assert!(page.text_width() >= 20.0 * CODE_WIDTH - 0.01 && page.text_height() >= 10.0 * LINE - 0.01, "{odd:?}");
        }
        assert!(near(page_in(Width::Window, &room(u16::MAX, 40, 10.0, 20.0), 100, MARGIN_DEFAULT).page_w, MAX_PAGE), "capped");
        // From the option, and from the values: within the range either way.
        assert_eq!((Width::given(None), Width::given(Some("window")), Width::given(Some("40"))), (Width::Window, Width::Window, Width::Columns(40)));
        assert_eq!((Width::given(Some("300")), Width::given(Some("x"))), (Width::Columns(200), Width::Window));
        let values = |value: Option<Value>| {
            let mut values = Values::default();
            if let Some(value) = value {
                assert!(values.set(WIDTH.name, value));
            }
            Width::wanted(&values)
        };
        assert_eq!(
            (values(None), values(Some(Value::Named(WINDOW))), values(Some(Value::Number(72)))),
            (Width::Window, Width::Window, Width::Columns(72))
        );
        assert_eq!(values(Some(Value::Number(7))), Width::Columns(20));
        // A code block, indented 8 pt on each side, holds about 3 characters fewer than the width.
        let lines =
            |columns: usize| lines_of(&looked_at_on(&format!("```\n{}\n```", "0".repeat(columns)), Paper::Printed, Some("40")), dark_ink).len();
        assert_eq!((lines(37), lines(38)), (1, 2));
        let frame = looked_at_on("One", Paper::Printed, Some("40"));
        assert_eq!(frame.width, (46.0 * CODE_WIDTH * SCALE).ceil() as u32);
    }

    /// The side margins are characters of the code face at the font size, at each side: at `window` they
    /// come out of the line, and count against a book's 60 characters a half; at a width the line keeps
    /// its length and the page grows by them, never past the room. The bounds go in order: the font's
    /// most whatever the margin, then the margin's most at the font laid out at, so the margin yields to
    /// the font; a margin past its most is laid out at it, and where even none leaves 20 characters, at
    /// one at most.
    #[test]
    fn margins_shorten_the_line_at_window_and_widen_the_page_at_a_width() {
        let small = room(80, 23, 10.0, 20.0);
        let window = |font, margin| page_in(Width::Window, &small, font, margin);
        let line = |font, margin| columns_of(&window(font, margin));
        // At 100%: 80 characters, 74, 40.
        assert_eq!((line(100, 0), line(100, 3), line(100, 20)), (80, 74, 40));
        assert!(near(window(100, 0).margin_left, 0.0) && near(window(100, 20).margin_left, 20.0 * CODE_WIDTH));
        assert!(near(window(100, 20).page_w, 80.0 * CODE_WIDTH), "the window's page, whatever the margins");
        // At 50%: 160 characters; halves of 78, so a book at 0 and at 3 (72), none at 20 (120).
        assert_eq!((line(50, 0), line(50, 3), line(50, 20)), (78, 72, 120));
        // At 200%, past the 170% this room holds: 47 characters, 41; at 20, still 170%, the margin laid out
        // at 13, 21 characters.
        assert_eq!((line(200, 0), line(200, 3), line(200, 20)), (47, 41, 21));
        assert!(near(window(200, 20).margin_left, 13.0 * CODE_WIDTH), "{:?}", window(200, 20));
        // A width keeps its line, and the page grows by the margins, up to the room.
        let width = |columns, margin| page_in(Width::Columns(columns), &small, 100, margin);
        assert!(
            near(width(40, 0).page_w, 40.0 * CODE_WIDTH)
                && near(width(40, 3).page_w, 46.0 * CODE_WIDTH)
                && near(width(40, 20).page_w, 80.0 * CODE_WIDTH)
        );
        assert_eq!((columns_of(&width(40, 20)), columns_of(&width(60, 20))), (40, 40), "never past the room");
        // The margin yields to the font: in 30 columns at 100%, a line keeps 20 characters within 5 at
        // most, so 12 is laid out at 5, the text at the size asked; the font's most is the same at any margin.
        let thirty = room(30, 23, 10.0, 20.0);
        let past = page_in(Width::Window, &thirty, 100, 12);
        assert!(near(past.margin_left, 5.0 * CODE_WIDTH) && columns_of(&past) == 20, "{past:?}");
        assert!(near(past.page_w, page_in(Width::Window, &thirty, 100, 0).page_w), "the page at 100%: {past:?}");
        // Where even none leaves 20 characters (9 columns, at the least size 18), one at most.
        let nine = room(9, 23, 10.0, 20.0);
        assert_eq!(margin_most(&nine, font_most(&nine)), None);
        assert!(near(page_in(Width::Window, &nine, 100, 3).margin_left, CODE_WIDTH));
        assert!(near(page_in(Width::Window, &nine, 100, 0).margin_left, 0.0), "never more than asked");
        // From the option and the values: within the range, 3 where they say nothing.
        assert_eq!((margin_given(None), margin_given(Some("7")), margin_given(Some("30")), margin_given(Some("x"))), (3, 7, 20, 3));
        let mut values = Values::default();
        assert!(values.set(MARGIN.name, Value::Number(12)));
        assert_eq!((margin_wanted(&Values::default()), margin_wanted(&values)), (3, 12));
    }

    /// With a count to a row ([`Room::per_row`]), `window` is a column: the room's columns less the gaps
    /// between pages, shared that many ways, as tall as the room, the book rule off. The width's most is a
    /// column's line, and the font's the largest at which a column holds 20 characters; where even 50%
    /// leaves a column under 20, it is laid out at 20 and grows, as a small room is.
    #[test]
    fn pages_to_a_row_are_columns_of_the_room() {
        let wide = room(134, 40, 10.0, 20.0);
        let at = |per_row| Room { per_row, ..wide };
        // Three to a row: (134 − 2 × 2) / 3 = 43 columns each, 37 characters between the margins.
        let three = page_in(Width::Window, &at(Some(3)), 100, MARGIN_DEFAULT);
        let alone = page_in(Width::Window, &wide, 100, MARGIN_DEFAULT);
        assert!(near(three.page_w, 43.0 * CODE_WIDTH) && near(three.page_h, alone.page_h), "{} × {}", three.page_w, three.page_h);
        assert_eq!(columns_of(&three), 37);
        // One to a row is the whole room, no book of two, which `auto` makes here.
        assert_eq!((columns_of(&page_in(Width::Window, &at(Some(1)), 100, MARGIN_DEFAULT)), columns_of(&alone)), (128, 60));
        assert_eq!(columns_of(&page_in(Width::Window, &at(Some(2)), 100, MARGIN_DEFAULT)), 60, "two: each half less the gap");
        // A width within a column: past it, the column's.
        assert_eq!(columns_of(&page_in(Width::Columns(72), &at(Some(3)), 100, MARGIN_DEFAULT)), 37);
        assert_eq!(columns_of(&page_in(Width::Columns(30), &at(Some(3)), 100, MARGIN_DEFAULT)), 30);
        // The font's most is a column's: 190% holds 20 characters in 43 columns, within margins of a column,
        // 200% doesn't; the margin then yields, to one.
        let three_most = (fits(&at(Some(3)), 190), fits(&at(Some(3)), 200), font_most(&at(Some(3))));
        assert_eq!((three_most, font_most(&wide)), ((true, false, 190), 300));
        let past = page_in(Width::Window, &at(Some(3)), 300, MARGIN_DEFAULT);
        let most = page_in(Width::Window, &at(Some(3)), 190, MARGIN_DEFAULT);
        assert!(near(past.margin_left, CODE_WIDTH), "{past:?}");
        assert!(near(past.page_w, most.page_w) && columns_of(&past) == 20, "laid out at the most: {past:?}");
        // Eight to a row of 40 columns: 3 each, under 20 characters even at 50% and no margin; laid out at
        // 20 with a margin of a column, grown.
        let narrow = Room { per_row: Some(8), ..room(40, 40, 10.0, 20.0) };
        assert_eq!(font_most(&narrow), 50);
        let grown = page_in(Width::Window, &narrow, 100, MARGIN_DEFAULT);
        assert!(near(grown.text_width(), 20.0 * CODE_WIDTH) && near(grown.margin_left, CODE_WIDTH), "{grown:?}");
        // However odd the count: a cell at least, never a panic.
        for per_row in [Some(0), Some(u16::MAX), Some(200)] {
            let page = page_in(Width::Window, &at(per_row), 100, MARGIN_DEFAULT);
            assert!(page.page_w.is_finite() && page.text_width() >= 20.0 * CODE_WIDTH - 0.01, "{per_row:?}: {page:?}");
            assert!((1..=wide.cols).contains(&page_cols(&at(per_row))), "{per_row:?}");
        }
    }

    /// At a font size, a cell of the room is that share of a character of the code face: at 200%, a
    /// page half as wide in points, which holds half the characters, and half as tall, until it is too
    /// short for ten lines, when it grows and is shown smaller. The margins stay at the text's size.
    #[test]
    fn a_font_size_shrinks_the_page_in_points_so_its_text_is_larger_on_screen() {
        // 80 × 30 cells hold 200%: half the page in points, half the characters, twice the text on screen.
        let tall = room(80, 30, 10.0, 20.0);
        let (at_100, at_200) = (page_in(Width::Window, &tall, 100, MARGIN_DEFAULT), page_in(Width::Window, &tall, 200, MARGIN_DEFAULT));
        assert!(near(at_200.page_w, at_100.page_w / 2.0) && near(at_200.margin_left, at_100.margin_left), "{} {}", at_200.page_w, at_100.page_w);
        assert!(near(at_200.page_h, at_100.page_h / 2.0), "{} {}", at_200.page_h, at_100.page_h);
        assert_eq!((columns_of(&at_100), columns_of(&at_200)), (74, 34));
        // A size past the most the room holds (170% at 80 × 23) is laid out at the most, as a width past
        // the room's is: the page never grows for a size the room can't hold.
        let small = room(80, 23, 10.0, 20.0);
        let (at_170, at_300) = (page_in(Width::Window, &small, 170, MARGIN_DEFAULT), page_in(Width::Window, &small, 300, MARGIN_DEFAULT));
        assert!(near(at_300.page_w, at_170.page_w) && near(at_300.page_h, at_170.page_h) && columns_of(&at_300) == 41, "{}", at_300.page_w);
        assert!(near(at_170.page_h, 23.0 * 2.0 * CODE_WIDTH * 100.0 / 170.0), "the room's height still: {}", at_170.page_h);
        // Where even the least size doesn't hold a page, the page grows to 20 × 10 of text and is shown
        // smaller.
        let grown = page_in(Width::Window, &room(12, 5, 10.0, 20.0), 50, MARGIN_DEFAULT);
        assert!(near(grown.text_height(), 10.0 * LINE) && near(grown.page_h, 10.0 * LINE + 2.0 * grown.margin_top), "grown: {}", grown.page_h);
        // At 50% the same window holds twice the characters: 154 a line, so two pages of 72 as a book.
        let (small_100, at_50) = (page_in(Width::Window, &small, 100, MARGIN_DEFAULT), page_in(Width::Window, &small, 50, MARGIN_DEFAULT));
        assert!(
            near(at_50.page_w, 39.0 * 2.0 * CODE_WIDTH) && near(at_50.page_h, small_100.page_h * 2.0) && columns_of(&at_50) == 72,
            "{}",
            at_50.page_w
        );
        assert_eq!(columns_of(&page_in(Width::Columns(200), &small, 50, MARGIN_DEFAULT)), 154);
        // A number is that many characters whatever the size, within the room.
        assert_eq!(columns_of(&page_in(Width::Columns(40), &tall, 200, MARGIN_DEFAULT)), 34, "the room's 34 at 200%");
        assert_eq!(columns_of(&page_in(Width::Columns(40), &small, 50, MARGIN_DEFAULT)), 40);
        // The most: the largest size at which the room holds 20 × 10 of text; a tiny room holds none, and
        // gets the least.
        assert_eq!((fits(&small, 170), fits(&small, 180)), (true, false));
        assert_eq!((font_most(&small), font_most(&room(200, 50, 10.0, 20.0)), font_most(&room(12, 5, 10.0, 20.0))), (170, 370, 50));
        assert_eq!(font_most(&room(0, 0, 0.0, 0.0)), 50);
        // From the option and the values: within the range.
        assert_eq!((font_given(None), font_given(Some("150")), font_given(Some("900")), font_given(Some("x"))), (100, 150, 400, 100));
        let mut values = Values::default();
        assert!(values.set(FONT.name, Value::Number(20)));
        assert_eq!((font_wanted(&Values::default()), font_wanted(&values)), (100, 50));
    }

    /// Where `window` sits among the widths, and the most a width is laid out at, for a room: the
    /// characters its line holds there, both the same for one page as wide as the room, the book's page
    /// under the room's where it holds two; at the font size the values want. The font's most in the
    /// room, too. Only for those, and only on Markdown and HTML.
    #[test]
    fn the_bounds_of_the_width_are_the_rooms_lines() {
        let dir = std::env::temp_dir().join(format!("clview-word-bounds-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let open = |name: &str, contents: &str| {
            let path = dir.join(name);
            std::fs::write(&path, contents).unwrap();
            let input = crate::source::Input::open(path.to_str().unwrap()).unwrap();
            let Ok(Opened::Pages(source)) = Wordcraft.open(input, &Parsed::default()) else { panic!("not pages: {name}") };
            source
        };
        let md = open("a.md", "One");
        let bounds = |cols: u16| md.bounds(WIDTH.name, room(cols, 23, 10.0, 20.0), Values::default()).unwrap();
        assert_eq!(bounds(80), Bounds { named_at: Some(74), most: Some(74) });
        assert_eq!(bounds(134), Bounds { named_at: Some(60), most: Some(128) });
        assert_eq!(bounds(12), Bounds { named_at: Some(20), most: Some(20) }, "a tiny room: the least a page holds");
        let mut at_200 = Values::default();
        assert!(at_200.set(FONT.name, Value::Number(200)));
        assert_eq!(
            md.bounds(WIDTH.name, room(80, 23, 10.0, 20.0), at_200),
            Some(Bounds { named_at: Some(41), most: Some(41) }),
            "laid out at 170%, the most"
        );
        // The margin's most: the largest at which a line holds 20 characters at the font laid out at; 20,
        // the setting's own, in 80 columns at 100%; 13 at 300%, laid out at 170%; in 12 columns, laid out at
        // 50%, 2; in 9, where even none leaves 20 characters, a column.
        let margin = |cols: u16, values| md.bounds(MARGIN.name, room(cols, 23, 10.0, 20.0), values).and_then(|bounds: Bounds| bounds.most);
        let mut at_300 = Values::default();
        assert!(at_300.set(FONT.name, Value::Number(300)));
        assert_eq!(
            (margin(80, Values::default()), margin(80, at_300), margin(12, Values::default()), margin(9, Values::default())),
            (Some(20), Some(13), Some(2), Some(1))
        );
        // And the font's most is the same whatever the margin asked: the margin yields to it.
        let mut wide_margin = Values::default();
        assert!(wide_margin.set(MARGIN.name, Value::Number(20)));
        assert_eq!(md.bounds(FONT.name, room(80, 23, 10.0, 20.0), wide_margin).and_then(|bounds| bounds.most), Some(170));
        assert_eq!(md.bounds(FONT.name, room(80, 23, 10.0, 20.0), Values::default()), Some(Bounds { named_at: None, most: Some(170) }));
        assert_eq!(md.bounds(PAPER.name, room(80, 23, 10.0, 20.0), Values::default()), None);
        let rtf = open("a.rtf", "{\\rtf1\\ansi One}");
        assert_eq!(
            (rtf.bounds(WIDTH.name, room(80, 23, 10.0, 20.0), Values::default()), rtf.bounds(FONT.name, room(80, 23, 10.0, 20.0), Values::default())),
            (None, None)
        );
    }

    /// A table is as wide as the text at any width, since the page goes into the import: at `window`, the
    /// room's text width, not the default page's, which wordcraft's own import would size it to.
    #[test]
    fn a_table_is_as_wide_as_the_text_at_any_width() {
        let nowhere = Pictures::beside(None);
        let grid = |document: &wordcraft_doc::Document| -> f32 {
            let table = document.body.iter().find_map(|block| match &**block {
                Block::Table(table) => Some(table),
                _ => None,
            });
            table.expect("a table").grid.iter().sum()
        };
        let table = "| One | Two |\n|---|---|\n| Three | Four |\n";
        let own = wordcraft_formats::html::import_with(&as_html(table), &|_| None);
        assert_eq!(grid(&own), 468.0, "the default page's");
        let window = || page_in(Width::Window, &ROOMLESS, 100, MARGIN_DEFAULT);
        assert!(near(grid(&markdown(table.as_bytes(), &nowhere, window()).unwrap()), 94.0 * CODE_WIDTH), "the room's");
        let cells = "<table><tr><td>One</td><td>Two</td></tr></table>";
        assert!(near(grid(&html(cells.as_bytes(), &nowhere, window()).unwrap()), 94.0 * CODE_WIDTH));
        let narrow = || page_in(Width::Columns(40), &ROOMLESS, 100, MARGIN_DEFAULT);
        for narrow in [markdown(table.as_bytes(), &nowhere, narrow()), html(cells.as_bytes(), &nowhere, narrow())] {
            assert!(near(grid(&narrow.unwrap()), 40.0 * CODE_WIDTH));
        }
    }

    /// A picture is shown at its own size, or smaller to fit the column, exactly and whatever the
    /// column: past wordcraft's 468 points, and in whole points or not; and one the document gives a
    /// size to is fitted to the column too.
    #[cfg(feature = "image")]
    #[test]
    fn a_picture_fits_the_column_however_wide() {
        let dir = std::env::temp_dir().join(format!("clview-word-column-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png =
            |w: u32, h: u32| crate::encode::encode_png(w, h, &[200, 0, 0, 255].repeat((w * h) as usize), crate::encode::PngEffort::Fast).unwrap();
        std::fs::write(dir.join("wide.png"), png(3000, 300)).unwrap();
        std::fs::write(dir.join("small.png"), png(40, 20)).unwrap();
        let pictures = Pictures::beside(Some(&dir));
        let sizes = |document: wordcraft_doc::Document| -> Vec<(f32, f32)> {
            let objects = document.body.iter().filter_map(|block| match &**block {
                Block::Para(para) => Some(para.objects.clone()),
                Block::Table(_) => None,
            });
            objects.flatten().filter_map(|object| if let InlineObject::Image { w, h, .. } = object { Some((w, h)) } else { None }).collect()
        };
        let page = page_in(Width::Window, &room(120, 40, 10.0, 20.0), 100, MARGIN_DEFAULT);
        let column = page.text_width();
        assert!(column > 468.0, "{column}");
        let shown = sizes(markdown(b"![two](wide.png)", &pictures, page).unwrap());
        assert!(matches!(shown[..], [(w, h)] if near(w, column) && near(h, column / 10.0)), "to the column: {shown:?}, not {column}");

        let page = page_in(Width::Columns(40), &room(120, 40, 10.0, 20.0), 100, MARGIN_DEFAULT);
        let column = page.text_width();
        let shown = sizes(html(b"<p><img src=\"small.png\" width=\"400\"></p>", &pictures, page).unwrap());
        assert!(matches!(shown[..], [(w, h)] if near(w, column) && near(h, column / 2.0)), "given a size: {shown:?}, not {column}");
    }

    /// A page's paragraphs are kept, by their order in the document, so that the one at a place on a page
    /// is found again in another layout: on the page it starts on, split or not; past the page's last
    /// paragraph, that one; a page without any, or a document that doesn't lay out again, gives none.
    #[test]
    fn a_paragraph_at_a_place_is_found_again_in_another_layout() {
        let dir = std::env::temp_dir().join(format!("clview-word-anchor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let open = |name: &str, contents: &str, width: Option<&str>| {
            let path = dir.join(name);
            std::fs::write(&path, contents).unwrap();
            let input = crate::source::Input::open(path.to_str().unwrap()).unwrap();
            let Ok(Opened::Pages(source)) = Wordcraft.open(input, &given(width)) else { panic!("not pages: {name}") };
            source
        };
        // Paragraphs of a line or two on the room's page, several at 40, then one long enough to split
        // across pages, then more of the short ones.
        let long = "A long paragraph, many words of it, so long that it runs from one page on to the next. ".repeat(60);
        let short =
            |n: u32| format!("Paragraph {n}, with words enough in it to take a line or two of a page as printed, and more of a narrow one.\n\n");
        let text: String = (1..=80).map(short).chain([format!("{long}\n\n")]).chain((81..=160).map(short)).collect();
        let letter = open("a.md", &text, None);
        let narrow = open("b.md", &text, Some("40"));
        assert!(letter.pages() >= 3 && narrow.pages() > letter.pages(), "{} and {} pages", letter.pages(), narrow.pages());
        // The top of page 2 of the wide layout, found further on in the narrow one, whose page 2 starts
        // earlier.
        let top = letter.anchor(1, 0.0).expect("a paragraph");
        let (page, y) = narrow.find(&top).expect("found");
        assert!(page > 1 || (page == 1 && y > 100.0), "page {page}, {y} pt down");
        assert!(narrow.anchor(1, 0.0).unwrap().0 < top.0);
        assert_eq!(letter.find(&top), Some((1, letter.find(&top).unwrap().1)), "found where it is");
        // Further down the same page, a later paragraph; past the last, the last.
        let lower = letter.anchor(1, 300.0).unwrap();
        assert!(lower.0 > top.0 && letter.find(&lower).is_some_and(|(page, y)| page == 1 && y >= 270.0));
        let bottom = letter.anchor(1, 10_000.0).unwrap();
        assert!(bottom.0 >= lower.0 && letter.find(&bottom).unwrap().0 == 1);
        // The long paragraph: on whichever page it ends, the anchor at its top is found on the page it starts.
        let split = Anchor(80);
        let (starts, _) = letter.find(&split).unwrap();
        let ends = letter.anchor(starts.saturating_add(1), 0.0).unwrap();
        assert_eq!(ends, split, "it goes on at the top of the next page");
        assert_eq!(letter.find(&ends).unwrap().0, starts);
        assert_eq!(letter.find(&Anchor(u64::MAX)), None, "one it doesn't know");
        assert_eq!(letter.anchor(99, 0.0), None, "no such page");
        let rtf = open("a.rtf", "{\\rtf1\\ansi One\\par Two}", None);
        assert_eq!((rtf.anchor(0, 0.0), rtf.find(&Anchor(0))), (None, None));
    }

    /// Markdown and HTML say the width they were laid out at and the room they were laid out for, exactly
    /// as asked, and give a job that lays them out again at another width or for another room, from their
    /// bytes, as a source of its own; at the same width in the same room, none. Their largest scale is
    /// the one at which a page is as tall as its room. What has a page of its own says nothing and lays
    /// out again at nothing.
    #[test]
    fn markdown_and_html_say_their_width_and_room_and_lay_out_again_at_another() {
        let dir = std::env::temp_dir().join(format!("clview-word-relayout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let open = |name: &str, contents: &str, width: Option<&str>, room: Room| {
            let path = dir.join(name);
            std::fs::write(&path, contents).unwrap();
            let input = crate::source::Input::open(path.to_str().unwrap()).unwrap();
            let Ok(Opened::Pages(source)) = Wordcraft.open_at(input, &given(width), room) else { panic!("not pages: {name}") };
            source
        };
        let at = |columns: u32| {
            let mut values = Values::default();
            assert!(values.set(WIDTH.name, Value::Number(columns)));
            values
        };
        // Given as the viewer gives it, with a cell a fraction off a whole pixel.
        let small = room(80, 23, 10.0, 20.03);
        let long: String = (1..=120).map(|n| format!("Paragraph {n}, with some words in it to fill a line or two.\n\n")).collect();
        let source = open("a.md", &long, Some("40"), small);
        assert_eq!(
            (source.laid_at().0.get(WIDTH.name), source.laid_at().1, source.settings().contains(&WIDTH.name)),
            (Some(Value::Number(40)), Some(small), true)
        );
        let (w, h) = source.size(0).unwrap();
        assert!(near(w, 46.0 * CODE_WIDTH) && near(h, 23.0 * 2.003 * CODE_WIDTH), "{w} × {h}");
        assert_eq!(source.laid_at().0.get(MARGIN.name), Some(Value::Number(MARGIN_DEFAULT)), "its margin, said");
        assert!(near(source.largest_scale(0).unwrap(), 23.0 * 20.03 / h) && source.largest_scale(99).is_none());
        assert!(source.relayout(at(40), small).is_none(), "at it already");
        assert!(source.relayout(at(40), room(80, 23, 10.1, 20.0)).is_none(), "the same room, within a little");
        assert!(source.relayout(at(40), room(80, 24, 10.0, 20.0)).is_some(), "another room");
        // Another margin lays out again: at a width, the line keeps its 40 characters and the page grows by
        // the margins, 5 at each side.
        let mut margins = at(40);
        assert!(margins.set(MARGIN.name, Value::Number(5)));
        let wide_margins = source.relayout(margins, small).expect("a job")().unwrap();
        assert_eq!(wide_margins.laid_at().0.get(MARGIN.name), Some(Value::Number(5)));
        assert!(near(wide_margins.size(0).unwrap().0, 50.0 * CODE_WIDTH), "{:?}", wide_margins.size(0));
        assert!(wide_margins.relayout(margins, small).is_none(), "at it already");
        let wider = source.relayout(at(76), small).expect("a job")().unwrap();
        assert_eq!(wider.laid_at().0.get(WIDTH.name), Some(Value::Number(76)));
        assert!(near(wider.size(0).unwrap().0, 80.0 * CODE_WIDTH));
        assert!(wider.pages() < source.pages(), "{} pages at 76, {} at 40", wider.pages(), source.pages());
        let large = room(200, 50, 10.0, 20.0);
        let window = wider.relayout(Values::default(), large).expect("a job")().unwrap();
        assert_eq!((window.laid_at().0.get(WIDTH.name), window.laid_at().1), (Some(Value::Named(WINDOW)), Some(large)));
        let (w, h) = window.size(0).unwrap();
        assert!(near(w, 99.0 * CODE_WIDTH) && near(h, 100.0 * CODE_WIDTH), "a book's page: {w} × {h}");
        assert_eq!(window.renderer().unwrap().render(Paper::Printed.asked(0, 1.0)).width, (99.0 * CODE_WIDTH).ceil() as u32);
        assert!(window.pages() < wider.pages());
        let html = open("a.html", "<p>One</p>", None, small);
        assert_eq!((html.laid_at().0.get(WIDTH.name), html.laid_at().0.get(FONT.name)), (Some(Value::Named(WINDOW)), Some(Value::Number(100))));
        assert!(html.relayout(at(20), small).is_some() && html.relayout(Values::default(), small).is_none());
        // The font size: another lays out again, a page half as wide in points at 200%, and half as many
        // characters a line; the scale it is shown at is as tall as the room still, so twice as large.
        let mut doubled = Values::default();
        assert!(doubled.set(FONT.name, Value::Number(200)));
        let larger = source.relayout(doubled, small).expect("a job")().unwrap();
        assert_eq!(larger.laid_at().0.get(FONT.name), Some(Value::Number(200)));
        let (w, h) = larger.size(0).unwrap();
        // Laid out at 170%, the most this room holds: the room's width at that size.
        assert!(near(w, 80.0 * CODE_WIDTH * 100.0 / 170.0) && h > source.size(0).unwrap().1 / 2.0, "{w} × {h}");
        assert!(larger.pages() > source.pages());
        assert!(larger.largest_scale(0).unwrap() > source.largest_scale(0).unwrap(), "shown larger");
        assert!(larger.relayout(doubled, small).is_none(), "at it already");
        let rtf = open("a.rtf", "{\\rtf1\\ansi One}", Some("40"), small);
        assert_eq!(
            (rtf.laid_at(), rtf.relayout(at(20), large).is_none(), rtf.settings().contains(&WIDTH.name)),
            ((Values::default(), None), true, false)
        );
        assert_eq!(rtf.largest_scale(0), None);
    }

    fn colour_at(frame: &Frame, x: u32, y: u32) -> [u8; 3] {
        let at = (y as usize * frame.width as usize + x as usize) * 4;
        [frame.rgba[at], frame.rgba[at + 1], frame.rgba[at + 2]]
    }

    fn brightness([r, g, b]: [u8; 3]) -> u8 {
        grey_of([r, g, b])
    }

    /// Dark ink on white: not a line or a wash, which are faint.
    fn dark_ink(colour: [u8; 3]) -> bool {
        brightness(colour) < 128
    }

    /// Anything on white.
    fn not_white(colour: [u8; 3]) -> bool {
        colour != [255; 3]
    }

    /// A line of text, in pixels.
    #[derive(Clone, Copy, Debug)]
    struct Inked {
        top: u32,
        bottom: u32,
        left: u32,
        right: u32,
    }

    /// What `ink` finds on a page, in lines: rows with some, apart from others by rows with none.
    fn lines_of(frame: &Frame, ink: fn([u8; 3]) -> bool) -> Vec<Inked> {
        let mut lines: Vec<Inked> = Vec::new();
        let mut in_line = false;
        for y in 0..frame.height {
            let inked: Vec<u32> = (0..frame.width).filter(|x| ink(colour_at(frame, *x, y))).collect();
            let (Some(left), Some(right)) = (inked.first().copied(), inked.last().copied()) else {
                in_line = false;
                continue;
            };
            match lines.last_mut().filter(|_| in_line) {
                Some(line) => *line = Inked { bottom: y, left: line.left.min(left), right: line.right.max(right), ..*line },
                None => lines.push(Inked { top: y, bottom: y, left, right }),
            }
            in_line = true;
        }
        lines
    }

    /// What decides that a page isn't a word processor's: Word documents stay as they say.
    #[test]
    fn only_markdown_and_html_are_given_this_look() {
        let font = |bytes: &[u8], kind| {
            let laid = laid_out(bytes, kind, None, "a", None, At { width: Width::Window, font: 100, margin: MARGIN_DEFAULT }, &ROOMLESS).unwrap();
            let document = laid.document.read().unwrap();
            document.styles.default_chr.font.clone().unwrap()
        };
        assert_eq!(font(b"One", Kind::Markdown), TEXT_FACE);
        assert_eq!(font(b"<p>One</p>", Kind::Html), TEXT_FACE);
        assert_ne!(font(b"{\\rtf1\\ansi One}", Kind::Rtf), TEXT_FACE);
    }

    /// wordcraft reads HTML two ways: as a web page, set in Times New Roman at a browser's sizes over
    /// any style of the document's, or as a fragment, which takes the document's. This look is styles.
    #[test]
    fn text_and_headings_take_this_look_not_a_web_pages() {
        let nowhere = Pictures::beside(None);
        let documents = [
            markdown(b"# Head\n\nText", &nowhere, page_in(Width::Window, &ROOMLESS, 100, MARGIN_DEFAULT)).unwrap(),
            html(b"<h1>Head</h1><p>Text</p>", &nowhere, page_in(Width::Window, &ROOMLESS, 100, MARGIN_DEFAULT)).unwrap(),
        ];
        for document in &documents {
            let paragraphs = document.para_paths(StoryRef::Body);
            assert_eq!(paragraphs.len(), 2);
            for para in paragraphs.iter().filter_map(|path| document.para(StoryRef::Body, path)) {
                assert_eq!(para.runs.len(), 1, "{:?}", para.text);
                let props = &para.runs[0].props;
                assert_eq!((props.font.as_deref(), props.size), (None, None), "{:?}", para.text);
            }
        }
    }

    #[test]
    fn headings_step_down_and_the_first_starts_the_page() {
        let frame = looked_at("# Hxg\n\n## Hxg\n\n### Hxg\n\n#### Hxg\n\n##### Hxg\n\n###### Hxg", Paper::Printed);
        let lines = lines_of(&frame, dark_ink);
        assert_eq!(lines.len(), 6, "{lines:?}");
        let heights: Vec<u32> = lines.iter().map(|line| line.bottom - line.top).collect();
        assert!(heights.is_sorted_by(|a, b| a > b), "{heights:?}");
        // The space before it would leave it further down: 54 points of margin, and some of its size.
        assert!(lines[0].top < 2 * 72, "{:?}", lines[0]);
    }

    /// A word processor's code block is its lines one under the other: here, one box the width of the
    /// text, blank lines included, on either paper.
    #[test]
    fn a_code_block_is_one_box_as_wide_as_the_text() {
        let box_of = |paper| {
            let frame = looked_at("Above\n\n```\none\n\nthree\n```\n\nBelow", paper);
            let empty = colour_at(&frame, 0, 0);
            // Inside its right edge, which the short text above and below it doesn't reach.
            let x = PAGE_WIDTH - SIDE_PX - 4;
            let rows: Vec<u32> = (0..frame.height).filter(|y| colour_at(&frame, x, *y) != empty).collect();
            let (top, bottom) = (rows[0], *rows.last().unwrap());
            assert_eq!(rows.len() as u32, bottom - top + 1, "{paper:?}: a gap in it");
            let middle = (top + bottom) / 2;
            let wash = colour_at(&frame, x, middle);
            // Its lines are fills that meet on parts of a pixel, by a step or two that doesn't show.
            let near = |colour: [u8; 3]| colour.iter().zip(wash).all(|(a, b)| a.abs_diff(b) <= 3);
            assert!((top + 1..bottom).all(|y| near(colour_at(&frame, x, y))), "{paper:?}: a seam in it");
            for (x, inside) in [(SIDE_PX - 3, false), (SIDE_PX + 3, true), (PAGE_WIDTH - SIDE_PX - 3, true), (PAGE_WIDTH - SIDE_PX + 3, false)] {
                assert_eq!(colour_at(&frame, x, middle) != empty, inside, "{paper:?}: at {x}");
            }
            (top, bottom, empty, wash, frame)
        };
        let (top, bottom, _, wash, frame) = box_of(Paper::Printed);
        assert!(wash.iter().zip([WASH.0, WASH.1, WASH.2]).all(|(a, b)| a.abs_diff(b) <= 3), "{wash:?}");
        let lines = lines_of(&frame, dark_ink);
        let [above, one, three, below] = lines[..] else { panic!("{lines:?}") };
        assert!(above.bottom < top && top < one.top && three.bottom < bottom && bottom < below.top, "{top}..{bottom}: {lines:?}");
        // On dark paper, a little lighter than the page, not a hole in it or a light in it.
        let (_, _, empty, wash, _) = box_of(Paper::Dark([0x1E; 3]));
        assert_eq!(empty, [0x1E; 3]);
        assert!(brightness(wash) > brightness(empty) && brightness(wash) < brightness(empty) + 16, "{wash:?}");
    }

    /// A paragraph's room around each line would leave them apart, in their box.
    #[test]
    fn a_code_blocks_lines_are_close() {
        let lines = lines_of(&looked_at("```\nHxg\nHxg\n```", Paper::Printed), dark_ink);
        let [one, two] = lines[..] else { panic!("{lines:?}") };
        assert!(two.top - one.top < 2 * 16, "{lines:?}");
    }

    #[test]
    fn code_is_monospaced_among_text_and_in_a_block_as_text_isnt() {
        let frame = looked_at("`llllllllll`\n\n`MMMMMMMMMM`\n\n```\nllllllllll\nMMMMMMMMMM\n```\n\nllllllllll\n\nMMMMMMMMMM", Paper::Printed);
        let lines = lines_of(&frame, dark_ink);
        assert_eq!(lines.len(), 6, "{lines:?}");
        let width = |at: usize| (lines[at].right - lines[at].left) as f32;
        for (narrow, wide, what) in [(0, 1, "among text"), (2, 3, "in a block")] {
            assert!(width(narrow) / width(wide) > 0.85, "{what}: {} of {}", width(narrow), width(wide));
        }
        assert!(width(4) / width(5) < 0.6, "text: {} of {}", width(4), width(5));
    }

    /// What most code is written to: at width 83, what a US Letter page held, 80 columns fit a line
    /// of a block and 81 don't.
    #[test]
    fn eighty_columns_of_code_fit_on_a_line_at_a_letter_pages_width() {
        let lines =
            |columns: usize| lines_of(&looked_at_on(&format!("```\n{}\n```", "0".repeat(columns)), Paper::Printed, Some("83")), dark_ink).len();
        assert_eq!((lines(80), lines(81)), (1, 2));
    }

    #[test]
    fn a_quote_is_where_text_is_with_a_bar_beside_it() {
        let frame = looked_at("> One\n>\n> Two", Paper::Printed);
        let lines = lines_of(&frame, dark_ink);
        let [one, two] = lines[..] else { panic!("{lines:?}") };
        // As a word processor has it, it would be in the middle of the page.
        assert!(one.left < SIDE_PX + 2 * 14 && two.left < SIDE_PX + 2 * 14, "{lines:?}");
        // The bar goes on between its paragraphs.
        let bar = (SIDE_PX..one.left).find(|x| not_white(colour_at(&frame, *x, one.top))).expect("a bar");
        assert!((one.top..=two.bottom).all(|y| not_white(colour_at(&frame, bar, y))), "the bar stops");
    }

    #[test]
    fn a_link_stands_out_on_either_paper_and_text_doesnt() {
        let blue = |markdown: &str, paper| {
            let frame = looked_at(markdown, paper);
            frame.rgba.as_chunks::<4>().0.iter().any(|pixel| i32::from(pixel[2]) > i32::from(pixel[0]) + 60)
        };
        for paper in [Paper::Printed, Paper::Dark([0x1E; 3])] {
            assert!(blue("[One](https://example.com)", paper), "{paper:?}");
            assert!(!blue("One", paper), "{paper:?}");
        }
    }

    #[test]
    fn lists_stand_apart_as_their_items_dont() {
        let lines = lines_of(&looked_at("- one\n- two\n\n1. three\n2. four\n\nAfter", Paper::Printed), dark_ink);
        let [one, two, three, four, after] = lines[..] else { panic!("{lines:?}") };
        let (items, lists) = (two.top - one.bottom, three.top - two.bottom);
        assert!(lists >= items + 2 * 5, "{items} between items, {lists} between lists");
        assert!(after.top - four.bottom >= items + 2 * 5, "after a list: {lines:?}");
    }

    /// Only a bullet goes, since a numbered task keeps its number; and what is in a table is styled too.
    #[test]
    fn a_tasks_box_takes_the_place_of_its_bullet() {
        let laid = laid_out(
            b"- one\n- [x] two\n\n1. [x] three\n\n| [`a`](b) |\n|---|\n",
            Kind::Markdown,
            None,
            "a",
            None,
            At { width: Width::Window, font: 100, margin: MARGIN_DEFAULT },
            &ROOMLESS,
        )
        .unwrap();
        let document = laid.document.read().unwrap();
        let paras: Vec<&wordcraft_doc::Paragraph> =
            document.para_paths(StoryRef::Body).iter().filter_map(|path| document.para(StoryRef::Body, path)).collect();
        let [one, two, three, cell, ..] = paras[..] else { panic!("{} paragraphs", paras.len()) };
        assert!(one.props.numbering.is_some());
        assert_eq!((two.props.numbering, two.text.as_str()), (None, "☑\ttwo"));
        assert_eq!(two.props.indent_left.unwrap() + two.props.indent_first.unwrap(), 6.0, "where the bullet would be");
        assert!(three.props.numbering.is_some() && three.text == "☑ three", "{:?}", three.text);
        let run = &cell.runs[0].props;
        assert_eq!((run.font.as_deref(), run.color), (Some(CODE_FACE), Some(TextColor::Rgb(LINK))));
    }

    #[test]
    fn a_table_keeps_its_distance_from_what_follows() {
        let lines = lines_of(&looked_at("| One |\n|---|\n| Two |\n\nAfter", Paper::Printed), not_white);
        let [table, after] = lines[..] else { panic!("{lines:?}") };
        assert!(after.top - table.bottom >= 2 * 10, "{lines:?}");
    }

    #[test]
    fn a_footnotes_number_is_on_its_line() {
        let lines = lines_of(&looked_at("One[^1]\n\n[^1]: Two", Paper::Printed), dark_ink);
        assert_eq!(lines.len(), 2, "{lines:?}");
        // What only looks like one, cut short or with no number, stays as it is.
        for html in ["<sup class=\"footnote-definition-label\">", "<sup class=\"footnote-definition-label\">x</sup>\n<p>y", "z"] {
            assert_eq!(numbers_in_their_footnotes(html), html);
        }
    }

    /// A dark page inverts a colour's lightness, which leaves one of middling lightness about as it was:
    /// a purple fine on white came out at 2.3 to 1 on dark paper.
    #[test]
    fn what_is_coloured_can_be_read_on_either_paper() {
        let contrast = |a: Rgb, b: Rgb| {
            let linear = |channel: u8| {
                let share = f32::from(channel) / 255.0;
                if share <= 0.04045 { share / 12.92 } else { ((share + 0.055) / 1.055).powf(2.4) }
            };
            let light = |c: Rgb| 0.2126 * linear(c.0) + 0.7152 * linear(c.1) + 0.0722 * linear(c.2);
            let (a, b) = (light(a), light(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        let kinds = [BlockQuoteKind::Note, BlockQuoteKind::Tip, BlockQuoteKind::Important, BlockQuoteKind::Warning, BlockQuoteKind::Caution];
        let dark = RenderOptions { dark: true, dark_paper: 0x1E, ..Default::default() };
        for colour in [INK, QUIET_INK, LINK].into_iter().chain(kinds.map(|kind| alert(kind).1)) {
            assert!(contrast(colour, Rgb::WHITE) >= 4.5, "{colour:?} on white");
            assert!(contrast(dark.ink(colour), Rgb(0x1E, 0x1E, 0x1E)) >= 4.5, "{colour:?} on dark, as {:?}", dark.ink(colour));
        }
    }

    /// As written, an alert is a quote that starts with its tag.
    #[test]
    fn an_alert_says_what_it_is() {
        let html = as_html("> [!WARNING]\n> Mind it\n\n> [!MAYBE]\n> Not one");
        assert!(html.contains(">Warning</strong>") && !html.contains("[!WARNING]"), "{html}");
        assert!(html.contains("[!MAYBE]"), "{html}");
        // Right above its text, a line's height (about 17.5 points) from it, not a paragraph apart.
        let lines = lines_of(&looked_at("> [!NOTE]\n> Mind it", Paper::Printed), dark_ink);
        let [kind, text] = lines[..] else { panic!("{lines:?}") };
        assert!(text.top - kind.top < 2 * 20, "{lines:?}");
        // With no paragraph to go on, it still says.
        for alert in ["> [!TIP]\n> - One", "> [!TIP]"] {
            assert!(as_html(alert).contains(">Tip</strong>"), "{alert}");
        }
    }

    /// `markdown` as HTML, in a build that draws no fenced block.
    fn as_html(markdown: &str) -> String {
        markdown_as_html(markdown, &Pictures::among(None, &[]), (468.0, 648.0))
    }

    /// A fenced block written in a language a backend draws is written as a picture, named by a key
    /// of the pictures', its info string's first word its language in any case, behind backticks or
    /// tildes, in a list or a quote. Every other block is written as it always was: an indented one,
    /// one with no language or one no backend lists, and one its backend refuses, known before any
    /// HTML of it is written.
    #[test]
    fn a_fenced_block_a_backend_draws_is_written_as_its_picture() {
        use crate::source::tests::Sketches;
        static SKETCHES: Sketches = Sketches { fences: &["sketch"], pictures: true };
        static DRAWING: [&dyn Backend; 1] = [&SKETCHES];
        let pictures = Pictures::among(None, &DRAWING);
        let drawn = |markdown: &str| markdown_as_html(markdown, &pictures, (468.0, 648.0));
        let html = drawn("Before\n\n```sketch\n40 20\n```\n\nAfter\n");
        let Some((before, rest)) = html.split_once("<p><img src=\"") else { panic!("{html}") };
        let Some((key, after)) = rest.split_once("\" alt=\"\"></p>\n") else { panic!("{html}") };
        assert_eq!((before, after), ("<p>Before</p>\n", "<p>After</p>\n"));
        assert!(pictures.measured(key, (468.0, 648.0)).is_some(), "{key}");
        for block in [
            "```Sketch title=\"x\"\n40 20\n```",
            "~~~~ SKETCH\n40 20\n~~~~",
            "- One\n\n  ```sketch\n  40 20\n  ```\n",
            "> [!TIP]\n> ```sketch\n> 40 20\n> ```",
        ] {
            let html = drawn(block);
            assert!(html.contains("<img src=\"fenced:") && !html.contains("<pre>") && !html.contains("40 20"), "{block}: {html}");
        }
        assert!(drawn("> [!TIP]\n> ```sketch\n> 40 20\n> ```").contains(">Tip</strong></p><p><img "), "what an alert is, before its block");
        // As code, exactly as a build that draws none writes it.
        for block in [
            "```svg\n<svg/>\n```",
            "```\n40 20\n```",
            "    40 20\n",
            "```sketches\n40 20\n```",
            "```a sketch\n40 20\n```",
            "```sketch\nflowchart <LR>\n  a & b\n```\n\nAfter",
            "```sketch\n```",
        ] {
            let html = drawn(block);
            assert!(html.contains("<pre><code") && !html.contains("<img"), "{block}: {html}");
            assert_eq!(html, as_html(block), "{block}");
        }
        assert_eq!(drawn("```sketch\n40 20").matches("<img ").count(), 1, "a block never closed ends with the document, as code would");
        assert_eq!(
            (language_of("mermaid"), language_of("  mermaid\ttitle"), language_of(""), language_of("   ")),
            (Some("mermaid"), Some("mermaid"), None, None)
        );
    }

    /// The pager's way: pages rendering side by side on a pool's threads, each drawing its pictures
    /// into what they share, and every page showing them, its own and the one on every page. Many times
    /// over, as a page that finds that one drawn by another thread must not render its stand-in, which
    /// only shows sometimes.
    #[cfg(feature = "image")]
    #[test]
    fn a_pools_threads_draw_their_pages_pictures() {
        let dir = std::env::temp_dir().join(format!("clview-word-pool-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let picture = |red: u8| {
            let pixels: Vec<u8> = [red, 0, 0, 255].repeat(400);
            crate::encode::encode_png(20, 20, &pixels, crate::encode::PngEffort::Fast).unwrap()
        };
        let shows =
            |frame: &Frame, red: u8| frame.rgba.as_chunks::<4>().0.iter().any(|pixel| pixel[0].abs_diff(red) < 3 && pixel[1] < 3 && pixel[2] < 3);
        const SHARED: u8 = 250;
        std::fs::write(dir.join("shared.png"), picture(SHARED)).unwrap();
        let pages = 6u8;
        let mut document = String::new();
        for n in 0..pages {
            std::fs::write(dir.join(format!("{n}.png")), picture(40 * (n + 1))).unwrap();
            document.push_str(&format!("<p style=\"page-break-after:always\"><img src=\"{n}.png\"> <img src=\"shared.png\"></p>"));
        }
        let path = dir.join("a.html");
        std::fs::write(&path, document).unwrap();

        for round in 0..300 {
            let input = crate::source::Input::open(path.to_str().unwrap()).unwrap();
            let Ok(Opened::Pages(source)) = Wordcraft.open(input, &Parsed::default()) else { panic!("not pages") };
            assert!(source.pages() >= usize::from(pages));
            let pool = source.pool(4);
            let requests: Vec<Request> = (0..usize::from(pages)).map(|page| Request { page, scale: 0.5, ..Request::default() }).collect();
            let started = std::time::Instant::now();
            let mut done = vec![false; requests.len()];
            while done.iter().any(|page| !page) {
                assert!(started.elapsed().as_secs() < 60, "still waiting for {done:?}");
                pool.set_queue(requests.iter().filter(|request| !done[request.page]).copied().collect());
                let Some(frame) = pool.try_recv() else {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                };
                assert_eq!(frame.error, None);
                let page = frame.request.page;
                assert!(shows(&frame, 40 * (u8::try_from(page).unwrap() + 1)), "round {round}: page {page} doesn't show its picture");
                assert!(shows(&frame, SHARED), "round {round}: page {page} doesn't show the one they share");
                done[page] = true;
            }
        }
    }

    /// A document's page that is a list's tile draws the pictures it refers to as tiles too: a
    /// drawing's HTML labels aren't drawn for it, so it starts no worker; opened, the page draws them.
    #[cfg(feature = "svg-html")]
    #[test]
    fn a_tiles_pictures_are_drawn_as_tiles() {
        let dir = std::env::temp_dir().join(format!("clview-word-tile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let drawing = "<svg xmlns='http://www.w3.org/2000/svg' width='60' height='60'><rect width='60' height='60' fill='red'/>\
            <foreignObject width='60' height='30'><div xmlns='http://www.w3.org/1999/xhtml' style='width:100%;height:100%;background:blue'/></foreignObject></svg>";
        std::fs::write(dir.join("labelled.svg"), drawing).unwrap();
        let path = dir.join("a.md");
        std::fs::write(&path, "![a drawing](labelled.svg)").unwrap();
        let shows = |frame: &Frame, colour: [u8; 3]| frame.rgba.as_chunks::<4>().0.iter().any(|p| p[..3] == colour && p[3] == 255);
        let page = |tile: bool| {
            let input = crate::source::Input::open(path.to_str().unwrap()).unwrap();
            let Ok(Opened::Pages(source)) = Wordcraft.open(input, &Parsed::default()) else { panic!("not pages") };
            let frame = source.renderer().unwrap().render(Request { page: 0, scale: 1.0, tile, ..Request::default() });
            assert_eq!(frame.error, None);
            frame
        };
        let tile = page(true);
        assert!(shows(&tile, [255, 0, 0]) && !shows(&tile, [0, 0, 255]), "a tile draws the drawing without its label");
        let opened = page(false);
        assert!(shows(&opened, [255, 0, 0]) && shows(&opened, [0, 0, 255]), "a page draws its label");
    }

    #[test]
    fn a_page_is_drawn_with_its_size_at_the_scale_rounded_up() {
        assert_eq!(pixels(612.0, 1.0), Ok(612));
        assert_eq!(pixels(200.0, 1.005), Ok(201));
        assert_eq!(pixels(0.0, 2.0), Ok(1), "at least one pixel");
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(pixels(612.0, scale).is_err(), "{scale}");
        }
        assert!(pixels(f32::MAX, f32::MAX).is_err());
    }

    /// `contents` opened as a file named `name`, laid out for no room of the viewer's.
    fn paged(name: &str, contents: &[u8]) -> Paged {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let at = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("clview-word-text-{}-{at}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        let input = crate::source::Input::open(path.to_str().unwrap()).unwrap();
        opened(input, &Parsed::default(), ROOMLESS).unwrap()
    }

    /// A docx of one 200 × 300 pt page whose body is `body`, with `settings`.
    fn docx(body: &str, settings: &str) -> Vec<u8> {
        use std::io::Write as _;
        const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let files = [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/></Types>"#.to_owned(),
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_owned(),
            ),
            (
                "word/_rels/document.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/></Relationships>"#.to_owned(),
            ),
            (
                "word/document.xml",
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}<w:sectPr><w:pgSz w:w="4000" w:h="6000"/><w:pgMar w:top="720" w:right="720" w:bottom="720" w:left="720"/></w:sectPr></w:body></w:document>"#
                ),
            ),
            ("word/settings.xml", format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings xmlns:w="{W}">{settings}</w:settings>"#)),
        ];
        let mut zip = ::zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in files {
            zip.start_file(name, ::zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(data.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// What `texts` gives for `wanted`, each once, within twenty seconds.
    fn gave(texts: &dyn Texts, wanted: Vec<Want>) -> Vec<Got> {
        let (mut got, begun) = (Vec::new(), std::time::Instant::now());
        texts.set_queue(wanted.clone());
        while got.len() < wanted.len() && begun.elapsed() < std::time::Duration::from_secs(20) {
            match texts.try_recv() {
                Some(one) => got.push(one),
                None => std::thread::sleep(std::time::Duration::from_millis(1)),
            }
        }
        assert_eq!(got.len(), wanted.len(), "not all given");
        got
    }

    /// Every chunk's text of `paged`, in order.
    fn all_text(paged: &Paged) -> Vec<String> {
        let texts = paged.texts().unwrap();
        let got = gave(&*texts, (0..texts.chunks()).map(Want::Text).collect());
        got.into_iter()
            .map(|got| match got {
                Got::Text(_, Ok(text)) => text,
                Got::Text(chunk, Err(why)) => panic!("chunk {chunk}: {why}"),
                Got::Rects(..) => panic!("rectangles not asked for"),
            })
            .collect()
    }

    /// The rectangles `texts` gives for the characters `from..to` of chunk `chunk`.
    fn placed(texts: &dyn Texts, chunk: usize, from: usize, to: usize) -> Vec<(usize, [f32; 4])> {
        match gave(texts, vec![Want::Rects { chunk, from, to }]).pop() {
            Some(Got::Rects(_, Ok(rects))) => rects,
            _ => panic!("no rectangles"),
        }
    }

    /// A document's text is its body's paragraphs, a chunk each, in order, a table's cells
    /// included: each its own text, a soft hyphen as nothing and a non-breaking one as a hyphen, so
    /// a word is found as it reads; and all on the one page they are on.
    #[test]
    fn a_documents_chunks_are_its_body_paragraphs_a_tables_cells_included() {
        let markdown = "# Title\n\nSoft hy\u{AD}phen and non\u{2011}breaking.\n\n| one | two |\n|---|---|\n| three | four |\n\nAfter.\n";
        let paged = paged("a.md", markdown.as_bytes());
        let text = all_text(&paged);
        assert_eq!(text.first().map(String::as_str), Some("Title"), "{text:?}");
        assert!(text.contains(&"Soft hyphen and non-breaking.".to_owned()), "{text:?}");
        let cells: Vec<&str> = text.iter().map(String::as_str).filter(|t| ["one", "two", "three", "four"].contains(t)).collect();
        assert_eq!(cells, ["one", "two", "three", "four"]);
        assert_eq!(text.last().map(String::as_str), Some("After."), "{text:?}");
        let texts = paged.texts().unwrap();
        assert_eq!((texts.chunks(), texts.chunks_on(0), texts.chunks_on(1)), (text.len(), 0..text.len(), 0..0));
        // Where "hyphen" is: one rectangle, on the line its paragraph is on, wider than its letters.
        // Where "hyphen" is, after a soft hyphen of two bytes: one rectangle, on its paragraph's line,
        // ending a space before where "and" starts.
        let at = text[1].find("hyphen").unwrap();
        let rects = placed(&*texts, 1, at, at + 6);
        let [(0, [x0, y0, x1, y1])] = rects[..] else { panic!("{rects:?}") };
        assert!(x1 > x0 + 10.0 && y1 > y0, "{rects:?}");
        let and = placed(&*texts, 1, at + 7, at + 10);
        assert!(matches!(and[..], [(0, [start, ..])] if start > x1 && start < x1 + 5.0), "{rects:?} {and:?}");
    }

    /// An inline field is its result, as it is drawn.
    #[test]
    fn a_fields_result_is_its_text() {
        let body = r#"<w:p><w:r><w:t xml:space="preserve">By </w:t></w:r><w:fldSimple w:instr=" AUTHOR "><w:r><w:t>Ada Lovelace</w:t></w:r></w:fldSimple><w:r><w:t>.</w:t></w:r></w:p>"#;
        // A page number saved as 99 shows the page it lands on, as the layout computes it, and is
        // left out; so is the count of pages.
        let page = |instr: &str, saved: &str| {
            format!(
                r#"<w:p><w:r><w:t xml:space="preserve">Page </w:t></w:r><w:fldSimple w:instr=" {instr} "><w:r><w:t>{saved}</w:t></w:r></w:fldSimple><w:r><w:t>.</w:t></w:r></w:p>"#
            )
        };
        let body = format!("{body}{}{}", page("PAGE", "99"), page("NUMPAGES", "98"));
        let paged = paged("field.docx", &docx(&body, ""));
        assert_eq!(all_text(&paged), ["By Ada Lovelace.", "Page \u{FFFC}.", "Page \u{FFFC}."]);
        let texts = paged.texts().unwrap();
        let rects = placed(&*texts, 0, 3, 7);
        assert!(matches!(rects[..], [(0, [x0, _, x1, _])] if x1 > x0), "the field's result is placed: {rects:?}");
    }

    /// A field left out of a search stands as a character no query holds, so that no match runs
    /// across it: `ab`, a page number, `cd` is never `abcd`. Fields drawn otherwise than as their
    /// result are left out too, but a drop-down, its entry shown: an overstrike (`EQ \o`), a
    /// legacy check box.
    #[test]
    fn a_field_left_out_stops_a_match() {
        let field = |instr: &str, saved: &str| format!(r#"<w:fldSimple w:instr=" {instr} "><w:r><w:t>{saved}</w:t></w:r></w:fldSimple>"#);
        let form = |data: &str, instr: &str, saved: &str| {
            format!(
                r#"<w:r><w:fldChar w:fldCharType="begin"><w:ffData><w:name w:val="F"/>{data}</w:ffData></w:fldChar></w:r><w:r><w:instrText xml:space="preserve"> {instr} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>{saved}</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
            )
        };
        let para = |inner: &str| format!(r#"<w:p><w:r><w:t>ab</w:t></w:r>{inner}<w:r><w:t>cd</w:t></w:r></w:p>"#);
        let body = [
            para(&field("PAGE", "9")),
            para(&field(r"EQ \o(=,/)", "stale")),
            para(&form(r#"<w:checkBox><w:sizeAuto/><w:default w:val="0"/></w:checkBox>"#, "FORMCHECKBOX", "stale")),
            para(&form(r#"<w:ddList><w:result w:val="1"/><w:listEntry w:val="One"/><w:listEntry w:val="Two"/></w:ddList>"#, "FORMDROPDOWN", "stale")),
            para(&form("<w:textInput/>", "FORMTEXT", "typed")),
        ]
        .concat();
        let paged = paged("left.docx", &docx(&body, ""));
        let stand = "ab\u{FFFC}cd";
        assert_eq!(all_text(&paged), [stand, stand, stand, "abTwocd", "abtypedcd"]);
        // The stand-in is the field's: a range over it is placed where the field is drawn.
        let texts = paged.texts().unwrap();
        let rects = placed(&*texts, 0, 1, 4);
        assert!(matches!(rects[..], [(0, [x0, _, x1, _])] if x1 > x0), "{rects:?}");
    }

    /// A word hyphenated at a line's end is found whole, its rectangles on both lines, the first
    /// reaching the line's end.
    #[test]
    fn a_word_hyphenated_at_a_lines_end_is_placed_on_both_lines() {
        let body = r#"<w:p><w:r><w:rPr><w:lang w:val="en-US"/></w:rPr><w:t>Internationalization considerations notwithstanding, extraordinarily comprehensive documentation characteristically demonstrates unquestionable professionalism</w:t></w:r></w:p>"#;
        let paged = paged("hyphens.docx", &docx(body, "<w:autoHyphenation/>"));
        let text = all_text(&paged).remove(0);
        let texts = paged.texts().unwrap();
        let mut split = Vec::new();
        let mut at = 0;
        for word in text.split(' ') {
            let chars = word.chars().count();
            let rects = placed(&*texts, 0, at, at + chars);
            if rects.len() == 2 {
                split.push((word.to_owned(), rects));
            }
            at += chars + 1;
        }
        let Some((_, rects)) = split.first() else { panic!("no word hyphenated in {text:?}") };
        let [(0, first), (0, second)] = rects[..] else { panic!("{rects:?}") };
        assert!(second[1] >= first[3] - 0.5, "the second line below the first: {rects:?}");
        assert!(first[2] > 150.0 && second[0] < 50.0, "the head at the line's end, the tail at the next's start: {rects:?}");
        // The hyphen drawn at the line's end is within the first: no ink to its right on that line.
        const SCALE: f32 = 4.0;
        let frame = paged.renderer().unwrap().render(Request { page: 0, scale: SCALE, ..Request::default() });
        let width = frame.width as usize;
        let rows = (first[1] * SCALE).ceil() as usize..(first[3] * SCALE).floor() as usize;
        let inked = |x: usize, y: usize| frame.rgba[(y * width + x) * 4..][..3].iter().any(|v| *v < 160);
        let right = (first[2] * SCALE).ceil() as usize + 1;
        assert!(rows.clone().all(|y| (right..width).all(|x| !inked(x, y))), "ink right of the head: the hyphen left out");
        assert!(rows.clone().any(|y| (((first[2] - 4.0) * SCALE) as usize..right).any(|x| inked(x, y))), "no ink at the head's end");
    }

    /// A paragraph past what a search keeps of one is left unread, with the page it is on; the
    /// others are read.
    #[test]
    fn a_paragraph_over_a_mebibyte_is_left_unread_with_its_page() {
        let markdown = format!("First.\n\n{}\n\nLast.\n", "word ".repeat(220_000));
        let paged = paged("long.md", markdown.as_bytes());
        let texts = paged.texts().unwrap();
        let got = gave(&*texts, vec![Want::Text(0), Want::Text(1), Want::Text(2)]);
        let [Got::Text(0, Ok(first)), Got::Text(1, Err(why)), Got::Text(2, Ok(last))] = &got[..] else { panic!("not what was asked") };
        assert_eq!((first.as_str(), last.as_str()), ("First.", "Last."));
        assert_eq!(why, "a paragraph on page 1 has more text than a search keeps of one");
    }

    /// A page's chunks run from the least paragraph on it or after to the most on it or before: its
    /// own, and of a table row over several pages, the row's on its neighbours too, never starting
    /// or ending before an earlier page's; a page with none has none.
    #[test]
    fn a_pages_chunks_take_in_a_table_row_over_several_pages() {
        // A row whose left cell has a paragraph on each of three pages, and whose right cell's one
        // (the fourth in the document) is on the first.
        assert_eq!(ranges_of(&[vec![0, 3], vec![1], vec![2], vec![4]]), [0..4, 1..4, 2..4, 4..5]);
        assert_eq!(ranges_of(&[vec![0, 1], vec![], vec![2]]), [0..2, 0..0, 2..3]);
        assert_eq!(ranges_of(&[vec![1], vec![0]]), [0..2, 0..2], "out of order: both, rather than none");
        // Such a row in a docx: its left cell's three paragraphs over several pages, its right
        // cell's on the first; then a paragraph after the table.
        let words = |name: &str| format!("{name} {}", "word ".repeat(40));
        let para = |text: &str| format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>");
        let left: String = ["Pone", "Ptwo", "Pthree"].iter().map(|name| para(&words(name))).collect();
        let body = format!("<w:tbl><w:tr><w:tc>{left}</w:tc><w:tc>{}</w:tc></w:tr></w:tbl>{}", para("Qright"), para("After"));
        let paged = paged("row.docx", &docx(&body, ""));
        let laid = paged.0.laid(&paged.0.shared()).unwrap();
        let (pages, _) = paragraphs_of(&laid);
        assert!(pages.len() > 2 && pages[0].iter().any(|para| para.index == 3), "not a row over pages: {pages:?}");
        let on = &paged.0.on;
        assert!(on.windows(2).all(|pair| pair[0].start <= pair[1].start && pair[0].end <= pair[1].end), "{on:?}");
        for (page, paras) in pages.iter().enumerate() {
            assert!(paras.iter().all(|para| on[page].contains(&(para.index as usize))), "page {page}: {paras:?} not in {:?}", on[page]);
        }
    }

    /// A search reads the layout the pool renders with: laid out once for both, whichever asks
    /// first; with both gone, it is let go of, and laid out again for the next.
    #[test]
    fn the_pool_and_a_search_lay_a_document_out_once() {
        let layouts = |paged: &Paged| paged.0.layouts.load(std::sync::atomic::Ordering::Relaxed);
        let rendered = |pool: &dyn Pool| {
            pool.set_queue(vec![Request { page: 0, scale: 1.0, ..Request::default() }]);
            let begun = std::time::Instant::now();
            loop {
                if let Some(frame) = pool.try_recv() {
                    assert_eq!(frame.error, None);
                    break;
                }
                assert!(begun.elapsed() < std::time::Duration::from_secs(20), "never rendered");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        let paged = paged("once.md", b"# One\n\nTwo.\n");
        // The search first, then the pool.
        let texts = paged.texts().unwrap();
        gave(&*texts, vec![Want::Text(1)]);
        let pool = paged.pool(1);
        rendered(&*pool);
        assert_eq!(layouts(&paged), 1);
        // Both let go of, once their threads have stopped: the next pool lays it out again, and a
        // search started meanwhile shares it.
        drop((texts, pool));
        let begun = std::time::Instant::now();
        while paged.0.shared.lock().unwrap().upgrade().is_some() {
            assert!(begun.elapsed() < std::time::Duration::from_secs(10), "the layout is held after its pool and search");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let pool = paged.pool(1);
        rendered(&*pool);
        let texts = paged.texts().unwrap();
        gave(&*texts, vec![Want::Text(0)]);
        assert_eq!(layouts(&paged), 2);
    }

    proptest! {
        /// Whatever a file starts with, and whatever its name, choosing never panics.
        #[test]
        fn any_first_bytes_are_safe(head in proptest::collection::vec(any::<u8>(), 0..1024), zip in any::<bool>(), name in "[a-z.]{0,8}") {
            let head = if zip { [ZIP, &head].concat() } else { head };
            let _ = Kind::of(&name, &head);
        }
    }
}
