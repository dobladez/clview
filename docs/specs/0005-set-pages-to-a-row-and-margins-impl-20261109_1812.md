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
- **The status line at `auto`.** The spec says "nothing at `auto`"; today it says what auto drew (`1 per
  row`, `2 per row`) on every file. Asked the owner on 2026-11-09; kept as it is meanwhile (the count
  drawn, at `auto` too), reading "nothing at auto" as the `Room` carrying nothing there.
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
