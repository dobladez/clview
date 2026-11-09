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

## Codex's review of task 1

- **Commands, counted as painted:** the tool's `Recorder` wraps anyrender's `Scene` behind its
  `PaintScene` trait and keeps commands while 200,000 last. It bounds memory but not time: Blitz
  still walks each command it keeps none of, 3,125 million in 18 s. So the owner chose to estimate
  the paint work too, after layout and before painting (option A, over a panic that stops the
  painting, against the rules, or the recorder alone). The estimate reads Stylo's computed
  values, which needs `stylo` named; the tool depends on it directly, and clv's `html.rs` will
  from task 3.
- **The estimate, with clv's code and its tests, lands in task 3,** where `html.rs` has its
  document and painter: the review's commit holds the tool, the ADR and the spec.
- **A picture a style asks for** (`background-image: url(data:…)`) is fetched as the styles
  resolve and answered after: the document's styles resolve again once their messages are
  handled. Before that, the `tile` row drew no picture at all (7 commands then, 8 now).
- **The styles fixture** is now rules of some 380 bytes, 8,313 of them under 3 MiB: 42.6 million
  rule tries with the document's wrappers, where the first one's 37,401 short rules passed 50
  million at element 1,337. `rules 10000 5000` was over by a little too: now `rules 9900 5000`.
- **Every row of the ADR was measured again** with the tool as it now is.

## Codex's review of task 2

- **Labels through `<use>`s** are counted by the rule `drawn` follows (`shown_parts`, shared), in
  one pass that remembers what each element shows: nested uses followed, a use referring back to
  one being counted showing nothing more. The first version walked every target's descendants,
  `defs` included, afresh: 4,000 nested targets took 2.9 s in a debug test, now 0.3 s with the
  parse. A cost bound alone couldn't tell the two apart below that size: roxmltree walks
  descendants as a run of its node array, 100 million in 0.4 s.
- **A fallback an entity makes** is kept, with its `<switch>`'s other cuts: its range is in the
  entity's declaration, and cutting it would cut every element the entity makes. A known
  difference, for usage (task 4). Pruning the tree and writing it out again was the other way;
  refusing the cut is simpler and as sound.
- **The child taken loses its `requiredFeatures`** (its attribute's range cut with the
  fallbacks), so usvg draws it: draw.io's `<g requiredFeatures=…>` with shapes in it.
- **Markup** is written through `Markup`, which knows what an escaped string comes to before it
  appends it; `affordable` keeps to the text and the elements. A `font-family` longer than 4 KiB
  is left out rather than quoted at its length.
- **Tests, each seen failing with its fix reverted:** `labels_through_uses_are_counted_once_an_element`
  (the first version's per-target descendants: 2.9 s against a bound of 1); the rule's new cases
  in `the_labels_a_browser_draws_are_told_by_one_rule` (the same revert: a hidden target, a
  branch not taken, a nested use, a cycle, a hidden use, a use in a hidden group);
  `a_switchs_child_taken_is_drawn_whatever_features_it_asks_for` (the attribute not cut);
  `a_fallback_from_an_entity_is_left_with_what_else_the_entity_makes` (the body check taken
  out); `markup_past_its_budget_is_refused_before_it_is_written` (escaped, then checked).

## Codex's review, round 2

- **Shared nodes:** each label's painting paints the document's `html` and `body` (and their
  `::before` and `::after`) again, so the estimate counts what they paint once a label; it walked
  no pseudo-element before, which it now does. Charging them was the sound one of the two ways:
  keeping a drawing's styles from making them paint would mean a user-agent rule for every
  property that paints, and their pseudo-elements.
- **Paths at a box's size:** blitz-paint 0.3.0-beta.4 builds a dotted border's dots into one path
  (some 170 bytes a dot), and draws a dotted, dashed or wavy decoration as a dot, a dash or a wave
  a pixel or two along each line (its thickness floored to a pixel); a dashed border is one stroke
  vello_cpu dashes as it replays. Outlines are one path whatever their style, and a radius's
  corners a few segments. So an element, or a line of text, past 24,576 pixels leaves its label
  out, and the estimate counts dots (20 steps each), decorations (4 steps a pixel of line) and a
  gradient's stops (a step for each 32): 20 million steps are a million dots, 191 MB.
- **A label of much text is left out now:** 1 MiB of it in an 800-pixel box is 950,000 pixels of
  lines, past 24,576.
- **Sub-pixel borders** are no border: 0.004 pixels dotted came out as nothing, so cost nothing.
- **A percentage label in a drawing that says no size** (no `viewBox`, no absolute `width` and
  `height`) is placed against the size usvg gives it from what it draws: the tree is made first,
  the first parse kept for the labels only in that case.
- **A discovery, for its home (ADR-0003's scan):** usvg's stylesheet parser (simplecss) takes time
  as the square of a declaration's length, working out a position for each part: a `<style>` with
  one 600 KB `box-shadow` list takes 45 s to open in a debug build, labels or not.

## Task 3: labels drawn, bounded, on by default

- **`html.rs`** has the painter: the labels' one document (slots packed square, placed by
  `transform`, 8 pixels apart), its styles resolved twice (for the pictures a style asks for), the
  paint work estimated, each label recorded through `Recorder` (200,000 commands at most, a fill of
  a colour with nothing of it, the root's `transparent`, not kept), and the document dropped; then
  each render's replay into the part of the page a label lands on, and `cost`, its commands and the
  pixels they paint there, for the page's estimate. `svg.rs` places the parts, adds them to the
  estimate, and draws them over resvg's pixels at the label's opacity.
- **A drawing being opened isn't laid out** (`guarded_open`): opening keeps only its size, so its
  labels are prepared by a render's parse alone, once.
- **Blitz's base URL** is `file:///`: its own, a `data:` URL, can't be a base, and Blitz panicked
  resolving a label's `<img src='other.png'>` against it (caught, and the labels left out: found by
  the links test). A relative link now resolves to a file not read, and is counted as a link.
- **Bugs the tests found:** a refused font or picture was counted in its drawing's total; a label
  left out for its size took the next label's slot.
- **The over-6,000-pixel test** renders a page 160 times the drawing (6,400 pixels) in a unit test:
  through a pipe the window is 80 × 24 cells, and `--zoom` sets pages to a row. Its label reaches
  4,000 units past the page, so an unclamped part is 1.3 million pixels across.
- **The stack:** clv's dev profile builds its dependencies at `opt-level = 2`, so Blitz in its tests
  holds 392 nested grids on 2 MiB, not the 58 of the tool's unoptimized build. The 8 MiB thread
  stays (margin, and unoptimized builds); the ADR has both. The at-limit test passes on 2 MiB too:
  seen failing on 128 KiB. No fixture within the limit can tell 8 MiB from 2 MiB as clv builds:
  2 MiB holds 392 nested grids, the costliest nesting tried, six times the 64 a label may nest.
- **New dependencies:** `stylo` (approved) and `peniko`, which anyrender's `PaintScene` names in its
  signatures and doesn't re-export (asked for).
- **Task 2's tests** with a label of the text "a" over a pixel they read now hold an empty label,
  and the rule's test expects, where labels are drawn, only those a `<use>` repeats as not drawn.
- **Tests, each seen failing with its fix reverted** (copied aside, `cmp` after): placed (the label's
  place ignored); overflow (`OVERFLOW` 0); the page over 6,000 pixels (the part not clamped to the
  page); opacity (ignored); the drawing's rules (not given); a picture (not answered); hints (of a
  specificity; none); paint work, once each for shadows, gradients and spaced tiles (each not
  counted); the recorder (keeping everything); layout (any size); styles (the labels' elements not
  counted); the stack (128 KiB); the page's estimate (the labels' pixels not counted); links (not
  told); the base URL (none); fonts (not sized); requests (unbounded); a spaced tile of no size;
  a face pasted into its rule.
- **Not tested:** a Blitz panic leaving the labels out (no panic to call up but a Blitz bug; the
  base URL one was, before its fix), and a thread that can't be spawned.
- **A discovery, for its home:** the drawing's scan counts a stylesheet's `@font-face`, and its
  `data:` URLs, as links not followed, so a tldraw drawing says "2 links … not followed" while its
  labels draw that font. Older than this spec (ADR-0003's scan).

## Links the labels follow

- **A stylesheet's `data:` URLs**, and its `@font-face` itself, are no links not followed where
  labels are drawn, which fetch them: the scan counted them, so a tldraw drawing said "2 links …
  not followed" while its labels drew its font. Its other URLs still count; a build without
  `svg-html` counts all of them as before. `a_stylesheets_data_urls_are_links_only_where_labels_are_not_drawn`
  failed with the `data:` URLs counted (3 for 1), and with `@font-face` counted (2).
- **A known issue, not this spec's:** usvg's stylesheet parser (simplecss) takes time as the
  square of a declaration's length. A `<style>` of one `box-shadow` list of 40,000 shadows
  (about 600 KB) took 45 s to open in clv's debug build, labels or not: ADR-0003's scan doesn't
  bound it. Raised with the owner as a follow-up.
