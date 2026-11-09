# Spec-0004: Draw an SVG's HTML labels

- **Status:** Ready
- **Date:** 2026-11-09
- **Related:** [ADR-0002](../adr/0002-render-svg-with-resvg.md) (resvg draws the drawing);
  [ADR-0003](../adr/0003-bound-what-a-drawing-costs-before-usvg-and-resvg-spend-it.md) (what a
  drawing may cost, bounded before it is spent, which the labels join);
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
system, at the scale it is shown, placed by the drawing's transforms, and laid over what resvg
draws; the fallbacks a browser skips are skipped. It is a new feature, `svg-html`, on by default:
the SVG backend still builds without it. What the labels cost is bounded before it is spent, as
ADR-0003 bounds the rest of a drawing, and a label left out (past a limit, or in a build without
the feature) is counted in a warning. It makes finished code of the spike on the branch
`spike/blitz-foreignobject` (its `spike/NOTES.md` has the evidence), with its tests, an ADR, and the
docs.

## Requirements

**Which labels, and where**

- Each `<foreignObject>` a browser would draw is drawn: not under `display="none"`, and in a
  `<switch>` only its first element child, `requiredFeatures` taken as met. What follows that child
  in the `<switch>` is removed before usvg parses, in a build that draws HTML only, so draw.io's
  "[Not supported by viewer]" and "Text is not SVG - cannot display" are not drawn.
- Placed as a browser places it: the root's `viewBox` and `preserveAspectRatio`, its ancestors'
  `transform`s, a nested `<svg>`'s `x`, `y` and `viewBox`, and its own `x`, `y`, `width` and
  `height` in user units or in percent of the nearest viewport (newer draw.io makes each label
  `100%` of the drawing). `overflow: visible` draws past its box. It takes its ancestors' `opacity`,
  multiplied together.
- Laid out at the scale it is shown, so it stays sharp however far it is zoomed, and painted only
  where it lands on the page: no window size or zoom loses a label, and a label's pixels are never
  more than the page's. (The spike painted whole labels first, and lost every label of a newer
  draw.io drawing in a window past some 4,100 pixels wide.)

**How it looks**

- Its XHTML takes the drawing's `<style>`s, scoped as in the drawing: copies of its ancestors'
  ids, classes and styles around it, so a rule such as Mermaid's `#id .nodeLabel` still matches.
- `<font face>`, `color` and `size` act as a browser takes them, as hints. Each value is quoted or
  parsed, never pasted, so a drawing can't write rules of its own.
- Text is drawn with the system's fonts and those the drawing embeds as `data:` URLs (WOFF, WOFF2,
  TTF, OTF, as tldraw embeds its own). The system's fonts are looked for once a run.
- Pictures in a label, `data:` URLs of a PNG, JPEG, GIF or WebP, are drawn, each at most 64 MiB
  decoded, as a drawing's own pictures are. An SVG in a label is left out, so nothing reaches usvg
  around ADR-0003's bounds.

**What it reads**

- Nothing but `data:` URLs: no network, no files. A link a label doesn't follow is counted in the
  warning a drawing already gives for its links.

**What it costs**

- A drawing's labels cost about one document, not one each: their styles parsed and their fonts
  loaded once. The spike's 1,024 labels under 3 MiB of styles took 111 s and 3.1 GB; they are drawn
  in seconds, none left out, and the 50 labels of a tldraw drawing sharing a 400 KB font all drawn.
- Bounded before it is spent, by limits named in the code and listed in usage's
  [Limits](../usage.md#limits): labels a drawing, their markup and text in all, and the pixels each
  paints, counted in the page's render estimate (its 1 GiB held at once).
- A label nested as deep as the drawing's limits allow (1024 levels) is drawn or left out, never
  overflows a render thread's stack.
- Past a limit, or where Blitz fails or panics on a label, that label is left out with a warning,
  and the rest drawn.

**Builds**

- `svg-html` needs `svg`, and is among the default features. A build with `svg` alone draws as clv
  does now, and warns how many labels it didn't draw: "N HTML labels (`<foreignObject>`) not drawn".
- `--help`'s SVG row and README's credit Blitz.
- Blitz and what it paints with are named only in `src/source/svg/html.rs`, its tests and
  `Cargo.toml`, as resvg is in its own, checked by a test.

**Docs**

- Usage's SVG drawings and Limits; architecture's backends table and How each opens;
  `CONTRIBUTING.md`'s library rule, its count of features and its gates (`svg` alone, `svg-html`
  alone); `CHANGELOG.md`; an ADR, why Blitz and how its cost is bounded, with what reproduces its
  evidence; and in usage, as known limits, Blitz's two found so far: inline content straight in a
  `display: table` element isn't drawn (5 of a Mermaid flowchart's 24 labels blank), and `<font>`'s
  attributes are read by clv, not Blitz.

## Out of scope

- A `<foreignObject>` reached through `<use>`: usvg copies the `<use>`, not the label. None of the
  exports seen do it; it is left out and counted in the warning.
- Clip paths and masks on a label's ancestors: a label is drawn unclipped. The exports seen don't
  clip their labels.
- Working around Blitz's `display: table` bug in clv: it is Blitz's to fix.
- HTML and Markdown documents through Blitz rather than wordcraft's importer: Blitz lays out one
  long page, and Spec-0003 makes pages the window's size. A question for README's Plans.
- What a label would fetch: web fonts by URL, stylesheets, `<iframe>`s; Blitz runs no scripts.
- Selecting or copying a label's text.
- Moving Blitz past `=0.3.0-beta.4`.

## Approach

**Files:**

- `src/source/svg.rs` — finds the labels and places them: the spike's `drawn_label`,
  `without_fallbacks`, `viewport`, `view_transform`, `font_rules` and `found_labels`, nested
  `<svg>`s and opacity added; the labels counted in `Scan` (their markup and text, before usvg
  parses) and in `Estimate` (the pixels they paint); the warning in a build without `svg-html`; the
  labels painted after `resvg::render`, as in the spike's `draw_labels`.
- `src/source/svg/html.rs` — the only file naming Blitz, anyrender, vello and kurbo: `Painter`, the
  net provider that answers `data:` URLs only, and the fonts, held in a process-wide `OnceLock`.
- `Cargo.toml` — the Blitz crates pinned `=0.3.0-beta.4` without their default features (which
  leaves out Blitz's own SVG, usvg without clv's bounds), with `woff` and `system-fonts`;
  `anyrender` 0.14, `anyrender_vello_cpu` 0.18, `kurbo` 0.13 (built already for Blitz) and
  `svgtypes` 0.16, each with its comment; `svg-html` among the defaults.
- `tests/it/resvg.rs` — the tests, and `nothing_but_svg_html_names_blitz` through
  `fixtures::named_only_in`.
- `.github/workflows/ci.yml` — `--features svg-html` alone among the backends built alone.
- `docs/adr/0023-…`, `docs/usage.md`, `docs/architecture.md`, `CHANGELOG.md`, `CONTRIBUTING.md`,
  `README.md`, and `CLAUDE.md`'s Layout (`svg/html.rs`).

**Patterns to follow:**

- Limits as named constants with their reason beside them, as ADR-0003's are in `svg.rs`.
- Warnings through `Notes`, as `left_out_text` gives them.
- The naming test as `nothing_but_this_backend_names_resvg` is.
- Fixtures made by the tests, as `svg(width, height, body)` in `tests/it/resvg.rs` makes them, and
  told by coloured boxes (`background`, `border`) that need no font, so a machine without fonts
  passes; text is looked at by hand.

**Decided:**

- **One Blitz document a drawing.** Each label in a box of its own, stacked so none overlap, inside
  the copies of its ancestors; laid out once, and painted once at scale 1 into a recorded anyrender
  `Scene`, kept with the usvg tree. Each render replays it for each label, through the label's
  transform (its scale too: the scene is vectors, so it stays sharp), clipped to the label's box
  (padded where its overflow is visible), into the part of the page it lands on. Measured against
  a document a label on the 1,024-label drawing and the 50-label tldraw one, the numbers in the
  ADR; if the replays cost more than the one document saves, a document a label under the spike's
  32 MiB budget, as the spike does.
- **Pictures** decode through the `image` crate clv already builds, which Blitz takes without
  default features, so with the formats clv turns on. The net provider reads a picture's size from
  its header, and refuses one past 64 MiB decoded, and SVG.
- **The stack:** a test lays out a label nested 1024 deep on a render thread. If it overflows,
  Blitz's work goes to a thread with a larger stack, or labels may nest less deep: whichever the
  test shows holds, said in the ADR.
- **Branch:** `spec-0004` from `main`, the spike's code ported to it; `spike/` stays on the spike's
  branch, its evidence carried into the ADR.
- **resvg still draws the drawing; Blitz only its labels.** The ADR says why each alternative
  was left: Blitz for the whole SVG (it hands an SVG to usvg with default options and paints
  usvg's tree, so it drops `<foreignObject>` too, and has none of ADR-0003's bounds); Gecko (not
  embeddable); Servo (a browser's weight for a drawing's labels); wordcraft's HTML importer (it
  lays out pages, not CSS boxes, no flex, and is another backend's library).
- **On by default**, though it adds 11.4 MB (+21%) and 75 crates to the binary, and Blitz is a
  beta: a drawing without labels never starts it.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- 🟡 **One document a drawing, or one a label** — settled by the measurement in Decided.
- 🟡 **How deep a label may nest** — settled by the stack test in Decided.
- 🟡 **The limit on a drawing's label text** — the spike's 3.6 MB in one label took 1.3 s and
  515 MB; the number is measured, as ADR-0012 measured documents', and set in the ADR.

## Tasks

One checkbox each: a session's work, and a commit that builds and passes on its own.

- [ ] **Labels drawn** — the spike ported, with its fixes (percentages, `requiredFeatures`,
  `<font>`, painting only what lands on the page) and nested `<svg>`s, opacity and pictures; the
  warning without the feature; the naming test; `Cargo.toml` and CI; the ADR's choice of Blitz;
  `CONTRIBUTING.md`'s rule. *Files:* `src/source/svg.rs`, `src/source/svg/html.rs`, `Cargo.toml`,
  `tests/it/resvg.rs`, `.github/workflows/ci.yml`, `docs/adr/0023-…`, `CONTRIBUTING.md`. *Verify:*
  `cargo test -j 2 --test it resvg`, and the gates with `--no-default-features --features svg`
  and `--features svg-html`.
- [ ] **Bounded, and one document a drawing** — the one document and its replays, measured; labels
  counted in `Scan` and `Estimate`; the stack test; hostile drawings (a label's text, styles times
  labels, nesting, a picture past 64 MiB) as tests, their numbers in the ADR; usage's Limits.
  *Files:* `src/source/svg.rs`, `src/source/svg/html.rs`, `tests/it/resvg.rs`, `docs/adr/0023-…`,
  `docs/usage.md`. *Verify:* `cargo test -j 2 --test it resvg`, and the ADR's commands under
  `/usr/bin/time -l` on a release build.
- [ ] **The docs** — usage's SVG drawings and known limits, architecture, CHANGELOG, README's
  credits, `CLAUDE.md`. *Files:* `docs/usage.md`, `docs/architecture.md`, `CHANGELOG.md`,
  `README.md`, `CLAUDE.md`. *Verify:* `cargo test -j 2 --test docs`, and the manual checks in Done.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass, with
  `--no-default-features --features svg` and `--features svg-html` alone too.
- [ ] Tests added, each seen failing without the change: name them.
- [ ] **Cost:** the spike's hostile drawings and the 1,024-label one measured under
  `/usr/bin/time -l` on a release build, against the numbers in Requirements, in the ADR.
- [ ] Manual, in kitty with a window over 6,000 pixels wide, and again zoomed in with `+`: a tldraw
  drawing shows every label in its own font; a draw.io 21 drawing (labels `100%` wide, set in
  `<font face>`) shows them as Chrome does, with no "Text is not SVG" line; a Mermaid flowchart
  shows its labels but those in `display: table`. Built with `--no-default-features --features svg`,
  the same tldraw drawing says how many labels it didn't draw.
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- 2026-11-09 — Ready (spec-new).
