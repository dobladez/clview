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
