# Spec-0003: implementation notes (2026-11-09)

Notes made while implementing [Spec-0003](0003-lay-markdown-and-html-out-to-the-window.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 1: pages the window's shape

- **The page's points.** A column is `CODE_WIDTH` (5.64 pt) and a row 5.64 × the cell's shape
  (`cell_points`), so at 100% a character of code is a cell when the page is as tall as the room, and
  the page's points are the room's cells: 80 × 23 cells of 10 × 20 px make 451.2 × 259.44 pt. The cell's
  shape is taken within 0.25 to 8 (a room is input), and a side is capped at `MAX_PAGE`, 14,400 pt.
- **The least a page holds**, the 🟡 question: 20 columns and 10 lines of body text (`LINE`, 11 pt at
  1.15), with the margins counted as two columns at each side and a row above and below. A page too
  small for them loses a column of each side margin first ("to a cell at least"), then grows past the
  room. Not measured on a real terminal yet: the owner's manual check.
- **`Backend::open` on the documents' backend** needs a room, so it lays out for one of the backend's
  own, `ROOMLESS` (100 × 40 cells of 10 × 20 px): only tests reach it, since the viewer opens through
  `open_at`. The backend's styling tests were rewritten to that page (564 pt wide, a 11.28 pt margin:
  `PAGE_WIDTH` 1128 and `MARGIN` 23 at scale 2), and `eighty_columns_of_code_fit_on_a_line` now says
  so at `--width 83`, what a Letter page held.
- **`stepped` takes one `at`** for the setting's named values (all of them sit there, in order), from
  `Bounds::named_at`, instead of each `Named`'s own: `width` has one named value, and a document says
  where it sits. `turned` takes the bounds too: the most here is the room's where that is under the
  setting's own, and `]` says `the most here` from it; a number past it is kept and said with what it is
  laid out at (`said_in`: `width 120 (window)`), and `]` from it says `…, the most here` rather than
  moving it; `[` from it goes to the last step under it, `window` where that sits at the most. The
  status line's slot is padded to that widest form (`width 200 (window)…`), 19 characters.
- **The first room is computed before the output is known.** `first_rooms` guesses the output's kind
  from the protocol asked for and whether this is inside tmux (text cells by name sample; tmux's
  placeholders bound the columns), since only `Output::fitting` and `Output::limits` matter to the
  cells. A `--protocol auto` that turns out to be text cells makes another room, which task 2's change
  of size lays out again. `tmux::client()` is run once in `view` and handed to `tmux_terminal` and
  `text_terminal`, rather than run again.
- **The relayout compares the room already in this task**, since `laid_at` and `relayout` take it: a
  document gone into from a list, laid out for the preview's room, is laid out again for the pager's as
  it shows (the status line says `width window…` meanwhile; task 2 drops the default from it). The
  pager's room is the one the files were opened for, fixed for the run until task 2 follows the window.
- **Printing lays out for the tty's room, as the spec says**, which makes a test's printed pages
  depend on the terminal the tests run from: from kitty, that window; under CI, with no controlling
  terminal, the preview's. `tests/it/wordcraft.rs`'s `printed_page` computes the expected page as
  `clv` does, from `/dev/tty` through rustix (already a dependency), so the shape tests hold in both.
  The spec's "printing with no terminal on the preview's page" is that test's no-terminal case, and
  "a list's document previewed portrait" is in `in_a_list_markdown_and_html_are_tiles` there rather than
  in `tests/it/view.rs`, which has no Markdown. An owner may prefer printing to a pipe at the preview's
  room instead; the spec is explicit, so it wasn't changed.
- **Not in *Files*:** `src/source/joined.rs` (`largest_scale` passed through to a tile's part),
  `src/view/clip.rs` (the new `Size` field, ignored: a copy is at twice the unit as before),
  `docs/adr/0023-…` and `docs/adr/README.md`, `CHANGELOG.md`, `docs/usage.md`, `docs/architecture.md`.
- **Tests seen failing**, each by copying the files aside, reverting, running, restoring and `cmp`
  (the cell compared exactly; the largest scale ignored in `fit::layout`; `page_in` laying out for a
  fixed room, `bounds` `None`, `largest_scale` `None` and `relayout` ignoring the room; `bounds_of`
  finding nothing; `relayout::wanted` and `ended` ignoring the room; `first_rooms` always the preview):
  - `a_room_is_the_same_within_a_little_of_its_cell` (`source`);
  - `a_page_laid_out_for_a_room_is_never_shown_larger_than_its_font_size` (`view::fit`);
  - `the_page_is_the_room_and_its_width_so_many_characters_of_the_code_face`,
    `the_bounds_of_the_width_are_the_rooms_lines`,
    `markdown_and_html_say_their_width_and_room_and_lay_out_again_at_another` and
    `a_table_is_as_wide_as_the_text_at_any_width` (`source::wordcraft`; the styling tests failed too, as
    the revert changed their page);
  - `the_room_bounds_the_steps_and_a_number_past_its_most_is_kept`,
    `the_status_line_shows_what_the_document_is_laid_out_at`,
    `step_keys_go_to_the_next_multiple_and_stay_at_the_ends` and
    `a_relayout_key_is_refused_in_a_list_and_with_pages_and_asks_for_no_redraw` (`view::pager`);
  - `a_layout_is_wanted_when_a_relayout_setting_or_the_room_differs_and_a_redraw_one_doesnt_count` and
    `a_result_is_taken_for_the_values_and_room_still_wanted_and_dropped_otherwise` (`view::relayout`);
  - `the_first_room_is_the_window_the_tty_says_and_a_lists_is_the_preview` (`view`);
  - `the_width_lays_markdown_and_html_out_to_so_many_characters`,
    `markdown_and_html_are_laid_out_into_pages` and `in_a_list_markdown_and_html_are_tiles`
    (`tests/it/wordcraft.rs`);
  - `the_width_keys_lay_the_document_out_again_and_the_status_line_says_what_it_is_at` and
    `a_document_gone_into_after_a_change_is_laid_out_at_the_new_width_and_its_tile_is_not`
    (`tests/it/view_pager.rs`).
- **The manual check** (in kitty, `target/debug/clv --pager README.md`: one page fills the screen, or
  two in a wide window; `40[`: narrow pages, as tall; `0]` back; Ctrl-C while a large file opens
  leaves the terminal as it was) is left to the owner, as asked.
- **Cost at 100%, printed** (the table is in ADR-0023's Evidence): the same as ADR-0022's at either
  width, 14.4 s and 5.8 GB for the costliest file at 80 × 24, width 20, within what Limits says. The
  files were made again by a script in the scratchpad (499,900 marks each), measured on the release
  build under `/usr/bin/time -l` on an 80 × 24 pseudo-terminal from script(1).

## Task 2: a change of size

- **The hold is a small struct with no IO**, `relayout::Hold`: the room the window has had since it
  changed, adopted once it has held `HOLD` (500 ms); the pager's `watch_room` feeds it each pass from
  `tty.size()` through the same `area` the draw uses, and the loop wakes when the hold is due. A room
  a little off the one pending (`Room::same`) is the same room, still holding; one dragged back to the
  document's ends the wait.
- **The failed pair lives on `Relayout`**, which the pager keeps and forgets on `enter`, `back` and
  `replace`, so it is the level's as the spec asks without a field on `Level`; `r` clears it through
  `nav.rerender`, read in `relayout()` before the draw consumes it.
- **`describe` returns an empty string** when nothing but the default is laid out, and the pager says
  `laying out` in its place (`LAYING_OUT`), `…` after it as for any value.
- **The second layout on a terminal without pixels** needs nothing of its own: the pager's room on the
  first pass, with the cell asked, differs from the opening room, and the hold adopts it 500 ms in.
  `a_terminal_without_pixels_gets_a_second_layout_at_the_cell_it_says` answers 8 × 20 to the question
  (a taller page than the 10 × 20 assumed), since a cell of the same shape would make the same page.
- **Inside tmux the test resizes the window**, after `window-size manual` (a window follows its client
  otherwise); the pane is the whole window there, as the test server has no status line, so the test
  takes the pane's size from tmux rather than assuming it. tmux hands its client what was passed through
  already unwrapped, and writes its own cursor moves between an image's chunks, so the test gathers an
  image's chunks wherever they come (`view_tmux::sizes_sent`).
- **Not in *Files*:** `docs/adr/0023-…` (the hold and the failed pair), `CHANGELOG.md`, `docs/usage.md`,
  `docs/architecture.md`.
- **Tests seen failing**, by copying `relayout.rs` aside, reverting (the hold never adopting a room,
  `refused` never true, `describe` saying the defaults), running, restoring and `cmp`:
  `a_changed_room_is_laid_out_for_once_it_has_held_still`,
  `a_pair_that_failed_is_not_asked_for_again_until_either_changes_or_r` and
  `a_layout_is_wanted_when_a_relayout_setting_or_the_room_differs_and_a_redraw_one_doesnt_count` (the
  `describe` assertion) in `view::relayout`;
  `a_change_of_size_lays_the_document_out_again_once_the_window_has_held` and
  `a_terminal_without_pixels_gets_a_second_layout_at_the_cell_it_says` in `tests/it/view_pager.rs`;
  `inside_tmux_a_pane_resized_lays_the_document_out_again` in `tests/it/view_tmux.rs`.
- **The manual check** (in kitty, resize the window: one layout once it settles, `laying out…`
  meanwhile) is left to the owner.
- **The pager's peak** (ADR-0023's Evidence): the costliest file at 80 × 24, `--width 20`, resized to
  100 × 30 once it shows: 41,659 pages laid out again as 31,244, 9.2 GB at the peak, the two layouts
  held together as for `]` (9.5 GB in ADR-0022). Two things went wrong measuring it, worth knowing:
  the shell these notes were written from runs inside tmux, so the first harness let `clv` inherit
  `TMUX` and take the pseudo-terminal for a hidden pane, sending nothing (CONTRIBUTING's warning;
  `env -u TMUX -u TMUX_PANE` fixed it); and a release build started while the "seen failing" revert
  was in `relayout.rs` compiled the reverted hold, which never adopted a room and spun the loop
  (seconds of system time). Rebuilt from the restored file, the numbers above.

## Task 3: the font size

- **The font scales the room's cell, not the margins.** At `font` percent a cell of the room is
  `CODE_WIDTH` × 100/`font` points (`cell_points`), so the page is smaller in points and its text larger
  on screen, while a character of code, the margins (two columns, a row: `margins`) and the least a
  page holds stay at the text's size. So `columns_of` is the text width over `CODE_WIDTH`, whatever
  the room.
- **The font's most in a room** is the largest of its steps at which the room holds 20 × 10 of text
  without the page growing (`fits`, `font_most`): 170% at 80 × 24, 370% at 200 × 51, the least (50%)
  in a room that holds none. An 80-column window at 50% holds two pages of 74 characters, so `window`
  is a book there, which a test first got wrong.
- **`Keys::up`** says which way a key steps, for `Step` and `Zoom` alike, so `turned` has one arm for
  both. `Nav::zoom_setting` is the one check (declared with `Keys::Zoom`, the document has it, not a
  list, not `--pages`); `key`'s `+`/`-` arms ask it first, `help_lines` puts the setting's line where the
  zoom's was and lists it nowhere else, and the status line's first reminder says `+ - font` from it.
  `wrong_keys` lets `Keys::Zoom` through alone; the same keys as a `Step` are still refused.
- **Not in *Files*:** `src/view/relayout.rs` (its test's `Numbers` literal), `docs/adr/0021-…` and
  `docs/adr/README.md` (ADR-0021's order set aside for these keys), `docs/adr/0023-…`, `CHANGELOG.md`,
  `docs/usage.md`, `docs/architecture.md`, `tests/it/wordcraft.rs` (`--font` printed and refused).
- **A flaky test, not this spec's:** `source::later::tests::a_job_runs_first_and_its_panic_is_caught`
  fails now and then at "the job went before the files waiting" (the four 60 ms opens finish together,
  and a fifth may begin before the job's result is polled); it fails on `main`'s own test binary too,
  one run in five here. Left as it is: a discovery for the owner.
- **Tests seen failing**, by copying `pager.rs`, `wordcraft.rs`, `source.rs` and `mod.rs` aside,
  reverting (`zoom_setting` never a setting's, the unit unsaid, the page ignoring the font, `font_most`
  the setting's own, the keys test refusing `Keys::Zoom`, the option's message without the unit),
  running, restoring and `cmp`: see the run recorded below.

## Codex review fixes

- **A font size past the room's most laid out at the most.** `page_in` laid out at the size asked and
  grew the page to 20 × 10 of text, so at 300% on 80 × 24 the page held 23 characters a line where the
  room's most, 170%, holds 43, while the status line said `(170%)`. Now the layout's font is the
  lesser of the one asked and `font_most(room)`, and `laid_at` keeps the value asked; the page grows
  only where even the least size holds no page. `a_font_size_shrinks_the_page_in_points_so_its_text_is_larger_on_screen`
  failed first with a page at 300% unlike the one at 170%.
- **`window` set past the setting's most.** At 50% in a 302-column room `window` sits at 296, past
  200, and `[` from it gave 296, a number the setting doesn't take, so the backend clamped it and the
  pager said `it can't be laid out again`. `stepped` now steps down from a named value only to a number
  within the range (200), and up from 200 to `window`, which says `the most`.
  `the_room_bounds_the_steps_and_a_number_past_its_most_is_kept` failed first with `width 296`.
- **Bounds asked again when the values change.** `Nav::bounds` were asked when the level was made,
  for the values then; the width's depend on the font size, so `150+[` typed together stepped from
  `window` at 96 (100%) instead of at 62 (150%), before the layout at 150% landed. `Pager::refresh_bounds`
  asks the source again after each key and each pass, whenever the level, the values or the room
  differ from what they were asked for (`bounds_for`), and refreshes the status line's `laid` with
  them. `a_width_key_after_a_font_key_steps_by_the_bounds_at_the_new_size` (`tests/it/view_pager.rs`)
  failed first with `width 96`; the right answer is 62, the most the window holds at 150% and a step
  of its own, which `window` sits after.
- **The preview's shape from the terminal's cell.** `in_a_list_markdown_and_html_are_tiles` expected
  a 2 : 1 cell, but the preview room takes the cell the run started with, the terminal's where it
  reports pixels; the test now takes it from `/dev/tty` as `printed_page` does (`tty_window`,
  `preview_page`), so a terminal with 9 × 23 cells accepts its previews. The other tests that print
  documents already went through `printed_page`; the pager and tmux tests run on pseudo-terminals
  without pixels.

