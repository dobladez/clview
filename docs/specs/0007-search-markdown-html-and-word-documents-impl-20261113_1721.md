# Spec-0007: implementation notes (2026-11-13)

Notes made while implementing [Spec-0007](0007-search-markdown-html-and-word-documents.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 1: the contract's pages, and the queue shared

- **`chunks_on(page)`** replaces `chunk_of(page)` in `Texts`. The search puts what a source says
  right as before: each page's range starts and ends no earlier than an earlier page's, within the
  chunks searched (`Search::on`, a range a page, in place of `starts`). A page's text past
  `MAX_CHUNKS` is a page never read whole (`Page::Beyond`), as a page past them was.
- **A chunk on two pages.** The matches of a chunk the top page shares with the page before may be
  on that page: `Search::above` counts a match as before the screen's top when all its rectangles
  are on earlier pages, so `Enter` (or a step from the top of the screen) on the second page waits
  for those rectangles (`shares_its_first`), as on a page taller than the window, and lands on a
  match in the paragraph's tail. A match is on screen when one of its rectangles is on a page on
  screen; with none known, when every page its chunk is on is. Its page (`page_of`) is the first its
  chunk is on, until its rectangles say otherwise, as before.
- **The status line counts pages** (`search::Page`): a page is read once every chunk on it is, and
  unread if one of them is; `· page N unread` for one, `· read X of Y pages` for more, or with
  reading stopped, or pages past the cap. `parts` is gone. A paragraph across a page break that
  can't be read costs both its pages.
- **The queue** (`src/source/texts.rs`): `Queue` (`new`, `start`, `set_queue`, `try_recv`), the
  backend's `Answer` (a chunk's text, a range's rectangles) run on its thread, and `Ending::on_end`,
  what the search's end does, which the reader sets and changes as it goes (the PDF's: kill the
  worker it has just greeted, or nothing once it is gone). `on_end` answers `false` once the search
  has ended, which is how the PDF's reader learns to let a worker just started go, as it did under
  the old lock. The trait is `Answer`, not `Read`: `pdfcraft.rs` already has a `Read`, and
  `wordcraft.rs` uses `std::io::Read`.
- **The PDF's preflight** (a file past `MAX_PDF`, a password too long) moved from `Reader::run`,
  which is gone, to `PagesText::new`, before the queue's thread starts: the same answers.
- **The three queue tests moved to `texts.rs`** with a reader of their own (`Two`) in place of the
  PDF's worker: `an_answer_whose_request_was_dropped_never_comes`,
  `answers_wait_for_the_viewer_within_a_budget`, `the_queue_stays_quick_with_tens_of_thousands_of_pages`,
  and `what_is_wanted_is_read_in_order_and_given_once` beside them. `pdfcraft.rs` keeps its worker's
  own (`a_pdfs_text_comes_a_page_a_chunk_through_its_worker`, `past_the_documents_time…`).
- **`texts.rs` is compiled with the `pdf` feature** for now; the next task adds `word`.
- **The 🟡 on `page_hint`, answered from wordcraft's source** (read for task 2): for the body story
  `DocLayout::pieces` takes a paragraph's pieces from its index of every page, and the hint is used
  only for headers, footers and notes. So a wrong hint can't lose a piece of a body paragraph.
- **Files beyond the task's list:** none.
- **Seen failing**, each by changing the line it covers and copying the file back (`cmp`): each
  page's chunks made again from where each starts, as `chunk_of` gave them
  (`a_chunk_across_a_page_break_is_searched_on_either_page_alone`,
  `chunks_follow_the_pages_given_and_where_they_start`,
  `what_is_left_unread_is_counted_in_pages_whatever_the_chunks`); a page's matches never taken for
  above the screen's top on a page all on screen (the first two); chunks counted, as `parts` where
  they aren't pages (`what_is_left_unread…`, which then said `read 4 of 6 parts`); the queue's
  drop not doing what was set (`the_search_ending_does_what_the_reader_set_last`); and, moved, the
  answers kept past their request and no budget on them (`an_answer_whose_request…`,
  `answers_wait…`).
- **Verify.** `cargo test -j 2 --bin clv` (456, among them `texts::` and `search::`), `cargo test
  -j 2` whole (the PDF search tests of Spec-0006 in `tests/it/view_pager.rs`, `view_tmux.rs`,
  `pdfcraft.rs` and `gridcraft.rs` unchanged and passing), the no-backend gates, and each backend
  alone, clippy and tests, and `pdf,svg`.

## Task 2: a document's text and its matches' places

- **The layout shared through a `Weak`, not one `Arc` the `Document` keeps.** The spec has the
  `Document` keep one `Arc<Cache>` for `pool()` and `texts()`. But the pager makes a fresh pool on
  entering a file from a list (`Pager::enter`) so that the old one lets go of what its files
  rendered with; a `Cache` the `Document` held would keep every Markdown file of a list laid out
  for as long as the list is open. So `Document::shared` hands out the `Cache` a pool or a search
  still holds, or a new one, keeping only a `Weak`: one layout while either is alive, none once both
  are gone. A pool's render threads and the search's thread let go of it when they stop, a moment
  after their owners are dropped; the test waits for that.
- **`chunks_on` for every kind**: opening already walks the body's paragraphs for Markdown and HTML
  (`paragraphs_of`, for `anchor` and `find`); it now does so for every kind (`paras_of`), keeping
  each page's first to last paragraph (`Document::on`) and their count, and `paragraphs` for the
  kinds that lay out again, as before. A page's range is from its lowest paragraph to its highest:
  a table row split over a page break, its cells side by side, may make that range take in a
  paragraph of the next page, which only costs asking for rectangles that turn out to be elsewhere.
- **A page with no paragraph** says `0..0`. The search took that as starting at 0 and stretched it
  to the page before's end; it now takes an empty range as none, between the pages around it
  (`search.rs`, a line in `chunks_follow_the_pages_given_and_where_they_start`). Not in the task's
  files: found writing this one.
- **A chunk's text** (`pieces`): the paragraph's `text`, an `OBJ` as its object's `plain_text()`, a
  soft hyphen as nothing and a non-breaking hyphen as `-` (both decided here: the spec said each
  inline object as its text, and was silent on these; without them `hyphen` doesn't match
  `hy­phen`, nor `-` a non-breaking one), a column break as a space (`U+000E` isn't whitespace).
  Line breaks, page breaks, tabs and no-break spaces are whitespace already, which the search makes
  one space.
- **A range's bytes** (`bytes_of`): from where the piece its first character is in starts to where
  the piece its last is in ends, so a match inside a field's result selects the field whole.
- **A paragraph over 1 MiB** is refused by the backend, before the viewer's own `MAX_CHUNK`, so it
  says which page it is on: `a paragraph on page N has more text than a search keeps of one`. Every
  page the paragraph is on is then unread by the rule of task 1; in the test, the two short
  paragraphs share their pages with it, so `read 0 of 264 pages`.
- **The hyphen drawn at a line's end is inside the first rectangle**, as `selection_rects` gives it:
  checked on the page's pixels (`a_word_hyphenated_at_a_lines_end_is_placed_on_both_lines`, no ink
  right of it on its line).
- **`page_hint`** (the 🟡): the paragraph's first page. For the body it is unused (task 1's note).
- **Panics.** wordcraft's calls on the queue's thread (`para_paths`, `selection_rects`) go through
  `caught`, as the renders do; a panic there would otherwise end the thread and leave the search
  reading for ever.
- **`opened`**, the body of `Wordcraft::open_at`, is a function of its own returning the `Paged`,
  so the unit tests can read the layouts counted (`Document::layouts`, `#[cfg(test)]`).
- **The 🟡 on `MAX_CHUNKS`.** Counted with a throwaway test through `opened` (debug build): Rust's
  `releases.md` (894 KB, mostly short list items) is 7,341 body paragraphs on 462 pages, laid out
  in 0.46 s; tokio's `CHANGELOG.md` (195 KB), 2,126 on 134. So real documents sit one or two orders
  under 100,000. A synthetic Markdown file of short list items can pass it (8 MiB of text and
  500,000 marks allow several hundred thousand): its first 100,000 paragraphs are searched, the
  pages past them counted unread. No number of its own is wanted for paragraphs; the cap stays.
- **Tests.** Unit, in `wordcraft.rs`: `a_documents_chunks_are_its_body_paragraphs_a_tables_cells_included`,
  `a_fields_result_is_its_text`, `a_word_hyphenated_at_a_lines_end_is_placed_on_both_lines`,
  `a_paragraph_over_a_mebibyte_is_left_unread_with_its_page`, `the_pool_and_a_search_lay_a_document_out_once`.
  In the pager, in `tests/it/wordcraft.rs` (so `--test it wordcraft` runs them), with
  `view_pager.rs`'s helpers made `pub(crate)`:
  `a_word_documents_body_is_searched_a_hyphenated_word_a_cell_and_a_field`,
  `a_paragraph_across_a_page_break_is_searched_on_the_page_shown_and_on_dark_paper`,
  `a_paragraph_over_a_mebibyte_is_left_unread_and_the_rest_searched`. The scroll to a match isn't
  tested for a document: its pages are as tall as the window, so nothing scrolls unzoomed; the
  scroll itself is the search's, tested on PDFs.
- **Files beyond the task's list:** `src/source.rs` (the `cfg` of `texts.rs` and of `Got`'s
  `expect`), `src/view/search.rs` (an empty page), `CHANGELOG.md`, `CONTRIBUTING.md` (Tests).
  `tests/it/view_pager.rs` only lends its helpers.
- **Seen failing**, each by changing the line it covers and copying the file back (`cmp`):
  `Paged::texts` giving none (the five unit tests and the three pager tests); the soft hyphen kept
  (`a_documents_chunks…`); an object as nothing (`a_fields_result…`); characters taken for bytes
  (`a_documents_chunks…`, by where "and" starts); no paragraph cap (`a_paragraph_over…`, unit); the
  pool with a `Cache` of its own (`the_pool_and_a_search…`: two layouts), and the shared one never
  let go of (the same, at its wait); pages' paragraphs kept only for Markdown and HTML
  (`a_word_documents_body…`); an empty page taken as said (`chunks_follow…`); and task 1's
  `above` without the shared chunk (`a_paragraph_across…`, which then never says `2/2`).
- **Verify.** The commands pass (below). The manual check (kitty, `clv README.md`, `/search⏎`, `n`,
  `p`, then `d`) is the owner's and hasn't been done: the task's box stays open until it is.
- **Gates.** The five pass (461 unit tests, 213 in `tests/it`), and each backend alone, clippy and
  tests, and `pdf,svg`. `word` alone first failed clippy: `Queue::ending` and `Ending::on_end` are
  the PDF's reader's alone, so dead code there; their `expect(dead_code)` now holds without `pdf`
  outside tests.

## Task 3: a search across a relayout

- **`Search::relaid`** builds the new level's search from the old one and the new source's `Texts`:
  a `Search::new` on the new text, given the old query, whether it is typed, and whether `Enter`
  is waiting; then, while the new text has as many chunks (the spec's test of "the same text"),
  the chunks read and those left unread, the matches, the count, the match the reader is on and
  what is still to match. The rectangles (`placed`), tints and queue are not carried: the new
  `Texts` is asked for every rectangle anew, and the old one, dropped, takes what was still asked
  of the old layout with it. Fewer or more chunks: the search starts over with the query kept,
  counted anew (`/with 80`), with no match to be on.
- **The view on the match** (the spec's "the view on the match"): `Pager::replace` already puts the
  view where the paragraph at the top of the screen went. On top of that, if the reader's match was
  on screen (`Nav::take_search`, `Search::showing`), the new search goes back to it once its new
  place is known (`revealing`, as after `n`), bringing its page on screen; a match scrolled away
  from isn't followed. Decided here: the spec said only "the view on the match".
- **Typed while laid out again.** `]` while a search is typed is part of the query, so the pager
  test lays out again with `0]` and types `/wit` in the same write: the relayout lands while the
  query is typed, which then counts the whole text's matches (`/wit 60`) and is typed on (`h⏎`).
- **Tests.** `search.rs`: `a_search_outlives_a_relayout_of_the_same_text`,
  `a_search_of_another_text_laid_out_starts_over`. `view_pager.rs`:
  `a_search_outlives_a_relayout_and_starts_over_on_another_text` (`/with⏎ 3n`, `40]`: `4/60`, the
  new layout's page placed with the reader's tint, `n` to `5/60`; the typed case; the file
  rewritten with 80 paragraphs, `40]`: `/with 80`). At 40 characters two pages share the screen,
  so the test looks at every page placed since, not the last.
- **Files beyond the task's list:** `docs/architecture.md`, `CHANGELOG.md`.
- **Seen failing**, each by changing the line it covers and copying the file back (`cmp`): the
  search not handed to the new level (the pager test: never `4/60` after `40]`); every relayout
  starting over (`a_search_outlives…`); none starting over (`a_search_of_another_text…`); the view
  never following the match, and the typing dropped (`a_search_outlives…`).
- **Verify.** `cargo test -j 2 --test it view_pager` passes, with the five gates (463 unit tests,
  214 in `tests/it`) and each backend alone and `pdf,svg`. The manual check (kitty, a Markdown
  file, `/word⏎ n`, then `]` and `+`) is the owner's: the task's box stays open until it is done.

## The Done checklist

- **The quality gates** pass at the last code commit (`8389dc3`): the five, and each backend alone
  and `pdf,svg`; `--test docs` again after the docs below.
- **Tests added, each seen failing** (the tasks above say how): in `texts.rs`,
  `the_search_ending_does_what_the_reader_set_last` (and the three moved, still failing without
  their fixes); in `search.rs`, `a_chunk_across_a_page_break_is_searched_on_either_page_alone`,
  `what_is_left_unread_is_counted_in_pages_whatever_the_chunks`,
  `chunks_follow_the_pages_given_and_where_they_start` (rewritten),
  `a_search_outlives_a_relayout_of_the_same_text`, `a_search_of_another_text_laid_out_starts_over`;
  in `wordcraft.rs`, the five of task 2; in `tests/it/wordcraft.rs`, the three of task 2; in
  `tests/it/view_pager.rs`, `a_search_outlives_a_relayout_and_starts_over_on_another_text`.
- **Manual** (kitty and tmux): the owner's, open, with the two tasks' own.
- **The docs.** `CHANGELOG.md`'s search entry names documents and a search outliving a relayout;
  usage's [Searching a document's text](../usage.md#searching-a-documents-text) says what is
  searched in a document and how, and the Limits row; architecture's
  [A document's text](../architecture.md#a-documents-text-wordcraftrs), the contract's
  `chunks_on`, the queue and a search across a relayout;
  [ADR-0030](../adr/0030-a-documents-text-read-from-the-pools-layout-on-a-thread.md) for the text
  read from the pool's layout, on a thread, through a `Weak` (the choice is task 2's; written after
  it, in its own commit); README's Plans bullet is out.
- **Found on the way:** `main` gained two commits after this branch was made (`c980915`, a tint
  test in `view_pager.rs` waiting for page 1 after `Esc`, and `071da53`, CI), which a merge will
  have to meet; this branch makes some of `view_pager.rs`'s helpers `pub(crate)`.

## After the first review (Codex, 2026-11-13)

Three findings, two on task 1 and one on task 2.

- **An empty page's range took the page before's chunks** (`[0..2, 0..0, 2..3]` made the blank page
  `0..2`). Already put right in task 2 (an empty range is none, between its neighbours), found
  while writing the Word backend's empty pages. Its test now has the reviewer's case of its own:
  `a_page_without_text_searched_alone_has_none`, the blank page alone (`--pages 2`) reading nothing
  and saying `no text in this file`. Seen failing with that fix taken out (`/a 2`).
- **A match across a page break, scrolled past, was taken for on screen, and Enter chose it.**
  `above` asked whether all of a match's rectangles were on earlier pages, else whether its first
  ended above the part shown: a match from the bottom of page 1 onto the top of page 2 was
  neither, so Enter on page 2 scrolled past it went back to it. `on_screen` took any rectangle on a
  page shown for on screen. Each rectangle's place is now kept as shares of its page
  (`Placed::shares`, beside `span`, which stays the first's, where the view goes), and a match is
  before the screen's top when every rectangle is (`Placed::before`), on screen when one is in what
  is shown (`Placed::shown`). Test: `a_match_across_a_page_break_scrolled_past_is_above_the_screen`,
  seen failing on the old code (Enter: `1/2` for `2/2`) and with `shown` taking any rectangle on a
  page shown (the match scrolled past taken for on screen; `the_first_match_on_screen_is_found…`
  failed too). The test's fake places a match across its 50 characters a page on both pages.
- **A table row over several pages made pages' ranges wrong.** Per page, from its least paragraph to
  its most, a row's right cell (fourth in the text, on page 1) made page 1 `0..4`, taking in the
  left cell's paragraphs on pages 2 and 3, and page 2 `1..2`, ending before page 1's, which the
  search's clamp then turned into page 2 holding the right cell's. Measured on a docx: a row of
  three 40-word paragraphs in a narrow cell runs over several pages, its right cell on the first.
  Now each page runs from the least paragraph on it or any page after to the most on it or any
  page before (`ranges_of`): monotone, each page's own paragraphs within it, a page with none
  empty. The contract stays a range (the reviewer's suggestion, taken): a page's range may take in
  a row's paragraphs on its neighbours, and `chunks_on`'s doc says so. Decided: rather than filter
  matches by their rectangles' pages (which the count, made before any rectangle is asked for,
  can't wait on), usage says that a table row over pages is searched with every page it is on,
  with `--pages` too, and is unread with each. Tints and where the view goes already follow the
  rectangles. Test: `a_pages_chunks_take_in_a_table_row_over_several_pages`, seen failing with
  each page's own least to most (`[0..4, 1..2, 2..3, 4..5]`).
- **A test of task 2 made patient.** In a full run `a_paragraph_across_a_page_break…` failed once,
  and passed three times of three alone: after `g` the first page placed may be the page as
  rendered, its tint coming with the next, and the test took the first placed. It now waits for a
  page placed since with the reader's colour (`placed_tinted`), after `G` too.
- **Gates.** The five pass; `tests/it` twice more in full, under the same load.
