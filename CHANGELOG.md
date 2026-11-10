# Changelog

What changed for someone using `clv`, newest first. The [manual](docs/usage.md) says how each thing
works today. A version's section becomes the notes of its
[release](https://github.com/dobladez/clview/releases).

## Unreleased

## 0.4.0 (2026-11-10)

### Added

- An SVG drawing's HTML labels (`<foreignObject>`, which tldraw, draw.io and Mermaid label their
  drawings with) are drawn, by Blitz, over the drawing, in the drawing's styles and the fonts it
  embeds; draw.io's fallbacks ("[Not supported by viewer]", "Text is not SVG - cannot display")
  aren't. A drawing showed them without their text, or with those fallbacks. Blitz runs in a
  process of its own, held to its time and memory: past either, the drawing's labels are left out
  with a warning and the drawing drawn without them; what the labels may cost before it is
  bounded, and a label past it left out with a warning. A list's tiles show drawings without
  their labels; opening one shows them. The labels' system fonts are the ones the drawing's own
  text has, found the same way on every system. A label whose text sits directly in a
  `display: table` box (Mermaid's edge labels) is drawn, which Blitz alone would leave blank. A
  build without the `svg-html` feature says how many labels it didn't draw.

- Markdown and HTML pages are the window's size, as on an e-reader: as tall as the pages area and as
  wide, or two side by side like an open book where the window holds two pages of 60 characters, a
  character of code to a cell. They were US Letter pages fitted to the window. A list's tiles are
  previews on a portrait page. When the window changes size, the document is laid out again for it
  once the size has held for half a second, the status line saying `laying out…` meanwhile; inside
  tmux, the pane is the window.
- Markdown and HTML have a font size: `+` and `-` make the text 10% larger or smaller, from 50% to
  400%, laying the document out again on pages still the window's size, `150+` to 150%, `0+` back to
  100%; `--font N` says where to start. On these files the keys are the font size, not the zoom, which
  a page the screen's size has no use for; everywhere else they zoom as before, and `?` says which. The
  status line shows the size when it isn't 100%.
- Markdown and HTML reflow to a width: `[` and `]` lay the document out again 4 characters a line
  narrower or wider, `72]` to 72, `0]` back to the window's, from 20 up to the longest line the window
  holds, with `window` among the steps; `--width window|N` says where to start, and a width the window
  doesn't hold is kept and laid out at the window's. The paragraph at the top of the screen stays on
  the page you land on. The status line shows the width when it isn't the window's. Only the document
  on screen reflows: a list's tiles keep their layout, and a document opened from the list after a
  change is laid out at the new width.
- `,` and `.` set the pages to a row on any file, as `-` and `+` zoom: one more, one fewer, then the
  width; `3.` sets three, as many as the window takes, and `0.` goes back to `auto`. In a list they set
  the tiles to a row, `.` stopping at one. They are the pages to a row on a Markdown or HTML file too,
  where `+` and `-` are its font size.
- Markdown and HTML in columns: `3.` (or `--per-row 3`) lays the document out again on three pages
  side by side, each a third of the window wide and as tall, the text at its font size; `.` stops at
  one. A count and a width you type both say how wide a page is, so the last you set wins: `3.` puts the
  width back to `window`, `72]` the count back to `auto`. `--width 72` with `--per-row 3` lays out at
  the width, with a warning. Printed, `--per-row 3` prints three-column pages.
- Markdown and HTML margins: `}` and `{` widen or narrow the side margins a character at a time, from 0
  to 20 characters of code, `5}` to 5, `0}` back to 3; `--margin N` says where to start. At the window's
  width they shorten the line; at a width of your own the page grows by them, and a margin wider than
  the page takes is laid out at the most it does, the text at its size (`margin 12 (6)`). The status
  line shows the margin when it isn't 3, and a font size laid out smaller than asked, 100% too, as
  `font 100% (80%)`.
- Homebrew, on macOS or Linux: `brew install dobladez/tap/clv`, and `brew upgrade clv` for each new
  version. No `xattr` step on macOS.
- A `clv` built from source between releases says which commit in `--help` and `--version`:
  `0.3.0+50.g3f677cb` is 50 commits past 0.3.0, at commit `3f677cb`, with `.dirty` after it when built
  with changes not committed. A release still says `0.3.0`.

### Changed

- A number being typed fills the status line with the keys that take it on this file, `7: g page · .
  per row`, and on a Markdown or HTML file `+ font · ] width · } margin` too; in a list `Enter tile`. It
  said `go to page 7`, though `,` `.` and the documents' keys take the number as well.
- Markdown and HTML side margins are 3 characters of code, from 2: a window 100 columns wide puts 94
  characters on a line, and a book of two pages needs 134 columns.
- `--zoom` is now `--per-row auto|width|1-8`, since it sets a list's tiles to a row too. `--zoom` is
  still taken, with a warning, and will go in a later release.
- `--help` lists `--theme` among the documents' options, as it comes with them; it is still taken
  with any file. A `clv` built without documents has no `--theme` and no `d`.

## 0.3.0 (2026-11-04)

### Changed

- A list shows at once, however long it is. A PDF, a document, a drawing or a spreadsheet is opened
  only when its tile nears the screen, and the keys keep working meanwhile. 518 Markdown files took
  about 4 s to show anything, and now take about 0.1 s on an M3 Max.
- Enter on a file still opening says `opening notes.md…` until it can go in, and Esc stops waiting.
- A PDF or a document that turns out not to open keeps its tile, marked `! name`, and says why when
  chosen, so nothing moves under you. It used to be left out, with a warning before anything showed;
  one never reached now says nothing. A broken picture is still left out with a warning.

## 0.2.0 (2026-11-03)

### Added

- Markdown and HTML, laid out into pages, with a look of their own. An epub's chapters open from its
  list. The pictures they refer to show, each drawn as clv would show it; nothing is fetched, so a
  picture at a URL shows as its alt text.
- Dark pages: Markdown and HTML start light on dark on a dark terminal, on the terminal's own colour.
  `d` turns them, and `--theme auto|light|dark` says how they start.
- Terminals without the graphics protocol: pages in text cells, with a warning, where `clv` used to stop
  with an error. `--protocol blocks` or `--protocol ascii` ask for them anywhere, a pipe too.

### Changed

- Newer versions of the PDF, Word and spreadsheet renderers.

## 0.1.1 (2026-11-02)

### Changed

- `--help` shows an icon beside its first lines, heads its options, and ends with a link to the manual.
  The keys leave it: `?` in the pager and the manual list them.

## 0.1.0 (2026-10-31)

The first release: PDFs, images, SVG drawings, Word documents, spreadsheets, and zip and tar archives,
through the kitty graphics protocol in kitty, Ghostty and WezTerm, inside tmux too. A pager over their
pages, and lists of tiles for several files, a directory or an archive.
