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

## Task 3: one spelling for a key

- **Two functions, one table.** `keys::name` writes a key and `keys::named` reads one. `named`
  finds a key with a name of its own (`enter`, `pagedown`) by calling `name`, so the two cannot
  differ. The cost: the property test alone cannot see a named key spelled wrong, because the
  round trip still holds. The unit test `a_key_has_one_name_in_lower_case_which_reads_back_as_the_key`
  pins each spelling.
- **More refusals than the spec lists**, each a key that the decoder never gives: Alt with `[`,
  `O`, `\`, `_` or `]` (after an ESC, each starts a sequence of the terminal's own), Ctrl with
  anything but a letter, and a control character. A name that is right but for its case says how
  to write it: `Enter` says `write enter`, and `Alt-V` says `write alt-V`.
- **`ctrl-c` reads as a key** (`Key::Interrupt`), so that `?` can write it. Task 6 refuses to
  bind it.
- **`named` has no caller yet**, outside the tests: it carries an `expect(dead_code)` that task 6
  takes out.
- **The text of three lines of `?`** said `⏎` and `Esc` for keys outside the keymaps (a search or
  a filter being typed, the links' labels). They now say `enter` and `esc`, as the keys column
  does. The reminders of the status line keep `⏎`.
- **The manual.** Every backticked key in `docs/usage.md` takes the spelling, not the two tables
  alone (86 places): one word for one thing. A key named in running text without backticks
  (Enter opens it) is English, and stays.
- **Seen failing:** with `Key::Ctrl` written `Ctrl-D` again, `the_name_of_every_key_reads_back_as_that_key`
  fails on `"Ctrl-D"`. With `pagedown` written `PageDown` again, the two unit tests fail.
- **Not in the spec's *Files:*:** `src/view/pager.rs` (its tests' expected text) and
  `docs/architecture.md`.

## Task 4: the options catalog

- **`OptionSpec` stays what a backend declares, and the catalog adds who declares it.**
  `OptionSpec` gets `shape` (`Shape`) and `config` (`Settable`). `Catalog` (in `args.rs`) holds an
  `Entry` for each: the `OptionSpec` and `By` (the viewer, a backend, or a setting). The catalog
  is made at run time (`view::catalog`), because the viewer names no backend.
- **A setting's option has the shape `Shape::Setting`.** The spec says that the catalog reads
  `Setting::named`, `numbers` and `auto`. A `Shape` with the named values written again would be
  a second declaration to keep alike. So `Shape::Setting` means "what the setting takes", and
  `Entry::named` and `Entry::numbers` read the setting. `Catalog::declare` sets that shape
  whatever the backend wrote, so the variant is constructed in the core in every build, and needs
  no `cfg` for a build without backends.
- **`args.rs` names `source::Numbers` and `source::Setting`.** `args.rs` is core and `source.rs`
  is the contract, so the layers hold. The range of a number is the setting's own type.
- **`OptionSpec::value` stays**, as `--help` writes the value (`auto|width|1-8`, `N`, `PW`). It
  could be made from the shape, but then `--help` would change (`1-8` and `N` are both a range).
  `source::tests::every_option_says_what_its_value_is` checks that a flag has none and the others
  have one.
- **The value read is `Arg`, not `Given`**: `view/mod.rs` has a `Given` already (the files given),
  and `source.rs` a `Got`.
- **One form of message.** `--per-row "9": use auto, width or a number of pages per row (1–8)`
  is now `--per-row: expected auto, width or 1–8, got 9`, as a setting's was. `Zoom::parse`,
  `Protocol::parse` and `Compression::parse` are gone: each is made from an `Arg`.
- **`--help` is the same text**, compared with a build of `main`: only the version line differs.
- **`config` has no reader yet** outside the tests. It carries an `expect(dead_code)` that task 5
  takes out.
- **Seen failing:** without the range check, `one_parser_reads_a_value_by_the_shape_of_its_option`
  reads `9` as a number. Without the shape set in `Catalog::declare`, the catalog test reads a
  setting declared as free text as text. With the PDF backend's `--password` declared a flag,
  `every_option_says_what_its_value_is` says `--password: the value --help writes and its shape
  disagree`.
- **Not in the spec's *Files:*:** `src/view/fit.rs` (`Zoom::parse` and its test go),
  `src/view/pager.rs` (a test's `OptionSpec`), `docs/architecture.md`, `CHANGELOG.md`.
