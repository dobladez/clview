# ADR-0038: An epub is one document through the backend that opens HTML, its package read by rbook in a worker process capped at 2 s and 256 MiB

- **Status:** Accepted
- **Date:** 2026-11-16
- **Deciders:** the owner chose rbook over a parser of clv's own, and one spec for the whole, on 2026-11-15 ([Spec-0012](../specs/0012-show-an-epub-as-a-book.md))

## Context

An epub showed as the zip it is: a list of its files, each chapter opened alone as HTML, without
its pictures. A book is more than that list: a package (the OPF) says which of its files are
chapters and in what order, and a reader wants to page through them, search them and follow their
links as one text.

Two things had to be chosen: what reads the package, and what a book is to the viewer.

[rbook](https://github.com/DevinSterling/rbook) reads EPUB 2 and 3 packages, their manifests,
spines, covers and tables of contents. Read at 0.7.13 for this, it is not made for hostile input:
asking whether a manifest item is a font slices its media type at a byte, and panics on one with a
character past ASCII there (`application/fontè`); it reads the package and the table of contents
whole, whatever their size, and builds some kilobyte of heap for every element of the package's
metadata; a table of contents nests without a bound and is dropped by recursion; and a table of
contents it can't read fails the whole book. clv never panics on a file
([Rules](../../CONTRIBUTING.md#rules)), and bounds what a file costs before it spends it.

The viewer has two shapes for many things shown together: a list of files, each a tile
(`Joined`), and one source of many pages. Search, links, relayout and the settings of a document
([ADR-0022](0022-markdown-and-html-reflow-to-a-width-in-characters-as-a-new-level.md),
[ADR-0023](0023-the-window-is-the-page-of-markdown-and-html.md)) all work on one source.

## Decision

1. **rbook reads the package, in a worker process of clv's own**, `clv --worker epub`, as Blitz
   and merman work
   ([ADR-0026](0026-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md),
   [ADR-0031](0031-mermaid-diagrams-through-merman-in-a-worker-process.md)): a panic, an abort or
   a heap past its cap costs the worker, and so the book, never the viewer.
2. **The worker is given the package alone, never the book.** The backend reads
   `META-INF/container.xml` (64 KiB at most) for the package's path, with a look of its own for
   one attribute, and the package (4 MiB at most), and sends the path and the package. The worker
   puts them in a zip of its own, in memory, under a container it writes that names the package,
   and has rbook read that with `skip_toc`. So nothing large crosses the pipe, the worker reads no
   file, rbook never parses the book's own container, and a table of contents it can't read
   doesn't cost the reading order.
3. **It answers the reading order and nothing else**: the spine's items in order, each its file's
   path from the book's top and its media type. The worker never asks rbook what kind of file an
   item is. The backend checks the answer as any input: 4,096 chapters, 4 KiB a path, every path
   brought to one from the book's top or dropped.
4. **Capped at 2 s of CPU and 256 MiB of heap**, one worker a book, gone once it has answered, one
   at a time. The cap is not the costliest package rbook accepts: it is above every package a
   book could really have, and below what a package made to cost the most takes, which is
   refused by it.
5. **A book is one HTML document, opened by whichever backend opens HTML**, found by what it
   opens, never by name, as a diagram's SVG and a spreadsheet's PDF are. The chapters are joined
   in the spine's order, each in a `<div>` that starts a page (`page-break-before: always`), after
   1,024 bytes of clv's own, so that nothing a book writes is among the bytes a backend is chosen
   by. Every chapter is there, `linear="no"` ones where the spine has them.
6. **The join is a pass of clv's own over each chapter**, cut into tags as the documents'
   importer cuts it (`markup.rs`): the chapter's own frame and head dropped; ids given their
   chapter's number, since two chapters' `id="s1"` would meet in one document; pictures' and
   links' paths written from the book's top, since the document is at it; a link to a file marked
   as one the book holds or doesn't, and answered by the book's source, so that no link is ever
   looked for beside the `.epub` on disk.
7. **The book's stylesheets are left out**, linked or in a `<style>` (the owner's answer,
   2026-11-15): the importer reads every `<style>` of a document into one stylesheet, so one
   chapter's rules would reach all of them, and the class names tools write (`calibre1`) meet.
   A `style` attribute stays.
8. **The book is assembled within bounds of its own, below the documents'**
   ([ADR-0012](0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md)): 48 MiB written,
   400,000 marks, 6 MiB of text between its tags, counted as it writes. The chapter that would
   pass one is left out with those after it, and said, where the documents' backend would refuse
   the whole file.
9. **The book is assembled again each time it is read**, from the reading order kept: to open, on
   its first render, on each relayout. An open book holds that order and where its file is, not
   its text, since a list may hold many
   ([ADR-0019](0019-a-list-opens-its-files-when-their-tiles-are-wanted.md)).

## Evidence

Apple M3 Max, macOS 26.6.2, rbook 0.7.13 with zip 9.5.1, [`0038-measure/`](0038-measure/Cargo.toml):
`cargo build --release` there, then `target/release/measure read <x.epub>…`, which reads each
book's package as the worker does, from a zip holding the package alone, and says the time and
the most heap held, as the allocator counts it; and `measure gen <kind> <n> <out.epub>`, which
writes the hostile ones.

**A zip of the package alone is enough**: with `strict(false)` and `skip_toc(true)`, rbook gave the
spine, the manifest and the cover of all 22 books below without asking for another file.

| Package | OPF | Read in | Heap |
|---|---|---|---|
| 22 real books: Project Gutenberg's 11, 74, 84, 98, 1342, 1400, 1661, 2701, 5200 and 28885, each as EPUB 2 and EPUB 3, and Standard Ebooks' *Alice's Adventures in Wonderland* and *Frankenstein* | 2.6 to 33 KiB | 0.03 to 0.24 ms | 26 to 175 KiB |
| `gen items 40000`: a manifest of 40,000 items, a spine of 4,096 | 2.9 MiB | 17 ms | 24 MiB |
| `gen refined 100000`: 100,000 metadata entries refining one | 3.9 MiB | 36 ms | 75 MiB |
| `gen refines 70000`: a chain of 70,000, each refining the one before | 3.8 MiB | refused by rbook, `Cycle detected in metadata refinements`: a chain of 200 is read, one of 300 refused the same | |
| `gen metas 599000`: 599,000 empty `<meta/>` | 4.0 MiB | 189 ms | 564 MiB |
| `gen font 1`: an item of media type `application/fontè` | 455 bytes | 0.04 ms, read | 5 KiB |

So a real package is five orders below either cap; a package as large as is read, of items or
metadata a book could have, takes a third of the heap cap; and only one made of nothing but empty
elements passes it, at some 970 bytes of heap an element. Through clv, that one is refused in
0.14 s with `its package couldn't be read`, the worker killed past its heap
(`a_hostile_package_costs_the_book_and_never_the_viewer`, `tests/it/epub.rs`).

So rbook follows a chain of `refines` only so far, and what the spec feared of it, a recursion
past the stack, isn't there at this version. In a debug build that chain makes rbook panic rather
than refuse (`attempt to add with overflow`, `epub/parser/package/metadata.rs:425`, where a release
build wraps and then calls it a cycle): the worker catches it, and the book is refused with the
same words as for a worker killed.

**Opening real books**, the 22 above, `/usr/bin/time -l clv x.epub --protocol kitty --pages 1`,
clv's dev profile (its dependencies optimized): the worker started, the package read, every
chapter joined and the whole book laid out, for its first page.

| Book | Chapters | To its first page | Memory |
|---|---|---|---|
| *Metamorphosis* (5200, EPUB 3), the quickest | 6 | 0.11 s | 47 MiB |
| *Alice's Adventures in Wonderland* (11, EPUB 3; Standard Ebooks') | 15; 20 | 0.16 s; 0.19 s | 57 MiB; 61 MiB |
| *Frankenstein* (84, EPUB 3; Standard Ebooks') | 32; 38 | 0.30 s; 0.29 s | 104 MiB; 86 MiB |
| *A Tale of Two Cities* (98, EPUB 3) | 64 | 0.58 s | 142 MiB |
| *Great Expectations* (1400, EPUB 3) | 77 | 0.77 s | 175 MiB |
| *Moby-Dick* (2701, EPUB 3; EPUB 2) | 13; 28 | 0.83 s; 0.96 s | 195 MiB; 280 MiB |
| *Pride and Prejudice*, illustrated (1342, EPUB 2), the slowest | 18 | 1.13 s | 212 MiB |

All 22 one after another take 11 s; a list opens four at a time.

**A page break on a `<div>`** reaches its first paragraph in the documents' backend, at the Word
backend's pin: `<div style="page-break-before: always">` starts a page before a heading, a
paragraph, bare text, a list, a block quote, a `<pre>`, a rule or a picture, nested or not, and
on the document's first `<div>` it makes no blank page. Tried with `clv x.html --protocol kitty`,
counting the pages printed. One case doesn't: a `<div>` whose first content is a `<table>` starts
no page, and neither does the table with the style on itself, nor an empty paragraph before it.
A paragraph of one zero-width space does, so the join writes one, a point high, before a table
that is the first thing a chapter shows.

## Alternatives considered

- **A parser of clv's own** over the container and the OPF, on quick-xml, in the viewer: some
  hundreds of lines for the spine, and as many again for the two kinds of table of contents, all
  of it reading untrusted XML in the viewer's process. The owner chose rbook, which knows the
  format's corners (EPUB 2's guide and NCX, EPUB 3's nav, fallbacks, covers of both kinds).
- **rbook in the viewer's process, guarded**: a `catch_unwind` for its panics, sizes checked
  before it reads. Nothing in the viewer's process stops a recursion past the stack, which aborts,
  or bounds the heap of a parse under way; and each guard would be against rbook's code as it
  stood, to be checked again at every version.
- **The worker given the epub**, by its path or its bytes: it would read a file, or 256 MiB would
  cross its pipe, to save the backend two small reads it has to bound anyway.
- **The book's own container given to rbook**: one more hostile file for it to parse, and the
  backend's look and rbook's could then disagree on which package it names.
- **A source a chapter, joined as a list's tiles are**: each chapter laid out alone, cheaply, and
  only when reached. But a search would stop at a chapter's end, a link to another chapter would
  be a link to another file, and `]` would lay one chapter out again, not the book.
- **A heap cap above the costliest package rbook accepts**, as Mermaid's worker has for merman's
  costliest diagram: 768 MiB or more for one book's reading order, to show a package no book has.
- **The book refused when it passes the documents' bounds**: a long book would show nothing, where
  cutting it shows its start and says what is missing.

## Consequences

- **Positive:** a book is read, searched, reflowed and followed as any document; nothing in rbook
  can cost more than the book; the documents' backend knows nothing of books, and the epub's
  nothing of it but that something opens HTML.
- **Accepted:**
  - A worker is started for every book opened, some milliseconds each, a list's tiles included,
    one at a time.
  - A book looks plainer than in a reader app: its own stylesheets aren't read
    ([Known issues](../usage.md#known-issues)).
  - A book is laid out whole to open, as any document: a novel takes a second, and a list of
    books that for each tile wanted.
  - A chapter whose first content is a table starts a point lower than any other.
  - A book past the bounds shows its first chapters only: 6 MiB of text is some 3,700 pages.
  - The reading order is all that is read of the package here: its cover, its fallbacks, its
    protected files and its table of contents are the same worker's to answer, as they come.
- **Follow-ups:** the book's stylesheets applied a chapter at a time
  ([backlog](../backlog.md#more-documents)); a cheaper tile, the cover alone, which is a contract
  change of its own; a worker kept between books, as Mermaid's is, if a shelf of books turns out
  slow to list.
- **Revisit when:** rbook moves (measure again, and read what it now does with a media type and
  with a chain of `refines`); the documents' backend can be given a stylesheet for a part of a
  document; or its importer stops carrying a page break from a `<div>` (`an_epub_is_a_book_its_chapters_in_order_each_on_a_new_page`
  fails then).
