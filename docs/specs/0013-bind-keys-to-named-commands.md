# Spec-0013: Bind keys to named commands, and rebind them

- **Status:** Implementing
- **Date:** 2026-11-15
- **Related:** [ADR-0021](../adr/0021-a-backend-declares-its-settings-and-their-options-hold-for-any-file.md)
  (a backend declares its settings and their keys; "the viewer's keys come first", which this
  amends); [ADR-0023](../adr/0023-the-window-is-the-page-of-markdown-and-html.md) (the zoom keys
  handed to a document's font size); [ADR-0033](../adr/0033-backends-share-a-setting-by-declaring-its-name.md)
  (what the viewer says of a setting is put together from its declarations);
  [Layers](../architecture.md#layers) and [Settings](../architecture.md#settings);
  [Keys](../usage.md#keys); the backlog's [A config file](../backlog.md#a-config-file), which
  rebinds keys on what this builds, and [`d` for drawings too](../backlog.md#d-for-drawings-too),
  whose key becomes `D`. Spec-0014, to come, makes `j` and `k` scroll a few rows on Markdown and
  HTML. It delivers none of README's Plans.

## Summary

The pager's keys are written into its `match` arms: `j` scrolls because `Nav::key` says
`Key::Char('j')`, and `?`'s list, the status line's reminders and `VIEWER_KEYS` repeat them by
hand. This spec separates what the pager can do from the keys that do it, as Emacs does. A
**catalog of commands** holds each action by name, with its signature (whether it takes the number
typed before it, and what it does without one) and its help: the viewer's own (`screen-down`,
`tile-left`, `back`, `quit`…) and one or two per setting a backend declares, named from the setting
(`paper`, `width-up`). **Keymaps** bind keys to command names, one for a file's pages and one for a
list, built from the viewer's defaults and the keys settings suggest. `?` lists commands with their
keys, and the status line's reminders are read from the keymap, so a config file (a later spec)
can rebind keys without changing a command.

On that layer the keys change. `d` and `u` go a screen down and up as Space and `b` do, `D` turns
the paper where `d` did, and `u` no longer means "back", which Backspace and Esc keep. Emacs keys
come in: `C-n` `C-p` `C-f` `C-b` move among a list's tiles as `j` `k` `l` `h` do, `C-n` and `C-p`
are `j` and `k` in a document, and `C-v` and `M-v` go a screen down and up everywhere. `M-v` works
because an ESC and a key arriving in the same read become Alt and that key, instead of Esc then the
key.

On Markdown and HTML, `j` and `k` still turn pages here. Making them scroll a few rows across pages
is Spec-0014.

## Requirements

### The catalog

- Each command has a name (kebab-case), a help line, a signature, and where it applies (a file's
  pages, a list, or both). The signature says whether it takes the number typed before it, and what
  it does with one and without.
- The digits stay the count, Emacs's prefix argument, outside the keymaps.
- A backend's setting adds its commands under names built from its own: `paper` for a
  `Keys::Cycle`, `width-down` and `width-up` for a `Keys::Step`. Their help comes from the phrases
  it already declares ([ADR-0033](../adr/0033-backends-share-a-setting-by-declaring-its-name.md)),
  and its keys are suggestions the keymaps take. The viewer names no backend's command.
- A setting on the zoom keys (`Keys::Zoom`, the documents' `font`) is reached through `zoom-in`
  and `zoom-out`, which hand over to it on a document shown by itself, as today
  ([ADR-0023](../adr/0023-the-window-is-the-page-of-markdown-and-html.md)).

### The pages keymap

By default:

| Command | Keys |
|---|---|
| `screen-down` | Space, PgDn, `d`, `C-v` |
| `screen-up` | `b`, PgUp, `u`, `M-v` |
| `scroll-down` | `j`, ↓, Enter, `C-n` |
| `scroll-up` | `k`, ↑, `C-p` |
| `next`: the next page, or with a search kept the next match | `n`, → |
| `previous`: the page before, or with a search kept the match before | `p`, ← |
| `previous-match` | `N` |
| `go-to`: the first page, or the page counted | `g`, Home; Enter with a count (`12⏎`) |
| `last` | `G`, End |
| `zoom-in`, `zoom-out` | `+` `=`, `-` `_` |
| `more-per-row`, `fewer-per-row` | `,`, `.` |
| `search` | `/` |
| `follow`, `copy-link` | `f`, `F` |
| `copy` | `c` |
| `open-in-app` | `o` |
| `rerender` | `r`, `C-l` |
| `back`: clear the search kept, then back through the links followed, then out of the file | Backspace |
| `leave`: clear the search kept, then out of the file; quit where there is nothing to leave to | Esc |
| `quit` | `q`, `C-c` |
| `help` | `?` |
| The documents' settings | `paper` `D`; `width-down` `width-up` `[` `]`; `margin-down` `margin-up` `{` `}`; `fences` `s` |

What each does, with a count and without, is what its keys do today.

### The list keymap

`tile-left` (←, `h`, `C-b`), `tile-right` (→, `l`, `C-f`), `tile-up` (↑, `k`, `C-p`) and
`tile-down` (↓, `j`, `C-n`); `screen-down` (Space, `n`, PgDn, `d`, `C-v`) and `screen-up` (`b`, `p`,
PgUp, `u`, `M-v`); then `go-to`, `last`, `open` (Enter, or the tile counted), `filter` (`/`), the
zoom and per-row commands, `rerender`, `copy`, `open-in-app`, `back` and `leave` (clearing the
filter kept first), `quit`, `help` and the settings, on the keys they have today.

### What else changes

- `u` no longer goes back anywhere: not in a file's pages, not in a list, not through the links
  followed, nor while a folder is being gone into. Backspace and Esc do what it did.
- `D` does everything `d` did, saying which formats go dark on a file that doesn't too. `d` is no
  setting's key.
- In a keymap, a key goes to one command; a setting's suggested key never takes one of the viewer's
  defaults; two settings never suggest the same key. A test checks all three, in place of
  `VIEWER_KEYS`.
- `?` and the status line's reminders write no key by hand: each comes from the keymap. Two commands
  may share a line, as down and up do.

### Keys decoded

- Ctrl and a letter becomes `Ctrl(letter)`, but for the bytes that already mean Enter, Tab,
  Backspace and `C-c`. `C-l` moves to it from `Char('\u{c}')`.
- ESC and a printable character arriving in one read become `Alt(char)`, apart from the sequences
  ESC `[`, `O`, `]` and `_` start. An ESC alone is still Esc.
- An unbound Ctrl or Alt key does nothing: in particular, it no longer acts as Esc. Neither types
  into a search or a filter.
- The links' labels (`f`), and the text of a search or a filter being typed, still take characters
  as they come, outside the keymaps.

## Out of scope

- **A config file, and rebinding keys by the reader.** The keymaps are built so that one can
  ([A config file](../backlog.md#a-config-file)).
- **`j` and `k` scrolling a few rows on Markdown and HTML**, across pages: Spec-0014.
- **kitty's keyboard protocol**, and with it Shift-Space and anything else only it tells apart.
- **Running a command by name** (an `M-x`): the catalog makes it possible
  ([backlog](../backlog.md#running-a-command-by-name-m-x)).
- **More Emacs keys:** `M-<` and `M->`, `C-s`, `C-g`, and line editing (`C-a`, `C-e`, `C-k`) in a
  search or a filter being typed.
- **The links' labels, a search or a filter on keymaps:** they take typed text, not commands.
- **The mouse.**

## Approach

**Files:**

- `src/view/keys.rs` — `Key::Ctrl(char)` and `Key::Alt(char)` in the decoder: `C-l` off
  `Char('\u{c}')`, and `escape()`'s "Esc, then another key typed quickly enough to arrive in the same
  read" turned into `Alt` for a printable character.
- `src/view/commands.rs` (new) — the catalog and the keymaps: `Command`, `ScreenDown`, `TileLeft`…
  and `Setting(&'static Setting, Turn)`, each with its `name()`, help, count signature and where it
  applies; `Keymap`, built once a run from the viewer's default bindings and the build's settings
  (`view::settings`), looked up from a key to its command and from a command to its keys. Its tests
  replace `VIEWER_KEYS` and `pager::tests::the_viewers_keys_are_those_listed`.
- `src/view/pager.rs` — `Nav::key` and `list_key` look the key up, then `match` on the `Command`, not
  the key. `help_lines`, `LIST_KEYS`, `OPENED_KEYS`, `KEYS`, `PAGE_HELP`, `LIST_HELP` and `BACK_HELP`
  take their keys from the keymap. The run loop's stop while a folder is gone into takes `back` and
  `leave`, not `u`.
- `src/source.rs` — `Keys` documented as the keys a setting suggests;
  `a_settings_keys_are_its_own_in_every_backend` checks them against the keymaps' defaults.
- `src/source/wordcraft.rs`, `src/source/mermaid.rs` — `paper` on `Keys::Cycle('D')`.
- `tests/it/view_pager.rs`, `tests/it/view_tmux.rs`, `tests/it/wordcraft.rs`,
  `tests/it/mermaid.rs` — `d` to `D` and `u` to Backspace where they mean the paper or back; new
  pager tests for `d`, `u`, `C-v` and `M-v` paging, and Emacs keys among a list's tiles.
- Docs — `docs/usage.md`'s Keys (both tables; that `M-v` needs the terminal to send Option or Alt as
  ESC, kitty's `macos_option_as_alt` on macOS), `docs/architecture.md` (a section on commands and
  keymaps, the viewer's file table, Settings), `CHANGELOG.md`, README's Plans line "`d` for SVG
  drawings" to `D`, and the backlog's `d` for drawings.

**Patterns to follow:**

- Settings declared as static tables: `PAPER` in `src/source/wordcraft.rs`.
- `?`'s lines put together from declarations: `Nav::help_lines`
  ([ADR-0033](../adr/0033-backends-share-a-setting-by-declaring-its-name.md)).
- The decoder's property tests (`keys.rs`: answers cut anywhere among keys are never keys), extended
  to Ctrl and Alt.

**Decided:**

- Commands are a Rust enum with names, not strings looked up at dispatch: an exhaustive `match`
  keeps every command handled, and the names are what a config file will use.
- A new ADR, "Keys are bound to named commands", records the catalog and keymaps, `u` no longer
  back, and ESC and a key in one read as Alt. It amends ADR-0021's "the viewer's keys come first" to
  "the viewer's defaults come first in the keymaps".
- `12⏎` stays: Enter with a count is `go-to`.

## Open questions

None.

## Tasks

One checkbox each: a session's work, and a commit that builds and passes on its own.

- [x] **Ctrl and Alt keys decoded** — `Key::Ctrl` and `Key::Alt`, `C-l` moved to `Ctrl('l')` with
  `r`'s binding following it, nothing else bound yet. *Files:* `src/view/keys.rs`,
  `src/view/pager.rs`. *Verify:* `cargo test -j 2 --bin clv keys::`, the property tests covering
  Ctrl and Alt cut anywhere.
- [ ] **The catalog and the keymaps, keys as today** — `src/view/commands.rs`, the pager dispatching
  through it, `?` and the reminders read from it, `VIEWER_KEYS` gone. A refactor: every key does what
  it did. *Files:* `src/view/commands.rs`, `src/view/pager.rs`, `src/view/mod.rs`, `src/source.rs`,
  `docs/architecture.md`. *Verify:* `cargo test -j 2`, the existing tests unchanged.
- [ ] **The new keys** — `d` and `u` a screen, `D` the paper, `u` no longer back, `C-n` `C-p` `C-f`
  `C-b`, `C-v` and `M-v`; the tests that pressed `d` or `u`; the docs, the changelog and the ADR.
  *Files:* `src/view/commands.rs`, `src/view/pager.rs`, `src/source/wordcraft.rs`,
  `src/source/mermaid.rs`, `tests/it/*.rs`, `docs/usage.md`, `docs/architecture.md`, `docs/adr/`,
  `CHANGELOG.md`, `README.md`, `docs/backlog.md`. *Verify:* `cargo test -j 2 --test it view_pager`;
  Manual: in kitty, open a PDF, press `d`, `u`, `C-v`, `M-v` (with `macos_option_as_alt`): a screen
  down and up each; on a Markdown file `D` turns the paper; in a list, `C-f` `C-n` move to the next
  tile and the one below; Backspace leaves the file and `u` doesn't.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [ ] Tests added, each seen failing without the change: name them.
- [ ] Manual: in kitty and inside tmux, `d`, `u`, `C-v`, `M-v` and `D` on a PDF, a Markdown file and
  a list, and `?` listing the keys from the keymap.
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- 2026-11-15 — Ready (spec-new).
- 2026-11-15 — Implementing ([notes](0013-bind-keys-to-named-commands-impl-20261115_2206.md)).
