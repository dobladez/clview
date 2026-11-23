# Spec-0015: Read options and keys from a config file

- **Status:** Implementing
- **Date:** 2026-11-21
- **Related:** [ADR-0021](../adr/0021-a-backend-declares-its-settings-and-their-options-hold-for-any-file.md)
  (a backend declares its settings); [ADR-0033](../adr/0033-backends-share-a-setting-by-declaring-its-name.md)
  (backends share a setting by its name); [ADR-0036](../adr/0036-keys-are-bound-to-named-commands.md)
  (keys bound to named commands); [Spec-0013](0013-bind-keys-to-named-commands.md), which this
  builds on; [Options](../usage.md#options) and [Keys](../usage.md#keys) in the manual;
  [Settings](../architecture.md#settings) and
  [Commands and keymaps](../architecture.md#commands-and-keymaps-commandsrs-pagerrs);
  the backlog's [A config file](../backlog.md#a-config-file) and
  [`[open-with]`](../backlog.md#open-with-a-program-of-the-readers-choosing-for-each-format),
  which this spec first held. It delivers part of README's Plans
  bullet "A config file: customization, settings and positions remembered". At `Done`, that bullet
  becomes "Positions remembered for files opened before".

## Summary

clv takes everything from its command line. A reader who always wants `--per-row width`, dark
paper or `--compression small` must type it each time. A reader cannot change a key. This spec
gives clv a config file.
clv reads it at startup: `$XDG_CONFIG_HOME/clv/config.toml`, or `~/.config/clv/config.toml`, on
macOS too.

First, the options become a catalog, as the commands are since Spec-0013. The catalog lists each
option once, with its name, the shape of its value, who declares it, and whether the config may
set it. One parser reads a value from the command line and from the config.

The config is flat, and the names of the two catalogs are its keys:

```toml
per-row = "width"
theme = "dark"
width = 72

[keys]
ctrl-d = "screen-down"
x = "none"
"/" = "search"

[keys.list]
J = "list-tile-down"
```

`clv --generate-config` writes a template with every setting commented out, and `clv --print-config`
prints the config in effect. Before the names become a contract, some commands get new names:
`paper` becomes `cycle-theme`, `width-up` becomes `increase-width`, and the list's commands start
with `list-`.

## Requirements

### The options catalog

- clv lists every option once: the viewer's own (`OPTIONS`), each backend's own
  (`Backend::options`), and each setting's (`Setting::option`). Each entry has these:
  - its name, its short name and its alias
  - its help line
  - who declares it: the viewer, a backend, or a setting
  - the shape of its value: a flag, one of some named values, a number in a range, a named value
    or a number (`--per-row auto|width|1-8`), a page list, or free text
  - whether a config may set it
- One parser reads the value of an option, from the command line or from the config. For the same
  bad value, the two give the same message.
- A config may set `per-row`, `compression`, `protocol`, `recurse`, `pager`, `no-pager`, and the
  option of every setting (`theme`, `width`, `font`, `margin`, `fences`). A config may not set
  these:
  - `pages`, which belongs to one run
  - `password`, which is a secret and does not belong in a plain file
  - `zoom`, the old name of `per-row`: the message says to use `per-row`
- `--help` shows what it shows today, made from the catalog, and the four options that this spec
  adds.

### The file

- clv reads `$XDG_CONFIG_HOME/clv/config.toml` when that variable holds an absolute path.
  Otherwise clv reads `$HOME/.config/clv/config.toml`, on macOS too. When the file does not exist,
  clv says nothing and uses its defaults.
- `--config FILE` reads another file, and a missing one is an error. `--no-config` reads no file.
  `--help` lists both, and says which file clv reads when neither is given.
- clv refuses a file larger than 1 MiB, a file that is not a regular file, and a file that it
  cannot read. `clv --worker` never reads a config.
- clv reads the config when it prints, too: `per-row` and `protocol` change what it prints.

### Options in the config

- An option goes at the top level, under its name without the dashes: `width = 72`,
  `theme = "dark"`, `recurse = true`. A number is a TOML integer or a string. A named value is a
  string. A flag is a boolean.
- An option on the command line wins over the same option in the config. `pager` and `no-pager`
  are one option for this, so `--no-pager` wins over `pager = true`. clv refuses a config that sets
  both to `true`.
- The command line cannot turn off a flag that the config sets and that has no opposite:
  `recurse = true` holds until `--no-config`.
- `width` and `per-row` both say how wide a page is. On a document that clv lays out for its
  room, the two disagree ([A page the window's size](../usage.md#a-page-the-windows-size)). Then
  these rules hold:
  - Both on the command line: the width wins, with a warning, as today.
  - One on the command line and the other in the config: the one on the command line wins. clv
    gives no warning.
  - Both in the config: the width wins on such a document, and `per-row` holds for the other
    files. clv gives no warning: a config may set the width for documents and `per-row` for the
    other files.

### Keys

- `[keys]` binds a key to a command by its name, in every keymap where the command applies.
  `[keys.pages]` and `[keys.list]` bind a key in one keymap, and win over `[keys]` there. A value
  is the name of a command, a setting's command too (`cycle-theme`, `increase-width`), or `"none"`,
  which unbinds the key.
- A key that the config binds loses its default binding. The command keeps its other keys. Where
  the config binds a key that a setting suggests for its command, the setting's suggestion is left
  out.
- A key that the config binds comes first among the keys of its command, so `?` and the reminders
  show it. Two such keys of one command keep the order that the file has them in.
- A binding that is the same as the default changes nothing: the key keeps its place. `=` and
  `_` are bound to the zoom by default, and `?` does not list them. They stay unlisted while the
  config binds them as the defaults do. `?` lists every other key that the config binds.
- A key bound with a number typed before it stays outside the config, as the digits do. `12` and
  Enter go to page 12, whatever the config binds Enter to: `enter = "next"` changes Enter alone.
- clv refuses these, and says why:
  - a command that does not apply in the keymap of its subsection (`[keys.list] s = "search"`)
  - a command that does not exist
  - a digit, which starts the number typed before a key
  - `ctrl-c`, which always quits, so that a config cannot leave the reader without a way out
- Keys are written in lower case, and the case of a character counts: `j` is not `J`, and `alt-v`
  is not `alt-V`. A Ctrl key is lower case: clv refuses `ctrl-D` and says to write `ctrl-d`,
  because a terminal sends the same byte for both. clv also refuses `ctrl-m`, `ctrl-j` and
  `ctrl-h`, and names the key to write for each (`enter`, `enter`, `backspace`). clv refuses
  `ctrl-i` and `tab` too: a terminal sends the same byte for both, and clv does not read Tab as a
  key. The names of keys are `space`, `enter`, `esc`, `backspace`, `up`, `down`, `left`, `right`,
  `home`, `end`, `pageup` and `pagedown`. A key with letters, digits, `-` and `_` only needs no
  quotes in TOML. A punctuation key does: `"/"`, `"?"`, `"["`.
- `?`, the reminders of the status line, and every message that names a key show the keys as
  bound. So does the phrase of a setting in `?` (`72] sets 72, 0] the window's`).
- `?` and the tables of the manual write keys as the config does: `ctrl-v`, `alt-v`, `enter`,
  `pagedown`. `?` keeps `↑ ↓ ← →` for the arrows.

### Names

The names of options and commands become a contract with the config. These rules hold for every
command, the commands of a setting too:

- A command that steps an option is `increase-` or `decrease-` and the name of the option. A
  command that goes to the next named value of an option is `cycle-` and the name of the option.
- A command for the list alone is `list-` and what it does. The commands for the pages, and those
  for both, have no prefix.
- No name has the prefix of a format or a backend. A name that two backends declare is one setting
  ([ADR-0033](../adr/0033-backends-share-a-setting-by-declaring-its-name.md)). A prefix would stop
  that later: `image-rotate` would be wrong on a PDF.

So these names change:

| Now | Becomes |
|---|---|
| the setting `paper` | `theme`, in the four backends that declare it |
| `paper` | `cycle-theme` |
| `fences` | `cycle-fences` |
| `width-down`, `width-up` | `decrease-width`, `increase-width` |
| `margin-down`, `margin-up` | `decrease-margin`, `increase-margin` |
| `fewer-per-row`, `more-per-row` | `decrease-per-row`, `increase-per-row` |
| `tile-left`, `tile-right`, `tile-up`, `tile-down` | `list-tile-left`, `list-tile-right`, `list-tile-up`, `list-tile-down` |
| `open` | `list-open` |
| `filter` | `list-filter` |

`zoom-in` and `zoom-out` keep their names: they make the pages bigger or smaller, and on a
document they change the font. A test checks that the commands of each setting have the name of
its option.

### `--generate-config` and `--print-config`

- `clv --generate-config` writes a template to the place of the config: the default path, or the
  file of `--config FILE`. It makes the directory when there is none, and prints the path that it
  wrote. It never writes over a file: when one is there, clv refuses and names it. This is the
  only time that clv writes a config.
- Every line of the template that sets something is commented out. As written, it sets nothing,
  so the defaults of a later release still reach the reader. A line that sets something starts
  with one `#`: it is an example that shows the default, for the reader to take the `# ` from and
  edit. A line of explanation starts with `##`, so a reader, and the test below, can tell the two
  apart. The file starts by saying so. A banner starts each of its parts (the options, the keys,
  and each of the three tables): a line of 78 `#`, the title of the part, and a line of 78 `#`. The three lines that start the tables of the keys (`[keys]`, `[keys.pages]`,
  `[keys.list]`) are not commented out: an empty table sets nothing, and a reader takes the `# `
  from one line to bind a key.
- clv makes the template from the catalogs of this build:
  - each option that a config may set, with its help line and its default (`# width = "window"`)
  - `[keys]`, `[keys.pages]` and `[keys.list]`, with every default binding that a config can
    write (`# ctrl-v = "screen-down"`), under the help line of each command. A binding that both
    keymaps have alike is written once, under `[keys]`. `--print-config` writes by the same rule,
    as far as the order of a command's keys allows
  - a line of help for the two bindings that a config cannot write: `ctrl-c`, which always quits,
    and a number before Enter, which goes to that page or tile
- `clv --print-config` prints the config in effect to stdout, from the file and the command line.
  A line that the file or the command line sets is not commented out, and has its value. The
  other lines are commented out, with their defaults. So `clv --no-config --print-config` prints
  the template that `--generate-config` writes.
- A test removes the `# ` from every line of the template that sets something, and checks two
  things: clv takes the result, and the result gives the same options and keymaps as no config.
  The keymaps are the same in the order of their keys and in which keys `?` lists, too.

### When the config is wrong

- clv stops before it shows anything, and names the file, the line and the problem:
  `clv: ~/.config/clv/config.toml:3: width: expected window or 20–200, got 300`. All text from the
  file goes through `printable`.
- These are errors: bad TOML, an unknown name at the top level, an option that the config may not
  set, a bad value, and a bad key or command.

## Out of scope

- **Positions remembered** for files opened before. They are state that clv writes, not
  configuration, and they are a later spec ([the backlog](../backlog.md#a-config-file)).
- **Settings saved from the pager.** A width set with `]`, or a theme turned with `D`, is not
  written to the config.
- **A value for each backend** of a shared setting, and **a keymap for each format**. A setting
  stays one value for the run ([ADR-0033](../adr/0033-backends-share-a-setting-by-declaring-its-name.md)),
  and a key stays one command in each keymap ([ADR-0036](../adr/0036-keys-are-bound-to-named-commands.md)).
- **Commands that set a value** (`W = "set-width window"`), which need an argument in a binding.
  The number typed before a key already sets a value: `72]`.
- **A Mermaid theme of the reader's choice.** It is a new setting for the backend of Mermaid to
  declare. When it is declared, the config takes it with no change here.
- **A program of the reader's choosing for each format** (`[open-with]`, `pdf = ["zathura"]`).
  This spec first held it. It needs two things that clv does not have: the viewer does not know
  the format of a file, and a program has no session of its own on macOS
  ([the backlog](../backlog.md#open-with-a-program-of-the-readers-choosing-for-each-format)).
- **Key sequences** (`g g`, `ctrl-x ctrl-c`). Keys that clv does not read: Tab, Delete, Insert and
  the function keys. Keys that only the kitty keyboard protocol tells apart (`ctrl-shift-d`).
- **A command run by its name** ([the backlog](../backlog.md#running-a-command-by-name-m-x)).
- **The config read again** while clv runs, a config for each directory, includes, and options
  from an environment variable.

## Approach

**Files:**

- `src/args.rs`: the options catalog. `OptionSpec` gets the shape of its value and whether a
  config may set it. One parser checks a value. `Parsed` takes the options of the config after the
  options of the command line, so the first value given still wins, and `takes_all` stays one
  check. `Parsed` also says which options came from the config, for the rule on `width` and
  `per-row`. Errors from the config carry the file and the line.
- `src/view/mod.rs`: `OPTIONS` gets its shapes. `--config`, `--no-config`, `--generate-config` and
  `--print-config` join it. `run` reads the config before it parses the options. `started` parses
  the value of a setting today: the catalog takes that over, for settings and for the viewer's
  options alike. The warning for `width` against `per-row`, in `view`, follows the new rule.
- `src/view/config.rs` (new, in the core): finds the file and reads it, up to 1 MiB. It parses
  the TOML with spans, so that every error has a line. It turns the file into options and
  bindings. It writes the template that `--generate-config` and `--print-config` share.
- `src/view/keys.rs`: the name of a key, both ways. One function spells a key for the config and
  for `?`, so that the two cannot differ.
- `src/view/commands.rs`: the new names. `Keymaps::build` applies the bindings of the config to
  the defaults, before the suggestions of the settings. It puts a key of the config first among
  the keys of its command. It leaves a binding that equals the default as it is, and it never
  changes a binding with a count (`counted`). `long` gives the lower-case spelling.
- `src/view/pager.rs`: the phrase of a setting is filled through `Keymap::fill`.
- `src/source.rs`: the docs of `Setting` say that its `help` writes its keys as
  `{increase-width}`, not `]`.
- `src/source/wordcraft.rs`, `src/source/org.rs`, `src/source/epub.rs`, `src/source/mermaid.rs`:
  the setting `paper` becomes `theme`; the phrase of the width uses the names of commands.
  `src/view/relayout.rs` and the tests that name the setting follow.
- `Cargo.toml`: `toml` moves from the dev-dependencies, where it is 0.9, to the dependencies, at
  1. Three tests read manifests with it (`tests/it/pdfcraft_pin.rs`, `tests/it/wordcraft.rs`,
  `tests/it/gridcraft.rs`), and they build against 1 after the move.
- Tests:
  - unit tests in `config.rs`, `args.rs`, `keys.rs` and `commands.rs`, with a property test that
    no text of a config makes clv panic
  - `tests/it/view.rs`, through a pipe: `per-row` from the config, the command line that wins, an
    error with its line, `--no-config`, `--generate-config` that writes and then refuses, and
    `--print-config`
  - `tests/it/view_pager.rs`: a key bound again, `?` with that key first, the phrase of the
    width, a number and Enter with Enter bound again, and `ctrl-c` with `q` unbound

**Patterns to follow:**

- Two catalogs, made the same way. The catalog of commands (`VIEWER`, and the commands of the
  settings) is the model for the options.
- A setting is still declared by its backend. The catalog reads `Setting::option`, `named`,
  `numbers` and `auto`, and the viewer names no backend
  ([ADR-0033](../adr/0033-backends-share-a-setting-by-declaring-its-name.md)).
- The config is part of the core, as `args.rs` is. Nothing under `src/source/` reads it.

**Decided:**

- **TOML, through the `toml` crate, version 1.** clv builds `toml` 1.2.1 already, as a build
  dependency of Stylo (`stylo_static_prefs`), and `cargo deny` checks it in `Cargo.lock`. clv's
  own tests use 0.9 today, which is 0.9.12 in `Cargo.lock`. With both at 1, the lock holds one
  version. Use serde only where `toml` needs it for spans. An ADR records why TOML, with these
  alternatives rejected:
  - an arguments file, as ripgrep and bat have: keys would become options, with shell-like
    quoting that clv writes itself
  - a line format of clv's own, as kitty and zathura have: a parser, quoting and escapes that clv
    writes and fuzzes, which no editor checks
  - YAML: indentation that matters, `no` read as false, and `serde_yaml` unmaintained
  - JSON: no comments, so no template
  - KDL: few readers know it, and its crate is young
  - Lua or another language: code that a file runs at startup

  The ADR also records the flat layout, keyed by the names of the catalogs, against a section for
  each backend, and the rules for names. It records that a binding with a count stays outside the
  config, which [ADR-0036](../adr/0036-keys-are-bound-to-named-commands.md) said to look at again.
- **The tests never read the developer's config.** Every test that runs `clv` goes through a
  helper that sets `XDG_CONFIG_HOME` to an empty directory. Without the helper, a reader's own
  `width = 72` would break the tests. Today 9 test files name the binary (`CARGO_BIN_EXE_clv`) in
  15 places, with no helper that they share. So the helper is the first task, before clv reads
  any config. The tests on a pseudo-terminal and in tmux need it too: there, `clv` gets its
  environment from script(1) or from the tmux server. `tests/docs.rs` is a binary of its own, and
  sets the variable itself.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- 🟡 **A config written for a full build** stops a build without some backends on an unknown
  name: `width` in a build with images alone. Few run such builds, so clv refuses for now. To
  skip such names, clv would have to know the names of the backends that it lacks.
- 🟡 **A flag with no opposite** stays on for every run once the config sets it: `recurse = true`.
  Only `--no-config` turns it off. An opposite for each flag (`--no-recurse`) would end that.

## Tasks

One checkbox each: a session's work, and a commit that builds and passes on its own.

- [x] **A config of their own for the tests** — one helper in `tests/it/` that runs `clv` with `XDG_CONFIG_HOME` set to an empty directory. Every test that runs the binary uses it, the tests on script(1) and in tmux too; `tests/docs.rs` sets the same. *Files:* `tests/it/*.rs`, `tests/docs.rs`. *Verify:* `cargo test -j 2`; `grep -rn CARGO_BIN_EXE_clv tests/` shows the helper and `tests/docs.rs` alone.
- [x] **The new names** — the setting `paper` as `theme`, and the commands of the table in Names. The phrase of a setting is written with the names of commands and filled through `Keymap::fill`. A test checks that the commands of each setting have the name of its option. *Files:* `src/view/commands.rs`, `src/view/pager.rs`, `src/view/relayout.rs`, `src/source.rs`, the four backends that declare `paper`, `docs/architecture.md`. *Verify:* `cargo test -j 2 --bin clv commands::`.
- [x] **One spelling for a key** — one function in `keys.rs` reads and writes the name of a key, in lower case. It refuses `ctrl-D`, `ctrl-m`, `ctrl-j`, `ctrl-h`, `ctrl-i` and `tab`, each with its message. `?` and the manual's tables take that spelling, and `?` keeps the arrows. *Files:* `src/view/keys.rs`, `src/view/commands.rs`, `tests/it/view_pager.rs`, `docs/usage.md`, `CHANGELOG.md`. *Verify:* `cargo test -j 2 --bin clv keys::`, with a property test that the name of every key reads back as that key; `cargo test -j 2 --test it the_keys_at_80`.
- [x] **The options catalog** — `OptionSpec` with the shape of its value and whether a config may set it. One parser reads a value, for the viewer's options and the settings' alike, with one form of message for a bad value. `--help` is made from the catalog. *Files:* `src/args.rs`, `src/view/mod.rs`, `src/source.rs`, the backends that declare an option, `tests/it/view.rs`. *Verify:* `cargo test -j 2 --bin clv args::`; `cargo test -j 2 --bin clv help_lists_the_options`.
- [x] **Options from the config** — `toml` at 1 in the dependencies, with its comment in `Cargo.toml` and its ADR. `config.rs` finds the file, reads it within 1 MiB and turns its top level into options. `--config` and `--no-config`. The command line wins; `pager` with `no-pager`, and `width` with `per-row`, follow their rules. Every error has its file and its line. *Files:* `Cargo.toml`, `src/view/config.rs`, `src/args.rs`, `src/view/mod.rs`, `tests/it/view.rs`, `docs/adr/`, `docs/usage.md`, `docs/architecture.md`, `CHANGELOG.md`. *Verify:* `cargo test -j 2 --bin clv config::`, with a property test that no text of a config makes clv panic; `cargo test -j 2 --test it config_`.
- [x] **Keys from the config** — `[keys]`, `[keys.pages]` and `[keys.list]` go into `Keymaps::build`, with the refusals of Keys: a digit, `ctrl-c`, a command that does not exist, and a command that does not apply. `?`, the reminders and the messages show the keys as bound. *Files:* `src/view/config.rs`, `src/view/commands.rs`, `src/view/pager.rs`, `tests/it/view_pager.rs`, `docs/usage.md`, `CHANGELOG.md`. *Verify:* `cargo test -j 2 --bin clv commands::`; `cargo test -j 2 --test it config_keys`.
- [x] **`--generate-config` and `--print-config`** — the template, made from the two catalogs. clv writes it once and never over a file, and prints it with what the file and the command line set. *Files:* `src/view/config.rs`, `src/view/mod.rs`, `tests/it/view.rs`, `docs/usage.md`, `CHANGELOG.md`. *Verify:* `cargo test -j 2 --bin clv the_template_uncommented_is_no_config`; `cargo test -j 2 --test it generate_config`; `Manual: run clv --generate-config twice, see the path, then a refusal that names the file`.
- [x] **Review fixes: `--print-config` prints a config that clv takes** — it checks the options as a run does before it writes them, and writes a number as clv read it, never as typed. *Files:* `src/view/mod.rs`, `src/view/config.rs`, `tests/it/view.rs`, `tests/it/wordcraft.rs`, `docs/usage.md`, `docs/architecture.md`. *Verify:* `cargo test -j 2 --test it print_config`; `cargo test -j 2 --bin clv a_value_is_written`.
- [x] **The template's keys in three tables** — the lines `[keys]`, `[keys.pages]` and `[keys.list]` are not commented out, and a key that both keymaps bind alike is written once, under `[keys]`. A line in the wrong part of the file says where it goes. `--generate-config` prints the path whole. *Files:* `src/view/config.rs`, `src/view/mod.rs`, `src/view/commands.rs`, `tests/it/view.rs`, `tests/it/view_pager.rs`, `docs/usage.md`, `docs/architecture.md`, `CHANGELOG.md`. *Verify:* `cargo test -j 2 --bin clv config::`; `cargo test -j 2 --test it config_keys_one_line`.

## Done

- [x] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [x] Tests added, each seen failing without the change:
  - the helper: without it, a config with `per-row = 3` in the developer's place breaks a test
    of `tests/it/view.rs`
  - the commands of each setting have the name of its option
  - the name of every key reads back as that key (a property test)
  - `the_template_uncommented_is_no_config`
  - no text of a config makes clv panic (a property test)
  - `config_…` in `tests/it/view.rs`: an option from the config, the command line that wins, an
    error with its line, `--no-config`, `width` against `per-row`
  - `config_keys_…` in `tests/it/view_pager.rs`: a key bound again and first in `?`, a number and
    Enter with Enter bound again, `ctrl-c` with `q` unbound
- [ ] Manual: in kitty, with `theme = "dark"` and `[keys] x = "quit"` in the config, open a
  Markdown file. See dark paper. Press `?`: see `x` first among the keys that quit. Press `x`:
  clv quits. Do the same inside tmux.
- [x] Manual: `clv --no-config --print-config` prints what `clv --generate-config` wrote.
- [x] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)): the ADR
  for TOML, the layout, the names and the bindings with a count; README's Plans bullet as
  "Positions remembered for files opened before"; the backlog's
  [A config file](../backlog.md#a-config-file) with what is left.

## Changelog

- 2026-11-21 — Ready (spec-new).
- 2026-11-22 — Back to Draft after a review against the code. `[open-with]` goes to the backlog: the viewer does not know the format of a file, and a program has no session of its own on macOS. `ctrl-c` and a number before Enter stay outside the config. New rules: `width` against `per-row`, the order of a command's keys, the comments of the template. Corrected: `toml` is 0.9 in the tests today, Tab is no key, and the function keys are among the keys that clv does not read. Tasks and Done written (spec-update).
- 2026-11-22 — Ready: the owner read the review and lifted it (spec-update).
- 2026-11-22 — Implementing ([notes](0015-read-options-and-keys-from-a-config-file-impl-20261122_2331.md)).
- 2026-11-23 — A task added after a review of the branch: `--print-config` printed options that a run refuses, and a number as it was typed.
- 2026-11-23 — The template's requirement changed after the owner tried it: its tables of keys are not commented out, and a key that both keymaps bind alike is written once, under `[keys]`. With the tables commented out, a key uncommented under one was read as an option (`unknown option x`), and `q = "quit"` was in the file twice.
- 2026-11-23 — The words at the top of the template and before its keys rewritten, after the owner read them: a line with one `#` is an example that shows the default, and a line with `##` is an explanation.
- 2026-11-23 — A banner before each part of the template, a title between two lines of `#`, after the owner asked for the sections to stand out.
