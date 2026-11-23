# ADR-0043: A config in TOML, flat, keyed by the names of the catalogs

- **Status:** Accepted
- **Date:** 2026-11-23

## Context

clv took everything from its command line. A reader who always wanted `--per-row width` or dark
paper typed it each time, and no reader could change a key.
[Spec-0015](../specs/0015-read-options-and-keys-from-a-config-file.md) gives clv a config file.

Three things were already there to build on:

- The commands have names, and keymaps bind keys to them
  ([ADR-0036](0036-keys-are-bound-to-named-commands.md)).
- A backend declares its settings, and two backends share a setting by its name
  ([ADR-0021](0021-a-backend-declares-its-settings-and-their-options-hold-for-any-file.md),
  [ADR-0033](0033-backends-share-a-setting-by-declaring-its-name.md)).
- The viewer names no backend and knows no format
  ([Layers](../architecture.md#layers)).

The file is untrusted input, as every file that clv reads is. So clv must not panic on it, must
cap what it reads, and must make safe every piece of its text that it prints.

Four choices follow: the format of the file, its layout, the names that become its keys, and what
stays outside it.

## Decision

**The config is one TOML file**, `$XDG_CONFIG_HOME/clv/config.toml`, or
`~/.config/clv/config.toml`, on macOS too. clv parses it with the `toml` crate, version 1, with
its parser alone (`default-features = false`, `std` and `parse`). clv reads the place of every
key and value (`toml::de::DeTable`), so every error has its line. clv does not use serde for it.

**The file is flat, and the names of the two catalogs are its keys.**

- An option is at the top level, under its name without the dashes: `width = 72`,
  `theme = "dark"`. The options catalog says which options a config may set
  ([The options catalog](../architecture.md#the-options-catalog-argsrs)).
- A key is under `[keys]`, `[keys.pages]` or `[keys.list]`, bound to a command by the name of
  the command: `ctrl-d = "screen-down"`.

There is no section for a backend or a format. A setting is one value for the run, whichever
backend's file clv shows, and a key is one command in each keymap.

**The names keep to three rules**, because a config now depends on them:

- A command that steps an option is `increase-` or `decrease-` and the name of the option. A
  command that goes to the next named value is `cycle-` and that name. A setting has the name of
  its option, so its commands have that name too.
- A command for the list alone is `list-` and what it does.
- No name has the prefix of a format or a backend.

So `paper` became `theme`, `width-up` became `increase-width`, and `open` became `list-open`,
before any config could name them.

**A binding with a number typed before it stays outside the config**, as the digits do. `12` and
Enter go to page 12, whatever a config binds Enter to. `ctrl-c` stays outside too: it always
quits, so a config cannot leave the reader without a way out.
[ADR-0036](0036-keys-are-bound-to-named-commands.md) said to look at the counted bindings again
when the config came. They stay as they are.

## Evidence

- `Cargo.lock` held `toml` 1.2.1 before this change, as a build dependency of Stylo
  (`stylo_static_prefs`), and `toml` 0.9.12 for three tests. After the change it holds 1.2.1
  alone: `grep -c 'name = "toml"' Cargo.lock` gives 1. `toml_datetime` 0.7.5 and `winnow` 0.7.15
  went with 0.9.12.
- `toml::de::DeTable::parse` needs the features `std` and `parse` only. `toml::Table`, which the
  three tests read manifests with, needs `serde`: the tests ask for it as a dev-dependency.
- A table of `toml` keeps its keys in the order of their names, not of the file. clv sorts the
  entries by where each key is (`Spanned::span`). So clv needs no `preserve_order` feature, and
  no `indexmap`.
- `view::config::tests::no_text_of_a_config_panics` reads text that is nearly a config, cut
  anywhere. With a slice by bytes put in the place of the one by characters, it fails: `end byte
  index 40 is out of bounds`.

## Alternatives considered

For the format:

- **An arguments file**, as ripgrep and bat have. It holds options and nothing else: a key would
  have to become an option, and clv would write the shell-like quoting itself.
- **A line format of clv's own**, as kitty and zathura have. clv would write and fuzz a parser,
  its quoting and its escapes, and no editor would check the file.
- **YAML.** Indentation matters, `no` reads as false, and `serde_yaml` is unmaintained.
- **JSON.** It has no comments, so it could not hold a template that explains itself.
- **KDL.** Few readers know it, and its crate is young.
- **Lua, or another language.** It is code that a file runs at startup.

For the layout:

- **A section for each backend** (`[documents] width = 72`). The viewer would have to know the
  backends by name, and a setting that two backends share would have two places.
- **A keymap for each format.** A key is one command in each keymap, and the viewer knows no
  format.

For the bindings with a number:

- **A way to write them** (`"12 enter" = "go-to"`). A number before a key is the count of the
  command, as in Emacs, and only Enter has a binding of its own with one. One line of help in the
  template costs less than a syntax for one key.

## Consequences

- **Positive:** a config needs no code of its own for a new option, a new setting or a new
  command. A backend declares a setting, and the config takes it. An error names the file and
  the line.
- **Accepted:** the names of options and commands are now a contract. To change one is to break
  a reader's config, so a change needs an alias, as `--zoom` has. A config written for a full
  build stops a build without some backends on a name that it does not have.
- **Accepted:** a flag with no opposite stays on once the config sets it (`recurse = true`), and
  only `--no-config` turns it off.
- **Follow-ups:** positions remembered for files opened before, which are state that clv writes
  ([the backlog](../backlog.md#a-config-file)); a program of the reader's choosing for each
  format ([the backlog](../backlog.md#open-with-a-program-of-the-readers-choosing-for-each-format)).
- **Revisit when:** a reader needs a value for one backend alone, or a key for one format alone;
  or a second key needs a binding with a number.
