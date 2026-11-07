# Spec-0003: Lay Markdown and HTML out to the window's size

- **Status:** Ready
- **Date:** 2026-11-07
- **Related:** [Spec-0002](0002-reflow-markdown-and-html-to-a-width.md), which must be `Done` first;
  [Spec-0001](0001-settings-a-file-declares.md);
  [The size of a cell](../architecture.md#the-size-of-a-cell);
  [Limits](../usage.md#limits)

## Summary

With [Spec-0002](0002-reflow-markdown-and-html-to-a-width.md), a reader chooses how many characters
go to a line of Markdown or HTML, on pages as tall as Letter. This spec adds a value of the `width`
setting, `window`: the page is the size of the window's pages area, its text set so that a
character of the code face is a cell wide, so one page fills one screen, as on an e-reader. `]`
past the window's own width goes to it, `0]` goes to it at once, and `--width window` starts there.
When the window changes size, the document is laid out again once the size has held for half a
second; until then the old pages are fitted to the new window, as now.

## Requirements

- **The value:** `width` gets a named value `window`, and `0]` (or `0[`) goes to it: the setting
  declares the named value a count of 0 sets.
- **The page:** the area pages are shown in, the window less the status line's row, in the
  terminal's cells and their size in pixels. Its text is as many code-face characters as the area
  has columns, less the margins; its height, the area's rows. A character is a cell wide at the
  zoom that fits one page to the window.
- **Bounds:** the margins shrink with a small window, to a cell at least; a window under 20 columns
  is laid out at 20, and one under some rows (settled by measuring) at that, rather than refused. A
  window's size is input like any other: checked arithmetic, and the page's size in points capped.
- **The order of the steps:** `window` among the numbers at the window's own width, wherever that
  falls, past 200 or under 20 included; equal to a number or to `page`, it comes after them.
- **A change of size:** with `window`, the document is laid out again once the window's size has
  held for 500 ms; the old pages are fitted to the new window until the new layout is in, through
  Spec-0002's relayout. A change of the cell size alone (a font change) does the same.
- **The first layout:** with `--width window`, files are opened after the terminal is asked for its
  size and its cell's, so the first layout is the window's, never `page` and then the window's. The
  wait is the terminal's answer, bounded by the timeout the question has now; without
  `--width window`, nothing waits that doesn't now.
- **A cell of unknown size** (a terminal that doesn't say it in pixels): laid out at the cell assumed
  now (`fit::FALLBACK_CELL`). The page still has the area's shape and fills it; only how many
  characters go to a line may be off.
- **Printing:** `--width window` without the pager lays out at the size printing uses now: the
  terminal's, or 80 × 24 (`FALLBACK_SIZE`) where there is none.
- **Inside tmux:** the pane's size, as the pager uses now.
- The status line shows `width window`.

## Out of scope

- **A page as tall as the window with a chosen width** (a number of characters on a window-tall
  page): `window` sets both.
- **Laying out while the window is still changing size.**
- Whatever Spec-0002 leaves out.

## Approach

**Files:**

- `src/source.rs` — what a relayout is given grows the window's area (`cols`, `rows`, the cell's
  pixels); the named value a count of 0 sets.
- `src/source/wordcraft.rs` — the page for `window`, its margins and bounds; `window`'s width in the
  order of the steps.
- `src/view/pager.rs` — the area given to a relayout; a change of size, held 500 ms, asks for one.
- `src/view/mod.rs` — with `--width window`, files opened after the terminal is asked; printing at
  its size, or `FALLBACK_SIZE`.
- Tests: in `wordcraft.rs`, the page for a window, a tiny one, a huge one; `tests/it/view_pager.rs`,
  `0]`, and a change of size laying out once; `tests/it/view_tmux.rs`, a pane resized.
- Docs: `docs/usage.md`, `docs/architecture.md`, `CHANGELOG.md`.

## Open questions

- 🟡 **A cell of unknown size** — laid out at the assumed cell; if that proves poor on a real
  terminal, falling back to a number is the other way.
- 🟡 **The least rows** a window is laid out at, and the margins of a small one: measured.

## Tasks

- [ ] **The window's page** — `window`, its page, margins and bounds; `--width window` at start,
  files opened after the terminal is asked; printing at its size or 80 × 24; the least rows
  measured. *Files:* `src/source.rs`, `src/source/wordcraft.rs`, `src/view/mod.rs`,
  `tests/it/wordcraft.rs`. *Verify:* `cargo test -j 2 --test it wordcraft`; manual: in kitty,
  `clv --pager --width window README.md`: one page fills the screen.
- [ ] **Going to it** — `window` among the steps at its width, `0]`, the status line. *Files:*
  `src/source/wordcraft.rs`, `src/view/pager.rs`, `tests/it/view_pager.rs`. *Verify:*
  `cargo test -j 2 --test it view_pager`; manual: in kitty, `]` past the window's width, then `0]`
  from 40.
- [ ] **A change of size** — laid out again once it has held 500 ms, a font change too; inside tmux,
  the pane's. *Files:* `src/view/pager.rs`, `tests/it/view_pager.rs`, `tests/it/view_tmux.rs`.
  *Verify:* `cargo test -j 2 --test it view_`; manual: in kitty, resize the window: one layout once
  it settles; again in a private tmux, the pane resized.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [ ] Tests added, each seen failing without the change: name them.
- [ ] Manual: in kitty, `0]` on a long Markdown file, then the window resized: one layout once it
  settles; again in a private tmux, the pane resized.
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- 2026-11-07 — Draft, split from Spec-0002 after a review.
- 2026-11-07 — Ready: files opened after the terminal is asked, an unknown cell taken as assumed.
