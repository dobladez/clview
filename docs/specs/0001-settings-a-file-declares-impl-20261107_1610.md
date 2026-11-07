# Spec-0001: implementation notes (2026-11-07)

Notes made while implementing [Spec-0001](0001-settings-a-file-declares.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 1: settings, `d` and `--theme` the first

- **`default` and `auto`.** The spec gives `paper` "default `auto`". A `Setting`'s `default` is a
  `Value`, so `auto` is declared apart (`auto: Some(Auto { light, dark })`): a setting with `auto`
  starts by the background, and `default` is where it starts on a terminal that doesn't say (for
  `paper`, `light`, as `Theme::Auto.papers(None)` was). Its option takes `auto` only when `auto` is
  declared.
- **`Numbers` has no `unit`.** Nothing in this spec says the unit (`width 72`, `--width: expected page
  or 20–200`, from Spec-0002), so the field would have been dead code under `-D warnings`. Spec-0002
  adds it if a message of its needs it.
- **A wrong value's message** is made from the declaration: `auto` if declared, the named values, then
  `least–most`, joined `a, b or c` (`--theme: expected auto, light or dark, got sepia`, as before).
  The parsing is the viewer's (`view::started`), so the value goes through `printable` there.
- **Where `Nav` gets the settings.** Values hold for the whole run while `Nav` is per level, so the
  build's table and the values are one `pager::Settings` behind an `Rc`, the values in a
  `std::cell::Cell` (`Values` is `Copy`: no borrow to fail). Each level's `Nav` has it and `has`, the
  names its source's `settings()` gave when the level was made. A changed value sets `rerender`, as
  `turn_paper` did through the pager; `Pager::turn_paper` and its flag are gone.
- **`?`'s order.** The `d` line is no longer between `r` and `c`: settings come after all the viewer's
  page or list keys, before the way out and `?`. Same in `docs/usage.md`'s tables.
- **`VIEWER_KEYS`** (pager, test-only) lists the viewer's key characters for the check in `source.rs`;
  `the_viewers_keys_are_those_listed` keeps it honest both ways (any other character does nothing to a
  `Nav`, and each listed does something).
- **Not in *Files*:** `src/view/files.rs`. A `Request` now carries `Values` (8 × `(name, Value)`, some
  320 bytes), and clippy's `large_enum_variant` fired on the jobs and answers carrying a `Frame`; the
  frame is boxed there. `tests/it/view.rs` too: `--help` lists `--theme` under documents, and a build
  without `word` refuses `--theme` (`without_documents_there_is_no_theme`).
- **`Request` derives `Default`**, so tests and pdfcraft's results say `..Default::default()` where they
  said `paper: Default::default()`.
- **Tests seen failing**, each by copying the file aside, changing it, running the test, copying it
  back and comparing:
  - `a_settings_keys_are_its_own_in_every_backend`: `paper`'s key made `r`.
  - `a_value_is_held_by_its_settings_name_and_values_hold_so_many`: `Values::set` adding a slot
    rather than replacing.
  - `auto_says_where_a_setting_starts_by_the_background`: a light terminal given `auto.dark`.
  - `a_settings_option_takes_what_it_declares`: the range check dropped.
  - `a_settings_option_is_taken_for_a_file_that_hasnt_the_setting`: `takes_all` without settings'
    options.
  - `a_settings_key_cycles_its_values_where_what_is_shown_has_it`: the `has` check dropped.
  - `the_keys_list_the_settings_after_the_viewers`: settings' lines after the way out.
  - `the_viewers_keys_are_those_listed`: `h` taken out of `VIEWER_KEYS`.
  - `help_and_version` (`tests/it/view.rs`): `--help` without settings' options.
  - `only_markdown_and_html_have_a_dark_form`: wordcraft ignoring the request's dark colour.
  - `without_documents_there_is_no_theme` (`tests/it/view.rs`, a build without `word`): `--theme` put
    back among the viewer's options, run with `--no-default-features --features pdf`.
- **Gates without `word`** first failed on `Keys::Cycle` never being made there (now `expect`ed
  outside `word` and tests), and on four helpers of `tests/it/view_pager.rs` used only by the tests
  of `d` (now gated on `word`, as those tests are).
- **A surprise, not the code's:** the disk filled during the per-feature gates; afterwards `word`
  alone failed to link (undefined `.llvm.` symbols) from incremental output left half-written.
  Removing `target/debug/incremental/clv-*` fixed it.

## Task 2: numbers

- **From a named value**, which the spec leaves open, `Step` goes up to the least and down to the
  most. Spec-0002 replaces this with `page` among the steps at its own width.
- **A count out of the range** sets the nearer end and says so as an end does (`width 200, the
  most`); in it, `width 72`. Nine digits at most are typed (`MAX_TYPED`), so a count fits a `u32`.
- **A step that reaches an end** (198 up to 200) says `width 200`; only a step that can't move says
  `the most`, and changes nothing, so nothing is rendered again.
- **`?`** gives a numbered setting's keys as `[ ]  12]`.
- **No docs for users:** no shipped setting has numbers until Spec-0002. `docs/architecture.md` says
  what `turned` does.
- **Tests seen failing**, the same way as task 1:
  - `step_keys_go_to_the_next_multiple_and_stay_at_the_ends`: up by the step rather than to its next
    multiple; and the arm that stays at the most taken out.
  - `a_count_sets_a_numbered_setting_and_is_used_up`: no count taken; the count kept on a file
    without the setting; the count not clamped.
