# ADR-0011: Text cells where the terminal has no graphics protocol

- **Status:** Accepted
- **Date:** 2026-11-02

## Context

`clv` stopped with an error on a terminal that doesn't speak the kitty graphics protocol, and through
a pipe unless told otherwise. Every backend already gives the viewer the same thing, a `Frame` of
pixels, so all that was missing was another way to put pixels on screen. Sixel, the other way, is
further off ([What sixel would take](../architecture.md#what-sixel-would-take)) and reaches fewer
terminals than text does.

A cell holds very little, so this can only be a preview. What it is
([Text cells](../usage.md#text-cells)) and how it works
([Text cells](../architecture.md#text-cells-cellsrs)) are in those docs. This records five choices a
later reader might otherwise undo.

## Decision

1. **`blocks` first, `ascii` beside it, both written here.** `--protocol blocks` is half-blocks (`▀`)
   with their two colours, and what `auto` falls back to. `--protocol ascii` is a ramp of ten
   characters, for a terminal with odd fonts or double-width blocks, and for plain text. The
   conversion is `src/view/cells.rs`, a little over 300 lines without its tests, with no dependency.
2. **`auto` falls back rather than stopping**, with a warning, when a terminal doesn't speak the
   protocol. It still stops in two cases, where going on would hide something the reader wants to
   know: a pane without `allow-passthrough` on a terminal that takes images, where one command gets
   them; and output to a file or a pipe, where nobody asked for escape codes in a file.
3. **A page renders for its samples, not for the screen.** A picture renders at a pixel for each
   sample, its natural size being a column for each pixel. A page with no natural size renders at
   four pixels a side for each sample, and is averaged down.
4. **The pager writes only the rows that changed**, comparing each with what it last wrote.
5. **Only what depends on the background asks about it**, with OSC 11: `ascii`, and `blocks` without
   colours. A terminal that doesn't say is taken to be dark.

## Evidence

Measured on macOS 26.6, Rust 1.95, kitty 0.49.2 and tmux 3.5a.

**How much to oversample.** Through a pipe at 80 columns, `TERM=xterm-256color COLORTERM=truecolor`,
with `cells::OVERSAMPLE` as in the table, and the output rasterized to look at it:

```sh
clv page.pdf --protocol blocks --no-pager --pages 1 --zoom width > out.txt
```

| Pixels a side per sample | A full-page photograph in a PDF | A page of text | Whole run |
|---|---|---|---|
| 1 | speckled: the PDF renderer point-samples pictures at so small a scale | a texture of lines | 54–111 ms |
| 2 | less so | the same | 54–86 ms |
| 4 | clean | the same | 56–81 ms |
| 8 | no different from 4 | the same | 56–82 ms |

The times are of one run each, and vary more from run to run than with the oversampling. They are
dominated by opening the file. A page of text wasn't legible at any of them, which is what the docs
say of documents.

Pictures aren't oversampled: the image backend's own resize (a triangle filter going down) was
smooth at a pixel per sample, and rendering more of a small picture than it has adds nothing.

**What a full window takes.** Printed to a file on a pseudo-terminal of 200 × 60, the first page or
picture, with `--protocol blocks` unless the table says otherwise:

| Page | Colours | Bytes | Rows |
|---|---|---|---|
| A dark photograph | 24-bit | 90,818 | 59 |
| The same | 256 | 27,840 | 59 |
| The same, `--protocol ascii` | 24-bit | 4,720 | 59 |
| A page of text, the whole page | 24-bit | 62,830 | 59 |
| A page of text, `--zoom width` | 24-bit | 256,002 | 142 |
| A synthetic picture of gradients, `--zoom width` | 24-bit | 332,610 | 67 |

A cell takes 39 bytes at the very most, so a screen of noise at that size would come to about 460 KB
(an estimate, from 200 × 59 cells). That is why a key that changes nothing shouldn't write a page
again, and why moving through a list writes only the row of names. `tests/it/view_pager.rs` checks
both.

**A real terminal.** In a hidden kitty (`--start-as=hidden --config NONE`), driven with `kitten @
send-text` and read with `kitten @ get-text --ansi`: the pager's rows are on its screen as written,
the page turns, the screen comes back empty after `q`, and with `-o background=white` it answers
OSC 11 and `--protocol ascii` draws dark on light. No other terminal has been tried by hand: a
terminal without the protocol is played by `tests/it/view_pager.rs` on a pseudo-terminal.

## Alternatives considered

- **Stopping with the error, as before, and text cells only by name**: the reader who needs it most
  is the one who doesn't know the option exists. The warning says what happened and how to ask for
  it without the warning.
- **Falling back in every case, passthrough off too**: tmux's users would get a blurry preview for
  good, and never learn that one setting gives them the pages.
- **Quadrants, sextants or Braille, for more samples to a cell**: each cell of these has only two
  colours for its several samples, so a photograph gains edges and loses colours, and sextants and
  Braille need fonts that have them. Not tried; they could be other `--protocol` values later.
- **A library** (chafa, viuer and so on): the clean-room rule bars reading the source of chafa and
  viu, chafa is C and LGPL where this code is Rust and MIT or Apache-2.0, and what is needed here is
  small.
- **Asking every terminal its background**: half-blocks colour both halves of a cell, so the answer
  would change nothing, and it is one more question for a terminal that may show it.
- **Rendering at the terminal's own pixels and averaging**: as good as oversampling, and tens of
  times the pixels for nothing: a 10 × 20 cell is 200 pixels for its two samples, against the 32 of
  oversampling.
- **A diff of cells, not rows**: fewer bytes for a page that shifts a little, but no page does:
  scrolling changes every row's content, and a row is what the pager already has at hand.

## Consequences

- **Positive:** `clv` shows something in any terminal, through tmux with no passthrough, and to a
  file. A second output with no ids in the terminal is what sixel will need, so `Output::Text` has
  already shown where that goes in the pager.
- **Accepted:** a preview isn't what `clv` is for, and body text can't be read in it. The warning
  comes on every run of `auto`, until `--protocol blocks` asks for text by name. A light terminal
  that doesn't answer OSC 11 gets a negative from `ascii`. `▀` is East Asian Ambiguous, so a terminal
  set up to draw those double-width breaks a page up, which `--protocol ascii` is the way around.
- **Follow-ups:** none needed. Sixel remains in the README's plans.
- **Revisit when:** a terminal that many readers use turns out to show these badly; another PDF
  backend doesn't point-sample its pictures, when oversampling could go; or a page turn over a slow
  link proves too slow, when a smaller colour depth could be chosen for SSH.
