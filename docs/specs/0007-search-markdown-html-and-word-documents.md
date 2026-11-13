# Spec-0007: Search Markdown, HTML and Word documents

- **Status:** Draft
- **Date:** 2026-11-13
- **Related:** [Spec-0006](0006-search-a-pdfs-text.md) (the search this extends, and its
  contract); [README's Plans](../../README.md#plans) ("Search!");
  [ADR-0018](../adr/0018-wordcraft-stays-before-its-proofing-needs-a-checkout-to-build.md) (the pin
  that keeps automatic hyphenation);
  [ADR-0012](../adr/0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md) (what a
  document may cost to lay out);
  [ADR-0024](../adr/0024-pages-to-a-row-of-markdown-and-html-are-columns-and-the-last-key-wins.md)
  (columns, a relayout);
  [A page the window's size](../usage.md#a-page-the-windows-size)

## Summary

Spec-0006 searches PDFs through `Source::texts`: chunks of text in the document's order, and the
rectangles a range of a chunk covers, asked for when its page is on screen. This slice gives
wordcraft's documents (Word, ODT, RTF, Markdown, HTML) their text through the same contract, a
paragraph a chunk: its own text, not the lines it was broken into, so a word wordcraft hyphenated
at a line's end (the pin keeps automatic hyphenation, ADR-0018), or a paragraph running onto the
next page, is found whole. A match's rectangles come from wordcraft's own selection
(`DocLayout::selection_rects`), across lines, hyphens, bidirectional text and pages. And a match is
a paragraph and a range in it, which a relayout (`[ ]`, `{ }`, `+ -`, `, .`, a resize) doesn't
change, so a search outlives one.

## Requirements

- Wordcraft's documents are searched as PDFs are (Spec-0006's typing, kept search and highlights),
  their body: tables' cells included, headers, footers, notes, text boxes and comments not.
- A chunk is a body paragraph, its text the paragraph's own; a match within a paragraph is found
  across its lines and pages, whatever wordcraft broke or hyphenated there. A match across two
  paragraphs isn't.
- A match's rectangles are its selection's: over a hyphen drawn at a line's end, over each visual
  piece of a bidirectional line, on each page it runs over.
- A relayout keeps the search: the query, whether it is being typed or kept, the chunks' text (the
  document is the same), and the match you are on; only its rectangles are asked for again, from the
  new layout, and those still wanted from the old one are dropped.
- On Markdown's and HTML's dark paper (`d`), the tint reads as on white.

## Out of scope

- Wordcraft's other stories: headers, footers, notes, text boxes, comments.
- Plain text, until it opens ([README's Plans](../../README.md#plans)).

## Approach

**Files:**

- `src/source/wordcraft.rs`: `Paged::texts`. The chunks are the body's paragraphs in order, tables'
  cells' among them; `chunk_at(page)` from the paragraphs each page starts with, which `Paged`
  already keeps for `anchor`. A paragraph's text from the document; a range's rectangles from
  `DocLayout::selection_rects(doc, a, b, page_hint)` (wordcraft's `layout/src/hit.rs`), its
  positions a `Pos` in the body at the range's byte offsets, on char boundaries.
- `src/view/pager.rs`: the search handed from the old level to the new across a relayout, the
  level being replaced whole today.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- 🔴 **Where the rectangles are found** — `selection_rects` needs the document and its layout. The
  render pool lays the document out on its own threads (`wordcraft.rs`'s `Cache`); a second layout
  for the text is some 200 MB more for 500 pages (usage's figure for what pages hold). The pool's
  layout lent to `Texts`, a second laid out only once a match is on screen, or another way: measure,
  then choose.
- 🟡 **The paragraphs' text** — the document's text model by `Path`, tables' cells nested; what a
  field, a tab or an inline object becomes in it.
- 🟡 **Paragraphs over the cap** — 1 MiB a chunk (Spec-0006): a paragraph past it, which ADR-0012's
  limits may allow.
- 🟡 **A paragraph across a page break** — the viewer asks where the matches are for the chunks of
  the pages on screen, from the chunk a page starts with to the next page's (`search.rs`'s
  `chunks_of`, from Spec-0006, where a chunk is a page). A paragraph that starts on one page and
  ends on the next is the first page's alone unless `chunk_of` says the next page starts with it
  too: say which in the backend, and check that a match in its tail is found and scrolled to with
  only the second page on screen. The status line's `page N unread` and `read N of M parts` are
  worded for chunks that aren't pages only as far as that.

## Tasks

To write once the 🔴 question is settled.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [ ] Tests added, each seen failing without the change: name them.
- [ ] Manual: …
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)); README's
  "Search!" comes out.

## Changelog

- 2026-11-13 — Draft, split from Spec-0006 after a review.
- 2026-11-13 — A paragraph a chunk, rectangles from `selection_rects`, after a second review.
