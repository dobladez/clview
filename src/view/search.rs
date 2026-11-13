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

use std::collections::{BTreeMap, BTreeSet, VecDeque};

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

    /// What it takes of the memory a search keeps.
    fn size(&self) -> usize {
        self.text.len().saturating_add(self.runs.len().saturating_mul(std::mem::size_of::<(u32, u32)>()))
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

/// What a search's queue was last set for ([`Search::queue`]).
#[derive(Debug, PartialEq, Eq)]
struct Queued {
    centre: usize,
    reading: bool,
    placed: Vec<(usize, u32)>,
}

/// What of the screen a search goes by: the document's page at its top left, and the pages on it.
#[derive(Clone, Copy, Debug)]
pub(super) struct Screen<'a> {
    pub(super) top: usize,
    pub(super) pages: &'a [usize],
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
    /// What the queue was last set for: the chunk read outward from, whether reading had stopped,
    /// and the matches whose rectangles are wanted.
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
        let status = self.status();
        polled.changed = status != self.status;
        self.status = status;
        polled
    }

    /// Whether there is text still to read or to match: [`Search::poll`] has more to do.
    pub(super) fn busy(&self) -> bool {
        self.reading() || !self.pending.is_empty()
    }

    /// Whether text is still being read.
    fn reading(&self) -> bool {
        !self.full && self.unread > 0
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
    }

    /// Tell the source what is wanted now, if that changed: where the matches on the screen's
    /// pages are, the reader's first, at most [`MAX_PLACED`] of them and none while the query is
    /// typed; then the text still unread, outward from the top of the screen.
    fn queue(&mut self, screen: &Screen<'_>) {
        if self.dirty || self.shown != screen.pages {
            self.dirty = false;
            self.shown = screen.pages.to_vec();
            let mut keys: BTreeSet<(usize, u32)> = BTreeSet::new();
            if !self.typing {
                keys.extend(self.at);
                let chunks = screen.pages.iter().flat_map(|page| chunks_of(&self.starts, *page));
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
        let wanted: Vec<(usize, u32)> = self.placed.keys().copied().collect();
        let queued = Queued { centre, reading, placed: wanted };
        if self.queued.as_ref() == Some(&queued) {
            return;
        }
        let wanted = &queued.placed;
        // The reader's match first, then the others in order.
        let rects = self.at.into_iter().chain(wanted.iter().copied().filter(|key| Some(*key) != self.at));
        let rects = rects.filter_map(|key| self.placed.get(&key).map(|placed| Want::Rects { chunk: key.0, from: placed.from, to: placed.to }));
        let mut queue: Vec<Want> = rects.collect();
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
        Screen { top: page[0], pages: page }
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
        assert_eq!(search.poll(&at(&[2]), &square), Polled { changed: true, go: None, said: None });
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
        // Typing again asks for none.
        search.edit();
        settle(&mut search, &at(&[1]));
        assert!(fake.state.borrow().queue.is_empty());
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
