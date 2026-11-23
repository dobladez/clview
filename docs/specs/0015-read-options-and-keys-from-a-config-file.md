# Spec-0015: Read options and keys from a config file

- **Status:** Ready
- **Date:** 2026-11-21
- **Related:** [ADR-0021](../adr/0021-a-backend-declares-its-settings-and-their-options-hold-for-any-file.md)
  (a backend declares its settings); [ADR-0033](../adr/0033-backends-share-a-setting-by-declaring-its-name.md)
  (backends share a setting by its name); [ADR-0034](../adr/0034-open-a-file-through-the-platforms-opener-by-its-real-path.md)
  (the platform's opener, and the name check); [ADR-0036](../adr/0036-keys-are-bound-to-named-commands.md)
  (keys bound to named commands); [Spec-0010](0010-open-file-in-default-app.md) and
  [Spec-0013](0013-bind-keys-to-named-commands.md), which this builds on;
  [Options](../usage.md#options), [Keys](../usage.md#keys) and
  [Opening in another app](../usage.md#opening-in-another-app) in the manual;
  [Settings](../architecture.md#settings) and
  [Commands and keymaps](../architecture.md#commands-and-keymaps-commandsrs-pagerrs);
  the backlog's [A config file](../backlog.md#a-config-file). It delivers part of README's Plans
  bullet "A config file: customization, settings and positions remembered". At `Done`, that bullet
  becomes "Positions remembered for files opened before".

## Summary

clv takes everything from its command line. A reader who always wants `--per-row width`, dark
paper or `--compression small` must type it each time. A reader cannot open a PDF in zathura in
place of the desktop's default app, and cannot change a key. This spec gives clv a config file.
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

[open-with]
pdf = ["zathura"]

[keys]
ctrl-d = "screen-down"
x = "none"
"/" = "search"

[keys.list]
J = "list-tile-down"
```

`clv --generate-config` writes a template with every line commented out, and `clv --print-config`
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
- `--help` shows what it shows today, made from the catalog.

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

### `[open-with]`

- An entry maps the name of a format, as `--help` lists it, to a program and its arguments:
  `pdf = ["zathura"]`. clv matches the name without regard to case, so most names need no quotes.
- `o` runs that program with the path of the file as its last argument, for files of that format
  only. The other formats still go to the desktop's opener.
- clv starts the program as it starts the desktop's opener
  ([ADR-0034](../adr/0034-open-a-file-through-the-platforms-opener-by-its-real-path.md)): in a
  session of its own, with its output hidden, at most four at a time. The status line says how it
  went.
- The refusals of the desktop's opener stay: a file with no file of its own (a pipe, a file in an
  archive), a session over SSH, no desktop session on Linux, a program not on `PATH`. One check
  goes: the name of the file need not say its format. That check exists because the desktop
  chooses the app by the name. Here, clv chooses the app by the format that it found in the file.
- A folder in a list still goes to the file manager.

### Keys

- `[keys]` binds a key to a command by its name, in every keymap where the command applies.
  `[keys.pages]` and `[keys.list]` bind a key in one keymap, and win over `[keys]` there. A value
  is the name of a command, a setting's command too (`cycle-theme`, `increase-width`), or `"none"`,
  which unbinds the key.
- A key that the config binds loses its default binding. The command keeps its other keys. Where
  the config binds a key that a setting suggests for its command, the setting's suggestion is left
  out.
- clv refuses these, and says why:
  - a command that does not apply in the keymap of its subsection (`[keys.list] s = "search"`)
  - a command that does not exist
  - a digit, which starts the number typed before a key
  - `ctrl-c`, which always quits, so that a config cannot leave the reader without a way out
- Keys are written in lower case, and the case of a character counts: `j` is not `J`, and `alt-v`
  is not `alt-V`. A Ctrl key is lower case: clv refuses `ctrl-D` and says to write `ctrl-d`,
  because a terminal sends the same byte for both. clv also refuses `ctrl-m`, `ctrl-j`, `ctrl-h`
  and `ctrl-i`, and names the key that a terminal sends for each (Enter, Enter, Backspace, Tab).
  The names of keys are `space`, `enter`, `esc`, `backspace`, `up`, `down`, `left`, `right`,
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
- Every line of the template is commented out. As written, it sets nothing, so the defaults of a
  later release still reach the reader.
- clv makes the template from the catalogs of this build:
  - each option that a config may set, with its help line and its default (`# width = "window"`)
  - `[open-with]`, with an example for each format that this build shows
  - `[keys.pages]` and `[keys.list]`, with every default binding (`# ctrl-v = "screen-down"`),
    under the help line of each command
- `clv --print-config` prints the config in effect to stdout, from the file and the command line.
  A line that the file or the command line sets is not commented out, and has its value. The
  other lines are commented out, with their defaults. So `clv --no-config --print-config` prints
  the template that `--generate-config` writes.
- A test removes the comments from every line of the template and checks two things: clv takes
  the result, and the result gives the same options and keymaps as no config.

### When the config is wrong

- clv stops before it shows anything, and names the file, the line and the problem:
  `clv: ~/.config/clv/config.toml:3: width: expected window or 20–200, got 300`. All text from the
  file goes through `printable`.
- These are errors: bad TOML, an unknown name at the top level, an option that the config may not
  set, a bad value, an unknown format in `[open-with]`, and a bad key or command.

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
- **Terminal programs in `[open-with]`** (`vim`, `less`): clv hides their output and does not give
  them the terminal. Also `[open-with]` for folders, for a file in an archive, and over SSH.
- **Key sequences** (`g g`, `ctrl-x ctrl-c`), and keys that only the kitty keyboard protocol tells
  apart (`ctrl-shift-d`, `F1`).
- **A command run by its name** ([the backlog](../backlog.md#running-a-command-by-name-m-x)).
- **The config read again** while clv runs, a config for each directory, includes, and options
  from an environment variable.

## Approach

**Files:**

- `src/args.rs`: the options catalog. `OptionSpec` gets the shape of its value and whether a
  config may set it. One parser checks a value. `Parsed` takes the options of the config after the
  options of the command line, so the first value given still wins, and `takes_all` stays one
  check. Errors from the config carry the file and the line.
- `src/view/mod.rs`: `OPTIONS` gets its shapes. `--config`, `--no-config`, `--generate-config` and
  `--print-config` join it. `run` reads the config before it parses the options. `started` parses
  the value of a setting today: the catalog takes that over, for settings and for the viewer's
  options alike.
- `src/view/config.rs` (new, in the core): finds the file and reads it, up to 1 MiB. It parses
  the TOML with spans, so that every error has a line. It turns the file into options, entries of
  `[open-with]`, and bindings. It writes the template that `--generate-config` and
  `--print-config` share.
- `src/view/keys.rs`: the name of a key, both ways. One function spells a key for the config and
  for `?`, so that the two cannot differ.
- `src/view/commands.rs`: the new names. `Keymaps::build` takes the bindings of the config after
  the defaults and before the suggestions of the settings. `long` gives the lower-case spelling.
- `src/view/launch.rs`: a `Launcher` that runs the program of the config. It uses `setsid` on
  Linux and the same refusals (`Here`), but no name check.
- `src/view/pager.rs`: the phrase of a setting is filled through `Keymap::fill`.
- `src/source.rs`: the docs of `Setting` say that its `help` writes its keys as
  `{increase-width}`, not `]`.
- `src/source/wordcraft.rs`, `src/source/org.rs`, `src/source/epub.rs`, `src/source/mermaid.rs`:
  the setting `paper` becomes `theme`; the phrase of the width uses the names of commands.
- `Cargo.toml`: `toml` 1 moves from the dev-dependencies to the dependencies.
- Tests:
  - unit tests in `config.rs`, `args.rs`, `commands.rs` and `launch.rs`, with a property test that
    no text of a config makes clv panic
  - `tests/it/view.rs`, through a pipe: `per-row` from the config, the command line that wins, an
    error with its line, `--no-config`, `--generate-config` that writes and then refuses, and
    `--print-config`
  - `tests/it/view_pager.rs`: a key bound again, `?` with that key, the phrase of the width,
    `ctrl-c` with `q` unbound, and `[open-with]` with a fake program on `PATH`, as the tests of
    `launch.rs` fake one

**Patterns to follow:**

- Two catalogs, made the same way. The catalog of commands (`VIEWER`, and the commands of the
  settings) is the model for the options.
- A setting is still declared by its backend. The catalog reads `Setting::option`, `named`,
  `numbers` and `auto`, and the viewer names no backend
  ([ADR-0033](../adr/0033-backends-share-a-setting-by-declaring-its-name.md)).
- The config is part of the core, as `args.rs` is. Nothing under `src/source/` reads it.

**Decided:**

- **TOML, through the `toml` crate, version 1.** clv builds `toml` 1.2.1 already, as a build
  dependency of Stylo, and `cargo deny` checks it in `Cargo.lock`. Version 1 keeps one version in
  the build; use serde only where `toml` needs it for spans. An ADR records why TOML, with these
  alternatives rejected:
  - an arguments file, as ripgrep and bat have: keys and `[open-with]` would become options, with
    shell-like quoting that clv writes itself
  - a line format of clv's own, as kitty and zathura have: a parser, quoting and escapes that clv
    writes and fuzzes, which no editor checks
  - YAML: indentation that matters, `no` read as false, and `serde_yaml` unmaintained
  - JSON: no comments, so no template
  - KDL: few readers know it, and its crate is young
  - Lua or another language: code that a file runs at startup

  The ADR also records the flat layout, keyed by the names of the catalogs, against a section for
  each backend, and the rules for names.
- **The tests never read the developer's config.** Every test that runs `clv` goes through a
  helper that sets `XDG_CONFIG_HOME` to an empty directory. Without the helper, a reader's own
  `width = 72` would break the tests.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- 🟡 **A config written for a full build** stops a build without some backends on an unknown
  name: `width` in a build with images alone. Few run such builds, so clv refuses for now. To
  skip such names, clv would have to know the names of the backends that it lacks.
- 🟡 **`[open-with]` over SSH** stays refused, as the desktop's opener is. A reader with X
  forwarding could want it.

## Tasks

One checkbox each: a session's work, and a commit that builds and passes on its own.

- [ ] **Noun phrase** — what gets built. *Files:* `src/...`, `tests/it/...`. *Verify:* `cargo test -j 2 ...`, or `Manual: in kitty, press X, see Y`.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [ ] Tests added, each seen failing without the change: name them.
- [ ] Manual: …
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- 2026-11-21 — Ready (spec-new).
