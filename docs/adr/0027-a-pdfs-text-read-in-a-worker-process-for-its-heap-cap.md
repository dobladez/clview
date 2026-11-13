# ADR-0027: A PDF's text read in a worker process, for its heap cap

- **Status:** Accepted
- **Date:** 2026-11-13

## Context

A search ([Spec-0006](../specs/0006-search-a-pdfs-text.md)) needs every page's text, read once,
while the reader types. pdfcraft reads a page's text (`PageRenderer::filtered_text`: every glyph,
its Unicode and its box, then lines and word gaps) with no bound on what it holds meanwhile, and
a PDF is untrusted input: a page can hold millions of glyphs in a few megabytes of file. In the
viewer's process that memory can't be capped: a Rust thread has no heap of its own to limit, and
an allocation refused aborts the viewer. `unsafe_code` is forbidden.

[ADR-0026](0026-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md) met the
same problem for an SVG's HTML labels, and built the answer for any backend
(`src/source/worker.rs`): clv run again as a worker, its heap capped by the allocator and its CPU
time by the kernel, killed past either.

## Decision

1. **A PDF's text is read in a worker process of its own,** `clv --worker pdf-text`, started when
   a search starts (`/`) and killed when it ends: one worker a search, not a page. It is given the
   password and the PDF's bytes once, the copy the viewer shows, written from where the viewer
   holds them (`Link::request_shared`), never copied there and never read from the file again. It
   keeps the document parsed and answers one request at a time: a page's text as it is searched,
   or the rectangles a range of that text covers. Only `src/source/pdfcraft.rs` names pdfcraft,
   and is both sides of this worker.
2. **Its caps:**

   | Cap | Value | Why |
   |---|---|---|
   | The largest PDF searched (`MAX_PDF`) | 512 MiB | The worker holds it whole, as the viewer does. Four times the largest of 1,542 documents. Past it, `/` says so |
   | The worker's heap (`MAX_HEAP`) | the PDF's 512 MiB and 256 MiB more | 256 MiB is 16 times the most a page of those documents took, and 35 times the most parsing one took over its file. It holds a page of about a million glyphs |
   | CPU for a request: parsing, a page's text, a range's rectangles (`OPEN_CPU`, `PAGE_CPU`) | 5 s each | Of 64,940 pages, two took longer (14.0 s and 8.9 s, in one document); the next slowest took 3.9 s. The kernel stops the worker within a second past it; the viewer's deadline is three times it |
   | A page's text (`MAX_PAGE_TEXT`) | 1 MiB | What the viewer keeps of one chunk. 35 times the most text on a page measured |
   | The time reading a document's text takes in all (`DOCUMENT`) | 120 s of requests | The slowest document measured took 32 s, the next 3.6 s; 64,940 pages averaged 1.8 ms |
   | Workers killed for one document (`MAX_KILLS`) | 3 | The one document with pages past the CPU cap has two of them |

3. **A page past a cap costs the search that page.** The worker is killed (by the kernel past
   its CPU time, by its allocator past its heap, by the viewer past the deadline), the page's
   text is answered as unread with why, and the next page asked for starts another worker, given
   the PDF again. Past three workers killed, or the document's time, the rest is answered as
   unread at once, with why; past the time alone, the worker then alive still answers where the
   matches already found are. A PDF pdfcraft can't parse for its text, or one over 512 MiB, is
   unread whole.
4. **Ending the search kills the worker without waiting for it** (`worker::Kill`, taken from the
   `Link` once the worker is greeted): the thread that waits on the worker's answer finds it
   gone, and the pager's thread never waits on either.
5. **What is searched is the text pdfcraft reads, as it reads it:** every glyph on the page, an
   invisible layer's (OCR) too, in its reading order, a space where it found a word gap and
   between one line and the next. Text a clip hides is searched too: pdfcraft's text device drops
   glyphs outside the page, not those outside a clip within it, and the backend is given no clip
   to drop them by. It is a known difference in [usage](../usage.md#searching-a-documents-text),
   seen where a spreadsheet's string is cut off by the next cell.
6. **`worker.rs` gains what this needs, for any worker:** a heap cap for each kind
   (`Worker::heap`, where one constant served all), a payload shared rather than copied, written
   as its header and its bytes, and the `Kill` handle. The six workers alive at most are still all
   kinds' together.

## Evidence

Measured 2026-11-13 on an Apple M3 Max (14 cores, 36 GiB), macOS 26.6.2, Rust 1.95.0, pdfcraft at
`168a3cb`, with the measuring tool [`0027-measure/`](0027-measure/src/main.rs) beside this ADR: a
crate of its own, in its own workspace, outside clv's build. `measure run <x.pdf>` opens the PDF
as the worker does and reads every page's text, timing each and counting the heap with an
allocator of its own, as the worker's cap counts it; `measure gen off|grid <strings> <x.pdf>`
writes the hostile pages. Built with `cargo build --release` there; each line below is
`/usr/bin/time -l target/release/measure run x.pdf`.

**Real documents:** every PDF under one home directory's documents, 1,542 of them, 64,940 pages
(six more names weren't files that could be read).

| What | Measured |
|---|---|
| The largest file | 120.5 MiB |
| Parsing | at most 21.6 ms; the heap after it at most 7.3 MiB over the file |
| A page's text | 1.8 ms on average over all pages; a document's median page at most 28.6 ms |
| The slowest pages | 14.0 s, 8.9 s, 3.9 s and 1.7 s, all in one document of 764 pages (drawings of many paths, 18,466 glyphs at most); then 0.54 s, 0.49 s and 0.31 s; every other page under 0.2 s |
| A whole document | 32 s for that one; then 3.6 s, 2.8 s, 2.6 s. Books of 400 to 650 pages: 0.3 to 0.9 s |
| The most heap one page took | 15.7 MiB; the next 9.5 MiB |
| The most glyphs on a page, and the most text | 18,466 glyphs; 30,246 bytes |

**Hostile pages,** one page each, strings of ten glyphs:

| Page | Text read in | The most heap the page took | Maximum resident set |
|---|---|---|---|
| `gen off 100000`: a million glyphs run off the page (1.5 MiB) | 0.18 s | 0.2 MiB: pdfcraft drops them, and keeps 542 | 10 MB |
| `gen off 1000000`: ten million (15 MiB) | 1.73 s | 0.2 MiB | 25 MB |
| `gen off 3000000`: thirty million (46 MiB) | 5.19 s | 0.2 MiB | 57 MB |
| `gen grid 30000`: 300,000 glyphs kept on the page (1.2 MiB) | 0.10 s | 107 MiB | 116 MB |
| `gen grid 100000`: a million (3.9 MiB) | 0.35 s | 224 MiB | 365 MB |
| `gen grid 300000`: three million (11.7 MiB) | 1.19 s | 872 MiB | 1.05 GB |
| `gen grid 400000`: four million (15.6 MiB) | 1.46 s | 896 MiB | 1.37 GB |

So time grows with the glyphs drawn (0.17 µs each), kept or not, which only a CPU cap bounds; and
memory with the glyphs kept, a few hundred bytes each, which only a heap cap bounds. Neither can
be told from the file's size.

**Through the worker,** in clv's build at this commit (its `dev` profile, which is optimized), in
the pager on a pseudo-terminal (a throwaway script's: `forkpty`, 120 × 50 cells,
`TERM=xterm-kitty`, no `TMUX`, running `clv <file> --protocol kitty --per-row 1 --pager`, which
writes the keys and notes when the status line, the last `\x1b[2K\x1b[7m…\x1b[0m`, changes), from
the last key of `/a` to what the status line says:

| File | What happens | After |
|---|---|---|
| A five-page PDF | all five pages read: the worker started, greeted, given the PDF, asked five times | 36 ms |
| The same with 500 MiB of padding in an unused stream | all five pages read: 500 MiB through the pipe, and parsed | 0.12 s |
| The same with 600 MiB | `read 0 of 5 pages`, and that it is too large to search | 15 ms |
| Books of 641 and 629 pages | the first count 26 to 35 ms after the key, growing every 10 ms; all read 0.6 to 1.0 s after it | |
| `gen grid 100000` | `page 1 unread`: more text than a search keeps of one page (1.1 MB) | 0.42 s |
| `gen grid 400000` | `page 1 unread`: too much memory; the worker aborted at its heap cap | 1.12 s |
| `gen off 3000000` | read: 5.13 s of CPU, inside the cap's second of grace | 5.13 s |
| `gen off 6000000`: sixty million glyphs (92 MiB) | `page 1 unread`: took too long; the kernel stopped the worker | 6.1 s |

Keys were answered throughout. `tests/it/view_pager.rs` runs the heap cap's case: a page of four
million glyphs among ordinary ones costs the search that page, and four of them the rest; no
worker is left when the search is cleared, the file left, or the pager quit.

**Text a clip hides.** A workbook whose A1 holds a long string and B1 another value, a number
wider than its column, a hidden column, and a formula, searched through the PDF it prints to:

| Cell | Searched for | Found |
|---|---|---|
| A string cut off by the next cell | a word in the part cut off | yes: pdfcraft reads it, clipped or not |
| A string that overflows into empty cells | its last word | yes |
| A number wider than its column | its digits; `###` | no; it prints as `1E+14`, which is found |
| A hidden column's cell | its text | no: it isn't printed |
| A formula | its value; its text | the value; not the formula |

## Alternatives considered

- **pdfcraft's `RenderPool`, which reads text on threads and can cancel a request**
  (`RequestKind::Text`): cancellation bounds time, not memory, and the threads are the viewer's.
  One page of three million glyphs would take the viewer past a gigabyte before anything could
  stop it.
- **A cap on the file's size alone:** the hostile pages above are 4 to 16 MiB.
- **A cap on a page's content stream:** a stream's length says little of its glyphs (a form
  drawn many times, a compressed stream), and reading it to count them is the work to be bounded.
- **One worker a page:** 5 ms to start one and the PDF sent each time, for every page of a
  book. A worker a search, replaced when killed, sends the PDF once in the ordinary case.
- **The text read ahead, when the file opens:** a worker and its copy of the PDF for every file
  shown, whether searched or not. `/` starts it, and a book is read in under a second.
- **Dropping glyphs outside their clip in the backend:** the backend sees glyphs and boxes, not
  clips; it would take pdfcraft's text device recording clips, or a second interpretation of the
  page here.
- **A test-only override of the caps:** a knob in the product for the tests' sake (ADR-0026); the
  heap cap's test fits the real cap, in about a second a kill.

## Consequences

- **Positive:** a PDF's text costs the viewer what it keeps of it (at most 64 MiB, in
  `search.rs`), whatever the PDF; a page that would take gigabytes costs one page of the search
  and about a second. The keys never wait on the text.
- **Accepted:** the PDF is held twice while a search is open, in the viewer and in the worker,
  up to 512 MiB each. A PDF over 512 MiB isn't searched. A real page that takes more than five
  seconds of CPU or 256 MiB to read is left unread, and a document with more than three of them
  is left unread from the fourth on: one document of 1,542 has two such pages. Text behind a clip
  is found, and its tint will fall where it would be drawn.
- **Follow-ups:** [Spec-0007](../specs/0007-search-markdown-html-and-word-documents.md) gives
  Markdown, HTML and Word documents their text through the same contract, without this worker.
  pdfcraft could give each glyph's clip, or drop clipped glyphs itself.
- **Revisit when:** pdfcraft bounds what reading a page's text holds, or reads it cancellably
  within a memory budget; or moving its pin changes what a page costs: measure again with
  `0027-measure/`, its `rev` moved with clv's.
