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
