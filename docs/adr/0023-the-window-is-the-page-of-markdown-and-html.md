# ADR-0023: The window is the page of Markdown and HTML

- **Status:** Accepted
- **Date:** 2026-11-09

## Context

Markdown and HTML give no page size, so clv laid them out on US Letter, and since
[ADR-0022](0022-markdown-and-html-reflow-to-a-width-in-characters-as-a-new-level.md) on pages as tall
as Letter at any width: a page fitted to the window left most of a wide window empty and showed small
text in a short one, and the reader zoomed to make it out, scrolling a page taller than the window. A
viewer on a screen has no reason to keep a printer's page
([Spec-0003](../specs/0003-lay-markdown-and-html-out-to-the-window.md)). An e-reader lays its text out
to its screen, and sets the size of the type; that is what a terminal pager should do.

The viewer knows nothing of documents (CONTRIBUTING's rule "The core never depends on an extension"),
and a document knows nothing of the terminal, so the window has to pass between them in the contract.
And `fit.rs` holds a page to its size in one way only: a natural page (an image's pixels) is never
enlarged past a scale of 1.

## Decision

- **The window is the page of a document that has none.** A Markdown or HTML page is always as tall as
  the pages area (the window less the status line), and at the default width as wide as it, or where
  the window holds two pages of `BOOK_LEAST` (60) characters, half of it less the gap, two side by side
  as an open book. `width` sets only the width, in characters of the code face, within the window; the
  height is the window's. US Letter goes, and `page` with it; `window` takes its place as the default.
- **What the backend knows of the window is a `Room`** (`source.rs`): the pages area's columns and
  rows, a cell's size in pixels as `f32`, exactly the cell the grid is fitted with (a text cell's
  sampling cell included), and the gap between two pages. It is handed to `Backend::open_at` (a default
  method calling `open`, so no other backend changes) and to `Source::relayout`, and said back by
  `Source::laid_at` exactly as given, so that the pager's comparison holds. Rooms are compared through
  `Room::same`, with a 2% give in the cell rather than rounded: a window dragged a few pixels lays
  nothing out again, a font change does, and a sampling cell keeps its shape.
- **The page is in points such that a character of the code face is a cell on screen** when the page
  is shown as tall as the room: a column of `CODE_SIZE` × `CODE_ADVANCE` = 5.64 pt, a row 5.64 × the
  cell's shape. Margins of two columns at each side and a row above and below; a page too small for
  20 characters a line and 10 lines of text (`LEAST_COLUMNS`, `LEAST_LINES`) loses a column of each
  side margin, then grows past the room and is shown smaller, rather than refused.
- **One largest scale, not an exact one.** `Source::largest_scale`, a default method answering `None`,
  gives the scale at which a page is as tall as its room; `fit::layout` holds a page to it at
  `Fit::Page` as it holds a natural page to 1.0, and at `Fit::Width` and N to a row the page is any
  page. So a page is never shown larger than its font size, but the fit may show it smaller: `auto`
  puts two pages to a row where each keeps 80% of its size alone (`TWO_UP_SHARE`), so a width between
  about half and five eighths of the window shows two pages at 80 to 100% of the size asked, while
  the status line says the font size. At `window` it never happens: a book page is laid out to fit two
  at 100%. The alternative, an `Exact` kind of size shown one to one and never shrunk to share a row,
  would have made `natural` an enum in every source and had the fit and the oversampling branch on it;
  should the shrinking prove a nuisance, the fix is one condition, two to a row only at
  `largest_scale` for a page that has one.
- **Where a setting's values sit is the document's to say.** `window` sits among the widths at the
  characters its page holds in this room, and the most a width is laid out at is the longest line the
  room holds: `Source::bounds`, asked when a level is made, in place of a static `Named::at`. A width
  past the most (typed, given, or left so by a smaller window) is kept, laid out at the most, said as
  `width 120 (window)`, and grows back when the room does. A count of 0 goes where the setting says
  (`Setting::zero`): `0]` is `window`.
- **Files open as they did, before any question and before raw mode**, at the room the tty says
  without asking: its columns and rows, its cell from the pixels it reports, or tmux's, or the one
  assumed. Asking first was rejected: opening in raw mode, or in a mode between, leaves the terminal
  altered when Ctrl-C ends a long layout. A terminal that reports no pixels pays a second layout once
  its cell is known.
- **A list's documents are previews**, laid out for a fixed portrait room (`PREVIEW`, 60 × 40 cells at
  the cell the run started with), as tiles are square and a landscape page the window's shape would be
  a strip of small text in one. A document gone into is laid out for the pager's room, through
  ADR-0022's relayout, by its values and its room together. Printing with no terminal lays out for
  the preview's room too.

## Evidence

The page: `cargo test -j 2 --bin clv the_page_is_the_room_and_its_width_so_many_characters_of_the_code_face`
(80 × 23 cells of 10 × 20 px make a 451.2 × 259.44 pt page of 76 columns; 132 columns hold a book
of 61-character pages and 128 don't; a tiny room keeps 20 × 10 of text), and
`a_page_laid_out_for_a_room_is_never_shown_larger_than_its_font_size` (`fit.rs`: held to the
largest scale alone and beside another, not at the width). On a pseudo-terminal
(`tests/it/view_pager.rs`, `the_width_keys_lay_the_document_out_again_and_the_status_line_says_what_it_is_at`),
a page fills the 100 × 59 pages area, 1000 × 1180 pixels.

The cost at the extremes, the files ADR-0012 made to cost the most within the limits (499,900 marks
each), written again by a script, printed with `/usr/bin/time -l clv FILE --protocol kitty --pages 1
--width 20` on an 80 × 24 pseudo-terminal from script(1) (`stty rows 24 cols 80`), a release build,
Apple M3 Max, macOS 26.6.2, wordcraft at `bbe3547`:

| File | `--width window` (76 characters) | `--width 20` |
|---|---|---|
| `<ul>`, then the `<li>` sharing out 8 MiB of text (9.5 MiB) | 13.8 s, 5.9 GB | 14.4 s, 5.8 GB |
| `<ul>`, then `<li>a` (2.4 MiB) | 11.0 s, 4.8 GB | 10.9 s, 4.8 GB |
| `<p>` each with a page break (16 MiB) | 6.7 s, 3.9 GB | 6.7 s, 3.9 GB |
| `<p>a` (1.9 MiB) | 5.7 s, 3.5 GB | 5.7 s, 3.5 GB |
| lines in a code block, Markdown (0.95 MiB) | 7.0 s, 3.9 GB | 7.0 s, 3.9 GB |
| 8 MiB of text in one paragraph | 2.7 s, 1.5 GB | 3.6 s, 1.5 GB |

The same as ADR-0022's Letter page at either width, within noise: importing is what costs, and a page
80 columns wide and 23 rows tall, 20 characters a line, changes nothing. The least a page holds stays
at 20 × 10.

## Alternatives considered

- **US Letter, fitted to the window** (ADR-0022): small text in a short window, an empty wide one.
- **A page as tall as Letter at any width** (ADR-0022's `width`): the reader still zoomed and scrolled;
  a page the window's height has nothing to scroll.
- **An `Exact` scale** beside `natural`: above, under *One largest scale*.
- **Asking the terminal before opening**, so that the first layout has the cell a terminal without
  pixels reports: the terminal in raw mode, or a mode between, while a long layout runs, altered when
  Ctrl-C ends it.
- **Laying a list's documents out for the window**: a landscape page in a square tile, a strip of small
  text; and every document's tile laid out again on each change of size.
- **Rounding the room's cell** to whole pixels before comparing: a text cell's sampling cell (1.25 × 2)
  would change shape.

## Consequences

- **Positive:** a Markdown or HTML page is the screen, text at a size for reading in a window of any
  shape, two pages where the window is wide; `[` and `]` still choose the length of a line.
- **Accepted:** a page is never larger than its font size but may be shown smaller, two to a row (the
  known limitation above); a list's tiles and a document's pages are laid out for different rooms, so
  going into a document lays it out again; a page printed to a file is the terminal's shape at the
  time.
- **Follow-ups:** the font size on `+` and `-`, and laying out again when the window changes size
  (Spec-0003's later tasks); a configuration file for `BOOK_LEAST` and `PREVIEW`; the width and the
  font size kept between runs.
- **Revisit when:** the two-up shrinking proves a nuisance (one condition in `fit::grid`), or a
  terminal reports its cell wrong often enough that the room's shape should be assumed instead.
