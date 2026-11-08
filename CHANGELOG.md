# Changelog

What changed for someone using `clv`, newest first. The [manual](docs/usage.md) says how each thing
works today. A version's section becomes the notes of its
[release](https://github.com/dobladez/clview/releases).

## Unreleased

### Added

- Markdown and HTML reflow to a width: `[` and `]` lay the document out again 4 characters a line
  narrower or wider, `72]` to 72, from 20 to 200, with `page` (US Letter, as before) among the steps;
  `--width page|N` says where to start. The paragraph at the top of the screen stays on the page you
  land on. The status line shows the width when it isn't a page's. Only
  the document on screen reflows: a list's tiles keep their layout, and a document opened from the
  list after a change is laid out at the new width.
- Homebrew, on macOS or Linux: `brew install dobladez/tap/clv`, and `brew upgrade clv` for each new
  version. No `xattr` step on macOS.

### Changed

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
