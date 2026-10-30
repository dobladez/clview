//! `view --pager`: pages on the alternate screen, like `less`. One at a time, or two side by side
//! when the window is wide enough, turning two at a time like a book's; zoomed out, rows of
//! them; zoomed in, one as wide as the window. Several files, or a directory, are a list of
//! tiles, one chosen at a time, which Enter opens: a [`Level`] of its own, which Esc leaves.
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
//! `README.md` beside this file has the details.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::encode::PngEffort;
use crate::source::{Container, Frame, Pool, Source};

use super::files::{Files, Written};
use super::fit::{self, Area, Cell, FALLBACK_CELL, Fit, Grid, Layout, Zoom};
use super::keys::{self, Key};
use super::kitty::{self, Rect};
use super::tty::{RawMode, Tty, WindowSize};
use super::{
    Contents, FALLBACK_SIZE, Item, Output, Pages, QUIET, Shown, Terminal, chars, checked, cut, label, left_out_note, page_layout, page_size, png,
    printable, request, tmux,
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
const LIST_KEYS: [&str; 3] = ["arrows move · ⏎ open · + - zoom · q quit", "arrows · ⏎ open · q quit", "⏎ open · q quit"];
const OPENED_KEYS: [&str; 3] = ["esc back · space b · n p · + - zoom · q quit", "esc back · n p · q quit", "esc back · q quit"];
const KEYS: [&str; 3] = ["space b j k · n p · g G · 12⏎ · + - zoom · q quit", "n p · + - zoom · q quit", "n p · q quit"];
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
    ("r  Ctrl-L", "render the pages again"),
];
/// In a list:
const LIST_HELP: &[(&str, &str)] = &[
    ("← →  h l", "the one before or after"),
    ("↑ ↓  k j", "the one above or below"),
    ("space b  n p", "a screen on or back"),
    ("g G  Home End", "the first or the last"),
    ("12 Enter", "the 12th"),
    ("Enter", "open it: a file's pages, or a directory's files"),
    ("+ -", "bigger, fewer to a row; smaller, more to a row"),
    ("r  Ctrl-L", "render again"),
];
/// Leaving, gone into from a list or not, and these keys themselves.
const BACK_HELP: &[(&str, &str)] = &[("Esc  Backspace  u", "back to the list"), ("q  Ctrl-C", "quit")];
const QUIT_HELP: &[(&str, &str)] = &[("q  Esc  Ctrl-C", "quit")];
const HELP_HELP: (&str, &str) = ("?", "these keys: any key goes back");

/// Page through what is `shown` until the user quits, `note` on the status line first. Image ids
/// are taken in turn from `first_id` on ([`super::kitty::IdColor::id`]). A container gone into
/// from a list is listed by `open_container`. Returns what the reader should know once the screen
/// is back: why the files those containers left out were.
pub(super) fn run(
    tty: &Tty,
    shown: Shown,
    note: Option<String>,
    terminal: &Terminal,
    zoom: Zoom,
    first_id: u64,
    open_container: &dyn Fn(&dyn Container) -> Contents,
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
    let mut level = Level::new(0, shown, zoom, false);
    level.nav.message = note;
    let mut pager = Pager {
        screen: Screen::enter(tty)?,
        tty,
        level,
        parents: Vec::new(),
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
        tmux_check: watch.as_ref().map(|_| Instant::now() + TMUX_EVERY),
        watch,
        hidden: false,
        transmitted: 0,
        resends: 0,
        budget: BUDGET,
        files: None,
        stalled_files: None,
        placed_at: None,
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
        } else if !pager.wanted.is_empty() && !pager.hidden {
            POLL
        } else {
            TICK
        };
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
                redraw = true;
                match pager.level.nav.key(key) {
                    Step::Continue => {}
                    // Esc at the top, where there is nothing to go back to, quits.
                    Step::Quit => quit = true,
                    Step::Leave if pager.parents.is_empty() => quit = true,
                    Step::Leave | Step::Back => pager.back()?,
                    Step::Open(at) => pager.open(at, open_container)?,
                }
                if quit {
                    break;
                }
            }
            if quit {
                pager.await_answers();
                break;
            }
        } else if read.is_none() {
            // Nothing typed: redraw if the window changed size.
            let now = tty.size();
            redraw = now != size;
            size = now;
        }
        let lost = pager.lost_files()?;
        // Nothing goes through tmux while the pane is hidden: it would be thrown away.
        let arrived = !pager.hidden && pager.receive()?;
        if arrived || redraw || lost {
            pager.draw()?;
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
    /// As asked: `--zoom`, then `+` and `-`.
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
            per_row: 1,
            grid_rows: 1,
            most_per_row: fit::MAX_PER_ROW,
            visible: 1,
            max_scroll: 0,
            scroll: 0,
            typed: String::new(),
            message: None,
            hint: true,
            help: false,
            rerender: false,
        }
    }

    /// What `pages` are called here: a list's are its tiles.
    fn unit(&self) -> &'static str {
        if self.list { "tile" } else { "page" }
    }

    /// What `?` shows here.
    fn help_lines(&self) -> Vec<(&'static str, &'static str)> {
        let keys = if self.list { LIST_HELP } else { PAGE_HELP };
        let leave = if self.nested { BACK_HELP } else { QUIT_HELP };
        keys.iter().chain(leave).copied().chain([HELP_HELP]).collect()
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
        // Any key leaves the keys, but Ctrl-C, which still quits.
        if std::mem::take(&mut self.help) && key != Key::Interrupt {
            return Step::Continue;
        }
        let typing = !self.typed.is_empty();
        match key {
            Key::Char('?') => self.help = true,
            Key::Char('q') | Key::Interrupt => return Step::Quit,
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
            _ if self.list => return self.list_key(key),
            Key::Char(' ') | Key::PageDown => self.forward(self.visible.max(1), self.per_screen()),
            Key::Char('b') | Key::PageUp => self.back(self.visible.max(1), self.per_screen()),
            Key::Char('j') | Key::Down | Key::Enter => self.forward(STEP, self.row()),
            Key::Char('k') | Key::Up => self.back(STEP, self.row()),
            Key::Char('n') | Key::Right => self.go(self.first().saturating_add(self.per_screen())),
            Key::Char('p') | Key::Left => self.go(self.first().saturating_sub(self.per_screen())),
            Key::Char('g') | Key::Home => self.go(0),
            Key::Char('G') | Key::End => self.go(usize::MAX),
            // `=` and `_` are the same keys as `+` and `-`, with Shift or without.
            Key::Char('+' | '=') => self.zoom_to(self.zoom.zoomed_in(self.per_row), "fully zoomed in"),
            Key::Char('-' | '_') => self.zoom_to(self.zoom.zoomed_out(self.per_row, self.most_per_row), "fully zoomed out"),
            Key::Char('r' | '\u{c}') => self.rerender = true,
            _ => {}
        }
        Step::Continue
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
            // As wide as the window is one to a row: a tile never scrolls.
            Key::Char('+' | '=') => {
                let zoom = (self.per_row > 1).then(|| Zoom::PerRow(self.per_row.saturating_sub(1)));
                self.zoom_to(zoom, "fully zoomed in");
            }
            Key::Char('-' | '_') => self.zoom_to(self.zoom.zoomed_out(self.per_row, self.most_per_row), "fully zoomed out"),
            Key::Char('r' | '\u{c}') => self.rerender = true,
            _ => {}
        }
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
    /// tile.
    fn go_typed(&mut self) {
        let wanted = self.typed.parse::<usize>().ok().and_then(|n| n.checked_sub(1));
        let unit = self.unit();
        match wanted.map(|page| (page, self.pages.iter().position(|p| *p == page))) {
            Some((_, Some(index))) if self.list => self.select(index),
            Some((_, Some(index))) => self.go(index),
            Some((page, None)) if page < self.count => self.message = Some(format!("{unit} {} isn't one of the {unit}s given", self.typed)),
            _ => self.message = Some(format!("no {unit} {} (1–{})", self.typed, self.count)),
        }
        self.typed.clear();
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
    /// in range.
    fn shown(&mut self, visible: u16, rows: u16) {
        self.visible = visible;
        self.max_scroll = rows.saturating_sub(visible);
        self.scroll = self.scroll.min(self.max_scroll);
    }

    /// The status line for a window `cols` wide: exactly that many characters, none of them a
    /// control character (the name and the messages may come from anywhere). The right side is
    /// at the same place whatever the reader does: where the reader is, padded to the widest it
    /// gets in this document, the zoom, and the key for the keys; `…` before them while pages
    /// render. A narrow window drops the zoom, then the key. On the left, the name, cut short in
    /// its middle to leave room for a message, which ends two spaces before the right side.
    /// Until the first key, the message is the reminder of the keys, as much of it as fits.
    fn status(&self, name: &str, cols: u16, rendering: bool) -> String {
        let width = usize::from(cols);
        let (place, widest) = self.place();
        let zoom = if self.zoom == Zoom::Width { "width".to_owned() } else { format!("{} per row", self.per_row) };
        let slot = if rendering { '…' } else { ' ' };
        let rights = [
            format!("{slot} {place:<widest$} · {zoom:<ZOOM_WIDTH$} · {HELP} "),
            format!("{slot} {place:<widest$} · {HELP} "),
            format!("{slot} {place} "),
        ];
        let right = rights.iter().find(|r| chars(r) <= width).cloned().unwrap_or_else(|| rights[2].chars().take(width).collect());
        let room = width.saturating_sub(chars(&right));
        let said = match (&self.message, self.typed.is_empty()) {
            (Some(message), _) => message.clone(),
            (None, false) => format!("go to {} {}", self.unit(), self.typed),
            // The keys leave the name room for its first characters at least.
            (None, true) if self.hint => {
                let fits = |keys: &&&str| chars(keys).saturating_add(chars(name).min(16)).saturating_add(5) <= room;
                let keys = match (self.list, self.nested) {
                    (true, _) => &LIST_KEYS,
                    (false, true) => &OPENED_KEYS,
                    (false, false) => &KEYS,
                };
                keys.iter().find(fits).map_or_else(String::new, |keys| (*keys).to_owned())
            }
            (None, true) => String::new(),
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

    /// Where the reader is, for the status line, and the most characters it takes in this
    /// document: the pages on screen, in a list the tile chosen, of how many; and where among the
    /// pages given when they are not all.
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
        if given != count {
            place.push_str(&format!(" · {} of {given}", at.saturating_add(1)));
            widest = widest.saturating_add(digits(given).saturating_mul(2).saturating_add(7));
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

/// A page rendered for the window and sent to the terminal under an image id of its own.
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
}

impl Sent {
    /// What it takes up in the terminal: its pixels, four bytes each.
    fn bytes(&self) -> u64 {
        u64::from(self.width).saturating_mul(u64::from(self.height)).saturating_mul(4)
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
}

impl Level {
    /// What is `shown`, at `zoom`; `nested` when opened from a list. A list gone into from a list
    /// needs no reminder of the keys: they are the list's.
    fn new(id: u64, shown: Shown, zoom: Zoom, nested: bool) -> Self {
        let list = shown.is_list();
        let Shown { pages: Pages { source, name, pages }, items } = shown;
        let nav = Nav { list, nested, hint: !(list && nested), ..Nav::new(pages, source.pages(), zoom) };
        Self { id, pool: source.pool(THREADS), source, nav, items, name }
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
}

/// A page ready to send.
enum Payload {
    Png(Vec<u8>),
    /// Straight RGBA pixels in a temporary file.
    File(PathBuf),
}

impl Drop for Pager<'_> {
    fn drop(&mut self) {
        let mut out = Vec::new();
        for sent in &self.sent {
            kitty::delete(&mut out, sent.id, self.output.wrap());
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
        // The last row is the status line.
        let area_rows = size.rows.saturating_sub(1).max(1);
        let (max_cols, max_rows) = self.output.limits(size.cols);
        let wrap = self.output.wrap();
        let mut out = Vec::new();
        // Pages made for another window, or asked to be made again, go.
        if std::mem::take(&mut self.level.nav.rerender) || self.size != Some(size) {
            for sent in self.sent.drain(..) {
                kitty::delete(&mut out, sent.id, wrap);
            }
            self.wanted.clear();
            self.failed.clear();
            self.placed.clear();
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
            if let Some(s) = self.sent.iter().find(|s| s.view == v) {
                near = near.saturating_add(s.bytes());
                continue;
            }
            let next = match self.wanted.iter().position(|w| w.view == v) {
                Some(at) => Ok(self.wanted.swap_remove(at)),
                None => page_layout(&*source, v.page, v.fit, v.cols, v.rows, cell, max_rows).map(|layout| {
                    self.tag = self.tag.wrapping_add(1);
                    Wanted { view: v, layout, tag: self.tag }
                }),
            };
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
        self.wrote(0);
        self.tty.write_all(&out)
    }

    /// How the pages share the window's `area`: as the reader zoomed, from where the reader is,
    /// as far as the window takes ([`fit::grid`]).
    fn grid(&mut self, area: Area) -> Grid {
        let (nav, source) = (&self.level.nav, &*self.level.source);
        let grid = match nav.list {
            true => fit::tiles(nav.zoom, nav.pages.len(), area),
            false => fit::grid(nav.zoom, nav.pages.len(), nav.at, |i| nav.pages.get(i).and_then(|p| page_size(source, *p)), area),
        };
        self.level.nav.grid(grid.per_row, grid.rows, grid.most);
        grid
    }

    /// Open the list's item at `at`: a file to its pages, a container, listed by `open_container`,
    /// to a list of its own. What was on screen stops rendering, and waits to be gone back to.
    fn open(&mut self, at: usize, open_container: &dyn Fn(&dyn Container) -> Contents) -> Result<(), String> {
        let id = self.levels;
        let level = match self.level.items.get(at) {
            Some(Item::File(pages)) => {
                let pages = Pages { name: format!("{}{}", self.level.name, pages.name), ..pages.clone() };
                Level::new(id, Shown { pages, items: Vec::new() }, Zoom::Auto, true)
            }
            Some(Item::Container { name, container }) => {
                // Its files are opened now, which takes a while for large ones: said first.
                let (name, container) = (format!("{}{name}", self.level.name), container.clone());
                self.level.nav.message = Some(format!("opening {name}…"));
                self.draw()?;
                self.level.nav.message = None;
                let Contents { items, left_out, warnings } = open_container(&*container);
                self.later.extend(warnings.into_iter().chain(left_out.iter().map(|(_, why)| why.clone())));
                if items.is_empty() {
                    self.level.nav.message = Some(format!("nothing in {name} that {} shows", crate::NAME));
                    return Ok(());
                }
                let mut level = Level::new(id, Shown::list(items, name), self.level.nav.zoom, true);
                level.nav.message = left_out_note(&left_out);
                level
            }
            None => return Ok(()),
        };
        self.levels = self.levels.wrapping_add(1);
        // A fresh pool, holding nothing: the old one lets go of what its files rendered with,
        // which for a PDF is the document parsed again.
        self.level.pool = self.level.source.pool(THREADS);
        self.wanted.clear();
        let parent = std::mem::replace(&mut self.level, level);
        self.parents.push(parent);
        Ok(())
    }

    /// Back to what the reader opened this from, as they left it. What this showed goes from the
    /// terminal: going back to it opens it anew.
    fn back(&mut self) -> Result<(), String> {
        let Some(parent) = self.parents.pop() else { return Ok(()) };
        let gone = std::mem::replace(&mut self.level, parent).id;
        let mut out = Vec::new();
        let wrap = self.output.wrap();
        for sent in self.sent.iter().filter(|s| s.view.level == gone) {
            kitty::delete(&mut out, sent.id, wrap);
        }
        let deleted: Vec<u32> = self.sent.iter().filter(|s| s.view.level == gone).map(|s| s.id).collect();
        self.sent.retain(|s| s.view.level != gone);
        self.placed.retain(|(id, _)| !deleted.contains(id));
        self.wanted.clear();
        self.wrote(0);
        self.tty.write_all(&out)
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

    /// Before the screen is put back: wait a moment for the answers to the last placements, so
    /// none of them reaches the command line.
    fn await_answers(&self) {
        let Some(at) = self.placed_at else { return };
        let deadline = at + ANSWER_WAIT;
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
        // Erasing text never erases images (only clearing the whole screen does, by the
        // protocol): placements are what show and move them.
        for row in 1..=size.rows {
            out.extend_from_slice(format!("\x1b[{row};1H\x1b[2K").as_bytes());
        }
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
            let Some(s) = view.and_then(|v| self.sent.iter().find(|s| s.view == v)) else { continue };
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
            cursor(out, row, left.saturating_add((slot % per_row).saturating_mul(across)));
            let name = printable(&label(item.name(), grid.cols));
            let (on, off) = if chosen == Some(v.page) { ("\x1b[7m", "\x1b[0m") } else { ("", "") };
            out.extend_from_slice(format!("{on}{name}{off}").as_bytes());
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
            // Otherwise a late result for a page no longer wanted.
            let Some(at) = self.wanted.iter().position(|w| w.tag == page.request.tag) else { continue };
            let wanted = self.wanted.remove(at);
            any = true;
            on_screen |= self.on_screen.contains(&Some(wanted.view));
            let id = self.new_id();
            let (width, height) = (page.width, page.height);
            match checked(&page).and_then(|warning| Ok((warning, self.payload(id, page)?))) {
                Ok((warning, Some(payload))) => {
                    let mut out = Vec::new();
                    let bytes = u64::from(width).saturating_mul(u64::from(height)).saturating_mul(4);
                    for gone in make_room(&mut self.sent, bytes, self.budget, self.kept, &self.on_screen) {
                        kitty::delete(&mut out, gone, wrap);
                        self.placed.retain(|(placed, _)| *placed != gone);
                    }
                    let (cols, rows) = (wanted.layout.cols, wanted.layout.rows);
                    match &payload {
                        Payload::Png(png) => kitty::transmit(&mut out, id, png, wrap),
                        Payload::File(path) => kitty::transmit_file(&mut out, id, path, width, height, wrap),
                    }
                    if let Output::Tmux(_) = self.output {
                        kitty::place_virtual(&mut out, id, cols, rows, wrap);
                    }
                    self.wrote(1);
                    self.tty.write_all(&out)?;
                    self.sent.push(Sent { view: wanted.view, id, cols, rows, width, height, warning });
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
    /// them, otherwise as a PNG. A file that can't be written ends the files, and says why.
    /// `None` when the files' thread stopped answering with the page.
    fn payload(&mut self, id: u32, page: Frame) -> Result<Option<Payload>, String> {
        let now = Instant::now();
        let mut page = page;
        if let Some(files) = self.files.as_mut()
            && files.ready(now)
        {
            match files.write(id, page, now) {
                Written::File(path) => return Ok(Some(Payload::File(path))),
                Written::Failed(e, back) => {
                    self.level.nav.message = Some(format!("{e}; sending pages in the terminal stream instead"));
                    page = back;
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
            kitty::delete(&mut out, *id, self.output.wrap());
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

    /// Tell the pool which renders are still wanted, most urgent first: it keeps those and
    /// cancels the rest.
    fn requeue(&self) {
        self.level.pool.set_queue(self.wanted.iter().map(|w| request(w.view.page, &w.layout, w.tag)).collect());
    }
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

    #[test]
    fn a_lists_status_line_says_which_tile_of_how_many() {
        let mut nav = tiles(10, 3, 2);
        let first = nav.status("a.png", 100, false);
        assert!(first.contains(&format!("  {}  ", LIST_KEYS[0])) && first.ends_with("  1/10  · 3 per row · ? help "), "{first:?}");
        press(&mut nav, &[Key::End]);
        assert!(nav.status("z.png", 100, false).ends_with("  10/10 · 3 per row · ? help "));
        press(&mut nav, &[Key::Char('4')]);
        assert!(nav.status("z.png", 100, false).contains("  go to tile 4  "));
        // Opened from a list, a file's pages remind of the way back, and its keys list it.
        let opened = Nav { nested: true, ..reader(0, 3, 40, 40) };
        assert!(opened.status("a.pdf", 100, false).contains(&format!("  {}  ", OPENED_KEYS[0])));
        assert!(opened.help_lines().contains(&("Esc  Backspace  u", "back to the list")));
        assert!(reader(0, 3, 40, 40).help_lines().contains(&("q  Esc  Ctrl-C", "quit")));
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
        // `f` and `d`, which fitted the width and turned two-up off, are keys no more.
        press(&mut nav, &[Key::Char('f'), Key::Char('d')]);
        assert_eq!((nav.zoom, nav.per_row), (Zoom::Width, 1));
        press(&mut nav, &[Key::Char('r')]);
        assert!(nav.rerender);
        assert_eq!(press(&mut nav, &[Key::Char('q')]), Step::Quit);
        assert_eq!(press(&mut nav, &[Key::Interrupt]), Step::Quit);
        assert_eq!(press(&mut nav, &[Key::Char('x'), Key::PageUp, Key::End]), Step::Continue);
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
        assert!(typing.status("a.pdf", 80, false).contains("  go to page 7  "));
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
        assert!(nav.status("report.pdf", 70, false).contains(&format!("  {}  ", KEYS[2])));
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
        Sent { view, id, cols: 50, rows: 40, width: 1024, height: mb * 256, warning: warning.map(String::from) }
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
        let shown = Sent { view, id: 1, cols: 50, rows: 100, width: 1000, height: 2000, warning: None };
        assert_eq!(source_rows(&shown, 0, 30), Rect { x: 0, y: 0, w: 1000, h: 600 });
        assert_eq!(source_rows(&shown, 70, 30), Rect { x: 0, y: 1400, w: 1000, h: 600 });
        // Never past the bottom, whatever it is asked.
        let past = source_rows(&shown, u16::MAX, 30);
        assert_eq!((past.y, past.h), (2000, 0));
        let empty = Sent { rows: 0, height: 0, ..shown };
        assert_eq!(source_rows(&empty, 5, 5), Rect { x: 0, y: 0, w: 1000, h: 0 });
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
