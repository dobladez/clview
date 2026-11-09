# Spec-0005: Pages to a row on their own keys, and a document's margins

- **Status:** Implementing
- **Date:** 2026-11-09
- **Related:** [Spec-0003](0003-lay-markdown-and-html-out-to-the-window.md), `Done`, whose room,
  font size and relayout this builds on
  ([ADR-0023](../adr/0023-the-window-is-the-page-of-markdown-and-html.md));
  [Spec-0002](0002-reflow-markdown-and-html-to-a-width.md)
  ([ADR-0022](../adr/0022-markdown-and-html-reflow-to-a-width-in-characters-as-a-new-level.md));
  [ADR-0021](../adr/0021-a-backend-declares-its-settings-and-their-options-hold-for-any-file.md);
  [Options](../usage.md#options), [Keys](../usage.md#keys), [Documents](../usage.md#documents);
  [The room a document is laid out for](../architecture.md#the-room-a-document-is-laid-out-for),
  [Zoom and the grid](../architecture.md#zoom-and-the-grid-fitrs-pagerrs). Delivers none of
  README's Plans.

## Summary

Since [Spec-0003](0003-lay-markdown-and-html-out-to-the-window.md), `+` and `-` on Markdown and HTML
change the font size, and the reader lost the control they have everywhere else: how many pages go
to a row. This spec gives pages to a row keys of their own, `,` and `.`, and gives Markdown and HTML
a margin setting on `{` and `}`.

- **`,` `.`** set pages (or tiles) to a row everywhere: `,` one more to a row, `.` one fewer; a count
  sets it (`3.`), `0.` back to `auto`. On files with their own pages (PDF, Word, RTF, OpenDocument,
  images, drawings) and in lists they are `-` and `+`. On Markdown and HTML they give back the row
  as **columns**: N pages as tall as the window side by side, laid out again at the font size, as
  `window`'s choice of one page or a book of two does now. There `.` stops at one to a row: a
  reflowed page never runs wider than the window. Laid out again at once, as `[` `]` are; when a
  count to a row and a typed `width` disagree, the key pressed last wins.
- **`{` `}`** narrow or widen a Markdown or HTML page's side margins, in characters of the code face
  at the font size, as now, 0 to 20; the default goes from 2 to **3**, `0}` back to it. At `window`
  wider margins shorten the lines, and they count against the 60 characters each half of a book
  needs; at a typed width the line keeps its length and the page grows by them, up to the window.
  Elsewhere they do nothing and say so, as `[` `]` do.
- **`--zoom` becomes `--per-row auto|width|1-8`**, since it counts a list's tiles too. `--zoom`
  stays as a deprecated alias that warns.

## Requirements

**Pages to a row, on `,` and `.`**

- **Keys:** `,` one more page (or tile) to a row, `.` one fewer. `N.` and `N,` set N, clamped;
  `0.` and `0,` go back to `auto`. They are viewer keys, in `VIEWER_KEYS`. At the ends they say
  what `-` and `+` say now (`fully zoomed out`, `fully zoomed in`). "One more" and "one fewer"
  count from what is drawn, as the zoom does now: from `auto` showing a book of two, `,` gives three
  and `.` one.
- **The most to a row** is the grid's, everywhere: `fit::max_per_row` of the pages area's columns,
  at most 8 (`MAX_PER_ROW`). One count, the one the grid draws, is used for the layout, the fit and
  the status line alike.
- **On files with their own pages** they do what `-`/`_` and `+`/`=` do, through the same
  `Zoom::zoomed_out` and `zoomed_in`: `.` from one goes to the width. **In a list** they do what the
  list's `+` and `-` do: `.` stops at one tile, never the width.
- **With `--pages`**, on any file, they are the ordinary zoom, the width included, and never change
  a document's settings or lay it out again (ADR-0023 keeps a document with `--pages` as laid out).
- **On Markdown and HTML, shown by itself without `--pages`, columns:**
  - The viewer tells the document how many to a row through the `Room` it already gives it: a new
    field, `per_row: Option<u16>`, `None` at `auto`, compared by `Room::same`. No new method, and
    the core still names no backend.
  - With `per_row` set, the page at `window` is the room's columns less the gaps between pages,
    divided by N (checked arithmetic, at least a cell), and as tall as the room; the book rule is
    off. At `auto` it is as now: one page, or two where each half holds 60 characters of text.
  - `.` stops at one to a row, saying `1 per row, the least`; `--per-row width` there is 1.
  - `width` and `font` are bounded per column: `window`'s `named_at` and `width`'s `most` are a
    column's line, and `font`'s most the largest at which a column holds 20 characters
    (`LEAST_COLUMNS`). A font already past it is kept and laid out at it, as Spec-0003 does in a
    small room; only where even 50% leaves a column under 20 characters is it laid out at 20 and
    shown smaller, as a small room is now.
  - Laid out again **at once**, as a change of `width` or `font` is: the 500 ms hold stays for a
    change of the window's size alone.
- **The last key wins:** a count to a row (`,`, `.`, `N.`) sets `width` back to `window`; a key that
  types a width (`72]`, `[`, `]`) sets the count back to `auto` **first**, and steps from the bounds
  of the room at `auto` (`refresh_bounds` before the step): `3.[` steps from `window` where it sits
  at `auto`. `0.` and `0]` change only their own.
- **Recovery:** a key that changes both the count and `width` is one change: if its layout fails,
  both go back, the failed pair (values and room) is not retried until either changes or `r`, as
  Spec-0003 does for values, and a layout that comes back stale is dropped.
- **Options:** on Markdown and HTML shown by themselves, `--width N` with `--per-row N` (or `width`)
  disagree: the width wins, with a warning. `--width window` and `--per-row auto` agree with
  anything. Elsewhere (a PDF, a list, `--pages`) both hold, each for what it is about.
- **Printing:** the printed room carries `--per-row`, so `clv --per-row 3 notes.md > out` prints
  three-column pages at the room the tty says, or the 60 × 40 preview without a tty (Spec-0003);
  `--per-row 3` on a PDF prints three to a row as `--zoom 3` does now. Without the option, printing
  is as now.
- **In a list,** `--per-row` sets its tiles to a row; a document opened from it starts at `auto`, as
  now. A list's previews are laid out at `per_row: None`.
- **The status line** says `3 per row` as the zoom does now, and nothing at `auto`. On text cells
  (`--protocol blocks|ascii`) and inside tmux the same, through the same room.

**Margins, on `{` and `}`**

- The documents' backend declares **`margin`**: `Numbers` 0 to 20, step 1, no unit; default 3,
  zero 3 (`0}`); `Keys::Step('{', '}')`; option `--margin N`; effect `Relayout`; message
  `only Markdown and HTML have margins`; `--margin` out of range is refused before any file opens
  (`--margin: expected 0–20, got 30`), as `--width` and `--font` are. `SIDE_MARGIN` (2 today) goes;
  the default of 3 is a change a reader sees, in `CHANGELOG.md`, and tests that assume 2 move to 3.
- **In characters of the code face at the font size**, at each side, as the margins are now: N
  characters are N × F/100 cells on screen at font F. Top and bottom stay one row at the font size,
  as now.
- **At `window`** the margins come out of the line, and each half of a book needs 60 characters of
  text between its margins. **At a typed width** the line keeps its length and the page grows by the
  margins, never past the window: `width`'s `most` is the longest line the room holds within the
  margins.
- **The order bounds are worked out in**, each from the values as asked:
  1. The page's width in cells: the room, `per_row` and `width`.
  2. `font`'s most: the largest font at which that page holds 20 characters a line within the
     margin asked.
  3. `margin`'s most: the largest margin at which a line holds 20 characters at the font laid out
     at. `}` stops there, saying `margin 9, the most here`, from `Source::bounds` as `width`'s and
     `font`'s most are.
- **Asked and laid out:** a value past its most is kept, said with what it is laid out at
  (`margin 12 (9)`, as `font 300% (170%)`), and laid out at the most; `laid_at` gives back what
  was asked. In a room where even margin 0 leaves fewer than 20 characters, the margin laid out at is
  `min(asked, 1)`, never more than asked, and the page then grows as now.
- **In a list, or with `--pages`,** it does nothing and says why, as `[` `]` do.

**`--per-row`**

- `--per-row auto|width|1-8` replaces `--zoom` in `--help`, its help line the one `--zoom` has now
  (`auto: two side by side if they fit; width; or N pages to a row`), saying tiles in a list too.
- `--zoom` is still taken, with the same values, through an alias (a new field on `OptionSpec`,
  hidden from `--help`); both given is an error. Given, it warns once, before anything else, through
  `printable`: `--zoom is now --per-row; --zoom will go in a later release`. To stderr when printing,
  before the pager takes the screen otherwise, as the other warnings are.
- Errors name the option as typed (`--zoom "9": use auto, width or …`).

**Help:** `?` has a `, .` line and a `{ }` line; the hints say `, . per row` where they have room.

## Out of scope

- **Columns or a font size on files with their own pages** (Word, RTF, OpenDocument): their pages
  stay their own, `,`/`.` and `+`/`-` zoom them. The owner decided on 2026-11-09 not to reflow them,
  for now ([ADR-0023](../adr/0023-the-window-is-the-page-of-markdown-and-html.md)).
- **A reflowed page wider than the window that scrolls**: `.` stops at one to a row, as Spec-0003
  left it.
- **An overview of a reflowed document**, its pages shrunk N to a row: `,` gives columns there.
- **Top and bottom margins** of their own, and margins in screen cells whatever the font.
- **More than 8 to a row.**
- **Holding a key's relayout** as a change of size is held.
- **The count to a row and the margins kept between runs**, and a configuration file for them: a
  later spec, as for `width` and `font`.
- **Removing `--zoom`**, and when: it warns, and stays.
- **`{` and `}` in a list**: nothing, as `[` and `]`.

## Approach

**Files:**

- `src/source.rs` — `Room.per_row: Option<u16>` and `Room::same` comparing it; every `Room` literal
  (`src/source/later.rs`, `src/view/relayout.rs`, tests) in the same task.
- `src/source/wordcraft.rs` — `MARGIN`, replacing `SIDE_MARGIN`; `margins` and `page_in` taking the
  margin and `room.per_row`; `bounds` for `margin`, and `width`'s and `font`'s minding it, in the
  order above; `settings_of` with it.
- `src/view/fit.rs` — `Zoom::parse` naming the option as typed; `max_per_row` the one limit.
- `src/view/pager.rs` — `,` and `.` in `key` and `list_key`, by `+` and `-`; `VIEWER_KEYS`; the room
  `draw` gives carrying the reader's count on a reflowed document; the last key winning, with
  `refresh_bounds` before a width key's step; the failed pair covering the count; the status line,
  `?`, the hints.
- `src/view/mod.rs` — `--per-row`, `--zoom` its alias and its warning; the printed room carrying
  `--per-row`; its conflict with `--width` on a reflowed document.
- `src/args.rs` — `OptionSpec`'s alias, hidden from `--help`, refused with its option; every
  `OptionSpec` literal (the backends' `OPTIONS`, `Setting.option`s, tests) in the same task.
- Tests: `tests/it/view_pager.rs`, `tests/it/wordcraft.rs`, `tests/it/view.rs`,
  `tests/it/view_tmux.rs`, and unit tests in `wordcraft.rs`, `pager.rs`, `fit.rs` and `args.rs`, as
  the tasks list.
- Docs: `docs/usage.md` (Options, Keys, Documents), `docs/architecture.md` (the room, the zoom),
  `CHANGELOG.md`, the ADR.

**Patterns to follow:**

- `width` on `[`/`]` for `margin`: the setting, its bounds, its steps, `0]`
  (`src/source/wordcraft.rs`, `src/view/pager.rs`'s `turned` and `stepped`).
- Spec-0003's `Room`, `laid_at` and failed pair for the count reaching the layout.
- The zoom in `key` and in `list_key` for `,` and `.`, each its own.

**Decided:**

- On Markdown and HTML, `,` `.` are columns, not an overview.
- The last key wins between a count to a row and a typed width.
- Keys lay out at once; the 500 ms hold is for a change of size alone.
- Margins in characters at the font size, sides only, 0 to 20, 3 by default.
- `--zoom` a deprecated alias that warns.
- One spec: the keys must play together.
- An ADR for columns rather than an overview, the last key winning, and the order of the bounds,
  linked from ADR-0023.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- 🟡 **Between tasks 1 and 2**, `,` and `.` on Markdown zoom its window-sized pages as on a PDF, until
  task 2 makes them columns: fine for a commit, not to be mistaken for the end.

## Tasks

- [x] **`--per-row`, and `,` `.` as the zoom** — the option and its `--zoom` alias with the warning,
  the keys on every file, in lists (stopping at one tile) and with `--pages`, `max_per_row` the
  limit, `?`, hints, status line, docs. Every `OptionSpec` literal updated here. *Files:*
  `src/args.rs`, `src/view/mod.rs`, `src/view/fit.rs`, `src/view/pager.rs`, the backends'
  `OPTIONS`, `tests/it/view.rs`, `tests/it/view_pager.rs`. *Verify:*
  `cargo test -j 2 --bin clv args`, `cargo test -j 2 --bin clv view::pager`,
  `cargo test -j 2 --test it view` (`--zoom` taken and warning, both given refused, `,` `.` on a
  PDF and in a list, a narrow window's most).
- [x] **Columns on Markdown and HTML** — `Room.per_row` (every literal here), `page_in` at N,
  `.` stopping at one, the last key winning with bounds refreshed first, the failed pair covering
  the count, the options' conflict, printing with `--per-row`, the ADR. *Files:* `src/source.rs`,
  `src/source/later.rs`, `src/source/wordcraft.rs`, `src/view/relayout.rs`, `src/view/pager.rs`,
  `src/view/mod.rs`, `tests/it/wordcraft.rs`, `tests/it/view_pager.rs`, `tests/it/view_tmux.rs`.
  *Verify:* `cargo test -j 2 --test it wordcraft` (printed columns with a tty and without),
  `cargo test -j 2 --test it view_pager` (`3.`, `3.72]`, `3.[`, `0.`, `--pages` zooming, a failed
  layout restoring both, text cells), `cargo test -j 2 --test it view_tmux` (columns in a pane).
- [x] **Margins on `{` `}`** — the setting at 3, its bounds in the order above, the book counting
  them, `--margin`, `min(asked, 1)` in a small room, the changelog line. *Files:*
  `src/source/wordcraft.rs`, `tests/it/wordcraft.rs`, `tests/it/view_pager.rs`. *Verify:*
  `cargo test -j 2 --bin clv wordcraft` (`page_in` at margins 0, 3 and 20, at 50%, 100% and 200%;
  the most of each bound), `cargo test -j 2 --test it wordcraft`.

## Done

- [x] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [x] Tests added, each seen failing without the change: name them. (Named in the
  [notes](0005-set-pages-to-a-row-and-margins-impl-20261109_1812.md), task by task, with what was
  reverted.)
- [ ] Manual, with a fresh `cargo build` and `target/debug/clv`: in kitty, `--pager README.md`:
  `3.` three columns as tall as the window, text at the font size; `,` four; `.` back to one, then
  `1 per row, the least`; `3.` then `72]`, the count back to `auto`; `0.`; `}` wider margins, `0}`
  back to 3; `+` with three columns; on a PDF `,` and `.` as `-` and `+`; in a list, tiles to a
  row, `.` stopping at one; resized, laid out again once. `target/debug/clv --zoom 2 x.pdf` works
  and warns. The same in a private tmux (`tmux -L x`).
- [x] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- 2026-11-09 — Ready (spec-new), revised after a Codex review: the grid's most, lists, `--pages`,
  the options' conflict, the bounds' order, recovery, keys at once, margins at the font size and 3.
- 2026-11-09 — Implementing ([notes](0005-set-pages-to-a-row-and-margins-impl-20261109_1812.md)).
