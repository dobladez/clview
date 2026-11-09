# Spec-0005: implementation notes (2026-11-09)

Notes made while implementing [Spec-0005](0005-set-pages-to-a-row-and-margins.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 1: `--per-row`, and `,` `.` as the zoom

- **The alias.** `OptionSpec.alias: Option<&'static str>`, `None` in every literal. `Parsed::parse` takes
  an option by its name, its short name or its alias, and records it under its name; which ones came by
  their alias is kept (`Parsed::aliased`, `Parsed::typed`), so the viewer warns once per option, right
  after parsing and before anything else (even a missing file), and an error names it as typed
  (`Zoom::parse(s, option)`). Name and alias both given is refused in the parser, generically:
  `--per-row and --zoom are one option: give one of them`. `--help` never shows the alias.
- **The keys.** `Nav::per_row_key` takes a typed count (`N.`, `N,`: N clamped to the last draw's most,
  `fully zoomed out` when clamped; `0` back to `auto`), else `Nav::step_per_row`: `,` is `-`, `.` is `+`
  (to the width on a file, stopping at one tile in a list). A list's `+` and `-` go through
  `step_per_row` too, and still take no count, as before.
- **The hints** got a fourth, longest variant each with `, . per row`; the old longest is shown where
  it doesn't fit (`OPENED_KEYS[0]` no longer fits a 100-column window: the unit test checks it at 120,
  and `[1]` at 100). `?` has a `, .  3.` line in a file's pages and in a list.
- **`max_per_row`** was already the one limit (grid and tiles alike); nothing to change in `fit.rs` but
  the option's name in `Zoom::parse`.
- **The status line at `auto`.** The spec said "nothing at `auto`"; today it says what auto drew (`1 per
  row`, `2 per row`) on every file. Asked the owner on 2026-11-09, who decided to keep it as it is
  everywhere: the count `auto` drew, at `auto` too; "nothing at auto" meant the `Room` carries nothing
  there. The spec's Requirements say so now (its Changelog).
- **Tests moved to `--per-row`** (`tests/it/*.rs`); `--zoom` is tested on its own in
  `view::zoom_is_still_taken_for_per_row_and_warns`. README's example too.
- **Seen failing**, each by reverting its fix and copying it back (`cmp` to be sure):
  `args::tests::an_option_is_taken_by_its_alias_and_refused_by_both` (alias matching removed from
  `parse`; and again with the both-given check disabled); `fit::tests::zoom_names` (`--zoom` hard-coded
  in the error again); `pager::tests::comma_and_dot_set_pages_to_a_row_as_the_zoom_does`,
  `pager::tests::a_lists_comma_and_dot_set_tiles_to_a_row_down_to_one` and
  `view_pager::comma_and_dot_set_pages_and_tiles_to_a_row` (the `,` `.` arms removed from `key` and
  `list_key`: "three to a row never happened"); `view::zoom_is_still_taken_for_per_row_and_warns` (the
  warning loop removed: stderr empty; the both-given check disabled: the run succeeded).

## Task 2: columns on Markdown and HTML

- **`Room.per_row`**, `None` at `auto`, compared by `Room::same`; every literal has it. The pager keeps
  its own `room` at `auto` (what `Hold` compares the window with) and adds the reader's count where it
  lays out and asks bounds: `Pager::room_for`, from `Nav::columns` (the count the last draw drew, or
  just set by a key; one at `width`; `None` unless `Nav::reflows`: a document with a relayout setting,
  not in a list, not `--pages`). So a count is laid out for at once, on the next pass, and only a
  change of size waits for the hold.
- **Which setting a count undoes is the backend's to say**: a new field, `Setting::sets_width` (true
  for the documents' `width` alone), since the core may not name `width`. Not in the spec's Approach,
  which listed no new `Setting` field; the alternative, the core recognising the setting by its named
  value or by name, would break "the core names no backend".
- **A width key's "count back to auto first, then step from the bounds at auto"** is `Step::Again`:
  `Nav::setting_key` sets the zoom to `auto` and hands the key back, untouched by its typed number; the
  pager runs `refresh_bounds` and presses it again. `0]` and `0.` change only their own.
- **`.` on a reflowed document** stops where the last draw showed one to a row (`1 per row, the
  least`), from `auto` too; it never goes to the width. `--per-row width` there is one to a row
  (`view()` maps it for printing, `Level::new` for the pager).
- **Options.** `--width N` (a number, from `starts`) with a count: the first room has no count, and once
  the file is known to reflow (shown alone, not `--pages`), the zoom goes to `auto` with
  `warning: --width and --per-row disagree on notes.md: the width wins` (the option as typed, so
  `--zoom` there too). On a PDF both hold, silently. With `--pages`, the document opens with no count:
  "both hold, each for what it is about", the count the zoom alone.
- **Recovery.** `Pager::put_back` puts the count back to the failed layout's `laid_at` room's
  (`Nav::put_back_columns`) with the values; the failed pair's room carries the count, so it is refused
  as any pair is. **A surprise:** the bounds were asked before `relayout` in each pass, so after a
  failure the status line said `width 72 (window)` (the bounds of the three columns) until the next
  pass; `refresh_bounds` now runs after `relayout` too.
- **Seen meanwhile, left as it is:** a count's key changes the grid at once, so until the new layout
  lands the old pages are drawn N to a row, shrunk (and rendered again for that). Holding the grid's
  count until the layout lands would be another change; noted here only.
- **`?`** says columns on a reflowed document (`COLUMNS_HELP`).
- **The ADR** is ADR-0024, linked from ADR-0023 and the architecture's room section. Margins' place in
  the order of the bounds goes in with task 3.
- **Seen failing**, each by reverting its fix and copying it back (`cmp`): `page_cols` and the book rule's
  arm in `page_in` (→ `wordcraft::tests::pages_to_a_row_are_columns_of_the_room`,
  `wordcraft::per_row_lays_markdown_and_html_out_in_columns`,
  `view_pager::printing_lays_markdown_out_in_columns_for_the_ttys_room`,
  `view_pager::text_cells_lay_markdown_out_in_columns_too`,
  `view_tmux::inside_tmux_a_count_to_a_row_lays_markdown_out_in_columns_of_the_pane`,
  `view_pager::a_count_to_a_row_lays_markdown_out_in_columns_and_the_last_key_wins`); `Step::Again` in
  `setting_key` (→ `pager::tests::on_a_reflowed_document_a_count_is_columns_and_the_last_key_wins`, and the
  pager test at `72]`); `width_gives_way` (→ both, "three again never happened"); the `refresh_bounds`
  before the key pressed again (→ the pager test: `[` stepped from a column's bounds); `put_back_columns`
  and, separately, the `refresh_bounds` after `relayout`
  (→ `view_pager::a_count_whose_layout_fails_puts_the_count_and_the_width_back`); `per_row` in
  `Room::same` (→ `source::tests::a_room_is_the_same_within_a_little_of_its_cell`, and the text-cells test:
  no layout); the printed room's count (→ both printing tests); the conflict's warning (→ the printing
  test's stderr).

## Task 3: margins on `{` `}`

- **`MARGIN`**, the setting, replaces `SIDE_MARGIN`: 0 to 20 characters, step 1, default and zero 3,
  `Keys::Step('{', '}')`, `--margin N`, `Relayout`. A margin is `margin × CODE_WIDTH` points (`side`), so
  N characters are N × F/100 cells at font F; top and bottom stay `TOP_MARGIN` (`top_margin`).
- **What a document is laid out at** is now one value, `At { width, font, margin }`, as asked; `laid_at`
  says all three, `relayout` compares all three, and `laid_out` lost its `too_many_arguments` expect.
- **The order of the bounds**, as the spec says: `font_most(room, margin asked)` over the room's share
  (`page_cols`), then `margin_most(room, font laid out at)`, then the page (`window_cols` decides the book
  with the margin laid out at). A consequence the spec's example (`margin 12 (9)`) doesn't show: since the
  font's most is worked out within the margin asked, a margin wider than a narrow page takes lowers the
  font laid out at first, and the margin's most binds only at the least size. Stepping `}` stops where
  the next margin would need a smaller font (`margin 6, the most here` in a 32-column column); a typed
  `12}` lowers the font instead. To make that visible, the status line now says a value laid out past
  its most even at its default (`font 100% (80%)`), which it hid before (`laid_at` in `pager.rs`, a file
  not in this task's list).
- **`min(asked, 1)`** where even no margin leaves 20 characters; where none leaves exactly 20, the most is
  0 and the margin 0 (a 10-column room at 50%).
- **3 for 2 moves numbers**: 80 columns hold 74, 100 hold 94, a book needs 134 columns (two pages of 60).
  The backend's tests, `tests/it/view_pager.rs` (`NARROW`, the steps `94`, `60` at 150%) and
  `tests/it/wordcraft.rs`'s `printed_page` moved with it; the styling tests' pixel margin is `SIDE_PX`
  34, renamed from `MARGIN`, which the setting now names.
- **Seen failing**, each by reverting and copying back (`cmp`): margins ignored in `page_in` (→
  `wordcraft::tests::margins_shorten_the_line_at_window_and_widen_the_page_at_a_width` and three more
  unit tests, `wordcraft::the_margin_shortens_the_line_or_widens_the_page`,
  `view_pager::the_margin_keys_lay_the_document_out_again_and_stop_at_the_most_here`); `margin_most`
  unbounded (→ the bounds test, the margins test, the pager test's `the most here`); `font_most`
  ignoring the margin asked (→ the margins and bounds tests); `min(asked, 1)` as `asked` (→ the margins
  and page tests); the default said past its most (→
  `pager::tests::the_zoom_keys_are_a_documents_font_size_where_it_has_one_and_the_zoom_elsewhere`); the
  margin left out of `laid_at` (→ `wordcraft::tests::markdown_and_html_say_their_width_and_room_and_lay_out_again_at_another`).
