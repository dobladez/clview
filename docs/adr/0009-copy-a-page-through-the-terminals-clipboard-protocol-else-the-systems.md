# ADR-0009: Copy a page through the terminal's clipboard protocol, else the system's clipboard

- **Status:** Accepted
- **Date:** 2026-10-31

## Context

The pager's `c` copies the picture the reader is on to the clipboard: the page, or in a list the
tile chosen. A picture has to reach the clipboard of the machine the reader sits at, which is
not the one `clv` runs on over SSH, and the way there may go through tmux. Three ways exist:

- **OSC 52**, which xterm defined and most terminals take: it carries text, base64 encoded.
- **kitty's clipboard protocol**, OSC 5522 ([spec](https://sw.kovidgoyal.net/kitty/clipboard/)):
  any MIME type (`image/png`), in packets of at most 4096 bytes before base64, each its own
  escape code; the terminal answers the end with `type=write:status=DONE`, or an error (`EPERM`,
  `EBUSY`, `EFBIG`…), echoing the write's `id`. DECRQM for mode 5522 (`CSI ? 5522 $ p`) asks
  whether a terminal speaks it.
- **The system's clipboard** (the macOS pasteboard, X11, Wayland), written by `clv` itself: only
  ever this machine's.

The terminal's answers arrive among the keys, where `ESC ]` (an OSC answer's start) used to
decode as Esc, which quits.

## Decision

- Ask the terminal once, when the reader first presses `c`, with DECRQM for mode 5522 (inside
  tmux, through passthrough). The page renders on the pool meanwhile.
- A terminal that speaks it gets the page as `image/png` through OSC 5522, wherever it is: over
  SSH, and inside tmux each packet in a passthrough sequence of its own. `clv` says "copied" on
  its `DONE`, or why not on an error.
- One that doesn't (it answers 0 or 4, or nothing within a second) gets nothing through the
  terminal: the page goes to the system's clipboard through
  [arboard](https://crates.io/crates/arboard), when the terminal runs on this machine (no SSH
  variables; inside tmux, how its client attached). Otherwise the status line says why there is
  no copy.
- A picture is copied at its own pixels; anything else at twice its unit (144 per inch for a
  PDF's points, twice an SVG's CSS pixels), within the limits a page on screen has (8192 pixels a
  side, 16 megapixels): a US Letter page is 1224 × 1584.
- `keys.rs` consumes OSC answers (`ESC ]` and a digit, through `ESC \` or BEL, up to 512 bytes,
  cut across reads), and DECRQM answers, as it does graphics answers: never keys.

## Evidence

Measured on 2026-10-31, macOS 26.6.2, with kitty 0.49.2 and Ghostty 1.3.1 as installed, tmux 3.5a,
each hidden or in a fresh window with no configuration of the user's (`kitty --start-as=hidden
--config NONE`, `ghostty --config-default-files=false`), tmux as a private server
(`tmux -L <name> -f <file>` with `set -g allow-passthrough on`). WezTerm wasn't installed: its
[escape sequence list](https://wezterm.org/escape-sequences.html) has OSC 52 (set only) and no
OSC 5522; unverified.

In the terminal under test, raw mode, then the question and the answer:

```sh
stty raw -echo min 0 time 10
printf '\033[?5522$p\033[c'; dd bs=256 count=1 2>/dev/null | od -c
# inside tmux: printf '\033Ptmux;\033\033[?5522$p\033\\\033[c'
```

A write of a PNG under 4096 bytes (one packet), and the answer, then the clipboard from outside:

```sh
d=$(base64 -i small.png); m=$(printf image/png | base64)
printf '\033]5522;type=write:id=t\033\\\033]5522;type=wdata:mime=%s:id=t;%s\033\\\033]5522;type=wdata:id=t\033\\' "$m" "$d"
dd bs=256 count=1 2>/dev/null | od -c
osascript -e 'clipboard info'
```

(The probe that measured the table wrote PNGs of 4,908 to 5,402 bytes, two packets each, a CSI
between packets, a size each, so `clipboard info` tells which write landed.)

| Terminal | DECRQM 5522 answer | Answer to the write | Clipboard after |
|---|---|---|---|
| kitty 0.49.2 | `CSI ? 5522 ; 2 $ y` | `OSC 5522 ; type=write:status=DONE:id=… ST` | `«class PNGf», 4908`: the PNG, byte for byte in size |
| kitty 0.49.2 inside tmux 3.5a | the same, through passthrough, back in the pane (after tmux's own DA answer) | the same, back in the pane | `«class PNGf», 5152` |
| Ghostty 1.3.1 | `CSI ? 5522 ; 0 $ y`: an unknown mode | none | unchanged |

With no configuration, kitty asked nothing before writing (its `clipboard_control` setting decides).

`clv` itself, driven through kitty's remote control (`kitty -o allow_remote_control=yes
--listen-on unix:$TMPDIR/k.sock clv doc.pdf`, then `kitty @ --to unix:… send-text nc` and
`get-text`), and inside Ghostty through `tmux -L … send-keys`:

| Where | What was copied | Status line | Clipboard |
|---|---|---|---|
| kitty, directly | page 3 of a US Letter PDF | `copied page 3, 1224 × 1584 pixels` | `«class PNGf», 51215`, a 1224 × 1584 PNG of the page |
| kitty, inside tmux | a 4000 × 3000 PNG of random noise (36,014,013 bytes, the worst a PNG gets) | `copied page 1, 4000 × 3000 pixels` (a source of one page is named now: `copied noise.png, …`) | `«class PNGf», 36005813`; converted to BMP with `sips`, byte for byte the source's pixels: nothing lost in some 8,800 passthrough sequences |
| Ghostty, inside tmux | page 1 of the same PDF | `copied page 1, 1224 × 1584 pixels` | `TIFF picture, 7755962` first, then the types macOS derives (`«class PNGf»`…): arboard's NSImage |

These overwrote the clipboard of the machine they ran on, five times.

## Alternatives considered

- **OSC 52 alone.** Text only: a PNG's bytes would arrive as text, if at all. No terminal
  observed or documented takes a picture through it.
- **The system's clipboard alone.** It never reaches the reader's machine over SSH, nor a tmux
  session attached from elsewhere, which OSC 5522 does as the pages do. Kept for the terminals
  without the protocol, on this machine.
- **Both for every copy.** Writing the system's clipboard as well as the terminal's would race
  them on the same machine, and over SSH write the wrong machine's.
- **`osascript`, `wl-copy` or `xclip` run as programs.** No crate, and `wl-copy` and `xclip` keep
  serving a picture after `clv` ends, where arboard on X11 can't. But each is a tool that may be
  missing, another program's interface to track, and AppleScript is code in another language
  (the project is Rust only).
- **Asking at start-up**, as for the cell size. Every run would ask, for a key most never press;
  asked lazily, the first copy on a terminal that never answers waits up to a second.
- **The scale the page is shown at.** It depends on the window and the zoom: a tile in a list of
  48 would copy as a thumbnail. **A fixed 300 per inch** made US Letter 2550 × 3300 (8.4 MP),
  about four times the bytes of 144 per inch, for every copy over a slow link too.
- **JPEG.** Smaller for photographs, but lossy, and the viewer has no JPEG encoder outside the
  `image` backend; PNG is what every application pastes.

## Consequences

- **Positive:** `c` works in kitty locally, over SSH and through tmux, confirmed by the terminal;
  in Ghostty and WezTerm on this machine through the system's clipboard; over SSH without the
  protocol, the reader is told why not. The pager never waits for a copy: the PNG is made on a
  thread, the packets go 128 KiB a round with the keys read between.
- **Accepted:** a new dependency, arboard (`image-data` only), built even without backends: on
  macOS the objc2 crates, on Linux and FreeBSD x11rb, and the `image` crate with its TIFF or PNG
  encoder. On X11 a picture stays on the clipboard only while `clv` runs, unless a clipboard
  manager takes it (`clv` waits half a second at the end for arboard to hand it over). Wayland's
  data-control protocol isn't enabled, so a Wayland session without XWayland has no system
  clipboard for `clv`. A terminal answering the protocol for one tmux client may be another by
  the next copy; a write to one that doesn't speak it goes unanswered, and the reader is told
  after five seconds.
- **Follow-ups:** not tried: Linux, FreeBSD, a real SSH connection, WezTerm.
- **Revisit when:** Ghostty or WezTerm speak OSC 5522 (they take the terminal's way with no change
  here); tmux handles OSC 5522 itself; a Linux user needs the picture to outlive `clv`, or a
  Wayland session without XWayland; readers ask for another resolution.
