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
