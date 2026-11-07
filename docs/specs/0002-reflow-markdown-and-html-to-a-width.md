# Spec-0002: Reflow Markdown and HTML to a width in columns

- **Status:** Ready
- **Date:** 2026-11-07
- **Related:** [Spec-0001](0001-settings-a-file-declares.md), which must be `Done` first;
  [Spec-0003](0003-lay-markdown-and-html-out-to-the-window.md), which adds `window` on top;
  [ADR-0012](../adr/0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md) (what a layout
  may cost, and that it can't be stopped);
  [ADR-0019](../adr/0019-a-list-opens-its-files-when-their-tiles-are-wanted.md) (the threads files open
  on); [Documents](../usage.md#documents), [Limits](../usage.md#limits)

## Summary

Markdown and HTML are laid out on US Letter, some 83 characters to a line, however wide the
window: a wide window shows narrow pages with room to spare, a narrow one small text. This spec
lets the reader choose the length of a line. The documents' backend gets a `width` setting
([Spec-0001](0001-settings-a-file-declares.md)): `page` (as printed, the default), or a number of
characters, 20 to 200, measured in the code face. `[` and `]` step it by 4, with `page` among the
steps at its own width; `72]` sets it, and `--width page|N` starts there. The status line shows it
(`width 72`). Changing it lays the document out again, the **relayout** Spec-0001 left out. The
width is the source's own, given when it is laid out, so its sizes and its pixels always agree. A
new layout is made on the threads a list opens its files on while the old pages stay on screen;
changes made meanwhile come down to the latest, and the reader's paragraph stays on screen. On any
other file, `[` and `]` say `only Markdown and HTML reflow`. A page the window's size is
[Spec-0003](0003-lay-markdown-and-html-out-to-the-window.md).

## Requirements

**The setting**

- The documents' backend declares `width`: named value `page`, default `page`; numbers 20 to 200 in
  steps of 4, in characters; keys `Step('[', ']')`; option `--width`; message
  `only Markdown and HTML reflow`; effect relayout. Only Markdown and HTML have it.
- **A number N:** the page's text area is N characters of the code face (JetBrains Mono at 9.4 pt,
  5.64 pt each) wide, with the side margins as now. The page keeps Letter's height. A code block,
  indented 8 pt on each side (`wordcraft.rs`, its paragraph style), holds some 3 characters fewer.
- **`page`:** US Letter, exactly as now: a text area of 468 pt, some 83 characters. Nothing changes
  until the reader asks.
- **The order of the steps:** the numbers, and `page` at its own width, which the source says
  (Spec-0001's declaration grows a width for a named value, asked of the source, so `Nav` knows no
  document). From `page`, `]` goes to 84 and `[` to 80; from 80, `]` goes to `page`, and from 84,
  `[`. A number equal to a named value's width comes before it.
- `N]` and `N[` set N, clamped to the range. `--width` out of the range, or not `page` or a number,
  fails with what it takes (`--width: expected page or 20–200, got 300`).
- Pages narrower than Letter go one or two to a row, as `auto` sets any page: nothing changes in
  `fit.rs`.

**Relayout**

- A relayout value is the source's own: the documents' source is opened with a width, and lays out,
  renders and copies at that width, whatever a request carries. Opening and the first render each
  lay out, as now, at the same width.
- Changing it asks the source for a job (`Source::relayout(values)`): a `Send` closure, run on the
  opener threads, giving a new leaf source laid out at the new value, or why not.
- **Who holds the layout:** a source with a relayout setting is shown through one wrapper
  (`src/source/laid.rs`) shared by whatever shows it: the level it is read in, and its tile in a
  list, through `Later` and `Joined`. The wrapper holds the source in use, a generation, and at
  most one job under way and one waiting. When a job's source is taken in, on the main thread, the
  generation goes up, and everything showing the file sees the new pages: the list's tile, and the
  file gone into again.
- **Only the latest:** a layout once started runs to its end (ADR-0012); a change meanwhile waits,
  replacing any change already waiting; a result for a value no longer wanted is dropped. Relayout
  jobs queue behind the openings the screen wants.
- **Taking it in:** until the new layout is ready, the old pages stay on screen and the status line
  shows `…`. Then the old pages sent are deleted, the pool's results for the old generation are
  dropped, and the new pages are rendered and sent. A copy under way is dropped, saying
  `the page was laid out again`.
- **A failure** keeps the old layout and its value, and says why, through `printable`.
- **The place:** the paragraph at the top of the screen before the change is on the reader's page
  after it; where a page scrolls (as wide as the window, taller than it), it is at the top. The
  source says which paragraph is where (`Source::anchor(page, y)`,
  `Source::find(&Anchor) -> (page, y)`), from what its layout keeps for it: each page's paragraphs,
  by their order in the document, and their heights. A paragraph split across pages is found on the
  page it starts on; one not found (the end of the document) goes to the last page.
- **`--pages`:** it picks pages of the layout given at start, so while it is given, `[` and `]` (and
  a count before them) change nothing and say `--pages picks pages as laid out: no reflow`.
  `--width` with `--pages` lays out at that width first, and the pages are picked from it.
- In a list, the value holds for every file opened after; one already open is laid out again when
  next shown.
- Laying out again costs what opening did, within the same [Limits](../usage.md#limits). At 20
  characters a document has some four times the pages it has at `page`: the worst case is measured
  at 20, and the least raised if it goes past the limits.

**Showing it**

- The status line shows a relayout setting's value when it isn't the default (`width 72`), and drops
  it, before the zoom, in a narrow window.
- `c` copies the page as it is laid out.

## Out of scope

- **A page the window's size** (`window`, `0]`, a change of the window's size):
  [Spec-0003](0003-lay-markdown-and-html-out-to-the-window.md).
- **A font size** of its own. How large the text looks stays `+` and `-`'s.
- **Reflowing Word, RTF and OpenDocument text**: they have a page size of their own.
- **Markdown through a pipe**: it is told by its name, so a pipe of it doesn't open, now or here.
  HTML through a pipe reflows, from the bytes the source keeps.
- **Stopping a layout under way**, and a faster layout: each would be a change of its own.
- **Remembering the width** between runs, and relayout for other backends.

## Approach

**Files:**

- `src/source.rs` — `Effect::Relayout`; `Source::relayout(&self, values) -> Option<Job>`, a `Send`
  closure giving a leaf `Box<dyn Source + Send>`; `Source::anchor` and `Source::find`, an `Anchor`
  meaning nothing to the viewer; a named value's width, for the order of the steps.
- `src/source/laid.rs` (new) — the wrapper: the source in use, its generation, the job under way
  and the one waiting; its pool renders through the source in use and drops results of an older
  generation, keeping `Pool`'s contract (`set_queue` replaces, a result only while asked for,
  `try_recv` never blocks).
- `src/source/wordcraft.rs` — declares `width`; `Document` keeps its width, and `marked_up` takes its
  page from it rather than always Letter; `relayout` reads the bytes again from `Document.stored`;
  the layout keeps each page's paragraphs and heights for anchors (`Placed::Lines`).
- `src/source/later.rs` — relayout jobs on `Openers`' threads, behind openings; a file's opened
  source goes into the wrapper, so its tile and its level share it.
- `src/source/joined.rs` — a list's parts through their wrappers.
- `src/view/pager.rs` — a step or count on a relayout setting asks the wrapper, unless `--pages` is
  given; on a new generation, the sent pages go, and the reader goes to the anchor's page (and
  scroll). The status line shows the value.
- `src/view/mod.rs` — `--width` at start; `print_pages` lays out at it.
- Tests: in `wordcraft.rs`, a page's text width for N, and `page` as now; in `laid.rs`, with jobs the
  test holds and lets go of: one waiting replaced, a stale result dropped, a failure keeping the
  old, a generation seen by two holders; `tests/it/wordcraft.rs`, `--width` printed changes the
  pages' count and size; `tests/it/view_pager.rs`, `]` lays out again and a known paragraph stays
  on screen, `[` on a PDF, a list's tile after its file is laid out again, piped HTML.
- Docs: `docs/usage.md` (Keys, Documents, the options, Limits as measured at 20),
  `docs/architecture.md` (Sources, Lists and levels), `CHANGELOG.md`, and an ADR: a width in
  characters of the code face; relayout by the source, held in a shared wrapper; coalescing rather
  than stopping.

**Patterns to follow:**

- Opening on threads of their own, a reader not kept waiting: `Later` and `Openers`.
- A pool over sources that change: `Pools` in `src/source/joined.rs`.
- The page Markdown and HTML go on: `marked_up` in `src/source/wordcraft.rs`.

**Decided:**

- The width is the source's, given when it is laid out; a request's values don't lay out.
- The source lays itself out again, from the bytes it keeps, rather than the viewer opening the
  file again: it needs no `Reopen`, and piped HTML works.
- Only the latest change is laid out; a layout under way runs to its end.
- One wrapper per file, shared by its level and its tile, rather than `Later` and the pager's `Level`
  each learning to take in a new source.
- The place is kept as the reader's page, and the paragraph at the top only where a page scrolls:
  the pager doesn't learn to start a screen part-way down a page.
- `[` and `]` are refused while `--pages` is given, rather than the selection dropped.

## Open questions

- 🟡 **Cost at 20 characters** — measured by the first task; it may raise the least.

## Tasks

- [ ] **A width at open** — `--width page|N` for Markdown and HTML, the width the source's own,
  printed and in the pager, no keys yet; the worst case measured at 20. *Files:*
  `src/source/wordcraft.rs`, `src/view/mod.rs`, `tests/it/wordcraft.rs`, `docs/usage.md`.
  *Verify:* `cargo test -j 2 --test it wordcraft`; manual: in kitty,
  `clv --pager --width 40 README.md`: narrow tall pages, two to a row.
- [ ] **The wrapper and relayout** — `laid.rs`, jobs on the openers, generations, the steps and
  the order with `page`, the status line, `only Markdown and HTML reflow`, the place kept as the
  page the reader was on, for now. *Files:* `src/source.rs`, `src/source/laid.rs`,
  `src/source/later.rs`, `src/source/joined.rs`, `src/source/wordcraft.rs`, `src/view/pager.rs`,
  `tests/it/view_pager.rs`. *Verify:* `cargo test -j 2 --bin clv source::laid`,
  `cargo test -j 2 --test it view_pager`; manual: in kitty, a long `.md`, `]` three times quickly:
  `width 92`, and the pages laid out at most twice.
- [ ] **Keeping the place** — anchors, and `[` and `]` refused while `--pages` is given. *Files:* `src/source.rs`,
  `src/source/wordcraft.rs`, `src/view/pager.rs`, `tests/it/view_pager.rs`. *Verify:* a pager test
  with a known paragraph at the top before `[` and on screen after.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [ ] Tests added, each seen failing without the change: name them.
- [ ] Manual: in kitty, `[`, `]` and `72]` on a long Markdown file; on HTML through a pipe
  (`cat page.html | clv --pager /dev/stdin`); `]` on a PDF; a list mixing them, its tile after its
  file is laid out again.
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)), with the
  ADR.

## Changelog

- 2026-11-07 — Ready (spec-new).
- 2026-11-07 — Back to Draft after a review: the width the source's own, a shared wrapper for
  relayout, coalescing rather than stopping, the place and `--pages` defined, `window` moved to
  Spec-0003.
- 2026-11-07 — Ready: one shared wrapper, the place as the reader's page, `[` and `]` refused with
  `--pages`.
