# Spec-0013: implementation notes (2026-11-15)

Notes made while implementing [Spec-0013](0013-bind-keys-to-named-commands.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 1: Ctrl and Alt keys decoded

As planned, with these readings of what the spec leaves to the code:

- **Which bytes are `Ctrl`.** 1 to 26 but for `C-c` (`Key::Interrupt`), `C-h` (Backspace), `C-i` (Tab,
  no key, as before) and `C-j`, `C-m` (Enter). NUL and the bytes past `C-z` (`C-\`, `C-]`, `C-^`,
  `C-_`) are still dropped: the spec says "Ctrl and a letter".
- **Which characters are `Alt`.** Any printable one after the ESC, past ASCII too (`M-é`), cut between
  reads or not. Not `[` and `O`, which start a sequence and wait for its rest as before, nor `_` and
  `]`: where no answer follows them they are Esc and that key, as they were, so `M-_` and `M-]`
  can't be typed. The tests that said so (`\x1b_j`, `\x1b]j`) stand unchanged.
- **ESC and a key that is no character** (Enter, a Ctrl key, another ESC) are still Esc and that key:
  the spec turns only a printable character into `Alt`. So Alt-Enter, and Alt with a Ctrl key, still
  begin with an Esc.
- **"In one read" is "within 50 ms".** The pager already waits `ESCAPE_WAIT` after a lone ESC before
  it is the Esc key, so an ESC and a character cut between two reads (over SSH) are still `Alt`.
  Nothing changed there.
- **Tests that wrote ESC and a key together.** Twelve pressed `"\x1bq"`, `"\x1bg"` and the like in
  one write, to clear a search or go back and then do something: that is `Alt` and the key now, which
  does nothing, so they hung. They press Backspace in its place (`"\x7fq"`), which clears a search
  kept and goes back as Esc does there; `tests/it/view_tmux.rs`'s `send-keys Escape q` likewise
  (`BSpace`). An Esc pressed alone, and waited for, is unchanged. A thirteenth showed only in the
  full run: `tests/it/tar.rs` pressed Esc alone, but what it waited for after was already on
  screen, so its `q` could be read with the ESC. Backspace there too.
- **Not in *Files:*** `tests/it/view_pager.rs`, `tests/it/gridcraft.rs`, `tests/it/tar.rs` and `tests/it/view_tmux.rs`,
  for those presses; `docs/architecture.md` and `CHANGELOG.md`, for what a reader of the code and a
  user would notice.
- **Seen failing.** With the decoder's two arms taken out (`0x0c` back to `Char('\u{c}')`, an ESC
  and a character back to Esc and the key) and the pager's `Ctrl('l')` back to `Char('\u{c}')`, the
  variants kept so the tests build: `keys::tests::ctrl_and_a_letter_is_a_key`,
  `esc_and_a_printable_character_in_one_read_is_alt` and
  `answers_among_keys_cut_anywhere_are_never_keys` (its Ctrl and Alt parts) failed, and in the pager
  `the_viewers_keys_are_those_listed` and
  `while_a_filter_is_typed_every_key_that_types_a_character_is_text` (Ctrl-L rendered nothing
  again). `while_a_search_is_typed_…` passed either way: there a Ctrl or Alt key did nothing before
  too, and the test only holds it to that. Both files copied back, `cmp` clean.
- **Verify.** `cargo test -j 2 --bin clv keys::`: 17 passed. The gates: `fmt`, clippy clean, 561
  unit tests, 13 docs tests, 276 integration tests.

## Task 2: the catalog and the keymaps, keys as today

- **→ and ← (the owner's answer, 2026-11-15).** The spec's pages table put `n` and → on one command,
  `next`, which with a search kept goes to the next match; but today the arrows still turn pages
  then, and the task is a refactor. Asked, the owner chose to keep the arrows: two more commands,
  `next-page` and `previous-page`, on → and ←, and `next` and `previous` on `n` and `p` alone. The
  spec's table is corrected, and its Changelog says so. The owner agreed to the two readings below
  as well.
- **`12 Home` goes to page 12.** Home is bound to `go-to`, as the table says, and `go-to` takes the
  number. Before, Home went to the first page and left the number typed.
- **While a number is typed, Esc and Backspace are the number's**, outside the keymaps as its
  digits are: Esc drops it, Backspace its last digit. Only "Enter after a number" is a binding
  (`counted`), `go-to` in both keymaps.
- **Bound and not listed.** `=` and `_` are bound to the zoom's commands and left out of `?`
  (`also`), as they were when `?` was written by hand: `+ -`, not `+ -  = _`. So are PageDown and
  PageUp in a list, which its `?` never listed: listed, they made its keys' column 28 characters
  wide where it was 17, and in a window of 100 columns the longest line (`o`'s) lost its end, which
  `open_in_a_list_opens_the_tile_chosen_and_refuses_an_archives` waits for. The width of that
  column is what the new keys of task 3 have to fit.
- **Where the keymaps live.** In the pager's `Settings`, which every level shares, built with it
  (`Settings::of`): the keymaps are made from the settings declared, and hold for the run as they
  do. The spec said "built once a run" and no more.
- **A setting's key must be free in both keymaps**, since its command is bound in both: one that is
  a tile's key in a list (`h`) is the viewer's, though a file's pages have no use for it.
  `VIEWER_KEYS` said the same, one string for both.
- **A setting named as a command is.** A setting called `quit`, or `screen` with two keys
  (`screen-down`), would have a command called as one of the viewer's is. `Keymaps::build` leaves
  it out and says so, with the keys it refuses.
- **`Step::Again` carries the command**, not the key: a width set on a document in columns is run
  again by `Nav::run`, no key pressed twice. Hence `Command: PartialEq + Eq`, a setting's commands
  compared by the setting's name.
- **`Keys::chars` and `Keys::up` are gone** (`src/source.rs`): the keymaps read `Keys` directly, and
  `turned` takes a `Turn` where it took a key.
- **What `?` says changed a little**, where it is now put together: a list's `12 Enter` line also
  says `12g` (true before too), and a file's `u  Backspace` reads `Backspace  u`. The lines the tests held (`+ -  150+`, `[ ]  12]`, `n  N p  3n`,
  the ways out) and the status line's reminders are byte for byte what they were, which
  `the_keys_listed_and_the_reminders_are_read_from_the_keymap` holds them to.
- **What a line of `?` says names keys through the keymap** (`{fewer-per-row}` in its text,
  `Keymap::fill`), and so does `no search: / starts one`. Not a setting's phrase, which its backend
  declares with its keys in it (`72] sets 72`): the spec keeps those phrases as declared. Raised as
  a discovery for the config file (the backlog's entry), where a rebound key would make them wrong.
- **While a filter is typed**, a key that types nothing runs its command only if that moves among
  the tiles or renders again: what `list_key`'s arms for such keys came to.
- **Seen failing.** With `commands.rs` broken six ways at once (the defaults binding a key twice
  and a command where it means nothing; a setting's key bound though taken; a number typed ignored
  in the lookup; `next-page` called `next`; Esc's short name `Esc`) and the pager taking a key bound
  to nothing as `leave`, all five of `commands::tests` failed, with
  `pager::tests::a_key_does_what_it_is_bound_to_and_nothing_otherwise`,
  `source::tests::a_settings_keys_are_its_own_in_every_backend` and a dozen of the pager's older
  tests. Both files copied back, `cmp` clean.
- **Not in *Files:*** `CONTRIBUTING.md` (its Tests names the catalog's), the spec itself (its table),
  `docs/backlog.md` (the discovery above, under A config file).
- **Verify.** `cargo test -j 2`: 566 unit tests, 13 docs tests, 276 integration tests, none of the
  integration tests changed in this task; `fmt` and clippy clean.

## Task 3: the new keys

- **The bindings.** As the spec's tables, in `commands.rs`'s defaults; `u` is taken off `back` in
  both keymaps, so nothing that asks for `back` or `leave` hears it: not the links followed, not a
  list, not the run loop while a folder is gone into. `paper` suggests `D` in the documents' backend
  and in Mermaid's. Nothing else in the pager changed for the keys themselves.
- **The order keys are bound in is the order `?` lists them**, so `scroll-down` is `j`, ↓, `Ctrl-N`,
  Enter (not the table's `j`, ↓, Enter, `Ctrl-N`): `?` sets down and up side by side, and Enter,
  which has no partner, goes last: `j k  ↓ ↑  Ctrl-N Ctrl-P  Enter`.
- **Ctrl and Alt keys in `?`** are `Ctrl-V` and `Alt-v`, as `Ctrl-L` and `Ctrl-C` already were, not
  the spec's `C-v` and `M-v`; the manual writes `Alt-V`.
- **`?`'s column of keys** (not in the spec). A screen down has four keys now (`space  PageDown  d
  Ctrl-V`, 26 characters; a list's screens line, 48), and `?` set every line's text after the
  longest keys: every line would have lost 10 to 30 columns of what it says, which is cut at the
  window's edge. The column is now as wide as the longest keys of 17 characters or fewer
  (`KEYS_COLUMN`, what the widest line was before), and a line with more has its text further right
  by itself. With that, a list's PageDown and PageUp are listed again, where task 2 left them out
  for room.
- **While a filter is typed**, Emacs's keys move among the tiles as the arrows do (they type
  nothing), and `d` and `u` are letters of the filter. The manual's Filtering a list says so.
- **Tests that pressed `d` or `u`.** 15 presses of `d` for the paper are `D` (`view_pager.rs`,
  `wordcraft.rs`, `view_tmux.rs`), 22 of `u` to go back are Backspace (`view_pager.rs`, `tar.rs`,
  `wordcraft.rs`), and the unit tests' own setting is on `D`. Their names still say `d` (`…_and_d_turns_it`):
  a test's name has no capitals.
- **The unreleased changelog entries** that named `u` and `d` (links, Mermaid's dark theme) say
  `Backspace` and `D`: they describe what will ship. "Keys changed" is for the keys 0.4.0 had.
- **ADR-0036**, with ADR-0021's and ADR-0015's status lines saying what it changes of them. Its
  Evidence says plainly that no terminal was measured: it rests on the tests and on the manual
  check still to do.
- **Seen failing.** With the defaults put back as task 2 left them, `paper` back on `d` and
  `KEYS_COLUMN` out of the way (1700): in the unit tests
  `commands::tests::a_key_is_its_command_and_with_a_number_the_one_bound_with_one`,
  `the_keys_listed_and_the_reminders_are_read_from_the_keymap`,
  `pager::tests::d_u_and_emacss_keys_do_what_the_keys_they_stand_beside_do`,
  `a_command_with_many_keys_doesnt_narrow_every_line_of_the_keys` and four older ones that hold `u`
  to going back nowhere; in `tests/it`, `view_pager::d_u_and_emacss_keys_page_and_move_and_u_goes_back_nowhere`
  and the twelve that press `D` for the paper. All four files copied back, `cmp` clean.
- **Not in *Files:*** `src/source.rs`, `src/view/mod.rs` and `src/view/relayout.rs` (their tests'
  own `paper` on `D`), `CONTRIBUTING.md` (its Tests), ADR-0015 and ADR-0021 (their status lines).
- **Manual check: not done.** The task's *Verify:* asks for kitty: `d`, `u`, `Ctrl-V`, `Alt-V`
  (with `macos_option_as_alt`), `D` on Markdown, `Ctrl-F` and `Ctrl-N` in a list, Backspace leaving
  a file where `u` doesn't. It is the owner's to confirm, so the task's box stays open.

## The Done checklist (2026-11-16)

- **The gates** pass on the last commit: `cargo fmt --check`, `cargo clippy --all-targets -j 2 -- -D
  warnings`, `cargo test -j 2` (568 unit tests, 13 docs tests, 277 integration tests).
- **Tests added, each seen failing** (how, under each task above):
  `keys::tests::ctrl_and_a_letter_is_a_key`, `esc_and_a_printable_character_in_one_read_is_alt`, and
  the Ctrl and Alt parts of `answers_among_keys_cut_anywhere_are_never_keys`;
  `commands::tests::every_command_has_a_name_of_its_own_in_kebab_case`,
  `the_default_keymaps_bind_each_key_once_and_every_command_where_it_applies`,
  `a_key_is_its_command_and_with_a_number_the_one_bound_with_one`,
  `a_settings_keys_are_suggestions_the_keymaps_take_after_the_viewers`,
  `the_keys_listed_and_the_reminders_are_read_from_the_keymap`;
  `pager::tests::a_key_does_what_it_is_bound_to_and_nothing_otherwise` (in place of
  `the_viewers_keys_are_those_listed`), `d_u_and_emacss_keys_do_what_the_keys_they_stand_beside_do`,
  `a_command_with_many_keys_doesnt_narrow_every_line_of_the_keys`;
  `source::tests::a_settings_keys_are_its_own_in_every_backend`, rewritten on the keymaps; and in
  `tests/it/view_pager.rs`, `d_u_and_emacss_keys_page_and_move_and_u_goes_back_nowhere`.
- **The docs say what shipped:** `CHANGELOG.md` (Keys changed; Emacs's keys; Alt), `docs/usage.md`
  (both tables of Keys, and every place that named `d` or `u`), `docs/architecture.md` (Commands and
  keymaps, the viewer's files, Settings, the decoder), ADR-0036 in the index with ADR-0015's and
  ADR-0021's status lines, README's `D` and Backspace. No bullet of README's Plans was this spec's.
- **Open: the manual checks**, task 3's and the Done list's. In kitty, and again inside tmux, with
  `macos_option_as_alt yes` for the Alt key: on a PDF, `d`, `u`, `Ctrl-V` and `Alt-V` go a screen
  down and up; on a Markdown file `D` turns the paper and `d` pages; in a list `Ctrl-F` and `Ctrl-N`
  go to the next tile and the one below; Backspace leaves a file and `u` doesn't; `?` lists the keys,
  the new ones among them. Until the owner has seen them, task 3's box and that item stay open and
  the spec stays `Implementing`.
