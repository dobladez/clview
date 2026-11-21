# ADR-0041: A book's tile is its first page alone, and the book is opened whole when gone into

- **Status:** Accepted
- **Date:** 2026-11-21

## Context

A list opens a file when its tile is wanted, and keeps what it opened to for when the reader goes
into it ([ADR-0019](0019-a-list-opens-its-files-when-their-tiles-are-wanted.md)). An epub opens as
one HTML document, its chapters joined, through the backend that opens HTML
([ADR-0038](0038-an-epub-as-one-document-its-package-read-by-rbook-in-a-worker.md)). That backend
lays the document out whole to know its pages, and the first render lays it out again
([ADR-0005](0005-word-documents-through-wordcrafts-readers-layout-and-renderer.md)). So a book's
tile cost two layouts of the whole book, for a tile that shows page 1. Going into the book lays it
out a third time, for the window, since a list lays its documents out for a preview's room
([Relayout](../architecture.md#relayout)).

ADR-0005 rejected a layout of the first page alone: wordcraft lays out a document whole. But the
epub backend writes the document itself, so it can write a document that holds only page 1.

## Decision

- A backend opens a list's file through `Backend::open_tile`, which calls `open_at` by default.
- The epub backend writes a tile's document as its first page alone: the cover's page, or else
  the first chapter shown. It reads no chapter after that one, and asks the worker for no table of
  contents.
- A source opened that way says so (`Source::tile`). When the reader goes into it, the pager asks
  it for a relayout whatever its values and its room. The book's relayout opens it whole, laid out
  for the window.
- A tile says only what its first page and the book's package tell. A chapter that only a read
  shows to be unreadable is said when the book is opened, not on its tile.

## Evidence

clv 0.6.0 (before) and this change (after), release builds, macOS 26.6.2, Apple M3 Max. The list
is a real directory of 36 epubs, 0.2 MB to 35 MB each, printed as a list:

```sh
/usr/bin/time -l clv --protocol kitty ~/books/*.epub > /dev/null
```

| | Time | CPU time | Peak memory |
|---|---|---|---|
| Before | 13.8 s | 13.2 s | 1,106 MB |
| After | 1.0 s | 0.8 s | 240 MB |

Three runs each; the table shows the median. Before, `sample` of the run put about 95% of the
main thread in wordcraft's layout (`wordcraft_layout::layout`), half of it in the first render.
Each book printed alone (`--pages 1`) took 0.1 s to 1.15 s. A page that holds only one book's
cover took 0.05 s, against 0.74 s for that book's page 1.

35 of the 36 tiles were the same PNG, byte for byte. The other tile lost a 1-pixel line at its
bottom edge. The book's second chapter starts with an `<hr/>`, and wordcraft drew it at the end
of the page before the page break. The book opened whole still shows it there.

## Alternatives considered

- **Keep the open's layout for the first render**: halves the cost of every document, not only
  books, but still lays out each book whole for its tile. It is a separate change, worth making
  for a document opened alone; ADR-0005 has its cost in memory.
- **A tile of the cover picture, through the image backend**: as quick, but a book with no cover,
  or whose first chapter shows the cover, would need the HTML path too. It also needs a way to name
  another backend's source as the book's.
- **A flag in `Room` that says it is a tile's**: no new method, but a room is the window's area,
  and every backend would have to look for the flag.
- **The pager opens the file again on going in**: the pager would need a second path for a file
  gone into. The relayout already shows the old pages while the new layout runs, and hands a
  link's place on to the new layout.

## Consequences

- **Positive:** a list of books shows its tiles more than ten times sooner, and holds a quarter of
  the memory. A book gone into costs one whole layout, as before.
- **Accepted:**
  - A tile's warnings say less than the book's: not a chapter after the first that can't be read,
    nor pictures missing in later chapters, nor a table of contents that can't be read.
  - A book gone into shows one page, its tile's, until its whole layout is ready.
  - A tile can differ from the book's page 1 where the next chapter changes how page 1 is laid
    out, as the `<hr/>` above does.
- **Follow-ups:** the first render could take the open's layout (the first alternative). Other
  documents in a list (Markdown, HTML, Org) could have tiles of their first page the same way.
- **Revisit when:** wordcraft can stop a layout after a page, which would give every document a
  tile of its first page.
