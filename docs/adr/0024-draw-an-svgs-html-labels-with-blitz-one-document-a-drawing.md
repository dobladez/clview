# ADR-0024: Draw an SVG's HTML labels with Blitz, one document a drawing, each stage bounded

- **Status:** Accepted
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
  nothing else. Painting multiplies through CSS lists and tiling: a command for each item of a
  `box-shadow` list, each `background-image` and `mask-image` layer, and each tile of a picture
  spaced (`background-repeat: space`); a gradient's tiles it stops at 500. A few bytes of style can
  ask for billions of commands, and a painting can't be stopped once begun.
- **The stack:** styles, layout and painting recurse once a level of nesting.

## Decision

1. **resvg still draws the drawing; Blitz only its labels,** drawn over it.
2. **One Blitz document a drawing.** Each label is a box of its own (a slot) in it: the drawing's
   `<style>`s, then the label inside copies of its ancestors and of the `<foreignObject>`, the
   copy made the box, the containing block of what it holds. The slots are packed in rows about
   as wide as all of them would be square, 8 CSS pixels apart, each placed by
   `transform: translate()` rather than `left`/`top` (Blitz culls by a box that counts an
   element's position twice, so a slot placed by `left` and `top` would not cull its neighbours).
   Its styles resolved, the pictures they ask for (a `background-image`, fetched as they resolve)
   answered, and its styles resolved again, it is laid out once; each label is painted once at
   scale 1, the viewport scrolled to its slot and sized to it, into an anyrender `Scene` of its own,
   kept with the usvg tree. Each render
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
| Layout | a label's laid-out size; any element of it, or line of its text | 16,384 CSS pixels a side; 24,576 (16,384 and the overflow each way) | that label left out |
| Overflow | what an `overflow: visible` label draws past its box | 4,096 CSS pixels each way | cut |
| Paint work | after layout, before painting: each element's shadows (6 steps each); each background and mask layer's tiles (a gradient 500, with a step for each 32 of its stops; a picture 1, or a tile an axis it spaces), 4 a layer; each dotted or dashed border edge's dots (20 steps each); 4 steps a pixel of text line for each dotted, dashed or wavy decoration; the document's own `html` and `body` once for each label | 20,000,000 | all labels left out, with a warning |
| Recording | commands, all labels, counted as they are painted: a recorder keeps none past the limit | 200,000 | all labels left out, with a warning |
| Painting | each label's part of the page, and its commands | in the page's estimate (ADR-0003's 1 GiB and 4,000 million pixels) | the page fails with why |

The paint work follows blitz-paint 0.3.0-beta.4's painting (`box_shadow.rs`, `background.rs`'s
`compute_layer_size`, `raster_axis_tiling` and `gradient_axis_tiling`, `mask.rs`, `border.rs`'s
dotted and dashed edges, `text.rs`'s decorations), reading Stylo's computed values and Parley's
lines, as ADR-0003's `Estimate` follows resvg 0.49. Each label's painting paints the document's
`html` and `body` (and their `::before` and `::after`) again, so what they paint counts once a
label; the labels' slots and what they hold, culled, once. The painting estimate counts, for
each label landing on the page, its part's pixels twice
(painted, then drawn over the page) and 2 KiB a command held; and as work, each command's box on
the page within that part, and the part's pixels.

## Evidence

Measured 2026-11-09 on an Apple M3 Max (14 cores, 36 GiB), macOS 26.6.2, Rust 1.95.0, with the
measuring tool [`0024-measure/`](0024-measure/src/main.rs) beside this ADR: a crate of its own, in
its own workspace, outside clv's build, with blitz-dom, blitz-html, blitz-paint and blitz-traits
0.3.0-beta.4, anyrender 0.14.0, anyrender_vello_cpu 0.18.0 (vello_cpu 0.3.0), stylo 0.22.0, taffy
0.15.1, parley 0.12.0, wuff 0.2.9 and image 0.25.10: its `Cargo.lock` is clv's, so every crate the two
share is at the version clv builds. `measure gen <kind> …`
writes each drawing, synthetic; `measure run <x.svg>` finds its labels as the spec says
(simplified: no `<switch>` or nested `<svg>`), lays them out in one document (or one a label,
`--each`), estimates its paint work and stops past the limit (unless `PAINT_ANYWAY=1`), records
each label through a recorder that keeps at most 200,000 commands and counts the rest, and replays
each into the page at `--scale` (1 unless said), past the commands' limit not at all. It prints
each stage's time and counts, the commands painted in all, and the rules times the elements of its
document, as ADR-0003 counts them. Built with `cargo build --release` in `docs/adr/0024-measure/`;
each line is `target/release/measure gen <kind> … > x.svg`, then
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
| `font 50 400`: 50 labels sharing a 400 KB `@font-face`, a TrueType font made by the tool | 0.07 s, 32 MB | 0.98 s, 57 MB |
| `styles 1024 3072`: 1,024 labels under 3 MiB of styles, 8,313 rules of some 380 bytes (8,313 rules × 5,125 elements, 42.6 million, within ADR-0003's 50 million) | 0.60 s, 145 MB | 315.6 s, 435 MB |

One document a label parses the styles and loads the font again for each label, as the spike did.

**Each stage,** one document:

| `measure gen …`, then `run` | What it is | Time, memory |
|---|---|---|
| `labels 1024` | 1,024 labels, a box and a word each | 0.18 s, 74 MB: 8,192 commands |
| `labels 10000` | 10,000 | 5.83 s, 503 MB, nearly all of it recording |
| `visible 1024` | 1,024 `overflow: visible` labels, each with a box three boxes past its own | 0.21 s, 76 MB; the boxes drawn |
| `text 1048576` | 1 MiB of text in one label | 0.43 s, 210 MB |
| `text 4194304` | 4 MiB | 1.58 s, 587 MB |
| `rules 9900 5000` | 9,900 rules matching every element (`:not(.cN)`), 5,000 elements each of its own class: 9,900 × 5,009, 49.6 million | 1.69 s, 53 MB |
| `commands 99990 16384`, `--scale 0.25 --no-replay` | 99,990 bordered boxes in a 16,384-pixel label: 199,985 commands, recorded | 0.41 s, 394 MB |
| the same, replayed at 0.25 (4,096 × 4,096) | | 1.90 s, 720 MB |
| `commands 1000000` | a million, in a 2,000-pixel label | 2.73 s, 1,779 MB (83,255 commands: the rest culled) |
| `lines 199990 16384`, `--scale 0.25` | 199,990 words, a line each: 199,995 commands | 0.50 s, 503 MB |
| `stacked 1000 4096` | 1,000 boxes each over a 4,096-pixel label: 16,800 million pixels painted | 1.72 s, 350 MB |
| `shadows 10000` | 10,000 blurred shadows: 11,957 commands | 0.30 s, 97 MB |
| `tile 16384`, `--scale 0.25` | a 1-pixel PNG tiled over a 16,384-pixel label | 0.08 s, 155 MB: 8 commands, a tiled brush |
| `woff-bomb 1024` | a 1 MB WOFF whose table unpacks to 1 GiB | 0.43 s, 2,190 MB |
| `woff-bomb 32` | one that unpacks to 32 MiB | 0.06 s, 92 MB |
| `pictures 4 4096` | 4 PNGs of 4,096 × 4,096 (256 MiB decoded) | 0.12 s, 360 MB |
| `pictures 16 4096` | 16 (1 GiB decoded) | 0.31 s, 1,175 MB |
| `pictures 1024 4` | 1,024 small PNGs, as many `data:` URLs | 0.18 s, 96 MB |

**Painting multiplied:** the work estimated, and with `PAINT_ANYWAY=1` the commands painted in all.

| `measure gen …`, then `run` | What it is | Work estimated | Painted anyway | Time, memory |
|---|---|---|---|---|
| `shadow-list 398 500` | 398 boxes, each with the 500 shadows of one rule | 1.19 million | 199,801 | 0.49 s, 418 MB, replayed |
| `shadow-list 500 500` | 500 of them | 1.5 million | 251,005: past 200,000, left out | 0.05 s, 115 MB |
| `shadow-list 99000 33` | 99,000 boxes, 33 shadows each | 19.6 million | 2.06 million: left out | 0.25 s, 255 MB |
| `shadow-list 99000 34` | 34 each | 20.2 million: left out | 2.13 million | 0.20 s, 169 MB |
| `shadow-list 99000 50000` | 50,000 each, 850 KB of style | 29,700 million: left out | 3,125 million, in 18.3 s | 0.23 s, 182 MB |
| `layers 1000 39` | 1,000 boxes, 39 gradient layers each | 19.66 million | 41,051 | 0.09 s, 181 MB |
| `layers 1000 40` | 40 each | 20.16 million: left out | 42,051 | 0.04 s, 21 MB |
| `space 4472 1` | a 1-pixel PNG spaced in 1-pixel tiles over a 4,472-pixel box | 19,998,788 | 19,998,791, in 106 ms: left out | 0.16 s, 206 MB |
| `space 4473 1` | over 4,473 pixels | 20,007,733: left out | 20,007,736 | 0.04 s, 19 MB |
| `html-shadows 1024 50000` | 1,024 labels, 50,000 shadows on the document's `html` | 307 million: left out | 51 million, a label at a time | 0.09 s, 58 MB |
| `html-shadows 100 33333` | 100 labels, 33,333 shadows | 20.0 million | past 200,000 commands: left out | 0.09 s, 130 MB |
| `html-shadows 101 33333` | 101 labels | 20.2 million: left out | | 0.06 s, 35 MB |
| `dotted 12 10000`, `--scale 0.05` | 12 boxes 10,000 pixels a side, each a dotted border of 1 pixel: 960,000 dots | 19.2 million | 149 commands | 0.11 s, 191 MB |
| `dotted 13 10000` | 13 of them | 20.8 million: left out | 149 commands, 205 MB | 0.03 s, 19 MB |
| `long-border 1000000 dotted 1` | a 400 × 100 label holding a box a million pixels wide, its border dotted | 80 million, and the box past 24,576 pixels: left out | 17 commands, 862 MB | 0.03 s, 19 MB |
| `wavy 100000` | 100,000 bytes of text, a wavy underline under it | 7.9 million | 1,653 commands | 0.18 s, 87 MB |
| `wavy 1048576` | 1 MiB of it | its box 950,000 pixels tall: left out | | |

"Left out" past the paint work's limit is before painting, so its time and memory are the
drawing's without it; past the commands' limit it is after a painting the recorder kept 200,000
of. The estimate errs high: six commands a shadow where an outset one paints one, 500 a gradient
where one paints one tile in most boxes; a spaced picture's tiles it counts as blitz-paint does,
to within the 4 commands around the layer.

What sets each limit:

- **Labels, 1,024:** each label's painting walks every slot to cull it, so recording grows as the
  square of the labels: 0.18 s in all for 1,024, 4.6 s for 10,000.
- **Their elements, 100,000 (new):** Blitz holds about 1.7 KB an element, so the drawing's million
  (ADR-0003's) would be 1.8 GB. 100,000 boxes hold 394 MB with their recording, and are some
  twenty times a large real drawing's labels.
- **Text, 1 MiB:** it lays out in 0.4 s and 210 MB; four times that holds 585 MB.
- **Styles, ADR-0003's 50 million:** Stylo matches them in 1.6 s and 53 MB, so the count holds for
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
- **An element, or a line of text, past 24,576 pixels (new):** blitz-paint builds some paths at
  their box's size before anything counts them: a dotted border's dots, all of an edge's in one
  path, some 170 bytes a dot, 862 MB for a box a million pixels wide (`long-border`). A label
  holding an element, or a line of text, larger than the label may be (16,384 pixels) and what it
  may draw past its box (4,096 each way), is left out. So is a label whose text runs past it:
  1 MiB of text in an 800-pixel box is 950,000 pixels of lines.
- **Paint work, 20 million (new):** the work Blitz would do painting, estimated before it does,
  since a painting runs to its end and a recorder that keeps nothing still lets Blitz walk every
  command: 3,125 million shadows took 18.3 s with none kept, and the 8 MiB of markup allows some
  500,000 in one rule, three minutes. 20 million tiles painted with nothing kept take 0.1 s
  (`space 4472 1`); estimating takes 3 ms for 99,000 elements. Painting's other commands, a few an
  element and some a line of text, are bounded by the elements and the text. A dot of a dotted
  border (or a dash of a dashed one) counts 20, its 170 bytes held: 20 million steps are a million
  dots, 191 MB (`dotted 12`). A decoration's dot, dash or wave is at least a pixel (blitz-paint
  floors its thickness to one), so a dotted, dashed or wavy decoration draws at most one a pixel of
  each of its three lines: 4 steps a pixel of every line of text, for each such decoration. A
  gradient's fills hold its stops, 20 bytes each: a step for each 32. What the document's `html`
  and `body` paint counts once a label: Codex's review found 50,000 shadows on `html` estimated at
  300,000 and painted 51 million times over 1,024 labels.
- **Recording, 200,000 commands (the spec started at a million), counted as they are painted:** a
  recorder in front of anyrender's `Scene` keeps them while the budget lasts and only counts past
  it, so what a recording holds is bounded whatever the estimate missed: 251,005 painted cost
  115 MB. 200,000 replay in about 1.5 s and hold some 1 KB each while replayed, besides the page;
  a million would be about 7 s and 2 GB a render. A real drawing's are far fewer: the tldraw
  drawing's 27 labels are 201 commands, 1,024 plain labels 8,192. Past it, what was kept isn't
  replayed: its layers, cut off, don't balance, and vello_cpu panics on that.
- **Painting, in the page's estimate:** a command covering a 4,096-pixel page holds about 190 KB
  while replayed, and vello_cpu fills about 10,000 million pixels a second here (`stacked`), so
  2 KiB a command and ADR-0003's 4,000 million pixels of work bound a render to well under a
  second of labels.

**The stack.** `measure stack-search <kind>` builds, lays out, paints, replays and drops one label
nested ever deeper, each depth in a child process (`measure stack <depth> <kind>`), on a spawned
thread with Rust's default 2 MiB stack, or `STACK=8` MiB, and prints the deepest that held before
the child overflowed its stack (SIGABRT). The debug build is `cargo build` there.

| Nested | 2 MiB, release | 2 MiB, debug | 8 MiB, release | 8 MiB, debug | 2 MiB, clv's dev profile | 8 MiB, clv's dev profile |
|---|---|---|---|---|---|---|
| `div` | 864 | 132 | 3,456 | 544 | 928 | 3,776 |
| `span` | 2,016 | 816 | 8,064 | 3,264 | | |
| `display: flex` | 640 | 176 | 2,624 | 720 | 720 | 2,880 |
| `display: inline-block` | 624 | 136 | 2,496 | 544 | | |
| `display: grid` | 360 | 58 | 1,440 | 240 | 392 | 1,600 |
| `display: table` | 16,128 | 2,880 | 64,512 | 11,520 | | |

"Debug" is the tool's own debug build, nothing optimized. clv's dev profile, what its tests run,
builds its dependencies at `opt-level = 2` (`[profile.dev.package."*"]`); the last two columns are
the tool built so (those lines added to its `Cargo.toml`, `CARGO_TARGET_DIR=target/clv-dev cargo
build`). An unoptimized build overflows a render thread's 2 MiB at 58 nested grids, below the
spec's 64; with clv's profiles, 2 MiB holds 392. Blitz runs on a thread of 8 MiB all the same: 64
levels then hold with a margin of 3.75 even unoptimized, and of 25 as clv builds, for a thread
spawned for each drawing with labels. The limit is 64.

## Alternatives considered

- **Blitz for the whole SVG:** Blitz hands an SVG to usvg with default options and paints usvg's
  tree, so it drops `<foreignObject>` too, and has none of ADR-0003's bounds.
- **One Blitz document a label,** as the spike did: each parses the drawing's styles and loads its
  fonts again, 16 times slower for a tldraw drawing, and 530 times for 1,024 labels under 3 MiB
  of styles (316 s and 435 MB here; 111 s and 3.1 GB for the spike's styles). Its 32 MiB budget
  of documents in all left out 1,014 of them.
- **Painting a whole label at the scale shown,** as the spike first did: a label is pixels the
  size it is shown at, so a large label at a large zoom is more pixels than the page, and past
  the spike's cap of 8,192 pixels a side it was lost: every label of a newer draw.io drawing, in
  a window past some 4,100 pixels wide.
- **Blitz on the render thread's 2 MiB,** its nesting held to 64 by what an optimized build holds
  (360 nested grids, 392 as clv's tests build it): an unoptimized build overflows at 58. Held to
  32 instead, it would hold unoptimized too, and leave out the labels 33 to 64 levels deep.
- **Counting commands only, as they are painted,** without estimating first: memory is bounded,
  time is not (above).
- **A recorder that stops the painting by panicking past its limit,** caught as Blitz's panics
  are: exact, but a panic on purpose, which `CONTRIBUTING.md`'s rules forbid.
- **Gecko** (not embeddable) and **Servo** (a browser's weight for a drawing's labels).
- **wordcraft's HTML importer:** it lays out pages, not CSS boxes, has no flex, and is another
  backend's library.

## Consequences

- **Positive:** a drawing's labels cost about one document; a label is sharp at any zoom and
  costs its part of the page.
- **Accepted:**
  - **The limits follow Blitz 0.3.0-beta.4,** anyrender 0.14 and vello_cpu 0.3, and the paint work
    estimate blitz-paint 0.3.0-beta.4's painting: moving any of them means reading and measuring
    again. `tests/it/resvg.rs` fails when they move.
  - **The estimate errs high:** a drawing of many gradients or inset shadows may have its labels
    left out though painting them would be cheap.
  - **Labels are drawn over the drawing,** not among its shapes or inside its groups' opacity.
  - **Blitz isn't built never to panic:** a panic while the labels are prepared leaves them all
    out, with a warning; a stack overflow or an abort is bounded only by the nesting limit.
  - **A thread spawned for each drawing with labels,** joined before its labels are kept.
- **Revisit when:** Blitz leaves beta, bounds its own costs, culls by the right box, or lays out
  `display: table`'s inline content.
