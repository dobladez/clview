# Spec-0006: implementation notes (2026-11-13)

Notes made while implementing [Spec-0006](0006-search-a-pdfs-text.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 1: the contract and the search, without a backend

- **The contract** (`src/source.rs`). `Source::texts() -> Option<Box<dyn Texts>>`; `Texts` has
  `chunks()`, `chunk_of(page)`, `set_queue(Vec<Want>)` and `try_recv() -> Option<Got>`. `Want` is
  `Text(chunk)` or `Rects { chunk, from, to }`, the range in characters (`char`s) of the chunk's text
  as given; `Got` is `Text(chunk, Result<String, String>)` or `Rects(Want, Result<Vec<(page, [f32;
  4])>, String>)`. One thing the spec left open: a thing given isn't given again while it stays
  listed (as `threads.rs` keeps `delivered`), so the viewer sets the queue only when what it wants
  changes, not on every arrival. `Got` carries `expect(dead_code)` outside tests until task 2 gives a
  backend that builds one.
- **How a backend says why it can't** stays inside the contract: a chunk's `Err(why)`. The viewer
  marks the chunk unread and tells the reason once (the status line's message), so "too large to
  search", "no worker could start" and "the rest is left unread" are all a backend answering each
  chunk asked with the same reason; there is no other channel.
- **`Step::Search`.** `Nav` has no source (it is pure, and tested without one), so `/` in a file's
  pages returns `Step::Search` and the pager calls `source.texts()` and `Nav::start_search`. `/` with a
  search kept needs no source: it edits the query in `Nav`.
- **The fold.** `search::fold` is a character at a time, as the spec says; the filter's was
  `str::to_lowercase`, which differs in one case, a capital sigma at a word's end (`ΟΔΟΣ` gave
  `οδος`). So `ς` folds to `σ` too: every name and query that matched before still does, and
  `οδοσ` now matches too. `fold` (the filter's) leaves whitespace alone; `Folded` (the search's)
  makes each run one space.
- **Narrowing is by chunk, not by match.** A longer query's matches aren't a subset of the shorter
  one's when matches don't overlap (`aaab`: `aa` matches at 0, `aab` at 1), but they are all in
  chunks the shorter one matched in. So a character typed matches those chunks again, and only
  those. Once the 10,000 cap dropped matches, the chunks that lost theirs are no longer known, and
  every chunk is matched again. The test counts scans through a `#[cfg(test)]` field, `Search::scans`:
  the narrowing changes no result, so nothing else shows it.
- **Decided where the spec was silent:**
  - `Esc` while typing leaves no search even when a kept query was being edited (the spec's words;
    the filter puts the kept one back).
  - A query of whitespace alone shows no count while typed (`/  `), and `no match` once kept.
  - `search wrapped` is part of what the search says (`/page 1/5 · search wrapped`), until the next
    key, rather than a message that would hide the count.
  - A kept search is always on the status line, before any message (`/word 2/5  page 2: …`): a
    page's standing warning would otherwise hide it for as long as the page is on screen.
  - `N` without a search says `no search: / starts one` (it is one of the viewer's keys now, and
    `the_viewers_keys_are_those_listed` wants each to do something somewhere).
  - With `--pages`, only the chunks of the pages given are read and searched, in the document's
    order.
  - "At or after the top of the screen" is by chunk: the chunk the first page on screen starts
    with. Zoomed in and scrolled half way down a page, `Enter` may go up to a match above on the
    same page.
  - "While it is on screen" for `n` and `N`: while the match's page is one of those on screen.
  - Scrolling to a match: a third of the way down the screen, only when it isn't wholly on screen,
    and not if a key was pressed between going to it and its rectangles arriving
    (`Search::heard`), so late rectangles never pull the view back.
  - The unread suffixes: `· page 40 unread` for one chunk, `· read 120 of 500 pages` for more or
    when reading stopped (`parts` where chunks aren't pages).
  - Caps the spec didn't name: 262,144 chunks, 1,024 matches placed at a time, 64 rectangles a
    match, and 4 MiB folded and 4 MiB matched a pass.
- **Files beyond the task's list:** `docs/architecture.md` (the contract, `search.rs`, a section on
  searching), `docs/usage.md` (one sentence: what `/` says in a file's pages at this commit),
  `CONTRIBUTING.md` (Tests), and Spec-0007's Open questions (below).
- **Raised for Spec-0007:** which page's a paragraph across a page break is, for the rectangles
  asked for a screen (`chunks_of`): added to its Open questions.
- **Verify.** `project.md`'s `--lib <name>` doesn't run here: clview has no library target, so it is
  `cargo test -j 2 --bin clv search` and `cargo test -j 2 --bin clv pager`. `project.md` says so now.
- **Seen failing**, each by changing the line it covers, running `cargo test --bin clv view::`, and
  copying the file back (`cmp`): the fold (to upper case: `text_is_folded…`,
  `a_match_maps_back…`); whitespace alone matching, and runs not made one; a match's end not past
  its last character (`a_match_maps_back…`, `a_matchs_rectangles…`, the property test
  `matches_lie_within_their_text`); page starts not put in order, and every chunk read whatever
  pages are given (`chunks_follow_the_pages_given…`); no narrowing (`the_count_grows…`, by its scan
  count) and narrowing past the cap (`the_first_ten_thousand…`); `n` not from the top of the screen,
  and `search wrapped` never said (`enter_n_and_shift_n…`, and the pager's `n_and_shift_n…`); the
  first match to arrive not gone to, and waiting for ever (`text_read_later…`, the pager's
  `text_arriving_out_of_order…`); a chunk given twice, the chunk cap, a failed chunk not counted and
  its reason not told (`what_a_source_gives_is_checked`, `a_chunk_that_fails…`); the text cap
  (`reading_stops…`); `no text in this file`; the match cap; the span as points instead of shares;
  rectangles not clipped (the property test `rectangles_lie_within_their_page` too); the queue set
  on every poll. In the pager: a key leaving `search wrapped` up; `Esc`, `Backspace` and `u` not
  clearing a kept search; `n` turning pages with a search kept; `N` silent without one; `no text to
  search` not said; the query uncapped; `Esc` while typing keeping the search; the count before `n`
  ignored; the match's page not brought on screen, and the reader taken off the pages (the property
  test `a_search_never_takes_the_reader_off_the_pages`); no scroll to the match; a kept search hidden
  by a message; `n` missing from the keys a number takes; a number before `/` kept; other keys
  moving while a search is typed.
