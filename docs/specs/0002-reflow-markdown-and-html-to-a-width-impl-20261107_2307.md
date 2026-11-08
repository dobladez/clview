# Spec-0002: implementation notes (2026-11-07)

Notes made while implementing [Spec-0002](0002-reflow-markdown-and-html-to-a-width.md): decisions,
changes from its plan, surprises, and what a later reader would want.

## Task 1: a width, laid out again while the reader waits

- **The page goes into wordcraft's conversion** through `html::parse_with` and `model::to_doc_in`, as the
  spec guessed. `import_with` is those two with `model::begin_import()` between, which is `pub(crate)`:
  it only starts afresh wordcraft's log of what an import dropped (one `log::warn!` per kind per
  document), which nothing here subscribes to, so skipping it changes nothing visible.
- **`page` sits at 83**, as the spec says: Letter's 468 pt of text are 82.98 characters at 5.64 pt
  (JetBrains Mono's advance, 0.6 em, at 9.4 pt: `CODE_ADVANCE`), and a test checks 83 is the nearest
  whole number. At width 40, 37 columns of code fit a line and 38 don't (the block's 16 pt of indent
  are about 3 characters), as the spec expected.
- **The order of the steps** is one function, `stepped`: the multiples of the step from the least to the
  most, and each named value with an `at` after the number equal to it, in declaration order. A typed
  83 then `]` goes to `page` (the number comes before the value set at it), then 84. A named value set
  at or past an end stays, saying `…, the most`, rather than becoming the end's number.
- **Where the width is read from the options.** The backend reads `--width` itself from the `Parsed` it
  opens with (`Width::given`), as it reads `--password`; the viewer parses the same option for the
  shared values (`started`), and refuses one out of the range before any file opens, so the backend
  only clamps. A list's files thus open at `--width` (through `OwnedOptions`) with nothing more.
- **`--pages` is a flag, not a guess.** `Pages` grew `picked`, set by `Shown::pick`, so `--pages 1-3`
  of a three-page file is still refused: pages given are pages as laid out, whatever they number.
- **The status line's slot** is padded to the widest a relayout setting is said (`width page`, 10
  characters, plus `…`), computed from the declarations (`Nav::laid_width`), so stepping doesn't move the
  right side; it is the first thing a narrow window drops.
- **The `c` then `]` message** is `stopped copying page 1` on a document of more than one page, since a
  copy is named by its page there; the test first expected the file's name.
- **In task 1 the job runs on the main thread**, under `catch_unwind`, between the keys and the draw
  (`Pager::relayout`, in the loop after `go_on_into`, so a document gone into after a change is laid out
  before it draws). `say` puts `width 40…` on the status line first, so the wait is seen.
- **Cost at 20, the 🟡 question:** the same as on a page for every file ADR-0012 made to cost the most
  (the table is in ADR-0022's Evidence): importing is what costs, and the least stays at 20. The
  costliest, `<li>`s sharing out 8 MiB, is 5.9 GB today at either width (4.9 in ADR-0012), and in the
  pager, `]` once it shows, peaks at 9.5 GB: the old level's pool holds its layout until the new level
  takes its place. Written into Limits. The files were made just under the marks limit (499,900):
  half a million exactly is refused, which ADR-0012's "half a million" must have been too. Its
  page-break file is `<p style=page-break-after:always>a`, unclosed, which its 17 MiB say.
- **The manual check** (in kitty, `target/debug/clv --pager --width 40 README.md`: narrow tall pages,
  two to a row; `]` widens them) is left to the owner, as asked: not run here.
- **Not in *Files*:** `docs/adr/0022-…`, the ADR the spec's Approach asks for (all four decisions,
  the coalescing among them, whose code is task 2's); `tests/it/wordcraft.rs` was listed; `CHANGELOG.md`,
  `docs/usage.md`, `docs/architecture.md`, `docs/adr/README.md` per *Where a fact goes*.
- **Tests seen failing**, each by copying the file aside, reverting, running, copying back and `cmp`:
  - `step_keys_go_to_the_next_multiple_and_stay_at_the_ends` and
    `named_values_sit_among_the_steps_where_they_are_set`: `stepped` sending a named value with an `at`
    to the least or the most, as before.
  - `a_relayout_key_is_refused_in_a_list_and_with_pages_and_asks_for_no_redraw`: the refusals taken
    out, and every change setting `changed`.
  - `the_status_line_shows_what_the_document_is_laid_out_at`: the slot left out of `status`.
  - `a_width_is_so_many_characters_of_the_code_face_and_page_is_letter` and
    `a_table_is_as_wide_as_the_text_at_any_width`: `Width::page` ignoring the columns.
  - `markdown_and_html_say_their_width_and_lay_out_again_at_another`: `laid_at` empty and `relayout`
    `None`.
  - `the_width_lays_markdown_and_html_out_to_so_many_characters` (`tests/it/wordcraft.rs`):
    `Width::given` always `page`.
  - `the_width_keys_lay_the_document_out_again_and_the_status_line_says_what_it_is_at`,
    `a_copy_still_rendering_stops_when_the_document_is_laid_out_again`,
    `a_document_gone_into_after_a_change_is_laid_out_at_the_new_width_and_its_tile_is_not`,
    `html_through_a_pipe_lays_out_again_too` and `with_pages_the_width_keys_are_refused`
    (`tests/it/view_pager.rs`): `Pager::relayout` doing nothing, with the reverts above.
  - `on_a_pdf_the_width_keys_say_only_markdown_and_html_reflow`: the setting's `missing` message
    changed (its only fix is the declaration).

## Task 2: on the openers' threads

- **No "waiting" set of values.** The spec has the level keep one job under way and one set of values
  waiting. The shared `Settings` already are what waits: on each pass the pager compares them with the
  source's `laid_at`, so `relayout.rs` keeps only the job under way and the values it is for
  (`Relayout<H>`), and `ended(wanted)` says whether its result is for the values wanted now. A change
  while one runs is nothing to record; it starts when the job ends, by the same comparison.
- **Compared by the relayout settings alone.** `relayout::of` takes the relayout settings a document
  has out of the shared values (at their default where they say nothing), so `d` during a layout
  doesn't make its result stale, and `page` said or unsaid is one value.
- **`Openers` runs a job as `Work::Run`**, pushed to the front; the worker takes the job out of its
  `Mutex<Option<Job>>` to run it, so a `Handle` dropped before that takes it instead and the job never
  runs. The result waits in the `Running` until `try_take`. The thread catches a panic as it does for
  an opening.
- **The help screen survives a relayout**: `replace` copies `help` along with the message, so `?` while
  a layout runs isn't thrown off when the new level arrives (the threaded test presses it then).
- **Not in *Files*:** `src/view/mod.rs` (`Opener::openers`, the threads handed to the pager, and
  `mod relayout`), `src/source.rs` (`Handle` re-exported).
- **Tests seen failing**, by copying aside, reverting, running, restoring and `cmp`:
  - `a_job_runs_first_and_its_panic_is_caught` (`source::later`): the job queued last, and run without
    `catch_unwind`.
  - `a_layout_is_wanted_when_a_relayout_setting_differs_and_a_redraw_one_doesnt_count` and
    `a_result_is_taken_for_the_values_still_wanted_and_dropped_otherwise` (`view::relayout`): whole
    values compared, and every result taken.
  - `keys_work_while_the_document_is_laid_out_again_and_the_new_pages_show_with_no_key`
    (`tests/it/view_pager.rs`): the pager waiting on the handle where it starts the job.
  - `a_layout_that_fails_keeps_the_old_pages_and_puts_the_value_back`: `put_back` doing nothing.
  - The `--pages` test from task 1 raced the page worked ahead (it counted a transmission that wasn't
    the key's); it now waits for both pages first.
- **The manual check** (in kitty, the costliest file of *Cost*, `]` three times quickly: keys and
  scrolling keep working, `width 96…` then `width 96`) is left to the owner.

## Task 3: keeping the place

- **An `Anchor` is a paragraph's index** in the body's order (`para_paths`), kept per page as
  `(index, top, height)` from the layout's `Placed::Lines` when a document opens or lays out again,
  for Markdown and HTML only: 12 bytes a placed paragraph, so the costliest file (13,887 pages) keeps a
  few megabytes. `Paged` keeps no layout, so this is what the spec meant by "what its layout keeps for
  it".
- **Where `find` fails**, the pager keeps the reader's page (clamped), not the first page: the two
  sources come from the same bytes, so an unknown anchor would be a bug, and the page number is the
  better guess. The spec's "first page" was for a source that knows nothing; only such a source gives
  no anchor at all, and then too the page is kept, as task 1 had it.
- **Scrolling to the paragraph** goes through `Nav::scroll_to`, a share of the page's height, taken
  by `shown` once the draw knows the page's rows: the pager doesn't learn to start a screen part-way
  down a page, as the spec says, since only a page as wide as the window scrolls.
- **The anchor is taken at `replace`**, not when the job starts, so scrolling while it runs counts.
- **Tests seen failing**, by copying aside, reverting, running, restoring and `cmp`:
  - `a_paragraph_at_a_place_is_found_again_in_another_layout` (`source::wordcraft`): `paragraphs_of`
    keeping nothing.
  - `a_scroll_asked_for_as_a_share_of_the_page_is_taken_once_the_rows_are_known` (`view::pager`):
    `shown` ignoring `scroll_to`.
  - `the_paragraph_at_the_top_of_the_screen_stays_on_the_readers_page_across_a_relayout`
    (`tests/it/view_pager.rs`): `replace` keeping the page number, as task 1 did.

## Done items

- All checked but the manual one, which is the owner's: the gates after each task (each backend alone
  after task 1, which changed `cfg` attributes, and once more on the finished branch); the tests named
  above, each seen failing; the cost at 20 and in the pager, in Limits and ADR-0022; the docs by *Where
  a fact goes*. README's Plans had no bullet for this.
- **Left to the owner**, with `target/debug/clv` built by a plain `cargo build`: in kitty, `[`, `]` and
  `72]` on a long Markdown file; HTML through a pipe (`cat page.html | target/debug/clv --pager
  /dev/stdin`); `]` on a PDF; in a list mixing them, `]` refused, and a Markdown file gone into after
  `]` in another shown at the new width; `--pages 2-3` with `]` refused; `--width 40 README.md`,
  narrow tall pages two to a row, `]` widening them; and the costliest file of *Cost*, `]` three times
  quickly, keys and scrolling working, `width 96…` then `width 96`. The spec stays `Implementing`
  until then.
- **Discoveries:** none beyond ADR-0022's follow-ups (the width kept between runs; a width for other
  backends that lay text out), which are its to hold.

## Codex review fixes

- **The anchor's page came from the last draw.** `place_on_screen` took the first page on screen from
  `self.on_screen`, which the draw fills; a jump (`9g`) read in the pass a layout's result arrives
  had moved `nav.at` but not the draw, so the new level anchored on the page before the jump. Now
  `place` (a function, tested with a `Nav` alone) takes the page from the navigation as it is, and uses
  the last draw's rows for the scroll only when that draw placed this page of this level; otherwise
  the page's top. `a_relayout_anchors_on_the_page_the_reader_is_on_now_not_the_one_last_drawn` failed
  with the page taken from the drawn view again.
