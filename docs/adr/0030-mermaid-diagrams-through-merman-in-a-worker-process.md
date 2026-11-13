# ADR-0030: Mermaid diagrams through merman, in a worker process capped at 2 s and 512 MiB

- **Status:** Accepted
- **Date:** 2026-11-13

## Context

[Spec-0008](../specs/0008-show-mermaid-diagrams.md) shows a Mermaid diagram (`.mmd`, `.mermaid`) by
turning it into SVG and handing that to the SVG backend, and leaves to a spike which crate turns it
into SVG and at what caps. A diagram is untrusted input: its text is small, but laying it out can
cost far more than its size. clv is Rust only, so mermaid.js, which also needs a browser to measure
text, is out; the candidates are the three pure-Rust renderers on crates.io: merman 0.10.0,
mermaid-rs-renderer 0.3.1 and ariel-rs 0.4.0.

The worker that runs it is [ADR-0026](0026-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md)'s
machinery (`src/source/worker.rs`): clv run again as a worker, its heap capped by the allocator,
its CPU time by the kernel, its wall clock by the viewer, killed past any. A list's tile starts no
worker for an SVG's HTML labels, so a diagram labelled with `<foreignObject>` would show in a tile
without its words.

## Decision

1. **merman 0.10.0**, from crates.io, with `default-features = false` and the features
   `all-diagrams`, `svg` and `layout-cytoscape`: every diagram kind it parses, its SVG, and the
   Cytoscape layout. Not `layout-elk`, a default, which brings ELK under EPL-2.0 and its notices;
   nor the features that read the system's fonts or the clock (`system-fonts`, `native-fonts`,
   `system-clock`, `system-timing`), nor `png`, `pdf`, `ascii`, `math`. It adds 25 crates to
   clv's build, under MIT or Apache-2.0 (`merman-unicode-linebreak` and
   `unicode-general-category` Apache-2.0 alone); no resvg, nothing from git, nothing that reads a
   file, the environment or the network on its render path.
2. **Labels as SVG `<text>`:** the renderer's site config sets `htmlLabels` false, for flowcharts
   too, and lists it in Mermaid's `secure` keys with Mermaid's own (`secure`, `securityLevel`,
   `startOnLoad`, `maxTextSize`, `suppressErrorRendering`, `maxEdges`), so a diagram's
   `%%{init}%%` can't turn it back on. So a tile shows a diagram's words. A user journey keeps
   `<foreignObject>` for its sections and tasks, as mermaid.js draws them, and a kanban board for
   its cards.
3. **merman's own limits are the first:** its `Constrained` resource profile (1 MiB of source,
   16,000 model items, 1 MiB of model text, 128 levels deep) and Mermaid's 500 edges, which refuse
   every hostile diagram below but the largest within them in milliseconds, with a message.
4. **The worker's caps, past those:** 2 s of CPU time a diagram, its wall clock three times that
   (6 s), and 512 MiB of heap: about ten times the CPU and twice the heap of the costliest diagram
   merman accepts. merman's `OperationControl` is given a deadline of 1.5 s, so it stops with its own
   message before the kernel kills it.
5. **On white:** merman's SVG has no background, as mermaid.js's has none, and clv shows an SVG on
   the terminal's background; Mermaid's default ink (#333 edges, lifelines and edge labels) is made
   for white, and nearly vanishes on a dark terminal. So clv puts a white `<rect>` over the
   diagram's viewBox as the first child of its `<svg>`, as mermaid-cli's `-b white` gives it: the
   `<svg>` start tag alone is read for it, within 4 KiB, the document never parsed again. The
   owner's choice, on 2026-11-13, over a diagram as transparent as a drawing.
6. **The numbers [Spec-0008](../specs/0008-show-mermaid-diagrams.md) left to this:**
   - a diagram read at most 1 MiB, merman's own limit;
   - its SVG answer at most 64 MiB (the largest seen, 22 MB, from a 1 MB label);
   - the cache at 64 MiB of SVG and 4,096 diagrams (a typical diagram's SVG is 5 to 65 KB);
   - after three workers killed in a row, none for 60 s: three kills cost at most 6 s of CPU;
   - a `.mmd` told by the first word of a label in `merman::diagram_header_facts()` (47 headers,
     `flowchart`, `graph`, `sequenceDiagram`, `stateDiagram-v2`…), so the list is merman's.

## Evidence

Measured 2026-11-13 on an Apple M3 Max (14 cores, 36 GiB), macOS 26.6.2, Rust 1.95.0, with the
crates in [`0030-measure/`](0030-measure/): each a crate of its own, in its own workspace, outside
clv's build, built with `cargo build --release -j 2`. `candidates/<crate>` turns a diagram into SVG
(`t-<crate> <in.mmd> <out.svg>`, or `batch <dir>` for every `.mmd` in one process), merman with the
config and profile above; `raster` draws an SVG as clv's SVG backend does (resvg 0.49, clv's pin,
the system's fonts, at 1.5×); `gen <dir>`, `gen within <dir>` and `gen worst <dir>` write the
hostile diagrams; `hostile.sh <bin> <dir>` runs each through `/usr/bin/time -l` and `timeout 60`, what [`hostile-results.txt`](0030-measure/hostile-results.txt) holds for the first set.
mermaid.js is mermaid-cli 12.2.0 (`npx @mermaid-js/mermaid-cli`, `mmdc -i x.mmd -o x.png -s 1.5 -b white`),
the reference.

**What each brings,** with default features off (merman with the features above):

| | merman 0.10.0 | mermaid-rs-renderer 0.3.1 | ariel-rs 0.4.0 |
|---|---|---|---|
| Licence | MIT or Apache-2.0 | MIT | MIT |
| Crates in its tree | 154, 25 new to clv | 38 | 22 |
| Build from nothing, `-j 2` | 3 min 12 s | 40 s | 19 s |
| A second resvg, usvg or fontdb | no | fontdb 0.23 (clv has 0.24); usvg 0.47 only with `png` | no |
| Reads files | no | the system's fonts, and **writes copies of them** to `~/.cache/mmdr/font-cache` | no (Liberation Sans built in) |
| Its own limits on input | source, items, text, nesting, layout work, Mermaid's edges | none found | none found |
| Lists its diagram kinds | `diagram_header_facts()`, `supported_diagrams()` | no | a `DiagramType` from `detect` |
| `%%{init}%%`, front matter | both | both | refuses the diagram that has both ("Unrecognized diagram type") |
| Labels | `<text>` when asked (`htmlLabels` false) | `<text>` | `<text>` |

**Twelve real diagrams** ([`0030-measure/diagrams/`](0030-measure/diagrams/)): a flowchart with a
subgraph, sequence, class, state with a composite state, ER, gantt, pie, mindmap, timeline, Unicode
and long labels, a styled flowchart with front matter and `init`, and a user journey.

| | merman | mermaid-rs-renderer | ariel-rs |
|---|---|---|---|
| Drawn | 12 of 12 | 11: refuses a class's annotation (`<<trait>>`) | 11: refuses front matter with `init` |
| Against mermaid.js 12.2 | the same look (Mermaid 12's default theme), the same layout but its edges' routing (ELK's there), the same subgraph membership | its own look; edges routed otherwise | its own look; a node put outside its subgraph; a state's self-loop drawn on the wrong state |
| Time a diagram, in one process | 40 ms for the first (setting up), then 1 to 10 ms | 0.3 to 120 ms (its fonts) | 0.1 to 4 ms |

The fonts: merman sizes its boxes by metrics of its own for Mermaid 12's font ("Recursive
Variable", then `trebuchet ms`, `arial`), and resvg draws them in the system's Arial. Labels fit
their boxes but for a sequence note too long for its box, which mermaid.js draws overflowing the
same way. Wrapping falls where mermaid.js's does.

**Twenty diagrams in one process** (the twelve and eight of them again, `batch`): merman 83 ms in
all and 21 MB at most; so a document referring to 20 diagrams costs one worker's start (about
40 ms, [ADR-0026](0026-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md)) and
some 0.1 s, where a worker a diagram would cost each its start and merman's 40 ms setting up, some
1.7 s.

**Hostile diagrams** (`gen`), merman with its `Constrained` profile; CPU time, maximum resident
set, and what it answered:

| Diagram | merman | mermaid-rs-renderer | ariel-rs |
|---|---|---|---|
| A chain of 1,000 / 10,000 nodes | refused, 500 edges, 3 ms | 4.1 s; 47 s (3,000: 10.5 s) | 0.13 s; aborts at 10,000 (1.1 s, 158 MB) |
| Every node to every other, 40 / 80 / 160 nodes | refused, 500 edges, 2 to 4 ms | past 60 s at each, 40 nodes included (645 MB at 160) | 4.0 s, 157 MB; past 60 s at 80 (1.2 GB) and 160 (3.5 GB) |
| Subgraphs nested 50 / 200 / 1,000 deep | 21 ms; refused, 128 levels; refused, 500 edges | 0.2 s; 0.7 s; 47 s, 231 MB | 0.2 s; 40 s; past 60 s |
| One label of 100 KB / 1 MB | 16 ms, 2.2 MB of SVG; 0.13 s, 83 MB, 22 MB of SVG | 24 ms; 0.22 s | 3 ms; 28 ms |
| 2,000 / 20,000 messages | 14 ms, 0.9 MB of SVG; refused, 16,000 items | 7.0 s; past 60 s | 6 ms; 60 ms, 8.8 MB of SVG |
| 1 MiB of edges | refused, 1 MiB of source, 1 ms | past 60 s | 1.2 s, 116 MB, 43 MB of SVG |
| Garbage, after a flowchart's header / alone | refused, its parse error / no diagram | refused, its parse error | drawn (67 KB of SVG); refused |
| 100,000 brackets opened | refused, its parse error, 5 ms | drawn | drawn |

**The most merman accepts** (`gen within`, `gen worst`), each a diagram made to cost it the most
under one limit:

| Diagram | CPU time | Maximum resident set | SVG |
|---|---|---|---|
| 15,000 nodes | 0.21 s | 239 MB | 9.7 MB |
| 300 classes of 50 members | 0.22 s | 35 MB | 6.7 MB |
| 15,000 messages | 0.09 s | 46 MB | 6.1 MB |
| An unbroken label of 1 MB | 0.14 s | 52 MB | 9.2 MB |
| 32 nodes each to every other (496 edges) | 0.05 s | 49 MB | 1.2 MB |
| A gantt over 10,000 years, ticked daily | 3 ms | 10 MB | 9 KB |
| 10,000 slices of a pie | 17 ms | 17 MB | 1.8 MB |
| 499 edges among subgraphs 100 deep | refused, its layout work, 0.15 s | 110 MB | |
| A mindmap 120 deep; 10,000 points of a chart; 15,000 long labels | refused: 128 levels, 16,000 items, 1 MiB of text | | |

**Through clv's own worker** (2026-11-13, at the commit that adds merman, debug build, its
dependencies optimised as `Cargo.toml` says): [`worker.sh`](0030-measure/worker.sh)` <clv> <dir>/*.mmd`
gives each diagram to `clv --worker mermaid` as one request, its caps armed. The `gen within` set:
at most 0.23 s of CPU (300 classes of 50 members, 0.21 s) and 232 MiB at peak (15,000 nodes, 247 MiB
resident); `gen worst`'s 15,000 labelled nodes and the 100-deep subgraphs refused by merman (its
model text, its layout's work). Twenty-four more hostile diagrams, made for the kinds the first sets
left out (a packet of 10⁹ bits, a block of 10⁸ spaces or columns, architecture, sankey, mindmap,
state and timeline diagrams at 15,000 items, a wrapping width of 1, spacing of 10¹⁵, class and ER
diagrams at the limits), were each drawn under 0.32 s and 170 MiB or refused by merman's resource
limits in milliseconds (`max_layout_work_units`, `max_model_items`, `max_svg_bytes`, which is 24 MiB
in its `Constrained` profile, a block's 200,000 spaces, a packet's 10,000 rows). So no diagram found
reaches the caps: they stand behind merman's limits, and clv's tests kill the worker with fakes
(`src/source/mermaid.rs`), the kernel's and the allocator's kills being `worker.rs`'s own tests. A
diagram merman draws may still be refused by the SVG backend's limits: 32 nodes each linked to every
other (496 edges, 1.2 MB of SVG) comes to more than its 128 MiB once its arrowheads are repeated; 24
(276 edges) is drawn.

## Alternatives considered

- **mermaid-rs-renderer**: fast and small, but it measures text with the system's fonts and writes
  copies of them into the reader's home directory (`~/.cache/mmdr/font-cache`) when a diagram is
  opened; its "fast" metrics avoid that, at the cost of its boxes. It brings a second fontdb, has no
  limits of its own, draws in a look of its own, and refuses valid diagrams (a class's annotation).
- **ariel-rs**: the smallest and fastest on ordinary diagrams, its font built in, but it has no
  limits: garbage is drawn, a 10,000-node chain aborts the process, a complete graph of 80 nodes
  runs past a minute and a gigabyte. Its layout differs from mermaid.js's where it matters (a node
  outside its subgraph, a self-loop on the wrong state), and it refuses front matter with `init`.
- **merman's default features**: ELK's routing is mermaid.js 12's own, but ELK is EPL-2.0, whose
  notices and source provenance every binary would have to carry; Cytoscape's layout differs only
  in how edges bend.
- **mermaid.js in a JavaScript engine** (rquickjs, boa, deno_core): Rust only rules it out, and
  mermaid.js measures text through a browser's DOM, which no engine alone gives.
- **merman in the viewer's process, its own limits and deadline alone**: they stopped everything
  tried, but a crate's limits are its own claims; a stack overflow or an allocation refused would
  take the viewer with it. The worker costs some 40 ms once.

## Consequences

- **Positive:** diagrams look as mermaid.js 12 draws them, every kind it parses; most hostile input
  is refused by merman itself, in milliseconds, with a message, before the worker's caps are
  reached; `.mmd` is told by merman's own list of headers.
- **Accepted:**
  - A build three minutes longer from nothing, and 25 more crates.
  - Edges bend as Cytoscape bends them, not as ELK does in mermaid.js 12.
  - Diagrams over 500 edges, 16,000 items or 128 levels are refused, as mermaid.js refuses past 500
    edges.
  - Labels measured by merman's metrics, drawn in the system's font: a long label may run past its
    box, as in mermaid.js.
  - A user journey's labels and a kanban board's cards are `<foreignObject>`, so their tiles in a
    list show them not.
- **Follow-ups:** [Spec-0008](../specs/0008-show-mermaid-diagrams.md) lifted to Ready with these
  numbers; merman's pin moved as clv's other pinned crates are, its caps measured again with
  [`0030-measure/`](0030-measure/).
- **Revisit when:** merman changes its limits or its layout's cost; ELK comes under a licence clv
  can carry without notices; a diagram kind clv's readers need is one merman doesn't parse.
