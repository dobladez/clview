# Spec-0015: implementation notes (2026-11-22)

Notes made while implementing [Spec-0015](0015-read-options-and-keys-from-a-config-file.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 1: a config of their own for the tests

- **The helper is `tests/it/bin.rs`, not `tests/it/fixtures.rs`.** The PDF backend's unit tests
  share `fixtures.rs` (`src/source/pdfcraft.rs` includes it by path), and Cargo gives the binary's
  path (`CARGO_BIN_EXE_clv`) to integration tests only. So the helper has a module of its own:
  `clv()` (the command, with `XDG_CONFIG_HOME` set to an empty directory), `own_config()` (the
  same for any command), `script()` and `CLV` (the path, for a shell line).
- **script(1) and tmux.** `script()` was written twice, in `view_pager.rs` and `view_tmux.rs`. It
  is now one function in `bin.rs`, and it sets the variable, so every `clv` in a line that script
  runs has it. A tmux server gets its environment from the command that starts it: a client under
  `script()`, or `Server::run` (`new-session -d`), which sets the variable too.
- **Not in the spec's *Files:*:** `tests/it/main.rs` (the new module) and `tests/it/bin.rs`.
- The Done item "without the helper, a config with `per-row = 3` breaks a test" can only be seen
  once clv reads a config. It is checked in task 5.
- *Verify:* the gates pass; `grep -rn CARGO_BIN_EXE_clv tests/` shows `tests/it/bin.rs` and
  `tests/docs.rs` alone.

## Task 2: the new names

- **A setting's commands are built from the setting's name, and a test holds that name to the
  option's.** `Command::name` writes `cycle-`, `decrease-` or `increase-` and `Setting::name`.
  `source::tests::the_commands_of_each_setting_have_the_name_of_its_option` checks, over the
  build's backends, that each setting is called as its option is (`theme`, `--theme`). Built from
  the option's name instead, the test could not fail.
- **The same test checks the phrases.** A setting's `help` now names a command between braces
  (`72{increase-width} sets 72`), and the pager fills it (`Settings::help`, through
  `Keymap::fill`). The test fails on a phrase that names a command no key is bound to.
- **The font's phrase names the viewer's command**: `150{zoom-in} sets 150%`. A setting on the zoom
  keys has no command of its own.
- **Only `per-row` can now clash with the viewer.** A setting's commands all start with `cycle-`,
  `decrease-` or `increase-`, so the only names of the viewer's that one can take are
  `decrease-per-row` and `increase-per-row`. The unit test of `Keymaps::build` that used settings
  called `quit` and `screen` now uses one called `per-row`.
- **Not in the spec's *Files:*:** `src/view/mod.rs` (its tests name the setting, and it exports
  the two helpers for the test in `source.rs`), `docs/backlog.md` (it named the setting `paper`).
  ADR-0033 and the older specs still say `paper`: they are records, and stay as written.
- **Seen failing:** with Mermaid's setting called `paper` again, the test says `merman: the
  setting paper has the option --theme` and `merman: the command cycle-paper is not named from
  the option --theme`.
- Nothing here is for `CHANGELOG.md` or the manual: a reader sees the names only once a config
  can write them (task 6).
