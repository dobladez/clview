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

## Task 2: a PDF's text, read in a worker

- **The worker's protocol** (`pdfcraft.rs`): `PASSWORD` (the password, apart, so that the PDF's bytes
  are a payload of their own), `OPEN` (the bytes), `READ` (a page: its text), `PLACE` (a page and a
  range of characters: its rectangles). The last page read is kept, with the character each glyph
  starts at, so a range is a binary search; two pages' glyphs are never held together.
- **The PDF is never copied in the viewer.** `Link::request` built a frame, a second copy of its
  payload; a 500 MiB PDF would have been held three times while it was sent. `worker.rs` now writes a
  frame as its header and its payload, and takes a payload shared (`Link::request_shared`, an
  `Arc<Vec<u8>>`, the one the renderers read). Not in the spec's list for `worker.rs`; the rest is: a
  heap cap for each kind (`Worker::heap`; the labels' 1 GiB moved to `svg/html.rs`, unchanged), the
  kill handle (`Link::killer`, `Kill`), and its `cfg_attr`, which now allows unused parts unless both
  `svg-html` and `pdf` are on, since each uses its part. `MAX_WORKERS` was already one count.
- **The caps** are in [ADR-0027](../adr/0027-a-pdfs-text-read-in-a-worker-process-for-its-heap-cap.md),
  with what they rest on: 512 MiB a PDF, a heap of that and 256 MiB, 5 s of CPU a request, 1 MiB of
  text a page, 120 s of requests a document, three workers killed. Measured on 1,542 PDFs (64,940
  pages) and the synthetic hostile pages, with a tool kept beside the ADR, `docs/adr/0027-measure/`,
  as ADR-0025's is: the scratch benchmark, made to count the heap a page takes with an allocator of
  its own, and its two Python generators ported into it (`measure gen off|grid`), since the repo is
  Rust only. The scratch copies stay outside the repo.
- **One real document changed the CPU cap.** The spec's figure was 25 ms a page at most; one book of
  764 pages has pages of 14.0, 8.9, 3.9 and 1.7 s. At 2 s a page it would have lost three pages and
  come within a slow machine of losing the rest (a fourth kill); at 5 s it loses two.
- **"Past three kills"** is the fourth: three workers killed each cost their page, and the fourth
  ends the reading.
- **After the document's 120 s,** rectangles are still answered by the worker then alive (the
  matches found are still to be gone to), and their time isn't the document's; after the kills, or
  with no worker, nothing is.
- **A page of a million glyphs is unread for its text's size** (1.1 MB, past 1 MiB), not its memory
  (224 MiB): the worker answers that it is too long rather than send it.
- **Spaces.** A space drawn in the PDF is a glyph of its own, with a box; the space a line's end
  becomes is not, and a range that is only that has no rectangle.
- **The clipping spike** (the spec's open question): a workbook with a string cut off by its
  neighbour, a number too wide for its column, a hidden column and a formula, searched through its
  PDF. The clipped part is found; the number is searched as it prints (`1E+14`, never `###`); the
  hidden column isn't printed, so isn't found; a formula is found by its value. Dropping glyphs
  outside their clip isn't the backend's to do: pdfcraft gives glyphs and boxes, no clips. So it is
  a known difference in usage, and ADR-0027 names what pdfcraft could give as a follow-up. A
  comment's text and a form field's value were checked too: both found.
- **Changed in task 1's code, found by running the real thing:**
  - `Enter` with no match known yet went to the first match of the file once one arrived; with a
    whole short PDF read in one pass, that was page 1 from page 2. It now goes as `Enter` would
    have, by what is known when the first match is: the first at or after the top of the screen
    (`Search::first_from`).
  - Why a page was left unread was the status line's message, which a search being typed hid. It is
    now said with the search while it is typed (`/ · read 0 of 5 pages  this PDF is too large…`).
- **Tests.** The pager's, on a pseudo-terminal with the real worker: a five-page PDF searched, its
  worker seen with `ps` and gone with the search, the file and the pager; a PDF opened from a list;
  a page of four million glyphs (written by the test, 15 MiB) costing that page, four of them the
  rest, and `Esc` killing a worker busy on one at once. That test fits the real heap cap, as
  ADR-0026's do: each kill takes a worker to 768 MiB for about a second, six times. `gridcraft.rs`'s: a
  workbook without its styles part, so wrapped in `Noted`, searched. `pdfcraft.rs`'s unit tests run
  the worker's session directly, and the reader's thread against the worker on a thread; they take
  turns (`ONE_AT_A_TIME`), since the workers alive in a test binary are counted together.
- **Files beyond the task's list:** `src/source/svg/html.rs` (its heap cap), `tests/it/pdfcraft.rs`
  (the line of `WORKERS` allowed to name the backend), `src/view/search.rs` and `src/view/pager.rs`
  (the two changes above), `CONTRIBUTING.md` (the pin, the tests), `docs/adr/README.md`,
  `docs/adr/0027-measure/`.
- **Found on the way, and put where it belongs:** four rows of usage's Limits (the spreadsheets')
  sat under Known issues, after the table ended; they are back in the table.
- **Verify.** `cargo test -j 2 --test it view_pager` and `-- gridcraft`; each backend alone,
  `pdf,svg` and none, clippy and tests.
- **Seen failing**, each by changing the line it covers and copying the file back (`cmp`):
  `Kill::kill` doing nothing (`a_worker_is_killed_from_another_thread…`, which then waits out its 30 s);
  a shared payload's bytes not written (`a_shared_payload_is_written…`); no space between lines
  (`a_pages_text_is_its_words…`); a range's first glyph taken as the one before it
  (`a_range_of_a_pages_text…`); a thing given again while listed
  (`a_pdfs_text_comes_a_page_a_chunk…`); the document's time unbounded, and placing a match
  spending it (`past_the_documents_time…`); `Pdf::texts` giving none (all four pager and workbook
  tests); the search's end not killing the worker, a worker gone asked again, and kills never
  ending the reading (`a_page_past_the_workers_heap…`, each at a different step); `Noted` giving no
  text (`a_workbook_that_opened_with_warnings…`); a reason hidden while a search is typed
  (`the_status_line_shows_the_search…`); a waiting `Enter` going to the first match of the file
  (`text_read_later…`).
