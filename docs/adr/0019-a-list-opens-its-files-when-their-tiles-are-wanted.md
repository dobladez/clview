# ADR-0019: A list opens its files when their tiles are wanted

- **Status:** Accepted
- **Date:** 2026-11-03

## Context

A directory, an archive or several files are a list, a tile for each file by its first page
([Lists and levels](../architecture.md#lists-and-levels)). `Opener` opened every file before the list
showed, one at a time on the main thread, because a tile was built from an opened source. What a file
costs to open is whatever its backend needs to know all its pages: a document's whole layout
([ADR-0005](0005-word-documents-through-wordcrafts-readers-layout-and-renderer.md)), a PDF's whole file,
a workbook printed ([ADR-0006](0006-spreadsheets-printed-by-gridcraft-to-a-pdf-the-pdf-backend-shows.md)).
A directory of 518 Markdown files took over four seconds to show anything, nearly all of it laying
out documents to keep only their page sizes, which a tile's first render then lays out again.

A list needs little of that. A tile's place comes from how many tiles there are (`fit::tiles`), the
filter goes by names, and a file's first bytes say which backend shows it. A page's size matters only
for a tile near the screen, and a file's page count only on going into it.

## Decision

- A backend whose `open` costs more than a list should spend says so (`Backend::deferred`): documents,
  PDFs, drawings and spreadsheets. Such a file, among others or in a container, becomes a `source::Later`
  once its first bytes are read: closed, and not opened until its tile is wanted. A file alone is opened
  at once, since it is shown at once, and a pipe or anything named in `/dev`, which can't be read from
  its start again, as it is reached.
- Images and archives stay opened as the list is listed. An image's open reads its header and nothing
  more, and whether a tar is a container can depend on what it unpacks to.
- Files are opened on threads of their own (`source::Openers`), never on the thread that reads keys.
- A file that turns out not to open **keeps its tile**, its name marked, saying why when chosen. It is
  not removed when found out, since that would move the tiles after it under the reader, and change what
  `--pages` numbers.
- Until a file is open its size is a stand-in, and **nothing is rendered at it**: a request made
  from it only opens the file, and is answered by `Frame::asked_again`, on which the pager lays the tile out
  afresh with the true size.
- Printing waits for each file as it reaches it (`Source::settle`).

## Evidence

clv 0.2.0 (before) and this change (after), release builds, macOS 26.6.2, Apple M3 Max, on three
directories: 518 Markdown files of 11.7 MB in all (a median of 19 KB, the largest 161 KB), 200 CSV files
of 400 rows, and 200 copies of a 4.6 MB PDF.

```sh
/usr/bin/time -l clv "$dir" --pages 1 --no-pager --protocol ascii > /dev/null
```

The pager's first screen was timed on script(1), 200 × 60 cells, `--protocol kitty --compression fast`,
by a program that answers `ESC [ c` and `ESC [ 16 t` as a terminal does, and notes the time of the
first status line and of each `a=t,f=100` in the output.

| Directory | First tile printed | Peak memory | Pager: first status line | Pager: 12 tiles sent |
|---|---|---|---|---|
| 518 Markdown, before | 4.72 s | 549 MB | 4.09 s | 4.33 s |
| 518 Markdown, after | 0.16 s | 222 MB | 0.09 s | 0.50 s |
| 200 CSV, before | 0.72 s | 43 MB | 0.71 s | 0.89 s |
| 200 CSV, after | 0.05 s | 19 MB | 0.11 s | 0.42 s |
| 200 PDFs, before | 0.77 s | 966 MB | 0.50 s | 0.68 s |
| 200 PDFs, after | 0.04 s | 36 MB | 0.11 s | 0.41 s |

A sample of the old opening of the Markdown directory (`sample $pid 3 1`) had 86% of it in
`wordcraft_layout::layout`, about 6% importing, and about 3% reading first bytes: what remains.
These are single runs of a machine in other use: a second run under load took the Markdown directory
4.5 to 7.5 s before and 0.09 s after.

## Alternatives considered

- **Open them all at once, across the cores.** Nothing changes for the reader, and no tile needs a
  stand-in. Not measured, but the wait would still grow with the list, up to the 4096 files one lists,
  wordcraft's font caches sit behind mutexes, a list of large PDFs would still hold all of them, and a
  compressed tar's files come out in one order. The cost should follow the screen.
- **Markdown and HTML only, whose first page is always US Letter.** Small, but it leaves documents,
  PDFs and spreadsheets as they were, and needs the same answer for a page count not yet known.
- **Keep opening everything after the list shows, to report what is broken as before.** Spends what
  this set out to save, for a report about files the reader may never come to.
- **Remove a file's tile when it turns out broken.** The list would keep the contents it had, but the
  tiles after it would move under the reader, and `--pages` numbers would stop being stable.
- **A disk cache of sizes and thumbnails.** Would make a second visit instant, but brings invalidation
  and one more input to distrust. Not needed at these numbers.
- **Rendering at the stand-in size, then again if it was wrong.** Saves a round trip for a file that
  happens to be US Letter, at the cost of a wasted render for one that isn't, and a backend's hint of
  its size for the round trip it saves.
- **Handing a document's first layout on to its first render, and more than two tiles rendering at
  once.** A tile still lays its document out twice, once to open and once to render, and renders two at
  a time (`Pools`). Twelve tiles in about 0.4 s after the status line doesn't call for either; the first
  would also hold a layout for every file opened and not yet rendered.

## Consequences

- **Positive:** a list shows in a tenth of a second however long it is, and holds only what the reader
  came near. Keys work while files open.
- **Accepted:** a broken PDF or document is found out when it is reached, not said first; one never
  reached is never reported. A broken picture is still left out with a warning, as its open costs
  nothing, so the two differ. Going into a tile not yet open waits for it, as does `c`. `Backend: Sync`
  and `Opened::Pages` holding a `Source + Send`, for the threads.
- **Follow-ups:** with `-r` at the 4096-file limit, reading first bytes may still take most of a second
  (an estimate, from 0.2 ms a file): that could move off the main thread too.
- **Revisit when:** a list's first screen takes long enough to feel, which would reopen the two
  above; or scrolling through opened files holds too much, since what a file opened to is kept until
  the list is left (a PDF, its whole file).
