# ADR-0024: Draw an SVG's HTML labels with Blitz, one document a drawing, each stage bounded

- **Status:** Proposed
- **Date:** 2026-11-09

## Context

tldraw, draw.io and Mermaid put a drawing's labels in `<foreignObject>`: HTML inside the SVG.
resvg draws the drawing ([ADR-0002](0002-render-svg-with-resvg.md)) but no HTML, so clv showed
such a drawing without its text, or with draw.io's fallback ("[Not supported by viewer]") in its
place. [Spec-0004](../specs/0004-draw-an-svgs-html-labels.md) draws them with
[Blitz](https://github.com/DioxusLabs/blitz) 0.3.0-beta.4, an HTML and CSS engine in Rust (Stylo's
styles, Taffy's layout, Parley's text, painted through anyrender into vello_cpu). The spike on the
branch `spike/blitz-foreignobject` (its `spike/NOTES.md`) showed it draws them, and left two
questions: whether all of a drawing's labels can share one Blitz document, and what each stage
costs. A drawing is untrusted input, and its cost is bounded before it is spent
([ADR-0003](0003-bound-what-a-drawing-costs-before-usvg-and-resvg-spend-it.md)); Blitz bounds
little of it itself:

- **Fonts:** wuff, which unpacks a WOFF for Blitz, sizes a WOFF 1 table's output by the size its
  header claims, with no cap: a 1 MB `data:` font unpacks to 1 GiB. WOFF 2 it caps at 128 MiB
  and a compression ratio of 100.
- **Pictures** decode at their own size, through the `image` crate's default limits (512 MiB).
- **Styles, layout, painting:** no limits. Blitz culls what lies outside its viewport, and
  nothing else.
- **The stack:** styles, layout and painting recurse once a level of nesting.

## Decision

1. **resvg still draws the drawing; Blitz only its labels,** drawn over it.
2. **One Blitz document a drawing.** Each label is a box of its own (a slot) in it: the drawing's
   `<style>`s, then the label inside copies of its ancestors and of the `<foreignObject>`, the
   copy made the box, the containing block of what it holds. The slots are packed in rows about
   as wide as all of them would be square, 8 CSS pixels apart, each placed by
   `transform: translate()` rather than `left`/`top` (Blitz culls by a box that counts an
   element's position twice, so a slot placed by `left` and `top` would not cull its neighbours).
   It is laid out once; each label is painted once at scale 1, the viewport scrolled to its slot
   and sized to it, into an anyrender `Scene` of its own, kept with the usvg tree. Each render
   replays a label's scene through its transform into the part of the page it lands on, so it
   stays sharp at any zoom and costs that part's pixels. The document is dropped once recorded.
3. **Blitz runs on a thread of its own,** with an 8 MiB stack (as `source/later.rs`'s threads
   that open files have), for a drawing's build, layout and recording, joined before its labels
   are kept; the render threads keep their 2 MiB and only replay, which doesn't recurse. A thread
   that can't be spawned is a Blitz failure: all the labels left out, with a warning.
4. **Each stage bounded before it is spent,** by limits named in `src/source/svg.rs` and
   `src/source/svg/html.rs`, measured below:

| Stage | What is counted | Limit | Past it |
|---|---|---|---|
| Finding labels | labels drawn; the elements of all of them | 1,024; 100,000 | the rest left out, with a warning |
| Their markup | markup written out; its text | 8 MiB; 1 MiB | the rest left out, with a warning |
| Their nesting | levels from a `<foreignObject>` down | 64 | that label left out |
| Styles | rules times the labels' elements, with the drawing's | 50,000,000 (ADR-0003's) | all labels left out, with a warning |
| Fonts | each `data:` font's unpacked size, from its header; in all | 32 MiB; 64 MiB | that font refused |
| Pictures | each decoded, from its header; in all | 64 MiB; 256 MiB | that picture refused |
| Requests | `data:` URLs answered | 1,024 | refused |
| Layout | a label's laid-out size | 16,384 CSS pixels a side | that label left out |
| Overflow | what an `overflow: visible` label draws past its box | 4,096 CSS pixels each way | cut |
| Recording | commands recorded, all labels | 200,000 | all labels left out, with a warning |
| Painting | each label's part of the page, and its commands | in the page's estimate (ADR-0003's 1 GiB and 4,000 million pixels) | the page fails with why |

The painting estimate counts, for each label landing on the page, its part's pixels twice
(painted, then drawn over the page) and 2 KiB a command held; and as work, each command's box on
the page within that part, and the part's pixels.

## Evidence

Measured 2026-11-09 on an Apple M3 Max (14 cores, 36 GiB), macOS 26.6.2, Rust 1.95.0, with the
measuring tool [`0024-measure/`](0024-measure/src/main.rs) beside this ADR: a crate of its own, in
its own workspace, outside clv's build, with blitz-dom, blitz-html, blitz-paint and blitz-traits
0.3.0-beta.4, anyrender 0.14.0, anyrender_vello_cpu 0.18.0 (vello_cpu 0.3.0), stylo 0.22.0, taffy
0.15.1, parley 0.12.0, wuff 0.2.10 and image 0.25.10 (its `Cargo.lock`). `measure gen <kind> …`
writes each drawing, synthetic; `measure run <x.svg>` finds its labels as the spec says
(simplified: no `<switch>` or nested `<svg>`), lays them out in one document (or one a label,
`--each`), records each and replays it into the page at `--scale` (1 unless said), printing each
stage's time and counts. Built with `cargo build --release` in `docs/adr/0024-measure/`; each
line is `target/release/measure gen <kind> … > x.svg`, then
`/usr/bin/time -l target/release/measure run x.svg …`, its `real` time and maximum resident set.

**One document, against one a label.** Their pages, written with `--out page.png --scale 2`, are
the same pixel for pixel (`cmp`) for `labels 40`, for `visible 1024`, and for four real drawings
(not in the repository: clients' files) with their labels: tldraw's (27 labels, its font embedded as
a 400 KB WOFF 2, and 33 labels), a newer draw.io's (11 labels, each `100%` of the drawing) and a
Mermaid flowchart's (24 labels). The fonts and pictures a scene paints with are drawn from it after
the document is dropped.

| Drawing | One document | One a label |
|---|---|---|
| tldraw, 27 labels and its 400 KB font: built | 15 ms | 239 ms |
| tldraw, 33 labels: built | 12 ms | 292 ms |
| `font 50 400`: 50 labels sharing a 400 KB `@font-face`, a TrueType font made by the tool | 0.06 s, 32 MB | 0.96 s, 57 MB |
| `styles 1024 3072`: 1,024 labels under 3 MiB of styles | 2.16 s, 165 MB | stopped after 10 minutes |
| `styles 16 3072`: 16 of them | | 27.4 s, 150 MB: 1.7 s a label |

One document a label parses the styles and loads the font again for each label, as the spike did.

**Each stage,** one document:

| `measure gen …`, then `run` | What it is | Time, memory |
|---|---|---|
| `labels 1024` | 1,024 labels, a box and a word each | 0.18 s, 73 MB: 8,192 commands |
| `labels 10000` | 10,000 | 4.56 s, 494 MB, nearly all of it recording |
| `visible 1024` | 1,024 `overflow: visible` labels, each with a box three boxes past its own | 0.15 s, 77 MB; the boxes drawn |
| `text 1048576` | 1 MiB of text in one label | 0.41 s, 210 MB |
| `text 4194304` | 4 MiB | 1.52 s, 585 MB |
| `rules 10000 5000` | 10,000 rules matching every element (`:not(.cN)`), 5,000 elements each of its own class: 50 million | 1.57 s, 55 MB |
| `commands 100000 16384`, `--scale 0.25 --no-replay` | 100,000 bordered boxes in a 16,384-pixel label: 200,005 commands, recorded | 0.38 s, 394 MB |
| the same, replayed at 0.25 (4,096 × 4,096) | | 1.82 s, 726 MB |
| `commands 1000000` | a million, in a 2,000-pixel label | 2.44 s, 1,776 MB (83,255 commands: the rest culled) |
| `lines 200000 16384`, `--scale 0.25` | 200,000 words, a line each: 200,005 commands | 0.48 s, 503 MB |
| `stacked 1000 4096` | 1,000 boxes each over a 4,096-pixel label: 16,800 million pixels painted | 1.63 s, 325 MB |
| `shadows 10000` | 10,000 blurred shadows: 11,957 commands | 0.28 s, 97 MB |
| `tile 16384`, `--scale 0.25` | a 1-pixel PNG tiled over a 16,384-pixel label | 0.08 s, 154 MB: 7 commands, a tiled brush |
| `woff-bomb 1024` | a 1 MB WOFF whose table unpacks to 1 GiB | 0.39 s, 2,186 MB |
| `woff-bomb 32` | one that unpacks to 32 MiB | 0.05 s, 92 MB |
| `pictures 4 4096` | 4 PNGs of 4,096 × 4,096 (256 MiB decoded) | 0.10 s, 359 MB |
| `pictures 16 4096` | 16 (1 GiB decoded) | 0.28 s, 1,175 MB |
| `pictures 1024 4` | 1,024 small PNGs, as many `data:` URLs | 0.16 s, 96 MB |

What sets each limit:

- **Labels, 1,024:** each label's painting walks every slot to cull it, so recording grows as the
  square of the labels: 0.18 s in all for 1,024, 4.6 s for 10,000.
- **Their elements, 100,000 (new):** Blitz holds about 1.7 KB an element, so the drawing's million
  (ADR-0003's) would be 1.8 GB. 100,000 boxes hold 394 MB with their recording, and are some
  twenty times a large real drawing's labels.
- **Text, 1 MiB:** it lays out in 0.4 s and 210 MB; four times that holds 585 MB.
- **Styles, ADR-0003's 50 million:** Stylo matches them in 1.6 s and 55 MB, so the count holds for
  Blitz as it does for usvg.
- **Fonts, 32 MiB each and 64 MiB in all:** wuff unpacks a WOFF 1 to what its header claims. A
  font's unpacked size is read from its header before it is handed to Blitz (WOFF 1: the larger of
  `totalSfntSize` and its tables' `origLength`s; WOFF 2: its tables' `origLength`s; TrueType and
  OpenType: its bytes). At 32 MiB a font holds about 70 MB.
- **Pictures, 64 MiB each and 256 MiB in all:** 256 MiB decoded holds 359 MB, and 1 GiB 1.2 GB. A
  picture's size is read from its header (imagesize) before it is decoded. The same `data:` URL is
  fetched once a document.
- **Requests, 1,024:** answered in 0.16 s and 96 MB.
- **Layout, 16,384 CSS pixels a side:** such a label lays out in a millisecond; what it costs is
  its painting, which the estimate bounds.
- **Overflow, 4,096 CSS pixels each way (new):** a slot is the label's box and that much around
  it, laid out and culled as the box is; 1,024 such labels cost 0.15 s and 77 MB. Every
  `overflow: visible` label seen (older draw.io's) overflows its box by less than its own size;
  newer draw.io's labels are the drawing's size, and inside it.
- **Recording, 200,000 commands (the spec started at a million):** 200,000 replay in about 1.5 s
  and hold some 1 KB each while replayed, besides the page; a million would be about 7 s and 2 GB
  a render. A real drawing's are far fewer: the tldraw drawing's 27 labels are 201 commands, 1,024
  plain labels 8,192.
- **Painting, in the page's estimate:** a command covering a 4,096-pixel page holds about 190 KB
  while replayed, and vello_cpu fills about 10,000 million pixels a second here (`stacked`), so
  2 KiB a command and ADR-0003's 4,000 million pixels of work bound a render to well under a
  second of labels.

**The stack.** `measure stack-search <kind>` builds, lays out, paints, replays and drops one label
nested ever deeper, each depth in a child process (`measure stack <depth> <kind>`), on a spawned
thread with Rust's default 2 MiB stack, or `STACK=8` MiB, and prints the deepest that held before
the child overflowed its stack (SIGABRT). The debug build is `cargo build` there.

| Nested | 2 MiB, release | 2 MiB, debug | 8 MiB, release | 8 MiB, debug |
|---|---|---|---|---|
| `div` | 864 | 132 | 3,456 | 544 |
| `span` | 2,016 | 816 | 8,064 | 3,264 |
| `display: flex` | 640 | 176 | 2,624 | 720 |
| `display: inline-block` | 624 | 136 | 2,496 | 544 |
| `display: grid` | 360 | 58 | 1,440 | 240 |
| `display: table` | 16,128 | 2,880 | 64,512 | 11,520 |

On a render thread's 2 MiB, a debug build (what the tests run) overflows at 58 nested grids, below
the spec's 64. On 8 MiB, 64 levels hold with a margin of 3.75 in a debug build and 22 in release,
nested grids being the deepest-costing kind tried: the limit is 64, on a thread of 8 MiB.

## Alternatives considered

- **Blitz for the whole SVG:** Blitz hands an SVG to usvg with default options and paints usvg's
  tree, so it drops `<foreignObject>` too, and has none of ADR-0003's bounds.
- **One Blitz document a label,** as the spike did: each parses the drawing's styles and loads its
  fonts again, 16 times slower for a tldraw drawing, and 1,024 labels under 3 MiB of styles took
  the spike 111 s and 3.1 GB. Its 32 MiB budget of documents in all left out 1,014 of them.
- **Painting a whole label at the scale shown,** as the spike first did: a label is pixels the
  size it is shown at, so a large label at a large zoom is more pixels than the page, and past
  the spike's cap of 8,192 pixels a side it was lost: every label of a newer draw.io drawing, in
  a window past some 4,100 pixels wide.
- **Blitz on the render thread's 2 MiB,** its nesting held to 64 by what a release build holds
  (360 nested grids): a debug build overflows at 58, so the tests could not show the limit holds.
  Held to 32 instead, it would hold in debug too, and leave out the labels 33 to 64 levels deep.
- **Gecko** (not embeddable) and **Servo** (a browser's weight for a drawing's labels).
- **wordcraft's HTML importer:** it lays out pages, not CSS boxes, has no flex, and is another
  backend's library.

## Consequences

- **Positive:** a drawing's labels cost about one document; a label is sharp at any zoom and
  costs its part of the page.
- **Accepted:**
  - **The limits follow Blitz 0.3.0-beta.4,** anyrender 0.14 and vello_cpu 0.3: moving any of
    them means measuring again. `tests/it/resvg.rs` fails when they move.
  - **Labels are drawn over the drawing,** not among its shapes or inside its groups' opacity.
  - **Blitz isn't built never to panic:** a panic while the labels are prepared leaves them all
    out, with a warning; a stack overflow or an abort is bounded only by the nesting limit.
  - **A thread spawned for each drawing with labels,** joined before its labels are kept.
- **Revisit when:** Blitz leaves beta, bounds its own costs, culls by the right box, or lays out
  `display: table`'s inline content.
