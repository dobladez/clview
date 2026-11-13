//! `view --pager`: pages on the alternate screen, like `less`. One at a time, or two side by side
//! when the window is wide enough, turning two at a time like a book's; zoomed out, rows of
//! them; zoomed in, one as wide as the window. Several files, or a directory, are a list of
//! tiles, one chosen at a time, which Enter opens: a [`Level`] of its own, which Esc leaves; `/`
//! filters the tiles by name ([`Filter`]).
//!
//! [`Nav`] is where the reader is and what each key does, with no IO. [`Pager`] renders and
//! draws what `Nav` says, and [`Screen`] puts the terminal back however the pager ends.
//!
//! Turning a page should be instant, so the pager works ahead. The pages on screen render first,
//! then the next screens and the one before, on the source's threads (keys keep working meanwhile,
//! and a backend may give up on a page that takes too long). Each page is sent to the
//! terminal as soon as it is ready, under an image id of its own, and stays there while it is
//! near: turning to it only places it. The pages on screen stay until the next ones are ready,
//! so the screen never goes blank in between.
//!
//! A terminal on this machine is handed pages through temporary files when it takes them
//! (`files.rs`): only a path goes through the stream, so a page turn never waits behind the
//! megabytes of pages sent ahead, and neither side spends time on PNG.
//!
//! What the terminal keeps can still go: kitty drops images past its storage quota, and tmux
//! throws away what it can't deliver. The pages kept stay well under the quota ([`BUDGET`]); a
//! placement of an image the terminal no longer has is answered, and the page is sent again; and
//! inside tmux, the pager asks tmux after writing whether everything reached the terminal.
//! `docs/architecture.md` has the details.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::clip;
use crate::encode::PngEffort;
use crate::source::{
    Bounds, Container, Effect, Frame, Handle, Keys, Later, Named, Numbers, Openers, Pool, Ready, Request, Room, Setting, Source, Texts, Value, Values,
};

use super::files::{Files, Written};
use super::fit::{self, Area, Cell, FALLBACK_CELL, Fit, Grid, Layout, Zoom};
use super::keys::{self, Key};
use super::kitty::{self, Rect};
use super::relayout::{self, Ended, Hold, Relayout};
use super::search::{self, Search, fold};
use super::tty::{RawMode, Tty, WindowSize};
use super::{
    FALLBACK_SIZE, Found, Item, Opener, Opening, Output, PROGRESS_AFTER, PROGRESS_EVERY, Pages, QUIET, Shown, Terminal, cell_lines, chars, checked,
    cut, label, left_out_note, opened, page_layout, page_size, png, printable, reading, request, tmux,
};

/// How often the window size is checked while no key comes.
const TICK: Duration = Duration::from_millis(200);
/// How often renders in progress are checked on.
const POLL: Duration = Duration::from_millis(10);
/// How long a lone ESC waits for more before it counts as the Esc key. A longer escape sequence
/// cut off waits for the rest up to [`PENDING_LIMIT`], and is never a key.
const ESCAPE_WAIT: Duration = Duration::from_millis(50);
/// Rows `j`, `k`, the arrows and Enter scroll.
const STEP: u16 = 3;
/// The most digits a typed page number keeps.
const MAX_TYPED: usize = 9;
/// The most characters a list's filter keeps.
const MAX_FILTER: usize = 256;
/// What the status line says after a filter being typed, when there is room.
const FILTER_KEYS: &str = "⏎ keep · esc cancel";
/// What the status line says after a search being typed, when there is room.
const SEARCH_KEYS: &str = "⏎ search · esc cancel";
/// What `/` says in the pages of a file with no text to search, and `N` without a search.
const NO_TEXT: &str = "no text to search";
const NO_SEARCH: &str = "no search: / starts one";
/// What a relayout setting's keys say in a list, and while `--pages` picks the pages.
const REFLOW_IN_LIST: &str = "open a document to reflow it";
const REFLOW_WITH_PAGES: &str = "--pages picks pages as laid out: no reflow";
/// What the status line says while the document is laid out again with nothing to say of its values
/// (a change of size at the defaults), `…` after it.
const LAYING_OUT: &str = "laying out";
/// Bytes kept from an escape sequence or a terminal's answer still arriving; past this many, or
/// [`PENDING_LIMIT`], they are dropped.
const MAX_PENDING: usize = keys::MAX_ANSWER * 2;
const PENDING_LIMIT: Duration = Duration::from_secs(1);
/// Screens rendered and sent ahead of the one on screen, and behind it.
const AHEAD: usize = 2;
const BEHIND: usize = 1;
/// Pages kept in the terminal, ready to show at once: the most recently shown stay. More when a
/// screen shows many pages, so the screens around it stay too.
const MAX_SENT: usize = 12;
/// The most bytes of pixels kept in the terminal. kitty keeps 320 MB of images per screen, and
/// past that drops nearly every image not on screen at once; inside tmux, other panes' images
/// count against the same 320 MB.
const BUDGET: u64 = 192 << 20;
/// The least the budget comes down to when the terminal drops images all the same.
const MIN_BUDGET: u64 = 32 << 20;
/// Failures remembered, so a page that fails isn't rendered again and again.
const MAX_FAILED: usize = 16;
/// Render threads: one can work ahead while the other renders what the reader asked for.
const THREADS: usize = 2;
/// Inside tmux, how long after the last write to ask tmux whether it reached the terminal.
const TMUX_AFTER_WRITE: Duration = Duration::from_millis(150);
/// Inside tmux, how often to ask anyway: whether the pane shows again, or another terminal came.
const TMUX_EVERY: Duration = Duration::from_secs(1);
/// How many times the pager sends its pages again after tmux or the terminal threw some away,
/// before it stops trying by itself (`r` still does).
const MAX_RESENDS: u32 = 8;
/// How long the pager waits at the end for the terminal's answers to its last placements, so
/// none of them lands on the command line.
const ANSWER_WAIT: Duration = Duration::from_millis(100);
/// The reminder of the keys the status line starts with, until the first key: shorter and
/// shorter for a narrow window. In a list; in a file's pages, from a list or not.
const LIST_KEYS: [&str; 4] = [
    "arrows move · ⏎ open · + - zoom · , . per row · q quit",
    "arrows move · ⏎ open · + - zoom · q quit",
    "arrows · ⏎ open · q quit",
    "⏎ open · q quit",
];
const OPENED_KEYS: [&str; 4] = [
    "esc back · space b · n p · + - zoom · , . per row · q quit",
    "esc back · space b · n p · + - zoom · q quit",
    "esc back · n p · q quit",
    "esc back · q quit",
];
const KEYS: [&str; 4] = [
    "space b j k · n p · g G · 12⏎ · + - zoom · , . per row · q quit",
    "space b j k · n p · g G · 12⏎ · + - zoom · q quit",
    "n p · + - zoom · q quit",
    "n p · q quit",
];
/// The characters of the viewer's keys, in a file's pages or in a list: a setting may have none of
/// them (`source.rs` checks the build's).
#[cfg(test)]
pub(crate) const VIEWER_KEYS: &str = "0123456789?qucgGbjknNphl +=-_,.r/\u{c}";
/// The key for the keys, always on the status line.
const HELP: &str = "? help";
/// The widest the zoom is said: `8 per row`.
const ZOOM_WIDTH: usize = 9;
/// What `?` shows: each key, and what it does. In a file's pages:
const PAGE_HELP: &[(&str, &str)] = &[
    ("space  PageDown", "the next screen, or the rest of a page taller than the window"),
    ("b  PageUp", "back a screen"),
    ("j k  ↓ ↑  Enter", "a few rows down or up, or a row of pages"),
    ("n p  → ←", "the next or the previous screen of pages"),
    ("g G  Home End", "the first or the last page"),
    ("12 Enter  12g", "page 12"),
    ("+ -", "zoom in, as far as the window's width; out, to rows of pages"),
    (", .  3.", "one more page to a row, or one fewer, then the width; 3. sets 3, 0. auto"),
    ("/", "search the text as typed: ⏎ goes to the first match, Esc clears the search"),
    ("n  N p  3n", "with a search: the next match, or with N or p the one before; 3n the third on"),
    ("r  Ctrl-L", "render the pages again"),
    ("c", "copy the page to the clipboard"),
];
/// In a list:
const LIST_HELP: &[(&str, &str)] = &[
    ("← →  h l", "the one before or after"),
    ("↑ ↓  k j", "the one above or below"),
    ("space b  n p", "a screen on or back"),
    ("g G  Home End", "the first or the last"),
    ("12 Enter", "the 12th"),
    ("Enter", "open it: a file's pages, or a directory's files"),
    ("/", "filter by name as you type: ⏎ keeps it, Esc clears it"),
    ("+ -", "bigger, fewer to a row; smaller, more to a row"),
    (", .  3.", "one more tile to a row, or one fewer; 3. sets 3, 0. auto"),
    ("r  Ctrl-L", "render again"),
    ("c", "copy its picture to the clipboard"),
];
/// The `, .` line on a document laid out for its room, where they are columns.
const COLUMNS_HELP: (&str, &str) = (", .  3.", "one more column, or one fewer, down to one; 3. sets 3, 0. auto");
/// Leaving, gone into from a list or not, and these keys themselves.
const BACK_HELP: &[(&str, &str)] = &[("Esc  Backspace  u", "back to the list"), ("q  Ctrl-C", "quit")];
const QUIT_HELP: &[(&str, &str)] = &[("q  Esc  Ctrl-C", "quit")];
const HELP_HELP: (&str, &str) = ("?", "these keys: any key goes back");

/// Page through what is `shown` until the user quits, `note` on the status line first. Image ids
/// are taken in turn from `first_id` on ([`super::kitty::IdColor::id`]). A container gone into
/// from a list is listed by `opener`, a few files at a time, saying how far it has got. `room` is
/// what a document on screen is laid out for: the room what is shown was opened for, its count to a
/// row `zoom`'s. Returns
/// what the reader should know once the screen is back: why the files those containers left out
/// were.
#[expect(clippy::too_many_arguments, reason = "each is a different part of what the pager shows, and how")]
pub(super) fn run(
    tty: &Tty,
    shown: Shown,
    note: Option<String>,
    terminal: &Terminal,
    zoom: Zoom,
    first_id: u64,
    opener: &Opener<'_, '_>,
    room: Room,
) -> Result<Vec<String>, String> {
    // Inside tmux: who shows the pane, before any image goes.
    let mut watch = None;
    let mut showing = 1;
    if let Output::Tmux(_) = terminal.output
        && let Some(report) = tmux::report()
    {
        let mut w = tmux::Watch::default();
        showing = w.update(&report, 0).showing;
        watch = Some(w);
    }
    let settings =
        Rc::new(Settings { declared: super::settings(opener.backends), values: std::cell::Cell::new(terminal.values), changed: Default::default() });
    // The window's room; the count to a row a document is laid out in is the reader's ([`Nav::columns`]).
    let room = Room { per_row: None, ..room };
    let mut level = Level::new(0, shown, zoom, false, settings.clone(), room);
    level.nav.message = note;
    let mut pager = Pager {
        screen: Screen::enter(tty)?,
        tty,
        level,
        parents: Vec::new(),
        room,
        reading: None,
        going_into: None,
        levels: 1,
        later: Vec::new(),
        output: terminal.output,
        cell: terminal.cell,
        effort: terminal.effort,
        sent: Vec::new(),
        kept: MAX_SENT,
        wanted: Vec::new(),
        failed: Vec::new(),
        on_screen: Vec::new(),
        placed: Vec::new(),
        size: None,
        first_id,
        next_id: 0,
        tag: 0,
        settings,
        dark: terminal.dark,
        tmux_check: watch.as_ref().map(|_| Instant::now() + TMUX_EVERY),
        watch,
        hidden: false,
        transmitted: 0,
        resends: 0,
        budget: BUDGET,
        files: None,
        stalled_files: None,
        placed_at: None,
        on_glass: Vec::new(),
        copier: clip::Copier::new(terminal.local, terminal.effort, terminal.output.wrap()),
        openers: opener.openers(),
        relayout: Relayout::new(),
        hold: Hold::default(),
        bounds_for: None,
        ready: false,
    };
    // Files only where one terminal gets the escape codes: inside tmux, every terminal showing
    // the pane gets them, and the first to read a file deletes it before the others can. (Inside
    // a tmux that doesn't say, the pager can't know: no files.)
    if terminal.files && showing == 1 && (terminal.output == Output::Direct || pager.watch.is_some()) {
        let probe_id = pager.new_id();
        if let Some((files, probe)) = Files::start(probe_id, pager.output.wrap()) {
            pager.wrote(1);
            tty.write_all(&probe)?;
            pager.files = Some(files);
        }
    }
    pager.draw()?;
    let mut size = tty.size();
    let mut pending: Vec<u8> = Vec::new();
    let mut pending_since = Instant::now();
    loop {
        let wait = if !pending.is_empty() {
            ESCAPE_WAIT
        } else if pager.reading.as_ref().is_some_and(|r| r.opening.is_some()) {
            // Opening a container's files: only a look for keys between them.
            Duration::ZERO
        } else if pager.reading.is_some() {
            PROGRESS_EVERY.min(TICK)
        } else if pager.going_into.is_some() || pager.relayout.is_under_way() {
            POLL
        } else if let Some(wait) = pager.copier.wait() {
            wait
        } else if !pager.wanted.is_empty() && !pager.hidden {
            POLL
        } else if pager.level.nav.search.as_ref().is_some_and(Search::expecting) {
            // A search's text being read or matched, or a match's place asked for: taken between keys.
            POLL
        } else {
            TICK
        };
        // A room that changed is laid out for once it has held: wake then.
        let wait = pager.hold.due().map_or(wait, |at| wait.min(at.saturating_duration_since(Instant::now())));
        let wait = pager.tmux_check.map_or(wait, |at| wait.min(at.saturating_duration_since(Instant::now())));
        let read = tty.read_timeout(wait)?;
        if let Some(bytes) = &read {
            if pending.is_empty() {
                pending_since = Instant::now();
            }
            pending.extend_from_slice(bytes);
        }
        let mut redraw = false;
        if !pending.is_empty() {
            // When nothing more arrived, a lone ESC was the Esc key.
            let (keys, used) = keys::decode(&pending, read.is_none());
            pending.drain(..used.min(pending.len()));
            // What is left is the start of something cut off: it gets a while to complete.
            if used > 0 {
                pending_since = Instant::now();
            }
            if pending.len() > MAX_PENDING || pending_since.elapsed() > PENDING_LIMIT {
                pending.clear();
            }
            let mut quit = false;
            for key in keys {
                if let Key::Missing(id) = key {
                    redraw |= pager.missing(id);
                    continue;
                }
                if let Key::Clipboard(_) | Key::Copied(..) = key {
                    redraw |= pager.copied(key);
                    continue;
                }
                redraw = true;
                // Reading a container gone into: Esc (or the others that go back) stops it, and
                // the keys that quit, quit. Nothing else moves meanwhile.
                if pager.reading.is_some() || pager.going_into.is_some() {
                    match key {
                        Key::Char('q') | Key::Interrupt => quit = true,
                        Key::Escape | Key::Backspace | Key::Char('u') => pager.stop_reading(),
                        _ => {}
                    }
                    if quit {
                        break;
                    }
                    continue;
                }
                let mut step = pager.level.nav.key(key);
                // A setting's bounds may depend on another's value (the width's on the font size) and on
                // the pages to a row: asked again before the next key.
                pager.refresh_bounds();
                if let Step::Again(key) = step {
                    step = pager.level.nav.key(key);
                    pager.refresh_bounds();
                }
                match step {
                    Step::Continue | Step::Again(_) => {}
                    // Esc at the top, where there is nothing to go back to, quits.
                    Step::Quit => quit = true,
                    Step::Leave if pager.parents.is_empty() => quit = true,
                    Step::Leave | Step::Back => pager.back()?,
                    Step::Open(at) => pager.open(at, opener)?,
                    Step::ToClipboard(page) => pager.copy(page)?,
                    // Its text is read from here on, by the source, and let go of with the search.
                    Step::Search => {
                        let texts = pager.level.source.texts();
                        pager.level.nav.start_search(texts);
                    }
                }
                if quit {
                    break;
                }
            }
            if quit {
                pager.stop_reading();
                pager.await_answers();
                break;
            }
        } else if read.is_none() {
            // Nothing typed: redraw if the window changed size.
            let now = tty.size();
            redraw = now != size;
            size = now;
        }
        if pager.reading.is_some() {
            redraw |= pager.read_on(opener)?;
        }
        redraw |= pager.go_on_into()?;
        redraw |= pager.watch_room();
        pager.refresh_bounds();
        redraw |= pager.relayout()?;
        // A layout that failed put the count to a row back: the bounds follow it before the draw.
        pager.refresh_bounds();
        redraw |= pager.copy_on()?;
        let (searched, moved) = pager.search_on();
        redraw |= moved;
        let lost = pager.lost_files()?;
        // Nothing goes through tmux while the pane is hidden: it would be thrown away.
        let arrived = !pager.hidden && pager.receive()?;
        if arrived || redraw || lost {
            pager.draw()?;
        } else if searched {
            pager.status_line()?;
        }
        if pager.tmux_check.is_some_and(|at| Instant::now() >= at) && pager.ask_tmux() {
            pager.draw()?;
        }
    }
    Ok(std::mem::take(&mut pager.later))
}

/// What a key asks of the pager, beyond where the reader is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Continue,
    Quit,
    /// Esc: back, or, with nowhere to go back to, quit.
    Leave,
    /// Backspace or `u` (up): back, if there is somewhere to go back to; at the top, nothing.
    Back,
    /// Open the list's item at this place.
    Open(usize),
    /// Copy this page of the source to the clipboard: in a list, this tile's picture.
    ToClipboard(usize),
    /// This key again, once the bounds are asked again: it set the pages to a row back to `auto`
    /// first, and steps from the bounds of the room at `auto` ([`Nav::setting_key`]).
    Again(Key),
    /// `/` in a file's pages: a search of the source's text, if it has any ([`Nav::start_search`]).
    Search,
}

/// Where the reader is, and what keys do to it: in a file's pages, or in a list, where the reader
/// is on one of its tiles.
#[derive(Debug)]
struct Nav {
    /// The pages to page through, in order: `--pages`, or the whole document; a list's tiles.
    pages: Vec<usize>,
    /// Where in `pages` the reader is: in a list, the tile chosen.
    at: usize,
    /// Pages in the document; tiles in the list.
    count: usize,
    /// Whether this is a list.
    list: bool,
    /// Whether this was gone into from a list, which Esc goes back to.
    nested: bool,
    /// In a list, the first tile on screen, as the last move left it: the screen moves only when
    /// the tile chosen leaves it.
    top: usize,
    /// As asked: `--per-row`, then `+` `-` and `,` `.`.
    zoom: Zoom,
    /// From the last draw: pages to a row, rows of pages, and the most pages to a row the window
    /// takes. A zoom key sets the first two at once, so keys before the next draw step by them.
    per_row: u16,
    grid_rows: u16,
    most_per_row: u16,
    /// From the last draw: the first page's image rows on screen, and the most `scroll` can be.
    visible: u16,
    max_scroll: u16,
    /// Image rows scrolled off the top. Past the bottom until the next draw clamps it.
    scroll: u16,
    /// Where to scroll to once the page's rows are known, as a share of its height: the paragraph the
    /// reader was at, after the document is laid out again.
    scroll_to: Option<f32>,
    /// A page number being typed.
    typed: String,
    /// Shown in the status line until the next key.
    message: Option<String>,
    /// Whether the status line still reminds of the keys: until the first key.
    hint: bool,
    /// Whether the keys are on screen, in place of the pages (`?`).
    help: bool,
    /// Render the pages again even if nothing changed (`r`, Ctrl-L).
    rerender: bool,
    /// The settings this build declares, and their values, shared by every level.
    settings: Rc<Settings>,
    /// The settings what is shown has, by name: a key of another says so, and changes nothing.
    has: Vec<&'static str>,
    /// Where a setting's values sit for the document shown in its room, by the setting's name, as
    /// the source says ([`Source::bounds`]): asked when the level is made.
    bounds: Vec<(&'static str, Bounds)>,
    /// A list's tiles' names, by tile, as a filter matches them ([`fold`]); none for a file's
    /// pages.
    names: Vec<String>,
    /// A list's filter by name (`/`), while it has one: `pages` are then the tiles that match.
    filter: Option<Filter>,
    /// Whether `pages` were picked by `--pages`: then no relayout, which would lay out other pages.
    picked: bool,
    /// What the document on screen is laid out at, for the status line (`width 72`): a relayout
    /// setting's value, said as its key says it, when it isn't the default. None in a list.
    laid: Option<String>,
    /// The value a relayout under way is for, said the same way, while it is.
    relaying: Option<String>,
    /// A file's search of its text (`/`), while it has one: typed, then kept.
    search: Option<Search>,
    /// Where to scroll to once the page's rows are known, if it isn't on screen: the match the reader
    /// went to, from its top to its bottom as shares of the page's height.
    reveal: Option<(f32, f32)>,
}

/// A list narrowed to the tiles whose names hold some text, typed after `/`. The list is whole
/// when it shows (a container's files are all opened first), so no tile arrives later.
#[derive(Debug)]
struct Filter {
    /// What the names must hold, as typed: upper and lower case alike.
    text: String,
    /// Whether it is still being typed: then every key that types a character is part of it.
    typing: bool,
    /// The tiles it filters, in order: the list's, or those given (`--pages`).
    given: Vec<usize>,
    /// The tile chosen last, which while none matches is the one to come back near.
    last: Option<usize>,
    /// What Esc while typing puts back: the text kept before this `/`, if one was, and the tile
    /// chosen then.
    before: Option<String>,
    chosen_before: Option<usize>,
}

/// The settings this build declares ([`super::settings`]), and their values: they hold for the whole
/// run, a list and every file opened from it alike, so every level shares them.
#[derive(Debug, Default)]
struct Settings {
    declared: Vec<&'static Setting>,
    values: std::cell::Cell<Values>,
    /// A value changed since the last draw: every page sent goes, whichever level draws next. Kept
    /// here, not in a level's `Nav`, so that leaving the level it changed in before the draw (`d`
    /// then `u`, read together) doesn't lose it.
    changed: std::cell::Cell<bool>,
}

/// The relayout settings `source` has (`has`) and is laid out at a value of that isn't the default, or
/// that is past its most here: for the status line, said as the keys say the value, with what a number
/// past its most here is laid out at (`bounds`), `font 100% (80%)` too. None otherwise.
fn laid_at(settings: &Settings, source: &dyn Source, has: &[&str], bounds: &[(&'static str, Bounds)]) -> Option<String> {
    let (laid, _) = source.laid_at();
    let relayout = settings.declared.iter().filter(|setting| setting.effect == Effect::Relayout && has.contains(&setting.name));
    let shown = relayout.filter_map(|setting| {
        let bounds = bounds_of(bounds, setting.name);
        let past = |value: &Value| matches!((value, bounds.and_then(|bounds| bounds.most)), (Value::Number(n), Some(most)) if *n > most);
        let value = laid.get(setting.name).filter(|value| *value != setting.default || past(value))?;
        Some(said_in(setting, value, bounds))
    });
    shown.reduce(|all, one| format!("{all} · {one}"))
}

/// Where the settings `source` has (`has`) sit for it shown in `room` with `values`, by name, as the
/// source says ([`Source::bounds`]): those it has a say over.
fn bounds_in(settings: &Settings, source: &dyn Source, has: &[&'static str], room: Room, values: Values) -> Vec<(&'static str, Bounds)> {
    let declared = settings.declared.iter().filter(|setting| has.contains(&setting.name));
    declared.filter_map(|setting| Some((setting.name, source.bounds(setting.name, room, values)?))).collect()
}

/// Setting `name`'s bounds among `bounds`, if the source said any.
fn bounds_of(bounds: &[(&'static str, Bounds)], name: &str) -> Option<Bounds> {
    bounds.iter().find(|(setting, _)| *setting == name).map(|(_, bounds)| *bounds)
}

/// A setting's keys, as `?` lists them.
fn setting_keys(setting: &Setting) -> String {
    match setting.keys {
        Keys::Cycle(key) => key.to_string(),
        Keys::Step(down, up) if setting.numbers.is_some() => format!("{down} {up}  12{up}"),
        Keys::Step(down, up) => format!("{down} {up}"),
        Keys::Zoom => "+ -  150+".to_owned(),
    }
}

/// What `setting` at `value` is said as: a named value's own words, or `width 72`.
pub(super) fn said(setting: &Setting, value: Value) -> String {
    match value {
        Value::Named(name) => setting.named.iter().find(|named| named.name == name).map_or(name, |named| named.says).to_owned(),
        Value::Number(number) => format!("{} {number}{}", setting.name, setting.numbers.map_or("", |numbers| numbers.unit)),
    }
}

/// What `setting` at `value` is said as on a document shown in a room, where `bounds` say its values
/// sit there: as [`said`], and a number past the most a number is laid out at there says what it is
/// laid out at instead, `width 120 (window)`.
fn said_in(setting: &Setting, value: Value, bounds: Option<Bounds>) -> String {
    let said = said(setting, value);
    match (value, bounds.and_then(|bounds| bounds.most)) {
        (Value::Number(number), Some(most)) if number > most => match setting.named.first() {
            Some(named) => format!("{said} ({})", named.name),
            None => format!("{said} ({most}{})", setting.numbers.map_or("", |numbers| numbers.unit)),
        },
        _ => said,
    }
}

/// What `key`, one of `setting`'s, does to its value `now`, with `count` typed before it, on a document
/// whose values sit as `bounds` say in its room: the value it goes to, and what the reader is told of
/// it. A number typed sets a setting with numbers to it, within their range, and 0 to where the setting
/// says ([`Setting::zero`]). Otherwise a cycling key goes to the next named value, after the last the
/// first; a stepping key to the next among the steps down or up ([`stepped`]), up to the most the room
/// holds. At an end the value stays, and the line says so; a number past the most here is kept, and
/// says so.
fn turned(setting: &Setting, bounds: Option<Bounds>, now: Value, key: char, count: Option<&str>) -> (Value, String) {
    let says = |value: Value| said_in(setting, value, bounds);
    let at_end = |number: u32, end: &str| (Value::Number(number), format!("{}, the {end}", says(Value::Number(number))));
    if let (Some(numbers), Some(count)) = (setting.numbers, count) {
        // At most `MAX_TYPED` digits, so it fits; past the most, it is the most anyway.
        let wanted = count.parse::<u32>().unwrap_or(u32::MAX);
        if wanted == 0 {
            return (setting.zero, says(setting.zero));
        }
        let number = wanted.clamp(numbers.least, numbers.most.max(numbers.least));
        return if wanted > number {
            at_end(number, "most")
        } else if wanted < number {
            at_end(number, "least")
        } else {
            (Value::Number(number), says(Value::Number(number)))
        };
    }
    let value = match (setting.keys, setting.numbers, now) {
        (Keys::Cycle(_), ..) => {
            let at = setting.named.iter().position(|named| Value::Named(named.name) == now);
            let next = at.map_or(0, |at| at.saturating_add(1)).checked_rem(setting.named.len()).and_then(|next| setting.named.get(next));
            next.map_or(now, |named| Value::Named(named.name))
        }
        (Keys::Step(..) | Keys::Zoom, None, _) => now,
        (keys @ (Keys::Step(..) | Keys::Zoom), Some(numbers), _) => {
            let up = keys.up(key).unwrap_or(true);
            // The most here: the room's, where it is under the setting's own.
            let (most, here) = match bounds.and_then(|bounds| bounds.most).filter(|most| *most < numbers.most) {
                Some(most) => (most.max(numbers.least), " here"),
                None => (numbers.most.max(numbers.least), ""),
            };
            let (end, name) = if up { (most, format!("most{here}")) } else { (numbers.least, "least".to_owned()) };
            match (stepped(setting.named, bounds.and_then(|bounds| bounds.named_at), Numbers { most, ..numbers }, now, up), now) {
                (Some(value), _) => value,
                (None, Value::Number(number)) if number > most => return (now, format!("{}, the {name}", says(now))),
                (None, Value::Number(_)) => return at_end(end, &name),
                (None, named) => return (named, format!("{}, the {name}", says(named))),
            }
        }
    };
    (value, says(value))
}

/// The value after `now` among a setting's steps, `up` or down, or `None` past the last: the multiples
/// of the step from the least to the most (the ends among them), and each of `named` set at `at` among
/// them, after the number equal to it, in their order. A number typed between two multiples steps to
/// either. A named value set nowhere steps to the least going up, the most going down.
fn stepped(named: &[Named], at: Option<u32>, numbers: Numbers, now: Value, up: bool) -> Option<Value> {
    let (least, most, step) = (numbers.least, numbers.most.max(numbers.least), numbers.step.max(1));
    // Where `now` sits: at which number, and for a named value, which among those set at it.
    let (number, here) = match now {
        Value::Number(number) => (number, None),
        Value::Named(name) => match (named.iter().position(|n| n.name == name), at) {
            (Some(i), Some(at)) => (at, Some(i)),
            _ => return Some(Value::Number(if up { least } else { most })),
        },
    };
    let set = || named.iter().enumerate().filter_map(|(i, _)| at.map(|at| (at, i)));
    let as_named = |(_, i): (u32, usize)| named.get(i).map(|n| Value::Named(n.name));
    if up {
        let next = (number < most).then(|| (number / step).saturating_add(1).saturating_mul(step).clamp(least, most));
        // Those set at this number after `now`, then those set before the next number, then it.
        let same = set().filter(|(at, i)| *at == number && here.is_none_or(|here| *i > here)).min();
        let between = set().filter(|(at, _)| *at > number && next.is_none_or(|next| *at < next)).min();
        same.or(between).and_then(as_named).or(next.map(Value::Number))
    } else {
        let previous = (number > least).then(|| (number.saturating_sub(1) / step).saturating_mul(step).clamp(least, most));
        // Those set at this number before `now`; the number itself, from a named value set at one of
        // the steps; those set since the previous number, the last first; then it.
        let same = here.and_then(|here| set().filter(|(at, i)| *at == number && *i < here).max());
        // Set past the most, a named value steps down to the most itself, never to a number out of range.
        let on_a_step = (least..=most).contains(&number) && (number == least || number == most || number % step == 0);
        let itself = (here.is_some() && on_a_step).then_some(Value::Number(number));
        let between = set().filter(|(at, _)| *at < number && previous.is_none_or(|previous| *at >= previous)).max();
        same.and_then(as_named).or(itself).or(between.and_then(as_named)).or(previous.map(Value::Number))
    }
}

impl Nav {
    fn new(pages: Vec<usize>, count: usize, zoom: Zoom) -> Self {
        Self {
            pages,
            at: 0,
            count,
            list: false,
            nested: false,
            top: 0,
            zoom,
            // Until the first draw says how many it drew: as many as asked, so that a level made at a
            // count (a relayout's) is laid out and bounded in it from the start.
            per_row: if let Zoom::PerRow(n) = zoom { n.max(1) } else { 1 },
            grid_rows: 1,
            most_per_row: fit::MAX_PER_ROW,
            visible: 1,
            max_scroll: 0,
            scroll: 0,
            scroll_to: None,
            typed: String::new(),
            message: None,
            hint: true,
            help: false,
            rerender: false,
            settings: Rc::default(),
            has: Vec::new(),
            bounds: Vec::new(),
            names: Vec::new(),
            filter: None,
            picked: false,
            laid: None,
            relaying: None,
            search: None,
            reveal: None,
        }
    }

    /// Whether the pages are to be made again: asked for here (`r`), or a setting's value changed,
    /// here or in a level left since. Taken: the next ask says no until either happens again.
    fn wants_rerender(&mut self) -> bool {
        let asked = std::mem::take(&mut self.rerender);
        self.settings.changed.take() || asked
    }

    /// What `pages` are called here: a list's are its tiles.
    fn unit(&self) -> &'static str {
        if self.list { "tile" } else { "page" }
    }

    /// What `?` shows here: the viewer's keys, then the settings' this build declares, whatever is
    /// shown, then how to leave. One `+ -` line: the zoom's, or on a document that has a setting on
    /// those keys ([`Keys::Zoom`]), that setting's in its place.
    fn help_lines(&self) -> Vec<(String, &'static str)> {
        let keys = if self.list { LIST_HELP } else { PAGE_HELP };
        let leave = if self.nested { BACK_HELP } else { QUIT_HELP };
        let zoomed = self.zoom_setting();
        let settings =
            self.settings.declared.iter().filter(|setting| setting.keys != Keys::Zoom).map(|setting| (setting_keys(setting), setting.help));
        let fixed = |lines: &[(&str, &'static str)]| lines.iter().map(|(keys, what)| ((*keys).to_owned(), *what)).collect::<Vec<_>>();
        let reflows = self.reflows();
        let keys = fixed(keys).into_iter().map(|(chars, what)| match zoomed {
            Some(setting) if chars == "+ -" => (setting_keys(setting), setting.help),
            _ if reflows && chars == COLUMNS_HELP.0 => (chars, COLUMNS_HELP.1),
            _ => (chars, what),
        });
        keys.chain(settings).chain(fixed(leave)).chain(fixed(&[HELP_HELP])).collect()
    }

    /// The setting the zoom keys go to here, if one does ([`Keys::Zoom`]): one this build declares,
    /// which what is shown has, on a document shown by itself (not a list, and not pages picked by
    /// `--pages`, as laid out).
    fn zoom_setting(&self) -> Option<&'static Setting> {
        if self.list || self.picked {
            return None;
        }
        self.settings.declared.iter().copied().find(|setting| setting.keys == Keys::Zoom && self.has.contains(&setting.name))
    }

    /// Pages to a row.
    fn row(&self) -> usize {
        usize::from(self.per_row.max(1))
    }

    /// Pages on screen: every row of them.
    fn per_screen(&self) -> usize {
        self.row().saturating_mul(usize::from(self.grid_rows.max(1)))
    }

    /// Where in `pages` the screen starts: at the start of the reader's row of pages (so two side
    /// by side are an even place and the odd one after it, the way a book's spreads pair them).
    /// Near the end, higher up, so the last row of pages is at the bottom of the screen, as `less`
    /// keeps the last line there. Either way the reader's page is on screen.
    fn first(&self) -> usize {
        if self.list {
            return self.list_first();
        }
        let row = self.row();
        let last = self.pages.len().saturating_sub(1);
        let last_screen = last.saturating_sub(last % row).saturating_sub(self.per_screen().saturating_sub(row));
        self.at.saturating_sub(self.at % row).min(last_screen)
    }

    /// In a list, where the screen starts: where it was, unless the tile chosen left it, and then
    /// as little further on or back as brings that tile's row on screen; near the end, higher up,
    /// so the last row is at the bottom.
    fn list_first(&self) -> usize {
        let (row, per) = (self.row(), self.per_screen());
        let row_of = |i: usize| i.saturating_sub(i % row);
        let last = self.pages.len().saturating_sub(1);
        let (at, top) = (row_of(self.at.min(last)), row_of(self.top.min(last)));
        let top = if at < top {
            at
        } else if at >= top.saturating_add(per) {
            at.saturating_add(row).saturating_sub(per)
        } else {
            top
        };
        top.min(row_of(last).saturating_add(row).saturating_sub(per))
    }

    /// The pages on screen, row by row (missing past the end).
    fn on_screen(&self) -> Vec<Option<usize>> {
        let first = self.first();
        (first..first.saturating_add(self.per_screen())).map(|i| self.pages.get(i).copied()).collect()
    }

    /// The pages worth having ready around the screen, nearest first: the next `ahead` screens,
    /// then the `behind` screens before.
    fn around(&self, ahead: usize, behind: usize) -> Vec<usize> {
        let per = self.per_screen();
        let first = self.first();
        let screen = |start: usize| (start..start.saturating_add(per)).filter_map(|i| self.pages.get(i).copied());
        // Both stop at the ends of the list, however many screens are asked for.
        let next = (1..=ahead).map(|k| first.saturating_add(k.saturating_mul(per))).take_while(|start| *start < self.pages.len()).flat_map(screen);
        let before = (1..=behind).map_while(|k| first.checked_sub(k.saturating_mul(per))).flat_map(screen);
        next.chain(before).collect()
    }

    fn key(&mut self, key: Key) -> Step {
        self.message = None;
        self.hint = false;
        if let Some(search) = self.search.as_mut() {
            search.heard();
        }
        // Any key leaves the keys, but Ctrl-C, which still quits.
        if std::mem::take(&mut self.help) && key != Key::Interrupt {
            return Step::Continue;
        }
        // A filter being typed takes every key that types a character.
        if self.filter.as_ref().is_some_and(|filter| filter.typing) {
            return self.filter_key(key);
        }
        // And so does a search.
        if self.search.as_ref().is_some_and(Search::typing) {
            return self.search_key(key);
        }
        let typing = !self.typed.is_empty();
        match key {
            Key::Char('?') => self.help = true,
            Key::Char('q') | Key::Interrupt => return Step::Quit,
            // A filter kept goes before the list does.
            Key::Escape | Key::Backspace if self.filter.is_some() && !typing => self.unfilter(self.chosen()),
            Key::Char('u') if self.filter.is_some() => self.unfilter(self.chosen()),
            // And a search kept before the file does.
            Key::Escape | Key::Backspace if self.search.is_some() && !typing => self.search = None,
            Key::Char('u') if self.search.is_some() => self.search = None,
            Key::Escape if !typing => return Step::Leave,
            Key::Escape => self.typed.clear(),
            // Past MAX_TYPED digits, more are ignored (no other arm takes a digit).
            Key::Char(c @ '0'..='9') if self.typed.len() < MAX_TYPED => self.typed.push(c),
            Key::Backspace if !typing => return Step::Back,
            Key::Backspace => {
                self.typed.pop();
            }
            Key::Char('u') => return Step::Back,
            Key::Enter | Key::Char('g') if typing => self.go_typed(),
            Key::Char('c') => return self.pages.get(self.at).map_or(Step::Continue, |page| Step::ToClipboard(*page)),
            _ if self.list => return self.list_key(key),
            Key::Char(' ') | Key::PageDown => self.forward(self.visible.max(1), self.per_screen()),
            Key::Char('b') | Key::PageUp => self.back(self.visible.max(1), self.per_screen()),
            Key::Char('j') | Key::Down | Key::Enter => self.forward(STEP, self.row()),
            Key::Char('k') | Key::Up => self.back(STEP, self.row()),
            // With a search kept, from match to match, `p` back as `N` is; the arrows still turn pages.
            Key::Char('n') if self.search.is_some() => self.step_match(true),
            Key::Char('N' | 'p') if self.search.is_some() => self.step_match(false),
            Key::Char('N') => self.message = Some(NO_SEARCH.into()),
            Key::Char('n') | Key::Right => self.go(self.first().saturating_add(self.per_screen())),
            Key::Char('p') | Key::Left => self.go(self.first().saturating_sub(self.per_screen())),
            Key::Char('g') | Key::Home => self.go(0),
            Key::Char('G') | Key::End => self.go(usize::MAX),
            // `=` and `_` are the same keys as `+` and `-`, with Shift or without.
            // A document's font size, where it has one; the zoom everywhere else.
            Key::Char('+' | '=' | '-' | '_') if self.zoom_setting().is_some() => return self.setting_key(key),
            Key::Char('+' | '=') => self.zoom_to(self.zoom.zoomed_in(self.per_row), "fully zoomed in"),
            Key::Char('-' | '_') => self.zoom_to(self.zoom.zoomed_out(self.per_row, self.most_per_row), "fully zoomed out"),
            Key::Char(',') => self.per_row_key(true),
            Key::Char('.') => self.per_row_key(false),
            Key::Char('r' | '\u{c}') => self.rerender = true,
            // A number typed before it is dropped. The search kept is typed on; with none, the pager
            // asks the source for its text.
            Key::Char('/') => {
                self.typed.clear();
                match self.search.as_mut() {
                    Some(search) => search.edit(),
                    None => return Step::Search,
                }
            }
            _ => return self.setting_key(key),
        }
        Step::Continue
    }

    /// `/` in a file's pages: a search to type, of `texts`, the source's text; or with none, a word
    /// that there is none to search.
    fn start_search(&mut self, texts: Option<Box<dyn Texts>>) {
        match texts {
            Some(texts) => self.search = Some(Search::new(texts, &self.pages, self.count)),
            None => self.message = Some(NO_TEXT.into()),
        }
    }

    /// What `key` does while a search is typed: a character is part of it, the matches counted as
    /// it is typed, the view where it was; Backspace takes the last one back; Enter keeps the
    /// search, and goes to the first match from the top of the screen; Esc leaves no search, as
    /// Backspace and Enter do with nothing typed. Ctrl-C still quits, and no other key does anything.
    fn search_key(&mut self, key: Key) -> Step {
        let Some(search) = self.search.as_mut() else { return Step::Continue };
        let empty = search.query().is_empty();
        match key {
            Key::Interrupt => return Step::Quit,
            // Past MAX_FILTER characters, more are ignored.
            Key::Char(c) if !c.is_control() && search.query().chars().count() < MAX_FILTER => search.typed(c),
            Key::Backspace if !empty => search.backspace(),
            Key::Enter if !empty => {
                let on_screen: Vec<usize> = self.on_screen().into_iter().flatten().collect();
                let screen =
                    search::Screen { top: on_screen.first().copied().unwrap_or(0), pages: &on_screen, near: &[], visible: self.part_shown() };
                let go = self.search.as_mut().and_then(|search| search.enter(&screen));
                self.go_to_match(go);
            }
            Key::Backspace | Key::Enter | Key::Escape => self.search = None,
            _ => {}
        }
        Step::Continue
    }

    /// `n` (`forward`) or `N` with a search kept: to the next match or the one before, or as many on
    /// as the number typed before it.
    fn step_match(&mut self, forward: bool) {
        // At most `MAX_TYPED` digits: it fits.
        let count = std::mem::take(&mut self.typed).parse::<usize>().unwrap_or(1);
        let on_screen: Vec<usize> = self.on_screen().into_iter().flatten().collect();
        let screen = search::Screen { top: on_screen.first().copied().unwrap_or(0), pages: &on_screen, near: &[], visible: self.part_shown() };
        let go = self.search.as_mut().and_then(|search| search.step(forward, count, &screen));
        self.go_to_match(go);
    }

    /// The view where a match is, if `go` says where: its page brought on screen if it isn't, and,
    /// where the page scrolls, the match scrolled into view once the page's rows are known.
    fn go_to_match(&mut self, go: Option<search::Go>) {
        let Some(go) = go else { return };
        let Some(index) = self.pages.iter().position(|page| *page == go.page) else { return };
        let first = self.first();
        if !(first..first.saturating_add(self.per_screen())).contains(&index) {
            self.go(index);
        }
        // Only a page alone on screen scrolls.
        self.reveal = go.span.filter(|_| self.first() == index);
    }

    /// Take what the search's source has read, and match it, a little at a time: whether the status
    /// line has news, and whether the pages do (the view went to a match, or a page's tint changed).
    /// `size_of` is a page's size, as the source says it.
    fn search_on(&mut self, size_of: &dyn Fn(usize) -> Option<(f32, f32)>) -> (bool, bool) {
        if self.search.is_none() {
            return (false, false);
        }
        let on_screen: Vec<usize> = self.on_screen().into_iter().flatten().collect();
        let near = self.around(AHEAD, BEHIND);
        let screen = search::Screen { top: on_screen.first().copied().unwrap_or(0), pages: &on_screen, near: &near, visible: self.part_shown() };
        let Some(polled) = self.search.as_mut().map(|search| search.poll(&screen, size_of)) else { return (false, false) };
        let moved = polled.go.is_some();
        self.go_to_match(polled.go);
        let said = polled.said.is_some();
        if let Some(said) = polled.said {
            self.message = Some(said);
        }
        (polled.changed || said, moved || polled.tinted)
    }

    /// The part of the first page on screen that shows, where it is taller than the window: from
    /// where it starts to where it ends, as shares of the page's height, by the last draw's rows.
    /// `None` for a page all on screen.
    fn part_shown(&self) -> Option<(f32, f32)> {
        if self.max_scroll == 0 {
            return None;
        }
        let rows = f32::from(self.visible) + f32::from(self.max_scroll);
        let top = f32::from(self.scroll.min(self.max_scroll));
        Some((top / rows, (top + f32::from(self.visible)) / rows))
    }

    /// Page `page`'s tint, where a search kept has matches on it, and whether it is the one the page
    /// holds now ([`Search::tint`]). None without a search.
    fn tint(&self, page: usize) -> (Option<Rc<search::Tint>>, bool) {
        self.search.as_ref().map_or((None, true), |search| search.tint(page))
    }

    /// What `key` does in a list: the arrows move from tile to tile, the screen following, and
    /// Enter opens the tile chosen.
    fn list_key(&mut self, key: Key) -> Step {
        let (row, per) = (self.row(), self.per_screen());
        let last = self.pages.len().saturating_sub(1);
        match key {
            Key::Enter => return self.pages.get(self.at).map_or(Step::Continue, |tile| Step::Open(*tile)),
            Key::Left | Key::Char('h') => self.select(self.at.saturating_sub(1)),
            Key::Right | Key::Char('l') => self.select(self.at.saturating_add(1)),
            Key::Up | Key::Char('k') if self.at >= row => self.select(self.at.saturating_sub(row)),
            // Below, or on the last row, the last tile.
            Key::Down | Key::Char('j') if self.at.saturating_add(row) <= last => self.select(self.at.saturating_add(row)),
            Key::Down | Key::Char('j') if last / row > self.at / row => self.select(last),
            Key::Char(' ' | 'n') | Key::PageDown => {
                self.top = self.first().saturating_add(per);
                self.select(self.at.saturating_add(per));
            }
            Key::Char('b' | 'p') | Key::PageUp => {
                self.top = self.first().saturating_sub(per);
                self.select(self.at.saturating_sub(per));
            }
            Key::Char('g') | Key::Home => self.select(0),
            Key::Char('G') | Key::End => self.select(last),
            Key::Char('+' | '=') => self.step_per_row(false),
            Key::Char('-' | '_') => self.step_per_row(true),
            Key::Char(',') => self.per_row_key(true),
            Key::Char('.') => self.per_row_key(false),
            Key::Char('r' | '\u{c}') => self.rerender = true,
            Key::Char('/') => self.start_filter(),
            _ => return self.setting_key(key),
        }
        Step::Continue
    }

    /// A key none of the viewer's took: a setting's, if this build declares one with it. A number
    /// typed before it is used up by a setting with numbers, whatever happens, and left as it is by
    /// one without. On what hasn't the setting, its message is said and nothing changes; a relayout
    /// setting's in a list, or with `--pages`, says why nothing changes either. Otherwise its value
    /// changes and the reader is told what it is now: a redraw setting's pages are made again, and a
    /// document whose layout isn't at a relayout setting's value is laid out again when it is on screen
    /// ([`Pager::relayout`]).
    ///
    /// On a document laid out in columns, a key of a setting that says how wide a page is
    /// ([`Setting::sets_width`]) sets the pages to a row back to `auto` first, and is pressed again once
    /// the bounds are those of the room at `auto` ([`Step::Again`]); `0` before it changes only its own.
    fn setting_key(&mut self, key: Key) -> Step {
        let Key::Char(char) = key else { return Step::Continue };
        let settings = self.settings.clone();
        let Some(setting) = settings.declared.iter().find(|setting| setting.keys.chars().contains(&char)) else { return Step::Continue };
        let zero = self.typed.parse::<u32>().ok() == Some(0);
        if setting.sets_width && self.has.contains(&setting.name) && self.columns().is_some() && !zero {
            self.zoom_to(Some(Zoom::Auto), "");
            return Step::Again(key);
        }
        let key = char;
        let count = setting.numbers.and_then(|_| (!self.typed.is_empty()).then(|| std::mem::take(&mut self.typed)));
        if !self.has.contains(&setting.name) {
            self.message = Some(setting.missing.into());
            return Step::Continue;
        }
        if setting.effect == Effect::Relayout && (self.list || self.picked) {
            self.message = Some(if self.list { REFLOW_IN_LIST } else { REFLOW_WITH_PAGES }.into());
            return Step::Continue;
        }
        let mut values = settings.values.get();
        let now = values.get(setting.name).unwrap_or(setting.default);
        let (value, said) = turned(setting, bounds_of(&self.bounds, setting.name), now, key, count.as_deref());
        if value != now && values.set(setting.name, value) {
            settings.values.set(values);
            if setting.effect == Effect::Redraw {
                settings.changed.set(true);
            }
        }
        self.message = Some(said);
        Step::Continue
    }

    /// In a list, choose the tile at `index`, or the last: the screen follows it.
    fn select(&mut self, index: usize) {
        self.at = index.min(self.pages.len().saturating_sub(1));
        self.top = self.first();
    }

    /// Down `rows` image rows; from the bottom of the screen's pages, on `pages` pages.
    fn forward(&mut self, rows: u16, pages: usize) {
        let next = self.first().saturating_add(pages);
        if self.scroll < self.max_scroll {
            self.scroll = self.scroll.saturating_add(rows).min(self.max_scroll);
        } else if next < self.pages.len() {
            self.at = next;
            self.scroll = 0;
        }
    }

    /// Up `rows` image rows; from the top of the screen's pages, back `pages` pages, to the
    /// bottom of the first.
    fn back(&mut self, rows: u16, pages: usize) {
        let first = self.first();
        if self.scroll > 0 {
            self.scroll = self.scroll.saturating_sub(rows);
        } else if first > 0 {
            self.at = first.saturating_sub(pages);
            self.scroll = u16::MAX;
        }
    }

    /// `,` (`more`) or `.`: one page more to a row, or one fewer, as the zoom does (`-` and `+`): from
    /// one page, the width; in a list, one tile to a row at the least, never the width. A number typed
    /// before it sets that many, as far as the window takes, and 0 goes back to `auto`.
    fn per_row_key(&mut self, more: bool) {
        if !self.typed.is_empty() {
            let typed = std::mem::take(&mut self.typed);
            // At most `MAX_TYPED` digits: past u32, it is past the most anyway.
            let wanted = typed.parse::<u32>().unwrap_or(u32::MAX);
            let most = self.most_per_row.max(1);
            let n = u16::try_from(wanted).unwrap_or(u16::MAX).min(most);
            if n == 0 {
                // `0.` changes only the count.
                self.zoom_to(Some(Zoom::Auto), "");
                return;
            }
            self.zoom_to(Some(Zoom::PerRow(n)), "");
            if wanted > u32::from(most) {
                self.message = Some("fully zoomed out".into());
            }
            self.width_gives_way();
            return;
        }
        if !more && self.reflows() && self.per_row <= 1 {
            // A reflowed page is never wider than the window: one to a row is as far in as it goes.
            self.message = Some("1 per row, the least".into());
            return;
        }
        let zoom = self.zoom;
        self.step_per_row(more);
        if self.zoom != zoom {
            self.width_gives_way();
        }
    }

    /// Whether what is shown is a document laid out for its room, by itself: one that has a relayout
    /// setting, not in a list, and not pages picked by `--pages` (as laid out, never laid out again).
    fn reflows(&self) -> bool {
        !self.list
            && !self.picked
            && self.settings.declared.iter().any(|setting| setting.effect == Effect::Relayout && self.has.contains(&setting.name))
    }

    /// How many pages to a row a document laid out for its room is asked to be laid out in: the count
    /// asked; `None` at `auto`, or for anything else.
    fn columns(&self) -> Option<u16> {
        match self.zoom {
            _ if !self.reflows() => None,
            Zoom::Auto => None,
            Zoom::Width => Some(1),
            Zoom::PerRow(n) => Some(n.max(1)),
        }
    }

    /// [`Nav::columns`] in `room`, the room the document is laid out for ([`Room::per_row`]): as many as it
    /// takes ([`fit::max_per_row`]), the count the grid draws once the window holds that room. Not the
    /// last draw's: a window that changes size changes the count laid out in as it changes the room, once
    /// it has held ([`Pager::watch_room`]), not at once and in the old room.
    fn columns_in(&self, room: &Room) -> Option<u16> {
        self.columns().map(|n| n.clamp(1, fit::max_per_row(room.cols)))
    }

    /// A count to a row was set, on a document laid out in columns: the settings that say how wide a
    /// page is ([`Setting::sets_width`]) go back to their default, as the last key wins.
    fn width_gives_way(&mut self) {
        if !self.reflows() {
            return;
        }
        let mut values = self.settings.values.get();
        let widths = self.settings.declared.iter().filter(|setting| setting.sets_width && self.has.contains(&setting.name));
        let mut changed = false;
        for setting in widths {
            if values.get(setting.name).is_some_and(|value| value != setting.default) {
                changed |= values.set(setting.name, setting.default);
            }
        }
        if changed {
            self.settings.values.set(values);
        }
    }

    /// The count to a row back to `per_row`, what the document on screen is laid out in, after a layout
    /// that failed: the next step starts from what is on screen.
    fn put_back_columns(&mut self, per_row: Option<u16>) {
        let zoom = per_row.map_or(Zoom::Auto, Zoom::PerRow);
        if self.reflows() && self.columns() != per_row {
            self.zoom_to(Some(zoom), "");
        }
    }

    /// One page (or tile) more to a row (`more`), or one fewer: `-` and `+`, and `,` and `.`.
    fn step_per_row(&mut self, more: bool) {
        if more {
            self.zoom_to(self.zoom.zoomed_out(self.per_row, self.most_per_row), "fully zoomed out");
        } else if self.list {
            // As wide as the window is one to a row: a tile never scrolls.
            let zoom = (self.per_row > 1).then(|| Zoom::PerRow(self.per_row.saturating_sub(1)));
            self.zoom_to(zoom, "fully zoomed in");
        } else {
            self.zoom_to(self.zoom.zoomed_in(self.per_row), "fully zoomed in");
        }
    }

    /// Zoom to `zoom`, or, when there is no further to go, say so with `limit`. Keys before the
    /// next draw step by the new row of pages; the draw finds how many rows fit.
    fn zoom_to(&mut self, zoom: Option<Zoom>, limit: &str) {
        let Some(zoom) = zoom else {
            self.message = Some(limit.into());
            return;
        };
        self.zoom = zoom;
        self.scroll = 0;
        self.per_row = if let Zoom::PerRow(n) = zoom { n.max(1) } else { 1 };
        self.grid_rows = 1;
    }

    /// To the top of the page at `index` in `pages`, or of the last one when past it.
    fn go(&mut self, index: usize) {
        self.at = index.min(self.pages.len().saturating_sub(1));
        self.scroll = 0;
    }

    /// To the page whose number was typed, if it is one being paged through; in a list, to that
    /// tile, if it matches the filter.
    fn go_typed(&mut self) {
        let wanted = self.typed.parse::<usize>().ok().and_then(|n| n.checked_sub(1));
        let unit = self.unit();
        match wanted.map(|page| (page, self.pages.iter().position(|p| *p == page))) {
            Some((_, Some(index))) if self.list => self.select(index),
            Some((_, Some(index))) => self.go(index),
            Some((page, None)) if self.unmatched(page).is_some() => {
                self.message = self.unmatched(page).map(|filter| format!("{unit} {} doesn't match {filter}", self.typed));
            }
            Some((page, None)) if page < self.count => self.message = Some(format!("{unit} {} isn't one of the {unit}s given", self.typed)),
            _ => self.message = Some(format!("no {unit} {} (1–{})", self.typed, self.count)),
        }
        self.typed.clear();
    }

    /// How many tiles there are to filter, matching or not: the list's, or those given. The
    /// tiles keep the size these give them while filtered.
    fn listed(&self) -> usize {
        self.filter.as_ref().map_or(self.pages.len(), |filter| filter.given.len())
    }

    /// The filter, as the status line shows it, if `tile` is one of those given that it leaves out.
    fn unmatched(&self, tile: usize) -> Option<String> {
        let filter = self.filter.as_ref().filter(|filter| filter.given.contains(&tile) && !self.pages.contains(&tile))?;
        Some(format!("/{}", filter.text))
    }

    /// The tile chosen; while none matches the filter, the one chosen last.
    fn chosen(&self) -> Option<usize> {
        self.pages.get(self.at).copied().or_else(|| self.filter.as_ref().and_then(|filter| filter.last))
    }

    /// `/` in a list: a filter to type, from the one kept if there is one.
    fn start_filter(&mut self) {
        self.typed.clear();
        let chosen = self.chosen();
        match self.filter.as_mut() {
            Some(filter) => {
                filter.before = Some(filter.text.clone());
                filter.chosen_before = chosen;
                filter.typing = true;
            }
            None => {
                let given = self.pages.clone();
                self.filter = Some(Filter { text: String::new(), typing: true, given, last: chosen, before: None, chosen_before: chosen });
            }
        }
    }

    /// What `key` does while a filter is typed: a character is part of it, as the tiles narrow to
    /// those that match; Backspace takes the last one back; Enter keeps it; Esc puts back what was
    /// there before `/`. With nothing typed, Backspace and Enter leave no filter. The keys that move
    /// still move, among the tiles that match, and Ctrl-C still quits.
    fn filter_key(&mut self, key: Key) -> Step {
        let Some(filter) = self.filter.as_mut() else { return Step::Continue };
        let empty = filter.text.is_empty();
        match key {
            Key::Interrupt => return Step::Quit,
            // Past MAX_FILTER characters, more are ignored.
            Key::Char(c) if !c.is_control() && filter.text.chars().count() < MAX_FILTER => filter.text.push(c),
            Key::Char(c) if !c.is_control() => {}
            Key::Backspace if !empty => {
                filter.text.pop();
            }
            Key::Enter if !empty => filter.typing = false,
            Key::Backspace | Key::Enter => self.unfilter(self.chosen()),
            Key::Escape => self.cancel_filter(),
            _ => return self.list_key(key),
        }
        self.refilter(self.chosen());
        Step::Continue
    }

    /// Esc while a filter is typed: the list as it was before `/`, with the filter kept then, if
    /// one was, and the tile chosen then.
    fn cancel_filter(&mut self) {
        let Some(filter) = self.filter.as_mut() else { return };
        let chosen = filter.chosen_before;
        match filter.before.take() {
            Some(text) => {
                filter.text = text;
                filter.typing = false;
                self.refilter(chosen);
            }
            None => self.unfilter(chosen),
        }
    }

    /// No filter: all the tiles given again, `chosen` chosen.
    fn unfilter(&mut self, chosen: Option<usize>) {
        if let Some(filter) = self.filter.as_mut() {
            filter.text.clear();
        }
        self.refilter(chosen);
        self.filter = None;
    }

    /// The tiles given that match the filter, in their order, in place of those before (all of
    /// them, with nothing typed): `chosen` stays chosen if it is one of them; otherwise the first
    /// after it among the tiles given, or the last before it. Its row stays where it was on
    /// screen, as far as the list allows.
    fn refilter(&mut self, chosen: Option<usize>) {
        let row = self.row();
        let rows_down = (self.at / row).saturating_sub(self.first() / row);
        let Some(filter) = self.filter.as_mut() else { return };
        let needle = fold(&filter.text);
        let from = chosen.and_then(|tile| filter.given.iter().position(|t| *t == tile)).unwrap_or(0);
        let (mut pages, mut at) = (Vec::new(), None);
        for (i, &tile) in filter.given.iter().enumerate() {
            if self.names.get(tile).map_or("", String::as_str).contains(&needle) {
                if at.is_none() && i >= from {
                    at = Some(pages.len());
                }
                pages.push(tile);
            }
        }
        filter.last = chosen;
        self.pages = pages;
        self.at = at.unwrap_or(usize::MAX).min(self.pages.len().saturating_sub(1));
        self.top = (self.at / row).saturating_sub(rows_down).saturating_mul(row);
        self.top = self.first();
    }

    /// For a draw: `per_row` pages to a row in `grid_rows` rows, of at most `most_per_row` to a
    /// row in this window.
    fn grid(&mut self, per_row: u16, grid_rows: u16, most_per_row: u16) {
        self.per_row = per_row.max(1);
        self.grid_rows = grid_rows.max(1);
        self.most_per_row = most_per_row.max(1);
        self.top = self.first();
    }

    /// After a draw: `visible` of the first page's `rows` image rows are on screen. Keeps `scroll`
    /// in range, after taking it where `scroll_to` asked, now that the rows are known.
    fn shown(&mut self, visible: u16, rows: u16) {
        self.visible = visible;
        self.max_scroll = rows.saturating_sub(visible);
        if let Some(share) = self.scroll_to.take() {
            // A share of the rows, so within them.
            self.scroll = (share.clamp(0.0, 1.0) * f32::from(rows)).round() as u16;
        }
        if let Some((top, bottom)) = self.reveal.take() {
            // Shares of the rows, so within them. A match off screen comes a third of the way down it,
            // as far as the page scrolls; one on screen leaves the view where it is.
            // Its first row, and the row after its last (a hundredth of a row is the arithmetic's).
            let rows_down = |share: f32| share.clamp(0.0, 1.0) * f32::from(rows);
            let (top, bottom) = (rows_down(top) as u16, (rows_down(bottom) - 0.01).ceil().max(0.0) as u16);
            if top < self.scroll || bottom > self.scroll.saturating_add(visible) {
                self.scroll = top.saturating_sub(visible / 3);
            }
        }
        self.scroll = self.scroll.min(self.max_scroll);
    }

    /// The status line for a window `cols` wide: exactly that many characters, none of them a
    /// control character (the name and the messages may come from anywhere). The right side is
    /// at the same place whatever the reader does: where the reader is, padded to the widest it
    /// gets in this document, what the document is laid out at when that isn't the default
    /// (`width 72`, with `…` after it while it is laid out again), the zoom, and the key for the
    /// keys; `…` before them while pages render. A narrow window drops the layout, then the zoom,
    /// then the key. On the left, the name, cut short in its middle to leave room for a message,
    /// which ends two spaces before the right side. Until the first key, the message is the
    /// reminder of the keys, as much of it as fits. A list's filter takes the message's place:
    /// always while it is typed, otherwise when there is no message. A file's search does while it is
    /// typed (`/word 17…`), and kept, comes before the message (`/word 3/17`).
    fn status(&self, name: &str, cols: u16, rendering: bool) -> String {
        let width = usize::from(cols);
        // A number being typed is all the line says: the number, and the keys that take it here.
        if !self.typed.is_empty() {
            return self.typing(width);
        }
        let (place, widest) = self.place();
        let zoom = if self.zoom == Zoom::Width { "width".to_owned() } else { format!("{} per row", self.per_row) };
        let slot = if rendering { '…' } else { ' ' };
        let laid = self.relaying.as_ref().map(|wanted| format!("{wanted}…")).or_else(|| self.laid.clone());
        let laid = laid.map(|laid| format!("{laid:<width$} · ", width = self.laid_width()));
        let mut rights = Vec::new();
        if let Some(laid) = laid {
            rights.push(format!("{slot} {place:<widest$} · {laid}{zoom:<ZOOM_WIDTH$} · {HELP} "));
        }
        rights.push(format!("{slot} {place:<widest$} · {zoom:<ZOOM_WIDTH$} · {HELP} "));
        rights.push(format!("{slot} {place:<widest$} · {HELP} "));
        rights.push(format!("{slot} {place} "));
        let shortest = rights.last().cloned().unwrap_or_default();
        let right = rights.iter().find(|r| chars(r) <= width).cloned().unwrap_or_else(|| shortest.chars().take(width).collect());
        let room = width.saturating_sub(chars(&right));
        let said = match &self.message {
            Some(message) => message.clone(),
            // The keys leave the name room for its first characters at least.
            None if self.hint => {
                let fits = |keys: &&&str| chars(keys).saturating_add(chars(name).min(16)).saturating_add(5) <= room;
                let keys = match (self.list, self.nested) {
                    (true, _) => &LIST_KEYS,
                    (false, true) => &OPENED_KEYS,
                    (false, false) => &KEYS,
                };
                let hint = keys.iter().find(fits).map_or_else(String::new, |keys| (*keys).to_owned());
                // The zoom keys are a setting's here: the hint says which.
                match self.zoom_setting() {
                    Some(setting) => hint.replace("+ - zoom", &format!("+ - {}", setting.name)),
                    None => hint,
                }
            }
            None => String::new(),
        };
        // A filter being typed is what the line says, with how to keep it or not when there is
        // room; one kept, when there is nothing else to say.
        let said = match &self.filter {
            Some(filter) if filter.typing => {
                let (typed, keys) = (format!("/{}", filter.text), format!("/{}  {FILTER_KEYS}", filter.text));
                let fits = chars(&keys).saturating_add(chars(name).min(16)).saturating_add(5) <= room;
                if fits { keys } else { typed }
            }
            Some(filter) if said.is_empty() => format!("/{}", filter.text),
            _ => said,
        };
        // A search likewise while it is typed, but for why some of its text was left unread, which
        // comes as it is typed; one kept is always said, with its matches, before anything else
        // there is to say.
        let said = match self.search.as_ref().map(|search| (search.typing(), search.status())) {
            Some((true, typed)) if !said.is_empty() => format!("{typed}  {said}"),
            Some((true, typed)) => {
                let keys = format!("{typed}  {SEARCH_KEYS}");
                let fits = chars(&keys).saturating_add(chars(name).min(16)).saturating_add(5) <= room;
                if fits { keys } else { typed }
            }
            Some((false, kept)) if said.is_empty() => kept,
            Some((false, kept)) => format!("{kept}  {said}"),
            None => said,
        };
        // A space at the start; the message, then two spaces; and at least two spaces between the
        // name and the message, or one after the name alone.
        let said = cut(&said, room.saturating_sub(3));
        let tail = if said.is_empty() { String::new() } else { format!("{said}  ") };
        let name = cut(name, room.saturating_sub(chars(&tail)).saturating_sub(if said.is_empty() { 2 } else { 3 }));
        let gap = room.saturating_sub(1usize.saturating_add(chars(&name)).saturating_add(chars(&tail)));
        let left: String = format!(" {name}{}{tail}", " ".repeat(gap)).chars().chain(std::iter::repeat(' ')).take(room).collect();
        printable(&format!("{left}{right}"))
    }

    /// The status line while a number is typed, exactly `width` characters, none a control character:
    /// the number and the keys that take it here ([`Nav::takes_a_number`]), as many of them as fit, the
    /// first first; or as much of the number as fits.
    fn typing(&self, width: usize) -> String {
        let room = width.saturating_sub(1);
        let keys = self.takes_a_number();
        let mut fitted = None;
        for n in (1..=keys.len()).rev() {
            let line = format!("{}: {}", self.typed, keys.get(..n).unwrap_or_default().join(" · "));
            if chars(&line) <= room {
                fitted = Some(line);
                break;
            }
        }
        let line = fitted.unwrap_or_else(|| cut(&format!("{}: ", self.typed), room));
        printable(&format!(" {line}").chars().chain(std::iter::repeat(' ')).take(width).collect::<String>())
    }

    /// The keys a number typed here goes to, as the status line names them: to a page (`g`), or in a
    /// list a tile (`Enter`); to the pages to a row (`.`); with a search kept, that many matches on
    /// or back (`n`, `p`); and to each setting with numbers what is
    /// shown has and lets change here (not one that lays out again, in a list or with `--pages`), by its
    /// up key and its name, the one on the zoom keys first.
    fn takes_a_number(&self) -> Vec<String> {
        let first = if self.list { "Enter tile" } else { "g page" };
        let mut keys = vec![first.to_owned(), ". per row".to_owned()];
        if self.search.is_some() {
            keys.push("n p match".to_owned());
        }
        let here = |setting: &&&Setting| {
            setting.numbers.is_some() && self.has.contains(&setting.name) && !(setting.effect == Effect::Relayout && (self.list || self.picked))
        };
        let zoomed = self.settings.declared.iter().filter(here).filter(|setting| setting.keys == Keys::Zoom);
        let rest = self.settings.declared.iter().filter(here).filter(|setting| setting.keys != Keys::Zoom);
        for setting in zoomed.chain(rest) {
            let up = match setting.keys {
                Keys::Zoom => Some('+'),
                Keys::Step(_, up) => Some(up),
                Keys::Cycle(_) => None,
            };
            if let Some(up) = up {
                keys.push(format!("{up} {}", setting.name));
            }
        }
        keys
    }

    /// The widest a relayout setting's value is said on the status line, `…` included (`width 200…`), so
    /// the right side stays put as it steps.
    fn laid_width(&self) -> usize {
        let digits = |n: u32| n.to_string().len();
        let widest = |setting: &&Setting| {
            let numbers = setting.numbers.map_or(0, |numbers| digits(numbers.most).max(digits(numbers.least)).saturating_add(chars(numbers.unit)));
            let named = setting.named.iter().map(|named| chars(named.says)).max().unwrap_or(0);
            // A number past the most here says what it is laid out at too (`said_in`).
            let past = setting.named.first().map_or(numbers, |named| chars(named.name)).saturating_add(3);
            named.max(chars(setting.name).saturating_add(1).saturating_add(numbers).saturating_add(past)).saturating_add(1)
        };
        self.settings.declared.iter().filter(|setting| setting.effect == Effect::Relayout).map(widest).max().unwrap_or(0)
    }

    /// Where the reader is, for the status line, and the most characters it takes in this
    /// document: the pages on screen, in a list the tile chosen, of how many; and where among the
    /// pages given when they are not all, or among the tiles that match a filter (`no match`).
    fn place(&self) -> (String, usize) {
        let digits = |n: usize| n.to_string().len();
        let count = self.count;
        let (mut place, mut widest, at) = if self.list {
            let tile = self.pages.get(self.at).map_or(0, |tile| tile.saturating_add(1));
            (format!("{tile}/{count}"), digits(count).saturating_mul(2).saturating_add(1), self.at)
        } else {
            let on_screen: Vec<usize> = self.on_screen().into_iter().flatten().collect();
            let place = match on_screen.as_slice() {
                [a, .., b] => format!("pages {}–{}/{count}", a.saturating_add(1), b.saturating_add(1)),
                [a] => format!("page {}/{count}", a.saturating_add(1)),
                [] => "no page".into(),
            };
            // `pages a–b/count`, both numbers as long as the count.
            (place, digits(count).saturating_mul(3).saturating_add(8), self.first())
        };
        let given = self.pages.len();
        if given != count || self.filter.is_some() {
            // Filtered, among the tiles that match, as wide as when they all do: the right side
            // stays put while a filter is typed.
            widest = widest.saturating_add(digits(self.listed()).saturating_mul(2).saturating_add(7));
            match given {
                0 if self.filter.is_some() => place = "no match".into(),
                _ => place.push_str(&format!(" · {} of {given}", at.saturating_add(1))),
            }
        }
        (place, widest)
    }
}

/// The alternate screen, raw mode, no cursor and no line wrap, all undone when this drops (on
/// return, on an error through `?` and while a panic unwinds).
struct Screen<'a> {
    tty: &'a Tty,
    _raw: RawMode<'a>,
}

impl<'a> Screen<'a> {
    fn enter(tty: &'a Tty) -> Result<Self, String> {
        // The guard comes first, so a write that fails halfway is undone too.
        let screen = Self { tty, _raw: tty.raw()? };
        QUIET.store(true, Ordering::Relaxed);
        tty.write_all(b"\x1b[?1049h\x1b[?25l\x1b[?7l")?;
        Ok(screen)
    }
}

impl Drop for Screen<'_> {
    fn drop(&mut self) {
        // End synchronized output and reset the colours too, in case a redraw stopped halfway.
        let _ = self.tty.write_all(b"\x1b[?2026l\x1b[0m\x1b[?7h\x1b[?25h\x1b[?1049l");
        QUIET.store(false, Ordering::Relaxed);
    }
}

/// A page as drawn: of which level's source, which page, fitted how, into how many columns and
/// rows of what window.
#[derive(Clone, Copy, Debug, PartialEq)]
struct View {
    level: u64,
    page: usize,
    fit: Fit,
    cols: u16,
    rows: u16,
    size: WindowSize,
}

/// A page rendered for the window and sent to the terminal under an image id of its own; or drawn
/// as text, which stays here until it goes on screen.
struct Sent {
    view: View,
    id: u32,
    /// The cells it covers and its size in pixels.
    cols: u16,
    rows: u16,
    width: u32,
    height: u32,
    /// What the reader should know about it while it is on screen (content skipped).
    warning: Option<String>,
    /// Its text, a line for each row of cells, when pages are drawn as text.
    text: Vec<Vec<u8>>,
    /// The tint it was sent with, a search's matches ([`search::Tint::key`]): 0 for none. A page
    /// whose tint is another by now is rendered and sent again, and this one deleted once that one
    /// is placed.
    tint: u64,
}

impl Sent {
    /// What it takes up, in the terminal: its pixels, four bytes each; or here: its text.
    fn bytes(&self) -> u64 {
        match self.text.is_empty() {
            true => u64::from(self.width).saturating_mul(u64::from(self.height)).saturating_mul(4),
            false => text_bytes(&self.text),
        }
    }
}

/// The bytes of `text`.
fn text_bytes(text: &[Vec<u8>]) -> u64 {
    text.iter().map(|line| u64::try_from(line.len()).unwrap_or(u64::MAX)).fold(0, u64::saturating_add)
}

/// Take image `id` out of the terminal. Text leaves nothing there to take.
fn delete(out: &mut Vec<u8>, id: u32, output: Output) {
    if !matches!(output, Output::Text(_)) {
        kitty::delete(out, id, output.wrap());
    }
}

/// A render asked for and not back yet.
struct Wanted {
    view: View,
    layout: Layout,
    tag: u64,
}

/// What the pager shows at a time, and where the reader is in it: a file's pages, or a list's
/// tiles. Each has its own source and render threads.
struct Level {
    /// Tells this level's pages apart from another's in the terminal.
    id: u64,
    source: Rc<dyn Source>,
    pool: Box<dyn Pool>,
    nav: Nav,
    /// A list's items, which its tiles open to; none for a file's pages.
    items: Vec<Item>,
    /// A file's name; for a list, what its items' names follow: the directories gone into from
    /// the first list.
    name: String,
    /// The container it lists, gone into from a list: told when the reader leaves it.
    from: Option<Rc<dyn Container>>,
}

impl Level {
    /// What is `shown`, at `zoom`, in `room` (the window's, at `auto` to a row); `nested` when opened
    /// from a list. A list gone into from a list needs no reminder of the keys: they are the list's. A
    /// document laid out for its room is never wider than it: the width is one to a row there.
    fn new(id: u64, shown: Shown, zoom: Zoom, nested: bool, settings: Rc<Settings>, room: Room) -> Self {
        let list = shown.is_list();
        let Shown { pages: Pages { source, name, pages, picked }, items } = shown;
        let names = items.iter().map(|item| fold(item.name())).collect();
        let has = source.settings();
        let mut nav = Nav { list, nested, hint: !(list && nested), names, settings, has, picked, ..Nav::new(pages, source.pages(), zoom) };
        if nav.reflows() && nav.zoom == Zoom::Width {
            nav.zoom_to(Some(Zoom::PerRow(1)), "");
        }
        let room = Room { per_row: nav.columns_in(&room), ..room };
        nav.bounds = bounds_in(&nav.settings, &*source, &nav.has, room, nav.settings.values.get());
        nav.laid = (!list).then(|| laid_at(&nav.settings, &*source, &nav.has, &nav.bounds)).flatten();
        Self { id, pool: source.pool(THREADS), source, nav, items, name, from: None }
    }

    /// What the status line calls what the reader is on: the file, or the tile chosen.
    fn name(&self) -> String {
        match self.nav.pages.get(self.nav.at).and_then(|tile| self.items.get(*tile)) {
            Some(item) if self.nav.list => format!("{}{}", self.name, item.name()),
            _ => self.name.clone(),
        }
    }
}

struct Pager<'a> {
    /// Dropped after the pager's own `drop` has deleted its images.
    screen: Screen<'a>,
    tty: &'a Tty,
    /// What is on screen, and what it was opened from, the first list first.
    level: Level,
    parents: Vec<Level>,
    /// The room a document on screen is laid out for: the window's pages area, as the files were
    /// opened for it, then as the window has held since it changed size ([`Pager::watch_room`]); at
    /// `auto` to a row, the reader's count added where it is used ([`Pager::room_for`]).
    room: Room,
    /// A container gone into, being read or its files opened before it shows.
    reading: Option<Reading>,
    /// A file of a list chosen, waited for until it is opened.
    going_into: Option<GoingInto>,
    /// Levels made so far, for their ids.
    levels: u64,
    /// What the reader should know once the screen is back.
    later: Vec<String>,
    output: Output,
    /// The cell size tmux or the terminal reports, for a tty that reports no pixel size.
    cell: Option<Cell>,
    effort: PngEffort,
    /// Pages in the terminal, ready to place, the most recently shown last.
    sent: Vec<Sent>,
    /// The most pages kept in `sent`: [`MAX_SENT`], or the screen and the screens around it.
    kept: usize,
    /// Renders asked for, the most urgent first: the pages on screen, then the ones around them.
    wanted: Vec<Wanted>,
    /// Views that failed and why: not asked for again until the window changes or `r`.
    failed: Vec<(View, String)>,
    /// The views the reader wants on screen, row by row, from the last draw.
    on_screen: Vec<Option<View>>,
    /// The images placed on screen by the last full redraw, and under which placement.
    placed: Vec<(u32, u32)>,
    /// The window the pages in `sent` were made for.
    size: Option<WindowSize>,
    first_id: u64,
    next_id: u64,
    tag: u64,
    /// The settings this build declares and their values, which every level shares; and the colour
    /// of dark paper.
    settings: Rc<Settings>,
    dark: [u8; 3],
    /// Inside tmux: which terminals show the pane and what they missed, and when to ask again.
    watch: Option<tmux::Watch>,
    tmux_check: Option<Instant>,
    /// Inside tmux: no terminal shows the pane, so nothing is sent (tmux would throw it away).
    hidden: bool,
    /// Images sent so far.
    transmitted: u64,
    /// Times the pages were sent again because tmux or the terminal threw some away.
    resends: u32,
    /// The most bytes of pages kept in the terminal: [`BUDGET`], less once it drops images.
    budget: u64,
    /// Temporary files, while the terminal may take pages through them.
    files: Option<Files>,
    /// Temporary files whose thread stopped answering, kept to the end: dropping them waits for
    /// the thread to remove its files, which is only worth it once the terminal is put back
    /// (`screen` drops first).
    stalled_files: Option<Files>,
    /// When the last placements went out: their errors are answered.
    placed_at: Option<Instant>,
    /// Pages drawn as text: what each row of the screen holds, so that a redraw writes only the
    /// rows that change. Empty once something else may have written to them.
    on_glass: Vec<Vec<u8>>,
    /// Copies to the clipboard (`c`), and what the terminal said of its clipboard.
    copier: clip::Copier,
    /// The threads a list's files open on, which lay a document out again too.
    openers: Rc<Openers>,
    /// The document on screen being laid out again, if it is ([`Pager::relayout`]).
    relayout: Relayout<Handle>,
    /// The room the window has had since it changed size, until it has held long enough to lay out for.
    hold: Hold,
    /// What the level's bounds were asked for: the level, the values and the room ([`Pager::refresh_bounds`]).
    bounds_for: Option<(u64, Values, Room)>,
    /// Whether the last draw had every page on screen ready: what the status line says of it.
    ready: bool,
}

/// A container gone into from the list, being read or its files opened ([`Pager::read_on`]): its
/// name from the first list, and since when.
struct Reading {
    name: String,
    container: Rc<dyn Container>,
    opening: Option<Opening>,
    since: Instant,
}

/// A file of a list the reader chose to go into, waited for until it is opened.
struct GoingInto {
    /// From the first list.
    name: String,
    later: Rc<Later>,
}

/// A page ready to send.
enum Payload {
    Png(Vec<u8>),
    /// Straight RGBA pixels in a temporary file.
    File(PathBuf),
    /// A line of text for each row of cells, kept here until it goes on screen.
    Text(Vec<Vec<u8>>),
}

impl Drop for Pager<'_> {
    fn drop(&mut self) {
        let mut out = Vec::new();
        for sent in &self.sent {
            delete(&mut out, sent.id, self.output);
        }
        let _ = self.screen.tty.write_all(&out);
    }
}

impl Pager<'_> {
    /// Redraw: decide what is on screen and what to work on ahead, then place the pages on
    /// screen once they are all in the terminal (until then the pages before stay up), and write
    /// the status line.
    fn draw(&mut self) -> Result<(), String> {
        let size = self.tty.size().unwrap_or(FALLBACK_SIZE);
        let cell = Cell::from_window(size.cols, size.rows, size.xpixel, size.ypixel).or(self.cell).unwrap_or(FALLBACK_CELL);
        let cell = self.output.fitting(cell);
        // The last row is the status line.
        let area_rows = size.rows.saturating_sub(1).max(1);
        let (max_cols, max_rows) = self.output.limits(size.cols);
        let mut out = Vec::new();
        // Pages made for another window or another value of a setting, or asked to be made again, go.
        if self.level.nav.wants_rerender() || self.size != Some(size) {
            for sent in self.sent.drain(..) {
                delete(&mut out, sent.id, self.output);
            }
            self.wanted.clear();
            self.failed.clear();
            self.placed.clear();
            self.on_glass.clear();
            self.size = Some(size);
        }
        let grid = self.grid(Area { cols: max_cols, rows: area_rows, cell, max_rows });
        let (level, source) = (self.level.id, self.level.source.clone());
        let view = |page| View { level, page, fit: grid.fit, cols: grid.cols, rows: grid.page_rows, size };
        let on_screen: Vec<Option<View>> = self.level.nav.on_screen().into_iter().map(|page| page.map(view)).collect();
        self.on_screen.clone_from(&on_screen);
        self.kept = MAX_SENT.max(on_screen.len().saturating_mul(1 + AHEAD + BEHIND));

        // What to have ready, most urgent first: the pages on screen, then the next screens, then
        // the one before, as far as they stay in the terminal together (`budget`). Renders already
        // under way for any of them carry on; the rest stop.
        let mut wanted = Vec::new();
        let mut near = 0u64;
        for v in on_screen.iter().flatten().copied().chain(self.level.nav.around(AHEAD, BEHIND).into_iter().map(view)) {
            if self.failed.iter().any(|(f, _)| *f == v) || wanted.iter().any(|w: &Wanted| w.view == v) {
                continue;
            }
            // In the terminal already, with the tint its matches give it now: nothing to do. While
            // that tint isn't known yet (the matches' places still asked for), with any.
            let (tint, settled) = self.level.nav.tint(v.page);
            let key = tint.as_ref().map_or(0, |tint| tint.key);
            if let Some(s) = self.sent.iter().find(|s| s.view == v && (s.tint == key || !settled)) {
                near = near.saturating_add(s.bytes());
                continue;
            }
            // Laid out each time, since a file's size may have come to be known meanwhile: then what
            // was asked for it is no longer wanted. A render under way is the same page whatever its
            // tint: it carries on, and gets the tint of the moment it arrives ([`Pager::receive`]).
            let kept = self.wanted.iter().position(|w| w.view == v).map(|at| self.wanted.swap_remove(at));
            let next =
                page_layout(&*source, v.page, v.fit, Area { cols: v.cols, rows: v.rows, cell, max_rows }, self.output).map(|layout| match kept {
                    Some(w) if w.layout == layout => w,
                    _ => {
                        self.tag = self.tag.wrapping_add(1);
                        Wanted { view: v, layout, tag: self.tag }
                    }
                });
            match next {
                Ok(w) => {
                    let size = source.size(v.page).unwrap_or_default();
                    near = near.saturating_add(pixel_bytes(size, w.layout.scale));
                    if near > self.budget && !on_screen.contains(&Some(v)) {
                        break;
                    }
                    wanted.push(w);
                }
                Err(e) => self.fail(v, e),
            }
        }
        self.wanted = wanted;
        self.requeue();

        // What is wrong with a page on screen; in a list, with the tile chosen.
        if self.level.nav.message.is_none() {
            let nav = &self.level.nav;
            let noted = if nav.list { vec![nav.pages.get(nav.at).copied().map(view)] } else { on_screen.clone() };
            self.level.nav.message = note(&noted, &self.sent, &self.failed);
        }
        // Scrolling follows the first page on screen: only one as wide as the window is ever
        // taller than the window.
        let first_rows = on_screen.first().copied().flatten().and_then(|v| self.rows(&v)).unwrap_or(area_rows);
        self.level.nav.shown(first_rows.min(area_rows), first_rows);

        // Synchronized output, where the terminal has it, shows the redraw all at once.
        out.extend_from_slice(b"\x1b[?2026h");
        let ready = on_screen.iter().flatten().all(|v| self.sent.iter().any(|s| s.view == *v) || self.failed.iter().any(|(f, _)| f == v));
        // A list's tiles go on screen as they come, as a gallery's do.
        if self.level.nav.help {
            self.help(&mut out, size);
        } else if ready || self.placed.is_empty() || self.level.nav.list {
            self.place(&mut out, size, area_rows, grid);
        }
        cursor(&mut out, size.rows.saturating_sub(1), 0);
        out.extend_from_slice(b"\x1b[2K\x1b[7m");
        out.extend_from_slice(self.level.nav.status(&self.level.name(), size.cols, !ready).as_bytes());
        out.extend_from_slice(b"\x1b[0m\x1b[?2026l");
        self.ready = ready;
        self.wrote(0);
        self.tty.write_all(&out)
    }

    /// The status line written again, the rest of the screen as it is: a search's count as its text
    /// is read.
    fn status_line(&mut self) -> Result<(), String> {
        let size = self.tty.size().unwrap_or(FALLBACK_SIZE);
        let mut out = Vec::new();
        cursor(&mut out, size.rows.saturating_sub(1), 0);
        out.extend_from_slice(b"\x1b[2K\x1b[7m");
        out.extend_from_slice(self.level.nav.status(&self.level.name(), size.cols, !self.ready).as_bytes());
        out.extend_from_slice(b"\x1b[0m");
        self.wrote(0);
        self.tty.write_all(&out)
    }

    /// Take the search on, if the file on screen has one ([`Nav::search_on`]): whether the status
    /// line has news, and whether the pages are to be drawn again: the view went to a match, or a
    /// page's tint changed.
    fn search_on(&mut self) -> (bool, bool) {
        let source = self.level.source.clone();
        self.level.nav.search_on(&|page| source.size(page))
    }

    /// How the pages share the window's `area`: as the reader zoomed, from where the reader is,
    /// as far as the window takes ([`fit::grid`]).
    fn grid(&mut self, area: Area) -> Grid {
        let (nav, source) = (&self.level.nav, &*self.level.source);
        let grid = match nav.list {
            // Filtered, the tiles keep the size the whole list gives them, so what is in the
            // terminal shows at once (ADR-0010).
            true => fit::tiles(nav.zoom, nav.listed(), area),
            false => fit::grid(nav.zoom, nav.pages.len(), nav.at, |i| nav.pages.get(i).and_then(|p| page_size(source, *p)), area),
        };
        self.level.nav.grid(grid.per_row, grid.rows, grid.most);
        grid
    }

    /// Open the list's item at `at`: a file to its pages, or a container, listed by `opener`, to a
    /// list of its own, once read ([`Pager::read_on`]). What was on screen stops rendering, and
    /// waits to be gone back to.
    fn open(&mut self, at: usize, opener: &Opener<'_, '_>) -> Result<(), String> {
        let level = match self.level.items.get(at) {
            Some(Item::File(pages)) => {
                let pages = Pages { name: format!("{}{}", self.level.name, pages.name), ..pages.clone() };
                Level::new(self.levels, Shown { pages, items: Vec::new() }, Zoom::Auto, true, self.settings.clone(), self.room)
            }
            // It may not be open yet, or be the file that turns out not to open.
            Some(Item::Later { name, later }) => {
                let name = format!("{}{name}", self.level.name);
                self.going_into = Some(GoingInto { name, later: later.clone() });
                self.go_on_into()?;
                return Ok(());
            }
            Some(Item::Container { name, container }) => {
                // Read, and its files opened, which takes a while for large ones: said first.
                let (name, container) = (format!("{}{name}", self.level.name), container.clone());
                self.level.nav.message = Some(format!("opening {name}…"));
                self.draw()?;
                self.reading = Some(Reading { name, container, opening: None, since: Instant::now() });
                self.read_on(opener)?;
                return Ok(());
            }
            None => return Ok(()),
        };
        self.enter(level);
        Ok(())
    }

    /// Read on the container gone into: how far it has got on the status line, until its files are
    /// opened and it shows, as a level of its own. Whether it does now.
    fn read_on(&mut self, opener: &Opener<'_, '_>) -> Result<bool, String> {
        let Some(gone_into) = self.reading.as_mut() else { return Ok(false) };
        let opening = match gone_into.opening.as_mut() {
            Some(opening) => opening,
            None => match gone_into.container.ready() {
                Ready::Reading { read, of } => {
                    let said = reading(&gone_into.name, read, of);
                    return self.say(said).map(|()| false);
                }
                Ready::Now => match opener.begin(gone_into.container.clone(), "", false) {
                    Ok(opening) => gone_into.opening.insert(opening),
                    Err(why) => {
                        let Some(Reading { name, .. }) = self.reading.take() else { return Ok(false) };
                        self.later.push(why.1);
                        self.level.nav.message = Some(format!("nothing in {name} that {} shows", crate::NAME));
                        return Ok(true);
                    }
                },
            },
        };
        if !opener.step(opening, Some(Instant::now() + PROGRESS_EVERY)) {
            let said = match gone_into.since.elapsed() >= PROGRESS_AFTER {
                true => opened(&gone_into.name, opening.done()),
                false => format!("opening {}…", gone_into.name),
            };
            return self.say(said).map(|()| false);
        }
        let Some(Reading { name, container, opening: Some(opening), .. }) = self.reading.take() else { return Ok(false) };
        let mut found = Found::default();
        opener.finish(&mut found, opening);
        let Found { items, left_out, warnings, .. } = found;
        self.later.extend(warnings.into_iter().chain(left_out.iter().map(|(_, why)| why.clone())));
        if items.is_empty() {
            self.level.nav.message = Some(format!("nothing in {name} that {} shows", crate::NAME));
            return Ok(true);
        }
        self.level.nav.message = None;
        let mut level = Level::new(self.levels, Shown::list(items, name), self.level.nav.zoom, true, self.settings.clone(), self.room);
        level.nav.message = left_out_note(&left_out);
        level.from = Some(container);
        self.enter(level);
        Ok(true)
    }

    /// Go on into the file the reader chose in a list, if there is one: into it, or no further and
    /// why, once it is opened. Whether either happened.
    fn go_on_into(&mut self) -> Result<bool, String> {
        let Some(going) = &self.going_into else { return Ok(false) };
        let Some(outcome) = going.later.outcome() else {
            let said = format!("opening {}…", going.name);
            return self.say(said).map(|()| false);
        };
        let Some(GoingInto { name, .. }) = self.going_into.take() else { return Ok(false) };
        match outcome {
            Ok(source) => {
                self.level.nav.message = None;
                let shown = Shown { pages: Pages::all(source, name), items: Vec::new() };
                let level = Level::new(self.levels, shown, Zoom::Auto, true, self.settings.clone(), self.room);
                self.enter(level);
            }
            Err(why) => self.level.nav.message = Some(format!("{name}: {why}")),
        }
        Ok(true)
    }

    /// Stop reading the container gone into, if one is: it lets go of what it read. Or waiting for a
    /// file to go into, which, begun, goes on opening for its tile.
    fn stop_reading(&mut self) {
        if let Some(GoingInto { name, .. }) = self.going_into.take() {
            self.level.nav.message = Some(format!("stopped opening {name}"));
        }
        if let Some(Reading { name, container, .. }) = self.reading.take() {
            container.release();
            self.level.nav.message = Some(format!("stopped opening {name}"));
        }
    }

    /// `said` on the status line, the rest of the screen as it is.
    fn say(&mut self, said: String) -> Result<(), String> {
        let size = self.tty.size().unwrap_or(FALLBACK_SIZE);
        self.level.nav.message = Some(said);
        let mut out = Vec::new();
        cursor(&mut out, size.rows.saturating_sub(1), 0);
        out.extend_from_slice(b"\x1b[2K\x1b[7m");
        out.extend_from_slice(self.level.nav.status(&self.level.name(), size.cols, true).as_bytes());
        out.extend_from_slice(b"\x1b[0m");
        self.wrote(0);
        self.tty.write_all(&out)
    }

    /// Show `level`, opened from this one, which waits to be gone back to.
    fn enter(&mut self, level: Level) {
        self.levels = self.levels.wrapping_add(1);
        // A fresh pool, holding nothing: the old one lets go of what its files rendered with,
        // which for a PDF is the document parsed again.
        self.level.pool = self.level.source.pool(THREADS);
        self.wanted.clear();
        self.relayout.forget();
        let parent = std::mem::replace(&mut self.level, level);
        if let Some(said) = self.copier.left(parent.id) {
            self.level.nav.message.get_or_insert(said);
        }
        self.parents.push(parent);
    }

    /// Back to what the reader opened this from, as they left it. What this showed goes from the
    /// terminal: going back to it opens it anew.
    fn back(&mut self) -> Result<(), String> {
        let Some(parent) = self.parents.pop() else { return Ok(()) };
        self.relayout.forget();
        let left = std::mem::replace(&mut self.level, parent);
        // What it read its files with goes; what they render with, with its pool.
        if let Some(container) = &left.from {
            container.release();
        }
        let gone = left.id;
        drop(left);
        self.let_go(gone)
    }

    /// The level `gone` is no more: a copy still rendering on its pool stops, and says so, and its
    /// pages go from the terminal.
    fn let_go(&mut self, gone: u64) -> Result<(), String> {
        if let Some(said) = self.copier.left(gone) {
            self.level.nav.message = Some(said);
        }
        let mut out = Vec::new();
        for sent in self.sent.iter().filter(|s| s.view.level == gone) {
            delete(&mut out, sent.id, self.output);
        }
        let deleted: Vec<u32> = self.sent.iter().filter(|s| s.view.level == gone).map(|s| s.id).collect();
        self.sent.retain(|s| s.view.level != gone);
        self.placed.retain(|(id, _)| !deleted.contains(id));
        self.wanted.clear();
        self.wrote(0);
        self.tty.write_all(&out)
    }

    /// Lay the document on screen out again when a relayout setting it has is wanted at another value
    /// than its layout is at, or it is shown in another room than it was laid out for
    /// ([`relayout::wanted`]): after its keys, or on going into it after a change, or from a list, whose
    /// documents are laid out for a preview's room; never in a list, whose tiles keep their layout, nor
    /// with `--pages`, which picks pages as laid out. The job runs on the openers' threads, polled here
    /// on each pass, the status line saying the value wanted with `…` meanwhile and the old pages on
    /// screen until the new ones are ready. At most one job is under way: a change meanwhile waits in
    /// the shared values and starts when it ends, and a result for values or a room no longer wanted is
    /// dropped. One for what is wanted makes a new level in place of this one ([`Pager::replace`]); a
    /// failure keeps this one, says why once, puts the values back to what is on screen, so the next
    /// step starts from it, and remembers the pair that failed, not asked for again until the values or
    /// the room change, or `r`. Whether the screen changed.
    fn relayout(&mut self) -> Result<bool, String> {
        if self.level.nav.list || self.level.nav.picked {
            return Ok(false);
        }
        if self.level.nav.rerender {
            self.relayout.retry();
        }
        let (declared, has) = (self.settings.declared.clone(), self.level.nav.has.clone());
        let (laid, laid_room) = self.level.source.laid_at();
        let mut wanted = relayout::wanted(&declared, &has, self.settings.values.get(), laid, self.room_for(), laid_room);
        // What is being laid out that isn't the default, or that it is.
        let saying = |values: Values| Some(relayout::describe(&declared, &has, values)).filter(|said| !said.is_empty()).unwrap_or(LAYING_OUT.into());
        let mut changed = false;
        if let Some(handle) = self.relayout.handle() {
            let Some(done) = handle.try_take() else {
                // Still running: the status line says what is wanted now, which may have moved on.
                let saying = wanted.map(|(values, _)| saying(values));
                let changed = self.level.nav.relaying != saying;
                self.level.nav.relaying = saying;
                return Ok(changed);
            };
            if self.relayout.ended(wanted) == Ended::Taken {
                match done {
                    Ok(source) => {
                        self.replace(source)?;
                        return Ok(true);
                    }
                    Err(why) => {
                        self.level.nav.message = Some(printable(&why));
                        self.put_back(laid, laid_room);
                        if let Some((values, room)) = wanted {
                            self.relayout.fail(values, room);
                        }
                        wanted = None;
                        changed = true;
                    }
                }
            }
        }
        let Some((values, room)) = wanted.filter(|wanted| !self.relayout.refused(wanted)) else {
            self.level.nav.relaying = None;
            return Ok(changed);
        };
        let Some(job) = self.level.source.relayout(values, room) else {
            self.level.nav.message = Some("it can't be laid out again".into());
            self.put_back(laid, laid_room);
            self.level.nav.relaying = None;
            return Ok(true);
        };
        self.relayout.start(values, room, self.openers.run(job));
        self.level.nav.relaying = Some(saying(values));
        Ok(true)
    }

    /// Follow the window: the room its pages area makes now, the same cells `draw` lays its grid out in
    /// ([`super::area`]), becomes the room a document is laid out for once it has differed from that and
    /// held still for [`relayout::HOLD`] (its columns and rows, or its cell where the tty reports pixels;
    /// a cell asked or told once holds for the run). Until then the old pages are fitted to the new window
    /// as any page is. Whether the room changed, for [`Pager::relayout`] to act on.
    fn watch_room(&mut self) -> bool {
        let size = self.tty.size().unwrap_or(FALLBACK_SIZE);
        let now = super::room(super::area(size, self.cell, self.output));
        match self.hold.seen(now, &self.room, Instant::now()) {
            Some(room) => {
                self.room = room;
                true
            }
            None => false,
        }
    }

    /// The level's bounds ([`Nav::bounds`]) asked of its source again when the values or the room have
    /// changed since they were, or the level is another: where a setting's values sit depends on both
    /// (the width's on the font size), and the keys step by them before any layout lands. What the
    /// status line says of the layout follows, as a number past the most is said with what it is laid
    /// out at.
    fn refresh_bounds(&mut self) {
        let (values, room) = (self.settings.values.get(), self.room_for());
        if self.bounds_for.is_some_and(|(level, asked, asked_in)| level == self.level.id && asked == values && asked_in.same(&room)) {
            return;
        }
        self.bounds_for = Some((self.level.id, values, room));
        let (source, nav) = (&*self.level.source, &mut self.level.nav);
        nav.bounds = bounds_in(&self.settings, source, &nav.has, room, values);
        if !nav.list {
            nav.laid = laid_at(&self.settings, source, &nav.has, &nav.bounds);
        }
    }

    /// The shared values of the relayout settings back to `laid`, and the count to a row back to
    /// `laid_room`'s, what the document on screen is laid out at: a key that changed both is one change.
    fn put_back(&mut self, laid: Values, laid_room: Option<Room>) {
        let values = relayout::put_back(&self.settings.declared, &self.level.nav.has, self.settings.values.get(), laid);
        self.settings.values.set(values);
        if let Some(room) = laid_room {
            self.level.nav.put_back_columns(room.per_row);
        }
    }

    /// The room the document on screen is laid out for: the window's ([`Pager::room`]), in as many
    /// columns as the reader asked for, as far as that room takes them ([`Nav::columns_in`]).
    fn room_for(&self) -> Room {
        Room { per_row: self.level.nav.columns_in(&self.room), ..self.room }
    }

    /// `source`, the document on screen laid out again, in a level of its own in place of this one: at
    /// the page where the paragraph at the top of the screen now is, scrolled to it where a page scrolls
    /// ([`Source::anchor`]), or at the reader's page where the sources don't say; at the same zoom, gone
    /// into from a list if this one was. This one goes as a level gone back from does ([`Pager::let_go`]).
    fn replace(&mut self, source: Box<dyn Source + Send>) -> Result<(), String> {
        let place = self.place_on_screen().and_then(|anchor| source.find(&anchor));
        let source: Box<dyn Source> = source;
        let shown = Shown { pages: Pages::all(Rc::from(source), self.level.name.clone()), items: Vec::new() };
        let mut level = Level::new(self.levels, shown, self.level.nav.zoom, self.level.nav.nested, self.settings.clone(), self.room);
        self.levels = self.levels.wrapping_add(1);
        let last = level.nav.pages.len().saturating_sub(1);
        match place {
            Some((page, y)) => {
                level.nav.at = page.min(last);
                let height = level.source.size(page).map_or(0.0, |(_, h)| h);
                level.nav.scroll_to = (height > 0.0).then(|| y / height);
            }
            None => level.nav.at = self.level.nav.at.min(last),
        }
        level.nav.hint = false;
        level.nav.help = self.level.nav.help;
        level.nav.message.clone_from(&self.level.nav.message);
        self.relayout.forget();
        let gone = std::mem::replace(&mut self.level, level);
        let id = gone.id;
        drop(gone);
        self.let_go(id)
    }

    /// Where the reader is, as the source on screen knows it: the paragraph at the top of the screen, on
    /// the first page there, as far down it as the page is scrolled ([`place`]).
    fn place_on_screen(&self) -> Option<crate::source::Anchor> {
        let drawn = self.on_screen.first().copied().flatten().and_then(|view| Some((view, self.rows(&view)?)));
        let (page, y) = place(&self.level.nav, self.level.id, drawn, |page| self.level.source.size(page).map_or(0.0, |(_, h)| h))?;
        self.level.source.anchor(page, y)
    }

    /// `c`: copy page `page` of this level's source to the clipboard: the page, or in a list the
    /// chosen tile's picture (a folder has none). It renders on the level's pool beside the pages
    /// ([`clip::Copier`]).
    fn copy(&mut self, page: usize) -> Result<(), String> {
        let what = match self.level.items.get(page) {
            Some(Item::Container { name, .. }) => {
                self.level.nav.message = Some(format!("{name} is a folder: no picture to copy"));
                return Ok(());
            }
            // Its picture depends on its size, which waits for it to open.
            Some(Item::Later { name, later }) => {
                let (name, later) = (name.clone(), later.clone());
                match later.outcome() {
                    Some(Ok(_)) => {
                        // Laid out with its true size first, as what is listed must be.
                        self.draw()?;
                        name
                    }
                    Some(Err(why)) => {
                        self.level.nav.message = Some(format!("{name}: {why}"));
                        return Ok(());
                    }
                    None => {
                        self.level.nav.message = Some(format!("{name} is still opening: try again in a moment"));
                        return Ok(());
                    }
                }
            }
            Some(item) => item.name().to_owned(),
            // A picture, or a document of one page: by its name.
            None if self.level.source.pages() == 1 => self.level.name.clone(),
            None => format!("page {}", page.saturating_add(1)),
        };
        self.tag = self.tag.wrapping_add(1);
        let (mut out, size) = (Vec::new(), page_size(&*self.level.source, page));
        self.level.nav.message = Some(self.copier.start(what, self.level.id, page, size, self.tag, Instant::now(), &mut out));
        self.requeue();
        self.write_copy(&out)
    }

    /// Carry a copy on, if one is under way: says whether the status line has news.
    fn copy_on(&mut self) -> Result<bool, String> {
        // Nothing goes through tmux while the pane is hidden: it would be thrown away.
        if self.hidden {
            return Ok(false);
        }
        let mut out = Vec::new();
        let said = self.copier.on(Instant::now(), &mut out);
        self.write_copy(&out)?;
        Ok(said.map(|said| self.level.nav.message = Some(said)).is_some())
    }

    /// The terminal's answer about its clipboard: says whether the status line has news.
    fn copied(&mut self, answer: Key) -> bool {
        self.copier.answer(answer).map(|said| self.level.nav.message = Some(said)).is_some()
    }

    /// Write what a copy put in `out`, if anything.
    fn write_copy(&mut self, out: &[u8]) -> Result<(), String> {
        if out.is_empty() {
            return Ok(());
        }
        self.wrote(0);
        self.tty.write_all(out)
    }

    /// Note a write, with `images` transmissions in it: inside tmux, ask a moment later whether
    /// it reached the terminal.
    fn wrote(&mut self, images: u64) {
        self.transmitted = self.transmitted.saturating_add(images);
        if self.watch.is_some() {
            self.tmux_check = Some(Instant::now() + TMUX_AFTER_WRITE);
        }
    }

    /// Inside tmux: ask tmux what became of what was written, and act on it. Says whether the
    /// screen needs a redraw.
    ///
    /// - The pane is hidden: tmux throws passthrough away, so nothing is sent until it shows, and
    ///   files waiting for the terminal are given up on (without giving files up).
    /// - A terminal showing it may lack images (it is new, attached again, or back after images
    ///   went out without it): everything is sent again.
    /// - tmux threw output away (`#{client_discarded}` rose): images went with it (tmux redraws
    ///   its text, but not what went through passthrough), so everything is sent again, up to
    ///   [`MAX_RESENDS`] times.
    /// - More than one terminal shows it: files are over, as the first terminal to read one
    ///   deletes it.
    fn ask_tmux(&mut self) -> bool {
        self.tmux_check = Some(Instant::now() + TMUX_EVERY);
        let (Some(watch), Some(report)) = (self.watch.as_mut(), tmux::report()) else { return false };
        let news = watch.update(&report, self.transmitted);
        if news.showing > 1 {
            self.files = None;
        }
        let was_hidden = std::mem::replace(&mut self.hidden, news.showing == 0);
        if self.hidden {
            if !was_hidden && let Some(files) = self.files.as_mut() {
                files.forget();
            }
            return false;
        }
        if news.discarded {
            if let Some(said) = self.copier.lost() {
                self.level.nav.message = Some(said);
            }
            if self.resends >= MAX_RESENDS {
                self.level.nav.message = Some("tmux is dropping images: the terminal can't keep up; r sends them again".into());
                return true;
            }
            self.resends = self.resends.saturating_add(1);
        }
        if news.discarded || news.missed {
            self.level.nav.rerender = true;
            return true;
        }
        false
    }

    /// The terminal says it no longer has image `id`, which it was asked to place: it dropped it
    /// to stay under its quota. Forget it, so it is sent again, and keep less in the terminal from
    /// now on, up to [`MAX_RESENDS`] times (`r` still sends). Says whether the screen needs a
    /// redraw: past that, only to say so once, since a redraw places the image again and draws
    /// the same answer.
    fn missing(&mut self, id: u32) -> bool {
        const DROPPING: &str = "the terminal is dropping images; r sends them again";
        if !self.sent.iter().any(|s| s.id == id) {
            return false;
        }
        if self.resends >= MAX_RESENDS {
            let said = self.level.nav.message.as_deref() == Some(DROPPING);
            self.level.nav.message = Some(DROPPING.into());
            return !said;
        }
        self.resends = self.resends.saturating_add(1);
        self.budget = (self.budget / 2).max(MIN_BUDGET);
        self.sent.retain(|s| s.id != id);
        self.placed.retain(|(placed, _)| *placed != id);
        true
    }

    /// Before the screen is put back: wait a moment for the answers to the last placements, and
    /// to the clipboard's question or write, so none of them reaches the command line.
    fn await_answers(&self) {
        let placed = self.placed_at.map(|at| at + ANSWER_WAIT);
        let Some(deadline) = placed.max(self.copier.due().map(|at| at + clip::ANSWER_WAIT)) else { return };
        while let Some(left) = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) {
            if self.tty.read_timeout(left).is_err() {
                break;
            }
        }
    }

    /// The keys, in place of the pages: the pages leave the screen, and come back with the next
    /// draw after it.
    fn help(&mut self, out: &mut Vec<u8>, size: WindowSize) {
        for row in 1..=size.rows {
            out.extend_from_slice(format!("\x1b[{row};1H\x1b[2K").as_bytes());
        }
        // Inside tmux, erasing their placeholder cells already took them away.
        if self.output == Output::Direct {
            for (id, placement) in &self.placed {
                kitty::hide(out, *id, *placement, self.output.wrap());
            }
        }
        self.placed.clear();
        self.on_glass.clear();
        let help = self.level.nav.help_lines();
        let keys = help.iter().map(|(keys, _)| chars(keys)).max().unwrap_or(0);
        let lines = std::iter::once(String::from("Keys")).chain(help.iter().map(|(k, what)| format!("  {k:<keys$}   {what}")));
        let width = usize::from(size.cols).saturating_sub(2);
        // Above the status line, as many as fit.
        for (row, line) in (1..size.rows.saturating_sub(1)).zip(lines) {
            cursor(out, row, 2);
            out.extend_from_slice(cut(&line, width).as_bytes());
        }
    }

    /// Put the pages on screen in place of the ones before, laid out as `grid` says: the rows of
    /// pages centred, each page centred in its place, and a short last row keeping the places on
    /// the left, unless it is the only row there is (fewer pages than the zoom puts in a row).
    /// Each place on screen has a placement of its own, so a page given twice shows twice.
    fn place(&mut self, out: &mut Vec<u8>, size: WindowSize, area_rows: u16, grid: Grid) {
        let wrap = self.output.wrap();
        // Drawn as text, each row is written whole, below, and erased only as what replaces it
        // comes: a terminal may paint between the two, and that shows as tearing, not a blank.
        let mut text_rows = match self.output {
            Output::Text(_) => vec![Vec::new(); usize::from(size.rows)],
            Output::Direct | Output::Tmux(_) => {
                // Erasing text never erases images (only clearing the whole screen does, by the
                // protocol): placements are what show and move them.
                for row in 1..=size.rows {
                    out.extend_from_slice(format!("\x1b[{row};1H\x1b[2K").as_bytes());
                }
                Vec::new()
            }
        };
        let per_row = grid.per_row.max(1);
        let (across, down) = (grid.cols.saturating_add(fit::GAP), grid.page_rows.saturating_add(grid.label).saturating_add(fit::ROW_GAP));
        let filled = u16::try_from(self.level.nav.pages.len()).map_or(per_row, |n| n.clamp(1, per_row));
        let width = across.saturating_mul(filled).saturating_sub(fit::GAP);
        // Centred on the rows that have pages: a document shorter than the screen sits in the
        // middle of it, not at the top.
        let last = self.on_screen.iter().rposition(Option::is_some).unwrap_or(0);
        let rows = u16::try_from(last / usize::from(per_row)).map_or(grid.rows, |r| r.saturating_add(1).min(grid.rows));
        let height = down.saturating_mul(rows).saturating_sub(fit::ROW_GAP);
        let (left, top) = (size.cols.saturating_sub(width) / 2, area_rows.saturating_sub(height) / 2);
        // Only a page as wide as the window scrolls; the draw keeps the others' scroll at 0.
        let scroll = self.level.nav.scroll;
        let mut placed = Vec::new();
        for ((slot, view), placement) in (0u16..).zip(&self.on_screen).zip(1u32..) {
            // Of a page sent twice, the one with the tint of now; before that one has come, the last sent.
            let key = view.map_or(0, |v| self.level.nav.tint(v.page).0.map_or(0, |tint| tint.key));
            let Some(s) = view.and_then(|v| shown(&self.sent, v, key)) else { continue };
            let visible = s.rows.min(area_rows);
            let left = left.saturating_add((slot % per_row).saturating_mul(across)).saturating_add(grid.cols.saturating_sub(s.cols) / 2);
            let top = top.saturating_add((slot / per_row).saturating_mul(down)).saturating_add(grid.page_rows.saturating_sub(visible) / 2);
            match self.output {
                Output::Direct => {
                    cursor(out, top, left);
                    let src = (visible < s.rows).then(|| source_rows(s, scroll, visible));
                    kitty::place(out, s.id, placement, s.cols, visible, src, wrap);
                    self.placed_at = Some(Instant::now());
                }
                Output::Tmux(colour) => {
                    for row in 0..visible {
                        cursor(out, top.saturating_add(row), left);
                        // Within the placeholders' table: the layout keeps it so.
                        let _ = kitty::placeholder_row(out, s.id, colour, scroll.saturating_add(row), s.cols);
                    }
                }
                Output::Text(_) => {
                    for row in 0..visible {
                        let at = top.saturating_add(row);
                        let text = s.text.get(usize::from(scroll.saturating_add(row)));
                        if let (Some(line), Some(text)) = (text_rows.get_mut(usize::from(at)), text) {
                            cursor(line, at, left);
                            line.extend_from_slice(text);
                        }
                    }
                }
            }
            placed.push((s.id, placement));
        }
        // A tile's name under it, the chosen one's highlighted, whether its tile is there yet or not.
        let nav = &self.level.nav;
        let chosen = nav.pages.get(nav.at).copied();
        let labelled = if grid.label > 0 { self.on_screen.as_slice() } else { &[] };
        for (slot, view) in (0u16..).zip(labelled) {
            let Some((v, item)) = view.and_then(|v| Some((v, self.level.items.get(v.page)?))) else { continue };
            let row = top.saturating_add((slot / per_row).saturating_mul(down)).saturating_add(grid.page_rows);
            let into = text_rows.get_mut(usize::from(row)).unwrap_or(&mut *out);
            cursor(into, row, left.saturating_add((slot % per_row).saturating_mul(across)));
            // A tile that failed is an empty place, as one not rendered yet is: this tells them apart.
            let broken = self.failed.iter().any(|(failed, _)| *failed == v);
            let name = if broken { format!("! {}", item.name()) } else { item.name().to_owned() };
            let name = printable(&label(&name, grid.cols));
            let (on, off) = if chosen == Some(v.page) { ("\x1b[7m", "\x1b[0m") } else { ("", "") };
            into.extend_from_slice(format!("{on}{name}{off}").as_bytes());
        }
        // The rows of text that aren't as they already are.
        if matches!(self.output, Output::Text(_)) {
            for (row, line) in (0u16..).zip(&text_rows) {
                if self.on_glass.get(usize::from(row)) != Some(line) {
                    cursor(out, row, 0);
                    out.extend_from_slice(b"\x1b[2K");
                    out.extend_from_slice(line);
                }
            }
            self.on_glass = text_rows;
        }
        // The pages before leave the screen; their data stays in the terminal for later. Inside
        // tmux, erasing their placeholder cells already did it.
        if self.output == Output::Direct {
            for (id, placement) in self.placed.iter().filter(|p| !placed.contains(p)) {
                kitty::hide(out, *id, *placement, wrap);
            }
        }
        // The pages on screen are the most recently used: the last to be dropped.
        for (id, _) in &placed {
            if let Some(p) = self.sent.iter().position(|s| s.id == *id) {
                let s = self.sent.remove(p);
                self.sent.push(s);
            }
        }
        // A page placed in place of itself as it was sent before, with another tint: that one goes
        // from the terminal now that this one shows.
        for (id, _) in &placed {
            let Some(view) = self.sent.iter().find(|s| s.id == *id).map(|s| s.view) else { continue };
            for gone in superseded(&mut self.sent, view, Some(*id)) {
                delete(out, gone, self.output);
            }
        }
        self.placed = placed;
    }

    /// The image rows `view` covers, once its layout is known.
    fn rows(&self, view: &View) -> Option<u16> {
        let sent = self.sent.iter().find(|s| s.view == *view).map(|s| s.rows);
        sent.or_else(|| self.wanted.iter().find(|w| w.view == *view).map(|w| w.layout.rows))
    }

    /// Take finished renders and send them to the terminal. Says whether one of the pages on
    /// screen arrived (the screen needs a redraw).
    fn receive(&mut self) -> Result<bool, String> {
        let wrap = self.output.wrap();
        let (mut any, mut on_screen) = (false, false);
        while let Some(page) = self.level.pool.try_recv() {
            if self.copier.wants(page.request.tag) {
                self.copier.rendered(page);
                any = true;
                continue;
            }
            // Otherwise a late result for a page no longer wanted.
            let Some(at) = self.wanted.iter().position(|w| w.tag == page.request.tag) else { continue };
            let wanted = self.wanted.remove(at);
            any = true;
            // Made for a size no longer true: a draw lays it out with the true one, on screen or not.
            if page.again {
                on_screen = true;
                continue;
            }
            let shown = self.on_screen.contains(&Some(wanted.view));
            on_screen |= shown;
            // A search's matches, tinted into the pixels before they are encoded or made into cells:
            // as they are now, not as they were when the page was asked for, nor at the last draw. A key
            // read in this pass may have moved the reader's match since, and then the page the terminal
            // holds may be the right one again: this one isn't sent.
            let (matches, _) = self.level.nav.tint(wanted.view.page);
            let tint = matches.as_ref().map_or(0, |tint| tint.key);
            if self.sent.iter().any(|s| s.view == wanted.view && s.tint == tint) {
                continue;
            }
            let id = self.new_id();
            let (width, height) = (page.width, page.height);
            let page = match (&matches, self.level.source.size(wanted.view.page)) {
                (Some(tint), Some((w, h))) if page.error.is_none() && w > 0.0 && h > 0.0 => {
                    let mut rgba = page.rgba.to_vec();
                    // Pixels to the page's unit, each way: what it was rendered at, rounding included.
                    search::tint(&mut rgba, width, height, (width as f32 / w, height as f32 / h), &tint.marks);
                    Frame { rgba: crate::source::Pixels::new(rgba), ..page }
                }
                _ => page,
            };
            match checked(&page).and_then(|warning| Ok((warning, self.payload(id, page, &wanted.layout)?))) {
                Ok((warning, Some(payload))) => {
                    let mut out = Vec::new();
                    let bytes = match &payload {
                        Payload::Text(text) => text_bytes(text),
                        Payload::Png(_) | Payload::File(_) => u64::from(width).saturating_mul(u64::from(height)).saturating_mul(4),
                    };
                    // The page as it was sent before, with another tint, goes now unless it is on
                    // screen, where it stays until this one is placed over it ([`Pager::place`]).
                    if !shown {
                        for gone in superseded(&mut self.sent, wanted.view, None) {
                            delete(&mut out, gone, self.output);
                            self.placed.retain(|(placed, _)| *placed != gone);
                        }
                    }
                    for gone in make_room(&mut self.sent, bytes, self.budget, self.kept, &self.on_screen) {
                        delete(&mut out, gone, self.output);
                        self.placed.retain(|(placed, _)| *placed != gone);
                    }
                    let (cols, rows) = (wanted.layout.cols, wanted.layout.rows);
                    let text = match payload {
                        Payload::Png(png) => {
                            kitty::transmit(&mut out, id, &png, wrap);
                            Vec::new()
                        }
                        Payload::File(path) => {
                            kitty::transmit_file(&mut out, id, &path, width, height, wrap);
                            Vec::new()
                        }
                        Payload::Text(text) => text,
                    };
                    if let Output::Tmux(_) = self.output {
                        kitty::place_virtual(&mut out, id, cols, rows, wrap);
                    }
                    self.wrote(1);
                    self.tty.write_all(&out)?;
                    self.sent.push(Sent { view: wanted.view, id, cols, rows, width, height, warning, text, tint });
                }
                // The files' thread kept the page: the next draw asks for it again.
                Ok((_, None)) => on_screen = true,
                Err(e) => self.fail(wanted.view, e),
            }
            // A page the reader waits for goes on screen before any more work ahead is encoded;
            // the other finished renders wait in the pool for the next round.
            if on_screen {
                break;
            }
        }
        if any {
            self.requeue();
        }
        Ok(on_screen)
    }

    /// `page`'s pixels ready to send as image `id`: in a temporary file when the terminal takes
    /// them, otherwise as a PNG; or as text, in the cells `layout` gives it. A file that can't be
    /// written ends the files, and says why. `None` when the files' thread stopped answering with
    /// the page.
    fn payload(&mut self, id: u32, page: Frame, layout: &Layout) -> Result<Option<Payload>, String> {
        if let Output::Text(style) = self.output {
            return cell_lines(&page, layout, style).map(|text| Some(Payload::Text(text)));
        }
        let now = Instant::now();
        let mut page = page;
        if let Some(files) = self.files.as_mut()
            && files.ready(now)
        {
            match files.write(id, page, now) {
                Written::File(path) => return Ok(Some(Payload::File(path))),
                Written::Failed(e, back) => {
                    self.level.nav.message = Some(format!("{e}; sending pages in the terminal stream instead"));
                    page = *back;
                }
                Written::Stalled => return Ok(None),
            }
        }
        png(&page, self.effort).map(|png| Some(Payload::Png(png)))
    }

    /// Forget the pages whose files the terminal never took, so they are sent again when wanted,
    /// and end the files if their thread stopped answering. Says whether there were any (the
    /// screen needs a redraw).
    fn lost_files(&mut self) -> Result<bool, String> {
        let Some(files) = self.files.as_mut() else { return Ok(false) };
        let lost = files.check(Instant::now());
        if files.stalled() {
            self.level.nav.message = Some("the temporary directory stopped answering; sending pages in the terminal stream instead".into());
            self.stalled_files = self.files.take();
        }
        if lost.is_empty() {
            return Ok(false);
        }
        // The terminal may have taken one after all, late: none is left behind in it.
        let mut out = Vec::new();
        for id in &lost {
            delete(&mut out, *id, self.output);
        }
        self.wrote(0);
        self.tty.write_all(&out)?;
        self.sent.retain(|s| !lost.contains(&s.id));
        self.placed.retain(|(id, _)| !lost.contains(id));
        Ok(true)
    }

    fn fail(&mut self, view: View, why: String) {
        if self.failed.len() >= MAX_FAILED {
            self.failed.remove(0);
        }
        self.failed.push((view, why));
    }

    /// The next image id: one never used in this run until the ids wrap (after 65,280 of them in
    /// the palette, far more than the pages kept).
    fn new_id(&mut self) -> u32 {
        let id = self.output.ids().id(self.first_id.wrapping_add(self.next_id));
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    /// Tell the pool which renders are still wanted, most urgent first (a page being copied, then
    /// the pages): it keeps those and cancels the rest. A page goes with the settings' values (on
    /// the paper it is shown on), and is copied as it is shown: a list's tile as a tile.
    fn requeue(&self) {
        let (values, dark, tile) = (self.settings.values.get(), self.dark, !self.level.items.is_empty());
        let copy = self.copier.request(self.level.id).map(|copy| Request { values, dark, tile, ..copy });
        let pages = self.wanted.iter().map(|w| Request { tile, ..request(w.view.page, &w.layout, w.tag, values, dark) });
        self.level.pool.set_queue(copy.into_iter().chain(pages).collect());
    }
}

/// The place to anchor a relayout on: the first page on screen as `nav` has it now, and how far down
/// it, in points, by the page's height from `height_of`. The rows come from the last draw (`drawn`:
/// the first view it placed, and its rows), and count only when it drew this page of level `level`: a
/// key read in the same pass may have moved the reader since, and then the place is that page's top.
fn place(nav: &Nav, level: u64, drawn: Option<(View, u16)>, height_of: impl Fn(usize) -> f32) -> Option<(usize, f32)> {
    let page = nav.pages.get(nav.first()).copied()?;
    let rows = drawn.filter(|(view, rows)| view.level == level && view.page == page && *rows > 0).map(|(_, rows)| rows);
    let y = rows.map_or(0.0, |rows| f32::from(nav.scroll.min(rows)) / f32::from(rows) * height_of(page));
    Some((page, y))
}

/// The page of `view` to place, of those in `sent`: the one sent with the tint `key`, the tint of
/// now; while there is none yet, the last sent, which stays on screen until that one comes.
fn shown(sent: &[Sent], view: View, key: u64) -> Option<&Sent> {
    sent.iter().rfind(|s| s.view == view && s.tint == key).or_else(|| sent.iter().rfind(|s| s.view == view))
}

/// The pages of `view` in `sent` that another takes the place of, taken out, and their ids: every
/// one but `kept`, the one now placed; or every one, for a page about to be sent again.
fn superseded(sent: &mut Vec<Sent>, view: View, kept: Option<u32>) -> Vec<u32> {
    let gone: Vec<u32> = sent.iter().filter(|s| s.view == view && Some(s.id) != kept).map(|s| s.id).collect();
    sent.retain(|s| !gone.contains(&s.id));
    gone
}

/// Room in the terminal for a page of `incoming` bytes: the least recently shown pages not
/// `on_screen` go while keeping them would make more than `most` pages, or more than `budget`
/// bytes. The ids of the ones that go.
fn make_room(sent: &mut Vec<Sent>, incoming: u64, budget: u64, most: usize, on_screen: &[Option<View>]) -> Vec<u32> {
    let mut gone = Vec::new();
    loop {
        let bytes = sent.iter().map(Sent::bytes).fold(incoming, u64::saturating_add);
        if sent.len() < most && bytes <= budget {
            break;
        }
        let Some(at) = sent.iter().position(|s| !on_screen.contains(&Some(s.view))) else { break };
        gone.push(sent.remove(at).id);
    }
    gone
}

/// The bytes of pixels a page `size` points large renders to at `scale` pixels per point.
fn pixel_bytes((w, h): (f32, f32), scale: f32) -> u64 {
    // Finite and clamped to the largest side a layout allows, so the cast is exact.
    let side = |points: f32| {
        let pixels = (points * scale).ceil();
        if pixels.is_finite() { pixels.clamp(0.0, fit::MAX_PIXELS) as u64 } else { 0 }
    };
    side(w).saturating_mul(side(h)).saturating_mul(4)
}

/// What the status line says about the pages on screen, the first page's first: why one failed,
/// or what was skipped on one.
fn note(on_screen: &[Option<View>], sent: &[Sent], failed: &[(View, String)]) -> Option<String> {
    on_screen.iter().flatten().find_map(|v| {
        let failure = failed.iter().find(|(f, _)| f == v).map(|(_, e)| e.clone());
        failure.or_else(|| sent.iter().find(|s| s.view == *v).and_then(|s| s.warning.clone()))
    })
}

/// Move the cursor to 0-based `row` and `col`.
fn cursor(out: &mut Vec<u8>, row: u16, col: u16) {
    out.extend_from_slice(format!("\x1b[{};{}H", u32::from(row) + 1, u32::from(col) + 1).as_bytes());
}

/// The pixels of `shown` that `visible` cell rows starting at row `scroll` cover.
fn source_rows(shown: &Sent, scroll: u16, visible: u16) -> Rect {
    let per_row = f64::from(shown.height) / f64::from(shown.rows.max(1));
    let height = f64::from(shown.height);
    // Finite, and clamped to the image's height, so the casts are exact.
    let y = (f64::from(scroll) * per_row).round().clamp(0.0, height);
    let h = (f64::from(visible) * per_row).round().clamp(0.0, height - y);
    Rect { x: 0, y: y as u32, w: shown.width, h: h as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader at `at` in a `count`-page document, after a draw that showed `visible` of a
    /// page's `rows` rows, one page to the screen.
    fn reader(at: usize, count: usize, visible: u16, rows: u16) -> Nav {
        let mut nav = Nav::new((0..count).collect(), count, Zoom::Auto);
        nav.go(at);
        nav.shown(visible, rows);
        nav
    }

    /// The page on screen (the first, when two are).
    fn page(nav: &Nav) -> usize {
        nav.on_screen()[0].unwrap()
    }

    /// A setting the tests declare, as a backend would: two named values, cycled by `d`.
    const SHADE: Setting = Setting {
        name: "shade",
        named: &[Named { name: "pale", says: "pale now" }, Named { name: "deep", says: "deep now" }],
        numbers: None,
        default: Value::Named("pale"),
        zero: Value::Named("pale"),
        auto: None,
        keys: Keys::Cycle('d'),
        option: crate::args::OptionSpec { name: "--shade", short: None, value: Some("pale|deep"), help: "", alias: None },
        help: "pale or deep",
        missing: "only some go deep",
        effect: Effect::Redraw,
        sets_width: false,
    };

    /// `nav`, in a build that declares [`SHADE`], showing what has it if `has`.
    fn declaring(nav: Nav, has: bool) -> Nav {
        let settings = Rc::new(Settings { declared: vec![&SHADE], values: std::cell::Cell::default(), changed: Default::default() });
        Nav { settings, has: if has { vec![SHADE.name] } else { Vec::new() }, ..nav }
    }

    /// The value `nav` has for [`SHADE`], if it was set.
    fn shade(nav: &Nav) -> Option<Value> {
        nav.settings.values.get().get(SHADE.name)
    }

    /// A setting's key, the viewer's taking none: on what has it, it goes to the next named value
    /// and says what that is, the pages made again; on what hasn't, it says the setting's message
    /// and changes nothing. A number typed before a setting without numbers is left as it is.
    #[test]
    fn a_settings_key_cycles_its_values_where_what_is_shown_has_it() {
        for nav in [reader(0, 12, 40, 40), list(9)] {
            let mut nav = declaring(nav, true);
            assert_eq!(press(&mut nav, &[Key::Char('d')]), Step::Continue);
            assert_eq!((shade(&nav), nav.message.clone(), nav.wants_rerender()), (Some(Value::Named("deep")), Some("deep now".to_owned()), true));
            press(&mut nav, &[Key::Char('d')]);
            assert_eq!((shade(&nav), nav.message.clone(), nav.wants_rerender()), (Some(Value::Named("pale")), Some("pale now".to_owned()), true));
        }
        let mut nav = declaring(reader(0, 12, 40, 40), false);
        press(&mut nav, &[Key::Char('d')]);
        assert_eq!((shade(&nav), nav.message.clone(), nav.wants_rerender()), (None, Some("only some go deep".to_owned()), false));
        let mut nav = declaring(reader(0, 12, 40, 40), true);
        press(&mut nav, &[Key::Char('4'), Key::Char('d')]);
        assert_eq!((nav.typed.as_str(), shade(&nav)), ("4", Some(Value::Named("deep"))), "the number is still there");
        // Shared: a level opened from this one, and this one gone back to, have the same.
        let other = Nav { has: vec![SHADE.name], settings: nav.settings.clone(), ..reader(0, 3, 40, 40) };
        assert_eq!(shade(&other), Some(Value::Named("deep")));
        // Without the setting declared, the key is nobody's.
        let mut nav = reader(0, 12, 40, 40);
        press(&mut nav, &[Key::Char('d')]);
        assert_eq!((nav.message.clone(), nav.wants_rerender()), (None, false));
    }

    /// A value changed in a file opened from a list, and the file left before the next draw (`d`
    /// then `u`, read together): the list, drawn next, still makes its tiles again with the new value,
    /// rather than keeping those made with the old. Once.
    #[test]
    fn a_value_changed_in_a_level_left_before_the_draw_still_renders_again() {
        let mut list = declaring(list(9), true);
        let mut opened = Nav { nested: true, has: vec![SHADE.name], settings: list.settings.clone(), ..reader(0, 3, 40, 40) };
        assert_eq!(press(&mut opened, &[Key::Char('d'), Key::Char('u')]), Step::Back);
        drop(opened);
        assert_eq!(shade(&list), Some(Value::Named("deep")));
        assert!(list.wants_rerender(), "the list's tiles were made on the old value");
        assert!(!list.wants_rerender(), "once");
    }

    /// A setting with numbers the tests declare: `window`, or 20 to 200 in steps of 4, by `[` and `]`;
    /// a relayout setting. Where `window` sits is the document's to say ([`declaring_both`]).
    const WIDTH: Setting = Setting {
        name: "width",
        named: &[Named { name: "window", says: "width window" }],
        numbers: Some(Numbers { least: 20, most: 200, step: 4, unit: "" }),
        default: Value::Named("window"),
        zero: Value::Named("window"),
        keys: Keys::Step('[', ']'),
        help: "narrower or wider",
        missing: "only some are as wide",
        effect: Effect::Relayout,
        sets_width: true,
        ..SHADE
    };

    /// `nav` in a build that declares [`SHADE`] and [`WIDTH`], showing what has `has` of them: a
    /// document whose room sets `window` at 83 characters, with nothing under the setting's own most.
    fn declaring_both(nav: Nav, has: &[&'static str]) -> Nav {
        declaring_in(nav, has, Bounds { named_at: Some(83), most: None })
    }

    /// [`declaring_both`], the width's values sitting as `bounds` say.
    fn declaring_in(nav: Nav, has: &[&'static str], bounds: Bounds) -> Nav {
        let settings = Rc::new(Settings { declared: vec![&SHADE, &WIDTH], values: std::cell::Cell::default(), changed: Default::default() });
        let bounds = if has.contains(&WIDTH.name) { vec![(WIDTH.name, bounds)] } else { Vec::new() };
        Nav { settings, has: has.to_vec(), bounds, ..nav }
    }

    /// The width `nav` has, and what the status line says.
    fn width(nav: &Nav) -> (Option<Value>, Option<&str>) {
        (nav.settings.values.get().get(WIDTH.name), nav.message.as_deref())
    }

    /// On a document laid out for its room, `,` and `.` are columns: the count reaches the layout
    /// ([`Nav::columns`]), `.` stops at one to a row, and the width is one to a row. Between a count and a
    /// typed width the last key wins: a count sets `width` back to `window`, a width key sets the count
    /// back to `auto` first and is pressed again once the bounds are asked again; `0.` and `0]` change
    /// only their own. In a list, with `--pages` and on a PDF, the count is the zoom alone.
    #[test]
    fn on_a_reflowed_document_a_count_is_columns_and_the_last_key_wins() {
        let chars = |text: &str| text.chars().map(Key::Char).collect::<Vec<_>>();
        // As the pager does: a key that asks to be pressed again is, the bounds asked again between.
        let press = |nav: &mut Nav, keys: &[Key]| {
            for key in keys {
                if let Step::Again(key) = nav.key(*key) {
                    nav.key(key);
                }
            }
        };
        let mut nav = declaring_both(reader(0, 20, 40, 40), &["width"]);
        nav.grid(1, 1, 8);
        assert_eq!((nav.reflows(), nav.columns()), (true, None), "auto: the document's to say");
        press(&mut nav, &chars("3."));
        assert_eq!((nav.zoom, nav.columns()), (Zoom::PerRow(3), Some(3)));
        press(&mut nav, &chars(","));
        assert_eq!(nav.columns(), Some(4));
        // `.` down to one, and no further: never the width.
        press(&mut nav, &chars("..."));
        assert_eq!((nav.zoom, nav.message.as_deref()), (Zoom::PerRow(1), None));
        press(&mut nav, &chars("."));
        assert_eq!((nav.zoom, nav.columns(), nav.message.as_deref()), (Zoom::PerRow(1), Some(1), Some("1 per row, the least")));
        // From `auto` showing one: the least already.
        let mut one = declaring_both(reader(0, 20, 40, 40), &["width"]);
        one.grid(1, 1, 8);
        press(&mut one, &chars("."));
        assert_eq!((one.zoom, one.message.as_deref()), (Zoom::Auto, Some("1 per row, the least")));
        // A count sets the width back to `window`; `0.` leaves it.
        press(&mut nav, &chars("72"));
        assert_eq!(nav.key(Key::Char(']')), Step::Again(Key::Char(']')), "the count goes to auto first");
        assert_eq!((nav.zoom, nav.typed.as_str()), (Zoom::Auto, "72"), "the number waits for the key again");
        assert_eq!(nav.key(Key::Char(']')), Step::Continue);
        assert_eq!((width(&nav), nav.columns()), ((Some(Value::Number(72)), Some("width 72")), None));
        press(&mut nav, &chars("0."));
        assert_eq!(width(&nav).0, Some(Value::Number(72)), "0. changes only the count");
        press(&mut nav, &chars("3."));
        assert_eq!((width(&nav).0, nav.columns()), (Some(Value::Named("window")), Some(3)));
        press(&mut nav, &chars("72]"));
        press(&mut nav, &chars(","));
        assert_eq!((width(&nav).0, nav.zoom), (Some(Value::Named("window")), Zoom::PerRow(2)), "`,` from auto showing one");
        // `[` steps from `window` as it sits at auto, once the count is back there.
        assert_eq!(nav.key(Key::Char('[')), Step::Again(Key::Char('[')));
        assert_eq!(nav.zoom, Zoom::Auto);
        nav.key(Key::Char('['));
        // `0]` changes only the width.
        press(&mut nav, &chars("3."));
        nav.key(Key::Char('0'));
        assert_eq!(nav.key(Key::Char(']')), Step::Continue);
        assert_eq!((nav.columns(), width(&nav).0), (Some(3), Some(Value::Named("window"))));
        // Before any draw, a level made at a count (a relayout's, or the first) is laid out in that many.
        let fresh = declaring_both(Nav::new((0..20).collect(), 20, Zoom::PerRow(3)), &["width"]);
        assert_eq!((fresh.per_row, fresh.columns()), (3, Some(3)));
        // The room says how many it takes: a window that takes two lays out two, whatever was drawn.
        let room = Room { cols: 26, rows: 23, cell_w: 10.0, cell_h: 20.0, gap: 2, per_row: None };
        assert_eq!((fresh.columns_in(&room), fresh.columns_in(&Room { cols: 100, ..room })), (Some(2), Some(3)));
        // A layout that failed puts the count back to what the document is laid out in.
        nav.put_back_columns(None);
        assert_eq!((nav.zoom, nav.columns()), (Zoom::Auto, None));
        nav.put_back_columns(Some(2));
        assert_eq!(nav.zoom, Zoom::PerRow(2));
        // Elsewhere the count is the zoom alone: in a list, with `--pages`, on what has no relayout setting.
        for mut other in [
            declaring_both(Nav { list: true, ..reader(0, 20, 40, 40) }, &["width"]),
            Nav { picked: true, ..declaring_both(reader(0, 20, 40, 40), &["width"]) },
            declaring_both(reader(0, 20, 40, 40), &["shade"]),
        ] {
            press(&mut other, &chars("3."));
            assert_eq!((other.zoom, other.columns(), other.reflows()), (Zoom::PerRow(3), None, false));
            press(&mut other, &chars("72]"));
            assert_eq!(other.zoom, Zoom::PerRow(3), "a width key leaves the zoom");
        }
        assert!(nav.help_lines().contains(&(COLUMNS_HELP.0.to_owned(), COLUMNS_HELP.1)), "`?` says columns");
        let mut picked = Nav { picked: true, ..declaring_both(reader(0, 20, 40, 40), &["width"]) };
        assert!(!picked.help_lines().iter().any(|(_, what)| *what == COLUMNS_HELP.1), "with --pages, the zoom's");
        picked.grid(1, 1, 8);
        press(&mut picked, &chars("."));
        assert_eq!(picked.zoom, Zoom::Width, "with --pages, the width");
    }

    /// `Step` keys go to the next multiple of the step down or up, through the named value set among
    /// them where it sits; at the least or the most the value stays and the line says so. A relayout
    /// setting's change asks for no redraw.
    #[test]
    fn step_keys_go_to_the_next_multiple_and_stay_at_the_ends() {
        let mut nav = declaring_both(reader(0, 12, 40, 40), &["width"]);
        press(&mut nav, &[Key::Char(']')]);
        assert_eq!(width(&nav), (Some(Value::Number(84)), Some("width 84")), "from window, set at 83, up is 84");
        assert!(!nav.wants_rerender(), "laid out again, not drawn again");
        press(&mut nav, &[Key::Char('[')]);
        assert_eq!(width(&nav), (Some(Value::Named("window")), Some("width window")), "and back down to it");
        press(&mut nav, &[Key::Char('[')]);
        assert_eq!(width(&nav), (Some(Value::Number(80)), Some("width 80")), "from window, down is 80");
        press(&mut nav, &[Key::Char(']')]);
        assert_eq!(width(&nav).0, Some(Value::Named("window")), "and back up to it");
        press(&mut nav, &[Key::Char('2'), Key::Char('0'), Key::Char(']'), Key::Char('['), Key::Char('[')]);
        assert_eq!(width(&nav), (Some(Value::Number(20)), Some("width 20, the least")));
        press(&mut nav, &[Key::Char('7'), Key::Char('3'), Key::Char(']')]);
        assert_eq!(width(&nav), (Some(Value::Number(73)), Some("width 73")));
        press(&mut nav, &[Key::Char(']')]);
        assert_eq!(width(&nav), (Some(Value::Number(76)), Some("width 76")), "73 up: the next multiple of 4");
        press(&mut nav, &[Key::Char('7'), Key::Char('3'), Key::Char(']'), Key::Char('[')]);
        assert_eq!(width(&nav), (Some(Value::Number(72)), Some("width 72")), "73 down");
        press(&mut nav, &[Key::Char('[')]);
        assert_eq!(width(&nav).0, Some(Value::Number(68)), "from a multiple, the one before");
        press(&mut nav, &[Key::Char('8'), Key::Char('3'), Key::Char(']'), Key::Char(']')]);
        assert_eq!(width(&nav).0, Some(Value::Named("window")), "83 typed: the number comes before the value set at it");
        press(&mut nav, &[Key::Char('8'), Key::Char('3'), Key::Char(']'), Key::Char('[')]);
        assert_eq!(width(&nav).0, Some(Value::Number(80)));
        press(&mut nav, &[Key::Char('1'), Key::Char('9'), Key::Char('8'), Key::Char(']')]);
        press(&mut nav, &[Key::Char(']')]);
        assert_eq!(width(&nav), (Some(Value::Number(200)), Some("width 200")), "198 up: the most, not past it");
        press(&mut nav, &[Key::Char(']')]);
        assert_eq!(width(&nav), (Some(Value::Number(200)), Some("width 200, the most")));
        assert!(!nav.wants_rerender());
    }

    /// Where a named value sits among the steps: after the number equal to it, in the order declared;
    /// one set at no number steps to the least going up and the most going down; a value set past the
    /// ends stays, saying so.
    #[test]
    fn named_values_sit_among_the_steps_where_they_are_set() {
        let numbers = Numbers { least: 20, most: 200, step: 4, unit: "" };
        let named = |count: usize| -> Vec<Named> { ["a", "b", "c"][..count].iter().map(|name| Named { name, says: "" }).collect() };
        let step = |named: &[Named], at: Option<u32>, now: Value, up: bool| stepped(named, at, numbers, now, up);
        let (a, b, c) = (Value::Named("a"), Value::Named("b"), Value::Named("c"));
        let (n, up, down) = (Value::Number, true, false);
        // Set at a step, 84: 80, 84, a, 88.
        let one = named(1);
        assert_eq!(
            (step(&one, Some(84), n(84), up), step(&one, Some(84), a, up), step(&one, Some(84), a, down), step(&one, Some(84), n(88), down)),
            (Some(a), Some(n(88)), Some(n(84)), Some(a))
        );
        assert_eq!(
            (step(&one, Some(84), n(80), up), step(&one, Some(84), n(85), up), step(&one, Some(84), n(85), down)),
            (Some(n(84)), Some(n(88)), Some(a))
        );
        // Between steps, 83: 80, a, 84.
        assert_eq!(
            (step(&one, Some(83), n(80), up), step(&one, Some(83), a, up), step(&one, Some(83), n(84), down)),
            (Some(a), Some(n(84)), Some(a))
        );
        assert_eq!(step(&one, Some(83), a, down), Some(n(80)));
        // Three at one number, in their order; set nowhere, the ends.
        let three = named(3);
        assert_eq!(
            (step(&three, Some(100), n(100), up), step(&three, Some(100), a, up), step(&three, Some(100), c, up), step(&three, Some(100), b, down)),
            (Some(a), Some(b), Some(n(104)), Some(a))
        );
        assert_eq!(
            (step(&three, Some(100), a, down), step(&three, None, c, up), step(&three, None, c, down)),
            (Some(n(100)), Some(n(20)), Some(n(200)))
        );
        // At the ends: past the most, nothing more; under the least, nothing less.
        assert_eq!(
            (step(&one, Some(200), n(200), up), step(&one, Some(200), a, up), step(&one, Some(200), n(196), up)),
            (Some(a), None, Some(n(200)))
        );
        assert_eq!(
            (step(&one, Some(20), a, down), step(&one, Some(20), n(20), down), step(&one, Some(20), n(24), down)),
            (Some(n(20)), None, Some(a))
        );
        let setting = Setting { named: &[Named { name: "top", says: "at the top" }], ..WIDTH };
        let at = |at| Some(Bounds { named_at: Some(at), most: None });
        assert_eq!(turned(&setting, at(200), Value::Named("top"), ']', None), (Value::Named("top"), "at the top, the most".into()));
        assert_eq!(turned(&setting, at(200), Value::Number(200), ']', None), (Value::Named("top"), "at the top".into()));
        assert_eq!(turned(&setting, None, Value::Named("top"), ']', None), (Value::Number(20), "width 20".into()), "set nowhere");
    }

    /// The room bounds the steps: the most a width is laid out at there is the last step (`window`
    /// after it, where it sits there), and `]` from it says so, `the most here`. A number past it,
    /// typed, is kept and said with what it is laid out at, `width 120 (window)`; `[` from it goes to
    /// the last step under it. `0]` and `0[` go where the setting says a count of 0 goes.
    #[test]
    fn the_room_bounds_the_steps_and_a_number_past_its_most_is_kept() {
        // A window 100 columns wide: 96 characters a line, `window` set at 96.
        let bounds = Bounds { named_at: Some(96), most: Some(96) };
        let mut nav = declaring_in(reader(0, 12, 40, 40), &["width"], bounds);
        press(&mut nav, &[Key::Char('9'), Key::Char('2'), Key::Char(']'), Key::Char(']')]);
        assert_eq!(width(&nav), (Some(Value::Number(96)), Some("width 96")));
        press(&mut nav, &[Key::Char(']')]);
        assert_eq!(width(&nav), (Some(Value::Named("window")), Some("width window")), "set at the most, after it");
        press(&mut nav, &[Key::Char(']')]);
        assert_eq!(width(&nav), (Some(Value::Named("window")), Some("width window, the most here")));
        press(&mut nav, &[Key::Char('['), Key::Char('[')]);
        assert_eq!(width(&nav), (Some(Value::Number(92)), Some("width 92")));
        // Typed past the most here: kept, laid out at the window's.
        press(&mut nav, &[Key::Char('1'), Key::Char('2'), Key::Char('0'), Key::Char(']')]);
        assert_eq!(width(&nav), (Some(Value::Number(120)), Some("width 120 (window)")));
        press(&mut nav, &[Key::Char(']')]);
        assert_eq!(width(&nav), (Some(Value::Number(120)), Some("width 120 (window), the most here")), "kept");
        press(&mut nav, &[Key::Char('[')]);
        assert_eq!(width(&nav), (Some(Value::Named("window")), Some("width window")), "the last step under it");
        press(&mut nav, &[Key::Char('3'), Key::Char('0'), Key::Char('0'), Key::Char(']')]);
        assert_eq!(width(&nav), (Some(Value::Number(200)), Some("width 200 (window), the most")), "the setting's own most still");
        // A book: `window` sits under the most, at the half page's characters.
        let mut book = declaring_in(reader(0, 12, 40, 40), &["width"], Bounds { named_at: Some(61), most: Some(128) });
        press(&mut book, &[Key::Char('6'), Key::Char('0'), Key::Char(']'), Key::Char(']')]);
        assert_eq!(width(&book), (Some(Value::Named("window")), Some("width window")));
        press(&mut book, &[Key::Char(']')]);
        assert_eq!(width(&book).0, Some(Value::Number(64)));
        press(&mut book, &[Key::Char('1'), Key::Char('2'), Key::Char('8'), Key::Char(']'), Key::Char(']')]);
        assert_eq!(width(&book), (Some(Value::Number(128)), Some("width 128, the most here")));
        // A count of 0: where the setting says.
        press(&mut book, &[Key::Char('0'), Key::Char(']')]);
        assert_eq!(width(&book), (Some(Value::Named("window")), Some("width window")));
        press(&mut book, &[Key::Char('4'), Key::Char('0'), Key::Char('['), Key::Char('0'), Key::Char('[')]);
        assert_eq!((width(&book).0, book.typed.as_str()), (Some(Value::Named("window")), ""));
        // A room wider than the setting's most, 302 columns at 50%: `window` sits at 296, past 200, the
        // setting's own most. The steps stop at 200, `window` after it; down from `window` is 200, never
        // a number the setting doesn't take.
        let mut vast = declaring_in(reader(0, 12, 40, 40), &["width"], Bounds { named_at: Some(296), most: Some(298) });
        press(&mut vast, &[Key::Char('[')]);
        assert_eq!(width(&vast), (Some(Value::Number(200)), Some("width 200")));
        press(&mut vast, &[Key::Char(']')]);
        assert_eq!(width(&vast), (Some(Value::Named("window")), Some("width window")));
        press(&mut vast, &[Key::Char(']')]);
        assert_eq!(width(&vast), (Some(Value::Named("window")), Some("width window, the most")));
        press(&mut vast, &[Key::Char('1'), Key::Char('9'), Key::Char('6'), Key::Char(']'), Key::Char(']')]);
        assert_eq!(width(&vast).0, Some(Value::Number(200)));
        // A tiny room, its most under the least: the least is the most here.
        let mut tiny = declaring_in(reader(0, 12, 40, 40), &["width"], Bounds { named_at: Some(20), most: Some(20) });
        press(&mut tiny, &[Key::Char('4'), Key::Char('0'), Key::Char(']'), Key::Char('[')]);
        assert_eq!(width(&tiny), (Some(Value::Named("window")), Some("width window")));
        press(&mut tiny, &[Key::Char('[')]);
        assert_eq!(width(&tiny), (Some(Value::Number(20)), Some("width 20")));
        press(&mut tiny, &[Key::Char('[')]);
        assert_eq!(width(&tiny), (Some(Value::Number(20)), Some("width 20, the least")));
        // Without a named value, a number past the most says the most.
        let bare = Setting { named: &[], ..WIDTH };
        assert_eq!(said_in(&bare, Value::Number(120), Some(bounds)), "width 120 (96)");
        assert_eq!(said_in(&WIDTH, Value::Number(96), Some(bounds)), "width 96");
        assert_eq!(said_in(&WIDTH, Value::Number(120), None), "width 120");
    }

    /// A relayout setting's keys change nothing in a list, or while `--pages` picks the pages, and say
    /// why; on a document they change the value without asking for a redraw, so `]` then `u`, read
    /// together, leaves the list as it is, the value set for the next document gone into.
    #[test]
    fn a_relayout_key_is_refused_in_a_list_and_with_pages_and_asks_for_no_redraw() {
        let mut tiles = declaring_both(list(9), &["width"]);
        press(&mut tiles, &[Key::Char('7'), Key::Char('2'), Key::Char(']')]);
        assert_eq!((width(&tiles), tiles.typed.as_str()), ((None, Some(REFLOW_IN_LIST)), ""));
        assert!(!tiles.wants_rerender());
        let mut none_has_it = declaring_both(list(9), &[]);
        press(&mut none_has_it, &[Key::Char(']')]);
        assert_eq!(width(&none_has_it), (None, Some("only some are as wide")), "the setting's own message first");
        let mut picked = Nav { picked: true, ..declaring_both(reader(0, 12, 40, 40), &["width"]) };
        press(&mut picked, &[Key::Char(']')]);
        assert_eq!(width(&picked), (None, Some(REFLOW_WITH_PAGES)));
        assert!(!picked.wants_rerender());
        let mut opened = Nav { nested: true, settings: tiles.settings.clone(), ..declaring_both(reader(0, 3, 40, 40), &["width"]) };
        assert_eq!(press(&mut opened, &[Key::Char(']'), Key::Char('u')]), Step::Back);
        assert_eq!(width(&opened).0, Some(Value::Number(84)));
        drop(opened);
        assert!(!tiles.wants_rerender(), "the list's tiles keep their layout");
        // A redraw setting still asks for one.
        let mut both = declaring_both(reader(0, 12, 40, 40), &["shade", "width"]);
        press(&mut both, &[Key::Char('d')]);
        assert!(both.wants_rerender());
    }

    /// A setting on the zoom keys the tests declare, as the documents' font size is: 50 to 400 in steps
    /// of 10, said with `%`; a relayout setting.
    const FONT: Setting = Setting {
        name: "font",
        named: &[],
        numbers: Some(Numbers { least: 50, most: 400, step: 10, unit: "%" }),
        default: Value::Number(100),
        zero: Value::Number(100),
        keys: Keys::Zoom,
        help: "larger or smaller text",
        missing: "only some have a font size",
        effect: Effect::Relayout,
        ..SHADE
    };

    /// `nav` in a build that declares [`SHADE`], [`WIDTH`] and [`FONT`], showing what has `has` of them,
    /// the font's most in the room `most`.
    fn declaring_font(nav: Nav, has: &[&'static str], most: Option<u32>) -> Nav {
        let settings = Rc::new(Settings { declared: vec![&SHADE, &WIDTH, &FONT], values: std::cell::Cell::default(), changed: Default::default() });
        let bounds = if has.contains(&FONT.name) { vec![(FONT.name, Bounds { named_at: None, most })] } else { Vec::new() };
        Nav { settings, has: has.to_vec(), bounds, ..nav }
    }

    /// The font `nav` has, and what the status line says.
    fn font(nav: &Nav) -> (Option<Value>, Option<&str>) {
        (nav.settings.values.get().get(FONT.name), nav.message.as_deref())
    }

    /// On a document that has a setting on the zoom keys, `+`, `=`, `-` and `_` step it, said with its
    /// unit, a count sets it (`0+` back to 100) and the room's most holds it (`the most here`); the zoom
    /// doesn't move. In a list, on what hasn't the setting and with `--pages`, they are the zoom.
    #[test]
    fn the_zoom_keys_are_a_documents_font_size_where_it_has_one_and_the_zoom_elsewhere() {
        let mut nav = declaring_font(reader(0, 12, 40, 40), &["font"], Some(170));
        press(&mut nav, &[Key::Char('+')]);
        assert_eq!((font(&nav), nav.zoom), ((Some(Value::Number(110)), Some("font 110%")), Zoom::Auto));
        assert!(!nav.wants_rerender(), "laid out again, not drawn again");
        press(&mut nav, &[Key::Char('='), Key::Char('_')]);
        assert_eq!(font(&nav), (Some(Value::Number(110)), Some("font 110%")));
        press(&mut nav, &[Key::Char('1'), Key::Char('5'), Key::Char('0'), Key::Char('+')]);
        assert_eq!(font(&nav), (Some(Value::Number(150)), Some("font 150%")));
        press(&mut nav, &[Key::Char('+'), Key::Char('+')]);
        assert_eq!(font(&nav), (Some(Value::Number(170)), Some("font 170%")));
        press(&mut nav, &[Key::Char('+')]);
        assert_eq!(font(&nav), (Some(Value::Number(170)), Some("font 170%, the most here")));
        press(&mut nav, &[Key::Char('3'), Key::Char('0'), Key::Char('0'), Key::Char('-')]);
        assert_eq!(font(&nav), (Some(Value::Number(300)), Some("font 300% (170%)")), "typed past the most here: kept");
        press(&mut nav, &[Key::Char('0'), Key::Char('+')]);
        assert_eq!(font(&nav), (Some(Value::Number(100)), Some("font 100%")));
        press(&mut nav, &[Key::Char('5'), Key::Char('+'), Key::Char('-')]);
        assert_eq!(font(&nav), (Some(Value::Number(50)), Some("font 50%, the least")));
        press(&mut nav, &[Key::Char('9'), Key::Char('0'), Key::Char('0'), Key::Char('+')]);
        assert_eq!(font(&nav).1, Some("font 400% (170%), the most"));
        assert_eq!(nav.zoom, Zoom::Auto, "the zoom never moved");
        // The hint and `?` say so: one `+ -` line, the font's, and no second line for it.
        nav.hint = true;
        nav.message = None;
        assert!(nav.status("a.md", 120, false).contains("+ - font ·"), "{:?}", nav.status("a.md", 120, false));
        let lines = nav.help_lines();
        let plus: Vec<&(String, &str)> = lines.iter().filter(|(keys, _)| keys.starts_with("+ -")).collect();
        assert_eq!(plus.len(), 1, "{lines:?}");
        assert_eq!((plus[0].0.as_str(), plus[0].1), ("+ -  150+", "larger or smaller text"));
        assert!(!lines.iter().any(|(_, what)| *what == PAGE_HELP[6].1), "the zoom's line is gone");
        // Elsewhere, the zoom: a list, a document without the setting, pages picked by `--pages`.
        for mut nav in [declaring_font(list(9), &["font"], Some(170)), declaring_font(reader(4, 20, 40, 40), &["shade"], None)] {
            nav.grid(2, 1, 8);
            press(&mut nav, &[Key::Char('-')]);
            assert_eq!((font(&nav).0, nav.zoom), (None, Zoom::PerRow(3)));
            assert!(nav.help_lines().iter().any(|(_, what)| *what == PAGE_HELP[6].1 || *what == LIST_HELP[7].1));
            assert!(!nav.help_lines().iter().any(|(_, what)| *what == FONT.help), "no line for the font elsewhere: {:?}", nav.help_lines());
            nav.hint = true;
            nav.message = None;
            assert!(nav.status("a.pdf", 120, false).contains("+ - zoom ·"));
        }
        let mut picked = Nav { picked: true, ..declaring_font(reader(4, 20, 40, 40), &["font"], Some(170)) };
        picked.grid(2, 1, 8);
        press(&mut picked, &[Key::Char('+')]);
        assert_eq!((font(&picked).0, picked.zoom), (None, Zoom::PerRow(1)));
        // Said with its unit on the status line, when it isn't the default.
        let mut values = Values::default();
        assert!(values.set("font", Value::Number(120)));
        let nav = declaring_font(reader(0, 3, 40, 40), &["font"], Some(170));
        assert_eq!(laid_at(&nav.settings, &LaidAt(values), &nav.has, &nav.bounds), Some("font 120%".into()));
        let mut values = Values::default();
        assert!(values.set("font", Value::Number(100)));
        assert_eq!(laid_at(&nav.settings, &LaidAt(values), &nav.has, &nav.bounds), None, "the default");
        // The default past the most here, as wide margins or a narrow column make it: said, with what it is
        // laid out at.
        let narrow = declaring_font(reader(0, 3, 40, 40), &["font"], Some(80));
        assert_eq!(laid_at(&narrow.settings, &LaidAt(values), &narrow.has, &narrow.bounds), Some("font 100% (80%)".into()));
    }

    /// A source laid out at something, for the status line's sake.
    struct LaidAt(Values);

    impl Source for LaidAt {
        fn pages(&self) -> usize {
            1
        }
        fn size(&self, _page: usize) -> Option<(f32, f32)> {
            Some((1.0, 1.0))
        }
        fn natural(&self, _page: usize) -> bool {
            false
        }
        fn renderer(&self) -> Result<Box<dyn crate::source::Render + '_>, String> {
            Err("none".into())
        }
        fn pool(&self, _threads: usize) -> Box<dyn Pool> {
            Box::new(Idle)
        }
        fn laid_at(&self) -> (Values, Option<Room>) {
            (self.0, None)
        }
    }

    /// A pool that renders nothing.
    struct Idle;

    impl Pool for Idle {
        fn set_queue(&self, _requests: Vec<Request>) {}
        fn try_recv(&self) -> Option<Frame> {
            None
        }
    }

    /// The status line says what the document is laid out at, when that isn't the default, before the
    /// zoom, with `…` after the value it is being laid out at again, and with what a number past the
    /// most here is laid out at; a narrow window drops it first.
    #[test]
    fn the_status_line_shows_what_the_document_is_laid_out_at() {
        let at = |value: Option<Value>| {
            let mut values = Values::default();
            if let Some(value) = value {
                assert!(values.set("width", value));
            }
            let nav = declaring_in(reader(0, 3, 40, 40), &["width"], Bounds { named_at: Some(96), most: Some(96) });
            laid_at(&nav.settings, &LaidAt(values), &nav.has, &nav.bounds)
        };
        assert_eq!(at(Some(Value::Number(72))), Some("width 72".into()));
        assert_eq!(at(Some(Value::Number(120))), Some("width 120 (window)".into()), "laid out at the window's");
        assert_eq!((at(Some(Value::Named("window"))), at(None)), (None, None), "the default");
        let mut nav = declaring_both(reader(0, 3, 40, 40), &["width"]);
        nav.laid = Some("width 72".into());
        nav.hint = false;
        assert_eq!(squeezed(&nav.status("a.md", 80, false)), " a.md page 1/3 · width 72 · 1 per row · ? help ");
        nav.relaying = Some("width 76".into());
        assert_eq!(squeezed(&nav.status("a.md", 80, true)), " a.md … page 1/3 · width 76… · 1 per row · ? help ");
        let padded = format!("{:<width$} · 1 per row", "width 76…", width = nav.laid_width());
        assert!(nav.laid_width() >= chars("width 200 (window)…"), "{}", nav.laid_width());
        assert!(nav.status("a.md", 80, true).contains(&padded), "padded to the widest, `width 200 (window)…`, so the right side stays put");
        nav.relaying = None;
        let (wide, narrow) = (nav.status("a.md", 70, false), nav.status("a.md", 46, false));
        assert!(wide.contains("width 72") && !narrow.contains("width 72") && narrow.contains("1 per row"), "{wide:?} {narrow:?}");
        let tiles = declaring_both(list(3), &["width"]);
        assert!(!tiles.status("shelf", 80, false).contains("width"));
    }

    /// Digits typed before a numbered setting's key set it to that number, within its range, and are
    /// used up whatever happens; before a setting without numbers they stay, and before `g` or Enter
    /// they still go to a page.
    #[test]
    fn a_count_sets_a_numbered_setting_and_is_used_up() {
        let mut nav = declaring_both(reader(0, 12, 40, 40), &["shade", "width"]);
        press(&mut nav, &[Key::Char('7'), Key::Char('2'), Key::Char(']')]);
        assert_eq!((width(&nav), nav.typed.as_str()), ((Some(Value::Number(72)), Some("width 72")), ""));
        press(&mut nav, &[Key::Char('4'), Key::Char('0'), Key::Char('[')]);
        assert_eq!(width(&nav).0, Some(Value::Number(40)), "either key sets it");
        press(&mut nav, &[Key::Char('3'), Key::Char('0'), Key::Char('0'), Key::Char(']')]);
        assert_eq!(width(&nav), (Some(Value::Number(200)), Some("width 200, the most")));
        press(&mut nav, &[Key::Char('5'), Key::Char('[')]);
        assert_eq!(width(&nav), (Some(Value::Number(20)), Some("width 20, the least")));
        press(&mut nav, &[Key::Char('9'); 12]);
        press(&mut nav, &[Key::Char(']')]);
        assert_eq!((width(&nav).0, nav.typed.as_str()), (Some(Value::Number(200)), ""), "as long as can be typed");
        // Before a setting without numbers, the count stays, and goes to a page after.
        press(&mut nav, &[Key::Char('3'), Key::Char('d')]);
        assert_eq!(nav.typed, "3");
        press(&mut nav, &[Key::Char('g')]);
        assert_eq!((page(&nav), width(&nav).0), (2, Some(Value::Number(200))));
        press(&mut nav, &[Key::Char('5'), Key::Enter]);
        assert_eq!(page(&nav), 4, "Enter too");
        // On what hasn't the setting, the count is used up all the same, and nothing changes.
        let mut nav = declaring_both(reader(0, 12, 40, 40), &["shade"]);
        press(&mut nav, &[Key::Char('7'), Key::Char('2'), Key::Char(']')]);
        assert!(!nav.wants_rerender());
        assert_eq!((width(&nav), nav.typed.as_str()), ((None, Some("only some are as wide")), ""));
        // `?` gives a count's form too.
        assert!(nav.help_lines().contains(&("[ ]  12]".into(), "narrower or wider")));
    }

    /// `?` lists the keys of the settings this build declares, after the viewer's and before the
    /// way out, whatever is shown.
    #[test]
    fn the_keys_list_the_settings_after_the_viewers() {
        for nav in [declaring(reader(0, 3, 40, 40), false), declaring(list(3), true)] {
            let lines = nav.help_lines();
            let at = |what: &str| lines.iter().position(|(_, said)| *said == what).unwrap();
            assert_eq!(lines.get(at("pale or deep")).map(|(keys, _)| keys.as_str()), Some("d"));
            assert!(at("pale or deep") > lines.iter().position(|(keys, _)| keys.starts_with("r  Ctrl-L")).unwrap());
            assert!(at("pale or deep") < at("quit"));
        }
        assert!(!reader(0, 3, 40, 40).help_lines().iter().any(|(_, said)| *said == "pale or deep"));
    }

    /// The viewer's keys are those [`VIEWER_KEYS`] lists: any other character does nothing to where
    /// the reader is, in a file's pages or in a list, so a setting may have it.
    #[test]
    fn the_viewers_keys_are_those_listed() {
        // Each somewhere it can move: a page and a tile in the middle.
        let tiles = |count, per_row, rows| {
            let mut nav = tiles(count, per_row, rows);
            nav.select(4);
            nav
        };
        for c in (' '..='~').chain(['\u{c}']).filter(|c| !VIEWER_KEYS.contains(*c)) {
            for nav in [reader(1, 12, 20, 40), tiles(10, 3, 2)] {
                let before = format!("{nav:?}");
                let mut nav = nav;
                assert_eq!(nav.key(Key::Char(c)), Step::Continue, "{c:?}");
                nav.hint = true;
                assert_eq!(format!("{nav:?}"), before, "{c:?}");
            }
        }
        // And each listed is taken by one or the other.
        for c in VIEWER_KEYS.chars() {
            let moved = [reader(1, 12, 20, 40), tiles(10, 3, 2)].into_iter().any(|nav| {
                let before = format!("{nav:?}");
                let mut nav = nav;
                let step = nav.key(Key::Char(c));
                nav.hint = true;
                step != Step::Continue || format!("{nav:?}") != before
            });
            assert!(moved, "{c:?}");
        }
    }

    /// `keys` pressed: what the first that asks more of the pager than a move asks.
    fn press(nav: &mut Nav, keys: &[Key]) -> Step {
        keys.iter().map(|k| nav.key(*k)).fold(Step::Continue, |first, step| if first == Step::Continue { step } else { first })
    }

    #[test]
    fn space_turns_pages_that_fit_and_stops_at_the_last() {
        let mut nav = reader(0, 3, 40, 40);
        press(&mut nav, &[Key::Char(' ')]);
        assert_eq!((page(&nav), nav.scroll), (1, 0));
        press(&mut nav, &[Key::PageDown, Key::Char(' '), Key::Char(' ')]);
        assert_eq!(page(&nav), 2);
        press(&mut nav, &[Key::Char('b')]);
        assert_eq!(page(&nav), 1);
    }

    /// The place a relayout anchors on is the page the reader is on now, not the one last drawn: a jump
    /// read in the pass the new layout arrives counts. The last draw's rows give how far down the page
    /// only when they are this page's, of this level.
    #[test]
    fn a_relayout_anchors_on_the_page_the_reader_is_on_now_not_the_one_last_drawn() {
        let view = |level, page| View { level, page, fit: Fit::Page, cols: 10, rows: 20, size: FALLBACK_SIZE };
        let height = |_page: usize| 1000.0;
        // Drawn at page 1 of level 3, scrolled 25 of 100 rows; then `9g`, before the draw.
        let mut nav = reader(0, 20, 30, 100);
        nav.scroll = 25;
        assert_eq!(place(&nav, 3, Some((view(3, 0), 100)), height), Some((0, 250.0)), "as drawn");
        nav.go(8);
        assert_eq!(place(&nav, 3, Some((view(3, 0), 100)), height), Some((8, 0.0)), "the jump counts, at its top");
        nav.go(0);
        nav.scroll = 25;
        assert_eq!(place(&nav, 3, Some((view(4, 0), 100)), height), Some((0, 0.0)), "another level's rows don't count");
        assert_eq!(place(&nav, 3, Some((view(3, 0), 0)), height), Some((0, 0.0)), "nor none");
        assert_eq!(place(&nav, 3, None, height), Some((0, 0.0)));
        nav.scroll = 300;
        assert_eq!(place(&nav, 3, Some((view(3, 0), 100)), height), Some((0, 1000.0)), "within the page");
        assert_eq!(place(&list(0), 3, None, height), None, "nothing on screen");
    }

    /// Where to scroll to is a share of the page, taken once the draw knows the rows, and kept in range.
    #[test]
    fn a_scroll_asked_for_as_a_share_of_the_page_is_taken_once_the_rows_are_known() {
        let mut nav = reader(0, 2, 30, 100);
        nav.scroll_to = Some(0.5);
        nav.shown(30, 100);
        assert_eq!((nav.scroll, nav.scroll_to), (50, None));
        nav.shown(30, 100);
        assert_eq!(nav.scroll, 50, "once");
        nav.scroll_to = Some(0.9);
        nav.shown(30, 100);
        assert_eq!(nav.scroll, 70, "within the rows the page has");
        nav.scroll_to = Some(0.5);
        nav.shown(40, 40);
        assert_eq!(nav.scroll, 0, "a page that fits doesn't scroll");
        nav.scroll_to = Some(f32::NAN);
        nav.shown(30, 100);
        assert_eq!(nav.scroll, 0);
    }

    #[test]
    fn a_tall_page_scrolls_before_the_next_one() {
        // 30 of 100 rows on screen: space scrolls 30, 30, then the last 10, then turns the page.
        let mut nav = reader(0, 2, 30, 100);
        for expected in [30, 60, 70] {
            press(&mut nav, &[Key::Char(' ')]);
            assert_eq!((page(&nav), nav.scroll), (0, expected));
        }
        press(&mut nav, &[Key::Char(' ')]);
        assert_eq!((page(&nav), nav.scroll), (1, 0));
        // Back from the top of page 2 lands on the bottom of page 1, once the draw clamps it.
        press(&mut nav, &[Key::Char('b')]);
        assert_eq!(page(&nav), 0);
        nav.shown(30, 100);
        assert_eq!(nav.scroll, 70);
        press(&mut nav, &[Key::Char('k')]);
        assert_eq!(nav.scroll, 70 - STEP);
        press(&mut nav, &[Key::Char('j')]);
        assert_eq!((page(&nav), nav.scroll), (0, 70));
        // At the bottom, a row more is the next page.
        press(&mut nav, &[Key::Down]);
        assert_eq!((page(&nav), nav.scroll), (1, 0));
        nav.shown(30, 100);
        press(&mut nav, &[Key::Enter]);
        assert_eq!((page(&nav), nav.scroll), (1, STEP));
    }

    #[test]
    fn page_keys_jump() {
        let mut nav = reader(4, 10, 40, 40);
        press(&mut nav, &[Key::Char('n')]);
        assert_eq!(page(&nav), 5);
        press(&mut nav, &[Key::Left, Key::Char('p')]);
        assert_eq!(page(&nav), 3);
        press(&mut nav, &[Key::Char('G')]);
        assert_eq!(page(&nav), 9);
        press(&mut nav, &[Key::Right]);
        assert_eq!(page(&nav), 9, "past the last page stays on it");
        press(&mut nav, &[Key::Home]);
        assert_eq!(page(&nav), 0);
        press(&mut nav, &[Key::Left]);
        assert_eq!(page(&nav), 0);
    }

    #[test]
    fn two_pages_side_by_side_turn_like_a_book() {
        let mut nav = reader(0, 5, 40, 40);
        nav.grid(2, 1, 8);
        assert_eq!(nav.on_screen(), [Some(0), Some(1)]);
        for (keys, expected) in [
            (&[Key::Char(' ')][..], [Some(2), Some(3)]),
            (&[Key::Char('n')], [Some(4), None]),
            (&[Key::Char('n')], [Some(4), None]),
            (&[Key::Char('b')], [Some(2), Some(3)]),
            (&[Key::Char('p')], [Some(0), Some(1)]),
            (&[Key::Char('p')], [Some(0), Some(1)]),
            (&[Key::Char('G')], [Some(4), None]),
        ] {
            press(&mut nav, keys);
            assert_eq!(nav.on_screen(), expected, "{keys:?}");
        }
        // A typed page shows the spread it is on: page 4 is the right page of 3–4.
        press(&mut nav, &[Key::Char('4'), Key::Enter]);
        assert_eq!(nav.on_screen(), [Some(2), Some(3)]);
        // Zoomed in, one to a screen from where the reader is, at once: a key before the next
        // draw steps by one page.
        press(&mut nav, &[Key::Char('+')]);
        assert_eq!(nav.zoom, Zoom::PerRow(1));
        assert_eq!(nav.on_screen(), [Some(3)]);
        press(&mut nav, &[Key::Char('p')]);
        assert_eq!(nav.on_screen(), [Some(2)]);
        // As wide as the window, one page too.
        press(&mut nav, &[Key::Char('+'), Key::Char('n')]);
        assert_eq!((nav.zoom, nav.on_screen()), (Zoom::Width, vec![Some(3)]));
    }

    /// `,` and `.` set pages to a row as `-` and `+` zoom: one more, one fewer, then the width, as far
    /// as the window takes; a count sets that many, clamped to the most the window takes, and `0.`
    /// goes back to `auto`. On a document with a font size they are still the pages to a row.
    #[test]
    fn comma_and_dot_set_pages_to_a_row_as_the_zoom_does() {
        let mut nav = reader(4, 20, 40, 40);
        let chars = |text: &str| text.chars().map(Key::Char).collect::<Vec<_>>();
        press(&mut nav, &chars(",,"));
        assert_eq!((nav.zoom, nav.on_screen()), (Zoom::PerRow(3), vec![Some(3), Some(4), Some(5)]));
        assert!(squeezed(&nav.status("a.pdf", 120, false)).contains(" pages 4–6/20 · 3 per row "));
        // A window that takes four to a row stops there, and says so; a count past it is clamped.
        nav.grid(3, 1, 4);
        press(&mut nav, &chars(",,"));
        assert_eq!((nav.zoom, nav.message.as_deref()), (Zoom::PerRow(4), Some("fully zoomed out")));
        press(&mut nav, &chars("7."));
        assert_eq!((nav.zoom, nav.message.as_deref(), nav.typed.as_str()), (Zoom::PerRow(4), Some("fully zoomed out"), ""));
        press(&mut nav, &chars("999999999,"));
        assert_eq!(nav.zoom, Zoom::PerRow(4));
        press(&mut nav, &chars("2,"));
        assert_eq!((nav.zoom, nav.message.as_deref()), (Zoom::PerRow(2), None));
        // In: one fewer, then the width, and no further.
        press(&mut nav, &chars(".."));
        assert_eq!(nav.zoom, Zoom::Width);
        press(&mut nav, &chars("."));
        assert_eq!((nav.zoom, nav.message.as_deref()), (Zoom::Width, Some("fully zoomed in")));
        press(&mut nav, &chars(","));
        assert_eq!(nav.zoom, Zoom::PerRow(1));
        // `0.` and `0,`: back to auto.
        press(&mut nav, &chars("3.0."));
        assert_eq!(nav.zoom, Zoom::Auto);
        press(&mut nav, &chars("3,0,"));
        assert_eq!(nav.zoom, Zoom::Auto);
        // From auto, from what was drawn: a book of two.
        let mut auto = reader(0, 20, 40, 40);
        auto.grid(2, 1, 8);
        press(&mut auto, &chars(","));
        assert_eq!(auto.zoom, Zoom::PerRow(3));
        let mut auto = reader(0, 20, 40, 40);
        auto.grid(2, 1, 8);
        press(&mut auto, &chars("."));
        assert_eq!(auto.zoom, Zoom::PerRow(1));
        // A document whose `+` and `-` are its font size: `,` and `.` are still the pages to a row.
        let mut doc = declaring_font(reader(0, 20, 40, 40), &["font"], Some(170));
        press(&mut doc, &chars("3."));
        assert_eq!(doc.zoom, Zoom::PerRow(3));
        // `?` lists them, in the pages and in a list.
        assert!(doc.help_lines().iter().any(|(keys, _)| keys == ", .  3."));
        assert!(tiles(10, 3, 2).help_lines().iter().any(|(keys, what)| keys == ", .  3." && what.contains("tile")));
    }

    #[test]
    fn plus_and_minus_zoom_between_the_width_and_the_most_pages_to_a_row() {
        let mut nav = reader(4, 20, 40, 40);
        // Out, a page more to a row each time, from the reader's row: page 5 stays on screen.
        press(&mut nav, &[Key::Char('-')]);
        assert_eq!((nav.zoom, nav.on_screen()), (Zoom::PerRow(2), vec![Some(4), Some(5)]));
        press(&mut nav, &[Key::Char('-')]);
        assert_eq!((nav.zoom, nav.on_screen()), (Zoom::PerRow(3), vec![Some(3), Some(4), Some(5)]));
        assert!(squeezed(&nav.status("a.pdf", 120, false)).contains(" pages 4–6/20 · 3 per row "));
        // Keys before the next draw step by the new row.
        press(&mut nav, &[Key::Char('n')]);
        assert_eq!(nav.on_screen(), [Some(6), Some(7), Some(8)]);
        // `_` is `-` with Shift. A window that takes four to a row stops there, and says so.
        press(&mut nav, &[Key::Char('_')]);
        nav.grid(4, 1, 4);
        press(&mut nav, &[Key::Char('-')]);
        assert_eq!((nav.zoom, nav.message.as_deref()), (Zoom::PerRow(4), Some("fully zoomed out")));
        // In, a page fewer each time (`=` is `+` without Shift), then the width, and no further.
        press(&mut nav, &[Key::Char('+'), Key::Char('='), Key::Char('+')]);
        assert_eq!(nav.zoom, Zoom::PerRow(1));
        press(&mut nav, &[Key::Char('+')]);
        assert_eq!(nav.zoom, Zoom::Width);
        assert!(squeezed(&nav.status("a.pdf", 120, false)).contains(" page 7/20 · width "));
        press(&mut nav, &[Key::Char('+')]);
        assert_eq!((nav.zoom, nav.message.as_deref()), (Zoom::Width, Some("fully zoomed in")));
        press(&mut nav, &[Key::Char('-')]);
        assert_eq!(nav.zoom, Zoom::PerRow(1));
        // From Auto, a step from what the last draw showed.
        let mut auto = reader(0, 20, 40, 40);
        auto.grid(2, 1, 8);
        press(&mut auto, &[Key::Char('-')]);
        assert_eq!(auto.zoom, Zoom::PerRow(3));
        let mut auto = reader(0, 20, 40, 40);
        auto.grid(2, 1, 8);
        press(&mut auto, &[Key::Char('+')]);
        assert_eq!(auto.zoom, Zoom::PerRow(1));
    }

    #[test]
    fn a_grid_turns_a_screen_or_a_row_and_ends_with_the_last_row_at_the_bottom() {
        // Ten pages, three to a row in two rows.
        let mut nav = Nav::new((0..10).collect(), 10, Zoom::PerRow(3));
        nav.grid(3, 2, 8);
        nav.shown(20, 20);
        let screen = |first: usize| (first..first + 6).map(|p| (p < 10).then_some(p)).collect::<Vec<_>>();
        assert_eq!(nav.on_screen(), screen(0));
        assert!(squeezed(&nav.status("a.pdf", 120, false)).contains(" pages 1–6/10 · 3 per row "));
        assert_eq!(nav.around(2, 1), [6, 7, 8, 9]);
        // A screen on is the last row and the one above: the last row stays at the bottom.
        for (key, first) in [(Key::Char(' '), 6), (Key::Char('b'), 0), (Key::Char('j'), 3), (Key::Down, 6), (Key::Char('j'), 6), (Key::Char('k'), 3)]
        {
            press(&mut nav, &[key]);
            assert_eq!(nav.on_screen(), screen(first), "{key:?}");
        }
        for (keys, first) in [(&[Key::Char('G')][..], 6), (&[Key::Char('p')], 0), (&[Key::Char('n')], 6), (&[Key::Home], 0)] {
            press(&mut nav, keys);
            assert_eq!(nav.on_screen(), screen(first), "{keys:?}");
        }
        // A typed page shows the row it is on, at the top when it can be.
        press(&mut nav, &[Key::Char('5'), Key::Enter]);
        assert_eq!(nav.on_screen(), screen(3));
        press(&mut nav, &[Key::Char('8'), Key::Enter]);
        assert_eq!(nav.on_screen(), screen(6));
    }

    #[test]
    fn the_pages_worked_on_ahead_are_the_next_screens_then_the_one_before() {
        let mut nav = reader(4, 10, 40, 40);
        assert_eq!(nav.around(2, 1), [5, 6, 3]);
        // Two to a screen, 4–5 on it.
        nav.grid(2, 1, 8);
        assert_eq!(nav.around(2, 1), [6, 7, 8, 9, 2, 3]);
        // At the ends, only what exists.
        assert_eq!(reader(9, 10, 40, 40).around(2, 1), [8]);
        assert!(reader(0, 1, 40, 40).around(2, 1).is_empty());
        // However many screens are asked for, the ends stop it.
        assert!(Nav::new(Vec::new(), 0, Zoom::Auto).around(usize::MAX, usize::MAX).is_empty());
        assert_eq!(reader(1, 3, 40, 40).around(usize::MAX, usize::MAX), [2, 0]);
    }

    /// A list of `count` tiles, all of them.
    fn list(count: usize) -> Nav {
        Nav { list: true, ..Nav::new((0..count).collect(), count, Zoom::Auto) }
    }

    /// A list of `count` tiles, `per_row` to a row in `rows` rows, after a draw.
    fn tiles(count: usize, per_row: u16, rows: u16) -> Nav {
        let mut nav = list(count);
        nav.grid(per_row, rows, 8);
        nav.shown(10, 10);
        nav
    }

    /// The tiles on screen, from the first.
    fn screen(nav: &Nav) -> (usize, usize) {
        let on_screen: Vec<usize> = nav.on_screen().into_iter().flatten().collect();
        (on_screen[0], on_screen.len())
    }

    #[test]
    fn in_a_list_the_arrows_go_from_tile_to_tile_and_the_screen_follows_only_when_it_must() {
        // Ten tiles, three to a row in two rows: six on screen.
        let mut nav = tiles(10, 3, 2);
        for (keys, at, first) in [
            (&[Key::Right, Key::Char('l')][..], 2, 0),
            (&[Key::Down], 5, 0),
            // Off the bottom: the screen moves a row, no more.
            (&[Key::Char('j')], 8, 3),
            // Back up within the screen: it stays.
            (&[Key::Up], 5, 3),
            (&[Key::Char('k')], 2, 0),
            (&[Key::Left, Key::Char('h'), Key::Left, Key::Left], 0, 0),
            // Below the last row's tiles, the last tile; then nowhere further.
            (&[Key::Down, Key::Down, Key::Right], 7, 3),
            (&[Key::Down], 9, 6),
            (&[Key::Down, Key::Right, Key::End], 9, 6),
            (&[Key::Home], 0, 0),
            // A screen on and back: the tile chosen with it.
            (&[Key::Right, Key::Char(' ')], 7, 6),
            (&[Key::Char('b')], 1, 0),
            (&[Key::Char('G'), Key::Char('p')], 3, 0),
            (&[Key::Char('n')], 9, 6),
        ] {
            assert_eq!(press(&mut nav, keys), Step::Continue, "{keys:?}");
            assert_eq!((nav.at, screen(&nav).0), (at, first), "{keys:?}");
        }
        // The last row stays at the bottom: four tiles on the last screen.
        assert_eq!(screen(&nav), (6, 4));
    }

    #[test]
    fn enter_opens_the_tile_chosen_and_a_number_chooses_one() {
        let mut nav = tiles(10, 3, 2);
        press(&mut nav, &[Key::Right]);
        assert_eq!(press(&mut nav, &[Key::Enter]), Step::Open(1));
        // A number then Enter chooses that tile, without opening it, the screen moving as little
        // as brings it on.
        assert_eq!(press(&mut nav, &[Key::Char('9'), Key::Enter]), Step::Continue);
        assert_eq!((nav.at, screen(&nav).0), (8, 3));
        assert_eq!(press(&mut nav, &[Key::Char('9'), Key::Char('9'), Key::Enter]), Step::Continue);
        assert_eq!((nav.at, nav.message.as_deref()), (8, Some("no tile 99 (1–10)")));
        assert_eq!((press(&mut nav, &[Key::Escape]), press(&mut nav, &[Key::Backspace])), (Step::Leave, Step::Back));
        assert_eq!(press(&mut nav, &[Key::Char('u')]), Step::Back);
        // An empty list opens nothing.
        assert_eq!(press(&mut list(0), &[Key::Enter]), Step::Continue);
    }

    /// Tiles given are a list's as pages given are a document's: they keep their numbers, a number
    /// typed is one of them, and Enter opens the tile chosen.
    #[test]
    fn tiles_given_keep_the_lists_numbers() {
        let mut nav = Nav { list: true, ..Nav::new(vec![6, 2, 9], 12, Zoom::Auto) };
        nav.grid(4, 1, 8);
        assert!(squeezed(&nav.status("", 120, false)).contains(" 7/12 · 1 of 3 · 4 per row "));
        press(&mut nav, &[Key::Right]);
        assert!(squeezed(&nav.status("", 120, false)).contains(" 3/12 · 2 of 3 · 4 per row "));
        assert_eq!(press(&mut nav, &[Key::Enter]), Step::Open(2));
        press(&mut nav, &[Key::Char('1'), Key::Char('0'), Key::Enter]);
        assert_eq!((nav.at, nav.message.as_deref()), (2, None));
        // 2 is a tile of the list, not the second of those given.
        press(&mut nav, &[Key::Char('2'), Key::Enter]);
        assert_eq!((nav.at, nav.message.as_deref()), (2, Some("tile 2 isn't one of the tiles given")));
        press(&mut nav, &[Key::Char('1'), Key::Char('3'), Key::Enter]);
        assert_eq!(nav.message.as_deref(), Some("no tile 13 (1–12)"));
    }

    #[test]
    fn a_list_zooms_between_one_tile_to_a_row_and_the_most_the_window_takes() {
        let mut nav = tiles(10, 3, 2);
        press(&mut nav, &[Key::Char('+')]);
        assert_eq!(nav.zoom, Zoom::PerRow(2));
        press(&mut nav, &[Key::Char('+'), Key::Char('+')]);
        assert_eq!((nav.zoom, nav.message.as_deref()), (Zoom::PerRow(1), Some("fully zoomed in")));
        press(&mut nav, &[Key::Char('-'), Key::Char('_')]);
        assert_eq!(nav.zoom, Zoom::PerRow(3));
    }

    /// In a list `,` and `.` are its `-` and `+`: `.` stops at one tile to a row, never the width, and a
    /// count sets that many tiles, as far as the window takes; `+` and `-` take no count.
    #[test]
    fn a_lists_comma_and_dot_set_tiles_to_a_row_down_to_one() {
        let chars = |text: &str| text.chars().map(Key::Char).collect::<Vec<_>>();
        let mut nav = tiles(10, 3, 2);
        press(&mut nav, &chars(".."));
        assert_eq!(nav.zoom, Zoom::PerRow(1));
        press(&mut nav, &chars("."));
        assert_eq!((nav.zoom, nav.message.as_deref()), (Zoom::PerRow(1), Some("fully zoomed in")));
        press(&mut nav, &chars(",,"));
        assert_eq!(nav.zoom, Zoom::PerRow(3));
        nav.grid(3, 2, 5);
        press(&mut nav, &chars("9,"));
        assert_eq!((nav.zoom, nav.message.as_deref()), (Zoom::PerRow(5), Some("fully zoomed out")));
        press(&mut nav, &chars("0."));
        assert_eq!(nav.zoom, Zoom::Auto);
        // A number before `+` stays a tile's number, as it was.
        press(&mut nav, &chars("4+"));
        assert_eq!(nav.typed, "4");
    }

    #[test]
    fn a_lists_status_line_says_which_tile_of_how_many() {
        let mut nav = tiles(10, 3, 2);
        let first = nav.status("a.png", 100, false);
        assert!(first.contains(&format!("  {}  ", LIST_KEYS[0])) && first.ends_with("  1/10  · 3 per row · ? help "), "{first:?}");
        press(&mut nav, &[Key::End]);
        assert!(nav.status("z.png", 100, false).ends_with("  10/10 · 3 per row · ? help "));
        press(&mut nav, &[Key::Char('4')]);
        assert_eq!(nav.status("z.png", 100, false), format!("{:<100}", " 4: Enter tile · . per row"));
        // Opened from a list, a file's pages remind of the way back, and its keys list it.
        let opened = Nav { nested: true, ..reader(0, 3, 40, 40) };
        assert!(opened.status("a.pdf", 120, false).contains(&format!("  {}  ", OPENED_KEYS[0])));
        // Without the pages to a row where there isn't room for them.
        assert!(opened.status("a.pdf", 100, false).contains(&format!("  {}  ", OPENED_KEYS[1])));
        assert!(opened.help_lines().contains(&("Esc  Backspace  u".into(), "back to the list")));
        assert!(reader(0, 3, 40, 40).help_lines().contains(&("q  Esc  Ctrl-C".into(), "quit")));
        assert!(nav.help_lines().iter().any(|(_, what)| what.starts_with("open it")));
    }

    #[test]
    fn pages_given_are_the_only_ones_paged_through() {
        let mut nav = Nav::new(vec![6, 2, 9], 12, Zoom::Auto);
        nav.shown(40, 40);
        assert_eq!(page(&nav), 6);
        press(&mut nav, &[Key::Char(' ')]);
        assert_eq!(page(&nav), 2);
        press(&mut nav, &[Key::Char('G')]);
        assert_eq!(page(&nav), 9);
        press(&mut nav, &[Key::Char('7'), Key::Enter]);
        assert_eq!(page(&nav), 6);
        press(&mut nav, &[Key::Char('5'), Key::Enter]);
        assert_eq!(page(&nav), 6);
        assert_eq!(nav.message.as_deref(), Some("page 5 isn't one of the pages given"));
        assert!(squeezed(&nav.status("a.pdf", 120, false)).contains(" page 7/12 · 1 of 3 · 1 per row "));
    }

    #[test]
    fn a_typed_number_goes_to_that_page() {
        let mut nav = reader(0, 12, 40, 40);
        press(&mut nav, &[Key::Char('1'), Key::Char('2'), Key::Enter]);
        assert_eq!((page(&nav), nav.typed.as_str()), (11, ""));
        press(&mut nav, &[Key::Char('3'), Key::Char('g')]);
        assert_eq!(page(&nav), 2);
        press(&mut nav, &[Key::Char('9'), Key::Char('9'), Key::Enter]);
        assert_eq!(page(&nav), 2);
        assert_eq!(nav.message.as_deref(), Some("no page 99 (1–12)"));
        press(&mut nav, &[Key::Char('0'), Key::Enter]);
        assert_eq!(page(&nav), 2);
        // Backspace and Esc edit and cancel; with nothing typed, they leave, which at the top
        // quits.
        press(&mut nav, &[Key::Char('5'), Key::Char('1'), Key::Backspace, Key::Enter]);
        assert_eq!(page(&nav), 4);
        assert_eq!(press(&mut nav, &[Key::Char('7'), Key::Escape]), Step::Continue);
        assert_eq!(nav.typed, "");
        assert_eq!(press(&mut nav, &[Key::Escape]), Step::Leave);
        assert_eq!(press(&mut nav, &[Key::Backspace]), Step::Back);
        // `u` goes up, a number typed or not.
        assert_eq!(press(&mut nav, &[Key::Char('u')]), Step::Back);
        assert_eq!(press(&mut nav, &[Key::Char('4'), Key::Char('u')]), Step::Back);
        // Long numbers stop growing and simply name no page.
        let mut nav = reader(0, 12, 40, 40);
        press(&mut nav, &[Key::Char('9'); 30]);
        assert_eq!(nav.typed.len(), MAX_TYPED);
    }

    #[test]
    fn q_and_ctrl_c_quit_and_r_renders_again() {
        let mut nav = reader(0, 2, 30, 100);
        // Zooming starts at the top of the page.
        press(&mut nav, &[Key::Char(' '), Key::Char('+')]);
        assert_eq!((nav.zoom, nav.scroll), (Zoom::Width, 0));
        press(&mut nav, &[Key::Char('r')]);
        assert!(nav.rerender);
        assert_eq!(press(&mut nav, &[Key::Char('q')]), Step::Quit);
        assert_eq!(press(&mut nav, &[Key::Interrupt]), Step::Quit);
        assert_eq!(press(&mut nav, &[Key::Char('x'), Key::PageUp, Key::End]), Step::Continue);
    }

    /// `c` copies the page the reader is on, by its number in the document; in a list, the tile
    /// chosen. Nothing else moves.
    #[test]
    fn c_copies_the_page_the_reader_is_on_or_the_tile_chosen() {
        let mut nav = reader(4, 10, 40, 40);
        assert_eq!(press(&mut nav, &[Key::Char('c')]), Step::ToClipboard(4));
        assert_eq!(page(&nav), 4);
        // Two side by side: the reader's page, the one zooming and typed numbers anchor on.
        nav.grid(2, 1, 8);
        press(&mut nav, &[Key::Char('n')]);
        assert_eq!((nav.on_screen(), press(&mut nav, &[Key::Char('c')])), (vec![Some(6), Some(7)], Step::ToClipboard(6)));
        // Pages given keep their numbers.
        let mut given = Nav::new(vec![6, 2, 9], 12, Zoom::Auto);
        given.shown(40, 40);
        press(&mut given, &[Key::Char(' ')]);
        assert_eq!(press(&mut given, &[Key::Char('c')]), Step::ToClipboard(2));
        // In a list, the tile chosen; the pager tells a folder's from a file's.
        let mut chosen = tiles(10, 3, 2);
        press(&mut chosen, &[Key::Right, Key::Down]);
        assert_eq!(press(&mut chosen, &[Key::Char('c')]), Step::ToClipboard(4));
        assert_eq!(chosen.at, 4);
        assert_eq!(press(&mut Nav { list: true, ..Nav::new(vec![6, 2, 9], 12, Zoom::Auto) }, &[Key::Char('c')]), Step::ToClipboard(6));
        // Nothing to copy in an empty list; with the keys on screen, `c` only goes back.
        assert_eq!(press(&mut list(0), &[Key::Char('c')]), Step::Continue);
        press(&mut nav, &[Key::Char('?')]);
        assert_eq!((press(&mut nav, &[Key::Char('c')]), nav.help), (Step::Continue, false));
    }

    #[test]
    fn an_empty_list_shows_no_page_and_never_panics() {
        let mut nav = Nav::new(Vec::new(), 0, Zoom::Auto);
        nav.grid(2, 1, 1);
        nav.shown(1, 1);
        assert_eq!(nav.on_screen(), [None, None]);
        let keys = [' ', 'b', 'j', 'k', 'n', 'p', 'G', '-', '-', '+', '+', '+', '1'].map(Key::Char);
        press(&mut nav, &keys);
        press(&mut nav, &[Key::Enter]);
        assert!(nav.status("a.pdf", 80, false).contains("no page"));
    }

    /// `line` with each run of spaces made one: what it says, whatever the padding.
    fn squeezed(line: &str) -> String {
        line.split(' ').filter(|w| !w.is_empty()).fold(String::from(" "), |all, w| all + w + " ")
    }

    /// The character the last `needle` in `line` starts at.
    fn column(line: &str, needle: &str) -> usize {
        chars(&line[..line.rfind(needle).unwrap_or_else(|| panic!("{needle:?} in {line:?}"))])
    }

    #[test]
    fn the_status_line_fills_the_width_exactly() {
        let nav = reader(2, 12, 40, 40);
        for cols in 0..=200 {
            let line = nav.status("report.pdf", cols, false);
            assert_eq!(chars(&line), usize::from(cols), "{line:?}");
        }
        let wide = nav.status("report.pdf", 120, false);
        assert!(wide.starts_with(" report.pdf  ") && wide.ends_with(" page 3/12      · 1 per row · ? help "), "{wide:?}");
        let mut two = reader(2, 12, 40, 40);
        two.grid(2, 1, 8);
        assert!(two.status("report.pdf", 120, false).ends_with(" pages 3–4/12   · 2 per row · ? help "));
        let mut typing = nav;
        typing.key(Key::Char('7'));
        assert_eq!(typing.status("a.pdf", 80, false), format!("{:<80}", " 7: g page · . per row"));
    }

    /// While a number is typed, the status line is the number and the keys that take it here, and
    /// nothing else: on a file's pages `g` and `.`; on a document, its settings with numbers too, by
    /// their names, the font's on `+` first; in a list `Enter` and `.`; with `--pages`, no setting that
    /// lays out again. Cut to fit, the last keys first, and as wide as the window whatever it is.
    #[test]
    fn a_number_typed_says_the_keys_that_take_it() {
        let typed = |mut nav: Nav, digits: &str| {
            for c in digits.chars() {
                nav.key(Key::Char(c));
            }
            nav
        };
        let pdf = typed(reader(0, 12, 40, 40), "7");
        assert_eq!(pdf.status("report.pdf", 60, true), format!("{:<60}", " 7: g page · . per row"), "the name, the place, the zoom gone");
        let doc = typed(declaring_font(reader(0, 12, 40, 40), &["shade", "font", "width"], Some(170)), "72");
        assert_eq!(doc.status("a.md", 80, false).trim_end(), " 72: g page · . per row · + font · ] width");
        // Cut to fit: the last keys go first, then all but the number.
        assert_eq!(doc.status("a.md", 30, false), format!("{:<30}", " 72: g page · . per row"));
        assert_eq!(doc.status("a.md", 12, false), format!("{:<12}", " 72: g page"));
        assert_eq!(doc.status("a.md", 5, false), " 72: ");
        let picked = typed(Nav { picked: true, ..declaring_font(reader(0, 12, 40, 40), &["font", "width"], Some(170)) }, "3");
        assert_eq!(picked.status("a.md", 80, false).trim_end(), " 3: g page · . per row");
        let list = typed(declaring_font(Nav { list: true, ..reader(0, 12, 40, 40) }, &["font", "width"], Some(170)), "4");
        assert_eq!(list.status("", 80, false).trim_end(), " 4: Enter tile · . per row");
        let long = typed(doc, "9999999");
        for cols in 0..=120 {
            assert_eq!(chars(&long.status("a.md", cols, false)), usize::from(cols));
        }
    }

    /// Where the reader is, the zoom and the key for the keys stay where they are whatever the
    /// reader does: whatever the page, the name, the message, the zoom or the rendering.
    #[test]
    fn the_right_side_of_the_status_line_never_moves() {
        let long = "a file name long enough to fill most of a window, and then some more.pdf";
        let mut nav = reader(0, 120, 40, 40);
        let at = column(&nav.status("a.pdf", 100, false), "page 1/120");
        for (page, per_row) in [(0, 1), (8, 1), (9, 2), (98, 2), (99, 1), (119, 8)] {
            nav.go(page);
            nav.grid(per_row, 1, 8);
            for name in ["a.pdf", long] {
                for message in [None, Some("no page 999 (1–120)")] {
                    nav.message = message.map(String::from);
                    for rendering in [false, true] {
                        let line = nav.status(name, 100, rendering);
                        assert_eq!(column(&line, "page"), at, "{line:?}");
                        assert_eq!(chars(&line), 100);
                        assert!(line.ends_with(" per row · ? help "), "{line:?}");
                    }
                }
            }
        }
        nav.zoom = Zoom::Width;
        nav.grid(1, 1, 8);
        nav.message = None;
        assert!(nav.status("a.pdf", 100, false).ends_with(" page 120/120      · width     · ? help "));
        // A message cuts the name short, in its middle, and stays whole itself.
        nav.message = Some("no page 999 (1–120)".into());
        let told = nav.status(long, 100, false);
        assert!(told.starts_with(" a file name long …hen some more.pdf  no page 999 (1–120)    page 120/120 "), "{told:?}");
        assert_eq!(column(&told, "page 120/120"), at);
    }

    #[test]
    fn the_keys_are_reminded_of_until_the_first_one_and_then_the_key_for_them() {
        let mut nav = reader(4, 13, 40, 40);
        let first = nav.status("report.pdf", 120, false);
        assert!(first.contains(&format!("  {}  ", KEYS[0])) && first.ends_with(" · ? help "), "{first:?}");
        // As much of the reminder as fits, or none.
        assert!(nav.status("report.pdf", 70, false).contains(&format!("  {}  ", KEYS[3])));
        assert!(!nav.status("report.pdf", 50, false).contains("quit"));
        nav.key(Key::Char('x'));
        let after = nav.status("report.pdf", 120, false);
        assert!(!after.contains("quit") && after.ends_with(" · ? help "), "{after:?}");
        // A narrow window drops the zoom, then the key and the padding; the place stays whole as
        // long as it can.
        assert_eq!(nav.status("report.pdf", 30, false), " …f   page 5/13      · ? help ");
        assert_eq!(nav.status("report.pdf", 18, false), " r…df   page 5/13 ");
        assert_eq!(nav.status("report.pdf", 8, false), "  page 5");
    }

    #[test]
    fn a_name_too_long_keeps_its_start_and_its_end() {
        assert_eq!(cut("IMG_20240817_2041.jpg", 12), "IMG_2…41.jpg");
        assert_eq!(cut("IMG_20240817_2042.jpg", 11), "IMG_2…2.jpg");
        assert_eq!(cut("short", 5), "short");
        assert_eq!(cut("rapport-été.pdf", 6), "ra…pdf");
        assert_eq!((cut("ab", 1), cut("ab", 0), cut("", 0)), ("…".into(), String::new(), String::new()));
    }

    #[test]
    fn question_mark_shows_the_keys_and_any_key_goes_back() {
        let mut nav = reader(3, 12, 40, 40);
        press(&mut nav, &[Key::Char('?')]);
        assert!(nav.help);
        // That key only goes back: n doesn't turn the page, q doesn't quit, Esc doesn't either.
        for key in [Key::Char('n'), Key::Char('q'), Key::Escape] {
            assert_eq!(press(&mut nav, &[key]), Step::Continue);
            assert_eq!((nav.help, page(&nav)), (false, 3), "{key:?}");
            press(&mut nav, &[Key::Char('?')]);
        }
        // Ctrl-C quits all the same.
        assert_eq!(press(&mut nav, &[Key::Interrupt]), Step::Quit);
    }

    #[test]
    fn the_status_line_never_carries_an_escape_sequence() {
        let mut nav = reader(0, 3, 40, 40);
        // A file name and an error message that would write to the clipboard (OSC 52).
        nav.message = Some("page 1: renderer crashed: \x1b]52;c;cHduZWQ=\x07".into());
        let line = nav.status("evil\x1b]52;c;cHduZWQ=\x07.pdf", 200, false);
        assert!(!line.chars().any(char::is_control), "{line:?}");
        assert_eq!(chars(&line), 200);
        nav.message = None;
        assert!(nav.status("a.pdf", 80, true).contains("… page 1/3 "));
    }

    /// Page `page` sent as image `id`, `mb` megabytes of pixels, with `warning`.
    fn sent(page: usize, id: u32, mb: u32, warning: Option<&str>) -> Sent {
        let view = View { level: 0, page, fit: Fit::Page, cols: 50, rows: 40, size: FALLBACK_SIZE };
        Sent { view, id, cols: 50, rows: 40, width: 1024, height: mb * 256, warning: warning.map(String::from), text: Vec::new(), tint: 0 }
    }

    #[test]
    fn pages_kept_in_the_terminal_stay_under_the_count_and_the_budget() {
        let mut kept: Vec<Sent> = (0..4).map(|p| sent(p, p as u32 + 1, 10, None)).collect();
        let screen = [Some(kept[0].view), None];
        // 40 MB kept, 10 more coming: under 64, nothing goes.
        assert!(make_room(&mut kept, 10 << 20, 64 << 20, MAX_SENT, &screen).is_empty());
        // Under 32: the least recently shown go first, never the page on screen.
        assert_eq!(make_room(&mut kept, 10 << 20, 32 << 20, MAX_SENT, &screen), [2, 3]);
        assert_eq!(kept.iter().map(|s| s.id).collect::<Vec<_>>(), [1, 4]);
        // A page bigger than the whole budget: all but the screen go, and it goes in anyway.
        assert_eq!(make_room(&mut kept, 100 << 20, 32 << 20, MAX_SENT, &screen), [4]);
        assert_eq!(kept.len(), 1);
        // And never more than the most pages kept, however small: a grid's screens keep more.
        let mut many: Vec<Sent> = (0..MAX_SENT).map(|p| sent(p, p as u32, 1, None)).collect();
        assert!(make_room(&mut many, 1, u64::MAX, 4 * MAX_SENT, &[]).is_empty());
        assert_eq!(make_room(&mut many, 1, u64::MAX, MAX_SENT, &[None, None]), [0]);
        // Sizes that would overflow don't.
        let mut huge = vec![Sent { width: u32::MAX, height: u32::MAX, ..sent(0, 1, 1, None) }];
        assert_eq!(make_room(&mut huge, u64::MAX, BUDGET, MAX_SENT, &[None, None]), [1]);
    }

    /// A page sent again with another tint takes the place of the one sent before: once it is
    /// placed, every other image of that page goes, and only of that page; about to be sent while
    /// off screen, they all go.
    #[test]
    fn a_page_sent_again_takes_the_place_of_the_one_before() {
        let tinted = |page: usize, id: u32, tint: u64| Sent { tint, ..sent(page, id, 1, None) };
        let mut kept = vec![tinted(0, 1, 0), tinted(1, 2, 0), tinted(0, 3, 7), tinted(0, 4, 9)];
        let view = kept[0].view;
        assert_eq!(superseded(&mut kept, view, Some(4)), [1, 3]);
        assert_eq!(kept.iter().map(|s| (s.id, s.tint)).collect::<Vec<_>>(), [(2, 0), (4, 9)]);
        assert!(superseded(&mut kept, view, Some(4)).is_empty(), "nothing more to go");
        assert_eq!(superseded(&mut kept, view, None), [4]);
        assert_eq!(kept.iter().map(|s| s.id).collect::<Vec<_>>(), [2]);
        // The one placed is the one with the tint of now, whichever was sent last; while none has it,
        // the last sent, which stays up until the right one comes.
        let kept = vec![tinted(0, 1, 7), tinted(1, 2, 0), tinted(0, 3, 9)];
        let id = |key| shown(&kept, view, key).map(|s| s.id);
        assert_eq!((id(7), id(9), id(5), shown(&kept, sent(4, 9, 1, None).view, 7).map(|s| s.id)), (Some(1), Some(3), Some(3), None));
    }

    #[test]
    fn a_page_takes_four_bytes_a_pixel() {
        // US Letter at 2 px per point: 1224 × 1584 pixels.
        assert_eq!(pixel_bytes((612.0, 792.0), 2.0), 1224 * 1584 * 4);
        assert_eq!(pixel_bytes((0.5, 0.5), 1.0), 4, "a part of a pixel is a pixel");
        assert_eq!(pixel_bytes((f32::MAX, 1.0), 2.0), 0, "an endless side counts for nothing");
        assert_eq!(pixel_bytes((1e6, 1e6), 1.0), 8192 * 8192 * 4);
        assert_eq!(pixel_bytes((f32::NAN, -5.0), 1.0), 0);
    }

    #[test]
    fn the_status_line_names_what_is_wrong_with_a_page_on_screen_whenever_it_is() {
        let kept = [sent(0, 1, 1, None), sent(1, 2, 1, Some("page 2: content past the page's safety budget was skipped"))];
        let view = |page| Some(View { level: 0, page, fit: Fit::Page, cols: 50, rows: 40, size: FALLBACK_SIZE });
        // Page 2 was rendered ahead with something skipped: said when it shows, not before.
        assert_eq!(note(&[view(0), None], &kept, &[]), None);
        assert_eq!(note(&[view(1), None], &kept, &[]).as_deref(), Some("page 2: content past the page's safety budget was skipped"));
        assert!(note(&[view(0), view(1)], &kept, &[]).is_some());
        // A failure first.
        let failed = [(view(0).unwrap(), "page 1: broken".to_string())];
        assert_eq!(note(&[view(0), view(1)], &kept, &failed).as_deref(), Some("page 1: broken"));
    }

    #[test]
    fn source_rows_follow_the_scroll() {
        let view = View { level: 0, page: 0, fit: Fit::Width, cols: 50, rows: 40, size: FALLBACK_SIZE };
        let shown = Sent { view, id: 1, cols: 50, rows: 100, width: 1000, height: 2000, warning: None, text: Vec::new(), tint: 0 };
        assert_eq!(source_rows(&shown, 0, 30), Rect { x: 0, y: 0, w: 1000, h: 600 });
        assert_eq!(source_rows(&shown, 70, 30), Rect { x: 0, y: 1400, w: 1000, h: 600 });
        // Never past the bottom, whatever it is asked.
        let past = source_rows(&shown, u16::MAX, 30);
        assert_eq!((past.y, past.h), (2000, 0));
        let empty = Sent { rows: 0, height: 0, ..shown };
        assert_eq!(source_rows(&empty, 5, 5), Rect { x: 0, y: 0, w: 1000, h: 0 });
    }

    /// The names of a list's tiles, 1 to 8.
    const PHOTOS: [&str; 8] = ["Beach.jpg", "city.png", "beach-2.jpg", "trip/", "Café.jpg", "notes.pdf", "BEACH/", "zoo.png"];

    /// A list of tiles named `names`, all of them, four to a row in two rows, after a draw.
    fn named<S: AsRef<str>>(names: &[S]) -> Nav {
        let mut nav = Nav { names: names.iter().map(|name| fold(name.as_ref())).collect(), ..list(names.len()) };
        nav.grid(4, 2, 8);
        nav.shown(10, 10);
        nav
    }

    /// `text` typed, a key a character: what the first key that asks more than a move asks.
    fn type_in(nav: &mut Nav, text: &str) -> Step {
        press(nav, &text.chars().map(Key::Char).collect::<Vec<_>>())
    }

    /// The tiles the reader moves among, and the one chosen.
    fn matching(nav: &Nav) -> (Vec<usize>, Option<usize>) {
        (nav.pages.clone(), nav.pages.get(nav.at).copied())
    }

    /// Whether a filter is being typed.
    fn typing(nav: &Nav) -> bool {
        nav.filter.as_ref().is_some_and(|filter| filter.typing)
    }

    /// The filter's text.
    fn filter_text(nav: &Nav) -> Option<&str> {
        nav.filter.as_ref().map(|filter| filter.text.as_str())
    }

    #[test]
    fn slash_narrows_a_list_to_the_names_that_hold_what_is_typed_as_it_is_typed() {
        let mut nav = named(&PHOTOS);
        assert_eq!(press(&mut nav, &[Key::Char('/')]), Step::Continue);
        // Nothing typed yet: every tile.
        assert_eq!((typing(&nav), matching(&nav)), (true, ((0..8).collect(), Some(0))));
        type_in(&mut nav, "b");
        assert_eq!(matching(&nav), (vec![0, 2, 6], Some(0)));
        // Upper and lower case alike, in the names and in what is typed. The tile chosen no longer
        // matches: the next that does is chosen.
        type_in(&mut nav, "EACH-");
        assert_eq!(matching(&nav), (vec![2], Some(2)));
        // It still matches: it stays chosen.
        press(&mut nav, &[Key::Backspace]);
        assert_eq!(matching(&nav), (vec![0, 2, 6], Some(2)));
        // A folder's name ends with `/`.
        type_in(&mut nav, "/");
        assert_eq!((filter_text(&nav), matching(&nav)), (Some("bEACH/"), (vec![6], Some(6))));
    }

    #[test]
    fn while_a_filter_is_typed_every_key_that_types_a_character_is_text() {
        let mut nav = declaring(named(&PHOTOS), true);
        let zoom = nav.zoom;
        press(&mut nav, &[Key::Char('/')]);
        // Keys that quit, move, zoom, go up, show the keys or type a number, elsewhere.
        assert_eq!(type_in(&mut nav, "q j1?u gG+-/rcd"), Step::Continue);
        assert_eq!(filter_text(&nav), Some("q j1?u gG+-/rcd"));
        assert_eq!(nav.settings.values.get(), Values::default(), "typed, a setting's key is a letter of the filter");
        assert_eq!((nav.help, nav.zoom, nav.typed.as_str(), nav.rerender, typing(&nav)), (false, zoom, "", false, true));
        // A control character isn't: Ctrl-L renders again, and Ctrl-C still quits.
        press(&mut nav, &[Key::Char('\u{c}'), Key::Char('\u{1b}')]);
        assert_eq!((nav.rerender, filter_text(&nav)), (true, Some("q j1?u gG+-/rcd")));
        assert_eq!(press(&mut nav, &[Key::Interrupt]), Step::Quit);
    }

    #[test]
    fn while_a_filter_is_typed_the_keys_that_move_move_among_the_matches() {
        let mut nav = named(&PHOTOS);
        type_in(&mut nav, "/b");
        for (key, chosen) in [(Key::Right, 2), (Key::End, 6), (Key::Left, 2), (Key::Home, 0), (Key::Down, 0), (Key::PageDown, 6), (Key::PageUp, 0)] {
            assert_eq!(press(&mut nav, &[key]), Step::Continue, "{key:?}");
            assert_eq!((matching(&nav).1, typing(&nav)), (Some(chosen), true), "{key:?}");
        }
    }

    #[test]
    fn enter_keeps_the_filter_and_the_lists_keys_move_among_the_matches() {
        let mut nav = named(&PHOTOS);
        type_in(&mut nav, "/b");
        assert_eq!(press(&mut nav, &[Key::Enter]), Step::Continue, "Enter keeps the filter, and opens nothing");
        assert!(!typing(&nav) && nav.filter.is_some());
        for (keys, chosen) in [(&[Key::Char('l')][..], 2), (&[Key::Char('G')], 6), (&[Key::Char('g')], 0), (&[Key::Char('l'); 3], 6)] {
            press(&mut nav, keys);
            assert_eq!(matching(&nav).1, Some(chosen), "{keys:?}");
        }
        assert_eq!(press(&mut nav, &[Key::Enter]), Step::Open(6));
        // `/` again edits it, from what was kept.
        type_in(&mut nav, "/e");
        assert_eq!((filter_text(&nav), typing(&nav), matching(&nav).0), (Some("be"), true, vec![0, 2, 6]));
    }

    #[test]
    fn with_a_filter_kept_esc_backspace_and_u_clear_it_before_they_go_back() {
        for key in [Key::Escape, Key::Backspace, Key::Char('u')] {
            let mut nav = named(&PHOTOS);
            type_in(&mut nav, "/b");
            press(&mut nav, &[Key::Enter, Key::Char('G')]);
            assert_eq!(press(&mut nav, &[key]), Step::Continue, "{key:?}");
            // Every tile again, the one chosen still chosen.
            assert_eq!((nav.filter.is_none(), matching(&nav)), (true, ((0..8).collect(), Some(6))), "{key:?}");
            let then = if key == Key::Escape { Step::Leave } else { Step::Back };
            assert_eq!(press(&mut nav, &[key]), then, "{key:?}");
        }
        // A number being typed goes first, as without a filter.
        let mut nav = named(&PHOTOS);
        type_in(&mut nav, "/b");
        press(&mut nav, &[Key::Enter, Key::Char('7'), Key::Escape]);
        assert_eq!((nav.typed.as_str(), filter_text(&nav)), ("", Some("b")));
    }

    #[test]
    fn esc_while_a_filter_is_typed_puts_back_what_was_there_before_the_slash() {
        let mut nav = named(&PHOTOS);
        press(&mut nav, &[Key::Char('l'); 5]);
        type_in(&mut nav, "/be");
        assert_eq!(matching(&nav), (vec![0, 2, 6], Some(6)));
        assert_eq!(press(&mut nav, &[Key::Escape]), Step::Continue);
        assert_eq!((nav.filter.is_none(), matching(&nav)), (true, ((0..8).collect(), Some(5))));
        // With a filter kept: that filter, and the tile chosen then.
        type_in(&mut nav, "/b");
        press(&mut nav, &[Key::Enter, Key::Char('g')]);
        type_in(&mut nav, "/each-");
        assert_eq!(matching(&nav), (vec![2], Some(2)));
        assert_eq!(press(&mut nav, &[Key::Escape]), Step::Continue);
        assert_eq!((filter_text(&nav), typing(&nav), matching(&nav)), (Some("b"), false, (vec![0, 2, 6], Some(0))));
    }

    #[test]
    fn backspace_or_enter_with_nothing_typed_leaves_no_filter() {
        let mut nav = named(&PHOTOS);
        assert_eq!(press(&mut nav, &[Key::Char('/'), Key::Backspace]), Step::Continue, "Backspace doesn't go back");
        assert!(nav.filter.is_none());
        assert_eq!(press(&mut nav, &[Key::Char('/'), Key::Enter]), Step::Continue, "Enter opens nothing");
        assert!(nav.filter.is_none());
        assert_eq!(press(&mut nav, &[Key::Enter]), Step::Open(0));
        // Nor past what was typed, from a filter kept or not.
        type_in(&mut nav, "/ab");
        press(&mut nav, &[Key::Backspace, Key::Backspace]);
        assert_eq!((typing(&nav), filter_text(&nav)), (true, Some("")));
        press(&mut nav, &[Key::Backspace]);
        assert!(nav.filter.is_none());
        type_in(&mut nav, "/b");
        press(&mut nav, &[Key::Enter, Key::Char('/'), Key::Backspace, Key::Enter]);
        assert_eq!((nav.filter.is_none(), nav.pages.len()), (true, 8));
    }

    #[test]
    fn a_filter_that_matches_nothing_says_so_and_every_key_still_works() {
        let mut nav = named(&PHOTOS);
        type_in(&mut nav, "/xyz");
        assert_eq!(matching(&nav), (vec![], None));
        assert!(squeezed(&nav.status("", 100, false)).contains(" /xyz ⏎ keep · esc cancel no match · 4 per row "));
        press(&mut nav, &[Key::Enter]);
        assert_eq!((nav.on_screen(), nav.around(AHEAD, BEHIND)), (vec![None; 8], vec![]));
        assert!(squeezed(&nav.status("", 100, false)).ends_with(" /xyz no match · 4 per row · ? help "));
        for key in [Key::Left, Key::Right, Key::Up, Key::Down, Key::PageDown, Key::PageUp, Key::Home, Key::End, Key::Enter]
            .into_iter()
            .chain([' ', 'b', 'n', 'p', 'j', 'k', 'g', 'G', '+', '-', '?', 'x'].map(Key::Char))
        {
            assert_eq!(press(&mut nav, &[key]), Step::Continue, "{key:?}");
            assert_eq!(matching(&nav), (vec![], None), "{key:?}");
            for cols in 0..=120 {
                assert_eq!(chars(&nav.status("", cols, false)), usize::from(cols));
            }
        }
        press(&mut nav, &[Key::Char('3'), Key::Enter]);
        assert_eq!(nav.message.as_deref(), Some("tile 3 doesn't match /xyz"));
        // Back to a filter that matches: near the tile chosen before none did.
        let mut nav = named(&PHOTOS);
        press(&mut nav, &[Key::Char('l'); 3]);
        type_in(&mut nav, "/t");
        assert_eq!(matching(&nav), (vec![1, 3, 5], Some(3)));
        type_in(&mut nav, "x");
        assert_eq!(matching(&nav), (vec![], None));
        press(&mut nav, &[Key::Backspace]);
        assert_eq!(matching(&nav), (vec![1, 3, 5], Some(3)));
    }

    /// A number is the list's, filtered or not, as with the tiles given: `12 Enter` chooses the
    /// 12th tile if it matches.
    #[test]
    fn a_number_typed_in_a_filtered_list_is_still_the_lists() {
        let mut nav = named(&PHOTOS);
        type_in(&mut nav, "/b");
        press(&mut nav, &[Key::Enter, Key::Char('7'), Key::Enter]);
        assert_eq!((matching(&nav).1, nav.message.as_deref()), (Some(6), None));
        press(&mut nav, &[Key::Char('2'), Key::Enter]);
        assert_eq!((matching(&nav).1, nav.message.as_deref()), (Some(6), Some("tile 2 doesn't match /b")));
        press(&mut nav, &[Key::Char('9'), Key::Char('9'), Key::Enter]);
        assert_eq!(nav.message.as_deref(), Some("no tile 99 (1–8)"));
    }

    #[test]
    fn a_filter_narrows_the_tiles_given_and_keeps_their_order() {
        // `--pages 7,3,1,6`.
        let mut nav = named(&PHOTOS);
        nav.pages = vec![6, 2, 0, 5];
        type_in(&mut nav, "/b");
        assert_eq!(matching(&nav), (vec![6, 2, 0], Some(6)));
        assert!(squeezed(&nav.status("", 120, false)).contains(" 7/8 · 1 of 3 · 4 per row "));
        press(&mut nav, &[Key::Enter, Key::Char('6'), Key::Enter]);
        assert_eq!(nav.message.as_deref(), Some("tile 6 doesn't match /b"));
        press(&mut nav, &[Key::Char('2'), Key::Enter]);
        assert_eq!(nav.message.as_deref(), Some("tile 2 isn't one of the tiles given"));
        // Cleared, the tiles given, not the list's.
        press(&mut nav, &[Key::Char('G'), Key::Escape]);
        assert_eq!(matching(&nav), (vec![6, 2, 0, 5], Some(0)));
        assert!(squeezed(&nav.status("", 120, false)).contains(" 1/8 · 3 of 4 · 4 per row "));
    }

    #[test]
    fn the_status_line_shows_the_filter_and_where_the_reader_is_among_the_matches() {
        let mut nav = named(&PHOTOS);
        press(&mut nav, &[Key::Char('/')]);
        let line = nav.status("Beach.jpg", 100, false);
        assert!(squeezed(&line).contains(" Beach.jpg / ⏎ keep · esc cancel 1/8 · 1 of 8 · 4 per row · ? help "), "{line:?}");
        let at = column(&line, "1/8");
        type_in(&mut nav, "beach");
        let line = nav.status("Beach.jpg", 100, false);
        assert!(squeezed(&line).contains(" Beach.jpg /beach ⏎ keep · esc cancel 1/8 · 1 of 3 · 4 per row "), "{line:?}");
        // Where the reader is stays where it was, whatever matches.
        assert_eq!(column(&line, "1/8"), at);
        type_in(&mut nav, "x");
        assert_eq!(column(&nav.status("", 100, false), "no match"), at);
        // A narrow window drops the keys first.
        press(&mut nav, &[Key::Backspace]);
        let narrow = nav.status("Beach.jpg", 60, false);
        assert!(narrow.contains("  /beach  ") && !narrow.contains("keep"), "{narrow:?}");
        // Kept, the filter, when there is nothing else to say.
        press(&mut nav, &[Key::Enter]);
        assert!(squeezed(&nav.status("Beach.jpg", 100, false)).contains(" Beach.jpg /beach 1/8 · 1 of 3 · "));
        press(&mut nav, &[Key::Char('4')]);
        assert_eq!(nav.status("Beach.jpg", 100, false).trim_end(), " 4: Enter tile · . per row");
        // Whatever is typed, the line is exactly as wide as the window.
        type_in(&mut nav, "/ünïcödé, 写真 and more");
        for cols in 0..=200 {
            let line = nav.status("Beach.jpg", cols, false);
            assert_eq!(chars(&line), usize::from(cols), "{line:?}");
            assert!(!line.chars().any(char::is_control), "{line:?}");
        }
    }

    /// A reader on the first of five pages, whose text is a chunk a page: `Title, page N`. And the
    /// text, to say what it gives and when.
    fn searching() -> (Nav, Rc<search::tests::Fake>) {
        let fake = search::tests::Fake::of(&["Title, page 1", "Title, page 2", "Title, page 3", "Title, page 4", "Title, page 5"]);
        (reader(0, 5, 40, 40), fake)
    }

    /// `keys` pressed as the pager takes them: `/` asks the source for its text, here `texts`, and
    /// what there is to read is read and matched after each key.
    fn search_keys(nav: &mut Nav, texts: &Rc<search::tests::Fake>, keys: &[Key]) -> Step {
        let mut first = Step::Continue;
        for key in keys {
            let mut step = nav.key(*key);
            if step == Step::Search {
                nav.start_search(Some(Box::new(texts.clone())));
                step = Step::Continue;
            }
            // Until two passes in a row have no news: what one asks for, the next is given.
            let news = |nav: &mut Nav| {
                let (said, drawn) = nav.search_on(&search::tests::square);
                said || drawn
            };
            while news(nav) | news(nav) {}
            if first == Step::Continue {
                first = step;
            }
        }
        first
    }

    /// `text` typed after `/`, as [`search_keys`] presses keys.
    fn search_for(nav: &mut Nav, texts: &Rc<search::tests::Fake>, text: &str) -> Step {
        search_keys(nav, texts, &text.chars().map(Key::Char).collect::<Vec<_>>())
    }

    /// What the status line says of the search, and nothing else of it.
    fn searched(nav: &Nav) -> String {
        let line = squeezed(&nav.status("a.pdf", 120, false));
        let said = line.trim_start_matches(" a.pdf ");
        said[..said.rfind(" page").unwrap_or(said.len())].to_owned()
    }

    /// `/` in a file's pages opens a prompt: what is typed is counted in the text, the view
    /// staying where it is; Enter goes to the first match from the top of the screen and keeps the
    /// search, which the status line says, with the match the reader is on.
    #[test]
    fn slash_in_a_files_pages_searches_its_text_and_enter_goes_to_the_first_match() {
        let (mut nav, texts) = searching();
        assert_eq!(nav.key(Key::Char('/')), Step::Search, "the pager's to ask the source");
        assert_eq!(search_for(&mut nav, &texts, "/page 3"), Step::Continue);
        assert_eq!((page(&nav), searched(&nav)), (0, "/page 3 1 ⏎ search · esc cancel".into()), "counted, the view where it was");
        search_keys(&mut nav, &texts, &[Key::Enter]);
        assert_eq!((page(&nav), searched(&nav)), (2, "/page 3 1/1".into()));
        assert!(nav.search.as_ref().is_some_and(|search| !search.typing()));
        // From the top of the screen, not of the file: on page 4, the first match is page 4's.
        let (mut nav, texts) = searching();
        nav.go(3);
        search_for(&mut nav, &texts, "/PAGE");
        search_keys(&mut nav, &texts, &[Key::Enter]);
        assert_eq!((page(&nav), searched(&nav)), (3, "/PAGE 4/5".into()));
        // A number typed before `/` is dropped.
        let (mut nav, texts) = searching();
        search_for(&mut nav, &texts, "4/e 2");
        search_keys(&mut nav, &texts, &[Key::Enter]);
        assert_eq!((page(&nav), nav.typed.as_str(), searched(&nav)), (1, "", "/e 2 1/1".into()));
    }

    /// With a search kept, `n` and `N` go from match to match, round the ends, saying so; a
    /// number first goes that many on; the arrows still turn pages; and paged away, `n` goes on from
    /// the top of the screen.
    #[test]
    fn n_and_shift_n_go_from_match_to_match_while_a_search_is_kept() {
        let (mut nav, texts) = searching();
        search_for(&mut nav, &texts, "/page");
        search_keys(&mut nav, &texts, &[Key::Enter, Key::Char('n'), Key::Char('n')]);
        assert_eq!((page(&nav), searched(&nav)), (2, "/page 3/5".into()));
        search_keys(&mut nav, &texts, &[Key::Char('N')]);
        assert_eq!((page(&nav), searched(&nav)), (1, "/page 2/5".into()));
        search_keys(&mut nav, &texts, &[Key::Char('3'), Key::Char('n')]);
        assert_eq!((page(&nav), nav.typed.as_str(), searched(&nav)), (4, "", "/page 5/5".into()));
        search_keys(&mut nav, &texts, &[Key::Char('n')]);
        assert_eq!((page(&nav), searched(&nav)), (0, "/page 1/5 · search wrapped".into()));
        search_keys(&mut nav, &texts, &[Key::Char('N')]);
        assert_eq!((page(&nav), searched(&nav)), (4, "/page 5/5 · search wrapped".into()));
        // The arrows turn pages, and the match the reader was on is no longer on screen: `n` goes on
        // from the top of the screen, to the match there.
        search_keys(&mut nav, &texts, &[Key::Left, Key::Left]);
        assert_eq!((page(&nav), searched(&nav)), (2, "/page 5/5".into()));
        search_keys(&mut nav, &texts, &[Key::Char('n')]);
        assert_eq!((page(&nav), searched(&nav)), (2, "/page 3/5".into()));
        // A number typed says that `n` takes it.
        search_keys(&mut nav, &texts, &[Key::Char('2')]);
        assert_eq!(nav.status("a.pdf", 80, false).trim_end(), " 2: g page · . per row · n p match");
        // Two pages to a screen: a match on the page beside the reader's moves nothing.
        let (mut nav, texts) = searching();
        nav.grid(2, 1, 8);
        search_for(&mut nav, &texts, "/page");
        search_keys(&mut nav, &texts, &[Key::Enter, Key::Char('n')]);
        assert_eq!((nav.on_screen(), searched(&nav)), (vec![Some(0), Some(1)], "/page 2/5".into()));
        search_keys(&mut nav, &texts, &[Key::Char('n')]);
        assert_eq!((nav.on_screen(), searched(&nav)), (vec![Some(2), Some(3)], "/page 3/5".into()));
    }

    /// With a search kept, `p` goes to the match before, as `N` does and as `n` goes to the next:
    /// round the start, saying so, and a number first goes that many back. The arrows still turn
    /// pages; and once the search is cleared, `p` turns back a page again.
    #[test]
    fn p_goes_to_the_match_before_while_a_search_is_kept_as_shift_n_does() {
        let (mut nav, texts) = searching();
        search_for(&mut nav, &texts, "/page");
        search_keys(&mut nav, &texts, &[Key::Enter, Key::Char('p')]);
        assert_eq!((page(&nav), searched(&nav)), (4, "/page 5/5 · search wrapped".into()), "round to the last");
        search_keys(&mut nav, &texts, &[Key::Char('p')]);
        assert_eq!((page(&nav), searched(&nav)), (3, "/page 4/5".into()));
        search_keys(&mut nav, &texts, &[Key::Char('3'), Key::Char('p')]);
        assert_eq!((page(&nav), nav.typed.as_str(), searched(&nav)), (0, "", "/page 1/5".into()));
        // `n` on, `p` back, as `N`: the same match either way.
        search_keys(&mut nav, &texts, &[Key::Char('n'), Key::Char('n'), Key::Char('p')]);
        let with_p = (page(&nav), searched(&nav));
        search_keys(&mut nav, &texts, &[Key::Char('n'), Key::Char('N')]);
        assert_eq!((with_p.clone(), (page(&nav), searched(&nav))), ((1, "/page 2/5".into()), with_p));
        // The arrows turn pages all along, and paged away, `p` goes from the top of the screen.
        search_keys(&mut nav, &texts, &[Key::Right, Key::Right, Key::Left]);
        assert_eq!((page(&nav), searched(&nav)), (2, "/page 2/5".into()));
        search_keys(&mut nav, &texts, &[Key::Char('p')]);
        assert_eq!((page(&nav), searched(&nav)), (1, "/page 2/5".into()), "the last match before the page on screen");
        // A number typed says that `n` and `p` take it; `?` lists `p` with `N`.
        search_keys(&mut nav, &texts, &[Key::Char('2')]);
        assert_eq!(nav.status("a.pdf", 80, false).trim_end(), " 2: g page · . per row · n p match");
        search_keys(&mut nav, &texts, &[Key::Escape]);
        assert!(nav.help_lines().iter().any(|(keys, what)| keys == "n  N p  3n" && what.contains("N or p")), "{:?}", nav.help_lines());
        // Cleared: `p` turns back a page, as `n` turns one on.
        search_keys(&mut nav, &texts, &[Key::Escape, Key::Char('n'), Key::Char('n'), Key::Char('p')]);
        assert_eq!((nav.search.is_none(), page(&nav)), (true, 2));
    }

    /// Text that arrives after Enter: the first match read is gone to, once; the match the reader
    /// is on stays theirs as the rest arrives, in whatever order, its number following.
    #[test]
    fn text_arriving_out_of_order_leaves_the_match_the_reader_is_on() {
        let (mut nav, texts) = searching();
        nav.go(3);
        texts.allow(0);
        search_for(&mut nav, &texts, "/page");
        assert_eq!(searched(&nav), "/page 0… ⏎ search · esc cancel");
        search_keys(&mut nav, &texts, &[Key::Enter]);
        assert_eq!((page(&nav), searched(&nav)), (3, "/page 0…".into()), "none read yet: the view stays");
        // Read outward from the page on screen: page 4 first, then, the reader back on page 1, pages 1, 2,
        // 3 and 5.
        nav.go(0);
        texts.allow(1);
        nav.search_on(&search::tests::square);
        assert_eq!((page(&nav), searched(&nav)), (3, "/page 1/1…".into()), "the first to arrive brings the view to it");
        nav.key(Key::Char('g'));
        for (allowed, said) in [(1, "/page 2/2…"), (2, "/page 4/4…"), (1, "/page 4/5")] {
            texts.allow(allowed);
            nav.search_on(&search::tests::square);
            nav.search_on(&search::tests::square);
            assert_eq!((page(&nav), searched(&nav).as_str()), (0, said), "no match found later moves it");
        }
    }

    /// A number being typed is cleared first, as ever. Then Esc, Backspace and `u` clear a kept
    /// search before they go back or quit, and `n` turns pages again; `/` again edits the query kept.
    #[test]
    fn with_a_search_kept_esc_backspace_and_u_clear_it_before_they_go_back() {
        for key in [Key::Escape, Key::Backspace, Key::Char('u')] {
            let (mut nav, texts) = searching();
            search_for(&mut nav, &texts, "/page");
            search_keys(&mut nav, &texts, &[Key::Enter, Key::Char('n')]);
            assert_eq!(search_keys(&mut nav, &texts, &[Key::Char('7'), Key::Escape]), Step::Continue);
            assert_eq!((nav.typed.as_str(), nav.search.is_some()), ("", true), "the number first");
            assert_eq!(search_keys(&mut nav, &texts, &[key]), Step::Continue, "{key:?}");
            assert_eq!((nav.search.is_none(), page(&nav)), (true, 1), "{key:?}: cleared, the view where it was");
            assert!(!nav.status("a.pdf", 100, false).contains(" /page"));
            // `n` turns pages again, and the key goes back, or quits.
            search_keys(&mut nav, &texts, &[Key::Char('n')]);
            assert_eq!(page(&nav), 2);
            let then = if key == Key::Escape { Step::Leave } else { Step::Back };
            assert_eq!(search_keys(&mut nav, &texts, &[key]), then, "{key:?}");
        }
        // `/` again: the query kept, typed on.
        let (mut nav, texts) = searching();
        search_for(&mut nav, &texts, "/page");
        search_keys(&mut nav, &texts, &[Key::Enter, Key::Char('n'), Key::Char('/')]);
        assert_eq!((page(&nav), searched(&nav)), (1, "/page 5 ⏎ search · esc cancel".into()));
        search_for(&mut nav, &texts, " 4");
        search_keys(&mut nav, &texts, &[Key::Enter]);
        assert_eq!((page(&nav), searched(&nav)), (3, "/page 4 1/1".into()));
    }

    /// While a search is typed, every key that types a character is part of it, at most
    /// [`MAX_FILTER`] of them; Backspace takes one back; Esc leaves no search, as Enter and Backspace
    /// do with nothing typed; Ctrl-C still quits, and the keys that move do nothing.
    #[test]
    fn while_a_search_is_typed_every_key_that_types_a_character_is_text() {
        let (nav, texts) = searching();
        let mut nav = declaring(nav, true);
        assert_eq!(search_for(&mut nav, &texts, "/q nN1?u gG+-/rcd"), Step::Continue);
        assert_eq!(nav.search.as_ref().map(Search::query), Some("q nN1?u gG+-/rcd"));
        assert_eq!((nav.help, nav.typed.as_str(), nav.rerender, page(&nav)), (false, "", false, 0));
        assert_eq!(nav.settings.values.get(), Values::default(), "typed, a setting's key is a letter of the search");
        for key in [Key::Right, Key::Left, Key::Down, Key::PageDown, Key::End, Key::Char('\u{c}')] {
            assert_eq!(search_keys(&mut nav, &texts, &[key]), Step::Continue, "{key:?}");
            assert_eq!((page(&nav), nav.rerender, nav.search.as_ref().map(Search::query)), (0, false, Some("q nN1?u gG+-/rcd")), "{key:?}");
        }
        search_keys(&mut nav, &texts, &[Key::Backspace]);
        assert_eq!(nav.search.as_ref().map(Search::query), Some("q nN1?u gG+-/rc"));
        assert_eq!(search_keys(&mut nav, &texts, &[Key::Interrupt]), Step::Quit);
        assert_eq!(search_keys(&mut nav, &texts, &[Key::Escape]), Step::Continue, "Esc doesn't leave the file");
        assert!(nav.search.is_none());
        for key in [Key::Enter, Key::Backspace] {
            assert_eq!(search_keys(&mut nav, &texts, &[Key::Char('/'), key]), Step::Continue, "{key:?} goes nowhere");
            assert!(nav.search.is_none(), "{key:?}");
        }
        search_for(&mut nav, &texts, "/ab");
        search_keys(&mut nav, &texts, &[Key::Backspace, Key::Backspace]);
        assert_eq!(nav.search.as_ref().map(|search| (search.typing(), search.query())), Some((true, "")));
        search_keys(&mut nav, &texts, &[Key::Backspace]);
        assert!(nav.search.is_none());
        // Esc while the query kept is typed on leaves no search either.
        search_for(&mut nav, &texts, "/page");
        search_keys(&mut nav, &texts, &[Key::Enter, Key::Char('/'), Key::Char('x'), Key::Escape]);
        assert!(nav.search.is_none());
        search_keys(&mut nav, &texts, &[Key::Char('/')]);
        search_keys(&mut nav, &texts, &[Key::Char('é'); MAX_FILTER + 10]);
        assert_eq!(nav.search.as_ref().map(|search| search.query().chars().count()), Some(MAX_FILTER));
        for cols in 0..=200 {
            assert_eq!(chars(&nav.status("a.pdf", cols, false)), usize::from(cols));
        }
    }

    /// A source with no text to search says so at `/`, and opens no prompt; `N` without a search
    /// says there is none.
    #[test]
    fn a_source_without_text_says_no_text_to_search() {
        let mut nav = reader(2, 12, 40, 40);
        assert_eq!(nav.key(Key::Char('/')), Step::Search);
        nav.start_search(None);
        assert_eq!((nav.message.as_deref(), nav.search.is_none(), page(&nav)), (Some(NO_TEXT), true, 2));
        assert!(nav.status("scan.png", 80, false).contains(&format!("  {NO_TEXT}  ")));
        // The keys after it are the pager's.
        assert_eq!(press(&mut nav, &[Key::Char('n'), Key::Char('q')]), Step::Quit);
        assert_eq!(page(&nav), 3);
        press(&mut nav, &[Key::Char('N')]);
        assert_eq!((nav.message.as_deref(), page(&nav)), (Some(NO_SEARCH), 3));
    }

    /// The status line with a search: typed, what is typed and its count, the keys when there is
    /// room; kept, the match the reader is on, before whatever else there is to say; the right side
    /// where it was, and nothing a query holds reaching the terminal raw.
    #[test]
    fn the_status_line_shows_the_search_and_the_match_the_reader_is_on() {
        let (mut nav, texts) = searching();
        let at = column(&nav.status("a.pdf", 100, false), "page 1/5");
        search_for(&mut nav, &texts, "/Page");
        let line = nav.status("a.pdf", 100, false);
        assert!(squeezed(&line).starts_with(" a.pdf /Page 5 ⏎ search · esc cancel page 1/5 "), "{line:?}");
        assert_eq!(column(&line, "page 1/5"), at);
        let narrow = nav.status("a.pdf", 50, false);
        assert!(narrow.contains("  /Page 5  ") && !narrow.contains("cancel"), "{narrow:?}");
        search_keys(&mut nav, &texts, &[Key::Enter, Key::Char('n')]);
        nav.message = Some("page 2: content past the page's safety budget was skipped".into());
        let line = nav.status("a.pdf", 120, false);
        assert!(line.contains("  /Page 2/5  page 2: content past the page's safety budget was skipped  "), "{line:?}");
        assert_eq!(column(&nav.status("a.pdf", 100, false), "page 2/5"), at);
        search_keys(&mut nav, &texts, &[Key::Char('/')]);
        search_for(&mut nav, &texts, "\u{1b}]52;c;cHduZWQ=\u{7}");
        for cols in 0..=200 {
            let line = nav.status("a.pdf", cols, false);
            assert!(chars(&line) == usize::from(cols) && !line.chars().any(char::is_control), "{line:?}");
        }
        // Why some text was left unread comes while the search is typed, and is said with it.
        nav.message = Some("page 4's text took too long to read".into());
        let line = nav.status("a.pdf", 140, false);
        assert!(line.contains(" /Page]52;c;cHduZWQ= no match  page 4's text took too long to read  "), "{line:?}");
        // `?` lists the search's keys in a file's pages.
        let keys: Vec<String> = nav.help_lines().into_iter().map(|(keys, _)| keys).collect();
        assert!(keys.contains(&"/".to_owned()) && keys.contains(&"n  N p  3n".to_owned()), "{keys:?}");
    }

    /// Zoomed in to a page that scrolls, the match gone to is scrolled into view once the source says
    /// where it is: a third of the way down the screen; one already on screen leaves the view.
    #[test]
    fn a_match_on_a_page_that_scrolls_is_scrolled_into_view() {
        let fake = search::tests::Fake::of(&["", &format!("{}abab and ab", "x ".repeat(30))]);
        let mut nav = reader(0, 2, 20, 100);
        search_for(&mut nav, &fake, "/ab");
        search_keys(&mut nav, &fake, &[Key::Enter]);
        // The match is 60 characters into its chunk: the fake puts it 60 of the page's 100 units down,
        // 10 high.
        assert_eq!((page(&nav), nav.scroll, nav.reveal), (1, 0, Some((0.6, 0.7))));
        nav.shown(20, 100);
        assert_eq!((nav.scroll, nav.reveal), (54, None), "row 60, a third of the 20 rows from the top");
        // The next match is on screen already (rows 62 to 72): the view stays. The one after is only
        // partly (rows 69 to 79): it comes a third of the way down.
        search_keys(&mut nav, &fake, &[Key::Char('n')]);
        nav.shown(20, 100);
        assert_eq!((page(&nav), nav.scroll, searched(&nav)), (1, 54, "/ab 2/3".into()));
        search_keys(&mut nav, &fake, &[Key::Char('n')]);
        nav.shown(20, 100);
        assert_eq!(nav.scroll, 63);
        search_keys(&mut nav, &fake, &[Key::Char('N')]);
        nav.shown(20, 100);
        assert_eq!((searched(&nav), nav.scroll), ("/ab 2/3".into(), 56), "back to the second, its top above the screen");
        // Scrolled away to the top of the page, the match off screen: `N` goes from the top of the screen,
        // where none comes before, so round to the last, into view. Near the end, as far as the page scrolls.
        nav.scroll = 0;
        search_keys(&mut nav, &fake, &[Key::Char('N')]);
        nav.shown(20, 100);
        assert_eq!((searched(&nav), nav.scroll), ("/ab 3/3 · search wrapped".into(), 63));
        // A match that ends where the screen does is on it.
        nav.scroll = 60;
        nav.reveal = Some((0.7, 0.8));
        nav.shown(20, 100);
        assert_eq!(nav.scroll, 60);
        nav.reveal = Some((0.95, 1.0));
        nav.shown(20, 100);
        assert_eq!(nav.scroll, 80);
    }

    /// On a page that scrolls, "on screen" is the part of it on screen: with the reader's match
    /// scrolled off, `n` and `N` go from the top of what is shown, not from that match; and Enter
    /// goes to the first match at or after the top of what is shown, not of the page.
    #[test]
    fn on_a_page_that_scrolls_n_goes_from_the_part_on_screen() {
        // Three matches, 10, 50 and 90 of the page's 100 units down, each 10 high; 20 rows of 100 show.
        let text = format!("{0}ab{1}ab{1}ab", "x".repeat(10), "x".repeat(38));
        let fake = search::tests::Fake::of(&[&text]);
        let mut nav = reader(0, 1, 20, 100);
        search_for(&mut nav, &fake, "/ab");
        search_keys(&mut nav, &fake, &[Key::Enter]);
        nav.shown(20, 100);
        assert_eq!((searched(&nav), nav.scroll), ("/ab 1/3".into(), 0), "rows 10 to 20: on screen already");
        // Scrolled to the bottom, rows 80 to 100: the match there is the next, and the view stays.
        nav.scroll = 80;
        search_keys(&mut nav, &fake, &[Key::Char('n')]);
        nav.shown(20, 100);
        assert_eq!((searched(&nav), nav.scroll), ("/ab 3/3".into(), 80));
        // Scrolled to the middle, rows 40 to 60, the reader's match off screen below: `N` goes to
        // the last match above the top of the screen, and `n` to the first from it.
        nav.scroll = 40;
        search_keys(&mut nav, &fake, &[Key::Char('N')]);
        assert_eq!(searched(&nav), "/ab 1/3");
        nav.shown(20, 100);
        nav.scroll = 40;
        search_keys(&mut nav, &fake, &[Key::Char('n')]);
        nav.shown(20, 100);
        assert_eq!((searched(&nav), nav.scroll), ("/ab 2/3".into(), 40));
        // With the match on screen, from it, as ever.
        search_keys(&mut nav, &fake, &[Key::Char('n')]);
        assert_eq!(searched(&nav), "/ab 3/3");
        // Enter, scrolled past the first two: the first match from the top of what is shown.
        nav.shown(20, 100);
        nav.scroll = 70;
        search_keys(&mut nav, &fake, &[Key::Char('/'), Key::Enter]);
        assert_eq!(searched(&nav), "/ab 3/3");
        nav.scroll = 30;
        search_keys(&mut nav, &fake, &[Key::Char('/'), Key::Enter]);
        assert_eq!(searched(&nav), "/ab 2/3");
    }

    proptest::proptest! {
        /// Whatever is pressed with a search typed or kept, and whenever its text comes, nothing
        /// panics, the reader's page is on screen, and the status line is as wide as the window.
        #[test]
        fn a_search_never_takes_the_reader_off_the_pages(
            steps in proptest::collection::vec(
                (proptest::sample::select(vec![
                    Key::Char('/'), Key::Char('p'), Key::Char('a'), Key::Char(' '), Key::Char('3'), Key::Char('n'), Key::Char('N'),
                    Key::Char('u'), Key::Char('+'), Key::Enter, Key::Escape, Key::Backspace, Key::Right, Key::PageDown,
                 ]),
                 0usize..3),
                0..60,
            ),
        ) {
            let (mut nav, texts) = searching();
            texts.allow(0);
            for (key, allowed) in steps {
                texts.allow(allowed);
                if nav.key(key) == Step::Search {
                    nav.start_search(Some(Box::new(texts.clone())));
                }
                nav.search_on(&search::tests::square);
                nav.shown(40, 40);
                proptest::prop_assert!(nav.at < 5 && nav.on_screen().contains(&Some(nav.at)), "{nav:?}");
                proptest::prop_assert_eq!(chars(&nav.status("a.pdf", 90, false)), 90);
            }
        }
    }

    #[test]
    fn a_filter_keeps_at_most_max_filter_characters() {
        let mut nav = named(&PHOTOS);
        press(&mut nav, &[Key::Char('/')]);
        press(&mut nav, &[Key::Char('é'); MAX_FILTER + 10]);
        assert_eq!(filter_text(&nav).map(|text| text.chars().count()), Some(MAX_FILTER));
        assert_eq!(chars(&nav.status("a.png", 80, false)), 80);
    }

    #[test]
    fn a_filter_matches_letters_past_ascii_in_any_case() {
        let mut nav = named(&["Café.jpg", "CAFÉ-2.png", "cafe.png", "ÉTÉ/"]);
        type_in(&mut nav, "/CAFÉ");
        assert_eq!(matching(&nav).0, [0, 1]);
        press(&mut nav, &[Key::Escape]);
        type_in(&mut nav, "/été");
        assert_eq!(matching(&nav).0, [3]);
        press(&mut nav, &[Key::Escape]);
        type_in(&mut nav, "/cafe");
        assert_eq!(matching(&nav).0, [2], "é is not e");
    }

    /// The row of the screen the tile chosen is on.
    fn screen_row(nav: &Nav) -> usize {
        nav.at / nav.row() - nav.first() / nav.row()
    }

    #[test]
    fn the_tile_chosen_keeps_its_row_on_screen_as_the_filter_changes() {
        // Forty tiles, four to a row in three rows; every other one's name holds an x.
        let names: Vec<String> = (0..40).map(|i| format!("{}{i}", if i % 2 == 1 { "x" } else { "f" })).collect();
        let mut nav = named(&names);
        nav.grid(4, 3, 8);
        // A screen on, tile 14 on its top row.
        press(&mut nav, &[Key::PageDown, Key::Right]);
        assert_eq!((matching(&nav).1, screen_row(&nav), nav.first()), (Some(13), 0, 12));
        // Filtered, cancelled, and filtered, kept and cleared: on the top row all along, where
        // the screen moving as little as brings it on would show it lower.
        for keys in [&[Key::Char('/'), Key::Char('x')][..], &[Key::Escape], &[Key::Char('/'), Key::Char('x'), Key::Enter], &[Key::Escape]] {
            press(&mut nav, keys);
            assert_eq!((matching(&nav).1, screen_row(&nav)), (Some(13), 0), "{keys:?}");
        }
        assert!(nav.filter.is_none());
        // From the bottom row, the bottom row.
        press(&mut nav, &[Key::Char('j'), Key::Char('j')]);
        type_in(&mut nav, "/x");
        assert_eq!((matching(&nav).1, screen_row(&nav)), (Some(21), 2));
    }

    /// What a list's screen shows, and what is rendered ahead, follows the filter: only the tiles
    /// that match. They keep the size the whole list gives them.
    #[test]
    fn only_the_tiles_that_match_are_on_screen_or_worked_on_ahead() {
        let names: Vec<String> = (0..40).map(|i| format!("{}{i}", if i % 3 == 0 { "beach" } else { "city" })).collect();
        let mut nav = named(&names);
        type_in(&mut nav, "/beach");
        assert_eq!(nav.pages, (0..40).filter(|i| i % 3 == 0).collect::<Vec<_>>());
        let wanted: Vec<usize> = nav.on_screen().into_iter().flatten().chain(nav.around(AHEAD, BEHIND)).collect();
        assert_eq!(wanted, [0, 3, 6, 9, 12, 15, 18, 21, 24, 27, 30, 33, 36, 39]);
        assert_eq!(nav.listed(), 40);
        press(&mut nav, &[Key::Enter]);
        assert_eq!(nav.listed(), 40);
    }

    proptest::proptest! {
        /// Whatever is typed into a list's filter, and whatever else is pressed, the tiles moved
        /// among are those given whose names hold the filter, in their order (all of them without
        /// one), and the tile chosen is one of them, on screen.
        #[test]
        fn a_filter_keeps_the_tiles_that_match_in_order_and_the_one_chosen_on_screen(
            names in proptest::collection::vec("[abAB/]{0,3}", 1..30),
            given in proptest::option::of(proptest::collection::vec(0usize..30, 1..30)),
            steps in proptest::collection::vec(
                (proptest::sample::select(vec![
                    Key::Char('/'), Key::Char('a'), Key::Char('B'), Key::Char('q'), Key::Char('2'), Key::Char('u'), Key::Char(' '),
                    Key::Char('+'), Key::Char('-'), Key::Enter, Key::Escape, Key::Backspace, Key::Left, Key::Right, Key::Down,
                    Key::Up, Key::End, Key::Home, Key::PageDown,
                 ]),
                 proptest::option::of((1u16..=8, 1u16..=6))),
                0..80,
            ),
        ) {
            let count = names.len();
            let given: Vec<usize> = given.map_or_else(|| (0..count).collect(), |g| g.into_iter().map(|t| t % count).collect());
            let mut nav = Nav { list: true, names: names.iter().map(|name| fold(name)).collect(), ..Nav::new(given.clone(), count, Zoom::Auto) };
            for (key, draw) in steps {
                if nav.key(key) == Step::Quit {
                    break;
                }
                if let Some((most, rows)) = draw {
                    let per_row = match nav.zoom { Zoom::PerRow(n) => n.min(most), _ => 4.min(most) };
                    nav.grid(per_row, rows, most);
                }
                let text = nav.filter.as_ref().map(|filter| fold(&filter.text));
                let expected: Vec<usize> = given.iter().copied().filter(|t| text.as_ref().is_none_or(|text| fold(&names[*t]).contains(text.as_str()))).collect();
                proptest::prop_assert_eq!(&nav.pages, &expected);
                if let Some(filter) = &nav.filter {
                    proptest::prop_assert_eq!(&filter.given, &given);
                }
                let first = nav.first();
                proptest::prop_assert!(nav.pages.is_empty() || (nav.at >= first && nav.at < first + nav.per_screen() && nav.at < nav.pages.len()), "{nav:?}");
                proptest::prop_assert_eq!(first % nav.row(), 0);
            }
        }
    }

    proptest::proptest! {
        /// In a list, whatever the reader presses and however the draws lay the tiles out, the
        /// tile chosen is on screen, and the screen starts a row.
        #[test]
        fn the_tile_chosen_is_always_on_screen(
            count in 1usize..60,
            steps in proptest::collection::vec(
                (proptest::sample::select(vec![Key::Left, Key::Right, Key::Up, Key::Down, Key::Char(' '), Key::Char('b'), Key::Home, Key::End, Key::Char('+'), Key::Char('-'), Key::Char('7'), Key::Enter]),
                 proptest::option::of((1u16..=8, 1u16..=6))),
                0..60,
            ),
        ) {
            let mut nav = list(count);
            for (key, draw) in steps {
                nav.key(key);
                if let Some((most, rows)) = draw {
                    let per_row = match nav.zoom { Zoom::PerRow(n) => n.min(most), _ => 4.min(most) };
                    nav.grid(per_row, rows, most);
                }
                let on_screen = nav.on_screen();
                proptest::prop_assert!(nav.at < count && on_screen.contains(&Some(nav.at)), "{nav:?}");
                proptest::prop_assert_eq!(nav.first() % nav.row(), 0);
            }
        }

        /// Whatever the reader presses, zooming included, and however the draws in between lay
        /// the pages out, the reader's page is on screen, and the screen starts a row of pages.
        #[test]
        fn the_readers_page_is_always_on_screen(
            count in 1usize..40,
            steps in proptest::collection::vec(
                (proptest::sample::select(vec![' ', 'b', 'j', 'k', 'n', 'p', 'g', 'G', '+', '-', '=', '_']),
                 proptest::option::of((1u16..=8, 1u16..=8, 1u16..=8))),
                0..40,
            ),
        ) {
            let mut nav = reader(0, count, 40, 40);
            for (key, draw) in steps {
                nav.key(Key::Char(key));
                if let Some((auto, rows, most)) = draw {
                    // As a draw does: the zoom asked, as far as the window takes.
                    let (per_row, rows) = match nav.zoom {
                        Zoom::Width => (1, 1),
                        Zoom::PerRow(n) => (n.min(most), rows),
                        Zoom::Auto => (auto.min(2), rows),
                    };
                    nav.grid(per_row, rows, most);
                    nav.shown(40, 40);
                }
                let on_screen = nav.on_screen();
                proptest::prop_assert!(nav.at < count && on_screen.contains(&Some(nav.at)), "{nav:?}");
                proptest::prop_assert_eq!(nav.first() % nav.row(), 0);
                proptest::prop_assert_eq!(on_screen.len(), nav.per_screen());
            }
        }
    }
}
