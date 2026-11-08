# Spec-0002: Reflow Markdown and HTML to a width in columns

- **Status:** Implementing
- **Date:** 2026-11-07
- **Related:** [Spec-0001](0001-settings-a-file-declares.md), `Done`, whose settings this builds on
  ([ADR-0021](../adr/0021-a-backend-declares-its-settings-and-their-options-hold-for-any-file.md));
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
(`width 72`). Changing it lays the document out again, the **relayout** Spec-0001 left out.

A relayout is the document on screen opened again, as a new level: the source makes a new source
of itself at the new width, on the threads a list opens its files on, while the old pages stay on
screen; then a level for the new source takes the old one's place, at the reader's paragraph.
Nothing that exists changes: sources stay as opened, each pool keeps its one source. Only a
document reflows, never a list's tiles: in a list, `[` and `]` say `open a document to reflow it`,
and a document gone into from a list is laid out at the width wanted then. On any other file they
say `only Markdown and HTML reflow`. A page the window's size is
[Spec-0003](0003-lay-markdown-and-html-out-to-the-window.md).

## Requirements

**The setting**

- The documents' backend declares `width` as a `Setting` (`src/source.rs`): named value `page`
  (`Named { name: "page", says: "width page", at: Some(83) }`), default `page`, no `auto`; `Numbers`
  20 to 200, step 4; keys `Step('[', ']')`; option `--width page|N`; message
  `only Markdown and HTML reflow`; effect `Relayout`. Only Markdown and HTML have it
  (`Source::settings()`, and `Deferred` for a file not yet opened, as `paper`).
- `Setting` grows `effect: Effect` (`Redraw`, what `paper` is, or `Relayout`), and `Named` grows
  `at: Option<u32>`: where a named value sits among the numbers. Both are static, in the
  declaration: `page` is always 83, and `Nav` still knows no document. `Numbers` needs no unit: what
  is said is `width 72` and `--width: expected page or 20–200`, as `turned` and the option's message
  say numbers now.
- **A number N:** the page's text area is N characters of the code face (JetBrains Mono at 9.4 pt,
  5.64 pt each) wide, with the side margins as now. The page keeps Letter's height. A code block,
  indented 8 pt on each side (`wordcraft.rs`, its paragraph style), holds some 3 characters fewer.
  Tables and pictures fit that width: the page is given to the fragment's import, where tables take
  their widths and pictures their fit, not set on the document after it, as `page` is now.
- **`page`:** US Letter, exactly as now: a text area of 468 pt, some 83 characters. Nothing changes
  until the reader asks.
- **The order of the steps:** the numbers, and each named value with an `at` among them. From
  `page`, `]` goes to 84 and `[` to 80; from 80, `]` goes to `page`, and from 84, `[`. A number
  equal to a named value's `at` comes before it. This replaces what `turned` (`src/view/pager.rs`)
  does from a named value now (`]` to the least, `[` to the most); a named value without `at` keeps
  that.
- `N]` and `N[` set N, clamped to the range. `--width` out of the range, or not `page` or a number,
  fails with what it takes (`--width: expected page or 20–200, got 300`).
- Pages narrower than Letter go one or two to a row, as `auto` sets any page: nothing changes in
  `fit.rs`.

**Where it applies**

- **A redraw setting** (`paper`) applies everywhere, lists included, as Spec-0001 built it.
- **A relayout setting** applies only to a document on screen: a level that shows one file's pages,
  opened alone or gone into from a list.
  - In a list, its keys change nothing and say `open a document to reflow it` (after the setting's
    own message, if no file shown has it); a count typed before them is used up.
  - While `--pages` is given, its keys change nothing and say
    `--pages picks pages as laid out: no reflow`. `--width` with `--pages` lays out at that width
    first, and the pages are picked from it.
- **The value is one for the run**, in the pager's shared `Settings`, as `paper`'s. A list's files
  open with the options given at start (`--width` among them), so its tiles are at that width for
  good: they are previews. A document whose layout isn't at the value wanted is laid out again when
  it is the level on screen: after `[` or `]`, and when it is gone into from a list after a change.
  Until the new layout is ready, its pages as they were show.

**Relayout**

- A relayout value is the source's own: the documents' source is opened with a width, and lays out,
  renders and copies at that width, whatever a request carries. It says the values it was laid out
  at (`Source::laid_at() -> Values`, none for any other source).
- **The job:** `Source::relayout(&self, values) -> Option<Job>` gives, for values its layout isn't
  at, a `Send + 'static` closure that owns what it needs (the bytes the source keeps, and the
  values) and gives a new leaf `Box<dyn Source + Send>` laid out at them, or why not. It captures no
  `Rc` and nothing of the viewer's. A panic in it costs the relayout, not the viewer, as an opening's
  does.
- **Where it runs:** on `Openers`' threads (`src/source/later.rs`), first in their queue: the reader
  is waiting on it. The pager is given a handle it polls, never waiting.
- **One under way, one waiting:** the level on screen has at most one job under way and one set of
  values waiting. A layout once started runs to its end (ADR-0012). A change while one is under way
  replaces what waits. When a job ends, a result for values no longer wanted (success or failure)
  is dropped, and what waits starts.
- **Taking it in:** the pager polls the job on each pass of its loop, as it polls a file gone into
  (`go_on_into`), waking every `POLL` while one is under way, so the new pages show without a key.
  A result for the values wanted makes a new `Level` (a new id, a new pool, the same zoom, `nested`
  as before) that replaces the one on screen, its parents untouched; then the old level goes as
  `back()` lets go of one: its pages sent are deleted by its id, its renders wanted dropped, and a
  copy still rendering on its pool stops and says so (`clip::Copier::left`). A copy already
  encoding or sent goes on: it is of the page the reader asked for.
- **Not through `changed`:** a relayout value changed doesn't set the shared `changed` flag, which
  deletes every page sent at once and is for redraw settings. Instead, after its keys and on each
  pass, the pager compares the document on screen's `laid_at()` with the values wanted, for the
  relayout settings it has, and asks for a job when they differ and none is under way or waiting
  for them. So `]` then `u` read together loses nothing: the list doesn't reflow, and the next
  document gone into is laid out at the new value.
- **Leaving** a level (`u`, Esc) with a job under way: the job runs to its end and its result is
  dropped.
- **A failure** of a job for the values wanted keeps the old level, says why through `printable`,
  and puts the shared value back to the document's `laid_at()`, so the next step starts from what is
  on screen.
- **While it runs**, the status line shows `…` after the value (`width 84…`).
- **The place:** the paragraph at the top of the screen before the change is on the reader's page
  after it; where a page scrolls (as wide as the window, taller than it), it is at the top. The
  source says which paragraph is where (`Source::anchor(page, y)`,
  `Source::find(&Anchor) -> (page, y)`), from what its layout keeps for it: each page's paragraphs,
  by their order in the document, and their heights. A paragraph split across pages is found on the
  page it starts on; one not found (the end of the document) goes to the last page. An anchor from
  the old source is found in the new one: an `Anchor` means nothing to the viewer, and one the new
  source doesn't know goes to its first page.
- Laying out again costs what opening did, within the same [Limits](../usage.md#limits); while it
  runs, the old level's layout is held too. At 20 characters a document has some four times the
  pages it has at `page`: see *Cost* under Done.

**Showing it**

- On a document, the status line shows a relayout setting's value when it isn't the default
  (`width 72`), the document's `laid_at()`, and drops it, before the zoom, in a narrow window.
- `c` copies the page as it is laid out.

## Out of scope

- **Reflowing a list's tiles**, and keeping a document's new layout for its tile or for going into it
  again: the tile keeps the layout it was opened with, and going into it again lays it out again.
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

- `src/source.rs` — `Setting.effect` (`Effect::Redraw`, `Effect::Relayout`) and `Named.at`;
  `Source::laid_at`, `Source::relayout` and `Job`; `Source::anchor` and `Source::find`, an `Anchor`
  meaning nothing to the viewer. `Keys::Step`'s `expect(dead_code)` becomes
  `not(any(feature = "word", test))`, as `Cycle`'s.
- `src/source/wordcraft.rs` — declares `width`; `Document` keeps its width, and `marked_up` takes its
  page from it rather than always Letter. `html::import_with` sizes tables to the default page, before
  `last_section` is set (`marked_up`), so the page goes into the conversion itself: wordcraft's
  `parse_with` then `model::to_doc_in`, or whatever it offers that takes a page, keeping the fragment's
  styling (`import_on` would read it as a web page, ADR-0017); `relayout` reads the bytes again from `Document.stored`;
  the layout keeps each page's paragraphs and heights for anchors (`Placed::Lines`).
- `src/source/later.rs` — `Openers` runs a job as well as a file: a slot's work is either, and a
  job's handle says when it is done (`try_take`), never waiting.
- `src/view/relayout.rs` (new) — the bookkeeping, with no IO: the values under way and waiting,
  what a key or a pass asks to start, and what a result comes to (take in, drop, fail). Unit-tested
  with values alone.
- `src/view/pager.rs` — `turned` steps by the order with `at`; `setting_key` refuses a relayout key
  in a list and with `--pages`, and doesn't set `changed` for one; the pass comparing `laid_at()`
  with the values wanted; polling the job; a new `Level` replacing the one on screen; the status
  line.
- `src/view/mod.rs` — `--width` at start, through the options as `--theme`; `print_pages` lays out
  at it.
- Tests: in `wordcraft.rs`, a page's text width for N, `page` as now (its styling and its tables'
  widths unchanged), a Markdown table and an HTML one within the text width at 40, `laid_at`, a job's source at
  its width, an anchor found again; in `relayout.rs`, a change while one is under way waiting,
  replaced by the next, a stale result and a stale failure dropped, a failure putting the value back;
  in `pager.rs`, `[` refused in a list and with `--pages`, `]` then `u` not setting `changed`;
  `tests/it/wordcraft.rs`, `--width` printed changes the pages' count and size;
  `tests/it/view_pager.rs`, `]` lays out again and the new pages show with no key after it, a known
  paragraph stays on screen, `[` on a PDF, a document gone into from a list after `]` laid out at
  the new width while its tile isn't, piped HTML.
- Docs: `docs/usage.md` (Keys, Documents, the options, Limits as measured at 20),
  `docs/architecture.md` (Sources, Lists and levels: a relayout as a level replaced),
  `CHANGELOG.md`, and an ADR: a width in characters of the code face; relayout as the document
  opened again into a new level, rather than a source that changes; documents only, not tiles;
  coalescing rather than stopping.

**Patterns to follow:**

- A file gone into from a list, waited for and then shown as a level: `go_on_into` and `enter` in
  `src/view/pager.rs`.
- A level let go of, its pages deleted by id: `back` in `src/view/pager.rs`.
- Opening on threads, a panic costing only the file: `Slot::open` in `src/source/later.rs`.
- The page Markdown and HTML go on: `marked_up` in `src/source/wordcraft.rs`.

**Decided:**

- The width is the source's, given when it is laid out; a request's values don't lay out.
- A relayout makes a new source and a new level; no source, pool or level changes in place. A
  shared wrapper holding a source that changes was rejected: every part of the viewer (`Nav`'s page
  count, the pools, renderers borrowing their source, `Later` and files opened at once) would have
  to learn that a source changes.
- Only a document on screen reflows; a list's tiles keep the layout they were opened with.
- The source lays itself out again, from the bytes it keeps, rather than the viewer opening the
  file again: it needs no `Reopen`, and piped HTML works.
- Only the latest change is laid out; a layout under way runs to its end.
- The place is kept as the reader's page, and the paragraph at the top only where a page scrolls:
  the pager doesn't learn to start a screen part-way down a page.
- `[` and `]` are refused while `--pages` is given, rather than the selection dropped.

## Open questions

- 🟡 **Cost at 20 characters** — measured by the first task; it may raise the least.

## Tasks

Each task's commit passes the quality gates alone: what it adds is called by the viewer in that
task, not only by tests.

- [x] **A width, laid out again while the reader waits** — `width` declared, with `effect` and
  `at`; `turned`'s order; `--width` printed and in the pager; `laid_at` and `relayout`; in the
  pager, `[` and `]` on a document run the job on the main thread and replace the level, the page
  number kept (clamped), the old level let go of as `back()` does, a copy still rendering on it
  stopped (`clip::Copier::left`), tested with `c` then `]` before the copy's render is done; refused in a list and with `--pages`; a document gone into after a change
  laid out then; the status line; the cost measured at 20. *Files:* `src/source.rs`,
  `src/source/wordcraft.rs`, `src/view/pager.rs`, `src/view/mod.rs`, `tests/it/wordcraft.rs`,
  `tests/it/view_pager.rs`. *Verify:* `cargo test -j 2 --bin clv view::pager`,
  `cargo test -j 2 --test it wordcraft`, `cargo test -j 2 --test it view_pager`; manual: in kitty,
  `target/debug/clv --pager --width 40 README.md`: narrow tall pages, two to a row; `]` widens them.
- [ ] **On the openers' threads** — `Openers` runs jobs; `relayout.rs`; the old level on screen
  until the new one is ready, `…` meanwhile, polled with no key; only the latest; a failure keeping
  the old and putting the value back; a copy still rendering stopped, as in task 1. *Files:*
  `src/source/later.rs`, `src/view/relayout.rs`, `src/view/pager.rs`, `tests/it/view_pager.rs`.
  *Verify:* `cargo test -j 2 --bin clv view::relayout`, `cargo test -j 2 --bin clv source::later`
  (a job run, its panic caught), `cargo test -j 2 --test it view_pager` (the new pages show with no
  key after `]`); manual: in kitty, on the costliest file of *Cost*, `]` three times quickly: keys
  and scrolling keep working, `width 96…` and then `width 96`.
- [ ] **Keeping the place** — anchors. *Files:* `src/source.rs`, `src/source/wordcraft.rs`,
  `src/view/pager.rs`, `tests/it/view_pager.rs`. *Verify:* a pager test with a known paragraph at
  the top before `[` and on screen after.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass, after each
  task, each backend alone too (`cfg` attributes change).
- [ ] Tests added, each seen failing without the change: name them.
- [ ] **Cost:** the files and commands ADR-0012 measured with, under `/usr/bin/time -l` on a release
  build, printed with `--width 20` and with `--width page`: at 20, time and peak memory stay within
  what [Limits](../usage.md#limits) says a Markdown or HTML file costs at most (about 5 GB and 20
  seconds); otherwise the least is raised until they do. And the costliest of them in the pager,
  `]` pressed once it shows: the peak, old and new layouts held together, written in Limits.
- [ ] Manual, with `target/debug/clv` built last by a plain `cargo build` (the gates leave it built
  with whatever features they built last), always run by that path: in kitty, `[`, `]` and `72]` on
  a long Markdown file; on HTML through a pipe (`cat page.html | target/debug/clv --pager
  /dev/stdin`); `]` on a PDF; in a list mixing them, `]` refused, and a Markdown file gone into after
  `]` in another shown at the new width; `--pages 2-3` with `]` refused.
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)), with the
  ADR.

## Changelog

- 2026-11-07 — Ready (spec-new).
- 2026-11-07 — Back to Draft after a review: the width the source's own, a shared wrapper for
  relayout, coalescing rather than stopping, the place and `--pages` defined, `window` moved to
  Spec-0003.
- 2026-11-07 — Ready: one shared wrapper, the place as the reader's page, `[` and `]` refused with
  `--pages`.
- 2026-11-07 — Brought up to Spec-0001 as built: `effect` and a static `at` on the declaration,
  no unit, a change taken from the shared `Settings`; the wrapper and the pager split into two
  tasks.
- 2026-11-07 — Back to Draft after a review: the shared wrapper dropped for a relayout as a new
  level; only a document on screen reflows, never a list's tiles; not through `changed`; jobs
  polled on the openers' threads; tasks staged so each commit passes the gates; cost and manual
  checks made measurable.
- 2026-11-07 — Ready after a review: the width given to the fragment's import, so tables fit it;
  a copy under way stopped from the first task.
- 2026-11-07 — Implementing ([notes](0002-reflow-markdown-and-html-to-a-width-impl-20261107_2307.md)).
