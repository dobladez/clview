# Spec-0012: Show an epub as a book

- **Status:** Implementing
- **Date:** 2026-11-15
- **Related:** [ADR-0013](../adr/0013-a-documents-pictures-are-drawn-by-the-backends.md) (a document's
  pictures, only from near it, which this lets a held file reach: its follow-up);
  [ADR-0012](../adr/0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md) (what a
  document costs, bounded); [ADR-0017](../adr/0017-htmls-style-rules-apply-as-far-as-wordcraft-reads-them.md)
  (HTML's style rules); [ADR-0019](../adr/0019-a-list-opens-its-files-when-their-tiles-are-wanted.md)
  (a list's files opened when wanted); [ADR-0023](../adr/0023-the-window-is-the-page-of-markdown-and-html.md)
  (the window as the page); [ADR-0026](../adr/0026-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md)
  and [ADR-0031](../adr/0031-mermaid-diagrams-through-merman-in-a-worker-process.md) (a library
  that can't be bounded before it runs, run in a worker process capped as it runs; and a backend
  handing what it makes to another, found by what it opens, as
  [ADR-0006](../adr/0006-spreadsheets-printed-by-gridcraft-to-a-pdf-the-pdf-backend-shows.md)'s
  spreadsheets do); [ADR-0033](../adr/0033-backends-share-a-setting-by-declaring-its-name.md) (a
  setting shared by name); [Spec-0009](0009-draw-fenced-blocks-as-pictures.md) (HTML's `<pre>`
  blocks, and the scanner this reuses); [Spec-0011](0011-follow-a-documents-links.md) (a document's
  links, which a book's follow); [Documents](../usage.md#documents), [Archives](../usage.md#archives),
  [Limits](../usage.md#limits), [Backends](../architecture.md#backends); the backlog's
  [More documents](../backlog.md#more-documents), [More pictures and archives](../backlog.md#more-pictures-and-archives)
  and [a table of contents](../backlog.md#a-table-of-contents-to-browse). It delivers README's Plans
  bullet "More documents: … ebooks (epub) as books" in its epub half.

## Summary

An epub shows today as a zip: a list of its files, each chapter opened alone as HTML, its pictures
as their alt text, since a file held in an archive has no directory to find them in. This shows an
epub as a book. A new backend, `epub.rs`, has [rbook](https://github.com/DevinSterling/rbook) read
the package (`META-INF/container.xml`, the OPF's manifest, spine and cover) in a worker process of
clv's own, capped on CPU time and heap, since rbook can panic, recurse without bound and read
without a cap on a hostile file. From the reading order it answers, the backend joins the
chapters into one HTML document, each chapter on a new page, the cover first, and hands it to
whichever backend opens HTML, found by what it opens, as Mermaid hands its SVG on. So the book has
everything a document has: a page the window's size, `width`, `font`, `margin`, `d`'s dark paper,
`s`'s fenced blocks, `/`'s search across chapters, and `f`'s links, rewritten so a link from one
chapter to another lands in the book and none opens a file beside it. Its table of contents (EPUB
3's nav, EPUB 2's NCX), read by rbook in the same worker, becomes a Contents page at the book's
end: a list of links, `f` going to a chapter, `u` back; and a key goes to the book's contents from
any page, its own page of them where it has one, else that page. For its pictures, the contract
lets a held file read what is beside it in its holder, which the epub gives its chapters, and
which zips and tars give the documents they hold too: a README in a zip shows its screenshots.

## Requirements

**Which files are books**

- A zip whose first file is `mimetype` saying `application/epub+zip`, whatever its name; or a zip
  named `.epub` whose first bytes don't say (a `mimetype` not first, or compressed, as some tools
  write). `epub` is listed as the format's extension, so `--help` lists the format with it and
  credits rbook. An unnamed book can't be opened with `o` (its name doesn't say what opens it), as
  any unnamed file.
- A file that is one but can't be read as a book (no `container.xml`, no OPF, an OPF with no spine,
  or one rbook refuses) is refused with why, rbook's reason through `printable` and cut to 256
  bytes, as a broken document is: it isn't shown as a zip instead. One whose reading kills the
  worker (past its CPU time or its heap, or a panic) is refused the same way:
  `its package couldn't be read`.
- **Protected chapters** (DRM): a chapter that `META-INF/encryption.xml` says is encrypted by any
  algorithm but the font obfuscations (IDPF's and Adobe's) is left out, and the first page says how
  many were (`3 chapters are protected (DRM)`); what is readable is shown. A book none of whose
  chapters is readable is refused: `it is protected (DRM)`. A protected picture shows as one that
  can't be drawn does. Fonts obfuscated are no matter, since a document's fonts aren't used.

**What the book shows**

- **The spine's chapters, in its order**, all of them, `linear="no"` ones included where they come,
  each starting a new page. A chapter is XHTML; one that isn't (an image or an SVG as a chapter)
  shows as a page of that picture; any other kind is followed through the manifest's `fallback`
  chain, at most 8 steps, to one that is, and otherwise left out. A chapter missing, unreadable or
  past a limit is left out; the first page says how many were and why, as it does pictures.
- **The cover first:** when rbook names a cover picture (EPUB 3's `cover-image` item, else EPUB 2's
  `<meta name="cover">`) and the first chapter doesn't show it already, it is a page of its own
  before the first chapter. So a list's tile for a book is its cover.
- **The book's look is clv's** ([ADR-0016](../adr/0016-how-markdown-and-html-look.md)), with what
  its markup says on top through `style` attributes. Its stylesheets, linked or in a chapter's
  `<style>`, are not read: wordcraft reads every `<style>` into one stylesheet for the document, so
  one chapter's rules would reach them all, and the class names tools write (`calibre1`) meet. Usage
  says so as a known difference.
- XHTML's empty elements written short (`<div/>`, `<a id="x"/>`) are read as empty, not left open.
  An `<svg>` wrapping only an `<image>` (the usual cover page) shows as that picture.
- **Fixed-layout books** (`rendition:layout` `pre-paginated`, comics and picture books) are laid out
  as any other, their pages flowing: usage says so as a known difference.
- **Its pictures** (`<img>`, and an `<svg>` cover's `<image>`) are read from the book, by their
  paths from their chapter, `..` included but never above the book's top, through the backends
  that draw pictures, as a document's on disk are ([ADR-0013](../adr/0013-a-documents-pictures-are-drawn-by-the-backends.md)),
  within its limits (1,024 a document, 256 MiB of them held). Measuring one for the layout reads
  only as much of it as its header takes, at most 64 KiB, never the whole picture: opening a book,
  or a list of them, unpacks none whole. A picture on the web, or by an absolute path, is still its
  alt text.
- **Everything a document has:** the settings `width`, `font`, `margin`, `paper` and `fences`, with
  the documents' keys and options; `/` across the whole book; `--pages`; text cells and tmux as for
  any document; `o` opens the `.epub` itself.

**Links**, as Spec-0011 has them in any document

- A link from one chapter to another (`ch2.xhtml`, `ch2.xhtml#s3`, `../Text/ch2.xhtml#s3`) or to a
  place in its own (`#s3`) is a place in the document: `f` and its label go there, `u` back, `F`
  copies it as its `#name`. A chapter's start is a place too. Ids are made unique in the book (an
  element's `id` already being a place, `ids_as_places`), so two chapters' `id="s1"` don't meet; an
  id that would pass Spec-0011's 256 bytes once made unique is left out, and links to it say the
  place is missing.
- **No link from a book is followed to a file**, never resolved beside the `.epub` on disk: one to a
  file in the book that isn't a chapter (a picture, a stylesheet) says `a file in the book, which clv
  doesn't open alone`, one to a file that isn't in the book says `not in this book`. A web address
  is one, as in any document.

**Contents**

- **A Contents page** at the book's end, after its last chapter, so that it moves none of the
  book's own pages (the owner's, 2026-11-16; it was after the cover): headed `Contents`, with a
  line under that saying clv made it from the book's table of contents, of the table of contents
  as rbook reads it: EPUB 3's nav (`toc`), else EPUB 2's NCX. Its entries are nested lists as
  deep as the book's, each its label as a link to the place it names (a chapter, or a place in
  one, rewritten as the chapters' links are), so `f` goes there and Backspace comes back, and `/`
  finds a chapter's title. An entry whose place isn't in the book is its label, not a link. It is
  made for every book that has a table of contents, one with a page of contents among its
  chapters too.
- **A key goes to the contents** (the owner's, 2026-11-16): `T`, the `contents` command
  ([Spec-0013](0013-bind-keys-to-named-commands.md)), from any page, Backspace coming back. To the
  book's own page of contents where it has one: the chapter the OPF marks (the nav document in the
  spine, or the guide's or landmarks' `toc`), or else one guessed, among the book's first
  chapters, by its links leading mostly to other chapters, within bounds a hostile chapter can't
  make costly. Otherwise to the Contents page. The viewer knows no format: a source declares where
  its contents are, and the viewer offers the key, lists it in `?` and says it on the status line
  for a second when the file opens (`Contents: T`) only where one is declared.
- **A table of contents rbook can't read** leaves the book without the page, the first page saying
  so; it never refuses the book.
- Limits: 4,096 entries, 256 bytes a label (cut at a character), 8 levels shown (deeper ones at the
  eighth); past them what is over left out, and said.

**The worker**

- `clv --worker epub` reads one book's package a request, and is gone once it has answered. It is
  given only what it reads: `container.xml` and the OPF, read from the epub by the backend within
  their limits, and `encryption.xml`, never a chapter or a picture; it answers the reading order, the
  cover, the fallbacks, the layout and which file holds the table of contents, which the backend
  checks against its own limits before keeping. The package is read without its table of contents
  first (`skip_toc`), since one rbook can't read fails the whole package; then, when a Contents page
  is wanted, a second request gives that file too (1 MiB at most), and is answered with its entries,
  or with why not. A worker that dies on the table of contents costs the book its Contents page,
  not the book.
- Its CPU time and heap are capped, as the other workers' are, at what the costliest real and
  hostile packages measure (a 4 MiB OPF, a chain of `refines` its length), set by an ADR with how
  to measure them again; one package read at a time, so a list of books takes one worker slot of
  the six.
- A package is read once a book is opened: what it answers, small, is kept with the book, and
  assembling it again (a render, a relayout) asks the worker nothing.

**Where the book is read from**

- On disk: the epub is read from its file each time the book is assembled (to open, on its first
  render, on each relayout) and for each picture; an open book holds where its file is and the
  reading order, not its text, since a list may hold many open
  ([ADR-0019](../adr/0019-a-list-opens-its-files-when-their-tiles-are-wanted.md)).
- In an archive: read through its holder each time, as any held file is; a picture costs reading
  the book from the archive again. Through a pipe: kept whole while open, at most 256 MiB, past which
  it is refused.

**Pictures beside a held file**

- A Markdown or HTML document in a zip or a tar shows the pictures beside it in that archive, by
  the same paths as on disk, `..` included but never above the archive's top, measured within 64
  KiB as a book's are. A file compressed alone, or through a pipe, still has nothing beside it.
- A document's relative links in an archive are still not followed (Spec-0011's Out of scope).

**Limits**, each checked before what it bounds is allocated, past it the book refused or what is
over left out, and said:

- `container.xml` 64 KiB; the OPF 4 MiB; `encryption.xml` 1 MiB; the zip's own limits on entries
  and sizes, the same as the zip backend's ([Archives](../usage.md#archives)).
- The worker's answer: 4,096 chapters, 4 KiB a path, else the book refused.
- 16 MiB a chapter. The book is assembled within bounds of the epub's own below the documents'
  ([ADR-0012](../adr/0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md)): what it
  writes, its marks and its text counted as it writes, so a chapter that would pass them is left out
  with those after it, and said, rather than the whole book refused by the documents' backend.
- A chapter UTF-8, or UTF-16 with its byte-order mark, else left out.
- Never a panic in clv on any file: a property test cuts and mutates chapters and the worker's
  answers; rbook's own are the worker's, which dies alone.

## Out of scope

- **A table of contents to browse over the pages**, for PDFs and documents too
  ([backlog](../backlog.md#a-table-of-contents-to-browse)): a key and an overlay of the viewer's, and
  a contract for a source's outline. A book's would come from this worker's entries, then.
- **The book's page list** (`page-list`) and its landmarks beyond `toc`.
- **The book's own stylesheets**, applied a chapter at a time.
- **Fixed-layout books as their pages**, one page of the book a page of clv, drawn as they are made
  ([backlog](../backlog.md#fixed-layout-epubs-through-blitz)); and right-to-left page order
  (`page-progression-direction`).
- **The book's files as a list**, as a zip: no key or option to see them.
- **The title** from the OPF's metadata in the status line or `--help`'s icon.
- **Comic archives (cbz) as books** ([backlog](../backlog.md#more-pictures-and-archives)).
- **Relative links between files in an archive** (Spec-0011's Out of scope), though `Near` would
  serve them.
- **A cheaper tile:** a list's tile lays the whole book out, as a document's tile does; showing the
  cover alone for a tile is a contract change of its own.
- **A worker kept between books**, as Mermaid's `Service` keeps one: started a book at a time
  until measured too slow for a list.
- **Other ebook formats:** MOBI, AZW, FB2.

## Approach

**Files:**

- `src/source.rs` (the contract): a held `Input` may carry what is beside it, `Near`
  (`Arc<dyn Fn(&str) -> Result<Input, String> + Send + Sync>`, a path from the root of its holder,
  normalized, to a held `Input` with its head, `Again` and `Bounded`), in `Rest::Held` beside
  `Again` and `Bounded`; `Input::held_near(…)` and `Input::near()`. `BACKENDS` lists `epub::Epub`
  after `gridcraft` and before `pdfcraft` (by how it starts, before PDF, and before zip, which takes
  any `PK`; wordcraft's and gridcraft's `Kind::of` already refuse an epub, their tests say so).
  `WORKERS` lists `epub::PACKAGE`.
- `src/source/picture.rs`: `Pictures.beside` becomes where pictures are read from, a directory or a
  `Near` and the document's own path in it; `Origin::Near { near, path }` opens through `shown_by`
  among the backends that draw pictures, as `opened` does; `local()`'s checks hold for both, `..`
  resolved against the holder's root and refused above it. The epub writes its `src`s
  percent-encoded (`%` as `%25`), since `local()` decodes them.
- `src/source/image.rs`: a held picture measured through its `Bounded` (64 KiB), not read whole
  (`Image::held` reads it whole today), as one on disk is measured by its header.
- `src/source/wordcraft.rs`: `Stored::Held` keeps the `Near` it was given, `Document::beside` hands
  it to `Pictures`, and `Document::at` (relayout) keeps it. The HTML lexer (`Piece::at`,
  `start_tag`, `SKIPPED`, as `ids_as_places` and `blocks_drawn` use it) moves to a shared module.
- `src/source/markup.rs` (new, moved): that lexer, extended to give every attribute of a start tag
  with its place in the text (`StartTag` keeps only `class`, `lang` and `self_closed` today), for the
  epub's rewriting; built for `word`, which `epub` brings.
- `src/source/zipped.rs` (new, moved from `zip.rs`): reading a zip's file whole or to a bound,
  with its limits (`MAX_ENTRIES`, `MAX_BYTES`) and the `catch_unwind` around the zip crate
  (`caught`), shared by the zip backend and the epub's, as `head.rs` is.
- `src/source/later.rs`: `Reopen::Held` carries the `Near`, so a list's file opened late keeps it.
- `src/source/zip.rs`, `src/source/tar.rs`, `src/source/tree.rs`: their containers' `open` give a
  `Near` resolving from the archive's root through their `Tree` (`Tree::file`, with `..` resolved
  before it, since it doesn't), reading as their `Again` and `Bounded` already do.
- `src/source/epub.rs` (new), two sides, as `mermaid.rs` has:
  - **The worker** (`PACKAGE`, a `worker::Worker`, its `Session` answering `READ` and `CONTENTS`):
    the files it is sent put in a zip of its own in memory, read by rbook (`Epub::options()
    .strict(false)`, `.skip_toc(true)` for `READ`, `.preferred_toc` the nav for `CONTENTS`), then for
    `READ` the spine's items in order (`manifest_entry()`, its href `.path().decode()`,
    `media_type()`, `is_linear()`), each fallback followed through `fallback()` (at most 8, a loop
    stopped), `manifest().cover_image()`, `rendition:layout`, the nav item's or the NCX's href, the
    `toc` landmark's or guide's href; and the files `encryption.xml` says are encrypted by an
    algorithm but the font obfuscations, each `EncryptedData`'s `EncryptionMethod` `Algorithm` and
    `CipherReference` `URI` (rbook doesn't read it: a scan of the worker's own over its 1 MiB, which
    a hostile file can only kill the worker on). For `CONTENTS`, the
    entries of `toc().contents()` walked with a stack of the worker's own, never `max_depth()` or
    `total_len()` (recursive): each its depth, label and href with its fragment, stopped at the
    limits. Answered in a format of the backend's own, framed and bounded; anything rbook says as a
    reason, cut. Its CPU time and heap from the ADR's measures.
  - **The backend**: the epub opened with `zipped.rs`; `container.xml`, the OPF it names (found by
    the backend's own look for `full-path="…"`, a 64 KiB file read for one attribute) and
    `encryption.xml` read within their limits and sent; the answer checked and kept (`Plan`). Then
    the book assembled as one HTML text through `markup.rs`'s lexer: its first 1,024 bytes clv's own
    (`<!doctype html>` and a comment filling them, so nothing a book writes decides which backend
    opens it: a tar's header is looked for at byte 257); each chapter's `<body>`'s content in a
    `<div>` with `page-break-before: always` and the chapter's id; ids prefixed with the chapter's
    number; every relative `href` rewritten (to a `#` place for a chapter, else left to be answered
    by `Book`); `src`s to paths from the book's root; `<x/>` written `<x></x>` for an element that
    isn't void; a cover `<svg><image>` as an `<img>`; `<style>` and `<link>` dropped; the Contents
    page written from the entries kept, its labels escaped. Opened through
    the first other backend whose `opens` takes it (as `mermaid.rs`'s `shown`, never by name), with
    `open_at` overridden to pass the `Room` on. What it gives is wrapped in a `Book` that hands on
    every `Source` method, as `mermaid.rs`'s `Diagram` does (not only some, as `gridcraft.rs`'s
    `Noted`), wraps the source a relayout's job makes too, adds the first page's note of chapters
    left out, and wraps `links()`: every `Target::File` it gives becomes `Target::Other` saying
    which it is, so the viewer never looks beside the `.epub`. The `Input` handed on is held, its
    `Again` assembling the book again from the kept `Plan`, its `Near` reading from the epub.
    Settings declared as the documents' are, under their names (`paper`, `width`, `font`, `margin`,
    `fences`, copies of wordcraft's, which `wrong_settings` checks are the same), the format listing
    them, and `deferred` saying so. Credits rbook.
- `src/source/worker.rs`: its `cfg_attr` on dead code names `epub` among those that start a worker.
- `Cargo.toml`: feature `epub = ["dep:rbook", "dep:zip", "word"]`, on by default; rbook `=0.7.13`
  without its default features (`write`, `prelude`, `threadsafe`: the worker reads on one thread).
  Its dependencies are in the tree already, but for `zip` moving from 9.1.3 to 9.5.1 in the lock,
  which the zip backend and gridcraft read with too.
- `tests/it/epub.rs` (new): books written by the test; hostile packages killing the worker; and
  that nothing but `epub.rs` and it names rbook. `tests/it/mermaid.rs`:
  `a_mermaid_block_in_html_is_drawn_through_a_pipe_and_in_an_epub` writes a `book.epub` without a
  package and expects a list of its files: it becomes a real book. `tests/it/zip.rs` and
  `tests/it/tar.rs`: a document's pictures beside it. `tests/it/view_pager.rs`: a book searched, its
  worker gone once it has answered (`ps`), its links followed.
- `docs/adr/NNNN-measure/` (new): the packages the worker's caps were measured with, as
  `docs/adr/0031-measure/` has Mermaid's.
- Docs: usage (Documents: a section on books; Archives: a document's pictures; Which files it shows:
  the row; Limits; Known issues: stylesheets and fixed layout), architecture (Backends' table, How
  each opens: the epub, its worker, `Near`), two ADRs, `CHANGELOG.md`, README (what it shows, its
  credits; Plans), backlog (More documents, More pictures and archives), `CONTRIBUTING.md` (rbook
  only in `epub.rs`; its pin moved with the caps measured again, as merman's is).

**Patterns to follow:**

- `mermaid.rs`: a library in a worker (`DIAGRAMS`, its `Session`, its requests' kinds and caps,
  its tests with fake workers that die); `shown`, what a backend makes opened by the first other
  backend that opens it; `Diagram`, a source handing on every method; its settings declared under
  the documents' names (ADR-0033), and its `deferred`.
- `zip.rs`'s `Directory::open` and `Archive::within`: reading a file held in the zip, whole or to a
  bound, holding no open file while idle.
- `wordcraft.rs`'s `ids_as_places` and `blocks_drawn`: a pass of clv's own over HTML before
  wordcraft imports it.
- `tests/it/mermaid.rs`'s epub, written by a test; `tests/it/view_pager.rs`'s Mermaid worker seen
  with `ps`.

**Decided** (two ADRs, numbered when written):

- **rbook, in a worker process** (the owner's choice, 2026-11-15, over a parser of clv's own on
  quick-xml). rbook 0.7.13 (Apache-2.0) was read for this: a media type with a character past ASCII
  (`application/fontè`) panics where it slices at a byte (`util/str.rs:22`, through
  `ResourceKind::is_font`); a chain of `refines` in the OPF is walked by recursion, and a nav or NCX
  nests without bound and is dropped recursively, which a hostile file could take past a thread's
  stack, an abort; it reads the OPF and the table of contents whole whatever their size; and a table
  of contents it can't read fails the whole book. In the viewer that would take guards against its
  own code, checked again at each version; in a worker, as ADR-0026 and ADR-0031 run Blitz and
  merman, a panic, an abort or a heap past its cap kills the worker alone, and the backend gives it
  only what it bounds first. The worker is given the package's files, not the epub, so nothing large
  crosses its pipe and it reads no file.
- **One document, through the backend that opens HTML**, not a source a chapter joined: search,
  links, relayout and settings come whole, and each chapter on a page of its own keeps the book's
  shape. wordcraft honours `page-break-before` and `break-before` from a `style`
  (`crates/formats/src/html/css.rs:120` at the pin) outside its web view, which clv doesn't use.
- **The table of contents as a page of the book**, not an overlay of the viewer's: it needs nothing
  the viewer doesn't have once links are followed (Spec-0011), works the same in kitty, tmux and text
  cells, and is what a printed book does. The overlay, for every kind of file, is the backlog's.
- **What is beside a held file, in the contract** (`Near`), not the epub reading its own pictures
  inside the documents' backend: the documents' backend never names the epub, and archives' documents
  get their pictures from it too. Updates ADR-0013's "a document … held in an archive has no
  directory".
- **The book assembled again when read**, from the reading order kept, not held, on disk and in an
  archive.
- **One spec**, pictures beside a held file among its tasks (the owner's choice, 2026-11-15), though
  some twenty files with the docs.
- **The owner's answers, 2026-11-15:** the book's stylesheets left out; fixed-layout books laid out
  flowing; `linear="no"` chapters kept in the spine's order; a book with protected chapters shows
  what is readable.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- 🟡 **rbook on a zip of the package alone** — that it reads the spine, the manifest and the cover
  from a zip holding only `container.xml` and the OPF, with `strict(false)` and `skip_toc`, without
  asking for a chapter or a picture: try it first in task 2. If it doesn't, the worker is sent the
  epub whole (a path on disk, the bytes otherwise, within 256 MiB).
- 🟡 **The worker's caps** — measured on real books and hostile packages in task 2; a worker a book
  costs its start (some milliseconds) on each open, a list's tiles included.
- 🟡 **The cost of a book's tile** — a list of books lays each out whole for its tile (a 400-page
  novel some 1.5 s, going by ADR-0012's 20 s for 5,000 pages). Measure on real books in task 3; if
  too slow for a shelf, a cover-only tile is the next spec.
- 🟡 **A page break on a `<div>`** — whether wordcraft carries `page-break-before` from a `<div>` to
  its first paragraph, or needs it on that paragraph: try in task 2 before choosing.
- 🟡 **64 KiB to measure a picture** — a JPEG's size comes after its EXIF, which a camera's photo may
  put past it; measure on real books' pictures, and a picture not measured within it is read whole.
- 🟡 **A book's own contents page, told** — the OPF's nav in the spine or a `toc` landmark or guide
  reference says a chapter is one; a book that has one and says nothing gets two. Look at a dozen
  real books (Gutenberg's, Standard Ebooks', a publisher's) in task 5 before settling.
- 🟡 **Book CSS** — real books may look plainer than in a reader without their stylesheets;
  revisit after use.

## Tasks

- [x] **Pictures beside a held file** — `Near` in the contract, `Origin::Near`, a held picture
  measured within 64 KiB, wordcraft's `Stored::Held` and relayout keeping it, `later.rs` carrying
  it; zip's and tar's containers giving it, `..` resolved, never above the top; ADR (pictures from a
  held file's holder, updating ADR-0013); usage's Documents and Archives, architecture, the
  changelog. *Files:* `src/source.rs`, `src/source/picture.rs`, `src/source/image.rs`,
  `src/source/wordcraft.rs`, `src/source/later.rs`, `src/source/zip.rs`, `src/source/tar.rs`,
  `src/source/tree.rs`, `tests/it/zip.rs`, `tests/it/tar.rs`, `docs/`. *Verify:* `cargo test -j 2
  --test it a_documents_pictures` (in a zip and in a tar, a Markdown README draws its PNG and its
  `../` one, `../../` past the top stays alt text, still drawn after `+`'s relayout, opened late from
  a list, a 10 MB picture measured without being read whole) and `cargo test -j 2 --bin clv
  picture::tests` (a `Near` path normalized, `%2F` and an absolute path refused).
- [x] **An epub's chapters as a book, its package read in a worker** — `epub.rs`'s worker
  (rbook, its caps measured, its answer's format) and backend (the package's files read and sent,
  the answer checked and kept; the book assembled: chapters in order a page each, ids prefixed,
  `src`s from the root, styles dropped, clv's own first 1,024 bytes, the assembly's limits; opened
  through the HTML backend with its room in a `Book` that hands every method on, notes chapters
  left out, and turns every `Target::File` into a message); settings and `deferred`; `zipped.rs`
  and `markup.rs` moved out; the feature and rbook's pin; ADR (rbook in a worker; one document
  through the HTML backend) and its measures; usage, architecture, `CONTRIBUTING.md`, README, the
  backlog, the changelog. *Files:* `src/source/epub.rs`, `src/source/markup.rs`,
  `src/source/zipped.rs`, `src/source/zip.rs`, `src/source/wordcraft.rs`, `src/source/worker.rs`,
  `src/source.rs`, `Cargo.toml`, `Cargo.lock`, `tests/it/epub.rs`, `tests/it/main.rs`,
  `tests/it/mermaid.rs`, `tests/it/view_pager.rs`, `docs/`, `CONTRIBUTING.md`, `README.md`,
  `CHANGELOG.md`. *Verify:* `cargo test -j 2 --bin clv epub::tests` (the worker's answer for an
  EPUB 2 and an EPUB 3 package, `linear="no"`, hrefs with `..` and `%20`; a fake worker that dies or
  answers too much; chapters assembled, a chapter's `<style>` dropped, a chapter cut at the
  assembly's bounds; a property test that cut or mutated chapters and answers never panic);
  `cargo test -j 2 --test it epub` (chapters in order, each on a new page; a picture by
  `../Images/`; an unnamed zip with an epub's `mimetype`; a `.epub` with it compressed; from stdin
  and in a zip; no spine refused; a media type `application/fontè` and a 4 MiB chain of `refines`
  refused, clv standing; a chapter missing left out and said; a link to a picture in the book said,
  and a file of that name beside the `.epub` not opened; `--width 60`, dark paper; nothing else
  names rbook; the Mermaid epub as a book); `cargo test -j 2 --test it view_pager::epub` (`/`
  finds a word in the third chapter; the worker gone once it has answered).
- [ ] **Covers, fallbacks and DRM** — the cover first (both kinds, not twice), an `<svg>` cover as
  its picture, empty elements written short, a picture or SVG as a chapter, the `fallback` chain,
  protected chapters left out and said (the book refused when none is left), fixed layout said; a list of books timed. *Files:* `src/source/epub.rs`,
  `tests/it/epub.rs`, `docs/usage.md`. *Verify:* `cargo test -j 2 --bin clv epub::tests` (both
  covers, a fallback chain and its loop, obfuscated fonts allowed, a chapter under another algorithm
  left out, `encryption.xml` past 1 MiB, `<div/>` read as empty); `cargo test -j 2 --test it epub`
  (the cover a page first and a list's tile; two of five chapters protected: three shown and the note
  says two; all protected: refused); `Manual: in kitty, clv a Project Gutenberg epub
  with pictures (Alice's Adventures in Wonderland): the cover, then each chapter on its own page with
  its pictures; / a word from the last chapter; d; + then -; and a directory of 20 epubs, timed`.
- [ ] **A book's links** — chapter-to-chapter and in-chapter links rewritten to places, a chapter's
  start a place, an id too long once prefixed; usage. *Files:* `src/source/epub.rs`,
  `tests/it/view_pager.rs`, `docs/usage.md`, `CHANGELOG.md`. *Verify:* `cargo test -j 2 --bin clv
  epub::tests::links` (relative, `..`, a fragment, none, a missing chapter, an id past 256 bytes) and
  `cargo test -j 2 --test it view_pager::epub_links` (`f` on a contents page's label lands on chapter
  3's heading, `u` back, `F` copies its `#name`); `Manual: in kitty, a Gutenberg epub's contents page,
  f, a label: its chapter`.
- [ ] **A book's Contents page** — the worker's `CONTENTS` (nav, else NCX; walked with its own
  stack; its limits), the page written at the book's end with links to places and a line saying
  clv made it (revised 2026-11-16: it was after the cover, and not made when a chapter is the
  book's own), a table of contents unread or killing the worker leaving the book whole; usage,
  the changelog. *Files:* `src/source/epub.rs`, `tests/it/epub.rs`, `tests/it/view_pager.rs`,
  `docs/usage.md`, `CHANGELOG.md`. *Verify:* `cargo test -j 2 --bin clv epub::tests::contents` (an
  EPUB 3 nav and an EPUB 2 NCX nested three deep; a label with markup escaped; 4,097 entries; a
  level past 8; an entry to a missing chapter its label alone; the page last, its room kept);
  `cargo test -j 2 --test it epub` (the page last, where the spine holds the nav too; an NCX
  nested 100,000 deep killing the worker, the book shown without the page and said); `cargo test
  -j 2 --test it view_pager::epub_t_goes` (`f` on the Contents page's third entry lands on its
  chapter, Backspace back); `Manual: in kitty, a Standard Ebooks epub: the Contents page its last
  page, saying clv made it; f, a chapter`.
- [x] **A shorter sentence where a setting isn't, on the whole status line** (the owner's, 2026-11-16,
  the sentence having outgrown 80 columns with one more format in it) — `this format doesn't
  reflow (Markdown, HTML and others do)`, two formats named from what the backends declare; it has
  the whole status line for a second, then the line comes back, a key bringing it back at once;
  the three tests' windows back to 80 columns; usage, architecture, the changelog, an update to
  ADR-0033. *Files:* `src/view/mod.rs`, `src/view/pager.rs`, `src/source.rs`,
  `src/source/wordcraft.rs`, `src/source/epub.rs`, `tests/it/view_pager.rs`, `docs/`,
  `CHANGELOG.md`. *Verify:* `cargo test -j 2 --bin clv view::` (the sentence's shapes; the
  message alone on the line at any width; a moment of a second, started over by a key) and `cargo
  test -j 2 --test it view_pager` (at 80 columns, `D`, `]` and `s` on a PDF: the sentence on the
  whole line, the status line back after a second, and at once with a key).
- [ ] **A key that goes to the contents** (the owner's, 2026-11-16) — the contract's
  `Source::contents` (a `Landmark`: what the place is called and its name among the source's
  links); the viewer's `contents` command, `T`, through Spec-0013's catalog and keymaps, offered
  only where a place is declared (`?`, and `Contents: T` on the status line for a second when the
  file opens), going there as a link followed does, so Backspace comes back; the book's place: the
  chapter its package marks, else one guessed by its links (the first eight chapters; three other
  chapters or more; most of its links), else the Contents page; usage, architecture, the
  changelog, an update to ADR-0038 with the guess measured on the 22 books. *Files:*
  `src/source.rs`, `src/source/epub.rs`, `src/view/commands.rs`, `src/view/pager.rs`,
  `tests/it/view_pager.rs`, `docs/`, `CHANGELOG.md`. *Verify:* `cargo test -j 2 --bin clv
  epub::tests` (`a_book_marks_its_own_page_of_contents`, `the_contents_are_the_books_own_page_…`,
  `a_chapter_of_links_costs_the_guess_…`), `cargo test -j 2 --bin clv view::` (the command only
  where a place is declared) and `cargo test -j 2 --test it view_pager` (`epub_t_goes_…`: from a
  page in the middle to the Contents page, to the book's own marked, to it guessed, Backspace
  back, the status line's `Contents: T` at opening; `t_is_offered_…`: never for a PDF); `Manual:
  in kitty, a Project Gutenberg EPUB 3 book: "Contents: T" for a second when it opens, T from a
  chapter lands on the book's own contents, Backspace back; a Standard Ebooks book: T lands on
  the last page`.

## Done

- [x] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [x] Tests added, each seen failing without the change: `a_documents_pictures` (zip and tar),
  `epub::tests`, `tests/it/epub.rs`, `view_pager::epub`, `view_pager::epub_links`,
  `view_pager::epub_t_goes`, `view_pager::t_is_offered`.
- [ ] Manual: in kitty, a Gutenberg epub with pictures and a contents page: the cover first, chapters
  a page each, its pictures, `/` across chapters, `D`, `+`, and `f` to a chapter, `T` to its own
  contents and Backspace back; a book without a contents page of its own: `T` to the Contents page
  at its end, `f` to a chapter; a directory of 20
  epubs as a list, timed, its tiles their covers; a Markdown README in a zip with its screenshots.
- [x] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- 2026-11-15 — Draft (spec-new-auto, for review).
- 2026-11-15 — rbook in a worker process, not a parser of clv's own; Spec-0011's links, now in,
  followed in a book and never to a file beside it.
- 2026-11-15 — The book's table of contents as a Contents page (task 5), the overlay left to the
  backlog.
- 2026-11-15 — Ready: the owner's answers in; fixed-layout books through Blitz in the backlog.
- 2026-11-15 — Implementing ([notes](0012-show-an-epub-as-a-book-impl-20261115_2208.md)).
- 2026-11-16 — The five tasks' code, tests and docs in, with ADR-0037 and ADR-0038. Tasks 3 to 5
  and Done's manual item stay open for their checks in kitty, which are the owner's; the worker
  was found never to be killed by a table of contents within its 1 MiB, nor by `application/fontè`
  or a chain of `refines`, at rbook 0.7.13 (the notes say what each test does instead).
- 2026-11-16 — A sixth task, the owner's: what a key says where its setting isn't, shorter and on
  the whole status line for a second.
- 2026-11-16 — Rebased onto main after Spec-0013: where this spec says `d` for the paper and `u`
  to come back from a link, the keys are now `D` and Backspace.
- 2026-11-16 — The owner's decisions on the contents: the Contents page at the book's end, for
  every book with a table of contents, saying clv made it (task 5 and its requirement revised);
  a seventh task, a key that goes to the contents, the book's own where marked or guessed.
