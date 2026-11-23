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

## Task 5: options from the config

[ADR-0043](../adr/0043-a-config-in-toml-flat-keyed-by-the-names-of-the-catalogs.md) records the
format, the layout, the names and the bindings with a number.

- **`toml` without serde.** `toml::de::DeTable::parse` gives the place of every key and value
  with the features `std` and `parse` alone. So clv's own `toml` has no serde, and no writer.
  The three tests that read manifests use `toml::Table`, which needs serde: they ask for it in
  `[dev-dependencies]`, the same crate with one more feature. `Cargo.lock` now holds `toml`
  1.2.1 alone; 0.9.12, `toml_datetime` 0.7.5 and `winnow` 0.7.15 went.
- **The order of the file comes from the places, not from a feature.** A `toml` table keeps its
  keys by name. `Config::parse` sorts the entries by where each key is, so clv needs no
  `preserve_order` and no `indexmap`.
- **Values are checked when the config is read**, by `Entry::read`, so the error has its line.
  `Parsed` then holds them as text, as the command line's, and the backends read them the same
  way (`options.value("--font")`). The spec's "Errors from the config carry the file and the
  line" is in `config::Wrong`, not in `Parsed`: `Parsed` only says which options go by the config
  (`by_config`).
- **A width from the config against a count typed.** The backend reads `--width` from the
  options when it opens a file, so "the count wins" means that the file is opened without the
  config's width. `view` does that for a file shown alone (`Opener::alone`,
  `Parsed::without_config`), and starts the setting at its default if the file is a document. A
  list's files, a file gone into from a list and a file with `--pages` are opened with all the
  options: there the count is the tiles' or the zoom's, and the two do not disagree.
- **`--per-row auto` typed disagrees with nothing**, as today: the config's width holds.
- **The file is checked before it is opened, and again after.** To open a pipe that nothing
  writes to waits for ever, so `load` asks `metadata` first. It asks again of the file it opened,
  and reads at most 1 MiB and one byte, because a file can change in between.
- **A message names the file from the reader's home** (`~/.config/clv/config.toml:3: …`), as the
  spec's example has it. A file elsewhere is named by its path.
- **`--config` with `--no-config` is refused**, as `--pager` with `--no-pager` is. The spec does
  not say; to pick one silently would hide a mistake.
- **A config can't set `config` or `no-config`** ("it says which config to read"). The spec's
  list has `pages`, `password` and `zoom`; these two follow from what they are.
- **`width = 0x48` is 72.** An integer is read in its base and given on in base ten. `width =
  72.5`, `true`, an array and a table are refused with what was written, cut at 40 characters.
- **`[keys]` is an unknown option until task 6.**
- **The test of the width is in `tests/it/wordcraft.rs`** (`config_width_…`), not `view.rs`: it
  needs a Markdown file and the helpers that measure its pages. `cargo test --test it config_`
  runs it with the others.
- **`tests/it/bin.rs` gets `config_home()`**, a directory with a config for a test to point
  `XDG_CONFIG_HOME` at.
- **The property test is one test.** A first version had a second property over any text at
  all. With a slicing bug put in, that one still passed: random text is almost never TOML. So it
  went, and the one that is left adds any text to a few of its cases only.
- **Seen failing**, each with its piece of the change taken out:
  - no option taken from the config: `config_sets_an_option_…` (36 columns, not 25)
  - the rule for `pager` and `no-pager`: `config_pager_…` (`give --pager or --no-pager, not both`)
  - the count typed winning over the config's width, and the warning kept for both typed:
    `config_width_…`, on "the count typed" and on "nothing is said"
  - the line of an error: `config_that_is_wrong_…` (`:1:` for `:3:`)
  - the sort by place, the refusal of `pages`, the size, `XDG_CONFIG_HOME` not absolute: the four
    unit tests of `config.rs`
  - a slice by bytes for the one by characters: `no_text_of_a_config_panics`
  - the count of typed options: `an_option_on_the_command_line_wins_…`
- **The helper of task 1, seen failing.** With `per-row = 3` in `.config/clv/config.toml` of a
  home made for it, and the helper setting nothing, three tests of `tests/it/view.rs` fail
  (`an_image_is_one_page_…`, `several_files_are_a_list_…`, `config_sets_an_option_…`). With the
  helper, the 34 pass under the same home.
- **Not in the spec's *Files:*:** `Cargo.lock`, `tests/it/bin.rs`, `tests/it/wordcraft.rs`,
  `CONTRIBUTING.md` (its Tests), `docs/adr/README.md`.

## Task 6: keys from the config

- **The config writes, the keymaps judge.** `config.rs` reads the three tables into
  `commands::Binding`s, text and place only. `Keymaps::bound` reads each key (`keys::named`) and
  finds each command, and its refusals come back with their line. So `config.rs` knows no key
  and no command.
- **The config goes on after the settings' suggestions**, not before them as the spec's
  Approach has it. Applied last, "the same as the default" covers a setting's key too (`D =
  "cycle-theme"` changes nothing), and a key that the config binds takes a suggestion's place
  by the same rule that takes a default's. The result is what the spec's Requirements ask.
  Applied before, the template with its `# ` taken out could not give the keymaps of no config.
- **A general binding where its command does not apply is skipped there, silently.** `[keys] x
  = "search"` binds `x` in a file's pages and leaves `x` in a list as it was. `none` in `[keys]`
  unbinds in both.
- **A key bound to what it is bound to by default keeps its place**, and that includes a key
  bound to nothing: `y = "none"` changes nothing.
- **The keymaps are built in `view`, before any file opens**, and handed to the pager
  (`pager::run`, `Settings::with`). So a binding that can't be made stops clv when it prints,
  too.
- **`pages` and `list` under `[keys]`.** A table there is a keymap's. A string there
  (`pages = "quit"`) is a key called `pages`, and clv reads no key of that name.
- **No message wrote a key by hand.** Every message that names a key of the keymaps already
  went through `Keymap::fill` or `Keymap::first`. The keys of a search, a filter and the links'
  labels being typed (`⏎ search · esc cancel`) are outside the keymaps, and stay.
- **`keys_shown` in `tests/it/view_pager.rs`** lost its `cfg(feature = "word")`: a test on a PDF
  uses it now.
- **Seen failing**, each with its piece taken out:
  - the config's key put last, not first: `a_configs_keys_come_first_…` (`ctrl-c  x  X  D`) and
    `config_keys_bound_again_…` (the reminder says `ctrl-c quit`)
  - the skip of a binding that is the default's: `a_configs_keys_come_first_…` (`/  S`)
  - the refusal of a digit and of `ctrl-c`: `what_a_config_cant_bind_…`, the property test, and
    `config_keys_that_cant_be_bound_…`
  - the old binding left in: the property test (a key bound twice) and `config_keys_ctrl_c_…`
    (`q` still quits)
  - a setting's phrase not filled: `config_keys_show_in_a_settings_line_…`
  - the keys not sorted by place: `the_tables_of_keys_…`
- **Not in the spec's *Files:*:** `src/view/mod.rs`, `src/view/keys.rs` (the `expect` of task 3
  goes), `tests/it/view.rs`, `docs/architecture.md`, `CONTRIBUTING.md`.

## Task 7: `--generate-config` and `--print-config`

- **One function writes both.** `config::written` makes the config in effect from the catalog,
  the options and the keymaps. `--generate-config` calls it with no option and the default
  keymaps, and that is the template.
- **`--generate-config` prints the path alone**, on stdout, so a shell can use it
  (`$EDITOR "$(clv --generate-config)"`). The path is written whole, not from the home.
- **`--print-config` prints the keys in the two tables of the keymaps**, each key where it is
  bound. A key that `[keys]` binds in both keymaps is printed in both. So what it prints is the
  config in effect, in a normal form, not the file again.
- **A key that the config unbinds is printed as `none` under the command that had it.** A key
  bound to another command is printed under its new command only. A command that is left with no
  key gets a line of help (`## No key is bound to cycle-theme.`), so the reader still sees its
  name.
- **A setting that several backends declare has a help line from each**, as `--help` lists it
  under each. Each option also gets a line of what it takes (`## Takes window or 20–200.`),
  which the spec does not ask for: a help line such as "a book's text at N% of its size" does not
  say the range in the form that the config takes.
- **The line of a table is commented out too** (`# [keys.pages]`), and loses its `# ` when a key
  in it is set. "Every line of the template is commented out" holds, and the test that takes
  every `# ` away gets valid TOML.
- **Empty lines** separate the options and the groups of keys. A line of the template is help
  (`## `), commented out (`# `), or empty.
- **The default of an option** is `Entry::default`: a setting's `auto` where it goes by the
  terminal's background, else its default; another option's first named value. So `Shape::Value`
  now says that its first named value is the default.
- **Each of the two options is the whole of a run.** With a file, clv refuses (`--print-config
  takes no file`); with both, it refuses. `--no-config --generate-config` writes the reader's own
  file: `--no-config` only says what to read.
- **Read back, the keymaps are the same command by command.** The printed file groups the keys
  by command, so the order of the bindings among the commands can differ from the first config's,
  which nothing shows. The unit test compares the keys of each command, and checks that the file
  printed again is the same file.
- **The test of the template is in `source.rs`**, as the other tests over the build's backends
  are (`source::tests::the_template_uncommented_is_no_config`), through `view::wrong_template`.
  It also runs on a build with other settings, with a PDF backend alone, and with no backend.
- **Seen failing**, each with its piece taken out:
  - the default taken as the last named value: the template test (`the options differ: (Width,
    Text(Ascii), Small, …`)
  - the line of a table never commented out: the template test and
    `the_config_in_effect_is_written_…`
  - the keys that `?` does not list left out: the template test (`92 keys written, of the 96
    that a config can write`)
  - the file opened without `create_new`: `a_config_is_written_once_…` and `generate_config_…`
  - a line that is set kept commented out: `print_config_…`
  - a `"` not escaped: `a_key_and_a_value_are_written_…`
- **The manual check of the task**, run in a shell with `XDG_CONFIG_HOME` on a new directory:
  the first run printed the path and exited 0; the second said `… is there already: clv never
  writes over a config` and exited 1; `clv --no-config --print-config` gave the file, byte for
  byte. It needs no terminal, and `generate_config_…` checks the same. Left for the owner to do
  once more in their own shell.
- **Not in the spec's *Files:*:** `src/args.rs` (`Entry::default`), `src/view/commands.rs`
  (`groups`, `Keymap::all`), `src/source.rs` (the test), `docs/architecture.md`,
  `CONTRIBUTING.md`.

## The Done checklist

- **The gates** pass on the last commit: `cargo fmt --check`, `cargo clippy --all-targets -j 2
  -- -D warnings`, `cargo test -j 2`.
- **The tests of `tests/it/view.rs` name no setting.** That file runs in a build with PDFs and
  SVG alone (CI's nightly `pdf,svg`), where `theme` and `width` are unknown names. So the
  `config_…` and `print_config_…` tests there use the viewer's own options (`per-row`,
  `compression`, `protocol`), and two cases of `bad_arguments_say_what_is_allowed` that named
  `--theme` and `--width` came out. A setting from the config has its own test with the
  documents, `config_sets_a_setting_as_its_option_does` in `tests/it/wordcraft.rs`: `theme =
  "dark"` prints what `--theme dark` prints, the command line wins, and a bad value has the
  command line's message after the file and the line. Seen failing with the config's options
  ignored (`"theme = \"dark\"\n" is not as its option`).
- **Other feature sets were not built**, as `docs/specs/project.md` says. What was checked by
  reading, for a build with no backend: every new enum variant is constructed in the core
  (`Shape::Setting` in `Catalog::declare`, `Shape::Text` by `--config`), and no new item is used
  by a backend alone. CI's nightly run is the check: run it after the merge (`gh workflow run
  CI`).
- **Manual, `clv --no-config --print-config` against `--generate-config`:** checked in a shell
  (`diff` says the same), and `generate_config_…` checks it on every run.
- **Manual, in kitty and inside tmux:** not done here, and left open. It needs a real terminal:
  with `theme = "dark"` and `[keys] x = "quit"` in the config, open a Markdown file, see dark
  paper, press `?` and see `x` first among the keys that quit, press `x` and see clv quit. The
  tests cover each part on a pseudo-terminal or through a pipe (`config_sets_a_setting_…`,
  `config_keys_bound_again_…`), but none looks at a screen, and none runs the config inside tmux.
- **README's Plans**: the bullet is now "Positions remembered for files opened before". The
  README also has a line for the config, under what you can do. The backlog's "A config file"
  says what is left, with the spec's two open questions and its Out of scope as ideas not planned.
- **The spec stays `Implementing`** until the owner does the manual check in kitty and tmux.

## Review fixes: `--print-config` prints a config that clv takes

A review of the branch found two ways for `--print-config` to print what clv does not read back.

- **It printed before the options were checked.** `--print-config --protocol sixel` printed
  `protocol = "sixel"`, and `--pager --no-pager` printed both as `true`. The checks of a run are
  now one function, `view::asked` (every value read against the catalog, and the two paging
  flags), and `print_config` calls it before it writes. It also reads every other option that a
  config may set, for a backend's option that is no setting, should one come. A run keeps its
  order: the missing file is still said before a bad value.
- **It wrote a number as typed.** `--per-row 03` printed `per-row = 03`, which is no TOML.
  `value_written` now writes what `Entry::read` read: `03`, `+3` and `0003` print `3`. An integer
  of the config in another base was already read in base ten when the config was parsed
  (`per-row = 0x3` prints `per-row = 3`, and `width = 0x10` is refused as 16).
- **`--width 72 --per-row 3 --print-config` prints both, and says nothing.** The warning of a
  run names the document that the two disagree on, and here no document is shown. Read back,
  both are the config's, and the width wins on a document, as it did typed: the same pages.
- **Decided beyond what was asked: the config's width under a count typed is printed as not
  set.** With `width = 40` in the config and `--per-row 2` typed, a run shows a document in two
  columns: the count wins. Printed as `width = 40` and `per-row = 2`, the config read back would
  give the width, since both are then the config's. So `print_config` goes by the same rule as
  the run (`view::widths`, factored out of `view`) and prints `# width = "window"`. One
  difference is left: in a list, a run with that config still lays its documents out at 40, and
  the printed config does not. A config cannot say "this width, but not under a count typed".
- **Seen failing before the fix**, all four: `print_config_refuses_what_a_run_refuses_and_prints_nothing`
  (it printed the template with `protocol = "sixel"`), `print_config_writes_a_number_as_it_was_read_not_as_it_was_typed`
  (`per-row = 03`), `print_config_with_a_settings_option_prints_what_is_in_effect` in
  `tests/it/wordcraft.rs` (`--width 300` printed), and the unit test
  `a_value_is_written_as_it_was_read_not_as_it_was_typed`.
- **Not fixed, measured: a config of many keys takes time that grows as the square.** 35,000
  keys under `[keys]` (530 KB) took 4.8 s in the debug build, and 10,000 took 0.4 s. Two causes.
  `config::line` counts the line breaks from the start of the text for every key: with it made
  constant, the same file took 1.9 s. The rest is `Keymaps::bound`, which for each key searches
  the keys already taken, and removes and inserts in the list of bindings.
- **A test of the Org links failed once, and it is not this change's.** In one full run of the
  gates, `org::links_to_an_org_files_heading_go_to_the_heading_never_a_target_of_its_name` failed
  at `place_now(…).unwrap()` (`tests/it/org.rs:367`), 200 ms after it had seen the other file's
  status line. Alone it passed six times of six, and the next full run passed. The test reads the
  status line after a fixed sleep; nothing here touches the pager. It was not seen in the eleven
  full runs before.
