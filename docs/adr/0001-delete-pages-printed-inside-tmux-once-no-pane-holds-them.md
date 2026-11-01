# ADR-0001: Delete pages printed inside tmux once no pane holds them

- **Status:** Accepted
- **Date:** 2026-10-30

## Context

After days of trying clv on directories of photos, kitty used 2 GB in Activity Monitor, and
10 GB the day before. clv was run inside tmux. There it prints a page as an image in the
terminal, under an id, plus Unicode placeholder cells that tmux keeps as text
([Directly, or inside tmux](../architecture.md#directly-or-inside-tmux)). `view` exits with
the image still in the terminal, so the cells can show it again when they scroll back. Nothing
tells the terminal when tmux lets go of them. tmux never sees the image, and the protocol leaves
deleting to whoever sent it.

## Decision

Each page printed inside tmux is recorded, by pane and image id, in the tmux server's user
option `@clv-printed`, keeping the newest 2048. Every run of clv inside tmux, before it draws,
reads each recorded pane: `capture-pane -p -e` of its history and screen, and `-a` for the
screen saved behind a full-screen program. It then deletes (`a=d,d=I`) the recorded images whose
placeholder cells the pane no longer holds, or whose pane is gone. A pane that can't be read, or
one holding more than 32 MiB, keeps its records. There's no flag: every run tidies up, and the
code is `src/view/sweep.rs`.

## Evidence

kitty 0.49.2 (the latest, 2026-10-01), tmux 3.5a and Ghostty 1.3.1, on macOS 26. The terminal
was a visible 1600 × 1000 window started with `--config NONE`, with `TMUX` unset. A script run
inside it read the terminal's `footprint(1)`. The photos were 2548 × 1698, about 17 MB of pixels
each.

| kitty, 40 photos printed by clv | GPU (IOAccelerator) | Footprint |
|---|---|---|
| fresh | 43 MB | 267 MB |
| printed directly | 399 MB | 1235 MB |
| …`clear; printf '\e[3J'` | 89 MB | 688 MB |
| 40 more, inside tmux (`tmux -L`, `allow-passthrough on`) | 399 MB | 1434 MB |
| …`clear-history`, then the tmux server killed | 395 MB | 1203 MB |

| kitty, 20 photos printed inside tmux | GPU |
|---|---|
| printed, then `clear-history` | 394 MB |
| the kitty tab closed | 89 MB |
| again, in a new tab: `printf '\ePtmux;\e\e_Ga=d,d=R,x=1,y=4294967295\e\e\\\e\\'` | 394 → 89 MB |
| with the sweep: printed, `clear-history`, then one more clv run | 244 → 66 MB |

- **Directly, nothing stays:** kitty frees a printed page once it leaves the scrollback, PNG and
  raw pixels alike, and it frees what the pager deletes.
- **Ghostty frees them too:** PNGs, raw pixels and clv's printed photos all went when cleared
  (GPU back to 6 MB). Its allocator kept about 450 MB, which later pictures reused rather than
  added to.
- **Inside tmux, the quota is the only bound:** in one window, 40 photos stopped growing at
  about 400 MB of GPU memory, near kitty's 320 MB per screen. How many kitty windows reached the
  10 GB wasn't measured.
- **A hidden kitty misleads.** In `kitty --start-as=hidden`, cleared PNGs stayed while raw
  pixels went: 120 printed photos held 2 GB, past clearing and closing the window. A visible
  window doesn't do that. This first suggested a kitty bug, and sending raw pixels instead of
  PNGs as the fix; neither held up.
- **What `capture-pane` returns:** `-e` keeps the placeholders' colours and diacritics, in
  24-bit colour and in the palette. While a pane is on the alternate screen, the plain capture
  gives the history and the alternate screen, and `-a` the main screen saved behind it.
  `tests/it/view_tmux.rs` needs both.

## Alternatives considered

- **Delete everything clv printed when it exits:** blanks its pages in tmux's history, which is
  why they stay in the first place.
- **Delete every id in this window (`a=d,d=R`), by hand or as an option:** deletes other
  programs' images too, and blanks what is still in the history.
- **A tmux hook after `clear-history` or when a pane exits:** a hook runs in no pane, so it
  can't send through passthrough. Writing straight to the client's terminal (`#{client_tty}`)
  could land inside tmux's own output.
- **A flag that only tidies:** not needed when every run does, and closing the tab frees
  everything anyway.
- **Delete every id the panes don't show, with no record:** clv can't ask the terminal which
  images it holds, so this would mean deleting ranges around the shown ids, other programs'
  images included.
- **Print smaller pages inside tmux:** cuts the cost without bounding it.

## Consequences

- **Positive:** a picture tmux no longer holds is freed by the next clv run inside tmux, and
  pages still in some pane's history keep showing.
- **Accepted:**
  - **Until the next run, they stay.**
  - **Deletes reach only the terminals showing the running pane.** A copy in a terminal attached
    only to another session stays, and its record goes anyway.
  - **Older records drop past 2048 or with the tmux server,** so those images stay until kitty's
    quota or a closed tab frees them.
  - **Each run inside tmux spends a few `tmux` commands on it, plus two captures per recorded
    pane.**
  - **Two runs at once can lose a record.** That leaks an image, as before this decision, but
    never deletes one still shown.
- **Revisit when:** tmux tells programs which placeholder cells it drops, or the terminal can be
  asked which images it holds.
