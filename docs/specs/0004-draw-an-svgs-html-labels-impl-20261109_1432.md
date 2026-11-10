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

## Task 4: the docs

- Usage's SVG drawings say which labels are drawn, how they look and what they read, and where clv
  draws one otherwise than a browser (the known differences: over the whole drawing, unclipped, a
  stylesheet's `display: none` and some selectors unseen, Blitz's `display: table`, an entity's
  fallback kept, overflow cut at 4,096 pixels, no selecting); Limits has the labels' four rows, and
  the page's estimate their parts. Architecture has `svg/html.rs` in Files, the backends table, and
  How each opens. The ADR is Accepted.
- README's Plans gains a bullet, at the owner's word: bounding what an SVG's stylesheets take to
  parse before they are parsed (the known issue above).

## Codex's review, round 3

Seven findings. The owner moved Blitz into a worker process, capped on time and memory, within
this spec, so the four that are estimate work (a decoration's insets, the text styles write
before layout, a clip path's points, a shared decoration once a pass) and the WOFF table check
were held, uncommitted; the two that hold in either design are fixed:

- **Stylesheets fetched:** Blitz asks the net provider for a URL without saying what for, and
  the provider told a font by its first bytes, so a `<link rel=stylesheet>` whose bytes began
  `OTTO` was answered and parsed as CSS. The labels' markup now holds no element that loads a
  stylesheet or a document (`link`, `iframe`, `frame`, `frameset`, `object`, `embed`, `base`,
  `portal`, `style`), in any namespace; a label's `<style>` joins the drawing's stylesheets,
  which Stylo reads and writes out again without `@import` before Blitz is given them. What is
  left asks only for fonts (`@font-face`) and pictures. `a_label_loads_no_stylesheet_or_document`
  failed with no element left out (the `<link>`), with `style` kept in the markup (its `@import`),
  with the `@import`s kept, and with a label's `<style>` not joining the drawing's (its rule).
- **Opacity twice:** a label's own `style` opacity is kept on its copy, which Blitz applies, and
  the composite counted it again: 0.5 drew at a quarter. Its own `opacity` attribute, which the
  copy doesn't keep, still counts, as its ancestors' do. Failed with the rule reverted (alpha 64
  for 128).
- **The styles fixture, corrected:** its 8,313 rules were within 50 million against the labels'
  document alone, but clv adds the drawing's elements (ADR-0003's scan), so clv left all 1,024
  labels out. Rules of some 750 bytes, 4,202 under 3 MiB, are 34.4 million as clv counts them:
  0.41 s and 120 MB as one document, 235 s and 414 MB as one a label.

## Task 5: Blitz in a worker

- **Built on the stopped session's work**, left uncommitted on 791d63f and never put through a
  gate: `worker.rs` and its tests, `WORKERS`, `--worker`, `Request::tile`, the split of `html.rs`
  into the worker's side (`Labels`, `Kept`) and the viewer's (`Painted`), `svg.rs`'s `Labelled`
  (labels found at parse, laid out by the first render that isn't a tile's, `failed` once the
  worker goes). Reviewed as new code; what changed:
  - **A dead worker gave up its place too late:** `Link` held its `Alive` until dropped, so a
    drawing whose worker was killed still counted among the six until the drawing was let go.
    `end` drops it now.
  - **Links back as strings, not a count.** The drawing's own scan counts a stylesheet's
    `url(https://…)` too, and `Notes::linked` dedupes by hash: a bare count from the worker said
    "2 links" for one (`a_stylesheets_data_urls_are_links_only_where_labels_are_not_drawn`). The
    worker tells each link, cut to 256 bytes on a character boundary (`LINK_BYTES`), at most 1,024;
    the viewer refuses more or longer, hashes them and shows none. `MAX_PREPARED` is 1 MiB for
    them. The spec's "counted, not named" is so in effect: nothing of them is shown.
  - **Codex P2, a document's pictures:** `picture::Promise::draw` made a default `Request`, so a
    Markdown or HTML tile referring to a labelled drawing started a worker. `tile` goes from
    `drawn_pictures`'s request through `Promised::draw` and `hold`; a picture drawn as a tile is
    remembered as one (`Held::as_tiles`) and drawn again, taking its place, when a page wants it,
    since the document keeps its pictures across tile and page renders.
  - **No global counter of workers for the tests:** a first tile test compared
    `worker::spawned()` before and after, and the other tests' workers, started beside it, made
    it flaky. It inspects the renderer's `Labelled::prepared` instead; the pager test counts
    `clv --worker` processes descended from its own `clv` (`ps -axo pid,ppid,command`).
  - **The estimate's parts term** got a test of its own (`a_labels_part_of_the_page_is_in_its_
    estimate`): a label the page's size at 16,000 × 8,000 doubles 512 MB past 1 GiB.
- **The nesting limit's premise, corrected:** the drawing's scan skips children by local name,
  in any namespace, so an XHTML `<metadata>` inside a label hides its depth from the scan as the
  spec says; but plain nesting is counted, root to leaf, and a label can nest at most 1,021 divs
  (the root, the `<foreignObject>` and its div are three levels) before the *drawing* is refused.
  The writer's 1,024 is reached only under a skipped name: `labels_past_their_limits_are_left_
  out_and_said` tests 1,021 drawn and 20,000 under `<metadata>` left out, with the writer's
  warning and no crash.
- **The `content` bomb is 99,000 elements**, not the spec's 100,000, which are past
  `MAX_LABEL_ELEMENTS` before the worker (the wrappers count too). The spec's line says so now.
- **How the worker's death reads:** `SIGXCPU` is "took too long", `SIGABRT` "took too much
  memory". A stack overflow in the worker aborts too, so it would read as memory; none can be made
  from input (the writer's 1,024 levels, the worker's 64 MiB stack).
- **The hang** (`text-decoration-inset: -1e20px`) dies by the heap cap, in under a second: the
  loop grows its path as it goes. Its warning is the memory one; the test takes either.
- **Tests, each seen failing with its fix reverted** (the files copied aside, `cmp` after):
  the worker's `end` without its kill (`a_worker_that_doesnt_answer…` and
  `a_worker_that_stops_reading…`: "reaped"; the second asserted nothing of the kill before, and
  does now); `why` mapping no signal (`how_a_worker_ended_is_told`: `Ended("signal 24")`);
  `Alive::take` without its bound (`past_the_most_workers…`: the seventh started); `limit_cpu`
  returning early (`a_worker_is_stopped_past_its_cpu_time`: still spinning after 20 s);
  `read_frame` without its length check (`a_worker_answers_its_requests_in_turn`: longer than it
  takes; `an_answer_that_isnt_one…`: `yes` not killed); the WOFF table checks off
  (`a_woffs_inconsistent_tables_are_refused`: 64 for larger than any); `Held::as_tiles` ignored
  (`a_picture_drawn_as_a_tile…`: not drawn again); the parts term zeroed
  (`a_labels_part_of_the_page…`: the page rendered); `drawn_pictures` given `false`
  (`a_tiles_pictures_are_drawn_as_tiles`: the tile's label drawn); `request.tile` ignored
  (`a_tile_draws_no_labels…`: blue at the tile's label);
  the pager's `tile` always false (`a_lists_tiles_start_no_worker…`: two workers for the tiles);
  the labels' links not told (`the_links_a_labels_worker…`: none said); `MAX_LABEL_DEPTH` unbounded
  (`labels_past_their_limits…`: the 20,000-deep label drawn, no warning: the printer's 8 MiB
  main thread holds that recursion, a render thread's 2 MiB wouldn't); a `cap::` written in
  another file (`nothing_but_the_worker_names_cap`); the heap cap at 8 GiB
  (`past_its_cpu_time_or_its_heap…`: the 2,000,000-pixel border drawn, no warning; the hang still
  died, in under two seconds); the worker's stack at 128 KiB (`the_deepest_label…`: left out,
  "took too much memory", the overflow's abort); `PREPARE_CPU` 0 (`what_an_estimate_refused…`:
  left out, "took too long"); `MAX_WORKERS` 0 (`each_drawing_printed_from_a_directory…`: no label
  drawn).
- **Within the caps, measured in the tests' debug build:** 10,000 elements clipped by a 1,000-point
  polygon lay out and record in some 40 ms of the viewer's wait; the whole SVG integration suite,
  33 tests with the hang, two bombs and 1,000 nested grids, runs in 2 s in parallel.
- **Not done here, for task 6:** the ADR's measurements on a release build, usage and
  architecture, CHANGELOG.

## Codex's review of task 5

Three findings, all fixed:

- **A worker that died between requests stayed a zombie**, holding its place among the six until
  the drawing's next render or its drop: the reader read only when an answer was awaited, so an
  end between requests went unseen. The reader now takes each frame's first byte as it comes
  (`first_byte`, retried on `Interrupted`: a resize's signal mustn't pass for an end), and the
  rest only when asked and within its length; at the end of the worker's answers it reaps it and
  gives its place up itself, through the state it shares with the viewer's `Link` (`Shared`:
  the process, the place, why it is gone). `a_worker_that_ends_between_requests_is_reaped_and_
  gives_up_its_place` (a fake that answers once and exits: counted among none and reaped within
  the wait, then "exit 0" when asked again) failed with the reader waiting to be asked first,
  as before: still counted after 5 s.
- **Labels painted before the worker went stayed on the page** while the warning said all were
  left out, and later renders drew none: `draw_labels` composited each as it came. All are
  painted first and drawn only if every one came (each part's pixels being in the page's estimate,
  so all held at once are too). `a_worker_gone_on_one_label_leaves_out_those_painted_before_it`
  (a blue label, then a 1,400,000-pixel dotted border) failed with the composite as it was: blue
  at the first label.
- **`--worker` repeated a bad name raw on stderr**, past `printable`: the name was in `serve`'s
  error. The error names no name now ("no such worker in this build").
  `a_bad_worker_name_is_refused_without_repeating_it` failed with the name back in it: the escape
  in stderr.

## Task 6: the docs

- ADR-0025 written from the design's measurements (the throwaway crates' numbers: what macOS
  enforces, spawn, fonts, the pipe) and the release build's, measured as its Evidence says, with
  the fixtures in the session's scratchpad: `styles 1024 3072` opens in 8.0 s, the worker's share
  0.32 s of CPU and 95 MB (usvg's stylesheet parse is the rest: usage's Known issues); `font 50
  400` in 0.10 s; one label costs 30–90 ms over none; the hang and the two bombs die in 0.20,
  0.27 and 0.30 s at the heap cap, the polygons draw in 0.19 s and the shared wavy line in 1.23 s
  (the worker 1.15 s of CPU); no worker is left after a kill, and one whose `clv` is killed goes
  within three seconds. `/usr/bin/time -l`'s maximum resident set covers a reaped child, so it
  reports the worker's peak for the bombs (about 1 GiB); the worker's own numbers were sampled
  from `ps`.
- ADR-0024 stays Accepted, its status line naming what ADR-0025 supersedes (the estimate, the
  thread); its measure tool stays, as the stage limits' evidence.
- Usage gains `## Known issues`, which README's Plans links instead of these notes.
- **Left for the owner:** Done's manual checks in kitty (the tldraw, draw.io and Mermaid
  drawings at over 6,000 pixels and zoomed; the `svg`-only build's warning; a worker made to panic
  by hand showing the warning and nothing else).

## Codex's review of the branch

- **A quadratic walk for the labels' stylesheets:** each XHTML `<style>` was asked whether a
  `<foreignObject>` is among its ancestors, so 100,000 `<style>`s nested under a `<metadata>`
  (which the drawing's scan skips by name, so its depth doesn't refuse them) were some 5 billion
  visits in the viewer, while opening, before any worker or deadline. `label_styles` walks once,
  knowing when it is inside a `<foreignObject>`. `label_styles_are_found_in_one_walk` (100,000
  nested, within 10 s) failed with the old walk back: 11.3 s in the debug build.
- **The musl release builds can't link fontconfig** (the owner's call, not fixed here). Blitz's
  `system-fonts` is parley's `system`, which is fontique's, which on Linux is
  `yeslogic-fontconfig-sys` 6.0.1, whose build script runs `pkg_config::find_library("fontconfig")
  .unwrap()` unless its `dlopen` feature is on or `RUST_FONTCONFIG_DLOPEN` is set in the
  environment. `release.yml` builds `x86_64-unknown-linux-musl` on `ubuntu-latest` and
  `aarch64-unknown-linux-musl` on `ubuntu-24.04-arm`, with `musl-tools` and nothing for pkg-config,
  so pkg-config refuses to cross-compile and both Linux artifacts fail at that build script.
  Reproduced here with `cargo check --target x86_64-unknown-linux-musl -p yeslogic-fontconfig-sys`
  (the script panics with "pkg-config has not been configured to support cross-compilation"), and
  seen passing that script with `RUST_FONTCONFIG_DLOPEN=1`. The options, with what a Linux user
  of the static binary would see:
  1. **`RUST_FONTCONFIG_DLOPEN=1` in `release.yml`'s musl steps**, or fontique's
     `fontconfig-dlopen` feature in `Cargo.toml` for every build: the builds pass; at run time a
     static musl binary can't `dlopen`, fontique's `SystemFonts::new` sees the library missing and
     holds no fonts (no panic: `Self::default()`). Labels draw, their boxes and pictures and any
     font the drawing embeds (tldraw), but text in the system's fonts draws nothing, and draw.io's
     fallback text is cut all the same: worse than today's fallback for draw.io and Mermaid. Same
     binary size. The feature in `Cargo.toml` also spares every Linux developer the fontconfig
     headers at build time (a dynamic build then `dlopen`s fontconfig where it is installed).
  2. **`svg-html` off in the musl release builds** (`--no-default-features --features
     pdf,image,svg,zip,tar,word,sheet` there): Linux users keep today's drawings, the fallbacks
     shown and a warning counting the labels not drawn; the binary some 11 MB smaller. macOS
     keeps the labels. The simplest honest release.
  3. **fontconfig built static for musl** in CI (fontconfig, freetype, expat from source or an
     Alpine toolchain): labels with the system's fonts where `/etc/fonts` is configured; heavy,
     fragile CI, and fontconfig's own files still needed at run time.
  4. **Linux builds against glibc** (`x86_64-unknown-linux-gnu`) with `dlopen`: fontconfig found at
     run time on nearly every desktop; but one binary no longer runs on any distribution, the
     reason the release is musl.
  5. **Fonts without fontconfig:** fontique has no directory scanner on Linux, but fontdb (resvg's,
     already loading the system's fonts in `svg.rs` through its own `fonts.conf` parser) does, and
     a face's `Source::SharedFile` holds the mapped file as an `Arc<dyn AsRef<[u8]>>`, which is a
     peniko `Blob`: the worker could register fontdb's faces into fontique's collection
     (`Collection::register_fonts`, no copy) and set the generic families as `svg.rs` does for
     resvg. Labels and the drawing's own text would then use the same fonts on every system, with
     no fontconfig at build or run time, and Blitz's `system-fonts` could go. A change of its own
     (fontdb named outside `svg.rs`, an ADR), the proper fix.
  Recommended now: 2 for the coming release, with the `fontconfig-dlopen` feature from 1 so that
  no build needs fontconfig's headers; 5 as the follow-up, in README's Plans.

## Label fonts through fontdb (the owner's decision on the musl builds)

- **No `unsafe`, so no fontdb mapping:** fontdb's `make_shared_face_data`, which would have
  given the mapped file as an `Arc`, is an `unsafe fn`; its `Source::File` is what
  `load_system_fonts` leaves (the `memmap-fonts` feature maps on each access, inside fontdb).
  fontique's own `SourceCache::get` maps a path (unsafe inside fontique) and gives a `Blob`,
  which `register_fonts` takes: the worker maps every font file once and reads none into its
  heap (some 700 MB of fonts on this machine).
- **`html.rs` names no resvg library** after all: `svg.rs` hands it paths and bytes (`FontFile`)
  and generic family names (`Generic`), so CONTRIBUTING's naming rule and its test stay as they
  were; the first wording of its comment did name fontdb, and the naming test caught it.
- **Fallbacks were the surprise:** with the files registered and the generics set, named Latin
  families rendered identically, but CJK text vanished and the generic rows differed, since a
  collection built from files has no script fallback. `FALLBACKS` (a script to a list of the
  usual families, the ones present registered) and `GenericFamily::Emoji` restored all sixteen
  Latin lines pixel for pixel and the CJK, emoji, Greek and Cyrillic lines, in fonts from the
  list: Hiragino Sans for Han here, where CoreText cascaded to PingFang. The comparison: two
  drawings of sixteen and three labelled lines, rendered with the binary before and after,
  decoded and compared line by line (the script is in the session's scratchpad).
- **Measured:** fontdb's load and the registration 18–23 ms against CoreText's 33–39 ms in a
  throwaway crate; `cargo check --target x86_64-unknown-linux-musl --no-default-features
  --features svg-html` passes (the std for musl downloaded at the third try; a full musl build
  needs the linker CI installs). `cargo tree -i yeslogic-fontconfig-sys` names nothing for the
  defaults, for `svg-html` alone and for the musl target; the lock loses yeslogic-fontconfig-sys,
  dlib, libloading and objc2-core-text.
- **Tests seen failing:** `a_labels_text_in_a_generic_family_is_drawn_in_the_systems_fonts`
  (skipped with a warning where the drawing's own `<text>` draws nothing) with the registration
  left out; `the_labels_fonts_come_without_fontconfig` with `system-fonts` back in Blitz's
  features, which puts fontconfig's bindings back in the lock.
- **On the release build, after:** a drawing with one label opens in 0.06 s warm (0.07 before),
  the worker's resident set 12 MB more for the font files it maps; the styles drawing in 8.2 s
  with the worker at 96 MB, the font drawing in 0.08 s. The gates, each backend alone included,
  all pass.

## Labels in `display: table` (the owner's decision)

- **The cause**, found for the upstream report: blitz-dom's `collect_table_cells` drops a table's
  children that aren't table parts (text returns at once; flow, flow-root, flex and grid children
  fall into an arm commented "probably a table caption"), so no anonymous row or cell exists where
  CSS 2.1 §17.2.1 makes one. The repro (a red square and text in a `display: table` box, three
  controls) renders the same on the pinned 0.3.0-beta.4 and on git main 4647f4a2: the first box
  empty, the controls drawn. The repro, PNGs and an issue draft are in the session's scratchpad,
  for the owner to file.
- **The workaround** is the CSS structure itself: once the styles resolve, each run of such
  children, in each `display: table` box, is moved into a `<div>` styled `table-row` holding a
  `<div>` styled `table-cell` (`anonymous_cells`, blitz-dom's `DocumentMutator`), decided by
  Stylo's computed `display` (a stylesheet's `display: table` counts too). Spaces between parts
  make no box and go with a run under way; table parts, `display: contents` and absolutely
  positioned children stay where they are. No Chrome reference was rendered: the equivalence is
  by the spec's own structure.
- **The canary** `blitz_still_drops_inline_content_in_a_table` renders `tests/data/blitz-table.html`
  through Blitz without the workaround (no fonts: the fixture's boxes are red squares) and asserts
  the first band empty and the controls drawn; then with it, the first band drawn. It fails with
  "Blitz now draws inline content in display: table: remove the workaround…" the day Blitz does.
- **Other canaries weighed:** the hang test already fails the day Blitz's decoration loop is fixed
  (said in CONTRIBUTING's checklist); the paint order, clipping and selector differences are clv's
  own choices, not Blitz bugs, so nothing to watch for; Blitz's panic on a relative URL with a
  `data:` base is avoided by `base_url`, harmless either way.
- **Tests seen failing** with `anonymous_cells` not called: `inline_content_in_a_display_table_
  label_is_drawn` (no blue), and the canary's last assertion with it made a no-op.
- **Upstream already tracks it**, the lead found: Blitz's issue #1430 (a block child of a table, the
  same CSS 2.1 fixup), PR #781 (anonymous cells for non-table children, blitz 0.3.0-beta.2), #783
  and #786 on top of it, and #1370 (Taffy's table layout in place of the grid flattening, on
  main). The issue text went to the owner as a comment for #1430 instead, with the inline case and
  the Mermaid impact; the canary's message and CONTRIBUTING's checklist name them. The repro was
  run on each PR's branch too (`refs/pull/N/head`, each with the anyrender its blitz-paint wants):
  #781 draws the inline box but not the text (a text node directly in the table is still
  dropped), #783 and #1370 draw both, as the controls are drawn; the pinned beta.4 and main draw
  neither. So the canary stays until #783's anonymous boxes or #1370's table layout land.
- **A probe that misled for a while:** the box-only probes through `clv` seemed blank by their
  output's size (5 KB), which was the PNG of a box with no text; decoded, they held the box. The
  real finding of that round was the detaching: moved nodes keep the construction flags Blitz's
  table code cleared, so a block inside a moved run (Mermaid's `<p>`) stayed unbuilt until each
  node was taken out of the document and put back.
- **A flaky test of mine, fixed on the way:** `a_picture_drawn_as_a_tile_is_drawn_again_for_a_page`
  named its temporary file by the same keys as `a_picture_is_drawn_once_and_only_those_a_page_shows`
  (`clview-picture-<pid>-a-b.png`), so the two, in parallel, wrote and read one file and one of
  them sometimes found it half written. It has a key of its own now.

## Codex's review of the workaround (the last round on it)

- **Cells and the content between them share one row.** The first version gave each run of
  non-cell content a row of its own and left the cells where they were, so a red cell, a blue
  inline box and a green cell came out as a row of the two cells and a second row for the box (the
  box at y = 23 with the workaround, nowhere without it). Now, in a table or a row group, each run
  of cells and content shares one anonymous row, and in a row (its own or an anonymous one) each
  run of content gets one anonymous cell: CSS 2.1 §17.2.1's two fixups, as `rows_in` and
  `cells_in`, planned from Stylo's computed `display` before anything moves. Test:
  `cells_and_the_content_between_them_share_a_row`, the three side by side on one line.
- **The moved nodes keep the rules that matched them.** Blitz 0.3.0-beta.4 makes anonymous boxes
  only for layout (`NodeData::AnonymousBlock`, styled as Servo's anonymous-box pseudo) and never as
  table parts; nothing public creates one. So the wrappers stay elements, named `clv-row` and
  `clv-cell` so that no rule on a name, a class or an id reaches them, their one declaration set
  with `set_style_property`; a rule on `*` does reach them, said in usage's known differences. The
  tree is edited directly rather than through the `DocumentMutator`, which sets a restyle of the
  box's subtree and would have matched the moved nodes anew in their new place: Stylo visits only
  the unstyled wrappers (their ancestors marked dirty), and recascades what they hold from them, so
  a cell third among the table's children keeps `#t > :nth-child(3)`'s blue. Test:
  `wrapping_leaves_the_tables_rules_as_they_matched`, which also checks a rule on `div` doesn't
  reach the wrappers.
- **Linear.** Each box's children are listed anew once (`realized`), and each node's `parent` set
  once; the mutator's `remove_node` per node scanned the box's children each time (and
  `append_children` again). Measured on 16,000 runs of three among 64,000 children, the
  workaround alone: 2.8 s before, under the 0.1 s the whole test takes now. Test:
  `anonymous_cells_cost_their_elements`, bounded at 1 s on the function alone (a bound on the whole
  run, as the earlier cost tests have, would have let the old version through: Blitz's layout of
  the 32,000 rows is most of it either way).
- **How the wrappers get laid out.** Blitz singles out boxes to construct anew by construction
  damage bits that are its own (`pub(crate)`), not in `RestyleDamage::all()`; without them the
  table kept its old layout children and dropped the wrappers. So `anonymous_cells` turns
  incremental layout off (`set_incremental_layout(false)`): the document is laid out anew whole at
  the next resolve, once, and `recorded` resolves again only when something moved (it returns
  whether anything did), which is also what a label without tables costs now: nothing.
- **Seen failing** against the committed version, with the three tests appended to it: the row
  test (the box on a second line), the rules test (the cell no longer blue) and the cost test
  (2.5 s). A first version of the green predicate asked for more green than CSS's `green`
  (0, 128, 0) has, and failed on both; fixed before the comparison.
- **Two Blitz limits met on the way**, neither this round's: parley's inline layout holds at most
  65,535 styles in one paragraph (`too many styles`, a panic) and blitz-dom's table layout counts
  rows in a `u16` (a debug build panics past 65,535 rows; a release build wraps). Either kills the
  worker, and the drawing's labels are left out with a warning, as any panic in it is.
- **The Mermaid drawing** renders byte-for-byte as it did after the first workaround: its edge
  labels are one `<span>` in a `display: table` box each, one row and one cell either way.
