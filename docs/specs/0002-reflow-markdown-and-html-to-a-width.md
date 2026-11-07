# Spec-0002: Reflow Markdown and HTML to a width in columns

- **Status:** Ready
- **Date:** 2026-11-07
- **Related:** [Spec-0001](0001-settings-a-file-declares.md), which it builds on;
  [ADR-0012](../adr/0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md) (what a layout
  may cost); [ADR-0019](../adr/0019-a-list-opens-its-files-when-their-tiles-are-wanted.md) (the
  threads files open on); [Documents](../usage.md#documents), [Limits](../usage.md#limits)

## Summary

Markdown and HTML are laid out on US Letter, some 83 characters to a line, however wide the
window: a wide window shows narrow pages with room to spare, a narrow one small text. This spec
lets the reader choose the length of a line. The documents' backend gets a `width` setting
([Spec-0001](0001-settings-a-file-declares.md)) with two named values, `page` (as printed, the
default) and `window` (a page the window's size, at the size of the terminal's text), and a
number of characters, 20 to 200, measured in the code face. `[` and `]` step it by 4, with `page`
and `window` among the steps at their own widths; `72]` sets it, `0]` goes to `window`, and
`--width page|window|N` starts there. The status line shows it (`width 72`). Changing it lays the
document out again, the **relayout** Spec-0001 left out: the source lays itself out again from the
bytes it keeps, on the threads a list opens its files on, while the pages stay on screen with `…`;
the paragraph at the top of the screen stays at the top. On any other file, `[` and `]` say
`only Markdown and HTML reflow`.

## Requirements

**The setting**

- The documents' backend declares `width`: named values `page` and `window`, numbers 20 to 200 in
  steps of 4, in characters; keys `Step('[', ']')`; option `--width`; effect relayout. Only
  Markdown and HTML have it.
- **A number N:** the page's text is N characters of the code face (JetBrains Mono at 9.4 pt)
  wide, with the margins as now. The page keeps Letter's height, so a narrow page is tall, and
  `auto` sets more of them side by side.
- **`page`:** US Letter, exactly as now; the default, so nothing changes until the reader asks.
- **`window`:** the page is the window's size, its text set so that one character of the code face
  is one terminal cell wide: one page fills one screen at `auto`. Without a window (printed into a
  pipe), the 80 × 24 assumed there (`FALLBACK_SIZE`).
- **The order of the steps:** the numbers, with `page` at its own width (some 83) and `window` at
  the window's, wherever those fall, past 200 or under 20 included. From `page`, `]` goes to 84
  and `[` to 80; from 80, `]` goes back to `page`. `]` at the top or `[` at the bottom stays, with
  a message.
- `N]` and `N[` set N, clamped to the range; `0]` (and `0[`) go to `window`.
- `--width page|window|N` starts there; a wrong value fails with what it expects.

**Relayout**

- Changing a relayout setting has the source lay itself out again with the new value, from the
  bytes it keeps, so a pipe reflows as a file does, on the threads a list opens its files on. The
  pages on screen stay, the status line's `…` with them, until the new layout is ready; then the
  pager takes the new source: its page count, its sizes, and new pages sent, the old ones deleted.
- A change before it is ready drops the layout under way: only the latest is laid out.
- Laying out again costs what opening did, within the same [Limits](../usage.md#limits). One that
  fails leaves the old layout, and says why.
- **The place is kept:** the paragraph at the top of the screen before the change is at the top
  after it. The backend says where on a page a paragraph is, and on which page and how far down a
  paragraph lands.
- In a list, the value holds for every file opened after; one already open is laid out again
  when next shown.
- With `window`, a change of the window's size lays out again once the size has held for half a
  second; until then the old pages are fitted to the new window, as now.

**Showing it**

- The status line shows a relayout setting's value (`width 72`, `width window`) when it isn't the
  default, and drops it, before the zoom, in a narrow window.
- On a file without `width`, `[` and `]` say `only Markdown and HTML reflow`.
- `c` copies the page as it is laid out.

## Out of scope

- **A font size** of its own. How large the text looks stays `+` and `-`'s, and the width is the
  one thing that reflows: two keys both changing how many characters go to a line would confuse.
- **Reflowing Word, RTF and OpenDocument text**: they have a page size of their own, and stay as
  printed.
- **An epub as one book**: its chapters reflow as HTML does, but nothing here joins them.
- **Remembering the width** between runs.
- **Relayout for other backends**: the effect is general, `width` its only user.
- **A faster layout**: a long document lays out again as slowly as it opened, up to the 20
  seconds in [Limits](../usage.md#limits). Laying out the pages near the reader first would be a
  change of its own.

## Approach

**Files:**

- `src/source.rs` — `Effect::Relayout`; `Source::relayout(&self, values, window) -> Option<Job>`,
  a `Send` closure making a new `Box<dyn Source + Send>`, run off the main thread (`None`: no
  relayout setting); `Source::anchor(page, y) -> Option<Anchor>` and
  `Source::find(&Anchor) -> Option<(page, y)>`, an `Anchor` meaning nothing to the viewer.
- `src/source/wordcraft.rs` — declares `width`. `marked_up` takes its page from the width rather
  than always Letter: N characters of the code face and the margins, or the window's size at a
  character to a cell. `relayout` reads the bytes again from `Document.stored`. An anchor is the
  paragraph (`Placed::Lines { story, path }`) at a height of a page.
- `src/source/later.rs` — relayout jobs on `Openers`' threads, beside openings; a newer job for a
  source drops an older one.
- `src/source/joined.rs` — a list lays out again those of its files that have the setting, each
  as it is shown.
- `src/view/pager.rs` — a step or count on a relayout setting starts a job; `Level` takes the new
  `source` and `pool` when it is done, at the anchor's page. The status line shows the value; a
  change of size waits some 500 ms for `window`.
- `src/view/mod.rs` — `--width` at start; printing with `window` at the terminal's size, or
  `FALLBACK_SIZE`.
- Tests: in `wordcraft.rs`, that N characters fit a line of a code block, and `window`'s page;
  `tests/it/wordcraft.rs`, that `--width` printed through the viewer changes the pages' count and
  size; `tests/it/view_pager.rs`, that `]` lays out again and keeps the place, `[` on a PDF, and a
  change of size with `window`.
- Docs: `docs/usage.md` (Keys, Documents, the options), `docs/architecture.md` (Sources, Zoom),
  `CHANGELOG.md`, and an ADR: a width in characters of the code face rather than terminal cells
  or points; relayout by the source rather than by opening the file again; `page` and `window`
  among the steps at their own widths.

**Patterns to follow:**

- Opening on threads of their own, and a reader not kept waiting: `Later` and `Openers`
  (`src/source/later.rs`, [ADR-0019](../adr/0019-a-list-opens-its-files-when-their-tiles-are-wanted.md)).
- The page Markdown and HTML go on: `marked_up` in `src/source/wordcraft.rs`.

**Decided:**

- The source lays itself out again, from the bytes it keeps, rather than the viewer opening the
  file again: a pipe can't be read twice.
- Only the latest change is laid out; one that fails keeps the old layout, with a message.
- A width is in characters of the code face, not terminal cells: what the layout is, whatever the
  zoom.

## Open questions

- 🟡 **`window` on a terminal that doesn't say its cell size in pixels** — it is assumed
  (`fit::FALLBACK_CELL`), so a character may not be quite a cell there.

## Tasks

- [ ] **A width at open** — `--width page|N` for Markdown and HTML, printed and in the pager, no
  keys yet. *Files:* `src/source/wordcraft.rs`, `src/view/mod.rs`, `tests/it/wordcraft.rs`.
  *Verify:* `cargo test -j 2 --test it wordcraft`; manual: in kitty, `clv --width 40 README.md`,
  narrow tall pages, several side by side.
- [ ] **Laying out again with keys** — `[`, `]`, `N]`, `N[`; the job on the openers' threads, the
  new source taken, older jobs dropped; the status line; `only Markdown and HTML reflow`. The
  place kept by how far through the document it is, for now. *Files:* `src/source.rs`,
  `src/source/later.rs`, `src/source/joined.rs`, `src/source/wordcraft.rs`, `src/view/pager.rs`,
  `tests/it/view_pager.rs`. *Verify:* `cargo test -j 2 --test it view_pager`; manual: in kitty,
  a long `.md`, `]` three times quickly: one layout, `width 92`.
- [ ] **Keeping the place** — anchors in place of how far through. *Files:* `src/source.rs`,
  `src/source/wordcraft.rs`, `src/view/pager.rs`, `tests/it/view_pager.rs`. *Verify:* a pager
  test: the paragraph at the top stays at the top after `[`.
- [ ] **`window`** — the window's page, `0]`, `page` and `window` among the steps, a change of size
  laid out once it settles, 80 × 24 into a pipe. *Files:* `src/source/wordcraft.rs`,
  `src/view/pager.rs`, `src/view/mod.rs`, `tests/it/view_pager.rs`, `tests/it/view_tmux.rs`.
  *Verify:* `cargo test -j 2 --test it view_`; manual: in kitty, `0]`, then resize the window: one
  layout once it settles; again in a private tmux.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [ ] Tests added, each seen failing without the change: name them.
- [ ] Manual: in kitty, `[`, `]`, `72]` and `0]` on a long Markdown file and a piped one; `]` on a
  PDF; a list mixing them.
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)), with the
  ADR.

## Changelog

- 2026-11-07 — Ready (spec-new).
