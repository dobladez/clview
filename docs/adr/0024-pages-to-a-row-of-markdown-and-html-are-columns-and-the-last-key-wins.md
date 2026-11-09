# ADR-0024: Pages to a row of Markdown and HTML are columns, and the last key wins

- **Status:** Accepted
- **Date:** 2026-11-09

## Context

Since [ADR-0023](0023-the-window-is-the-page-of-markdown-and-html.md), `+` and `-` on Markdown and
HTML are the font size, and the reader lost the control they have everywhere else: how many pages go
to a row. [Spec-0005](../specs/0005-set-pages-to-a-row-and-margins.md) gives that control keys of its
own, `,` and `.`, on every file, and `--per-row` for `--zoom`. On a PDF a count to a row is a size: N
pages shrunk into the window's width. On a document laid out for the window, whose page is the window,
it could mean that too, or something a reader of a reflowed page has more use for.

A count to a row and a typed width (`width`, `[` `]`) both set how wide a reflowed page is, so they
can disagree: three columns of a 100-column window hold 32 characters each, and `72]` asks for 72.

And a page's width, its font size and its side margins bound one another: the most each can be
depends on the others, and the room.

## Decision

- **Columns.** On Markdown and HTML shown by itself (not in a list, not with `--pages`), N to a row
  lays the document out again on N pages side by side, each as tall as the window and as wide as the
  window's columns, less the gaps between them, shared N ways, at the font size. The viewer says the
  count through the `Room` it already gives (`Room::per_row`, `None` at `auto`, compared by
  `Room::same`), so the core still names no backend, and a count is laid out for at once, as a change
  of `width` or `font` is; only a change of the window's size waits for it to hold. `.` stops at one to
  a row: a reflowed page is never wider than the window, so the width has nothing to show there, and
  `--per-row width` is one to a row.
- **The last key wins** between a count and a typed width. A count (`,`, `.`, `N.`) sets `width` back
  to `window`; a key that types a width (`72]`, `[`, `]`) sets the count back to `auto` first, and steps
  from the bounds of the room at `auto`. `0.` and `0]` change only their own. Which setting a count
  undoes is the backend's to say (`Setting::sets_width`), not a name the viewer knows. On the command
  line, `--width N` with a count wins, with a warning, since there is no order to go by.
- **The bounds, in order**, each worked out from the values asked: the page's width in cells (the
  room, the count and `width`); then `font`'s most, the largest at which that page, a column, holds
  20 characters a line within the margin asked; then `margin`'s most, the largest at which a line
  holds 20 characters at the font laid out at. A value past its most is kept and laid out at the most.
  So a margin wider than a narrow page takes lowers the font laid out at first, said on the status line
  (`font 100% (80%)`), and binds at the least size; in a room where even no margin leaves 20
  characters, the margin is `min(asked, 1)`.

## Evidence

No measurement: a choice of what the keys mean. What it rests on is checked by
`cargo test -j 2 --test it view_pager` (`3.`, `3.72]`, `3.[`, `0.`, a failed layout restoring both)
and `cargo test -j 2 --bin clv wordcraft` (`page_in` at a count, and each bound).

## Alternatives considered

- **An overview, as on a PDF**: the window's pages shrunk N to a row. Their text shrinks with them,
  which on a page laid out at a font size undoes the font size; to see more of a document at once,
  narrower columns at the same size show more text, readable. Left for a later spec, if wanted.
- **A count and a width both held**, the width within a column: a width typed for one column would
  be cut by the next count, and a count would be undone by a width wider than a column, each key
  silently overruled by the other. One wins, the last, so what was pressed is what is shown.
- **Settings past their most refused**, as a step past the most is: a font size typed for a wide
  window would be lost on narrowing it, or on a count. Kept and laid out at the most, as ADR-0023 keeps
  a width past the window's.

## Consequences

- **Positive:** the reader sets the pages to a row on every file with the same keys, and on a
  reflowed document gets readable columns at the font size.
- **Accepted:** `,` and `.` mean a size on a PDF and a layout on Markdown; `?` and the manual say so.
  A count resets a typed width, and a typed width a count.
- **Follow-ups:** none.
- **Revisit when:** readers ask for an overview of a reflowed document, or for a scrolling page wider
  than the window.
