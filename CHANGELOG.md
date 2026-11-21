# Changelog

What changed for someone using `clv`, newest first. The [manual](docs/usage.md) says how each thing
works today. A version's section becomes the notes of its
[release](https://github.com/dobladez/clview/releases).

## Unreleased

## 0.6.0 (2026-11-20)

### Added

- clv shows an ebook (epub) as one book, with its chapters in reading order and their pictures.
  The cover is the first page, and a Contents page is the last page. `T` goes to the contents,
  and `f` follows the links between chapters. Before this release, clv showed an epub as a list of
  its files.
- clv shows Org-mode files (`.org`) as documents, as it shows Markdown. It shows TODO keywords in
  color, tables, footnotes, pictures and Mermaid blocks. `f` follows Org links to headings, to
  other Org files and to the web. clv does not run code or `elisp:` links.
- A Markdown or HTML document in a zip or a tar shows the pictures that are next to it in the
  archive. Before this release, clv showed only their alt text.

### Changed

- A key that has no effect on the file, such as `D` on a PDF, gives a shorter message. The
  message uses the full status line, so a narrow window does not cut it.
- `?` shows the keys that have no effect on the file in faint text. In a narrow window, a long
  line continues on the next row.

### Fixed

- clv shows a zip as a zip when its first file is a PDF without compression. Before this release,
  clv tried to open it as a PDF and gave an error.

## 0.5.0 (2026-11-16)

### Added

- `f` in a file's pages labels every link on screen with a short code of home-row letters, as the
  Vimium browser extension does; type a code and its link is followed, `Esc` drops the labels. A
  link to a place in the document goes there: a PDF's table of contents or named destination, a
  Word document's cross-reference, Markdown's `[see](#usage)`, which finds `## Usage` by the id
  GitHub gives it, HTML's `#name`, which finds any element with that `id`. `Backspace` comes
  back, through the last 100 links followed, before it goes back
  to the list. A link to another file (`[guide](docs/guide.md#usage)`, a PDF's link to another
  PDF) opens it in `clv`, at the place it names, and `Backspace` comes back to the document; only a regular
  file or a folder is opened. A web address (`http`, `https` and `mailto` only, checked for hidden
  characters, and never a `mailto` that would attach a file) opens in your browser or mail app,
  the status line naming it; over SSH it is copied to your clipboard instead. `F` labels the links
  and copies the address of the one chosen. A link that would run a script or start a program is
  never followed, and says what it is. The labels are drawn over the page, which is never rendered again for them: at their
  links' own places directly in kitty, Ghostty and WezTerm, and as capitals over the page's cells
  inside tmux and in text cells.
- `o` opens the file you are reading in the app the desktop opens it with (`open` on macOS,
  `xdg-open` on Linux and the BSDs), without waiting for it, the status line saying how it went;
  in a list, the tile chosen, a folder in the file manager. A file whose real name doesn't say what
  `clv` showed it as isn't opened, nor one with no file of its own (in an archive, through a pipe or
  `/dev/stdin`), nor anything over SSH.
- A Mermaid diagram shown by itself follows the paper, as Markdown does: on a terminal with a dark
  background it is drawn in Mermaid's dark theme on the terminal's own colour, where it was a white
  rectangle; `D` turns it, in a list too, and `--theme auto|light|dark` says how it starts.
  `--help` lists `--theme` under Mermaid diagrams too.
- `/` in a file's pages searches its text: a PDF's, a spreadsheet's, through the PDF it prints
  to, and a Word document's, an OpenDocument text's, an RTF's, Markdown's and HTML's, their body a
  paragraph at a time, so that a word hyphenated at a line's end, or a paragraph running onto the
  next page, is found whole; and their search outlives their being laid out again. The status line counts the matches as you type (`/word 17…`), `Enter` goes to the first from
  the top of the screen and keeps the search, `n` goes to the next match and `N` or `p` to the one
  before (`3n` three on, `3p` three back), and `Esc` clears it. The matches on screen are tinted, the one you are on in
  another colour. The text is read when `/` is pressed, in a process of its
  own held to its time and memory, so the keys never wait on it and a page made to cost too much
  is left unread, the status line saying which. `/` only said that it filters lists. A picture or
  a drawing says it has no text to search.
- Mermaid diagrams (`.mmd`, `.mermaid`): each one page, drawn by merman as Mermaid 12 draws it, on
  white and as sharp as a drawing. A `.mmd` that is MultiMarkdown is left alone. merman runs in a process of its
  own held to its time and memory, and a diagram is drawn once however often it is opened: a list's
  tile, then its page, or a Markdown or HTML document's picture (`![a flow](flow.mmd)`).
- A ` ```mermaid ` block in Markdown shows as its diagram, as on GitHub, where it showed as code,
  and so does HTML's `<pre>` that says it is Mermaid's (`<pre><code class="language-mermaid">`,
  GitHub's `<pre lang="mermaid">`, pandoc's `<pre class="mermaid">`), an epub's chapters included:
  drawn as a document's pictures are, and through a pipe or from an archive too, since its text
  comes with the document. A block that can't be drawn stays code, and the first page says how
  many did. `clv --help` lists the languages drawn, beside each format's extensions; a block in any
  other stays code, ` ```svg ` included. `s` shows a document's blocks as code, to read or search a
  diagram's source, and again draws them; `--fences code` starts that way. On dark paper a block is
  drawn in Mermaid's dark theme, on the paper's colour, and `D` turns it with the page; so is a
  diagram a document shows by path (`![a flow](flow.mmd)`).
- Emacs's keys beside `less`'s and `vi`'s: `Ctrl-V` and `Alt-V` go a screen down and up,
  `Ctrl-N` and `Ctrl-P` are `j` and `k`, and in a list `Ctrl-F`, `Ctrl-B`, `Ctrl-N` and `Ctrl-P`
  go to the tile after, before, below and above. `Alt-V` needs a terminal that sends Alt or Option
  as `ESC` (in kitty on macOS, `macos_option_as_alt yes`). `Ctrl-L` still renders again.
- A number, then `Home`, goes to that page or tile, as a number and `g` do.

### Changed

- An archive's folder in a list has a zipper down its front and a plate at its corner saying which
  kind it is (`ZIP`, `TAR`, or `TGZ`, `TBZ`, `TXZ` for a compressed tar), where it was tied up
  with tape and a bow.
- The pager keeps up to 256 MiB of pages ready in the terminal, where it kept 192 MiB: in a large
  window, four of the largest pages where there were three.
- A PDF of more than 100,000 pages is refused, saying how many it has. A file of a few megabytes
  could name a million pages and take several gigabytes to open.
- Inside tmux, a page turned no longer flashes the screen blank before the next page shows.
- PDFs are drawn by a newer pdfcraft, and spreadsheets printed by a newer gridcraft, with their
  fixes since. A PDF's page warns of an image whose colour table is malformed, and of a Type 3
  font without its widths. A spreadsheet's Arabic is drawn where Noto Sans Arabic is installed,
  where it was blank.
- **Keys changed.** `d` and `u` go a screen down and up, as `space` and `b` do. Dark and light
  paper is `D`, where it was `d`. `u` no longer goes back: `Backspace` does, and `Esc`, in a
  file's pages, in a list, through the links followed and while a folder is being opened.
- `Alt` and a character, from a terminal that sends Alt or Option as `ESC`, is no longer read as
  `Esc` and then that character, which went back or quit: one the pager has no use for does
  nothing. Alt with `Enter` or with a `Ctrl` key is still read as `Esc` and then that key.
  `Alt-]` and `Alt-_` do nothing.

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
