//! Searching a file's text (`/` in its pages): what is typed, the text read so far, the matches in
//! it and the one the reader is on. No IO of its own: the text comes from the source, in chunks
//! ([`Texts`]), as it is read, and [`Search::poll`] takes what has come, a little at a time, so
//! that no key waits on it.
//!
//! A chunk's text is kept folded, once ([`Folded`]): case folded a character at a time and each
//! run of whitespace made one space, with what it takes to say which of the original's characters
//! a match covers, since the source is asked where those are on its pages. Matching is
//! `str::match_indices` on that: left to right, never overlapping, never from one chunk onto the
//! next.
//!
//! The matches on a page are shown by tinting its pixels ([`tint`]) where the source says they
//! are ([`Tint`]), the reader's more strongly: into the page as rendered, before it is encoded or
//! made into text cells, so that one mechanism serves kitty, tmux and text cells alike
//! ([ADR-0028](../../docs/adr/0028-matches-tinted-into-the-pages-pixels.md)).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use crate::source::{Got, Texts, Want};

/// The most text kept, folded, with its maps: past it, reading stops.
const MAX_TEXT: usize = 64 << 20;
/// The most a chunk's text may be, in bytes: a longer one is left unread.
const MAX_CHUNK: usize = 1 << 20;
/// The most chunks searched: the rest of a text in more are left unread.
const MAX_CHUNKS: usize = 1 << 18;
/// The most matches kept and counted, the first in the document's order.
pub(super) const MAX_MATCHES: usize = 10_000;
/// The most matches whose rectangles are asked for at a time, and the most rectangles one keeps.
const MAX_PLACED: usize = 1_024;
const MAX_LINES: usize = 64;
/// The bytes of text folded, and the bytes matched, in one [`Search::poll`] or after one key: a few
/// milliseconds' work.
const SLICE: usize = 4 << 20;

/// A character as a search or a filter matches it: in lower case, so that case doesn't matter,
/// and nothing else does (`É` is `é`, and `e` isn't). A sigma at the end of a word is a sigma.
fn fold_char(c: char) -> impl Iterator<Item = char> {
    if c == 'ς' { 'σ' } else { c }.to_lowercase()
}

/// A name, or what is typed, as a filter matches it ([`fold_char`]).
pub(super) fn fold(text: &str) -> String {
    text.chars().flat_map(fold_char).collect()
}

/// What is typed as a search matches it: folded as the text is ([`Folded`]). `None` when nothing
/// is typed but whitespace, which matches nothing.
fn needle(query: &str) -> Option<String> {
    let folded = Folded::new(query).text;
    (!folded.trim().is_empty()).then_some(folded)
}

/// A chunk's text as it is matched: each character folded ([`fold_char`]) and each run of
/// whitespace one space.
#[derive(Debug, Default)]
struct Folded {
    text: String,
    /// Which of the original's characters the folded ones are: at this byte of `text`, this
    /// character of the original, and one on for each character after it, until the next entry.
    /// A run of whitespace made one, or a character that folds to two, starts a new entry.
    runs: Vec<(u32, u32)>,
}

impl Folded {
    /// `original` folded: at most [`MAX_CHUNK`] bytes of it, so that every place in it is a `u32`
    /// (a character folds to at most three, of at most as many bytes each as its own).
    fn new(original: &str) -> Self {
        let mut folded = Self { text: String::with_capacity(original.len().min(MAX_CHUNK)), runs: Vec::new() };
        // The original's character the next folded one is, if the run goes on.
        let mut expected = None;
        let mut spaced = false;
        for (index, c) in (0u32..).zip(original.chars()) {
            if folded.text.len() >= MAX_CHUNK.saturating_mul(3) {
                break;
            }
            if c.is_whitespace() {
                if !std::mem::replace(&mut spaced, true) {
                    folded.push(' ', index, &mut expected);
                }
                continue;
            }
            spaced = false;
            for c in fold_char(c) {
                folded.push(c, index, &mut expected);
            }
        }
        // Kept as long as the search is: no more held than it came to (a megabyte of spaces is one).
        folded.text.shrink_to_fit();
        folded.runs.shrink_to_fit();
        folded
    }

    fn push(&mut self, c: char, index: u32, expected: &mut Option<u32>) {
        if *expected != Some(index) {
            // Within u32: `new` stops well before.
            self.runs.push((u32::try_from(self.text.len()).unwrap_or(u32::MAX), index));
        }
        self.text.push(c);
        *expected = Some(index.saturating_add(1));
    }

    /// What it takes of the memory a search keeps: what it holds, used or not.
    fn size(&self) -> usize {
        self.text.capacity().saturating_add(self.runs.capacity().saturating_mul(std::mem::size_of::<(u32, u32)>()))
    }

    /// The matches of `needle`, at most `most` of them: where each starts and ends, in bytes.
    fn find(&self, needle: &str, most: usize) -> Vec<Hit> {
        let place = |at: usize| u32::try_from(at).unwrap_or(u32::MAX);
        self.text.match_indices(needle).take(most).map(|(at, found)| (place(at), place(at.saturating_add(found.len())))).collect()
    }

    /// The original's characters each of `hits` covers, from the first to one past the last: for
    /// hits in order, none overlapping, in one pass.
    fn originals(&self, hits: &[Hit]) -> Vec<(usize, usize)> {
        let mut out = Vec::with_capacity(hits.len());
        let mut hits = hits.iter().peekable();
        let mut runs = self.runs.iter().peekable();
        let (mut original, mut from) = (0usize, None);
        for (byte, c) in self.text.char_indices() {
            let Some(&&(start, end)) = hits.peek() else { break };
            if let Some((_, at)) = runs.next_if(|(run, _)| usize::try_from(*run) == Ok(byte)) {
                original = usize::try_from(*at).unwrap_or(usize::MAX);
            }
            if usize::try_from(start) == Ok(byte) {
                from = Some(original);
            }
            if usize::try_from(end) == Ok(byte.saturating_add(c.len_utf8())) {
                out.push((from.take().unwrap_or(original), original.saturating_add(1)));
                hits.next();
            }
            original = original.saturating_add(1);
        }
        out
    }
}

/// A match: where it starts and ends in its chunk's folded text, in bytes.
type Hit = (u32, u32);

/// A chunk of the text.
#[derive(Debug)]
enum Chunk {
    /// Not searched: of a page that isn't one of those given, or past [`MAX_CHUNKS`].
    Out,
    Unread,
    Read(Folded),
    /// The source couldn't give it, or gave more than is kept.
    Failed,
}

/// Where a match's rectangles are, asked of the source: the original's characters it covers, and
/// the rectangles once they have come, each on its page, within it.
#[derive(Debug)]
struct Placed {
    from: usize,
    to: usize,
    rects: Option<Vec<(usize, [f32; 4])>>,
}

/// What a search's queue was last set for ([`Search::queue`]): the chunk read outward from, whether
/// text was still read, and the ranges whose rectangles were asked for, as they were asked. The
/// ranges, not the matches they are of: another query's match may start where this one's did.
#[derive(Debug, PartialEq, Eq)]
struct Queued {
    centre: usize,
    reading: bool,
    placed: Vec<Want>,
}

/// What of the screen a search goes by: the document's page at its top left, the pages on it, and
/// the pages `near` it, rendered ahead of the reader, whose matches are tinted as theirs are.
#[derive(Clone, Copy, Debug)]
pub(super) struct Screen<'a> {
    pub(super) top: usize,
    pub(super) pages: &'a [usize],
    pub(super) near: &'a [usize],
}

/// A match's rectangle on a page, in the page's unit from its top left (left, top, right, bottom),
/// and whether it is the match the reader is on.
pub(super) type Mark = ([f32; 4], bool);

/// A page's tint: where its matches are.
#[derive(Debug, PartialEq)]
pub(super) struct Tint {
    /// Tells this tint from another of the same page: what a page was sent with. Never 0, which
    /// is a page with no tint.
    pub(super) key: u64,
    pub(super) marks: Vec<Mark>,
}

impl Tint {
    fn new(marks: Vec<Mark>) -> Self {
        let mut hasher = std::hash::DefaultHasher::new();
        for (rect, strong) in &marks {
            rect.map(f32::to_bits).hash(&mut hasher);
            strong.hash(&mut hasher);
        }
        Self { key: hasher.finish().max(1), marks }
    }
}

/// What a match's pixels are tinted with, the others' first and then the reader's: a colour
/// multiplied into the light ones, and one screened onto the dark ones. Paper turns the colour
/// and ink stays ink, on white as on dark paper: a yellow for the matches, an orange for the
/// reader's (ADR-0028).
const LIGHT: [[u8; 3]; 2] = [[255, 236, 130], [255, 170, 60]];
const DARK: [[u8; 3]; 2] = [[70, 58, 0], [120, 60, 0]];

/// Tint `marks` into `rgba`, a page's pixels: premultiplied RGBA, `width` by `height`, at `scale`
/// pixels to the page's unit across and down. Each pixel under a mark is tinted once, as the
/// reader's if any mark over it is; what lies outside the pixels is left out, and pixels that
/// aren't the size they say are left as they are.
pub(super) fn tint(rgba: &mut [u8], width: u32, height: u32, scale: (f32, f32), marks: &[Mark]) {
    let (Ok(wide), Ok(tall)) = (usize::try_from(width), usize::try_from(height)) else { return };
    if wide == 0 || tall == 0 || wide.checked_mul(tall).and_then(|pixels| pixels.checked_mul(4)) != Some(rgba.len()) {
        return;
    }
    // A mark's pixels, from the one it starts in to the one it ends in, within the page; `as`
    // saturates, and a side that isn't a number is no mark.
    let pixel = |at: f32, scale: f32, most: usize, up: bool| {
        let at = at * scale;
        at.is_finite().then(|| (if up { at.ceil() } else { at.floor() }.max(0.0) as usize).min(most))
    };
    let spans: Vec<(usize, usize, usize, usize, bool)> = marks
        .iter()
        .filter_map(|([x0, y0, x1, y1], strong)| {
            let (left, right) = (pixel(*x0, scale.0, wide, false)?, pixel(*x1, scale.0, wide, true)?);
            let (top, bottom) = (pixel(*y0, scale.1, tall, false)?, pixel(*y1, scale.1, tall, true)?);
            (left < right && top < bottom).then_some((left, right, top, bottom, *strong))
        })
        .collect();
    let Some(first) = spans.iter().map(|span| span.2).min() else { return };
    let last = spans.iter().map(|span| span.3).max().unwrap_or(first);
    // Which marks cover each pixel of a row: 1 for a match, 2 for the reader's.
    let mut row_marks = vec![0u8; wide];
    let rows = rgba.chunks_exact_mut(wide.saturating_mul(4)).enumerate().skip(first).take(last.saturating_sub(first));
    for (y, row) in rows {
        let mut any = false;
        for (left, right, top, bottom, strong) in &spans {
            if (*top..*bottom).contains(&y)
                && let Some(covered) = row_marks.get_mut(*left..*right)
            {
                covered.iter_mut().for_each(|mark| *mark = (*mark).max(1 + u8::from(*strong)));
                any = true;
            }
        }
        if !any {
            continue;
        }
        for (pixel, mark) in row.chunks_exact_mut(4).zip(row_marks.iter_mut()) {
            if *mark != 0 {
                tinted(pixel, *mark == 2);
                *mark = 0;
            }
        }
    }
}

/// One pixel tinted, premultiplied RGBA: by how light it is, between its colour multiplied by the
/// tint for light pixels and screened by the tint for dark ones, so that a light page takes the
/// colour and its dark ink stays dark, and the other way on dark paper. Its alpha stays.
fn tinted(pixel: &mut [u8], strong: bool) {
    let [red, green, blue, alpha] = pixel else { return };
    let opacity = u32::from(*alpha);
    if opacity == 0 {
        return;
    }
    let (light, dark) = (LIGHT[usize::from(strong)], DARK[usize::from(strong)]);
    // How light, 0 to 255, of the colour as shown: Rec. 709's weights, in 256ths.
    let luma = (u32::from(*red) * 54 + u32::from(*green) * 183 + u32::from(*blue) * 19) / 256;
    let luma = (luma * 255 / opacity).min(255);
    for (channel, (light, dark)) in [red, green, blue].into_iter().zip(light.into_iter().zip(dark)) {
        // Within its alpha, as a premultiplied channel is.
        let value = u32::from(*channel).min(opacity);
        let multiplied = value * u32::from(light) / 255;
        let screened = opacity - (opacity - value) * (255 - u32::from(dark)) / 255;
        *channel = u8::try_from((multiplied * luma + screened * (255 - luma)) / 255).unwrap_or(u8::MAX);
    }
}

/// Where the view is to go: to a page of the document, and once the match's rectangles are known,
/// to the part of it the match is in, from its top to its bottom as shares of the page's height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Go {
    pub(super) page: usize,
    pub(super) span: Option<(f32, f32)>,
}

/// What [`Search::poll`] came to.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Polled {
    /// Whether what the status line says of the search changed.
    pub(super) changed: bool,
    /// Where the view is to go, if anywhere.
    pub(super) go: Option<Go>,
    /// Why some text was left unread, to tell the reader once, in the source's words.
    pub(super) said: Option<String>,
    /// Whether a page's tint changed ([`Search::tint`]): its page is to be drawn again.
    pub(super) tinted: bool,
}

/// A file's search: what is typed after `/`, then kept by Enter; the text read so far, the matches
/// in it, and the one the reader is on.
pub(super) struct Search {
    texts: Box<dyn Texts>,
    /// Each chunk, by its place in the text.
    chunks: Vec<Chunk>,
    /// The chunk each page of the document starts with, and after the last, how many chunks.
    starts: Vec<usize>,
    /// The chunks searched, and those of them still to come, or that never will.
    wanted: usize,
    unread: usize,
    failed: usize,
    first_failed: Option<usize>,
    /// Why the last chunk left unread was, as told.
    told: Option<String>,
    /// The bytes kept ([`Folded::size`]), and whether reading stopped at [`MAX_TEXT`] or
    /// [`MAX_CHUNKS`].
    kept: usize,
    full: bool,
    /// Whether any chunk read holds anything but whitespace.
    any_text: bool,
    /// Whether the query is still being typed.
    typing: bool,
    query: String,
    needle: Option<String>,
    /// The matches, by chunk, in order: only chunks with some.
    hits: BTreeMap<usize, Vec<Hit>>,
    count: usize,
    /// Whether there are more matches than are kept.
    more: bool,
    /// The chunks read and not yet matched against the needle.
    pending: VecDeque<usize>,
    /// The match the reader is on: its chunk, and where it starts there.
    at: Option<(usize, u32)>,
    /// Enter found no match yet: the first to come is gone to.
    waiting: bool,
    /// The match the reader is on is to be scrolled to, once its rectangles are known.
    revealing: bool,
    wrapped: bool,
    /// The rectangles of the matches on the pages on screen, by chunk and start.
    placed: BTreeMap<(usize, u32), Placed>,
    /// Whether the matches, the one the reader is on or the typing changed since the queue was set.
    dirty: bool,
    shown: Vec<usize>,
    /// Each page's tint, of the pages on screen and near it: the last one whose matches' rectangles
    /// had all come, if it has any, and whether that is what the page holds now (`false` while
    /// some are still asked for). A page not here has none.
    tints: BTreeMap<usize, (Option<Rc<Tint>>, bool)>,
    /// Whether what the tints are made from changed since they were.
    marked: bool,
    /// What the queue was last set for.
    queued: Option<Queued>,
    status: String,
    /// How many times a chunk was matched against a needle.
    #[cfg(test)]
    scans: usize,
}

impl std::fmt::Debug for Search {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Search")
            .field("query", &self.query)
            .field("typing", &self.typing)
            .field("count", &self.count)
            .field("at", &self.at)
            .field("waiting", &self.waiting)
            .field("wrapped", &self.wrapped)
            .finish_non_exhaustive()
    }
}

impl Search {
    /// A search of `texts`, the text of a document of `count` pages, of which `pages` are shown
    /// (`--pages`, or all): being typed, with nothing typed yet. Only the chunks of those pages are
    /// read and searched.
    pub(super) fn new(texts: Box<dyn Texts>, pages: &[usize], count: usize) -> Self {
        let total = texts.chunks().min(MAX_CHUNKS);
        let full = texts.chunks() > MAX_CHUNKS;
        let mut starts = Vec::new();
        let mut start = 0;
        for page in 0..count {
            start = texts.chunk_of(page).clamp(start, total);
            starts.push(start);
        }
        starts.push(total);
        let mut chunks: Vec<Chunk> = std::iter::repeat_with(|| Chunk::Out).take(total).collect();
        let mut wanted = 0usize;
        for page in pages {
            for chunk in chunks_of(&starts, *page) {
                if let Some(slot @ Chunk::Out) = chunks.get_mut(chunk) {
                    *slot = Chunk::Unread;
                    wanted = wanted.saturating_add(1);
                }
            }
        }
        Self {
            texts,
            chunks,
            starts,
            wanted,
            unread: wanted,
            failed: 0,
            first_failed: None,
            told: None,
            kept: 0,
            full,
            any_text: false,
            typing: true,
            query: String::new(),
            needle: None,
            hits: BTreeMap::new(),
            count: 0,
            more: false,
            pending: VecDeque::new(),
            at: None,
            waiting: false,
            revealing: false,
            wrapped: false,
            placed: BTreeMap::new(),
            dirty: true,
            shown: Vec::new(),
            tints: BTreeMap::new(),
            marked: false,
            queued: None,
            status: String::new(),
            #[cfg(test)]
            scans: 0,
        }
    }

    /// Whether the query is still being typed.
    pub(super) fn typing(&self) -> bool {
        self.typing
    }

    /// What is typed.
    pub(super) fn query(&self) -> &str {
        &self.query
    }

    /// A key was pressed: what the status line said of the last one, it says no more, and the view
    /// is the reader's again, no longer scrolled to the match the last one went to.
    pub(super) fn heard(&mut self) {
        self.wrapped = false;
        self.revealing = false;
    }

    /// `/` again: the query kept is typed on.
    pub(super) fn edit(&mut self) {
        self.typing = true;
        self.waiting = false;
        self.wrapped = false;
        self.dirty = true;
    }

    /// A character typed: the matches are those of the longer query, counted as far as a moment
    /// allows, the rest between the keys after it ([`Search::poll`]).
    pub(super) fn typed(&mut self, c: char) {
        self.query.push(c);
        self.requery();
    }

    /// The last character typed taken back.
    pub(super) fn backspace(&mut self) {
        self.query.pop();
        self.requery();
    }

    /// The query changed: its matches are to be found again. A query the last one starts only
    /// matches in the chunks the last one matched in, so only those are looked at again, unless
    /// some of their matches weren't kept.
    fn requery(&mut self) {
        let new = needle(&self.query);
        if new == self.needle {
            return;
        }
        let narrows = !self.more && matches!((&self.needle, &new), (Some(old), Some(new)) if new.starts_with(old.as_str()));
        let matched = std::mem::take(&mut self.hits);
        if narrows {
            self.pending.extend(matched.keys());
        } else {
            self.pending = self.chunks.iter().enumerate().filter(|(_, chunk)| matches!(chunk, Chunk::Read(_))).map(|(at, _)| at).collect();
            self.more = false;
        }
        if new.is_none() {
            self.pending.clear();
        }
        self.needle = new;
        self.count = 0;
        self.at = None;
        self.waiting = false;
        self.revealing = false;
        self.wrapped = false;
        self.placed.clear();
        self.dirty = true;
        self.count_on(SLICE);
    }

    /// Enter: the search is kept, and the view goes to the first match known at or after the top of
    /// the screen, or past the end, to the first of all. With none known yet, the view goes the same
    /// way once one is, by what is known then ([`Search::poll`]).
    pub(super) fn enter(&mut self, screen: &Screen<'_>) -> Option<Go> {
        self.typing = false;
        self.wrapped = false;
        self.dirty = true;
        let found = self.first_from(screen);
        self.at = found;
        self.waiting = found.is_none() && self.busy();
        found.map(|hit| self.go_to(hit))
    }

    /// The first match known at or after the top of the `screen`, or past the end, the first of all.
    fn first_from(&self, screen: &Screen<'_>) -> Option<(usize, u32)> {
        let top = self.starts.get(screen.top).copied().unwrap_or(0);
        let first = |(chunk, hits): (&usize, &Vec<Hit>)| hits.first().map(|hit| (*chunk, hit.0));
        self.hits.range(top..).find_map(first).or_else(|| self.hits.iter().find_map(first))
    }

    /// `n` (`forward`) or `N`, `count` matches on: from the match the reader is on while its page
    /// is on screen, otherwise from the top of the screen; past the last, round to the first, and
    /// the other way, which the status line then says. `None` with no match to go to.
    pub(super) fn step(&mut self, forward: bool, count: usize, screen: &Screen<'_>) -> Option<Go> {
        self.wrapped = false;
        let total = isize::try_from(self.count).ok().filter(|total| *total > 0)?;
        let steps = isize::try_from(count.max(1)).unwrap_or(isize::MAX);
        let on = self.at.filter(|at| screen.pages.contains(&self.page_at(*at))).and_then(|at| self.ordinal(at));
        let from = match on {
            Some(ordinal) => isize::try_from(ordinal).unwrap_or(0),
            None => {
                // From the top of the screen: the first match at or after it is one step on, and
                // the last before it one step back.
                let top = self.starts.get(screen.top).copied().unwrap_or(0);
                let before = self.hits.range(..top).map(|(_, hits)| hits.len()).fold(0usize, usize::saturating_add);
                let before = isize::try_from(before).unwrap_or(0);
                if forward { before.saturating_sub(1) } else { before }
            }
        };
        let to = if forward { from.saturating_add(steps) } else { from.saturating_sub(steps) };
        self.wrapped = to < 0 || to >= total;
        let hit = self.nth(usize::try_from(to.rem_euclid(total)).unwrap_or(0))?;
        self.at = Some(hit);
        self.waiting = false;
        self.dirty = true;
        Some(self.go_to(hit))
    }

    /// Where the view goes for the match `hit`, now the one the reader is on: its page, and its
    /// place there if its rectangles are known; otherwise once they are ([`Search::poll`]).
    fn go_to(&mut self, hit: (usize, u32)) -> Go {
        self.revealing = true;
        Go { page: self.page_at(hit), span: None }
    }

    /// Take what the source has read, match what is still to match, and ask the source for what is
    /// wanted now: the text not yet read, outward from the top of the `screen`, and where the
    /// matches on its pages are. `size_of` is a page's size, as the source says it. A little at a
    /// time ([`SLICE`]): called between keys, until [`Search::busy`] says there is no more.
    pub(super) fn poll(&mut self, screen: &Screen<'_>, size_of: &dyn Fn(usize) -> Option<(f32, f32)>) -> Polled {
        let mut polled = Polled::default();
        let mut spent = 0usize;
        while spent < SLICE {
            let Some(got) = self.texts.try_recv() else { break };
            match got {
                Got::Text(chunk, text) => spent = spent.saturating_add(self.read(chunk, text, &mut polled.said)),
                Got::Rects(want, rects) => self.place(want, rects, size_of),
            }
        }
        self.count_on(SLICE);
        if self.waiting {
            // As Enter would have gone, had this been read by then.
            match self.first_from(screen) {
                Some(hit) => {
                    self.at = Some(hit);
                    self.waiting = false;
                    self.dirty = true;
                    polled.go = Some(self.go_to(hit));
                }
                None => self.waiting = self.busy(),
            }
        }
        if self.revealing
            && let Some(at) = self.at
            && let Some((page, span)) = self.span(at, size_of)
        {
            self.revealing = false;
            polled.go = Some(Go { page, span: Some(span) });
        }
        self.queue(screen);
        polled.tinted = self.retint(screen);
        let status = self.status();
        polled.changed = status != self.status;
        self.status = status;
        polled
    }

    /// Page `page`'s tint, and whether it is the one the page holds now: while some of its matches'
    /// rectangles are still asked for, the last one there was, which a page already shown keeps
    /// until the new one is known. None while the query is typed.
    pub(super) fn tint(&self, page: usize) -> (Option<Rc<Tint>>, bool) {
        self.tints.get(&page).cloned().unwrap_or((None, true))
    }

    /// Make the tints of the screen's pages and those near it again, if what they are made from
    /// changed: for each page, its matches' rectangles once all those asked for have come, the
    /// reader's match marked. Whether any tint changed.
    fn retint(&mut self, screen: &Screen<'_>) -> bool {
        if !std::mem::take(&mut self.marked) {
            return false;
        }
        let mut tints = BTreeMap::new();
        // While the query is typed no match's rectangles are asked for or kept, so no page has a tint.
        for page in screen.pages.iter().chain(screen.near) {
            let mut asked = chunks_of(&self.starts, *page).flat_map(|chunk| self.placed.range((chunk, 0)..=(chunk, u32::MAX)));
            if asked.any(|(_, placed)| placed.rects.is_none()) {
                // Until they have all come, the page keeps what it had.
                tints.insert(*page, (self.tints.get(page).and_then(|(tint, _)| tint.clone()), false));
                continue;
            }
            let on_page = |(key, placed): (&(usize, u32), &Placed)| {
                let strong = Some(*key) == self.at;
                let rects = placed.rects.iter().flatten().filter(|(on, _)| on == page);
                rects.map(move |(_, rect)| (*rect, strong)).collect::<Vec<Mark>>()
            };
            let marks: Vec<Mark> = self.placed.iter().flat_map(on_page).collect();
            if !marks.is_empty() {
                tints.insert(*page, (Some(Rc::new(Tint::new(marks))), true));
            }
        }
        let changed = tints != self.tints;
        self.tints = tints;
        changed
    }

    /// Whether there is text still to read or to match: [`Search::poll`] has more to do.
    pub(super) fn busy(&self) -> bool {
        self.reading() || !self.pending.is_empty()
    }

    /// Whether text is still being read.
    fn reading(&self) -> bool {
        !self.full && self.unread > 0
    }

    /// Whether the source still owes something asked of it, text or where a match is: until it
    /// has come, [`Search::poll`] is to be called again soon.
    pub(super) fn expecting(&self) -> bool {
        self.busy() || self.placed.values().any(|placed| placed.rects.is_none())
    }

    /// A chunk's text from the source, or why there is none: kept, folded, if it is one still
    /// wanted, within [`MAX_CHUNK`], and there is room for it. The bytes it took to fold.
    fn read(&mut self, chunk: usize, text: Result<String, String>, said: &mut Option<String>) -> usize {
        let Some(slot @ Chunk::Unread) = self.chunks.get_mut(chunk) else { return 0 };
        let text = text.and_then(|text| if text.len() > MAX_CHUNK { Err("more text than a search keeps of one page".into()) } else { Ok(text) });
        let text = match text {
            Ok(text) => text,
            Err(why) => {
                *slot = Chunk::Failed;
                self.unread = self.unread.saturating_sub(1);
                self.failed = self.failed.saturating_add(1);
                self.first_failed.get_or_insert(chunk);
                if self.told.as_ref() != Some(&why) {
                    self.told = Some(why.clone());
                    *said = Some(why);
                }
                return 0;
            }
        };
        let folded = Folded::new(&text);
        if self.kept.saturating_add(folded.size()) > MAX_TEXT {
            self.full = true;
            return text.len();
        }
        self.kept = self.kept.saturating_add(folded.size());
        self.any_text |= folded.text.chars().any(|c| c != ' ');
        *slot = Chunk::Read(folded);
        self.unread = self.unread.saturating_sub(1);
        if self.needle.is_some() {
            self.pending.push_back(chunk);
        }
        text.len()
    }

    /// Match the chunks still to match, until `budget` bytes of them have been.
    fn count_on(&mut self, budget: usize) {
        let mut spent = 0usize;
        while spent < budget {
            let Some(chunk) = self.pending.pop_front() else { break };
            spent = spent.saturating_add(self.scan(chunk).max(1));
        }
    }

    /// Match chunk `chunk` against the needle: its matches in place of those it had, and of all the
    /// matches, only the first [`MAX_MATCHES`] kept. The bytes matched.
    fn scan(&mut self, chunk: usize) -> usize {
        let (Some(needle), Some(Chunk::Read(folded))) = (&self.needle, self.chunks.get(chunk)) else { return 0 };
        let found = folded.find(needle, MAX_MATCHES.saturating_add(1));
        let size = folded.text.len();
        #[cfg(test)]
        {
            self.scans += 1;
        }
        let had = self.hits.remove(&chunk).map_or(0, |hits| hits.len());
        self.count = self.count.saturating_sub(had).saturating_add(found.len());
        if !found.is_empty() {
            self.hits.insert(chunk, found);
        }
        while self.count > MAX_MATCHES {
            self.more = true;
            let Some(mut last) = self.hits.last_entry() else { break };
            let over = self.count.saturating_sub(MAX_MATCHES).min(last.get().len());
            let left = last.get().len().saturating_sub(over);
            last.get_mut().truncate(left);
            if left == 0 {
                last.remove();
            }
            self.count = self.count.saturating_sub(over);
        }
        if self.at.is_some_and(|at| self.ordinal(at).is_none()) {
            self.at = None;
        }
        self.dirty = true;
        size
    }

    /// Where the match `at` is among them all, from 0, if it is one.
    fn ordinal(&self, (chunk, start): (usize, u32)) -> Option<usize> {
        let within = self.hits.get(&chunk)?.binary_search_by_key(&start, |hit| hit.0).ok()?;
        Some(self.hits.range(..chunk).map(|(_, hits)| hits.len()).fold(within, usize::saturating_add))
    }

    /// The match that is `ordinal` among them all, from 0.
    fn nth(&self, ordinal: usize) -> Option<(usize, u32)> {
        let mut left = ordinal;
        for (chunk, hits) in &self.hits {
            match hits.get(left) {
                Some(hit) => return Some((*chunk, hit.0)),
                None => left = left.saturating_sub(hits.len()),
            }
        }
        None
    }

    /// The page chunk `chunk` starts on.
    fn page_of(&self, chunk: usize) -> usize {
        // The last page that starts at or before it; a chunk of text a page starts within is that
        // page's too, which is the last of those that start with it.
        let pages = self.starts.len().saturating_sub(1);
        self.starts.get(..pages).map_or(0, |starts| starts.partition_point(|start| *start <= chunk)).saturating_sub(1)
    }

    /// The page the match `at` is on: where its first rectangle is, once known; until then, where
    /// its chunk starts.
    fn page_at(&self, at: (usize, u32)) -> usize {
        let known = self.placed.get(&at).and_then(|placed| placed.rects.as_ref()).and_then(|rects| rects.first()).map(|(page, _)| *page);
        known.unwrap_or_else(|| self.page_of(at.0))
    }

    /// The page the match `at` is on and where on it, from its top to its bottom as shares of the
    /// page's height, once its rectangles are known.
    fn span(&self, at: (usize, u32), size_of: &dyn Fn(usize) -> Option<(f32, f32)>) -> Option<(usize, (f32, f32))> {
        let (page, [_, top, _, bottom]) = *self.placed.get(&at)?.rects.as_ref()?.first()?;
        let (_, height) = size_of(page).filter(|(_, height)| *height > 0.0)?;
        Some((page, (top / height, bottom / height)))
    }

    /// The rectangles the source gives for `want`, or why not: kept for the match that asked, each
    /// on a page of the document, finite, and clipped to its page; what isn't is left out.
    fn place(&mut self, want: Want, rects: Result<Vec<(usize, [f32; 4])>, String>, size_of: &dyn Fn(usize) -> Option<(f32, f32)>) {
        let Want::Rects { chunk, from, to } = want else { return };
        let pages = self.starts.len().saturating_sub(1);
        let mut asked = self.placed.range_mut((chunk, 0)..=(chunk, u32::MAX));
        let Some((_, placed)) = asked.find(|(_, placed)| placed.from == from && placed.to == to) else { return };
        let clipped = |(page, [x0, y0, x1, y1]): (usize, [f32; 4])| {
            let (width, height) = size_of(page).filter(|_| page < pages)?;
            let finite = [x0, y0, x1, y1, width, height].iter().all(|v| v.is_finite());
            let rect = [x0.max(0.0), y0.max(0.0), x1.min(width), y1.min(height)];
            (finite && rect[0] < rect[2] && rect[1] < rect[3]).then_some((page, rect))
        };
        placed.rects = Some(rects.unwrap_or_default().into_iter().take(MAX_LINES).filter_map(clipped).collect());
        self.marked = true;
    }

    /// Tell the source what is wanted now, if that changed: where the matches on the screen's
    /// pages are, the reader's first, at most [`MAX_PLACED`] of them and none while the query is
    /// typed; then the text still unread, outward from the top of the screen.
    fn queue(&mut self, screen: &Screen<'_>) {
        if self.dirty || !self.shown.iter().eq(screen.pages.iter().chain(screen.near)) {
            self.dirty = false;
            self.marked = true;
            self.shown = screen.pages.iter().chain(screen.near).copied().collect();
            let mut keys: BTreeSet<(usize, u32)> = BTreeSet::new();
            if !self.typing {
                keys.extend(self.at);
                let chunks = self.shown.iter().flat_map(|page| chunks_of(&self.starts, *page));
                let hits = chunks.flat_map(|chunk| self.hits.get(&chunk).into_iter().flatten().map(move |hit| (chunk, hit.0)));
                keys.extend(hits.take(MAX_PLACED));
            }
            self.placed.retain(|key, _| keys.contains(key));
            // The original's characters each covers, a chunk's in one pass.
            let mut new = keys.iter().filter(|key| !self.placed.contains_key(key)).copied().peekable();
            let mut found = Vec::new();
            while let Some((chunk, _)) = new.peek().copied() {
                let starts: Vec<u32> = std::iter::from_fn(|| new.next_if(|key| key.0 == chunk)).map(|key| key.1).collect();
                let (Some(Chunk::Read(folded)), Some(all)) = (self.chunks.get(chunk), self.hits.get(&chunk)) else { continue };
                let hits: Vec<Hit> = all.iter().filter(|hit| starts.binary_search(&hit.0).is_ok()).copied().collect();
                for (hit, (from, to)) in hits.iter().zip(folded.originals(&hits)) {
                    found.push(((chunk, hit.0), Placed { from, to, rects: None }));
                }
            }
            self.placed.extend(found);
        }
        let centre = self.starts.get(screen.top).copied().unwrap_or(0);
        let reading = self.reading();
        // The reader's match first, then the others in order.
        let keys = self.at.into_iter().chain(self.placed.keys().copied().filter(|key| Some(*key) != self.at));
        let rects = keys.filter_map(|key| self.placed.get(&key).map(|placed| Want::Rects { chunk: key.0, from: placed.from, to: placed.to }));
        let queued = Queued { centre, reading, placed: rects.collect() };
        if self.queued.as_ref() == Some(&queued) {
            return;
        }
        let mut queue = queued.placed.clone();
        if reading {
            let unread = |chunk: &usize| matches!(self.chunks.get(*chunk), Some(Chunk::Unread));
            let (mut on, mut back) = (centre..self.chunks.len(), (0..centre.min(self.chunks.len())).rev());
            loop {
                let (next, before) = (on.next(), back.next());
                queue.extend(next.into_iter().chain(before).filter(unread).map(Want::Text));
                if next.is_none() && before.is_none() {
                    break;
                }
            }
        }
        self.texts.set_queue(queue);
        self.queued = Some(queued);
    }

    /// What the status line says of the search: what is typed, then how many matches in what has
    /// been read (`/word 17`), `…` after it while there is more to read or to match; kept, the
    /// match the reader is on first (`/word 3/17`); `no match` once all is read and none is; that
    /// the last step went round; and what was left unread.
    pub(super) fn status(&self) -> String {
        let mut said = format!("/{}", self.query);
        let busy = if self.busy() { "…" } else { "" };
        let count = if self.more { format!("{MAX_MATCHES}+") } else { self.count.to_string() };
        let on = self.at.filter(|_| !self.typing).and_then(|at| self.ordinal(at));
        match on {
            _ if self.needle.is_none() && self.typing => {}
            Some(ordinal) => said.push_str(&format!(" {}/{count}{busy}", ordinal.saturating_add(1))),
            None if self.count == 0 && !self.busy() && !self.any_text && self.failed == 0 && !self.full => said.push_str(" no text in this file"),
            None if self.count == 0 && !self.busy() => said.push_str(" no match"),
            None => said.push_str(&format!(" {count}{busy}")),
        }
        if self.wrapped {
            said.push_str(" · search wrapped");
        }
        let pages = self.starts.len().saturating_sub(1);
        let unit = if self.chunks.len() == pages { "pages" } else { "parts" };
        match self.first_failed {
            _ if self.full || self.failed > 1 => {
                let read = self.wanted.saturating_sub(self.unread).saturating_sub(self.failed);
                said.push_str(&format!(" · read {read} of {} {unit}", self.wanted));
            }
            Some(chunk) => said.push_str(&format!(" · page {} unread", self.page_of(chunk).saturating_add(1))),
            None => {}
        }
        said
    }
}

/// The chunks of page `page`, by the chunk each page starts with (`starts`, and after the last
/// page's, how many chunks there are): from the one it starts with to the next page's; or, of a
/// page within one chunk, that chunk.
fn chunks_of(starts: &[usize], page: usize) -> std::ops::Range<usize> {
    let total = starts.last().copied().unwrap_or(0);
    let (Some(start), Some(end)) = (starts.get(page).copied(), page.checked_add(1).and_then(|next| starts.get(next)).copied()) else {
        return 0..0;
    };
    if start < end { start..end } else { start..start.saturating_add(1).min(total) }
}

#[cfg(test)]
pub(super) mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    /// A text for the tests: its chunks, a page each unless `starts` says which chunk each page
    /// starts with. It gives what is wanted in the order it is asked, when it is asked, as far as
    /// `allowed` goes; a chunk in `broken` fails.
    #[derive(Default)]
    pub(in crate::view) struct Fake {
        pub(in crate::view) chunks: Vec<String>,
        pub(in crate::view) starts: Option<Vec<usize>>,
        pub(in crate::view) broken: Vec<usize>,
        pub(in crate::view) state: RefCell<State>,
    }

    #[derive(Default)]
    pub(in crate::view) struct State {
        pub(in crate::view) queue: Vec<Want>,
        pub(in crate::view) given: Vec<Want>,
        /// How many more chunks' text it gives: all that is asked, when `None`. What is asked of a
        /// chunk given, it always gives.
        pub(in crate::view) allowed: Option<usize>,
        /// Every queue it was set.
        pub(in crate::view) queues: usize,
        /// What it gives in place of a match's rectangles, when set.
        pub(in crate::view) rects: Option<Vec<(usize, [f32; 4])>>,
    }

    impl Fake {
        pub(in crate::view) fn of(chunks: &[&str]) -> Rc<Self> {
            Rc::new(Self { chunks: chunks.iter().map(|chunk| (*chunk).to_owned()).collect(), ..Self::default() })
        }

        /// Give `n` more chunks' text, and no more.
        pub(in crate::view) fn allow(&self, n: usize) {
            self.state.borrow_mut().allowed = Some(n);
        }
    }

    impl Texts for Rc<Fake> {
        fn chunks(&self) -> usize {
            self.chunks.len()
        }
        fn chunk_of(&self, page: usize) -> usize {
            self.starts.as_ref().map_or(page, |starts| starts.get(page).copied().unwrap_or(usize::MAX))
        }
        fn set_queue(&self, wanted: Vec<Want>) {
            let mut state = self.state.borrow_mut();
            state.given.retain(|given| wanted.contains(given));
            state.queue = wanted;
            state.queues += 1;
        }
        fn try_recv(&self) -> Option<Got> {
            let mut state = self.state.borrow_mut();
            let held = state.allowed == Some(0);
            let ready = |want: &&Want| !state.given.contains(want) && (!held || matches!(want, Want::Rects { .. }));
            let want = *state.queue.iter().find(ready)?;
            state.given.push(want);
            if let Want::Text(_) = want {
                state.allowed = state.allowed.map(|n| n - 1);
            }
            Some(match want {
                Want::Text(chunk) if self.broken.contains(&chunk) => Got::Text(chunk, Err(format!("chunk {chunk} is broken"))),
                Want::Text(chunk) => Got::Text(chunk, Ok(self.chunks[chunk].clone())),
                // A line 10 high, as far down its page as the match is into its chunk, and as wide as it.
                Want::Rects { chunk, from, to } => {
                    let page = self.starts.as_ref().map_or(chunk, |starts| starts.iter().rposition(|start| *start <= chunk).unwrap_or(0));
                    let rects = state.rects.clone().unwrap_or_else(|| vec![(page, [from as f32, from as f32, to as f32, from as f32 + 10.0])]);
                    Got::Rects(want, Ok(rects))
                }
            })
        }
    }

    /// Every page 100 by 100.
    pub(in crate::view) fn square(_page: usize) -> Option<(f32, f32)> {
        Some((100.0, 100.0))
    }

    /// A search of `fake`, all its pages shown.
    fn search(fake: &Rc<Fake>) -> Search {
        let pages = fake.starts.as_ref().map_or(fake.chunks.len(), Vec::len);
        Search::new(Box::new(fake.clone()), &(0..pages).collect::<Vec<_>>(), pages)
    }

    /// `query` typed into `search`.
    fn type_in(search: &mut Search, query: &str) {
        query.chars().for_each(|c| search.typed(c));
    }

    /// The first page on screen, alone.
    fn at(page: &[usize]) -> Screen<'_> {
        Screen { top: page[0], pages: page, near: &[] }
    }

    /// Polled until there is no more to read or to match.
    fn settle(search: &mut Search, screen: &Screen<'_>) -> Vec<Polled> {
        let mut all = vec![search.poll(screen, &square)];
        while search.busy() {
            all.push(search.poll(screen, &square));
        }
        all.push(search.poll(screen, &square));
        all
    }

    fn folded(text: &str) -> String {
        Folded::new(text).text
    }

    /// Case is folded a letter at a time, past ASCII too, and nothing else is: an accent matters. A
    /// run of whitespace, of any kind, is one space.
    #[test]
    fn text_is_folded_a_letter_at_a_time_and_whitespace_runs_are_one_space() {
        assert_eq!(folded("Hello  World"), "hello world");
        assert_eq!(folded("É\tÉTÉ\r\n\u{a0} été"), "é été été");
        assert_ne!(folded("é"), folded("e"));
        assert_eq!(folded("ΟΔΟΣ οδος οδοσ"), "οδοσ οδοσ οδοσ", "a final sigma is a sigma");
        assert_eq!(folded("  a  "), " a ");
        assert_eq!((fold("Café/ ÉTÉ"), fold("ΟΔΟΣ")), ("café/ été".into(), "οδοσ".into()), "the filter's: the same letters, whitespace as it is");
        assert_eq!((needle("  \t "), needle(""), needle(" A  b")), (None, None, Some(" a b".into())));
    }

    /// What a chunk is charged against the text kept is what its folded text holds, not what it
    /// uses: a megabyte of whitespace folds to one space, and holds no more than that; and whatever
    /// it holds is counted.
    #[test]
    fn a_folded_chunk_holds_what_it_is_charged_for() {
        let spaces = Folded::new(&" \n".repeat(MAX_CHUNK / 2));
        assert_eq!(spaces.text, " ");
        assert!(spaces.text.capacity() < 64 && spaces.runs.capacity() < 8, "{} bytes held for a space", spaces.text.capacity());
        for text in [" \n".repeat(1000), "word ".repeat(1000), "É  ".repeat(1000), String::new()] {
            let folded = Folded::new(&text);
            assert!(folded.size() >= folded.text.capacity() + folded.runs.capacity() * 8, "{text:.12}");
            assert!(folded.size() <= folded.text.len() + folded.runs.len() * 8 + 64, "{text:.12}: {} held", folded.size());
        }
        // Eighty pages of whitespace cost the search eighty spaces, not eighty megabytes it doesn't count.
        let chunks: Vec<String> = (0..80).map(|_| " ".repeat(MAX_CHUNK)).collect();
        let mut search = search(&Rc::new(Fake { chunks, ..Fake::default() }));
        settle(&mut search, &at(&[0]));
        assert!(!search.full && search.kept < 80 * 64, "{}", search.kept);
        let held: usize = search.chunks.iter().map(|chunk| if let Chunk::Read(folded) = chunk { folded.text.capacity() } else { 0 }).sum();
        assert!(held <= search.kept, "{held} held, {} counted", search.kept);
    }

    /// A match in the folded text says which of the original's characters it covers, past
    /// whitespace runs, characters of several bytes and one that folds to two.
    #[test]
    fn a_match_maps_back_to_the_originals_characters() {
        let original = "Ab  \n cDé  İx é";
        let text = Folded::new(original);
        assert_eq!(text.text, "ab cdé i\u{307}x é");
        let covered = |needle: &str| -> Vec<String> {
            let hits = text.find(needle, 100);
            text.originals(&hits).iter().map(|(from, to)| original.chars().skip(*from).take(to - from).collect()).collect()
        };
        assert_eq!(covered("ab"), ["Ab"]);
        assert_eq!(covered("b c"), ["b  \n c"], "the whole run of whitespace");
        assert_eq!(covered("dé"), ["Dé"]);
        assert_eq!(covered("é"), ["é", "é"]);
        assert_eq!(covered("i"), ["İ"]);
        assert_eq!(covered("\u{307}x"), ["İx"], "from the second half of what one character folds to");
        assert_eq!(covered(" "), [" ", " ", " "], "a run alone is its first character");
        assert!(covered("zz").is_empty());
    }

    /// Matches don't overlap, are found left to right, and never run from a chunk onto the next;
    /// whitespace alone matches nothing; a run of whitespace matches a run.
    #[test]
    fn matches_are_found_left_to_right_within_a_chunk() {
        let fake = Fake::of(&["aaa aa", "a", "b  A\nb", ""]);
        let mut search = search(&fake);
        settle(&mut search, &at(&[0]));
        type_in(&mut search, "aa");
        assert_eq!(search.hits.iter().map(|(chunk, hits)| (*chunk, hits.clone())).collect::<Vec<_>>(), [(0, vec![(0, 2), (4, 6)])]);
        assert_eq!(search.status(), "/aa 2");
        search.backspace();
        assert_eq!((search.count, search.status()), (7, "/a 7".into()));
        type_in(&mut search, " b");
        assert_eq!((search.count, search.status()), (1, "/a b 1".into()), "a line's end is whitespace");
        for _ in 0..3 {
            search.backspace();
        }
        type_in(&mut search, "  \t");
        assert_eq!((search.count, search.status()), (0, "/  \t".into()), "nothing to count yet");
        search.enter(&at(&[0]));
        assert_eq!(search.status(), "/  \t no match");
        search.edit();
        search.backspace();
        search.backspace();
        search.backspace();
        type_in(&mut search, "a a");
        assert_eq!(search.count, 1, "never across chunks 0 and 1");
    }

    /// The count while typing is of what has been read, with `…` while more is to come; a
    /// character added looks only at the chunks that matched, and anything else at them all.
    #[test]
    fn the_count_grows_as_text_is_read_and_narrows_as_the_query_grows() {
        let fake = Fake::of(&["one page", "two pages", "three", "page four", "five"]);
        fake.allow(0);
        let mut search = search(&fake);
        type_in(&mut search, "page");
        assert_eq!(search.poll(&at(&[2]), &square), Polled { changed: true, go: None, said: None, tinted: false });
        assert_eq!(search.status(), "/page 0…");
        // Outward from the page on screen.
        assert_eq!(fake.state.borrow().queue, [2, 1, 3, 0, 4].map(Want::Text));
        fake.allow(2);
        assert!(search.poll(&at(&[2]), &square).changed);
        assert_eq!(search.status(), "/page 1…");
        fake.allow(3);
        search.poll(&at(&[2]), &square);
        assert_eq!((search.status(), search.busy()), ("/page 3".into(), false));
        assert!(!search.poll(&at(&[2]), &square).changed, "nothing new to say");
        // A character more: only the three chunks that matched are looked at again.
        let scans = search.scans;
        search.typed('s');
        assert_eq!((search.status(), search.count, search.scans - scans), ("/pages 1".into(), 1, 3));
        search.typed('x');
        assert_eq!(search.scans - scans, 4, "the one chunk that matched, and it no longer does");
        search.typed('y');
        assert_eq!(search.scans - scans, 4, "none matched: none to look at");
        search.backspace();
        assert_eq!(search.status(), "/pagesx no match");
        // Taken back: every chunk again.
        search.backspace();
        search.backspace();
        assert_eq!((search.status(), search.scans - scans), ("/page 3".into(), 19), "three times taken back, five chunks each");
        // The queue is set when what is wanted changes, not on every poll.
        let queues = fake.state.borrow().queues;
        search.poll(&at(&[2]), &square);
        search.poll(&at(&[2]), &square);
        assert_eq!(fake.state.borrow().queues, queues);
    }

    /// Enter goes to the first match at or after the top of the screen, past the end to the first
    /// of all; `n` and `N` go on and back from the match the reader is on, round the ends, saying
    /// so, a count going that many; paged away, from the top of the screen.
    #[test]
    fn enter_n_and_shift_n_go_from_match_to_match() {
        let fake = Fake::of(&["page one", "page two", "page three", "page four", "page five"]);
        let mut search = search(&fake);
        settle(&mut search, &at(&[0]));
        type_in(&mut search, "page");
        assert_eq!(search.enter(&at(&[1])), Some(Go { page: 1, span: None }));
        assert_eq!(search.status(), "/page 2/5");
        assert_eq!(search.step(true, 1, &at(&[1])), Some(Go { page: 2, span: None }));
        assert_eq!(search.step(true, 2, &at(&[2])), Some(Go { page: 4, span: None }));
        assert_eq!(search.status(), "/page 5/5");
        assert_eq!(search.step(true, 1, &at(&[4])), Some(Go { page: 0, span: None }));
        assert_eq!(search.status(), "/page 1/5 · search wrapped");
        assert_eq!(search.step(false, 1, &at(&[0])), Some(Go { page: 4, span: None }));
        assert_eq!(search.status(), "/page 5/5 · search wrapped");
        assert_eq!(search.step(false, 3, &at(&[4])), Some(Go { page: 1, span: None }));
        assert_eq!(search.status(), "/page 2/5", "said until the next step");
        // Paged away to page 4: on from the top of the screen is the match there, and back the one before it.
        assert_eq!(search.step(true, 1, &at(&[3])), Some(Go { page: 3, span: None }));
        search.step(false, 2, &at(&[3]));
        assert_eq!(search.status(), "/page 2/5");
        assert_eq!(search.step(false, 1, &at(&[3])), Some(Go { page: 2, span: None }), "the last before the top of the screen");
        // More steps than matches go round as often.
        assert_eq!(search.step(true, 11, &at(&[2])), Some(Go { page: 3, span: None }));
        assert!(search.wrapped);
        // Enter past the last match: round to the first, without a word.
        search.edit();
        type_in(&mut search, " t");
        assert_eq!(search.enter(&at(&[3])), Some(Go { page: 1, span: None }));
        assert_eq!(search.status(), "/page t 1/2");
    }

    /// Text that comes after Enter: with no match known, the first to come is gone to, once; the
    /// match the reader is on stays theirs as earlier chunks come, its number following.
    #[test]
    fn text_read_later_leaves_the_match_the_reader_is_on() {
        let fake = Fake::of(&["page one", "page two", "page three", "page four", "page five"]);
        fake.allow(0);
        let mut search = search(&fake);
        type_in(&mut search, "page");
        assert_eq!(search.enter(&at(&[3])), None, "none known yet");
        assert_eq!(search.poll(&at(&[3]), &square).go, None);
        assert_eq!(search.status(), "/page 0…");
        fake.allow(1);
        assert_eq!(search.poll(&at(&[3]), &square).go, Some(Go { page: 3, span: None }), "the first to come");
        assert_eq!(search.status(), "/page 1/1…");
        // Pages 3, 5, 2, 1 come: the reader stays on page 4's, now the fourth.
        let mut gone = Vec::new();
        for _ in 0..4 {
            fake.allow(1);
            gone.push(search.poll(&at(&[3]), &square).go.filter(|go| go.page != 3));
        }
        assert_eq!((gone, search.status()), (vec![None; 4], "/page 4/5".into()));
        // Several chunks read in one pass: the first match from the top of the screen, as Enter goes.
        let fake = Fake::of(&["page one", "page two", "page three"]);
        fake.allow(0);
        let mut search = self::search(&fake);
        type_in(&mut search, "page");
        assert_eq!((search.enter(&at(&[1])), search.poll(&at(&[1]), &square).go), (None, None));
        fake.allow(3);
        assert_eq!(search.poll(&at(&[1]), &square).go, Some(Go { page: 1, span: None }));
        assert_eq!(search.status(), "/page 2/3");
        // With no match at all, Enter waits no longer than the reading.
        let mut none = self::search(&Fake::of(&["a", "b"]));
        type_in(&mut none, "x");
        assert_eq!((none.enter(&at(&[0])), none.waiting), (None, true));
        settle(&mut none, &at(&[0]));
        assert_eq!((none.waiting, none.status()), (false, "/x no match".into()));
    }

    /// The reader's match is scrolled to once the source says where it is: its top and bottom as
    /// shares of its page's height, on the page its rectangle is on.
    #[test]
    fn a_matchs_rectangles_say_where_on_its_page_it_is() {
        let fake = Fake::of(&["", "0123456789 0123456789 0123456789 needle"]);
        let mut search = search(&fake);
        settle(&mut search, &at(&[0]));
        type_in(&mut search, "NEEDLE");
        assert_eq!(search.enter(&at(&[0])), Some(Go { page: 1, span: None }));
        let polled = settle(&mut search, &at(&[1]));
        let gone: Vec<Go> = polled.iter().filter_map(|polled| polled.go).collect();
        assert_eq!(gone, [Go { page: 1, span: Some((0.33, 0.43)) }], "once");
        assert_eq!(fake.state.borrow().queue, [Want::Rects { chunk: 1, from: 33, to: 39 }]);
        assert!(!search.expecting(), "all it asked for has come");
        // The next match's place is owed until it comes: the pager keeps asking meanwhile.
        let fake = Fake::of(&["needle"]);
        let mut search = self::search(&fake);
        settle(&mut search, &at(&[0]));
        type_in(&mut search, "needle");
        search.enter(&at(&[0]));
        search.poll(&at(&[0]), &square);
        assert!(search.expecting() && !search.busy(), "asked, not yet come");
        search.poll(&at(&[0]), &square);
        assert!(!search.expecting());
        // Typing again asks for none.
        search.edit();
        settle(&mut search, &at(&[1]));
        assert!(fake.state.borrow().queue.is_empty());
    }

    /// A query changed and kept again before the next pass (`/p⏎` read together, after `a`): the
    /// match starts where the last one did, and its rectangles are asked for anew, for the
    /// characters it covers now. The ones the source gave for the old match aren't taken for it.
    #[test]
    fn a_query_changed_between_passes_asks_for_its_matches_rectangles_anew() {
        let fake = Fake::of(&["apple"]);
        let mut search = search(&fake);
        let screen = at(&[0]);
        type_in(&mut search, "a");
        search.enter(&screen);
        settle(&mut search, &screen);
        assert_eq!(fake.state.borrow().queue, [Want::Rects { chunk: 0, from: 0, to: 1 }]);
        assert_eq!(search.tint(0).0.map(|tint| tint.marks.clone()), Some(vec![([0.0, 0.0, 1.0, 10.0], true)]));
        search.edit();
        search.typed('p');
        search.enter(&screen);
        settle(&mut search, &screen);
        assert_eq!(fake.state.borrow().queue, [Want::Rects { chunk: 0, from: 0, to: 2 }], "the longer match's");
        assert!(!search.expecting(), "nothing is owed for ever");
        assert_eq!(search.tint(0).0.map(|tint| tint.marks.clone()), Some(vec![([0.0, 0.0, 2.0, 10.0], true)]));
        // And rectangles that come late for the old match change nothing.
        search.place(Want::Rects { chunk: 0, from: 0, to: 1 }, Ok(vec![(0, [50.0, 50.0, 60.0, 60.0])]), &square);
        assert_eq!(search.placed.values().next().and_then(|placed| placed.rects.clone()), Some(vec![(0, [0.0, 0.0, 2.0, 10.0])]));
    }

    /// What the source gives is checked: a chunk that isn't one of its own or comes twice, text past
    /// the cap, a rectangle on no page, not finite, or outside its page. Each costs only itself.
    #[test]
    fn what_a_source_gives_is_checked() {
        let fake = Fake::of(&["needle", "needle"]);
        fake.allow(0);
        let mut search = search(&fake);
        let mut said = None;
        assert_eq!(search.read(7, Ok("needle".into()), &mut said), 0, "no such chunk");
        assert_eq!(search.read(0, Ok("x".repeat(MAX_CHUNK + 1)), &mut said), 0);
        assert_eq!((said.as_deref(), search.failed), (Some("more text than a search keeps of one page"), 1));
        assert_eq!(search.read(0, Ok("needle".into()), &mut said), 0, "given twice");
        search.read(1, Ok("needle".into()), &mut said);
        type_in(&mut search, "needle");
        assert_eq!(search.status(), "/needle 1 · page 1 unread");
        search.enter(&at(&[1]));
        let rects = vec![
            (1, [10.0, 10.0, 20.0, 20.0]),
            (2, [10.0, 10.0, 20.0, 20.0]),
            (1, [f32::NAN, 10.0, 20.0, 20.0]),
            (1, [10.0, 10.0, f32::INFINITY, 20.0]),
            (1, [-5.0, 90.0, 300.0, 200.0]),
            (1, [200.0, 10.0, 220.0, 20.0]),
            (1, [20.0, 10.0, 10.0, 20.0]),
        ];
        fake.state.borrow_mut().rects = Some(rects);
        search.poll(&at(&[1]), &square);
        search.poll(&at(&[1]), &square);
        let kept = search.placed.values().next().unwrap().rects.clone().unwrap();
        assert_eq!(kept, [(1, [10.0, 10.0, 20.0, 20.0]), (1, [0.0, 90.0, 100.0, 100.0])]);
        // Rectangles nobody asked for change nothing.
        search.place(Want::Rects { chunk: 1, from: 9, to: 12 }, Ok(vec![(1, [1.0, 1.0, 2.0, 2.0])]), &square);
        search.place(Want::Text(1), Ok(vec![(1, [1.0, 1.0, 2.0, 2.0])]), &square);
        assert_eq!(search.placed.values().next().unwrap().rects.as_ref().map(Vec::len), Some(2));
    }

    /// A chunk the source can't give costs the search that chunk: the status line says which page,
    /// and why is told once; with more than one, how far it read.
    #[test]
    fn a_chunk_that_fails_is_left_unread_and_said() {
        let fake = Rc::new(Fake { chunks: ["a", "b", "a", "a"].map(String::from).to_vec(), broken: vec![1], ..Fake::default() });
        let mut search = search(&fake);
        type_in(&mut search, "a");
        let said: Vec<String> = settle(&mut search, &at(&[0])).into_iter().filter_map(|polled| polled.said).collect();
        assert_eq!((said, search.status()), (vec!["chunk 1 is broken".to_owned()], "/a 3 · page 2 unread".into()));
        let fake = Rc::new(Fake { chunks: ["a", "b", "a", "a"].map(String::from).to_vec(), broken: vec![1, 3], ..Fake::default() });
        let mut search = self::search(&fake);
        type_in(&mut search, "a");
        let said: Vec<String> = settle(&mut search, &at(&[0])).into_iter().filter_map(|polled| polled.said).collect();
        assert_eq!((said.last().map(String::as_str), search.status().as_str()), (Some("chunk 3 is broken"), "/a 2 · read 2 of 4 pages"));
        // Every chunk failed: no match, not "no text".
        let fake = Rc::new(Fake { chunks: vec!["a".into()], broken: vec![0], ..Fake::default() });
        let mut search = self::search(&fake);
        type_in(&mut search, "a");
        settle(&mut search, &at(&[0]));
        assert_eq!(search.status(), "/a no match · page 1 unread");
    }

    /// A file whose pages hold no text says so once all are read; one with text and no match, that.
    #[test]
    fn a_file_without_text_says_so() {
        let mut search = search(&Fake::of(&["", " \n", ""]));
        type_in(&mut search, "a");
        assert_eq!(search.status(), "/a 0…");
        settle(&mut search, &at(&[0]));
        assert_eq!(search.status(), "/a no text in this file");
        search.enter(&at(&[0]));
        assert_eq!(search.status(), "/a no text in this file");
        let mut search = self::search(&Fake::of(&[]));
        settle(&mut search, &at(&[0]));
        assert_eq!(search.status(), "/", "nothing typed, nothing said");
    }

    /// At most [`MAX_MATCHES`] matches are kept, the first in the document's order however the
    /// chunks come, and counted as that many and more; reading goes on.
    #[test]
    fn the_first_ten_thousand_matches_are_kept() {
        let chunks: Vec<String> = (0..6).map(|_| "a".repeat(3_000)).collect();
        let fake = Rc::new(Fake { chunks, ..Fake::default() });
        let mut search = search(&fake);
        type_in(&mut search, "a");
        // Read outward from page 6: 5, 4, 3, 2, 1, 0.
        settle(&mut search, &at(&[5]));
        assert_eq!((search.count, search.more, search.status()), (MAX_MATCHES, true, "/a 10000+".into()));
        assert_eq!(search.hits.iter().map(|(chunk, hits)| (*chunk, hits.len())).collect::<Vec<_>>(), [(0, 3000), (1, 3000), (2, 3000), (3, 1000)]);
        assert_eq!(search.unread, 0, "all read");
        // Past the cap, a longer query looks at every chunk again: those whose matches went too.
        search.typed('a');
        settle(&mut search, &at(&[5]));
        assert_eq!((search.count, search.more), (9_000, false));
    }

    /// Only the chunks of the pages shown are read and searched; a page within a chunk, or a chunk
    /// over several pages, is found by the page it starts on.
    #[test]
    fn chunks_follow_the_pages_given_and_where_they_start() {
        let fake = Fake::of(&["page one", "page two", "page three", "page four"]);
        let mut search = Search::new(Box::new(fake.clone()), &[3, 1], 4);
        type_in(&mut search, "page");
        settle(&mut search, &at(&[3]));
        assert_eq!(search.status(), "/page 2");
        assert_eq!(search.enter(&at(&[3])), Some(Go { page: 3, span: None }));
        assert_eq!(search.step(true, 1, &at(&[3])), Some(Go { page: 1, span: None }));
        // Seven paragraphs on three pages: the first page's three, a fourth onto the second page, where two
        // more start, and one on the third.
        let chunks = ["a", "b", "c", "long", "d", "e", "last"].map(String::from).to_vec();
        let fake = Rc::new(Fake { chunks, starts: Some(vec![0, 3, 6]), ..Fake::default() });
        let mut search = self::search(&fake);
        settle(&mut search, &at(&[0]));
        assert_eq!((0..7).map(|chunk| search.page_of(chunk)).collect::<Vec<_>>(), [0, 0, 0, 1, 1, 1, 2]);
        type_in(&mut search, "l");
        assert_eq!(search.enter(&at(&[1])), Some(Go { page: 1, span: None }));
        assert_eq!(search.step(true, 1, &at(&[1])), Some(Go { page: 2, span: None }));
        assert_eq!(search.status(), "/l 2/2");
        // A source that says a page starts before an earlier one, or past the end, is put right.
        let odd = Rc::new(Fake { chunks: vec!["a".into(), "b".into()], starts: Some(vec![1, 0, 9]), ..Fake::default() });
        let search = self::search(&odd);
        assert_eq!(search.starts, [1, 1, 2, 2]);
    }

    /// Past the memory a search keeps, reading stops, and the status line says how far it read.
    #[test]
    fn reading_stops_at_the_most_text_kept() {
        let chunks: Vec<String> = (0..80).map(|_| "a".repeat(MAX_CHUNK)).collect();
        let fake = Rc::new(Fake { chunks, ..Fake::default() });
        let mut search = search(&fake);
        type_in(&mut search, "aaaa");
        settle(&mut search, &at(&[0]));
        assert!(search.full && search.kept <= MAX_TEXT, "{}", search.kept);
        assert_eq!(search.status(), "/aaaa 10000+ · read 63 of 80 pages");
        assert!(fake.state.borrow().queue.iter().all(|want| !matches!(want, Want::Text(_))), "no more text asked for");
    }

    /// A page of `width` by `height` pixels, all of one opaque colour.
    fn page(width: usize, height: usize, colour: [u8; 3]) -> Vec<u8> {
        [colour[0], colour[1], colour[2], 255].repeat(width * height)
    }

    /// The pixel at `x`, `y` of a page `width` wide.
    fn pixel(rgba: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
        rgba[(y * width + x) * 4..][..4].try_into().unwrap()
    }

    /// A match's pixels are tinted where its rectangle is, at the page's scale, and nowhere else:
    /// white paper takes the tint's colour and black ink stays dark; dark paper takes a darker one
    /// and light ink stays light; the reader's match another colour than the others'.
    #[test]
    fn a_match_is_tinted_into_its_pixels_and_nowhere_else() {
        // A page 20 units by 10, at 2 pixels a unit across and 3 down: marks at units 2 to 5 by 1 to 3.
        let marks = [([2.0, 1.0, 5.0, 3.0], false), ([12.0, 6.0, 14.5, 7.2], true)];
        let mut white = page(40, 30, [255, 255, 255]);
        let ink = (5 * 40 + 6) * 4;
        white[ink..ink + 3].copy_from_slice(&[0, 0, 0]);
        let before = white.clone();
        tint(&mut white, 40, 30, (2.0, 3.0), &marks);
        for y in 0..30 {
            for x in 0..40 {
                let weak = (4..10).contains(&x) && (3..9).contains(&y);
                let strong = (24..29).contains(&x) && (18..22).contains(&y);
                let (was, is) = (pixel(&before, 40, x, y), pixel(&white, 40, x, y));
                match (weak, strong, was) {
                    (true, _, [255, 255, 255, 255]) => assert_eq!(is, [255, 236, 130, 255], "{x},{y}"),
                    (_, true, _) => assert_eq!(is, [255, 170, 60, 255], "{x},{y}"),
                    (true, _, _) => assert_eq!(is, [70, 58, 0, 255], "ink stays dark, {x},{y}"),
                    _ => assert_eq!(is, was, "{x},{y}"),
                }
            }
        }
        // Dark paper, light ink.
        let mut dark = page(40, 30, [30, 30, 30]);
        dark[ink..ink + 3].copy_from_slice(&[230, 230, 230]);
        tint(&mut dark, 40, 30, (2.0, 3.0), &marks);
        let (paper, light, own) = (pixel(&dark, 40, 4, 3), pixel(&dark, 40, 6, 5), pixel(&dark, 40, 24, 18));
        assert!(paper[0] > 60 && paper[0] > paper[2] + 30 && paper[0] < 130, "a dark yellow: {paper:?}");
        let luma = |pixel: [u8; 4]| (u32::from(pixel[0]) * 54 + u32::from(pixel[1]) * 183 + u32::from(pixel[2]) * 19) / 256;
        assert!(luma(light) >= 200 && luma(paper) <= 100, "light ink stays light on its paper: {light:?} {paper:?}");
        assert!(own[0] > paper[0] + 20 && own[1] <= paper[1] + 10, "the reader's is told from the others: {own:?} {paper:?}");
        assert_eq!(pixel(&dark, 40, 0, 0), [30, 30, 30, 255]);
        // In between, a grey turns by degrees, never a jump from one blend to the other.
        let mut greys: Vec<u8> = (0..=255u8).flat_map(|v| [v, v, v, 255]).collect();
        tint(&mut greys, 256, 1, (1.0, 1.0), &[([0.0, 0.0, 256.0, 1.0], false)]);
        let reds: Vec<u8> = greys.chunks(4).map(|pixel| pixel[0]).collect();
        // (A level down here and there is the rounding.)
        assert!(reds.windows(2).all(|pair| pair[1].abs_diff(pair[0]) <= 3) && reds[0] < 80 && reds[255] == 255, "{reds:?}");
    }

    /// Marks over one another tint a pixel once, as the reader's if either is; a transparent pixel
    /// stays one, and a channel never passes its alpha; marks off the page, turned inside out or not
    /// numbers are left out; and pixels that aren't the size they say are left as they are.
    #[test]
    fn a_pixel_is_tinted_once_within_its_alpha_and_bad_marks_are_left_out() {
        let mut twice = page(10, 4, [255, 255, 255]);
        tint(&mut twice, 10, 4, (1.0, 1.0), &[([0.0, 0.0, 6.0, 4.0], false), ([4.0, 0.0, 10.0, 4.0], false), ([5.0, 2.0, 7.0, 4.0], true)]);
        assert_eq!(
            (pixel(&twice, 10, 5, 0), pixel(&twice, 10, 5, 3), pixel(&twice, 10, 9, 0)),
            ([255, 236, 130, 255], [255, 170, 60, 255], [255, 236, 130, 255])
        );
        let mut clear = [0, 0, 0, 0, 100, 100, 100, 128, 128, 128, 128, 128].to_vec();
        tint(&mut clear, 3, 1, (1.0, 1.0), &[([0.0, 0.0, 3.0, 1.0], true)]);
        assert_eq!(clear[..4], [0, 0, 0, 0]);
        assert!(
            clear.chunks(4).all(|pixel| pixel[..3].iter().all(|v| *v <= pixel[3])) && clear[7] == 128 && clear[8] == 128 && clear[10] < 128,
            "{clear:?}"
        );
        let plain = page(10, 4, [255, 255, 255]);
        let odd = [
            ([f32::NAN, 0.0, 5.0, 4.0], false),
            ([0.0, 0.0, f32::INFINITY, 4.0], true),
            ([20.0, 0.0, 30.0, 4.0], false),
            ([-9.0, -9.0, -1.0, -1.0], false),
            ([5.0, 3.0, 2.0, 1.0], true),
        ];
        let mut marked = plain.clone();
        tint(&mut marked, 10, 4, (1.0, 1.0), &odd);
        assert_eq!(marked, plain);
        for (width, height) in [(10, 5), (0, 4), (u32::MAX, u32::MAX), (9, 4)] {
            tint(&mut marked, width, height, (1.0, 1.0), &[([0.0, 0.0, 10.0, 4.0], false)]);
            assert_eq!(marked, plain, "{width} × {height}");
        }
        tint(&mut marked, 10, 4, (f32::NAN, 1.0), &[([0.0, 0.0, 10.0, 4.0], false)]);
        assert_eq!(marked, plain);
    }

    /// A page's tint is its matches' rectangles, the reader's marked, once those asked for have all
    /// come; until then it is what it was, and said not to be settled. Stepping moves the mark,
    /// which is another tint; typing again leaves none; pages near the screen have theirs too.
    #[test]
    fn a_pages_tint_is_its_matches_once_their_rectangles_have_come() {
        let fake = Fake::of(&["page page", "page", "none", "page"]);
        let mut search = search(&fake);
        let screen = Screen { top: 0, pages: &[0], near: &[1, 2] };
        type_in(&mut search, "page");
        settle(&mut search, &screen);
        assert_eq!((search.tint(0), search.tint(1)), ((None, true), (None, true)), "none while it is typed");
        search.enter(&screen);
        assert!(search.poll(&screen, &square).tinted, "asked for");
        assert_eq!((search.tint(0), search.tint(1), search.tint(2)), ((None, false), (None, false), (None, true)));
        assert!(search.poll(&screen, &square).tinted, "and come");
        let (first, settled) = search.tint(0);
        let first = first.unwrap();
        assert!(settled && first.key != 0);
        assert_eq!(first.marks, [([0.0, 0.0, 4.0, 10.0], true), ([5.0, 5.0, 9.0, 15.0], false)]);
        assert_eq!(search.tint(1).0.map(|tint| tint.marks.clone()), Some(vec![([0.0, 0.0, 4.0, 10.0], false)]), "a page near the screen");
        assert_eq!((search.tint(2), search.tint(3)), ((None, true), (None, true)), "no match, and not near");
        assert!(!search.poll(&screen, &square).tinted, "nothing new");
        // The next match: the same rectangles, the other marked, another tint.
        search.step(true, 1, &screen);
        assert!(search.poll(&screen, &square).tinted);
        let (second, _) = search.tint(0);
        let second = second.unwrap();
        assert_eq!(second.marks, [([0.0, 0.0, 4.0, 10.0], false), ([5.0, 5.0, 9.0, 15.0], true)]);
        assert_ne!(second.key, first.key);
        // Paged on: page 4 is on screen, its rectangles asked for, and page 1's tint let go of.
        let on = Screen { top: 3, pages: &[3], near: &[2] };
        assert!(search.poll(&on, &square).tinted);
        assert_eq!((search.tint(3), search.tint(0)), ((None, false), (None, true)));
        search.poll(&on, &square);
        assert!(search.tint(3).0.is_some_and(|tint| tint.marks == [([0.0, 0.0, 4.0, 10.0], false)]));
        // Typed on: no tint.
        search.edit();
        assert!(search.poll(&on, &square).tinted);
        assert_eq!(search.tint(3), (None, true));
    }

    proptest::proptest! {
        /// Whatever the page, the scale and the marks: no panic, alpha as it was, a channel within
        /// its alpha where it was, and nothing changed outside the pixels a mark covers.
        #[test]
        fn a_tint_never_leaves_its_marks(
            (width, height) in (1usize..24, 1usize..24),
            seed in proptest::num::u8::ANY,
            scale in (0.1f32..4.0, 0.1f32..4.0),
            marks in proptest::collection::vec((proptest::array::uniform4(-8.0f32..40.0), proptest::bool::ANY), 0..6),
            odd in proptest::collection::vec((proptest::array::uniform4(proptest::num::f32::ANY), proptest::bool::ANY), 0..3),
        ) {
            let before: Vec<u8> = (0..width * height).flat_map(|i| {
                let alpha = seed.wrapping_mul(31).wrapping_add((i * 37) as u8);
                let channel = |n: usize| ((i * n) as u8).min(alpha);
                [channel(7), channel(11), channel(13), alpha]
            }).collect();
            let mut after = before.clone();
            tint(&mut after, width as u32, height as u32, scale, &marks);
            for (i, (was, is)) in before.chunks(4).zip(after.chunks(4)).enumerate() {
                let (x, y) = ((i % width) as f32, (i / width) as f32);
                let covered = marks.iter().any(|([x0, y0, x1, y1], _)| {
                    (x0 * scale.0).floor() <= x && x < (x1 * scale.0).ceil() && (y0 * scale.1).floor() <= y && y < (y1 * scale.1).ceil()
                });
                proptest::prop_assert!(covered || was == is, "pixel {i}: {was:?} to {is:?}");
                proptest::prop_assert_eq!(was[3], is[3]);
                proptest::prop_assert!(is[..3].iter().all(|v| *v <= is[3]));
            }
            // Marks that aren't numbers, or are anywhere at all: no panic.
            tint(&mut after, width as u32, height as u32, scale, &odd);
        }
    }

    proptest::proptest! {
        /// Whatever the text and the query: no panic, every match within its chunk's folded text,
        /// on character boundaries, in order and apart; the original's characters it covers within
        /// the original, and folding them gives the match back, whitespace runs aside.
        #[test]
        fn matches_lie_within_their_text(
            chunks in proptest::collection::vec("[aAbBéÉİσς \t\n\u{307}]{0,40}", 0..6),
            query in "[aAbéİς ]{0,4}",
        ) {
            let fake = Rc::new(Fake { chunks: chunks.clone(), ..Fake::default() });
            let mut search = search(&fake);
            type_in(&mut search, &query);
            settle(&mut search, &at(&[0]));
            let needle = needle(&query);
            proptest::prop_assert_eq!(search.count, search.hits.values().map(Vec::len).sum::<usize>());
            for (chunk, hits) in &search.hits {
                let Some(Chunk::Read(folded)) = search.chunks.get(*chunk) else { panic!("matches in a chunk not read") };
                let mut last = 0;
                for (start, end) in hits {
                    let (start, end) = (*start as usize, *end as usize);
                    proptest::prop_assert!(last <= start && start < end && end <= folded.text.len());
                    proptest::prop_assert_eq!(folded.text.get(start..end), needle.as_deref());
                    last = end;
                }
                let original: Vec<char> = chunks[*chunk].chars().collect();
                let covered = folded.originals(hits);
                proptest::prop_assert_eq!(covered.len(), hits.len());
                for (from, to) in covered {
                    proptest::prop_assert!(from < to && to <= original.len());
                    let again = Folded::new(&original[from..to].iter().collect::<String>()).text;
                    proptest::prop_assert!(again.contains(needle.as_deref().unwrap().trim()), "{again:?}");
                }
            }
            // And the keys that go among them never panic, nor leave the reader on a match that isn't one.
            for (forward, count) in [(true, 1), (false, 3), (true, usize::MAX), (false, 0)] {
                search.enter(&at(&[0]));
                search.step(forward, count, &at(&[chunks.len()]));
                proptest::prop_assert!(search.at.is_none_or(|at| search.ordinal(at).is_some()));
            }
        }

        /// Whatever rectangles a source gives, those kept are on a page of the document, within it.
        #[test]
        fn rectangles_lie_within_their_page(
            rects in proptest::collection::vec((0usize..4, proptest::array::uniform4(proptest::num::f32::ANY)), 0..80),
        ) {
            let fake = Fake::of(&["needle", "needle"]);
            fake.state.borrow_mut().rects = Some(rects);
            let mut search = search(&fake);
            type_in(&mut search, "needle");
            settle(&mut search, &at(&[0]));
            search.enter(&at(&[0]));
            settle(&mut search, &at(&[0]));
            for placed in search.placed.values() {
                let rects = placed.rects.as_ref().unwrap();
                proptest::prop_assert!(rects.len() <= MAX_LINES);
                for (page, [x0, y0, x1, y1]) in rects {
                    proptest::prop_assert!(*page < 2 && 0.0 <= *x0 && x0 < x1 && *x1 <= 100.0 && 0.0 <= *y0 && y0 < y1 && *y1 <= 100.0);
                }
            }
        }
    }
}
