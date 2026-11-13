# Spec-0007: Search Markdown, HTML and Word documents

- **Status:** Implementing
- **Date:** 2026-11-13
- **Related:** [Spec-0006](0006-search-a-pdfs-text.md) (the search this extends, and its
  contract) and [its implementation notes](0006-search-a-pdfs-text-impl-20261113_1121.md);
  [README's Plans](../../README.md#plans) ("Search Markdown, HTML and Word documents");
  [ADR-0005](../adr/0005-word-documents-through-wordcrafts-readers-layout-and-renderer.md) (a
  document laid out again by its pool, and what a layout holds);
  [ADR-0018](../adr/0018-wordcraft-stays-before-its-proofing-needs-a-checkout-to-build.md) (the pin
  that keeps automatic hyphenation);
  [ADR-0012](../adr/0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md) (what a
  document may cost to lay out);
  [ADR-0024](../adr/0024-pages-to-a-row-of-markdown-and-html-are-columns-and-the-last-key-wins.md)
  (columns, a relayout);
  [ADR-0028](../adr/0028-matches-tinted-into-the-pages-pixels.md) (the tint);
  [A page the window's size](../usage.md#a-page-the-windows-size)

## Summary

Spec-0006 searches PDFs through `Source::texts`: chunks of text in the document's order, and the
rectangles a range of a chunk covers, asked for when its page is on screen. This slice gives
wordcraft's documents (Word, ODT, RTF, Markdown, HTML) their text through the same contract, a
paragraph a chunk: its own text, not the lines it was broken into, so a word wordcraft hyphenated
at a line's end (the pin keeps automatic hyphenation, ADR-0018), or a paragraph running onto the
next page, is found whole. A match's rectangles come from wordcraft's own selection
(`DocLayout::selection_rects`), across lines, hyphens, bidirectional text and pages, read from the
layout the render pool already holds, so a search costs no second layout. And a match is a
paragraph and a range in it, which a relayout (`[ ]`, `{ }`, `+ -`, `, .`, a resize) doesn't
change, so a search outlives one.

## Requirements

- Wordcraft's documents are searched as PDFs are (Spec-0006's typing, kept search, `n`, `N`, `p`
  and highlights), their body: tables' cells included, headers, footers, notes, text boxes and
  comments not.
- A chunk is a body paragraph, its text the paragraph's own, each inline object as its text (a
  field as its result) or nothing; a match within a paragraph is found across its lines and pages,
  whatever wordcraft broke or hyphenated there. A match across two paragraphs isn't.
- A match's rectangles are its selection's: over a hyphen drawn at a line's end, over each visual
  piece of a bidirectional line, on each page it runs over.
- A paragraph across a page break is on both pages: with only the second on screen, a match in its
  tail is tinted, gone to and scrolled to; with only the first, one in its head.
- A relayout keeps the search: the query, whether it is being typed or kept, the chunks' text (the
  document is the same), and the match you are on; only its rectangles are asked for again, from the
  new layout, and those still wanted from the old one are dropped.
- The status line counts what is left unread in pages, for every backend (`· page 40 unread`,
  `· read 120 of 500 pages`), never `parts`: a page is unread while a chunk on it is.
- On Markdown's and HTML's dark paper (`d`), the tint reads as on white.
- A paragraph over a chunk's 1 MiB is left unread, its page said so; the rest is searched.

## Out of scope

- Wordcraft's other stories: headers, footers, notes, text boxes, comments.
- Plain text, until it opens ([README's Plans](../../README.md#plans)).
- A match across two paragraphs, or a table's cells.

## Approach

**Files:**

- `src/source.rs`: `Texts::chunk_of(page)` becomes `chunks_on(page) -> Range<usize>`, the chunks
  with text on the page, first to last; two pages share a chunk that runs from one onto the other.
  A PDF's page is `page..page + 1`.
- `src/source/texts.rs` (new): the queue a backend's `Texts` keeps, lifted from `pdfcraft.rs`'s
  `PagesText` with what its four reviews put right: the answers held under the queue's lock and
  dropped from it when no longer listed, the lookups by hash set and a cursor, at most 4 MiB of
  answers waiting (`MAX_WAITING`). A backend lends it the function that reads a chunk's text or
  places a range, and what to do when the search ends (the PDF's kills its worker), as
  `threads.rs` serves `Pool`.
- `src/source/pdfcraft.rs`: `PagesText` on `texts.rs`, its worker and caps as they are.
- `src/source/wordcraft.rs`: `Paged::texts`.
  - The layout: `Document` keeps one `Arc<Cache>` that `pool()` and `texts()` share (`Cache` and
    `pool` at :693 and :718 today, each pool making its own). The search reads the pool's `Laid`;
    if `/` came before the first render, the search lays it out and the pool finds it done. A
    list's tiles (`renderer()`) keep their own and never search.
  - The chunks: `para_paths(StoryRef::Body)` in order (wordcraft's `doc/src/document.rs:270`, tables'
    cells walked). A chunk's text from `Paragraph.text`, each `OBJ` replaced by its object's
    `plain_text()` or nothing (`doc/src/para.rs:100`), with a map from the chunk's characters to
    `Paragraph.text`'s bytes, since a `Pos.off` is a byte offset there (`doc/src/position.rs:60`).
  - `chunks_on(page)`: from each page's body paragraphs (`Placed::Lines`, as `paragraphs_of` reads
    them at :612), kept at open for every kind; `paragraphs` is filled only for Markdown and HTML
    today.
  - A range's rectangles: `DocLayout::selection_rects(doc, a, b, page_hint)` (wordcraft's
    `layout/src/hit.rs:792`), `a` and `b` the `Pos` at the range's mapped bytes, `page_hint` the
    paragraph's first page; each `(page, Rect)` as `[left, top, right, bottom]` in points.
  - The work on one thread of its own, through `texts.rs`. No worker: what a document costs to lay
    out is bounded before it is (ADR-0012), and a paragraph's text and selection are small next to
    its layout.
- `src/view/search.rs`: `chunks_of` (:1022) from `chunks_on`; the unread counted in pages (:1006's
  `pages`/`parts` goes); the search handed to a new `Texts` across a relayout, its chunks' text and
  matches kept while the new one has as many chunks, its rectangles dropped.
- `src/view/pager.rs`: the search moved from the old level to the new on a relayout (the level is
  replaced whole today, `search: Option<Search>` at :476 with it).

**Patterns to follow:**

- `pdfcraft.rs`'s `PagesText` and its tests (`an_answer_whose_request_was_dropped_never_comes`,
  `the_queue_stays_quick_with_tens_of_thousands_of_pages`, `answers_wait_for_the_viewer_within_a_budget`):
  they move to `texts.rs` with it.
- `tests/it/view_pager.rs`'s PDF search test for the pager's side; `tests/it/wordcraft.rs` for
  documents written by the tests.

**Decided:**

- The rectangles from the pool's own layout, shared through one `Cache`: no second layout (some
  200 MB more for 500 pages, [usage](../usage.md#a-page-the-windows-size)), no worker.
- The text queue shared by both backends in `src/source/texts.rs`, not copied.
- The status line counts pages, for every backend.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- 🟡 **Paragraphs past `MAX_CHUNKS`** — 100,000 chunks (Spec-0006's cap, set for pages). A Markdown
  file at its 8 MiB of text in short list items may have more: the rest is left unread and the
  status line says so. Count a long real document's paragraphs, and say if the cap wants its own
  number for paragraphs.
- 🟡 **`page_hint`** — what `selection_rects` does with a hint that is wrong (a paragraph's first
  page, for a match on its second): check it finds the pieces whatever the hint, or give the page
  the match's chunk is asked for on.

## Tasks

- [ ] **The contract's pages, and the queue shared** — `chunks_on` for `chunk_of`, `search.rs`'s
  `chunks_of` and unread counting on it (pages, for every backend), `src/source/texts.rs` lifted
  from `pdfcraft.rs` with its tests, `PagesText` on it. Nothing a user sees changes for a PDF.
  *Files:* `src/source.rs`, `src/source/texts.rs`, `src/source/pdfcraft.rs`, `src/view/search.rs`,
  `docs/architecture.md`. *Verify:* `cargo test -j 2 --bin clv texts`, `cargo test -j 2 --test it
  pdfcraft`, and Spec-0006's PDF search tests unchanged; a fake `Texts` whose chunk runs over two
  pages, tinted with either page alone on screen (seen failing on `chunk_of`); a fake of paragraphs
  says `read N of M pages`, never `parts`.
- [ ] **A document's text and its matches' places** — `Paged::texts` on the shared `Cache`,
  `chunks_on` for every kind, the chunks' text and map, `selection_rects`. *Files:*
  `src/source/wordcraft.rs`, `tests/it/wordcraft.rs`, `tests/it/view_pager.rs`, `docs/usage.md`,
  `docs/architecture.md`. *Verify:* `cargo test -j 2 --test it wordcraft`: a word hyphenated at a
  line's end (a narrow `--width`) found whole and tinted on both lines; a table cell's word found;
  a field's result found; a paragraph across a page break, its tail tinted and scrolled to with
  only the second page on screen; a paragraph over 1 MiB left unread, the next found; a match on
  dark paper (`d`) tinted as ADR-0028's dark blend; the pool laid out once with a search going (a
  count of layouts through a `#[cfg(test)]` field). Manual: in kitty, `clv README.md`, `/search⏎`,
  `n`, `p`; then `d`: the tint reads on dark paper.
- [ ] **A search across a relayout** — the search handed to the new level and its `Texts`, the
  match you are on kept, its rectangles asked for again. *Files:* `src/view/pager.rs`,
  `src/view/search.rs`, `tests/it/view_pager.rs`, `docs/usage.md`. *Verify:* `cargo test -j 2
  --test it view_pager`: `/the⏎ 3n` then `]` on a Markdown file: still `3/N`, the tint on the new
  layout's pages, the view on the match; typing `/th`, then `]`, then `e⏎`: the count of `the`;
  a document whose chunks differ after a relayout (the file changed on disk) starts its search
  over. Manual: in kitty, a Markdown file, `/word⏎ n`, then `]` and `+`: the same match, tinted.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [ ] Tests added, each seen failing without the change: name them.
- [ ] Manual: in kitty, a Markdown file and a Word document with a table, `/word⏎`, `n` through
  the matches, `]` and a resize keeping the match, `d` on the Markdown; inside tmux the same.
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)); README's
  Plans bullet "Search Markdown, HTML and Word documents" comes out.

## Changelog

- 2026-11-13 — Draft, split from Spec-0006 after a review.
- 2026-11-13 — A paragraph a chunk, rectangles from `selection_rects`, after a second review.
- 2026-11-13 — Ready: the rectangles from the pool's layout, shared; `chunks_on` for a paragraph
  across a page break; the text queue shared in `src/source/texts.rs`; the status line in pages;
  the paragraphs' text settled; three tasks (spec-update).
- 2026-11-13 — Implementing ([notes](0007-search-markdown-html-and-word-documents-impl-20261113_1721.md)).
