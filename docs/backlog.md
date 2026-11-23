# Backlog

The details behind [README's Plans](../README.md#plans), which stay a line each, and ideas and loose
ends not yet planned: not promised, and not in the Plans until they are. What `clv` does today is in
[usage](usage.md); a change being built, in its [spec](specs/README.md).

## The Plans, in detail

### `D` for drawings too

An SVG shown alone following the paper: on white, or transparent on the terminal's colour. Markdown,
HTML and Mermaid diagrams already do, a diagram shown alone and one a document shows alike. Nothing
of the contract (`src/source.rs`) or of the viewer needs to change for it: the SVG backend would
declare its own setting named `paper`, as Mermaid's does beside the documents', and the viewer
would hold one value for all three, ask the terminal's background for an SVG too, and name SVG
among the formats that go dark
([ADR-0033](adr/0033-backends-share-a-setting-by-declaring-its-name.md)). What is still to decide is
the drawing's side: what a drawing's dark form is, when most are drawn for one paper and say
nothing of the other. Not part of the change that gave Mermaid diagrams theirs, which only had to
ask merman for its dark theme.

### More diagram languages

Graphviz (`dot`), D2, math, drawn from their files and from their fenced blocks as Mermaid's
are: each a backend of its own, listing the fences it draws
([Spec-0009](specs/0009-draw-fenced-blocks-as-pictures.md)). PlantUML needs Java, which Rust only
rules out.

### More documents

AsciiDoc and reStructuredText, their source blocks (`[source,x]`, `.. code::`) drawn as
Markdown's fenced blocks are. Org shows now ([Spec-0014](specs/0014-show-org-mode-documents.md));
AsciiDoc and reStructuredText could go the way it and the epub go: a backend of their own, the one file naming their parser, that writes HTML and
hands it to the backend that opens HTML, so they have what a document has without the documents'
backend growing. An ebook's own stylesheets, applied a chapter
at a time: an epub is a book now ([Spec-0012](specs/0012-show-an-epub-as-a-book.md)), in clv's
own look, since the stylesheets of its chapters would meet in the one document they are joined
into. Other ebook formats: MOBI, AZW, FB2.

### `/` as a search in lists too

As in a file's pages: `n` and `N` among the names that match, the list not narrowed, and the filter
by name on a key of its own, so `/` means one thing everywhere.

### A config file

[Spec-0015](specs/0015-read-options-and-keys-from-a-config-file.md) gives clv a config file. The
file holds the defaults of options, an app for `o` to open each format with, and keys bound to
commands by name.

The positions of files opened before are left. clv would write them as state of its own, apart from
the config, which clv writes only when the reader asks (`--generate-config`). Open questions for
that change:

- How clv knows a file again: by its path, or by its size and the time it changed, after a move or
  an edit.
- Where clv keeps the state (`$XDG_STATE_HOME`), how many files it remembers, and how a reader
  turns it off.
- What a reader may not want written down: the names of the files that the reader opened.

### More pictures and archives

AVIF, colour profiles, and an animation's frames as its pages. zstd and 7z; a comic book archive
(cbz) as a book, its pictures its pages.

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
- Quicker to open a document. clv lays a Markdown, HTML, Word, Org or epub document out twice to
  show it: to open it, and again for its first render
  ([ADR-0005](adr/0005-word-documents-through-wordcrafts-readers-layout-and-renderer.md)). The first
  render could take the open's layout, which halves the wait. The layout must not stay with a
  list's file that is opened but never rendered. A book's pictures are measured by opening the
  `.epub` again for each picture: a book of 10,061 pictures opens it 20,000 times.
- A list's Markdown, HTML and Org documents could have a tile of their first page alone, as books
  do ([ADR-0041](adr/0041-a-books-tile-is-its-first-page-alone.md)).

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

### Words in a user journey's and a kanban board's tiles

A user journey's sections and tasks, and a kanban board's cards, are HTML labels (`<foreignObject>`)
even with merman's `htmlLabels` off, so a list's tile, which starts no HTML labels' worker, shows
those diagrams without their words ([Mermaid diagrams](usage.md#mermaid-diagrams)). merman has a
best-effort fallback, `foreign_object_label_fallback_svg_text`, that writes a `<foreignObject>`'s
text as SVG `<text>` for rasterizers: run on a diagram's SVG, it could give their tiles words.

- It isn't Mermaid's look: what it draws differs from the labels Blitz lays out when the diagram is
  opened. The diagram's page would still be drawn from the SVG as merman wrote it.
- Spec-0008 decided labels as SVG `<text>` through `htmlLabels` alone; this would be a second
  path, for tiles only.

### Dense diagrams the SVG backend refuses

Some diagrams merman accepts are refused by the SVG backend's own limits: 32 nodes each linked to
every other (496 edges, 1.2 MB of SVG) comes to more than 128 MiB once what its markers
(arrowheads) repeat is counted, as
[ADR-0003](adr/0003-bound-what-a-drawing-costs-before-usvg-and-resvg-spend-it.md) counts a marker
for each vertex it marks; 24 nodes (276 edges) draw. Is that counting too strict for Mermaid's
markers, a small path at each edge's end? Measuring what usvg actually spends on such a drawing
would say.

### A tar member's head without unpacking it whole

Past 8,192 headers, tar reads a member's head, for the backend that opens it, by unpacking the
member whole, up to 512 MiB (Codex, round 2 of
[Spec-0008](specs/0008-show-mermaid-diagrams.md)). A prefix read would do: from the disk, from the
spill, or from the decompressor stopped after the head.

### A bound on what a document's blocks cost in all

A Markdown document's ` ```mermaid ` blocks are each drawn within Mermaid's limits, and up to 1,024
of them, but nothing bounds them together: a 2.6 MB file of eight diagrams of 15,000 nodes took
67 s to show its first page, and one of 1,024 small diagrams 10 s
([ADR-0032](adr/0032-a-fenced-block-is-drawn-by-the-backend-that-lists-its-language.md)'s
Evidence), where the costliest Markdown without blocks takes some 20 s. A document's marks and
bytes hold it to minutes. Pictures by path cost the same, but they take as many files.

- A bound in time (the blocks sized until so many seconds are spent, the rest left as code) or in
  text (so many MiB of blocks drawn a document) would hold it; either makes what a document shows
  depend on more than the document.
- What waits is sizing, not drawing. A picture is drawn, to pixels, only when its page first
  shows; but the document is laid out whole to open, to know its pages, and the layout needs each
  picture's size. A diagram's size is known only once merman has laid it out, so every block is
  turned into SVG, and that SVG opened, before the first page shows. Sizing blocks as their pages
  are reached would need a layout around a size not yet known, laid out again as each comes: pages
  that move under the reader, as a list's tiles do before they are opened.
- merman's own work on such a diagram is 0.2 s
  ([ADR-0031](adr/0031-mermaid-diagrams-through-merman-in-a-worker-process.md)), of some 4 s a block
  a reading: the rest comes after it, its SVG of 9.7 MB opened by the SVG backend, and all of it
  twice, since eight such SVGs don't fit Mermaid's cache of 64 MiB. Where the time goes isn't
  measured further yet.

### `O`: open a file in an app of the reader's choosing

`o` opens a file in the desktop's default app ([Spec-0010](specs/0010-open-file-in-default-app.md));
`O` could list the apps that open it and let the reader pick one, over the pages as `?`'s keys are.
Neither `open` nor `xdg-open` lists them; the platforms keep the list:

- macOS: Launch Services, through `NSWorkspace`'s `urlsForApplications(toOpen:)`. `osascript -l
  JavaScript` reaches it without a dependency or `unsafe` (tried on 2026-11-14: Finder, QuickTime,
  VS Code, Zed, VLC… for a folder); from Rust it would take the objc2 crates. `open -a <app>` or
  `open -b <bundle id>` then opens with the one picked.
- Linux and the BSDs: freedesktop's MIME-apps spec (`mimeapps.list`, and `mimeinfo.cache` in each
  `applications/` directory), which `gio mime <type>` prints as registered and recommended apps on
  most desktops; `xdg-mime query filetype` gives the type. The one picked opens through
  `gio launch`, `gtk-launch <desktop id>`, or its desktop entry's `Exec` line with its `%f` and
  `%u`.
- WSL: no portable list.

The launchers' trait leaves room for it: `apps(path)` and a command for the app picked, with
defaults for a launcher that can't list. Apps that take a page (Skim, Zathura, Okular) could open
at the page the reader is on.

### Open an archive's file in another app

`o` refuses a file that lives only in an archive ([Spec-0010](specs/0010-open-file-in-default-app.md)).
It could unpack it to a temporary file and open that, but `clv` can't know when the app has read
it: deleting it at exit may pull it from under the app, and keeping it leaves the reader's files in
a temporary directory. `open -W` waits for the app on macOS; `xdg-open` has no such thing.

### More openers, WSL among them

`o` opens a file through `open` on macOS and `xdg-open` elsewhere
([Spec-0010](specs/0010-open-file-in-default-app.md)); another platform or desktop is another
launcher and its test. Candidates: `gio open`, `kde-open`, `exo-open`, `termux-open`, Haiku's `open`,
and WSL's `wslview`. `wslview` hands the path on to PowerShell or `cmd.exe`, a parser of its own, so
it waits for tests on a real WSL with names holding quotes, `$()`, backticks and `&`; `explorer.exe`
with `wslpath -w` is the other way there, with parsing of its own to test too.

### `f` in a list: labels on tiles

`f` labels a file's links and follows the one typed ([Spec-0011](specs/0011-follow-a-documents-links.md)).
In a list, the same labels could go on the tiles on screen, typing one opening it as `Enter` does:
Vimium's own use, a click without a mouse. The labels' codes and drawing would be the same; only
what a label stands for differs, a tile and not a link.

### A table of contents to browse

A PDF's outline (pdfcraft's `DocInfo.outline`: titles, levels, each a page and a place) and a
document's headings (paragraphs with an outline level, which wordcraft's PDF exporter already lists
with their pages) could be shown over the pages as `?`'s keys are, and one chosen gone to, as
following a link to a place in the document goes there
([Spec-0011](specs/0011-follow-a-documents-links.md)), Backspace coming back. A book (epub) has its
table of contents as a page of its own meanwhile, a Contents page of links at its end, and `T`
goes to its contents ([Spec-0012](specs/0012-show-an-epub-as-a-book.md)): its entries, which the
epub's worker reads, would serve the overlay too, and the `contents` command would open it.

### Fixed-layout epubs through Blitz

A fixed-layout epub (`rendition:layout` `pre-paginated`: comics, picture books, magazines) is a page
an XHTML file, of the size its `<meta name="viewport">` says, its text and pictures placed by CSS,
often text over a picture that fills the page.
[Spec-0012](specs/0012-show-an-epub-as-a-book.md) lays such a book out flowing, through wordcraft's
importer, which has no positioning and reads none of the book's stylesheets: a page of one picture
comes out well, text placed over a picture comes out beside it. Shown as it is made, each page would
be drawn by Blitz at its viewport's size, a page of clv each, in a worker process capped as the SVG
labels' Blitz is ([ADR-0026](adr/0026-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md)):
with the book's stylesheets, its pictures read from the book, its embedded fonts (an obfuscated one
undone, by the key the OPF's identifier gives), and its text and links from Blitz's layout for `/`
and `f`. A book that mixes fixed and flowing chapters would need both in one source. A cheaper step
first: a book whose every page is one picture shown as those pictures, each at its own size, as a
comic archive would be ([More pictures and archives](#more-pictures-and-archives)). Weighed with
[HTML through Blitz](#html-through-blitz-maybe), which would draw them the same way.

### Running a command by name (`M-x`)

Every key the pager takes goes to a command with a name, its help and whether it takes a count
([Spec-0013](specs/0013-bind-keys-to-named-commands.md)). A key could ask for one by name, as Emacs's
`M-x` does: its name typed on the status line, completed among the commands where the reader is, a
number before it taken as its count. A command no key is bound to would be reached that way.
