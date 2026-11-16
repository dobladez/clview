# ADR-0036: Keys are bound to named commands

- **Status:** Accepted
- **Date:** 2026-11-16

## Context

The pager's keys were written into its `match` arms: `j` scrolled because `Nav::key` said
`Key::Char('j')`, and `?`'s lines, the status line's reminders and `VIEWER_KEYS` (the keys no
setting may take) repeated them by hand, four copies to keep alike. A config file that rebinds keys
([the backlog](../backlog.md#a-config-file)) had nothing to bind them to, and Emacs's keys couldn't
be added beside `less`'s without another arm each. A backend's setting brought its keys with it
([ADR-0021](0021-a-backend-declares-its-settings-and-their-options-hold-for-any-file.md)): "the
viewer's keys come first", and a key the viewer didn't take was looked up among the settings.

Two keys were in the way of the ones wanted. `d` was the paper's
([ADR-0015](0015-markdown-and-html-on-dark-paper-the-terminals-own-colour.md)) and `u` went back
(up), where `less` and `vi` readers expect `d` and `u` to page. And Alt keys didn't exist: a
terminal that sends Alt or Option as `ESC` writes `ESC v` for `Alt-V`, which the decoder read as
the Esc key and then `v`, so `Alt-V` left the file.

[Spec-0013](../specs/0013-bind-keys-to-named-commands.md) is the change.

## Decision

**A catalog of commands, and keymaps that bind keys to them**
([Commands and keymaps](../architecture.md#commands-and-keymaps-commandsrs-pagerrs)).

- What the pager can do is a `Command`, a Rust enum, each with a kebab-case name (`screen-down`,
  `tile-left`, `go-to`), what it makes of the number typed before it, and where it applies. A
  setting a backend declares adds one or two, named from the setting (`paper`; `width-down`,
  `width-up`): the viewer names no backend's command.
- Two keymaps, one for a file's pages and one for a list, built once a run from the viewer's
  defaults and the keys the settings suggest. **The viewer's defaults come first in the keymaps**,
  which is what ADR-0021's "the viewer's keys come first" becomes: a setting's key is a suggestion,
  left out where a default has the key in either keymap, or another setting has. A test holds the
  build to none left out.
- The pager looks a key up and matches on the command, exhaustively. `?`, the reminders and every
  message that names a key read the keymap. Outside the keymaps stay the digits (the number typed,
  Emacs's prefix argument) with Esc and Backspace while one is typed, the links' labels, and the
  text of a search or a filter being typed.
- A key may be bound only with a number typed before it: Enter scrolls in a file's pages and opens
  a tile in a list, and after a number goes to the page or the tile counted (`12⏎`), which is
  `go-to` in both.

**The keys change on that layer.**

- `d` and `u` go a screen down and up, as Space and `b` do. The paper's key is `D`. `u` goes back
  nowhere: Backspace does, and Esc leaves.
- Emacs's keys beside the others: `Ctrl-N` and `Ctrl-P` are `j` and `k` (in a list, the tile below
  and above), `Ctrl-F` and `Ctrl-B` the tile after and before, `Ctrl-V` and `Alt-V` a screen down
  and up.
- → and ← keep turning pages while a search is kept, as before, where `n` and `p` go from match to
  match: so they are two more commands, `next-page` and `previous-page`, beside `next` and
  `previous`. The spec first had one command for `n` and →, which would have changed the arrows;
  the owner chose to keep them (2026-11-15).

**An ESC and a printable character read together are Alt and that character**, never Esc and
then the key. "Together" is the 50 ms the pager already waited after a lone `ESC` before taking it
for the Esc key, so the two cut between reads, as over SSH, are still Alt. Not `[` and `O`, which
start a key's sequence, nor `_` and `]`, which start an answer. Ctrl and a letter is a key too,
but for the bytes that are Enter, Backspace, Tab and Ctrl-C. A Ctrl or Alt key bound to nothing
does nothing.

## Evidence

No terminal was measured for this: it rests on how terminals are documented to send these keys
(a control byte for Ctrl and a letter; `ESC` and the key for Alt where the terminal is set to,
kitty's `macos_option_as_alt` on macOS, xterm's `metaSendsEscape`), and on tests that write those
bytes to the pager on a pseudo-terminal.

- `cargo test -j 2 --bin clv keys::`: Ctrl and Alt keys among the terminal's answers, cut into
  reads anywhere, come out as those keys, and no answer as a key.
- `cargo test -j 2 --bin clv commands::`: each keymap binds a key once and every command where it
  applies; a setting's key that is the viewer's, or another setting's, is left out and said; `?`'s
  keys and the reminders, read from the keymaps, are what they were when written by hand.
- `cargo test -j 2 --test it d_u_and_emacss_keys`: `ESC v` written in one write pages up, `ESC q`
  does nothing, and `u` leaves neither a file nor a list.
- What the Alt decoding cost showed in the tests: thirteen wrote an `ESC` and another key in one
  write, or pressed a key before the Esc had been read, to mean Esc and then the key. Each is Alt
  and the key now, and does nothing. They press Backspace instead.
- A manual check in kitty and inside tmux is the spec's last item, `Alt-V` with
  `macos_option_as_alt yes` among it.

## Alternatives considered

- **Keep the keys in the `match`, and add arms for the new ones.** No change to explain, and the
  fifth copy of the keys to keep alike; a config file would still have nothing to bind.
- **Commands as strings, looked up when a key is pressed.** What a config file writes, but a
  command nobody handles is found by the reader, not the compiler. The enum's `match` is
  exhaustive, and the names are still there for a file to use.
- **One keymap for everything.** `j` is a few rows down in a file and the tile below in a list,
  and Enter scrolls in one and opens in the other: one map would need each command to mean two
  things. Two maps, and a command says where it applies.
- **A setting's key wins over the viewer's default.** A backend could then take `q`. The viewer's
  defaults come first, as its keys did.
- **`d` and `u` half a screen, as in `less` and `vi`.** A page here is a picture, usually the
  size of the window: half a screen would put the cut through the middle of it. A whole screen, as
  Space and `b`.
- **Keep `u` as back too, where there is nothing to page up to.** One key, one command: `u`
  meaning "up a screen, or back at the top" would leave a file by a key pressed once too often.
  Backspace and Esc go back.
- **Read Alt through kitty's keyboard protocol.** It would tell `Alt-V` from Esc and `v` for
  certain, and Shift-Space too, but only in the terminals that speak it, and it changes what
  every key sends for as long as the pager runs. Out of this change's scope; the `ESC` prefix is
  what every terminal can be set to send.
- **Keep Esc and a key read together as Esc and the key.** Then `Alt-V`, and any Alt key pressed
  by habit, leaves the file.

## Consequences

- **Positive:** a key is written once, in the defaults or in a setting's declaration; a config
  file has names to bind keys to; `less`'s, `vi`'s and Emacs's paging keys all work.
- **Accepted:** `d` no longer turns the paper and `u` no longer goes back, which readers of
  earlier versions have in their fingers: the changelog says so. Esc and then a key within 50 ms
  is Alt and that key: typed by hand that is rare, and it is what a paste or a test that writes
  both at once gets. `M-_` and `M-]` can't be told from an answer's start, and aren't keys.
  Alt with Enter, or with a Ctrl key, is still Esc and that key. A setting's phrase in `?` still
  names its keys as its backend declared them (`72] sets 72`).
- **Follow-ups:** the config file; running a command by name
  ([the backlog](../backlog.md#running-a-command-by-name-m-x)); `j` and `k` scrolling Markdown and
  HTML a few rows across pages, on these commands.
- **Revisit when:** kitty's keyboard protocol is taken up, which would end the guess about `ESC`;
  or the config file shows the two keymaps or the counted bindings to be the wrong shape for what
  readers want to write.
