# Spec-0004: implementation notes (2026-11-09)

Notes made while implementing [Spec-0004](0004-draw-an-svgs-html-labels.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 1: measured

- **Where the tool lives:** the owner put it in `docs/adr/0024-measure/` on this branch, a crate of
  its own with its own `[workspace]`, rather than on the spike's branch as the task's wording has
  it; nothing was committed to the spike. It isn't in the task's *Files:*. Its fixtures are
  generated (`measure gen`), never kept; the real drawings it was compared on are clients' files,
  named in the spike's `spike/NOTES.md`.
- **One document holds,** with one change from the spec's sketch: the slots are placed by
  `transform: translate()`, 8 CSS pixels apart. Placed by `left`/`top`, each label's recording
  carried an empty clip layer for every slot between half its position and its own: Blitz's paint
  culls by `scrollable_overflow ∪ border_box`, and the border box is in its parent's coordinates
  while the transform it goes through already holds the position. Slots that touched weren't
  culled either, hence the gap. Seen with `COUNTS=1` and `DUMP=<i>`.
- **The packing** first held rows to 65,536 CSS pixels; with `overflow: visible` labels (slots
  8,192 pixels larger) that made the document 1.2 million pixels tall, where an f32 is a tenth
  of a pixel apart. Rows are now about as wide as all the slots would be square.
- **A transparent fill** the size of the whole document starts every label's recording (the root's
  background, painted whatever the viewport). It draws nothing; task 3 may drop it.
- **Limits that moved** from the spec's starting table: label elements 100,000 (new), commands
  200,000 (from a million), overflow 4,096 CSS pixels (the spec named no number); and the nesting
  limit's thread. The owner chose a thread of 8 MiB for Blitz (option C) over 64 on 2 MiB sized
  by release builds, or 32 on 2 MiB. Each is reasoned in ADR-0024.
- **One document a label** for 1,024 labels under 3 MiB of styles was stopped after 10 minutes:
  this stylesheet costs 1.7 s a label (16 labels: 27 s), more than the spike's (111 s for 1,024).
- **wuff** sizes a WOFF 1 table's output by its header's `origLength`, unbounded: the hostile font
  is a WOFF 1. WOFF 2 it caps itself (128 MiB, a ratio of 100). The header must say the file's
  padded length, or wuff refuses it before unpacking.
- **Blitz requests a repeated `data:` URL once** a document (1,024 labels of 256 distinct pictures
  made 256 requests).
- **Verify:** every row of the ADR's tables was run again from `docs/adr/0024-measure/` under
  `/usr/bin/time -l`; the numbers are the second run's, within a few percent of the first.

## Task 2: labels found, behind `svg-html`

- **The stub painter:** `svg/html.rs`'s `prepare` paints nothing yet, so a build with `svg-html`
  says its labels weren't drawn, as one with `svg` alone does. `CREDIT` is there, credited from
  task 3, when the labels are painted.
- **The warnings** follow the file's own wording: "3 HTML labels (<foreignObject>) were not
  drawn" (the spec has "N HTML labels (`<foreignObject>`) not drawn"), and "… were left out, past
  1024 of them" (or "nested more than 64 deep", "past 1 MiB of text", "past 100000 elements",
  "past 8 MiB of markup").
- **One walk finds the labels** (`drawn`), top down: a first version checked each label's
  ancestors for itself, which a `<switch>` of 50,000 labels it doesn't take made 2.5 billion
  steps (14 s in a debug test). A `<use>` counts only where it is drawn (not in a `defs`), and
  each target once.
- **Labels in an embedded SVG picture** are counted as not drawn, and their fallbacks kept: only
  the file's own drawing has its labels drawn.
- **Fallbacks are cut in every `<switch>`,** after the child taken, as the spec says. An element
  in a fallback that a `<use>` elsewhere refers to goes too, where a browser would still draw the
  use. The scan of ADR-0003 counts the drawing with its fallbacks, so it errs high.
- **The root's viewport** for percentages is its `viewBox`, else its own `width` and `height`;
  a root with neither absolute takes 100 × 100, usvg's size for a drawing that draws nothing.
- **Cargo.lock:** Blitz's tree moved serde_spanned (1.1.2 to 1.1.3), toml_datetime (1.1.2 to
  1.1.3) and toml_writer (1.1.3 to 1.2.0), which its crates need; wuff stays at clv's 0.2.9. The
  measuring tool's lock is now clv's, so the two build the same versions; ADR-0024's numbers were
  measured again with it, the same within noise, and its versions line says so.
- **CI** builds `svg-html` alone and `pdf,svg`, as the spec asks; the integration tests with the
  labels found (`--features svg-html`) run there once task 3 makes the feature a default.
- **Tests, each seen failing with its fix reverted** (the file copied aside and back, `cmp`):
  - `the_labels_a_browser_draws_are_told_by_one_rule`: the walk not skipping hidden,
    definitions and labels in labels; `taken` ignoring `requiredExtensions` and
    `systemLanguage`; a `<switch>`'s every child walked; labels through a `<use>` not counted.
  - `a_fallback_is_drawn_only_where_labels_are_not`: `without_fallbacks` never used.
  - `entities_in_a_fallback_are_refused_before_it_is_parsed`: `xml` not bounding what it parses.
  - `labels_past_their_limits_are_left_out_and_said`: each of the five limits taken out in turn.
  - `nothing_but_svg_html_names_blitz`: Blitz named in a comment of `svg.rs`.
  - `the_label_limits_are_for_the_blitz_this_builds`: anyrender's version changed in
    `Cargo.lock`, run on the test binary already built (cargo itself would put it back).
  - `a_label_is_written_out_from_the_tree`: the label's children copied from the file's text.
  - `a_label_is_written_inside_its_context`: the font and colour attributes left out.
  - `a_label_is_placed_through_the_viewports_it_is_in`: nested `<svg>`s not made viewports; the
    root's own transform applied.
  - `labels_are_found_in_one_walk`: the first version's per-label checks and every `<use>`.
- **A label's ancestors' copies are checked against the markup budget one by one,** so an
  ancestor with a style as long as the drawing ends the labels before a second one is copied.
  No test tells it from checking once at the end (both leave the labels out): only the memory
  held on the way differs.
