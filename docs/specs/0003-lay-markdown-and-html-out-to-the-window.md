# Spec-0003: Markdown and HTML on pages the window's size, and a font size

- **Status:** Implementing
- **Date:** 2026-11-07
- **Related:** [Spec-0002](0002-reflow-markdown-and-html-to-a-width.md), `Done`, whose relayout this
  builds on and whose US Letter pages it replaces
  ([ADR-0022](../adr/0022-markdown-and-html-reflow-to-a-width-in-characters-as-a-new-level.md));
  [Spec-0001](0001-settings-a-file-declares.md)
  ([ADR-0021](../adr/0021-a-backend-declares-its-settings-and-their-options-hold-for-any-file.md));
  [The size of a cell](../architecture.md#the-size-of-a-cell); [Limits](../usage.md#limits)

## Summary

Markdown and HTML have no page size of their own, yet they are laid out on US Letter, and since
[Spec-0002](0002-reflow-markdown-and-html-to-a-width.md) on pages as tall as Letter at any width. This
spec makes the window their page, as on an e-reader: a page is always as tall as the window's pages
area. US Letter goes, and `page` with it.

Two things then set the layout, each laying the document out again:

- **The font size**, the new `font` setting: `+` and `-` on a document make its text larger or
  smaller, in 10% steps from 50% to 400%; at 100%, the default, a character of the code face is a
  cell wide. Larger text puts fewer characters on a line and fewer lines on a page. Scaling the
  picture, what `+` and `-` do elsewhere, has no use on a page the screen's size. `0+` goes back to
  100%.
- **The length of a line**, `width`, as Spec-0002 built it. `window`, the default, fills the
  window: one page as wide as it, or, where the window holds two pages of at least 60 characters,
  two side by side, as an open book. A number N makes the page N characters wide at the same font
  size, its height still the window's; narrow pages go two to a row. `0]` goes back to `window`.

When the window changes size, the document is laid out again once the size has held for half a
second. Word, RTF and OpenDocument keep their own pages, and `+` and `-` keep scaling them. A list's
documents are previewed on portrait pages of a fixed size.

## Requirements

**The room a document is laid out for**

- **`Room`** (`src/source.rs`, new): what a page is laid out for, all of it the viewer's to say and
  none of it a document's: the pages area's columns and rows (the window less the status line's
  row), a cell's width and height in pixels, as `f32`, exactly the cell the grid is fitted with
  (for text cells a sampling cell, 1.25 × 2 for a 10 × 16 terminal cell: rounding it would change
  its shape), and the cells between two pages side by side (`fit::GAP`). It is not `fit::Area`,
  and `source.rs` takes nothing from `src/view/`.
- **The same room** (`Room::same`): the same columns, rows and gap, and a cell within 2% of the
  other's, width and height each. So a window dragged by a few pixels, which changes a cell's size
  by a fraction, is the same room and lays nothing out; a font change in the terminal, which
  changes the cell by more and its columns and rows with it, isn't. Rooms are only ever compared
  through it.
- **The room the pager gives** is the one `draw` lays its grid out in (the same columns, rows and
  cell, `output.fitting` included), made by one function both use, so that what is laid out and
  what is shown agree.
- **Opening:** `Backend::open_at(input, options, room)`, a default method calling `open`, which only
  the documents' backend overrides; `Opener` and the openers' `Slot` call it. No other backend
  changes. `Source::relayout(values, room)` takes it too.
- **`laid_at`** gives back the values and the room the layout was asked for, exactly as given,
  never what the backend made of them (its margins shrunk, its least size): the pager compares
  them with what it wants (the room through `Room::same`), and a source that answered otherwise
  would be laid out again on every pass.

**The page**

- **Its height:** the room's, always. **Its width:** at a number N, N characters of the code face
  with the margins, never wider than the room; at `window`, the room's width, or half of it less
  the gap where each half holds at least 60 characters (`BOOK_LEAST` in `wordcraft.rs`, a constant
  until a later spec's configuration file). All in points such that, at font size F%, a character
  of the code face (5.64 pt) is F/100 of a cell wide on screen when the page is shown as tall as
  the room.
- **The margins:** two characters of the code face at each side, one row at the top and bottom, at
  the font size; in a small room they shrink, to a cell at least.
- **Bounds:** a room is input like any other: checked arithmetic, and the page's size in points
  capped. A page holds at least 20 characters a line and 10 lines of text. In a small room the
  font sizes past that can't be reached, and `+` says `font 250%, the most here`; a room smaller
  than that at 50% is laid out at that size, shown smaller, rather than refused.
- **Shown at the font size, through one generic method:** `Source::largest_scale(page) ->
  Option<f32>`, a default method answering `None`: the largest scale (pixels per unit) the page is
  shown at. Only the documents' backend answers, with the scale at which a page is as tall as the
  room. `fit::layout` takes it as it takes the 1.0 a natural page is held to: at `Fit::Page`, never
  past it; at `Fit::Width` and at N to a row, as any page. Nothing else in `fit.rs` changes:
  `natural` stays, and so does the 80% (`TWO_UP_SHARE`) by which two pages go to a row. A
  document's pages aren't natural, so they are oversampled for text cells as a PDF's are
  (`Output::rendering`).
- **The cell's size:** the tty's, where it reports pixels; elsewhere, the one asked at start (or
  `fit::FALLBACK_CELL` without an answer), which holds for the run. The page still has the room's
  shape and fills it; only the characters to a line may be off.

**`width`**

- `width` loses `page` and gets `window`: named value `window`, the default; `Numbers` 20 to 200,
  step 4, as now; `--width window|N`.
- **A count of 0** sets what the setting declares for it (`Setting.zero`): `window` for `width`, 100
  for `font`. `0]`, `0[`, `0+` and `0-` go there.
- **Where a document's values sit, from the document:** `Source::bounds(setting, room, values) ->
  Option<Bounds>`, `Bounds { named_at: Option<u32>, most: Option<u32> }`: where the setting's named
  value sits among its numbers, and the most a number is laid out at, both for this room and these
  values. The level asks it when it is made, and again when the room it is shown in changes, and
  keeps the answer on its `Nav`; `turned` takes it as an argument, as `Nav` holds no source.
  `Named.at` is no longer static, and goes.
- **The order of the steps:** the multiples of 4 from 20, and `window` at its `named_at` (its line
  length: the room's width, or a book page's), after a number equal to it, up to `most` (the
  longest line the room holds). From the last step, `]` says `width …, the most`. A number past
  `most` (typed, `--width`, or left so by a smaller window or a larger font) is kept and laid out
  at `most`, and its page grows back when the room does; `[` from it goes to the last step under
  `most`, and the status line says `width 120 (window)`.
- The status line shows `width 72` on a document, and nothing at `window`.

**`font`**

- The documents' backend declares `font` as a `Setting`: no named values; `Numbers` 50 to 400,
  step 10, unit `%`; default 100; zero 100; option `--font N`; message
  `only Markdown and HTML have a font size`; effect `Relayout`; keys `Keys::Zoom`. Its `most` in a
  small room comes from `bounds`.
- **`Numbers.unit`** (new, `""` for `width`): said after a number wherever one is, in `said`,
  `turned`'s ends, the status line's (`laid_width` and its like) and the option's message
  (`--font: expected 50–400%, got 500`).
- **`Keys::Zoom`** (new, under the same `cfg_attr(expect(dead_code))` as `Step`): `+` and `=` step
  the setting up, `-` and `_` down; `N+` and `N-` set N, clamped. They go to it only on a document
  that has the setting, not in a list and not while `--pages` is given: `Nav`'s `+` and `-` arms
  (in `key` and in `list_key`'s caller) ask first. Everywhere else they are the viewer's zoom, as
  now. `a_settings_keys_are_its_own_in_every_backend` lets `Keys::Zoom` through, it alone among the
  viewer's keys. This changes ADR-0021's order (the viewer's keys first) for these keys alone, and
  is recorded in the new ADR.
- **`?` and the hints:** `help_lines` takes what the level has, and shows one `+ -` line: the font
  size on a document that has it, the zoom elsewhere. `KEYS` and `OPENED_KEYS` say `+ - font` on
  such a document.
- The status line shows `font 120%` on a document when it isn't 100%.

**When it is laid out**

- **The first layout:** files open where and as they do now, before the terminal is asked
  anything and before raw mode, so Ctrl-C stops a long layout and leaves the terminal as it was.
  A Markdown or HTML file is laid out for the room the tty says without a question: its columns
  and rows, and its cell from the pixels it reports (kitty, Ghostty, WezTerm and iTerm2 do), or
  tmux's cell inside tmux, as now; otherwise `fit::FALLBACK_CELL`. Where the cell asked later, as
  now, isn't the same room, the document on screen is laid out once more, through a change of size
  (task 2): the old pages meanwhile, `laying out…`. Only a terminal that reports no pixels pays
  that second layout.
- **Printing** (`--protocol` without the pager) lays out at the room the tty says, as above, with
  no second layout; with no terminal, at the room a list's previews take.
- **A list's documents** are opened for a fixed portrait room (`PREVIEW`, 60 × 40 cells at the cell
  the run started with), as tiles are square: a landscape page the window's shape would be a strip
  of small text in one. They keep it: they are previews, as Spec-0002 decided. A document gone into
  is laid out for the pager's room and the values wanted then.
- **A change of size:** the document on screen is laid out again once the room (its columns and
  rows, or the cell where the tty reports pixels) has held for 500 ms; until then its old pages are
  fitted to the new window, as now. It goes through Spec-0002's relayout: the old level until the
  new one is ready, only the latest, the place kept. A layout is asked for, and its result taken
  in, by its values and its room together.
- **A failure** for a room: the values go back as Spec-0002 says, and the pair (values, room) that
  failed is remembered on the level; it isn't asked for again until the room or the values change,
  or `r`. Its why is said once.
- **While it runs**, the status line says what is being laid out that isn't the default
  (`width 72…`), or `laying out…` when nothing is (a change of size at the defaults):
  `relayout::describe` drops the defaults.
- **Inside tmux:** the pane's size, as the pager uses now. A font change in the outer terminal
  isn't seen (the cell is tmux's at start); `docs/usage.md`'s tmux section says so.
- **`--pages`** picks pages as laid out at start: a change of size doesn't lay them out again, and
  `[`, `]` say what they say now; `+` and `-` are the viewer's zoom.
- **Turning pages:** a page as tall as the room doesn't scroll, so `j`, `k` and Enter turn pages as
  `n` and `p` do. `docs/usage.md`'s Keys say so.
- **Cost:** a change of size now lays out every Markdown or HTML document on screen. A small page
  means many: 8 MiB of text at 20 characters and 10 lines is some 40,000 pages. Measured with
  ADR-0012's files on a release build, under `/usr/bin/time -l`:
  - **Printed**, at the extremes: task 1 at 80 × 24, width 20, 100%; task 3 at 80 × 24 at the
    font's most for it, and a large window at 50%. Within what [Limits](../usage.md#limits) says a
    Markdown or HTML file costs at most, or the least lines a page holds is raised until they are.
  - **In the pager**, where a relayout holds the old layout beside the new (Spec-0002's 9.5 GB):
    task 2, the costliest file at 80 × 24, width 20, the window resized once it shows; task 3, the
    same at the font's most. The peak goes into Limits.

## Known limitation

**The font size is a most, not a promise.** A page is never shown larger than its font size, but
the existing fit may show it smaller: `auto` puts two pages to a row where each keeps 80% of its
size alone (`TWO_UP_SHARE`). So a width between about half and five eighths of the window (50 to
62 characters in a window of 100 columns, at 100%) shows two pages at 80% to 100% of the font
size asked, rather than one at 100% with the rest of the window empty, while the status line says
`font 100%`. At `window` it never happens: a book page is laid out to fit two at 100%.

This was chosen over a third kind of page size (an `Exact` scale: shown one to one, never shrunk
to share a row), which would have made `natural` an enum in every source, and the fit and the
oversampling branch on it. Should the shrinking prove a nuisance, the fix is one condition: two to
a row only at `largest_scale`, for a page that has one.

## Out of scope

- **US Letter pages for Markdown and HTML**, and a page as printed: gone.
- **A page height other than the window's**, and a page wider than the window that scrolls.
- **A font size for Word, RTF and OpenDocument**: their pages are their own; `+` and `-` stay the
  zoom on them.
- **Laying out while the window is still changing size**, and drawing the old layout only once
  across a change of size (it is drawn at the new size, and again once laid out).
- **A configuration file** for `BOOK_LEAST`, `PREVIEW` and the like: a later spec.
- **Remembering the font size or the width** between runs.
- Whatever Spec-0002 leaves out.

## Approach

**Files:**

- `src/source.rs` — `Room`; `Backend::open_at`; `Source::relayout` and `laid_at` with the room;
  `Source::largest_scale`; `Source::bounds` and `Bounds`; `Named.at` gone; `Setting.zero`;
  `Numbers.unit`; `Keys::Zoom`, and the keys test letting it through.
- `src/source/wordcraft.rs` — the page from the room, the font size and the width, the book, its
  margins and bounds; `window` replacing `page`; `font` declared; `bounds`; `largest_scale`; the
  room in `laid_at`.
- `src/source/later.rs` — `Slot` opening with `open_at`, at the preview room.
- `src/view/fit.rs` — `layout` held to a page's largest scale at `Fit::Page`.
- `src/view/mod.rs` — the first room from the tty, without a question; the preview room for a
  list's documents and for printing with no terminal; printing at the tty's room;
  `Output::rendering` as now, which a document's pages now go through; the option's message with
  the unit.
- `src/view/pager.rs` — the room `draw` uses, from one function, compared through `Room::same`; `bounds` kept on `Nav` and given to
  `turned`; `Numbers.unit` said; `+` and `-` to a `Zoom` setting on a document; the room held
  500 ms and compared with `laid_at`; a failed pair not asked again; `help_lines`, the hints and
  the status line.
- `src/view/relayout.rs` — the pair (values, room); a failed pair remembered; `describe` dropping
  defaults.
- `src/source/picture.rs` — the scale a picture renders at reads a page's size; checked, changed
  only if the documents' page in a room needs it.
- Tests: in `source.rs`, `Room::same` (a cell 1% off the same, 3% off not, a sampling cell kept
  exact); in `wordcraft.rs`, the page for a room at 100% and at 200%, a number, a book page and the
  room that just doesn't hold two, a tiny room and a huge one, the font's most in a small room,
  `laid_at` as asked; in `fit.rs`, a page held to its largest scale alone and two to a row, and
  `--zoom 2` and the width as any page; in `relayout.rs`, a failed (values, room) not asked for
  again until either changes, and `describe`; in `pager.rs`, the steps' order with `bounds`, a
  number past `most` kept and its status, `0]` and `0+`, `+` to the font on a document and to the
  zoom in a list, on a PDF and with `--pages`, `?`'s one `+ -` line; `tests/it/wordcraft.rs`,
  printing with no terminal on the preview's page; `tests/it/view.rs`, a list's document previewed
  portrait; `tests/it/view_pager.rs`, a page as tall as the window, `+` laying out again, a change
  of size laying out once; `tests/it/view_tmux.rs`, a pane resized.
- Docs: `docs/usage.md` (Keys, Documents, the options, tmux, Limits), `docs/architecture.md`
  (`Room` and `Room::same`, `largest_scale`, `bounds`, the first room from the tty), `CHANGELOG.md` (Spec-0002's line
  rewritten: `page` never shipped), and an ADR superseding ADR-0022's page: the window as the page
  of a document without one; the font size on `+` and `-`; one largest scale rather than an
  `Exact` page size, and the limitation that brings.

**Patterns to follow:**

- A relayout, its job and the new level: `src/view/relayout.rs` and `Pager::replace` (Spec-0002).
- A cap fitting holds a page to: `natural`'s 1.0 in `fit::layout`.
- The window's size and cell, inside tmux too: `Pager::draw` and `Cell::from_window`.
- The terminal asked before the pager starts, and why raw mode spans it: `src/view/mod.rs`.

**Decided:**

- The window is the page of a document that has none; US Letter and `page` go.
- A page is always as tall as the room; `width` sets only its width; `window` is a book where the
  window holds two pages of 60 characters.
- `+` and `-` are the font size on a Markdown or HTML document, the viewer's zoom elsewhere.
- The font size holds through one generic default method, `largest_scale`, which `fit.rs` applies
  as it does a natural page's 1.0: no `Exact` scale, no source but the documents' touched (see
  *Known limitation*).
- What the backend knows of the window comes as a `Room`, said back exactly as given, and compared
  with a 2% give in the cell rather than rounded, so text cells keep their shape.
- Files open as now, before any question and before raw mode, at the room the tty says; a terminal
  that reports no pixels gets a second layout once its cell is known. Asking first was rejected:
  opening in raw mode, or in a mode between, leaves the terminal altered when Ctrl-C ends the run.
- A list's documents are previewed on a fixed portrait page.
- A change of size lays the document on screen out again, once it has held 500 ms.

## Open questions

- 🟡 **The least lines a page holds** (10 for now), and a small room's margins: measured.
- 🟡 **A cell of unknown size** — laid out at the assumed cell; if that proves poor on a real
  terminal, a page of the room's shape at 80 characters is the other way.
- 🟡 **Printing to a pipe or a file** lays out at the room the tty says, as decided: so the page's
  shape is the terminal the command runs in, and a test that prints a document from a terminal gets
  that terminal's room (the tests compute it from `/dev/tty` as `clv` does). The other way is the
  preview's room whenever stdout isn't a terminal, which would make printed pages the same anywhere.

## Tasks

Each task's commit passes the quality gates alone.

- [x] **Pages the window's shape** — `Room`, `open_at`, `laid_at` and `relayout` with it;
  `largest_scale` and `fit::layout` held to it; the page from the room at 100%, the book; `window`
  replacing `page`, `Setting.zero` and `0]`; `bounds` and the steps' order; `Room::same`; the
  first room from the tty; the preview room for a list and for printing with no terminal; printing
  at the tty's room; the cost printed at 100%. A change of size is fitted, as now. *Files:*
  `src/source.rs`, `src/source/wordcraft.rs`, `src/source/later.rs`, `src/view/fit.rs`,
  `src/view/mod.rs`, `src/view/pager.rs`, `tests/it/wordcraft.rs`,
  `tests/it/view.rs`, `tests/it/view_pager.rs`. *Verify:* `cargo test -j 2 --bin clv view::fit`,
  `cargo test -j 2 --bin clv view::pager`, `cargo test -j 2 --test it wordcraft`,
  `cargo test -j 2 --test it view`; manual: in kitty, `target/debug/clv --pager README.md`: one
  page fills the screen, or two in a wide window; `40[`: narrow pages, as tall; `0]` back; Ctrl-C
  while a large file opens leaves the terminal as it was.
- [x] **A change of size** — the room held 500 ms, then laid out again, a cell's size alone where
  the tty reports pixels, and the cell asked after the first layout; a failed (values, room) not
  asked for again; `describe` without defaults; inside tmux, the pane's; the pager's peak at
  80 × 24, width 20. *Files:* `src/view/pager.rs`, `src/view/relayout.rs`,
  `tests/it/view_pager.rs`, `tests/it/view_tmux.rs`. *Verify:*
  `cargo test -j 2 --bin clv view::relayout`, `cargo test -j 2 --test it view_`; manual: in kitty,
  resize the window: one layout once it settles, `laying out…` meanwhile.
- [x] **The font size** — `font`, `Numbers.unit`, `Keys::Zoom` and the keys test, `0+`, `--font`,
  its most in a small room, `?`, the hints and the status line; the cost at the font's extremes,
  printed and in the pager. *Files:* `src/source.rs`,
  `src/source/wordcraft.rs`, `src/view/pager.rs`, `src/view/mod.rs`, `tests/it/view_pager.rs`.
  *Verify:* `cargo test -j 2 --bin clv view::pager`, `cargo test -j 2 --test it view_pager`;
  manual: in kitty, `+` and `-` on a Markdown file, `0+`, `+` on a PDF and in a list.

## Done

- [x] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass, after each
  task, each backend alone too (`cfg` attributes change).
- [x] Tests added, each seen failing without the change: named, with what was reverted for each, in the
  [notes](0003-lay-markdown-and-html-out-to-the-window-impl-20261109_0025.md), under each task.
- [ ] Manual, with `target/debug/clv` built last by a plain `cargo build`, always run by that path:
  in kitty, a long Markdown file: one page a screen, two in a wide window, `+` and `-`, `[` and `]`
  keeping the height, `0]` and `0+`, the window resized; HTML through a pipe; `+` on a PDF; a
  list, its documents' portrait tiles and `+`; `--protocol ascii` on a Markdown file, legible; again
  in a private tmux, the pane resized.
- [x] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)), with the
  ADR ([ADR-0023](../adr/0023-the-window-is-the-page-of-markdown-and-html.md); README's Plans had no
  bullet for this).

## Changelog

- 2026-11-07 — Draft, split from Spec-0002 after a review.
- 2026-11-07 — Ready: files opened after the terminal is asked, an unknown cell taken as assumed.
- 2026-11-08 — Rewritten, back to Draft: the window is every Markdown and HTML page, as tall as
  the area always; US Letter and `page` go; `+` and `-` are the font size on a document.
- 2026-11-08 — After a review: pages laid out to the area shown at its height (`fit.rs`); `window`
  the most of the steps, a number past it kept; a failed (values, area) not retried; a cell's
  change only where the tty reports pixels; a page at least 20 × 10, the cost measured at the
  extremes.
- 2026-11-08 — `fit.rs` kept generic: `natural` becomes a `Scale` (`Free`, `AtMost`, `Exact`), the
  documents' pages `Exact`, in pixels of the screen; `--zoom` applies to them as to any page.
- 2026-11-09 — After a second review: no `Exact`, one default `largest_scale` instead, the
  shrinking it allows written down as a known limitation; `Room` in whole pixels, said back as
  given; `open_at` as a default method; `bounds` from the source to `turned`; `window` a book where
  the window holds two of 60; `Numbers.unit`; `Keys::Zoom` spelt out; `0+`; files opened with
  Ctrl-C working; portrait previews; `describe` without defaults.
- 2026-11-09 — Ready.
- 2026-11-09 — After a third review: files open as now, before any question, at the room the tty
  says, and a terminal without pixels lays out again once its cell is known (no `Tty::opening`,
  which Ctrl-C would have left behind); `Room`'s cell exact, compared with a 2% give; the pager's
  peak measured, and the font's extremes in task 3.
- 2026-11-09 — Implementing ([notes](0003-lay-markdown-and-html-out-to-the-window-impl-20261109_0025.md)); an open
  question added: where printing to a pipe lays out.
