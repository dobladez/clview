# ADR-0003: Bound what a drawing costs before usvg and resvg spend it

- **Status:** Accepted
- **Date:** 2026-10-30

## Context

An SVG is untrusted input, and clv's rules say an allocation sized by input is capped
([`CLAUDE.md`](../../CLAUDE.md)). The SVG backend renders through resvg
([ADR-0002](0002-render-svg-with-resvg.md)), and resvg, usvg and their XML parser bound only
some of what a drawing can make them spend. Read in their source, at the versions locked
(resvg and usvg 0.49.0, tiny-skia 0.12.0, roxmltree 0.21.2):

What they bound:

- **roxmltree:** an entity nests at most 10 deep, and one reference expands at most 255 others.
  An external entity is read only through a resolver, and clv gives none.
- **usvg:** at most 1,000,000 elements once `use`s are copied (`svgtree::parse`), nested at most
  1024 deep; `use` and paint references are to `#id` only; no network.
- **tiny-skia:** at most 1,000,000 dashes a path.

What they don't, each a way for a small file to cost gigabytes or hours:

- **Entities at the top level:** roxmltree counts no references outside an entity, so
  `&big;` a thousand times is a thousand copies of `big`.
- **Conversions repeated per element:** usvg converts a clip path, a mask, a filter or a paint
  server anew for each element that uses it when its units are the element's bounding box (the
  default for masks and filters), and a marker anew for each vertex it marks, with no count
  (`parser/clippath.rs`, `mask.rs`, `filter.rs`, `paint_server.rs`, `marker.rs`). Text, and long
  path data, repeat through `use`s within the million elements.
- **Stylesheets:** every rule is tried on every element.
- **Text layout:** usvg shapes a `<text>` in time that grows as the square of its characters,
  and as its spans times its characters (harfrust shaping, called for each span, in
  `text::layout::shape_text_with_font`): 20,000 characters in 6,667 `<tspan>`s take half a
  minute.
- **Layers:** resvg draws a group with opacity, a clip path, a mask, a filter or a blend mode
  into a layer of its own, the size of what it holds but within five times the canvas each way
  (`render.rs`, `max_filter_bbox`), and holds it while what is in it draws, so nested layers add
  up. A clip path or a mask takes pixmaps the layer's size; a filter keeps a result the layer's
  size for each of its primitives (`filter/mod.rs`); a pattern's tile is its own size, however
  large (`path.rs`, `render_pattern_pixmap`).
- **Work:** a morphology's radius, a convolution's kernel and turbulence's octaves multiply the
  pixels a filter computes; resvg's turbulence overflows an integer past some 30 octaves.
- **Embedded pictures** decode at their own size, and embedded SVGs nest without end; usvg's
  svgz gunzip has no cap.

A render can't be stopped once begun (resvg has no cancellation), and the viewer opens every
file in a list before it shows anything.

## Decision

`src/source/svg.rs` bounds what a drawing costs, in three places, and names each limit:

1. **Reading:** a drawing of more than 64 MiB, or an svgz that unpacks to more, is refused. So
   is one whose entities can expand to more than 128 MiB, bounded from its text before roxmltree
   expands them: each `&` as long as the longest entity, itself expanded ten levels down.
2. **Before usvg parses it** (`Scan`): the document is walked from its root the way usvg goes,
   counting each `use` copied again, a marker for each vertex it marks (a path's data needs two
   bytes a vertex), and the clip path, mask, filter or paint a shape refers to (by attribute,
   style or stylesheet, inherited or not) once for each shape. A resource is walked once and
   counted by its weight after that. Refused past 128 MiB of content counted, 1,000,000
   elements, 1024 levels (uses and references counted), or stylesheet rules times elements past
   50,000,000. Left out with a warning, the rest drawn: text past 200,000 characters, or whose
   layout would cost more than 5,000,000,000, counting for each `<text>` its characters
   squared plus 170 times its spans times its characters (about three seconds a parse here);
   an embedded picture past 64 MiB decoded; an embedded SVG broken or past four deep. Font
   fallback looks through every font at most 64 times a drawing.
3. **Before resvg renders a page** (`Estimate`): what the render holds at once and the pixels it
   paints are estimated by walking the tree the way resvg 0.49 renders it (the layers, clip and
   mask pixmaps, filter buffers, pattern tiles and embedded SVGs' pixmaps above, nested ones
   added up). The page fails with why past 1 GiB held, 4,000,000,000 pixels painted, or ten
   million nodes visited (a pattern draws again for each shape it fills), and past 16,384
   pixels a side. A smaller render of the same drawing, its tile in a list, still comes.

## Evidence

Measured 2026-10-30 on an Apple M3 Max, macOS 26.6.2, Rust 1.95.0, with
`cargo build --release --no-default-features --features svg`. Each drawing below is 300 × 200,
printed with `env -u TMUX TERM=dumb clv <file> --protocol kitty >/dev/null`: an 80 × 24 window
of 10 × 20 pixel cells, so a 690 × 460 canvas (317,400 pixels). "Estimated" is the `Estimate`'s
work, printed by building with `MAX_WORK` at 1 for the run; the time is the whole run.

| Drawing | Estimated work | Time |
|---|---|---|
| 1,000 rectangles over the page, half transparent | 317 million | 0.61 s |
| 6,000 of them | 1,904 million | 3.49 s |
| 12,000 of them | 3,809 million | 7.08 s |
| `feMorphology radius='5'` over the page (12 px) | 287 million | 0.33 s |
| `feMorphology radius='13'` (30 px) | 1,702 million | 1.85 s |
| `feTurbulence numOctaves='10'` over the page | 92 million | 0.14 s |
| `numOctaves='100'` | 915 million | 1.01 s |
| `numOctaves='1000'` | 9,142 million: refused | 9.83 s, timed before it was |
| A drop shadow (blur, offset, merge) under a circle | 5 million | 0.03 s |

Fills paint about 550 million of the estimate's pixels a second, the slowest, so the cap is
about seven seconds here; filters run at about 900 million, so they are estimated high. An
octave of turbulence was first counted as 8 pixels and measured at more than twice that: it is
counted as 20. A machine slower than this one takes longer within the same cap. The drawings
are generated, a rectangle `<rect width='300' height='200' fill='teal' fill-opacity='0.5'/>`
repeated, or one rectangle filtered by `<filter id='f'><feMorphology radius='5'/></filter>` and
so on, in a `<svg width="300" height="200">`.

| Parse | Time |
|---|---|
| 10,000 elements, a stylesheet of 1,000 class rules (10 million matches) | 0.27 s |
| 50,000 elements, 100 rules | 0.29 s |
| 50,000 elements, 1,000 rules | refused: past 50 million |
| 50,000 characters of text, in 500 `<text>`s | 0.25 s |
| 200,000 characters | 0.85 s |

| Text layout: one `<text>` of | Time |
|---|---|
| 5,000 characters | 0.12 s |
| 20,000 | 0.50 s |
| 40,000 | 1.88 s |
| 80,000 | 7.37 s |
| 20,000 characters in 100 `<tspan>`s | 1.15 s |
| 20,000 in 500 | 2.67 s |
| 20,000 in 1,000 | 4.61 s |
| 20,000 in 6,667 | 27.4 s: left out now |
| 10,000 in 3,333 | 7.03 s |
| 6,000 in 2,000 | 2.59 s |

The times include clv's two parses (to open and to render) and the render. Text fits
1.15 × 10⁻⁹ n² + 2 × 10⁻⁷ s·n seconds for n characters in s spans, so a span costs 170 times
a character squared, and the limit is about three seconds a parse. The text drawings are a
`<text y='10' font-size='2'>` of `ab ` repeated, or of `<tspan>` elements each `a` repeated
and a space.

The tests pin each limit and show it acts: `src/source/svg.rs`'s
`entities_that_expand_too_far_are_refused`, `uses_that_repeat_too_much_are_refused_before_parsing`,
`markers_and_references_are_counted_as_often_as_usvg_repeats_them`,
`text_is_counted_through_uses_and_too_much_is_left_out`, `text_too_long_to_lay_out_is_left_out`,
`nesting_too_deep_is_refused_before_parsing`,
`an_embedded_picture_is_drawn_and_one_too_large_left_out`, `embedded_drawings_nest_only_so_deep`,
`a_drawing_too_large_is_refused`, `a_render_that_would_hold_too_much_is_refused` and
`a_render_that_would_take_too_long_is_refused`; and that an ordinary drawing isn't refused:
`an_ordinary_drawing_renders_at_the_largest_size` (a drop shadow, a clip path and opacity at
4000 × 4000, the most the viewer asks for) and `the_deepest_drawing_parses_and_renders_on_a_render_threads_stack`
(1022 nested groups on a 2 MiB stack). Each was seen to fail with its limit taken out; their
cases are sized just past each limit, so that rendering them anyway stays affordable (about
1.2 GB or eight seconds, by the estimate). Memory wasn't measured.

## Alternatives considered

- **Trusting usvg's million elements:** 30 levels of doubling `use`s in 2 KB reach it and are
  refused, but 15,000 vertices marked with a 10 KB marker, 512 uses of a 300 KB path or forty
  nested opacity groups are within it, and usvg and resvg spend on each what it repeats: 150 MB
  of content counted, or 1.27 GB of layers estimated, and many times that for slightly larger
  files.
- **Rendering in a child process, killed past a time or memory limit:** a process per page, a
  protocol between them, and rlimits that differ between macOS and Linux, for what an estimate
  does in the process.
- **A counting allocator that stops a render past a budget:** implementing `GlobalAlloc` is
  `unsafe` (forbidden here), and a failed allocation aborts rather than unwinds.
- **Rendering smaller when over budget,** blurred to the size wanted: possible later; meanwhile
  the drawing's tile in a list renders.
- **Refusing filters, masks or patterns:** loses ordinary drawings (a drop shadow) to stop
  hostile ones, which the estimate tells apart.

## Consequences

- **Positive:** a drawing opens within 64 MiB read and 128 MiB counted, its text laid out in
  about three seconds a parse at most, and a page renders within about 1 GiB and seven seconds
  here, whatever the drawing says.
- **Accepted:**
  - **The estimate follows resvg 0.49:** moving resvg means reading how it renders again.
    `tests/it/resvg.rs`'s `the_render_estimate_is_for_the_resvg_this_builds` fails when it moves.
  - **It errs high:** a drawing at its limit may be refused at the window's size and shown as a
    tile. A layer is estimated as large as its bounds, even where little of it is drawn, and a
    filter's work at about 1.7 times what it takes, against a fill's.
  - **The scan errs high too:** it counts every shape a stylesheet rule might match unless the
    rule's selector is a list of classes, and a marker for every vertex.
  - **Not every cost is bounded:** a chain of gradients each inheriting from the next, or a
    selector's work per match, still costs what usvg spends; and a render within the limits runs
    to its end.
  - **Seven seconds is this machine's:** a slower one takes longer within the same count.
- **Revisit when:** resvg moves past 0.49, gains limits or cancellation of its own, or usvg
  bounds its conversions.
