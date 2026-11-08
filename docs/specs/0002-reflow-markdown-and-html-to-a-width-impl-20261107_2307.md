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
