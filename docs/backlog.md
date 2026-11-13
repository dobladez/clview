# Backlog

The details behind [README's Plans](../README.md#plans), which stay a line each, and ideas and loose
ends not yet planned: not promised, and not in the Plans until they are. What `clv` does today is in
[usage](usage.md); a change being built, in its [spec](specs/README.md).

## The Plans, in detail

### `d` for drawings too

An SVG or a Mermaid diagram shown alone, and a document's pictures by path, following the paper: an
SVG on white or transparent, a diagram in Mermaid's default theme on white or its dark theme on the
terminal's colour, a photograph as it is. A document's Mermaid blocks will already
([Spec-0009](specs/0009-draw-fenced-blocks-as-pictures.md)). The `paper` setting moves into the
contract for them, so a backend reads it without naming the documents' backend.

### More diagram languages

Graphviz (`dot`), D2, math, drawn from their files and from their fenced blocks as Mermaid's will
be: each a backend of its own, listing the fences it draws
([Spec-0009](specs/0009-draw-fenced-blocks-as-pictures.md)). PlantUML needs Java, which Rust only
rules out.

### More documents

Org-mode, AsciiDoc and reStructuredText, their source blocks (`#+begin_src`, `[source,x]`,
`.. code::`) drawn as Markdown's fenced blocks are. Proper support for ebooks: an epub as a book
(its chapters in order, its pictures), not a list of its files.

### `/` as a search in lists too

As in a file's pages: `n` and `N` among the names that match, the list not narrowed, and the filter
by name on a key of its own, so `/` means one thing everywhere.

### A config file

For customization and remembering settings, a Mermaid theme of the reader's choosing among them;
and the positions of files opened before.

### More pictures and archives

AVIF, colour profiles, and an animation's frames as its pages. zstd and 7z; a comic book archive
(cbz) as a book, its pictures its pages; an epub's pictures, and a document's in an archive.

### Lighter on memory, and steadier

- Let go of a list's files once the reader has moved on: a PDF opened holds its whole file until
  the list is left.
- Hold the grid's count to a row until a Markdown or HTML document's new layout lands: after `3.`
  the old pages are drawn three to a row, shrunk, until the columns are ready.
- Bound what an SVG's stylesheets take to parse, a [known issue](usage.md#known-issues): one long
  declaration in a `<style>` (a 600 KB `box-shadow` list) takes some 45 seconds to open, labels or
  not, which the drawing's limits don't count. A cap on a `<style>`'s size before it is parsed, or
  usvg and resvg in a worker process as the labels' Blitz is
  ([ADR-0026](adr/0026-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md)).

### HTML through Blitz, maybe

Blitz, which draws an SVG's HTML labels, weighed against wordcraft's importer for HTML documents:
Blitz lays out one long page, where documents are pages the window's size.

## Not yet planned

### Ask before opening a file past a limit

Today a file past one of [usage's Limits](usage.md#limits) is refused: a PDF of more than 100,000
pages, a Word document over 256 MiB, a Markdown, HTML, SVG or spreadsheet file over 64 MiB, and so
on. In the pager, `clv` could ask instead (`y` to open it all the same), and refuse only when there
is no one to ask, as when printing.

- The contract has no way for a backend to ask yet: a backend refuses with why, or opens.
- A list opens its files as their tiles are reached: a question there needs a design (on the tile,
  or only when the file is opened).
- A PDF's page count is known only once its page tree is parsed, some 2 KB a page, so the question
  would come after that cost: about 3.7 s and 2 GB for a million pages, 21 GB for ten million.
  Asking before the cost is paid needs the count from elsewhere. One option is a cap inside
  pdfcraft's page-tree walk, which clv can't count on. The other is a capped worker process that
  counts the pages first, as a search reads a PDF's text, at the cost of parsing every PDF once
  more.
