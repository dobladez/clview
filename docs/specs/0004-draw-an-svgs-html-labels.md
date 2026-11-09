# Spec-0004: Draw an SVG's HTML labels

- **Status:** Ready
- **Date:** 2026-11-09
- **Related:** [ADR-0002](../adr/0002-render-svg-with-resvg.md) (resvg draws the drawing);
  [ADR-0003](../adr/0003-bound-what-a-drawing-costs-before-usvg-and-resvg-spend-it.md) (what a
  drawing may cost, bounded before it is spent, which the labels join);
  [ADR-0012](../adr/0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md) (HTML's cost
  bounded by its marks and text, measured the same way);
  [Spec-0003](0003-lay-markdown-and-html-out-to-the-window.md) (why HTML documents through Blitz are
  another question); [SVG drawings](../usage.md#svg-drawings), [Limits](../usage.md#limits),
  [How each opens](../architecture.md#how-each-opens-and-what-it-keeps). It delivers no Plans
  bullet, and adds one: HTML documents through Blitz, weighed.

## Summary

tldraw, draw.io and Mermaid put a drawing's labels in `<foreignObject>`: HTML inside the SVG.
resvg draws no HTML, so clv shows such a drawing without its text (tldraw, Mermaid) or with
draw.io's fallback in its place ("[Not supported by viewer]", "Text is not SVG - cannot display").
This spec draws them: each `<foreignObject>` a browser would draw is laid out and painted by
[Blitz](https://github.com/DioxusLabs/blitz), an HTML and CSS engine in Rust on Servo's style
system, at the scale it is shown, placed by the drawing's transforms, and laid **over** what resvg
draws; the fallbacks a browser skips are skipped. It is a new feature, `svg-html`, on by default
once its bounds are in: the SVG backend still builds without it. What the labels cost is bounded
before it is spent, stage by stage, as ADR-0003 bounds the rest of a drawing, and a label left out
(past a limit, or in a build without the feature) is counted in a warning. It makes finished code
of the spike on the branch `spike/blitz-foreignobject` (its `spike/NOTES.md` has the evidence so
far); its first task measures what the spike left open, and sets the limits below by it.

## Requirements

**Which labels**

- One rule says which `<foreignObject>`s are drawn, and the same rule serves finding them, removing
  fallbacks and counting what is left out. A label is drawn when:
  - none of its ancestors is `defs`, `symbol`, `clipPath`, `mask`, `pattern` or `marker` (a label
    reached through `<use>` is out of scope);
  - neither it nor an ancestor has `display: none` or `visibility: hidden` by attribute or
    `style` (by a stylesheet rule it isn't seen: a known difference, in usage);
  - in a `<switch>`, it is the first child that passes `systemLanguage` and `requiredExtensions` as
    usvg evaluates them, `requiredFeatures` taken as met; within any ancestor `<switch>`, it is in
    the child taken.
- What follows the child taken in a `<switch>` is removed before usvg parses, in a build that
  draws HTML only, so draw.io's "[Not supported by viewer]" and "Text is not SVG - cannot display"
  are not drawn. A label that isn't drawn by the rule costs no quota and is in no warning.

**Where**

- A stack of viewports places it, as SVG's coordinate rules do: the root's, at the size usvg
  resolves for it (`width`, `height`, `viewBox`, `preserveAspectRatio`), then each nested `<svg>`'s
  `x`, `y`, `width` and `height` (`100%` when left out), `viewBox` and `preserveAspectRatio`; and
  between them the ancestors' `transform`s, in document order. The root's own `transform` is
  ignored, as usvg ignores it. Its own `x`, `y`, `width` and `height` are in user units, in units
  (`px`, `mm`, `in`… at 96 pixels an inch), or in percent of the nearest viewport (newer draw.io
  makes each label `100%` of the drawing). A nested `<svg>`'s overflow clip is not applied, as clip
  paths are not.
- What it draws past its box, under `overflow: visible`, is drawn: what is recorded of it reaches as
  far as its layout says it overflows, up to a named limit, past which it is cut, said in usage.
- Laid out at the scale it is shown, so it stays sharp however far it is zoomed, and painted only
  where it lands on the page: no window size or zoom loses a label, and a label's pixels are never
  more than the page's. (The spike painted whole labels first, and lost every label of a newer
  draw.io drawing in a window past some 4,100 pixels wide.)
- **Drawn over the whole drawing**, after resvg, with its ancestors' `opacity` multiplied together
  and applied to the label as a whole. A browser composites a label among the shapes in their order
  and inside their groups' opacity: where a shape overlaps a label, or a group with opacity holds
  both, clv's differs. A known difference, in usage.

**How it looks**

- Its markup is written out from the parsed tree, never copied from the file's text: namespaces
  bound as in the drawing, entities expanded, text and attributes escaped.
- It takes the drawing's `<style>`s, inside copies of its context: the `<foreignObject>` itself,
  its box made the containing block of what it holds, and its ancestors, each with its id, class and
  style, and their font and colour presentation attributes (`font-family`, `font-size`,
  `font-weight`, `font-style`, `color`) as style. So a rule such as Mermaid's `#id .nodeLabel`, or one
  on the `<foreignObject>`'s class, still matches. Selectors on SVG element names, namespaces,
  siblings or structural pseudo-classes may not: a known difference, in usage.
- `<font face>`, `color` and `size` are hints, under any rule of the drawing's: rules of no
  specificity (`:where(…)`), before the drawing's styles. `face` a list of names; `color` a CSS
  colour; `size` 1 to 7, or `+n`/`-n` from 3, clamped to 1–7. A value that doesn't parse is ignored;
  each is quoted or parsed, never pasted, so a drawing can't write rules of its own.
- Text is drawn with the system's fonts, looked for once a run, and those the drawing embeds as
  `data:` URLs (WOFF, WOFF2, TTF, OTF, as tldraw embeds its own), which stay the drawing's own.
- Pictures in a label, `data:` URLs of a PNG, JPEG, GIF or WebP, told by their bytes rather than
  their MIME type, are drawn. `svg-html` brings their decoders itself, whether or not the image
  backend is built. An SVG in a label is left out, so nothing reaches usvg around ADR-0003's bounds;
  so are a stylesheet or a document by a `data:` URL (`@import`, `<iframe>`, `<link>`).

**What it reads**

- Nothing but `data:` URLs: no network, no files. A link a label doesn't follow is counted in the
  warning a drawing already gives for its links.

**What it costs**

Every stage is bounded before it is spent, by limits named in the code and listed in usage's
[Limits](../usage.md#limits). The numbers below are where task 1 starts; it measures each, and the
ADR records the numbers set and how to measure them again.

| Stage | What is counted | Limit, to start | Where | Past it |
|---|---|---|---|---|
| The XML | the file, svgz, entities, as now | ADR-0003's | before **every** parse, the fallbacks' included | refused |
| Finding labels | labels drawn; their elements and depth, in the drawing's | 1,024; within the million elements and 1024 levels | while the drawing is scanned | the rest left out, with a warning |
| Their markup | markup and wrappers written out; their text | 8 MiB; 1 MiB of text | while written out | the rest left out, with a warning |
| Their nesting | levels from a `<foreignObject>` down | what task 1's stack test shows holds, at least 64 | while written out | that label left out |
| Styles | rules times the labels' elements, as ADR-0003 counts them | 50 million, with the drawing's | before Blitz | all labels left out, with a warning |
| Fonts | each `data:` font, its size unpacked read from its header before it is unpacked; in all | 32 MiB; 64 MiB | in the net provider | that font refused |
| Pictures | each decoded, from its header; in all | 64 MiB; 256 MiB | in the net provider | that picture refused |
| Requests | `data:` URLs answered | 1,024 | in the net provider | refused |
| Layout | a label's laid-out size | 16,384 CSS pixels a side | after layout, before recording | that label left out |
| Recording | commands recorded | 1 million | after recording, before any replay | all labels left out, with a warning |
| Painting | pixels painted, and commands replayed, for all labels | in the page's estimate: 1 GiB held, 4,000 million pixels | before the page renders | the page fails with why, as now |

- A drawing's labels cost about one document, not one each: their styles parsed and their fonts
  loaded once (if task 1 shows one document holds; else one document a label, under the markup
  budget, its omissions said).
- If Blitz fails or panics while the labels are prepared, all of that drawing's labels are left
  out, with a warning, and the drawing drawn; if one label's painting fails, only it is.
- A label's nesting is bounded before Blitz runs, at a depth that never overflows a render
  thread's stack.

**Builds**

- `svg-html` needs `svg`. A build with `svg` alone draws as clv does now, and warns how many labels
  it didn't draw: "N HTML labels (`<foreignObject>`) not drawn".
- `--help`'s SVG row and README's credit Blitz, the name given by `src/source/svg/html.rs` through
  the backend's `credits`.
- Blitz and what it paints with (the names `blitz`, `anyrender`, `kurbo`, `stylo`, `parley`,
  `taffy`, `wuff`) are named only in `src/source/svg/html.rs`, `tests/it/resvg.rs` and
  `Cargo.toml`, checked by a test. Not `vello`, which pdfcraft's pin names.
- A test fails when the Blitz crates or anyrender move from the versions the limits were measured
  with, as `the_render_estimate_is_for_the_resvg_this_builds` does for resvg.

**Docs**

- Usage's SVG drawings, its Limits and the known differences above, and Blitz's found so far:
  inline content straight in a `display: table` element isn't drawn (5 of a Mermaid flowchart's
  24 labels blank). Architecture's backends table and How each opens. `CONTRIBUTING.md`'s library
  rule, its count of features and its gates. `CHANGELOG.md`. An ADR,
  [ADR-0024](../adr/README.md) (the next free number), why Blitz and how its cost is bounded, with
  what reproduces its evidence.

## Out of scope

- A `<foreignObject>` reached through `<use>`: usvg copies the `<use>`, not the label. None of the
  exports seen do it; it is left out and counted in the warning.
- Clip paths and masks on a label's ancestors, and a nested `<svg>`'s clip: a label is drawn
  unclipped. The exports seen don't clip their labels.
- A label among the shapes in their paint order, or inside their groups' opacity: it is drawn over
  the drawing.
- A stylesheet's `display: none` on a label's ancestor.
- Working around Blitz's `display: table` bug in clv: it is Blitz's to fix.
- HTML and Markdown documents through Blitz rather than wordcraft's importer: Blitz lays out one
  long page, and Spec-0003 makes pages the window's size. In README's Plans.
- What a label would fetch: web fonts by URL, stylesheets, `<iframe>`s; Blitz runs no scripts.
- Selecting or copying a label's text.
- Moving Blitz past `=0.3.0-beta.4`.

## Approach

**Files:**

- `src/source/svg.rs` — finds the labels and places them, from the one parsed document the
  fallbacks are removed from, after ADR-0003's bounds on the XML: the spike's `drawn_label`,
  `without_fallbacks`, `viewport`, `view_transform`, `font_rules` and `found_labels`, made to the
  rules above (the eligibility rule, the viewport stack, markup written from the tree, the
  `<foreignObject>` in its context, hints of no specificity); the labels counted in `Scan` and in
  `Estimate`; the warning without `svg-html`; the labels painted after `resvg::render`. It names no
  Blitz crate, its comments included.
- `src/source/svg/html.rs` — the only file naming Blitz and what it paints with: `Painter`, the
  net provider (`data:` fonts and pictures only, each sized from its header before it is handed on),
  the system's fonts in a process-wide `OnceLock`, the drawing's own fonts with its document, and
  `CREDIT`.
- `Cargo.toml` — the Blitz crates pinned `=0.3.0-beta.4` without their default features (which
  leaves out Blitz's own SVG, usvg without clv's bounds), with `woff` and `system-fonts`;
  `anyrender` 0.14, `anyrender_vello_cpu` 0.18, `kurbo` 0.13 (built already for Blitz) and
  `svgtypes` 0.16, each with its comment; `svg-html = ["svg", "dep:image", …]`, so its decoders
  come with it; among the defaults from task 3.
- `tests/it/resvg.rs` — the tests; `nothing_but_svg_html_names_blitz`; the version guard.
- `.github/workflows/ci.yml` — `--features svg-html` alone among the backends built alone, and
  `--no-default-features --features pdf,svg`, so the integration tests (gated on `pdf`) run the
  SVG backend without HTML too.
- `docs/adr/0024-…`, `docs/usage.md`, `docs/architecture.md`, `CHANGELOG.md`, `CONTRIBUTING.md`,
  `README.md`, and `CLAUDE.md`'s Layout (`svg/html.rs`).

**Patterns to follow:**

- Limits as named constants with their reason beside them, as ADR-0003's are in `svg.rs`.
- Warnings through `Notes`, as `left_out_text` gives them.
- The naming test as `nothing_but_this_backend_names_resvg` is; the version guard as
  `the_render_estimate_is_for_the_resvg_this_builds` is.
- Fixtures made by the tests, as `svg(width, height, body)` in `tests/it/resvg.rs` makes them, and
  told by coloured boxes (`background`, `border`), which need no font, so a machine without fonts
  passes. Text is looked at by hand: a font in `tests/data` would not be synthetic.

**Decided:**

- **resvg still draws the drawing; Blitz only its labels.** The ADR says why each alternative
  was left: Blitz for the whole SVG (it hands an SVG to usvg with default options and paints
  usvg's tree, so it drops `<foreignObject>` too, and has none of ADR-0003's bounds); Gecko (not
  embeddable); Servo (a browser's weight for a drawing's labels); wordcraft's HTML importer (it
  lays out pages, not CSS boxes, no flex, and is another backend's library).
- **The design task 1 tries first: one Blitz document a drawing.** Each label in a box of its
  own, stacked so none overlap, the document's viewport covering them all (Blitz paints nothing
  outside its viewport); laid out once, and each label painted once at scale 1 into a recorded
  anyrender `Scene` of its own (a fragment, its layers balanced), kept with the usvg tree. Each
  render replays a label's fragment through its transform (its scale too: the fragment is vectors,
  so it stays sharp) into the part of the page it lands on, so the work is each label's commands,
  not every label's. A recorded scene keeps its fonts and pictures (anyrender clones them in);
  task 1 checks that it keeps all Blitz records. If one document doesn't hold, one document a
  label, under the markup budget.
- **The stack:** render threads keep the stack `source/threads.rs` gives them. Task 1 lays out,
  paints and drops labels nested ever deeper on such a thread, in a child process so an overflow
  can't take the test runner down, and the nesting limit is set below the depth that holds, with a
  margin.
- **Panics** print through the viewer's hook, which makes them printable and keeps them off the
  pager (`src/view/mod.rs`'s `run`); `catch_unwind` around Blitz turns them into the warnings above.
- **Branch:** `spec-0004` from `main`, the spike's code ported to it; `spike/` stays on the spike's
  branch, its evidence carried into the ADR.
- **On by default**, though it adds 11.4 MB (+21%) and 75 crates to the binary, and Blitz is a
  beta: a drawing without labels never starts it. Default only from task 3, once every limit above
  is in.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- 🟡 **Whether one document holds** — task 1 settles it; the fallback is in Decided.
- 🟡 **The limits' numbers** — task 1 measures each stage's, starting from the table's.

## Tasks

One checkbox each: a session's work, and a commit that builds and passes on its own.

- [ ] **Measured** — on the spike's branch, the open design tried and measured: one document a
  drawing with a fragment a label, against a document a label; the stack test in a child process;
  each stage's hostile drawing (a label's text, styles times labels, deep nesting, a font declaring
  a huge size unpacked, many pictures, a 1-pixel background tile over the largest box, many
  commands). The fixtures made by a generator, synthetic: N labels sharing a 400 KB `@font-face`,
  1,024 under 3 MiB of styles. The ADR written, Proposed, with the numbers and commands; this
  spec's table set to the numbers, Decided to the design that held. *Files:* `docs/adr/0024-…`,
  `docs/adr/README.md`, this spec. *Verify:* the ADR's commands, under `/usr/bin/time -l` on a release
  build, give its numbers again.
- [ ] **Labels found, behind `svg-html`, off by default** — the dependencies and the feature; the
  XML bounded before every parse; the eligibility rule and the fallbacks removed; the viewport
  stack; markup written from the tree, in its context, and its budgets; the warning without the
  feature; the naming test and the version guard; CI. *Files:* `Cargo.toml`, `src/source/svg.rs`,
  `src/source/svg/html.rs` (a stub painter), `tests/it/resvg.rs`, `.github/workflows/ci.yml`,
  `CONTRIBUTING.md`. *Verify:* `cargo test -j 2 --test it resvg` with `--features svg-html` and with
  `--no-default-features --features pdf,svg`; tests for each eligibility case, a fallback after a
  `<foreignObject>` and after a `requiredFeatures` child, entities in a fallback refused before it is
  parsed, a root-declared XHTML prefix, an internal entity in a label.
- [ ] **Labels drawn, bounded, on by default** — `Painter`, the net provider and its budgets, the
  document, fragments and replays as task 1 decided; painting only what lands on the page; opacity;
  overflow; hints; pictures; the stage limits and their warnings; `Estimate`; the nesting limit;
  `svg-html` among the defaults. *Files:* `src/source/svg.rs`, `src/source/svg/html.rs`,
  `Cargo.toml`, `tests/it/resvg.rs`. *Verify:* `cargo test -j 2 --test it resvg`; boxes for: placed
  through nested `<svg>`s and a `preserveAspectRatio`, a percentage label, a child several boxes
  outside an `overflow: visible` label, a label in a window over 6,000 pixels wide, a group's
  opacity, a shape overlapping a label (drawn over it), a rule on the `<foreignObject>`'s class, an
  inherited `color`, labels of different sizes with percentage children, a `data:` PNG, each hint
  overridden by a rule and a malformed one; and each stage's limit, with its warning.
- [ ] **The docs** — usage's SVG drawings, Limits and known differences; architecture; CHANGELOG;
  README's credits; `CLAUDE.md`; the ADR Accepted. *Files:* `docs/usage.md`, `docs/architecture.md`,
  `CHANGELOG.md`, `README.md`, `CLAUDE.md`, `docs/adr/0024-…`. *Verify:* `cargo test -j 2 --test docs`,
  and the manual checks in Done.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass, with
  `--features svg-html` alone and `--no-default-features --features pdf,svg` too.
- [ ] Tests added, each seen failing without its change, the change reverted named beside it.
- [ ] **Cost**, on a release build under `/usr/bin/time -l`, on the machine the ADR names (an Apple
  M3 Max, as ADR-0003's): the 1,024 labels under 3 MiB of styles drawn, none left out, in at most
  3 s and 500 MB; 50 labels sharing a 400 KB font drawn in at most 1 s; each stage's hostile drawing
  within its limit, or left out with its warning.
- [ ] Manual, in kitty with a window over 6,000 pixels wide, and again zoomed in with `+`: a tldraw
  drawing shows every label in its own font; a draw.io 21 drawing (labels `100%` wide, set in
  `<font face>`) shows them as Chrome does, with no "Text is not SVG" line; a Mermaid flowchart
  shows its labels but those in `display: table`. Built with `--no-default-features --features svg`,
  the same tldraw drawing says how many labels it didn't draw.
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- 2026-11-09 — Ready (spec-new).
- 2026-11-09 — Codex's review: the limits a stage each, measured first (task 1); one eligibility
  rule; the XML bounded before every parse; the viewport stack; markup written from the tree, in
  its context; hints of no specificity; labels drawn over the drawing, opacity approximated; a
  shared failure leaves out all labels; decoders brought by `svg-html`; the feature default only
  once bounded; ADR-0024; the 32 MiB fallback's omissions and the 50-label fixture made honest.
