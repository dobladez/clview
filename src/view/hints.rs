//! The labels over a page's links (`f`, `F`), as the Vimium browser extension shows them: a short
//! code of home-row letters on each link on screen, and what is typed of one. No IO of its own: the
//! links come from the source, a page at a time ([`Hints::told`]), the pager says where the pages
//! are on screen ([`Shown`]) and draws what this says ([`Hints::visible`]).
//!
//! The labels are made once, when every page on screen has answered or the wait is over
//! ([`Hints::settle`]), and never renumbered: a page that answers later has none. Each is at the top
//! left of the part of its link on screen, in reading order, every code the same length
//! ([`codes`]).
//!
//! Directly, a label is drawn as placements of parts of one image ([`Sheet`]), the letters in
//! yellow boxes, drawn here from a bitmap font
//! ([ADR-0035](../../docs/adr/0035-link-labels-as-placements-of-one-image-of-letters.md)). Where
//! pages are cells (inside tmux, in text cells), it is written over the page's cells as
//! characters ([`Hints::runs`]).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::source::{Ask, Link};

/// The letters of the labels, Vimium's own: the home row first.
pub(super) const LETTERS: &str = "sadfjklewcmpgh";
/// The most links labelled at once, in reading order: the rest are left out, and said.
pub(super) const MAX_LABELS: usize = 1000;
/// How long the labels wait for the pages' links: a page that hasn't answered by then has none.
pub(super) const WAIT: Duration = Duration::from_secs(2);
/// What the status line says while the pages' links are waited for.
pub(super) const READING: &str = "reading links…";
/// What it says of a screen with no link to label, or a file with none at all.
pub(super) const NONE: &str = "no links on screen";

/// What the labels are for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Aim {
    /// `f`: the link chosen is followed.
    Follow,
    /// `F`: its address is copied.
    #[cfg_attr(not(test), expect(dead_code, reason = "`F` comes with Spec-0011's last task"))]
    Copy,
}

/// A page as it is on screen: which page, the cell its top left is at, the cells it covers there,
/// how many of its rows of cells are scrolled off above them, of how many in all, and its size
/// in its own unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Shown {
    pub(super) page: usize,
    pub(super) left: u16,
    pub(super) top: u16,
    pub(super) cols: u16,
    pub(super) rows: u16,
    pub(super) scroll: u16,
    pub(super) of: u16,
    pub(super) size: (f32, f32),
}

/// Where on screen a label is: its cell, and how far into it, in pixels, where the terminal
/// draws at pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct At {
    pub(super) row: u16,
    pub(super) col: u16,
    pub(super) x: u16,
    pub(super) y: u16,
}

/// A link's label: its code, where it is on screen, and the link.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Label {
    pub(super) code: String,
    pub(super) at: At,
    pub(super) link: Link,
}

/// Cells on a row of the screen that labels are written over, side by side: the row, the first
/// cell's column, and each cell's letter, with whether it is typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Run {
    pub(super) row: u16,
    pub(super) col: u16,
    pub(super) letters: Vec<(char, bool)>,
}

/// What became of a page's links.
#[derive(Debug)]
enum Asked {
    Waiting,
    /// Its links, checked, and how many more it has.
    Told(Vec<Link>, usize),
    Failed,
}

/// What a key did to the labels.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Typed {
    /// More than one label is left: they are drawn again.
    More,
    /// The labels go, and nothing else happens.
    Dropped,
    /// One label is left: its link, chosen.
    Chosen(Box<Link>),
}

/// The labels `f` or `F` asked for: waited for, then shown, with what is typed of one.
#[derive(Debug)]
pub(super) struct Hints {
    aim: Aim,
    /// The pages on screen when they were asked for, each once, and what became of its links.
    pages: Vec<(usize, Asked)>,
    since: Instant,
    /// The labels, once made.
    labels: Option<Vec<Label>>,
    typed: String,
    /// What the status line says of them beside what is typed: pages unread, links left out.
    notes: Vec<String>,
}

impl Hints {
    /// Labels for the links of `pages`, the pages on screen, asked for at `now`.
    pub(super) fn new(aim: Aim, pages: &[usize], now: Instant) -> Self {
        let mut asked: Vec<(usize, Asked)> = Vec::new();
        for page in pages {
            if !asked.iter().any(|(listed, _)| listed == page) {
                asked.push((*page, Asked::Waiting));
            }
        }
        Self { aim, pages: asked, since: now, labels: None, typed: String::new(), notes: Vec::new() }
    }

    pub(super) fn aim(&self) -> Aim {
        self.aim
    }

    /// Whether the pages' links are still waited for: no label is made yet.
    pub(super) fn waiting(&self) -> bool {
        self.labels.is_none()
    }

    /// What to ask the source: the links of the pages not yet answered, none once the labels are
    /// made.
    pub(super) fn wanted(&self) -> Vec<Ask> {
        if !self.waiting() {
            return Vec::new();
        }
        self.pages.iter().filter(|(_, asked)| matches!(asked, Asked::Waiting)).map(|(page, _)| Ask::Page(*page)).collect()
    }

    /// Page `page`'s links as the source gave them, checked, and how many more it has; or that it
    /// has none to give. One that comes once the labels are made, or for a page not asked, changes
    /// nothing.
    pub(super) fn told(&mut self, page: usize, links: Result<Vec<Link>, String>, more: usize) {
        if !self.waiting() {
            return;
        }
        if let Some((_, asked)) = self.pages.iter_mut().find(|(listed, asked)| *listed == page && matches!(asked, Asked::Waiting)) {
            *asked = links.map_or(Asked::Failed, |links| Asked::Told(links, more));
        }
    }

    /// Whether the labels are to be made at `now`: every page has answered, or the wait is over.
    pub(super) fn due(&self, now: Instant) -> bool {
        self.waiting() && (self.pages.iter().all(|(_, asked)| !matches!(asked, Asked::Waiting)) || self.late(now))
    }

    /// Whether the wait for the pages' links is over at `now`.
    pub(super) fn late(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.since) >= WAIT
    }

    /// What is typed of a label so far.
    pub(super) fn typed(&self) -> &str {
        &self.typed
    }

    /// Make the labels, from what came: one for each link at least partly on a page `shown` on
    /// screen, at most [`MAX_LABELS`], in reading order (the pages as they are on screen, then down
    /// each, then across), each at the top left of the part of its link that shows, where a cell is
    /// `cell` pixels. A page given twice on screen has its links labelled in both places. How many
    /// labels there are.
    pub(super) fn settle(&mut self, shown: &[Shown], cell: (f32, f32)) -> usize {
        // Each with where it is on its page, in the page's unit: what the order goes by, finer than
        // the screen's pixels.
        let mut found: Vec<(usize, (f32, f32), At, &Link)> = Vec::new();
        let mut over = 0usize;
        for (slot, page) in shown.iter().enumerate() {
            let Some((_, Asked::Told(links, more))) = self.pages.iter().find(|(listed, _)| *listed == page.page) else { continue };
            over = over.saturating_add(*more);
            found.extend(links.iter().filter_map(|link| spot(page, link, cell).map(|(on_page, at)| (slot, on_page, at, link))));
        }
        found.sort_by(|a, b| (a.0, a.1).partial_cmp(&(b.0, b.1)).unwrap_or(std::cmp::Ordering::Equal));
        over = over.saturating_add(found.len().saturating_sub(MAX_LABELS));
        found.truncate(MAX_LABELS);
        let labels: Vec<Label> =
            codes(found.len()).zip(&found).map(|(code, (_, _, at, link))| Label { code, at: *at, link: (*link).clone() }).collect();
        let unread: Vec<String> =
            self.pages.iter().filter(|(_, asked)| !matches!(asked, Asked::Told(..))).map(|(page, _)| page.saturating_add(1).to_string()).collect();
        self.notes.clear();
        match unread.as_slice() {
            [] => {}
            [page] => self.notes.push(format!("links on page {page} unread")),
            pages => self.notes.push(format!("links on pages {} unread", pages.join(", "))),
        }
        match over {
            0 => {}
            1 => self.notes.push("1 link left out".into()),
            n => self.notes.push(format!("{n} links left out")),
        }
        // What was read is let go of: the labels hold their links.
        for (_, asked) in &mut self.pages {
            if let Asked::Told(links, _) = asked {
                links.clear();
            }
        }
        let count = labels.len();
        self.labels = Some(labels);
        count
    }

    /// What `c`, typed, does: with a label's next letter, the labels that no longer match go, and
    /// with one left its link is chosen; a character in no label drops them all.
    pub(super) fn letter(&mut self, c: char) -> Typed {
        let mut typed = self.typed.clone();
        typed.push(c);
        let mut matching = self.labels.as_deref().unwrap_or_default().iter().filter(|label| label.code.starts_with(&typed));
        match (matching.next(), matching.next()) {
            (None, _) => Typed::Dropped,
            (Some(only), None) => Typed::Chosen(Box::new(only.link.clone())),
            (Some(_), Some(_)) => {
                self.typed = typed;
                Typed::More
            }
        }
    }

    /// Take the last letter typed back: whether there was one.
    pub(super) fn backspace(&mut self) -> bool {
        self.typed.pop().is_some()
    }

    /// The labels to draw: those what is typed still matches, each with how many of its letters
    /// are typed. None while they are waited for.
    pub(super) fn visible(&self) -> impl Iterator<Item = (&Label, usize)> {
        let typed = self.typed.chars().count();
        self.labels.as_deref().unwrap_or_default().iter().filter(|label| label.code.starts_with(&self.typed)).map(move |label| (label, typed))
    }

    /// The labels as written over cells, where pages are cells (inside tmux, in text cells): each
    /// run of cells on a row that labels cover, left to right, with its letters and whether each
    /// is typed. A label is kept within the cells of the page it is on, of those `shown`, starting
    /// further left if it must; where two share a cell, the later one's letter shows.
    pub(super) fn runs(&self, shown: &[Shown]) -> Vec<Run> {
        let mut cells: BTreeMap<(u16, u16), (char, bool)> = BTreeMap::new();
        for (label, typed) in self.visible() {
            let on = |page: &&Shown| {
                let (right, bottom) = (page.left.saturating_add(page.cols), page.top.saturating_add(page.rows));
                (page.left..right).contains(&label.at.col) && (page.top..bottom).contains(&label.at.row)
            };
            let Some(page) = shown.iter().find(on) else { continue };
            let length = u16::try_from(label.code.chars().count()).unwrap_or(u16::MAX).min(page.cols);
            let start = label.at.col.min(page.left.saturating_add(page.cols).saturating_sub(length)).max(page.left);
            for ((nth, letter), col) in label.code.chars().enumerate().zip(start..start.saturating_add(length)) {
                cells.insert((label.at.row, col), (letter, nth < typed));
            }
        }
        let mut runs: Vec<Run> = Vec::new();
        for ((row, col), cell) in cells {
            match runs.last_mut() {
                Some(run) if run.row == row && usize::from(run.col).saturating_add(run.letters.len()) == usize::from(col) => run.letters.push(cell),
                _ => runs.push(Run { row, col, letters: vec![cell] }),
            }
        }
        runs
    }

    /// What the status line says of them: that they are waited for; or what the key does, what is
    /// typed, and what was left unread or out.
    pub(super) fn status(&self) -> String {
        if self.waiting() {
            return READING.into();
        }
        let does = match self.aim {
            Aim::Follow => "f: type a label to follow its link",
            Aim::Copy => "F: type a label to copy its link's address",
        };
        let typed = if self.typed.is_empty() { String::new() } else { format!(" · {}", self.typed) };
        let notes: String = self.notes.iter().map(|note| format!(" · {note}")).collect();
        format!("{does}{typed}{notes} · esc cancel")
    }

    /// What the status line says once they are gone with nothing chosen, if they left something
    /// to say: none, here.
    pub(super) fn notes(&self) -> &[String] {
        &self.notes
    }
}

/// The codes of `count` labels, in order: every one the same length, the shortest that gives each
/// its own (one letter up to 14 labels, two up to 196, three past), counting through [`LETTERS`].
pub(super) fn codes(count: usize) -> impl Iterator<Item = String> {
    let letters: Vec<char> = LETTERS.chars().collect();
    let base = letters.len().max(1);
    let mut length = 1u32;
    while base.checked_pow(length).is_some_and(|covers| covers < count) {
        length = length.saturating_add(1);
    }
    (0..count).map(move |n| {
        let digit = |place: u32| letters.get((n / base.pow(place)) % base).copied().unwrap_or('s');
        (0..length).rev().map(digit).collect()
    })
}

/// Where the label of `link` goes on the page `shown`: the top left of the first of its rectangles,
/// down then across, that is at least partly in the part of the page on screen, clipped to that
/// part; how far down and across the page that is, in the page's unit, and where on screen. `None`
/// for a link with nothing there.
fn spot(shown: &Shown, link: &Link, cell: (f32, f32)) -> Option<((f32, f32), At)> {
    let (width, height) = shown.size;
    let (rows, of) = (f32::from(shown.rows), f32::from(shown.of.max(shown.rows).max(1)));
    if !(width > 0.0 && height > 0.0 && rows > 0.0 && shown.cols > 0) {
        return None;
    }
    // The part of the page on screen, in its own unit.
    let (from, to) = (f32::from(shown.scroll) / of * height, (f32::from(shown.scroll) + rows) / of * height);
    let part = |[x0, y0, x1, y1]: [f32; 4]| {
        let (top, bottom) = (y0.max(from), y1.min(to));
        (top < bottom && x0 < x1 && x0 < width && x1 > 0.0).then_some((top, x0.max(0.0)))
    };
    let (top, left) = link.rects.iter().filter_map(|rect| part(*rect)).min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))?;
    // In cells from the page's top left on screen: finite, and within the page's cells.
    let across = (left / width * f32::from(shown.cols)).clamp(0.0, f32::from(shown.cols.saturating_sub(1)));
    let down = ((top - from) / (to - from) * rows).clamp(0.0, f32::from(shown.rows.saturating_sub(1)));
    let within = |cells: f32, pixels: f32| {
        let into = (cells.fract() * pixels).floor();
        // A whole number of pixels under the cell's size, itself under u16's.
        if into.is_finite() { into.clamp(0.0, (pixels - 1.0).clamp(0.0, 4096.0)) as u16 } else { 0 }
    };
    let at = At {
        row: shown.top.saturating_add(down as u16),
        col: shown.left.saturating_add(across as u16),
        x: within(across, cell.0),
        y: within(down, cell.1),
    };
    Some(((top, left), at))
}

/// The colours of a label: its box, the rule along its top and bottom, a letter, and a letter
/// typed.
const BOX: [u8; 3] = [255, 221, 87];
const RULE: [u8; 3] = [214, 170, 30];
const INK: [u8; 3] = [48, 37, 5];
const TYPED: [u8; 3] = [190, 160, 70];

/// The letters' shapes, in [`LETTERS`]' order: seven rows of five pixels each, the leftmost the
/// high bit of the five. Capitals, as Vimium shows them.
const GLYPHS: [[u8; 7]; 14] = [
    // S A D F
    [0b01110, 0b10001, 0b10000, 0b01110, 0b00001, 0b10001, 0b01110],
    [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
    [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
    [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
    // J K L E
    [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
    [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
    [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
    [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
    // W C M P
    [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
    [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
    [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
    [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
    // G H
    [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
    [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
];
const _: () = assert!(GLYPHS.len() == LETTERS.len());

/// The largest a letter's pixel is drawn, in pixels: a cell is never taller than what that covers.
const MAX_ENLARGED: u16 = 32;

/// The image the labels are drawn from: every letter in a box of its own, side by side, once as
/// a label shows it and under that once as a letter typed. A label is its letters' boxes placed
/// side by side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Sheet {
    pub(super) width: u32,
    pub(super) height: u32,
    /// Opaque RGBA, row by row.
    pub(super) rgba: Vec<u8>,
    /// A letter's box, in pixels.
    pub(super) box_w: u16,
    pub(super) box_h: u16,
}

impl Sheet {
    /// The letters for a cell `cell_h` pixels high: each of a letter's pixels as many pixels as
    /// fit its seven rows of them, and half of one above and below, in the cell's height; one at
    /// the least, in a cell too small for that.
    pub(super) fn new(cell_h: f32) -> Self {
        let cell_h = if cell_h.is_finite() { cell_h.clamp(1.0, 4096.0) as u16 } else { 20 };
        // Seven rows of the letter's pixels and half of one above and below, within the cell.
        let fits = |enlarged: &u16| enlarged * 7 + enlarged.div_ceil(2) * 2 <= cell_h;
        let enlarged = (1..=MAX_ENLARGED).rev().find(fits).unwrap_or(1);
        let (pad_x, pad_y) = (enlarged.div_ceil(2), enlarged.div_ceil(2));
        let (box_w, box_h) = (enlarged * 5 + pad_x * 2, enlarged * 7 + pad_y * 2);
        let (width, height) = (usize::from(box_w) * LETTERS.len(), usize::from(box_h) * 2);
        let mut rgba = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            let (typed, y) = (y >= usize::from(box_h), y % usize::from(box_h));
            for x in 0..width {
                let (letter, x) = (x / usize::from(box_w), x % usize::from(box_w));
                // The letter's pixel this one is in, if it is in one.
                let (across, down) = (
                    x.checked_sub(usize::from(pad_x)).map(|x| x / usize::from(enlarged)),
                    y.checked_sub(usize::from(pad_y)).map(|y| y / usize::from(enlarged)),
                );
                let row = down.and_then(|down| GLYPHS.get(letter)?.get(down)).copied().unwrap_or(0);
                let inked = across.is_some_and(|across| across < 5 && row >> (4 - across) & 1 == 1);
                let colour = match (inked, y == 0 || y + 1 == usize::from(box_h)) {
                    (true, _) if typed => TYPED,
                    (true, _) => INK,
                    (false, true) => RULE,
                    (false, false) => BOX,
                };
                rgba.extend_from_slice(&[colour[0], colour[1], colour[2], 255]);
            }
        }
        // At most 14 boxes of 192 by 256 pixels: within u32.
        Self { width: width as u32, height: height as u32, rgba, box_w, box_h }
    }

    /// Where letter `c`'s box is in the image, as a label shows it or as a letter `typed`: its
    /// left, top, width and height in pixels. `None` for a character that is no label's.
    pub(super) fn letter(&self, c: char, typed: bool) -> Option<(u32, u32, u32, u32)> {
        let at = u32::try_from(LETTERS.chars().position(|letter| letter == c)?).ok()?;
        let (box_w, box_h) = (u32::from(self.box_w), u32::from(self.box_h));
        Some((at * box_w, if typed { box_h } else { 0 }, box_w, box_h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Target;

    /// A link over `rect` of its page, leading to the top of page `to`.
    fn link(page: usize, rect: [f32; 4], to: usize) -> Link {
        Link { page, rects: vec![rect], address: None, target: Target::Place { page: to, y: 0.0 } }
    }

    /// A page of 100 by 200 units, all on screen in 50 columns and 20 rows from the screen's top left.
    fn whole(page: usize) -> Shown {
        Shown { page, left: 0, top: 0, cols: 50, rows: 20, scroll: 0, of: 20, size: (100.0, 200.0) }
    }

    const CELL: (f32, f32) = (10.0, 20.0);

    /// The labels of `links`, all on page 0, shown as `shown` has it.
    fn labelled(links: Vec<Link>, shown: &[Shown]) -> Hints {
        let pages: Vec<usize> = shown.iter().map(|shown| shown.page).collect();
        let mut hints = Hints::new(Aim::Follow, &pages, Instant::now());
        hints.told(0, Ok(links), 0);
        hints.settle(shown, CELL);
        hints
    }

    fn shown_codes(hints: &Hints) -> Vec<&str> {
        hints.visible().map(|(label, _)| label.code.as_str()).collect()
    }

    /// Every label has a code of its own, all the same length, the shortest that covers them: one
    /// letter up to 14, two up to 196, three past, counting through Vimium's letters.
    #[test]
    fn hints_codes_are_all_one_length_the_shortest_that_covers_the_links() {
        let all = |count| codes(count).collect::<Vec<_>>();
        assert_eq!(all(0), Vec::<String>::new());
        assert_eq!(all(1), ["s"]);
        assert_eq!(all(14).join(""), LETTERS);
        assert_eq!(all(15).get(..3).unwrap(), ["ss", "sa", "sd"]);
        assert_eq!(all(15).last().unwrap(), "as");
        for (count, length) in [(1, 1), (14, 1), (15, 2), (196, 2), (197, 3), (MAX_LABELS, 3)] {
            let codes = all(count);
            assert!(codes.iter().all(|code| code.chars().count() == length && code.chars().all(|c| LETTERS.contains(c))), "{count}");
            let mut unique = codes.clone();
            unique.sort();
            unique.dedup();
            assert_eq!((codes.len(), unique.len()), (count, count), "each its own");
        }
        assert_eq!(all(196).last().unwrap(), "hh");
        assert_eq!(all(197).first().unwrap(), "sss");
    }

    /// The labels are in reading order, down the page then across, each at the top left of its
    /// link: its cell, and how far into the cell.
    #[test]
    fn hints_label_links_in_reading_order_at_their_top_left() {
        let links = vec![link(0, [50.0, 100.0, 80.0, 110.0], 3), link(0, [50.0, 15.0, 80.0, 25.0], 1), link(0, [3.0, 100.0, 40.0, 110.0], 2)];
        let hints = labelled(links, &[whole(0)]);
        let labels: Vec<(&str, At, &Target)> = hints.visible().map(|(label, _)| (label.code.as_str(), label.at, &label.link.target)).collect();
        // 100 units over 50 columns of 10 px: 3 units are a column and a half; 200 over 20 rows of 20 px:
        // 15 units are a row and a half.
        assert_eq!(labels[0], ("s", At { row: 1, col: 25, x: 0, y: 10 }, &Target::Place { page: 1, y: 0.0 }));
        assert_eq!(labels[1], ("a", At { row: 10, col: 1, x: 5, y: 0 }, &Target::Place { page: 2, y: 0.0 }));
        assert_eq!(labels[2], ("d", At { row: 10, col: 25, x: 0, y: 0 }, &Target::Place { page: 3, y: 0.0 }));
        assert_eq!(hints.status(), "f: type a label to follow its link · esc cancel");
    }

    /// On a page taller than the window, the links in the part shown are labelled, one cut by its
    /// edge at the edge, and one that runs over lines by its first line on screen; on pages side by
    /// side, each page's, the left page's first.
    #[test]
    fn hints_label_the_part_of_a_tall_page_on_screen_and_every_page_side_by_side() {
        // Rows 10 to 19 of 40 show: from 50 to 100 of the page's 200 units, 5 units a row.
        let tall = Shown { page: 0, left: 4, top: 2, cols: 50, rows: 10, scroll: 10, of: 40, size: (100.0, 200.0) };
        let lines = Link { rects: vec![[10.0, 30.0, 90.0, 40.0], [10.0, 40.0, 50.0, 60.0]], ..link(0, [0.0; 4], 4) };
        let links = vec![
            link(0, [10.0, 10.0, 30.0, 20.0], 1),
            link(0, [20.0, 45.0, 60.0, 55.0], 2),
            link(0, [10.0, 120.0, 30.0, 130.0], 3),
            lines,
            link(0, [10.0, 95.0, 30.0, 130.0], 5),
            link(0, [f32::NAN, 60.0, 30.0, 70.0], 6),
        ];
        let hints = labelled(links, &[tall]);
        let labels: Vec<(At, &Target)> = hints.visible().map(|(label, _)| (label.at, &label.link.target)).collect();
        let to = |page| Target::Place { page, y: 0.0 };
        // The link over lines, by its second line, from the top of the part shown; the one cut by the
        // top, at the top; the one cut by the bottom, where it starts.
        assert_eq!(
            labels,
            [(At { row: 2, col: 9, x: 0, y: 0 }, &to(4)), (At { row: 2, col: 14, x: 0, y: 0 }, &to(2)), (At { row: 11, col: 9, x: 0, y: 0 }, &to(5))]
        );
        // Two pages side by side, and the second given twice: the left page's links first.
        let (left, right) = (whole(0), Shown { page: 1, left: 60, ..whole(1) });
        let mut hints = Hints::new(Aim::Copy, &[0, 1, 1], Instant::now());
        assert_eq!(hints.wanted(), [Ask::Page(0), Ask::Page(1)]);
        hints.told(1, Ok(vec![link(1, [0.0, 0.0, 10.0, 10.0], 7)]), 0);
        assert!(!hints.due(Instant::now()) && hints.wanted() == [Ask::Page(0)]);
        hints.told(0, Ok(vec![link(0, [0.0, 100.0, 10.0, 110.0], 8)]), 0);
        assert!(hints.due(Instant::now()));
        assert_eq!(hints.settle(&[left, right, Shown { top: 30, ..right }], CELL), 3);
        let at: Vec<(u16, u16, &Target)> = hints.visible().map(|(label, _)| (label.at.row, label.at.col, &label.link.target)).collect();
        assert_eq!(at, [(10, 0, &to(8)), (0, 60, &to(7)), (30, 60, &to(7))]);
        assert_eq!(hints.status(), "F: type a label to copy its link's address · esc cancel");
    }

    /// Each letter typed hides the labels that no longer match, and counts as typed in the others;
    /// with one left its link is chosen; Backspace takes a letter back; a key in no label drops
    /// them.
    #[test]
    fn hints_typing_narrows_the_labels_to_one_and_a_key_in_none_drops_them() {
        let links: Vec<Link> = (0..16).map(|n| link(0, [0.0, 10.0 * n as f32, 10.0, 10.0 * n as f32 + 5.0], n)).collect();
        let mut hints = labelled(links, &[whole(0)]);
        assert_eq!(shown_codes(&hints).len(), 16);
        assert_eq!(hints.letter('s'), Typed::More);
        assert_eq!(shown_codes(&hints), ["ss", "sa", "sd", "sf", "sj", "sk", "sl", "se", "sw", "sc", "sm", "sp", "sg", "sh"]);
        assert!(hints.visible().all(|(_, typed)| typed == 1));
        assert_eq!(hints.status(), "f: type a label to follow its link · s · esc cancel");
        assert!(hints.backspace());
        assert_eq!(shown_codes(&hints).len(), 16);
        assert!(!hints.backspace(), "nothing typed");
        assert_eq!(hints.letter('s'), Typed::More);
        assert_eq!(hints.letter('d'), Typed::Chosen(Box::new(link(0, [0.0, 20.0, 10.0, 25.0], 2))));
        // The fifteenth and the sixteenth start with `a`: two left, then one.
        assert!(hints.backspace());
        assert_eq!(hints.letter('a'), Typed::More);
        assert_eq!(shown_codes(&hints), ["as", "aa"]);
        assert_eq!(hints.letter('a'), Typed::Chosen(Box::new(link(0, [0.0, 150.0, 10.0, 155.0], 15))));
        // A letter no label goes on with, a letter of no label, a capital, a digit.
        for none in ['d', 'x', 'S', '1', ' '] {
            assert_eq!(hints.letter(none), Typed::Dropped, "{none:?}");
        }
        // One label left by its first letter is chosen by it.
        let links: Vec<Link> = (0..15).map(|n| link(0, [0.0, 10.0 * n as f32, 10.0, 10.0 * n as f32 + 5.0], n)).collect();
        let mut hints = labelled(links, &[whole(0)]);
        assert_eq!(hints.letter('a'), Typed::Chosen(Box::new(link(0, [0.0, 140.0, 10.0, 145.0], 14))));
    }

    /// The labels are made once every page has answered, or failed, or when the wait is over: a
    /// page that failed or answers late has none, and is named; what comes after changes nothing.
    #[test]
    fn hints_wait_for_the_pages_and_a_page_late_has_no_labels() {
        let begun = Instant::now();
        let mut hints = Hints::new(Aim::Follow, &[4, 5, 6], begun);
        assert!(hints.waiting() && !hints.due(begun) && hints.status() == READING);
        assert_eq!(hints.wanted(), [Ask::Page(4), Ask::Page(5), Ask::Page(6)]);
        assert_eq!(hints.letter('s'), Typed::Dropped, "no label yet");
        hints.told(4, Ok(vec![link(4, [0.0, 0.0, 10.0, 10.0], 0)]), 0);
        hints.told(9, Ok(vec![link(9, [0.0, 0.0, 10.0, 10.0], 0)]), 0);
        hints.told(5, Err("its links can't be read".into()), 0);
        assert!(!hints.due(begun + WAIT - Duration::from_millis(1)), "page 7 still waited for");
        assert_eq!(hints.wanted(), [Ask::Page(6)]);
        assert!(hints.due(begun + WAIT));
        let shown = [whole(4), Shown { left: 60, ..whole(5) }, Shown { left: 120, ..whole(6) }];
        assert_eq!(hints.settle(&shown, CELL), 1);
        assert!(!hints.waiting() && !hints.due(begun + WAIT) && hints.wanted().is_empty());
        assert_eq!(hints.status(), "f: type a label to follow its link · links on pages 6, 7 unread · esc cancel");
        // Late: no label for it, and none renumbered.
        hints.told(6, Ok(vec![link(6, [0.0, 0.0, 10.0, 10.0], 0)]), 0);
        assert_eq!(shown_codes(&hints), ["s"]);
        // Every page answered: due at once. One alone unread is named alone.
        let mut hints = Hints::new(Aim::Follow, &[0, 1], begun);
        hints.told(0, Ok(Vec::new()), 0);
        hints.told(1, Err("gone".into()), 0);
        assert!(hints.due(begun));
        assert_eq!(hints.settle(&[whole(0), whole(1)], CELL), 0);
        assert_eq!(hints.notes(), ["links on page 2 unread"]);
    }

    /// At most a thousand links are labelled, the first in reading order, and how many were left
    /// out is said, with those the source left out itself.
    #[test]
    fn hints_label_a_thousand_links_and_say_how_many_were_left_out() {
        let links: Vec<Link> = (0..MAX_LABELS + 20).rev().map(|n| link(0, [0.0, n as f32 / 10.0, 10.0, n as f32 / 10.0 + 0.05], n)).collect();
        let mut hints = Hints::new(Aim::Follow, &[0], Instant::now());
        hints.told(0, Ok(links), 4);
        assert_eq!(hints.settle(&[whole(0)], CELL), MAX_LABELS);
        assert_eq!(hints.notes(), ["24 links left out"]);
        let first = hints.visible().next().unwrap().0;
        assert_eq!((first.code.as_str(), &first.link.target), ("sss", &Target::Place { page: 0, y: 0.0 }));
        let last = hints.visible().last().unwrap().0;
        assert_eq!(last.link.target, Target::Place { page: MAX_LABELS - 1, y: 0.0 });
        let mut hints = Hints::new(Aim::Follow, &[0], Instant::now());
        hints.told(0, Ok(vec![link(0, [0.0, 0.0, 10.0, 10.0], 0)]), 1);
        hints.settle(&[whole(0)], CELL);
        assert_eq!(hints.notes(), ["1 link left out"]);
    }

    /// A page with no size, no cells or no rows on screen has no labels, and nothing here panics on
    /// what a screen may say.
    #[test]
    fn hints_never_panic_on_a_page_with_no_room() {
        let links = || vec![link(0, [0.0, 0.0, 10.0, 10.0], 0), link(0, [f32::NEG_INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::INFINITY], 1)];
        for shown in [
            Shown { size: (0.0, 0.0), ..whole(0) },
            Shown { size: (f32::NAN, f32::INFINITY), ..whole(0) },
            Shown { cols: 0, ..whole(0) },
            Shown { rows: 0, of: 0, ..whole(0) },
        ] {
            assert_eq!(labelled(links(), &[shown]).visible().count(), 0, "{shown:?}");
        }
        for shown in [Shown { scroll: u16::MAX, of: 3, ..whole(0) }, Shown { left: u16::MAX, top: u16::MAX, ..whole(0) }] {
            let _ = labelled(links(), &[shown]).visible().count();
        }
        let hints = labelled(links(), &[Shown { size: (f32::MAX, f32::MAX), ..whole(0) }]);
        assert!(hints.visible().count() <= 2);
        let mut hints = Hints::new(Aim::Follow, &[0], Instant::now());
        hints.told(0, Ok(links()), usize::MAX);
        hints.settle(&[whole(0)], (f32::NAN, 0.0));
        assert_eq!(hints.notes(), [format!("{} links left out", usize::MAX)]);
    }

    /// Where pages are cells, a label is its letters over the page's cells from its link's cell on:
    /// kept within its page's cells, a letter typed marked so, labels side by side one run, and of
    /// two over the same cell the later one's letter.
    #[test]
    fn hints_runs_are_the_cells_the_labels_cover_within_their_pages() {
        // Sixteen links, so two letters a label: at a row's first cell, in its middle, at its last
        // cell, two side by side, two over the same cells, and ten down the next page.
        let page = Shown { left: 10, top: 2, ..whole(0) };
        let next = Shown { page: 1, left: 70, top: 2, ..whole(1) };
        let at = |x: f32, y: f32| [x, y, x + 1.0, y + 1.0];
        let mut hints = Hints::new(Aim::Follow, &[0, 1], Instant::now());
        let first = vec![
            link(0, at(0.0, 0.0), 0),
            link(0, at(50.0, 0.0), 1),
            link(0, at(99.0, 0.0), 2),
            link(0, at(20.0, 10.0), 3),
            link(0, at(24.0, 10.0), 4),
            link(0, at(60.0, 20.0), 5),
            link(0, at(62.0, 20.0), 6),
        ];
        hints.told(0, Ok(first), 0);
        hints.told(1, Ok((0..9).map(|n| link(1, at(0.0, 10.0 * n as f32), 7 + n)).collect()), 0);
        assert_eq!(hints.settle(&[page, next], CELL), 16);
        let plain = |letters: &str| letters.chars().map(|c| (c, false)).collect::<Vec<_>>();
        let runs = hints.runs(&[page, next]);
        // `ss` at the page's first cell; `sa` in its middle; `sd` moved left to end at its last cell.
        assert_eq!(runs[0], Run { row: 2, col: 10, letters: plain("ss") });
        assert_eq!(runs[1], Run { row: 2, col: 35, letters: plain("sa") });
        assert_eq!(runs[2], Run { row: 2, col: 58, letters: plain("sd") });
        // The next page's first, on the same row, after them. Then `sf` and `sj`, two cells apart: one
        // run; and `sk` and `sl`, a cell apart: the later one over it.
        assert_eq!(runs[3], Run { row: 2, col: 70, letters: plain("se") });
        assert_eq!(runs[4], Run { row: 3, col: 20, letters: plain("sfsj") });
        assert_eq!(runs[6], Run { row: 4, col: 40, letters: plain("ssl") });
        assert_eq!(runs.len(), 5 + 9);
        // A letter typed is marked in the labels it starts, and the others are gone.
        assert_eq!(hints.letter('a'), Typed::More);
        let runs = hints.runs(&[page, next]);
        assert_eq!(
            runs,
            [Run { row: 9, col: 70, letters: vec![('a', true), ('s', false)] }, Run { row: 10, col: 70, letters: vec![('a', true), ('a', false)] }]
        );
        // A page a cell wide holds a letter of each; no page there, no run; none while they are read.
        let narrow = Shown { cols: 1, ..page };
        assert!(hints.runs(&[narrow]).is_empty(), "the labels left are on the other page");
        assert!(hints.backspace());
        assert_eq!(hints.runs(&[narrow]).first(), Some(&Run { row: 2, col: 10, letters: plain("s") }));
        assert!(hints.runs(&[]).is_empty() && Hints::new(Aim::Follow, &[0], Instant::now()).runs(&[page]).is_empty());
    }

    /// The image of the letters: each in a box of its own, as tall as fits the cell, in the
    /// labels' yellow with dark ink, and under it again in the ink of a letter typed; the same
    /// shape both times, and a different one for each letter.
    #[test]
    fn hints_letters_are_drawn_in_boxes_that_fit_the_cell() {
        let sheet = Sheet::new(20.0);
        // Two pixels a pixel: letters of 10 by 14 in boxes of 12 by 16.
        assert_eq!((sheet.box_w, sheet.box_h, sheet.width, sheet.height), (12, 16, 12 * 14, 32));
        assert_eq!(sheet.rgba.len(), 12 * 14 * 32 * 4);
        let pixel = |x: u32, y: u32| {
            let at = ((y * sheet.width + x) * 4) as usize;
            [sheet.rgba[at], sheet.rgba[at + 1], sheet.rgba[at + 2]]
        };
        let shape = |c: char, typed: bool, ink: [u8; 3]| {
            let (x, y, w, h) = sheet.letter(c, typed).unwrap();
            (0..h).flat_map(|dy| (0..w).map(move |dx| (dx, dy))).map(|(dx, dy)| pixel(x + dx, y + dy) == ink).collect::<Vec<bool>>()
        };
        let mut shapes = Vec::new();
        for c in LETTERS.chars() {
            let (x, y, w, h) = sheet.letter(c, false).unwrap();
            assert_eq!((y, w, h), (0, 12, 16));
            assert_eq!((pixel(x, 0), pixel(x, 15), pixel(x, 1)), (RULE, RULE, BOX), "{c}");
            let plain = shape(c, false, INK);
            assert!(plain.iter().filter(|inked| **inked).count() >= 4 * 11, "{c} has ink");
            assert_eq!(plain, shape(c, true, TYPED), "{c} typed is the same shape");
            assert!(!shapes.contains(&plain), "{c} is its own shape");
            shapes.push(plain);
        }
        assert_eq!((sheet.letter('s', true), sheet.letter('h', false)), (Some((0, 16, 12, 16)), Some((13 * 12, 0, 12, 16))));
        assert_eq!((sheet.letter('x', false), sheet.letter('S', false)), (None, None));
        // As tall as the cell lets it, never taller, whatever the cell.
        for cell in [0.0, 1.0, 8.0, 9.0, 16.0, 37.0, 4096.0, f32::NAN, f32::INFINITY, -5.0] {
            let sheet = Sheet::new(cell);
            assert_eq!(sheet.rgba.len(), sheet.width as usize * sheet.height as usize * 4, "{cell}");
            assert!(sheet.box_h >= 8 && (cell.is_nan() || cell < 9.0 || f32::from(sheet.box_h) <= cell), "{cell}: {}", sheet.box_h);
        }
        assert_eq!((Sheet::new(40.0).box_w, Sheet::new(40.0).box_h), (24, 32));
    }
}
