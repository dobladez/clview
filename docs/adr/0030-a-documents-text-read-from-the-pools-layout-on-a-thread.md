# ADR-0030: A document's text read from the pool's layout, on a thread

- **Status:** Accepted
- **Date:** 2026-11-13
- **Deciders:** the owner, in [Spec-0007](../specs/0007-search-markdown-html-and-word-documents.md)

## Context

A search asks a backend for its text in chunks, and for the rectangles a range of a chunk covers
([Searching a file's text](../architecture.md#searching-a-files-text-searchrs-pagerrs)). A PDF's
text is read in a worker process, since a page's text can't be bounded before it is read
([ADR-0027](0027-a-pdfs-text-read-in-a-worker-process-for-its-heap-cap.md)). A Word document's,
an ODT's, an RTF's, Markdown's and HTML's comes from wordcraft: a paragraph's text, and where a
range of it is, from a layout. Its pool lays the document out once, on its first render, and keeps
that layout for the renders after ([ADR-0005](0005-word-documents-through-wordcrafts-readers-layout-and-renderer.md)).

## Decision

A document's text is a body paragraph a chunk, read on a thread of the search's own
(`source/texts.rs`), from the layout the pool renders with: the pool and the search share one
`Cache` while either holds it (`Document::shared`, a `Weak`), so the document is laid out once for
both, and let go of when both are gone. A range's rectangles are wordcraft's own selection of it
(`DocLayout::selection_rects`). No worker process.

## Evidence

- What a document costs to lay out is bounded before it is laid out, by its marks and its text
  ([ADR-0012](0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md)), and opening lays
  it out once already; a paragraph's text and its selection are small beside its layout. A
  paragraph's text is capped at 1 MiB, as any chunk is.
- A second layout for the search would cost what the pages hold shown: some 200 MB for 500 pages
  ([usage](../usage.md#a-page-the-windows-size)).
- Rust's `releases.md` (894 KB, the 1.95.0 toolchain's `share/doc/rust/html/releases.md`), opened
  through the backend in a debug build (`opened`, as `wordcraft.rs`'s tests do): 7,341 body
  paragraphs on 462 pages, laid out in 0.46 s; tokio 1.54.1's `CHANGELOG.md` (195 KB), 2,126 on
  134. Real documents are far under the search's 100,000 chunks, which stays its cap.
- `the_pool_and_a_search_lay_a_document_out_once` (`src/source/wordcraft.rs`) counts the layouts.

## Alternatives considered

- **A worker process, as a PDF's.** It would lay the document out a second time in that process,
  and send the rectangles back: the cost of a second layout, for a bound the layout already has.
- **A second layout in the viewer's process, the search's own.** Simpler to share nothing, at some
  200 MB for 500 pages more while a search is open.
- **The `Document` keeping the shared layout (`Arc`)**, as the spec first had it. The pager makes a
  fresh pool when a file is entered from a list, so that the old one lets go of what its files
  rendered with (`Pager::enter`); a layout the `Document` kept would keep every Markdown file of a
  list laid out for as long as the list is open. Hence the `Weak`.
- **A chunk a line, or a page**, as a PDF's is a page: a word hyphenated at a line's end, or a
  paragraph across a page break, would not be found whole.

## Consequences

- **Positive:** a search costs no second layout; `/` before the first render lays the document
  out for the pool too; a match across lines, hyphens, bidirectional text and pages is placed as
  wordcraft selects it, the hyphen drawn at a line's end included.
- **Accepted:** the search's thread waits on the layout's lock (an `RwLock` that a render takes to
  draw a picture into the document) while it reads a paragraph; wordcraft's calls there are caught
  if they panic, and a panic costs the search that chunk, answered as unread.
- **Revisit when:** a document's paragraph text or selection is found to cost what its layout
  doesn't bound; or wordcraft gives a document's text without a layout.
