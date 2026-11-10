# Spec-0004: Draw an SVG's HTML labels

- **Status:** Implementing
- **Date:** 2026-11-09
- **Related:** [ADR-0002](../adr/0002-render-svg-with-resvg.md) (resvg draws the drawing);
  [ADR-0003](../adr/0003-bound-what-a-drawing-costs-before-usvg-and-resvg-spend-it.md) (what a
  drawing may cost, bounded before it is spent, which the labels join);
  [ADR-0012](../adr/0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md) (HTML's cost
  bounded by its marks and text, measured the same way);
  [Spec-0003](0003-lay-markdown-and-html-out-to-the-window.md) (why HTML documents through Blitz are
  another question); [SVG drawings](../usage.md#svg-drawings), [Limits](../usage.md#limits),
  [How each opens](../architecture.md#how-each-opens-and-what-it-keeps); [ADR-0025](../adr/0025-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md), written by
  task 6 and linked then (Blitz in a worker process of clv's own, capped on CPU time and heap,
  which replaces ADR-0024's estimate and thread). It delivers no Plans bullet, and adds two: HTML documents through Blitz,
  weighed; and a stylesheet's parse bounded, by a cap on a `<style>`'s size before usvg or by
  usvg in the worker.

## Summary

tldraw, draw.io and Mermaid put a drawing's labels in `<foreignObject>`: HTML inside the SVG.
resvg draws no HTML, so clv shows such a drawing without its text (tldraw, Mermaid) or with
draw.io's fallback in its place ("[Not supported by viewer]", "Text is not SVG - cannot display").
This spec draws them: each `<foreignObject>` a browser would draw is laid out and painted by
[Blitz](https://github.com/DioxusLabs/blitz), an HTML and CSS engine in Rust on Servo's style
system, at the scale it is shown, placed by the drawing's transforms, and laid **over** what resvg
draws; the fallbacks a browser skips are skipped. It is a new feature, `svg-html`, on by default:
the SVG backend still builds without it.

Blitz runs in a **worker process**: clv started again as `clv --worker svg-html`, its stdin and
stdout pipes, nothing it writes reaching the terminal. The worker caps its own CPU time and heap
and dies past either; the viewer kills it past a wall-clock backstop. What a drawing's labels
may cost before they reach the worker (how many, their markup, their text, their styles, what
they fetch) is bounded as before, cheaply, in the viewer. Nothing Blitz, Stylo, Parley or
vello_cpu does on a drawing's bytes runs in the viewer's process. A label left out (past a
limit, or in a build without the feature) is counted in a warning; a worker killed leaves all of
that drawing's labels out, with a warning, and the drawing drawn.

It makes finished code of the spike on the branch `spike/blitz-foreignobject`. Tasks 1 to 4
built it in-process, bounded by an estimate of Blitz's painting; three reviews each found CSS
the estimate missed (a `text-decoration-inset` of `-1e20px` loops for ever; `content` strings
copied once an element; clip paths; a shared decoration painted once a label; a dotted border a
million pixels long; a WOFF whose tables lie). Task 5 moves Blitz into the worker and deletes
the estimate; each of those cases is a worker killed, or a font refused, and tested as such.

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
- Text is drawn with the system's fonts, looked for once a drawing, in its worker, and those the
  drawing embeds as `data:` URLs (WOFF, WOFF2, TTF, OTF, as tldraw embeds its own), which stay the
  drawing's own.
- Pictures in a label, `data:` URLs of a PNG, JPEG, GIF or WebP, told by their bytes rather than
  their MIME type, are drawn. `svg-html` brings their decoders itself, whether or not the image
  backend is built. An SVG in a label is left out, so nothing reaches usvg around ADR-0003's bounds;
  so are a stylesheet or a document by a `data:` URL (`@import`, `<iframe>`, `<link>`).

**In a list's tiles**

- A list's tiles show drawings without their HTML labels: a tile's render starts no worker. Its
  `<switch>` fallbacks are still cut (in the viewer, before usvg, with no Blitz), so a draw.io
  tile shows no "Text is not SVG - cannot display" or "[Not supported by viewer]", and it gives no
  warning of labels not drawn. `c` on a tile copies the tile's picture, without labels.
- A drawing opened, from a list too, and a drawing printed (a list's printed too) draw their
  labels as before.
- The viewer says which renders are a list's tiles, in the contract (`src/source.rs`): a field of
  `Request`, `tile`, which the pager sets for a list's tiles and their copies, and nothing else
  does. Not an option to open with: a list's file is opened once ([`Later`](../../src/source/later.rs)
  keeps it open), and going into its tile shows that same source, through a pool of its own. As
  a list's tiles keep the layout they were opened with ([`Effect::Relayout`](../../src/source.rs)),
  a tile is drawn as a tile, by what it is asked for. A drawing's labels are found and written out
  when it is parsed, whatever it is shown as; its worker starts at its first render that isn't a
  tile's.

**What it reads**

- Nothing but `data:` URLs: no network, no files. A link a label doesn't follow is counted in the
  warning a drawing already gives for its links.

**What it costs**

Before the worker, each stage is bounded by limits named in the code and listed in usage's
[Limits](../usage.md#limits), as ADR-0003 bounds the rest of a drawing; the worker is bounded
as a whole, by the kernel and its own allocator
([ADR-0025](../adr/0025-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md)).
The counts are task 1's, measured
([ADR-0024](../adr/0024-draw-an-svgs-html-labels-with-blitz-one-document-a-drawing.md)).

| Stage | What is counted | Limit | Where | Past it |
|---|---|---|---|---|
| The XML | the file, svgz, entities, as now | ADR-0003's | before **every** parse, the fallbacks' included | refused |
| Finding labels | labels drawn; their elements in all, within the drawing's million; their depth, within the drawing's 1,024 levels | 1,024; 100,000 | while the drawing is scanned | the rest left out, with a warning |
| Their markup | markup and wrappers written out; their text | 8 MiB; 1 MiB | while written out | the rest left out, with a warning |
| Their nesting | levels from a `<foreignObject>` down, counted by the viewer's writer itself: the drawing's scan skips some elements (`<metadata>`, `<defs>`) by name, so its 1,024 levels don't reach everything a label holds | 1,024 | while written out, in the viewer | that label left out, with a warning |
| Styles | rules times the labels' elements, as ADR-0003 counts them | 50 million, with the drawing's | before the worker | all labels left out, with a warning |
| Fonts | each `data:` font, its size unpacked read from its header before it is unpacked, a WOFF's tables checked against its file (one stored larger than it unpacks, out of the file or over another is as large as any); in all | 32 MiB; 64 MiB | in the worker's net provider | that font refused |
| Pictures | each decoded, from its header; in all | 64 MiB; 256 MiB | in the worker's net provider | that picture refused |
| Requests | `data:` URLs answered | 1,024 | in the worker's net provider | refused |
| Layout | a label's laid-out size | 16,384 CSS pixels a side | in the worker, after layout | that label left out |
| Overflow | what an `overflow: visible` label draws past its box | 4,096 CSS pixels each way | while written out | cut |
| The worker's time | CPU seconds laying the labels out and recording them; then each render's | 10 s; 5 s. Wall clock three times each | the kernel (`RLIMIT_CPU`, the worker arms it before each request); the viewer | the worker dies: all labels left out, with a warning |
| The worker's memory | its heap, every allocation | 1 GiB | the worker's allocator | the worker aborts: the same |
| Workers | drawings with labels being shown at once | 6 | before a worker starts | that drawing's labels left out, with a warning |
| The pipe | a request's sending and its answer, under one deadline; replies held in the viewer | the request's wall clock; one reply | the viewer, from outside the blocked call | the worker killed: all labels left out, with a warning |
| Painting | each label's part of the page, twice (painted, then drawn over the page), and the part's pixels as work | in the page's estimate: 1 GiB held, 4,000 million pixels | before the page renders | the page fails with why, as now |

- A drawing's labels cost about one document, not one each: their styles parsed and their fonts
  loaded once, in one worker a drawing, alive while the drawing is shown.
- At most six workers live at once, the worst case 6 GiB of heap, a backstop: since tiles start
  none, a worker is a drawing opened or printed, and the pager shows one drawing at a time (its
  level's), the printer prints one at a time. The worst case is reached only by six drawings with
  labels shown other than as tiles at once, which neither does. Past six, a drawing's labels are
  left out, with a warning, rather than wait.
- One deadline covers a request, its sending and its answer. The request is written on a thread
  of its own while the caller waits with the deadline; at the deadline the viewer kills the
  worker, which ends a blocked write (the pipe broken) and a blocked read (its end). So a worker
  that stops reading, and fills the pipe while the viewer writes a large request, is killed at
  the deadline, without using CPU and so out of `RLIMIT_CPU`'s reach, and the render isn't hung.
- The viewer holds at most one reply: its reader reads a frame only when a reply is awaited,
  handing it over through a channel of one, and a frame that isn't the answer to the request
  outstanding (its kind wrong, or a second one) kills the worker. What a worker writes unasked
  waits in the pipe's own buffer, which the kernel bounds.
- A worker that dies, for whatever reason (killed, out of memory, a panic, a crash, an answer
  that isn't one), isn't started again for that drawing: its labels are left out until the
  drawing is opened again, with a warning saying which. The drawing is drawn.
- Everything the viewer reads from a worker is untrusted: each frame's kind and length checked,
  pixels exactly the size asked for, counts no larger than what was sent; a reason is a code,
  and the viewer's own words say it.
- The worker's cost to start: about 5 ms to spawn, about 35 ms to find the system's fonts, once a
  drawing with labels.

**Builds**

- `svg-html` needs `svg`. A build with `svg` alone draws as clv does now, and warns how many labels
  it didn't draw: "N HTML labels (`<foreignObject>`) not drawn".
- `--help`'s SVG row and README's credit Blitz, the name given by `src/source/svg/html.rs` through
  the backend's `credits`.
- Blitz and what it paints with (the names `blitz`, `anyrender`, `kurbo`, `stylo`, `parley`,
  `taffy`, `wuff`) are named only in `src/source/svg/html.rs`, `tests/it/resvg.rs` and
  `Cargo.toml`, checked by a test. Not `vello`, which pdfcraft's pin names.
- The Blitz crates stay pinned `=0.3.0-beta.4`, for their API: no limit follows blitz-paint's
  internals now, so no test guards their versions (task 3's version guard is deleted with the
  estimate).
- `cap`, the worker's allocator, is named only in `src/source/worker.rs` and `Cargo.toml`.

**Docs**

- Usage's SVG drawings, its Limits and the known differences above, and Blitz's found so far:
  inline content straight in a `display: table` element isn't drawn (5 of a Mermaid flowchart's
  24 labels blank; worked around in the worker since 2026-11-10, with a canary test for the day
  Blitz draws it: the Changelog). And clv's own: a `<switch>` whose child taken or fallback an entity makes
  keeps its fallback, drawn as usvg draws it. Architecture's backends table and How each opens. `CONTRIBUTING.md`'s library
  rule, its count of features and its gates. `CHANGELOG.md`. An ADR,
  [ADR-0024](../adr/0024-draw-an-svgs-html-labels-with-blitz-one-document-a-drawing.md) (the next free number), why Blitz and how its cost is bounded, with
  what reproduces its evidence; and ADR-0025, the worker.
- Usage gains `## Known issues`, after Limits: usvg's stylesheet parser (simplecss) takes time as
  the square of a stylesheet's size, once to open and once to render, labels or not (a 3 MiB
  stylesheet some 8 s to open; a 600 KB declaration 45 s in a debug build). README's Plans bullet
  links it.

## Out of scope

- A `<foreignObject>` reached through `<use>`: usvg copies the `<use>`, not the label. Left out
  and counted in the warning.
- Clip paths and masks on a label's ancestors, and a nested `<svg>`'s clip: a label is drawn
  unclipped.
- A label among the shapes in their paint order, or inside their groups' opacity: drawn over the
  drawing.
- A stylesheet's `display: none` on a label's ancestor.
- Working around Blitz's `display: table` bug in clv. *Reversed 2026-11-10 by the owner:* worked
  around in the worker, with a canary test for the day Blitz fixes it.
- HTML and Markdown documents through Blitz rather than wordcraft's importer. In README's Plans.
- What a label would fetch: web fonts by URL, stylesheets, `<iframe>`s; Blitz runs no scripts.
- Selecting or copying a label's text.
- Moving Blitz past `=0.3.0-beta.4`.
- **usvg and resvg in the worker too**, which would bound a stylesheet's parse (simplecss takes
  time as the square of a stylesheet's size: some 8 s for 3 MiB). A design of its own: the
  whole page's pixels each render, ADR-0003's estimate rethought. `worker.rs` serves it when it
  comes; a bound on a `<style>`'s size before usvg is the cheaper first step. In README's Plans.
- Starting a dead worker again for the same drawing; a worker shared between drawings; a worker
  answering two requests at once.
- Rendering labels while the worker is kept alive across drawings (a worker outlives only its
  `Drawn`).

## Approach

**Files:**

- `src/source/worker.rs` — new: a worker process of clv's own, for any backend. The viewer's
  side: `spawn` (`current_exe()`, `--worker <name>`, stdin and stdout piped, stderr null, the
  environment cleared but for `HOME`, `XDG_*` and `FONTCONFIG_*`, which fonts need), frames
  (kind, length, payload; each reply's length capped), a writer thread a request and a reader
  reading only when a reply is awaited (a channel of one), one deadline for both enforced by
  killing the worker, kill and reap, the live count. The worker's side: `serve` (the heap limit set, `RLIMIT_CPU` armed before
  each request, the frame loop). The `Worker` trait. The process's allocator, `cap`, with its
  limit `usize::MAX` in the viewer. Names no library but `cap` and `rustix`.
- `src/source.rs` — `WORKERS`, this build's workers, one line a feature, beside `BACKENDS`;
  `the_viewer_names_no_backend` lets `main.rs` alone name it. `Request::tile`: the render is a
  list's tile.
- `src/view/pager.rs` — `tile` set on the requests of a list's level (its tiles, and a tile's
  copy); names no backend.
- `src/source/picture.rs`, `src/source/wordcraft.rs` — a document's page that is a tile draws
  the pictures it refers to as tiles (`Promise::draw` takes `tile` from `drawn_pictures`'s
  request), so a Markdown or HTML tile referring to a labelled drawing starts no worker; a
  picture drawn as a tile is drawn again, and replaced, when a page wants it.
- `src/main.rs` — `--worker <name>` before `view::run`, handed to `source::serve` with `WORKERS`.
- `src/source/svg.rs` — finds the labels and places them, as task 2 built it; `prepared` hands
  them to `html::prepare` and tells the warnings; `Drawn::render` counts each label's part of the
  page in the estimate. Keeps a nesting limit, 1,024 levels from a `<foreignObject>`, counted by
  `written` itself (no longer 64, which was Blitz's stack's). Loses the commands' share of the
  estimate. Keeps
  `LOADING` (no element that loads a stylesheet or a document reaches the worker; a label's
  `<style>` joins the drawing's). Names no Blitz crate.
- `src/source/svg/html.rs` — the only file naming Blitz and what it paints with. The worker
  (`Labels`, a `Worker`): one document a drawing, the slots, the net provider (`data:` fonts and
  pictures only, each sized from its header), each label recorded once, the replays into a part
  of a page. The viewer's side (`Painted`): `prepare` through `worker::spawn("svg-html")`, the
  regions, `paint` asking for a part's pixels. Loses `paint_work`, `Recorder`, the 8 MiB
  thread, `catch_unwind`, `cost`, `system_fonts`. Keeps `without_imports` (no stylesheet is ever
  fetched, a rule rather than a bound) and gains the WOFF table check in `font_size`. `CREDIT`
  stays.
- `Cargo.toml` — the Blitz crates pinned `=0.3.0-beta.4` without their default features, with
  `woff` and `system-fonts`; `anyrender` 0.14, `anyrender_vello_cpu` 0.18, `kurbo` 0.13,
  `svgtypes` 0.16, each with its comment; `cap` 0.1.2, the worker's allocator, with its comment;
  `rustix` with `time` for the CPU used; `stylo` `=0.22.0` for `without_imports` alone, its
  comment said so. `peniko` out. `svg-html = ["svg", "dep:image", …]`, among the defaults.
- `tests/it/resvg.rs` — the tests; `nothing_but_svg_html_names_blitz` without `peniko`; the
  version guard out.
- `.github/workflows/ci.yml` — as task 2 left it.
- `docs/adr/0025-…`, `docs/adr/0024-…` (its status), `docs/adr/README.md`, `docs/usage.md`,
  `docs/architecture.md`, `CHANGELOG.md`, `CONTRIBUTING.md`, `README.md`, and `CLAUDE.md`'s
  Layout (`worker.rs`).

**Patterns to follow:**

- Limits as named constants with their reason beside them, as ADR-0003's are in `svg.rs`.
- Warnings through `Notes`, as `left_out_text` gives them.
- The naming test as `nothing_but_this_backend_names_resvg` is.
- Fixtures made by the tests, as `svg(width, height, body)` in `tests/it/resvg.rs` makes them, and
  told by coloured boxes (`background`, `border`), which need no font, so a machine without fonts
  passes. Text is looked at by hand: a font in `tests/data` would not be synthetic.

**Decided:**

- **resvg still draws the drawing; Blitz only its labels.** ADR-0024 says why each alternative
  was left: Blitz for the whole SVG, Gecko, Servo, wordcraft's HTML importer.
- **One Blitz document a drawing,** as task 1 measured it (ADR-0024): each label in a slot placed
  by `transform: translate()`, 8 CSS pixels apart; its styles resolved twice, for the pictures a
  `background-image` asks for; laid out once; each label painted once at scale 1 into an
  anyrender `Scene` of its own, kept; the document dropped. Each render replays a label's scene
  through its transform into the part of the page it lands on. Unchanged, inside the worker.
- **Blitz in a worker process** ([ADR-0025](../adr/0025-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md)):
  clv itself, `--worker svg-html` (hidden from `--help`), stdin and stdout pipes, stderr null, a
  minimal environment. In-process, neither time nor memory can be capped: a thread can't be
  stopped, Blitz has no cancellation, and a refused allocation aborts the viewer. The worker caps
  itself where the kernel and the allocator enforce it: `RLIMIT_CPU` (rustix, safe; enforced on
  macOS and Linux, the soft limit raised again before each request, measured), and the heap
  through the `cap` allocator, exact to the byte and portable (`RLIMIT_AS`, `RLIMIT_DATA` and
  `RLIMIT_RSS` are refused by macOS, measured; a polled footprint overshoots by what a poll
  interval allocates, about 150 MB). The viewer's wall clock is the backstop, three times the
  CPU budget, for a worker that isn't running. No estimate of Blitz's painting: three reviews
  found CSS it missed, and the next would too.
- **A worker a drawing,** owned by the `Drawn` the render's first parse makes (a pool's, or the
  printer's renderer's), killed when it goes; none for a list's tiles. At most six alive, a
  backstop, the worst case 6 GiB of heap; past six, a drawing's labels
  are left out, with a warning, rather than wait (a wait on a render thread for a slot another
  `Drawn` holds could wait for ever). A pool's threads share the worker, one request at a time:
  two scales of one drawing replay in turn, not at once. A render no longer wanted is dropped
  when it finishes, as `source/threads.rs` does; the worker is killed only past its limits.
- **A dead worker isn't started again** for that drawing: a deterministic death would be repeated
  every render. A new `Drawn` (the drawing opened again) starts afresh.
- **What crosses the pipe** is pixels, not scenes: the worker keeps the scenes and answers each
  render with a part's RGBA (64 MiB for a 4,096-pixel page's worth in 21 ms, measured; a real
  label's part is kilobytes). anyrender's scenes serialize (its `serde` feature), but a glyph run
  carries its whole font file and a fill its decoded picture, and the viewer would replay them
  with vello_cpu and skrifa in-process, which is what the worker is for.
- **The cheap bounds stay before the worker** (labels, elements, markup, text, styles, what is
  fetched and how large): exact, and they spare the worker for a real drawing with one bad
  label. Deleted with the estimate: the paint work, the recorder's budget, the 24,576-pixel
  element bound, the 8 MiB thread, `catch_unwind`, the version guard. The nesting limit becomes
  1,024 levels, counted by the viewer's writer (the drawing's scan skips `<metadata>` and `<defs>`
  by name, so it doesn't bound a label's depth, and `written` recurses in the viewer); the worker
  lays out on a thread of 64 MiB, which holds 1,024 nested grids unoptimized. Round 3's generated-text count, never committed, isn't taken
  up: what styles write is heap and CPU, capped. Kept, because they are rules and not bounds: no
  stylesheet or document is ever fetched (`LOADING` before the worker, `without_imports` in it,
  so Stylo stays), and a WOFF's tables are checked against its file before wuff unpacks them,
  which refuses one font instead of losing all the labels to the heap cap.
- **Panics**: a worker's kills it, and its labels are left out with a warning ("Blitz failed on
  them": its stderr is null, so its message is lost); the viewer's hook stays for the viewer's
  own.
- **Tests run the real worker** through the binary (`tests/it/`); the viewer's unit tests that
  draw labels run the worker's code on a thread through `std::io::pipe()`s, the same frames, no
  caps (the test harness can't be re-executed as `clv --worker`). `worker.rs`'s own tests use
  `sleep`, `sh` and `yes` as workers; its CPU cap's test runs its own test binary again as the
  worker (the harness's `--exact` filter and an environment variable, as ADR-0024's stack test
  ran a child), `serve` with a test worker that spins without allocating and a budget of a
  second: killed by `SIGXCPU` at its budget, and, with the arming removed, by the wall clock
  instead, which the test tells apart.
- **Branch:** `spec-0004` from `main`; task 5 on it after task 4.
- **On by default,** though it adds about 11 MB to the binary, and Blitz is a beta: a drawing
  without labels never starts a worker.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

None: task 1 settled both (one document holds; the limits are its measurements).

## Tasks

One checkbox each: a session's work, and a commit that builds and passes on its own.

- [x] **Measured** — on the spike's branch, the open design tried and measured: one document a
  drawing with a fragment a label, against a document a label; the stack test in a child process;
  each stage's hostile drawing (a label's text, styles times labels, deep nesting, a font declaring
  a huge size unpacked, many pictures, a 1-pixel background tile over the largest box, many
  commands). The fixtures made by a generator, synthetic: N labels sharing a 400 KB `@font-face`,
  1,024 under 3 MiB of styles. The ADR written, Proposed, with the numbers and commands; this
  spec's table set to the numbers, Decided to the design that held. *Files:* `docs/adr/0024-…`,
  `docs/adr/README.md`, this spec. *Verify:* the ADR's commands, under `/usr/bin/time -l` on a release
  build, give its numbers again.
- [x] **Labels found, behind `svg-html`, off by default** — the dependencies and the feature; the
  XML bounded before every parse; the eligibility rule and the fallbacks removed; the viewport
  stack; markup written from the tree, in its context, and its budgets; the warning without the
  feature; the naming test and the version guard; CI. *Files:* `Cargo.toml`, `src/source/svg.rs`,
  `src/source/svg/html.rs` (a stub painter), `tests/it/resvg.rs`, `.github/workflows/ci.yml`,
  `CONTRIBUTING.md`. *Verify:* `cargo test -j 2 --test it resvg` with `--features svg-html` and with
  `--no-default-features --features pdf,svg`; tests for each eligibility case, a fallback after a
  `<foreignObject>` and after a `requiredFeatures` child, entities in a fallback refused before it is
  parsed, a root-declared XHTML prefix, an internal entity in a label.
- [x] **Labels drawn, bounded, on by default** — `Painter`, the net provider and its budgets, the
  document, fragments and replays as task 1 decided; painting only what lands on the page; opacity;
  overflow; hints; pictures; the stage limits and their warnings; the paint work estimate and the
  recorder; `Estimate`; the nesting limit; `svg-html` among the defaults. *Files:* `src/source/svg.rs`, `src/source/svg/html.rs`,
  `Cargo.toml`, `tests/it/resvg.rs`. *Verify:* `cargo test -j 2 --test it resvg`; boxes for: placed
  through nested `<svg>`s and a `preserveAspectRatio`, a percentage label, a child several boxes
  outside an `overflow: visible` label, a label in a window over 6,000 pixels wide, a group's
  opacity, a shape overlapping a label (drawn over it), a rule on the `<foreignObject>`'s class, an
  inherited `color`, labels of different sizes with percentage children, a `data:` PNG, each hint
  overridden by a rule and a malformed one; and each stage's limit, with its warning: the paint
  work's for each multiplier at its limit and past it (shadows, layers, `space` tiles), each seen
  failing without the estimate, and the recorder's past 200,000 commands; shadows on the
  document's `html` and `body`, counted once a label; dotted borders at the paint work's limit and
  past it; a label holding an element past 24,576 pixels (a dotted one a million wide), left out.
- [x] **The docs** — usage's SVG drawings, Limits and known differences; architecture; CHANGELOG;
  README's credits; `CLAUDE.md`; the ADR Accepted. *Files:* `docs/usage.md`, `docs/architecture.md`,
  `CHANGELOG.md`, `README.md`, `CLAUDE.md`, `docs/adr/0024-…`. *Verify:* `cargo test -j 2 --test docs`,
  and the manual checks in Done.

- [x] **Blitz in a worker, the estimate deleted** — `source/worker.rs` (spawn, frames, caps,
  kill, count; serve, the heap limit, `RLIMIT_CPU` armed a request; the `Worker` trait; the
  allocator), `WORKERS` and `--worker`; `html.rs` split into the worker (`Labels`) and `Painted`
  over the pipe, pixels back; the deletions (`paint_work` and its helpers, `Recorder`, the 8 MiB
  thread, `catch_unwind`, `cost`, `system_fonts`, `peniko`, the version guard); the nesting limit
  at 1,024, in the writer; the WOFF table check in `font_size`; the page's estimate counting parts only; the
  in-process seam for the unit tests; `cap` and `rustix`'s `time` in `Cargo.toml`; `tile`
  carried to the pictures a document's page refers to (`picture.rs`'s `Promise::draw`, from
  `wordcraft.rs`'s `drawn_pictures`), a picture drawn for a tile drawn again for a page, so a
  Markdown or HTML tile that refers to a labelled drawing starts no worker either. *Files:*
  `src/source/worker.rs`, `src/source.rs`, `src/main.rs`, `src/view/pager.rs`, `src/source/svg.rs`,
  `src/source/svg/html.rs`, `src/source/picture.rs`, `src/source/wordcraft.rs`, `Cargo.toml`,
  `tests/it/resvg.rs`, `tests/it/view_pager.rs`, `tests/it/view.rs`. *Verify:* `cargo test -j 2
  --test it resvg` with `--features svg-html` and with `--no-default-features --features pdf,svg`;
  a Markdown page referring to a labelled drawing rendered as a tile shows the drawing without
  its label, and as a page with it (`a_tiles_pictures_are_drawn_as_tiles`), and a picture drawn
  as a tile is drawn again for a page (`a_picture_drawn_as_a_tile_is_drawn_again_for_a_page`);
  `cap` named in `worker.rs` alone (`nothing_but_the_worker_names_cap`);
  `worker.rs`'s tests with `--no-default-features`: a `sleep` killed on the wall clock and
  reaped, `sh -c 'exit 3'` and `sh -c 'kill -SEGV $$'` told apart, `yes` refused past the length
  cap, an unknown kind refused, a `Pixels` of the wrong length refused, the seventh worker
  refused; a worker that never reads, sent a request larger than the pipe holds, killed at the
  deadline and the render returning; a worker writing valid frames without end, killed at its
  first unasked frame, the viewer holding at most one; the CPU cap, a spinning worker that
  allocates nothing killed by `SIGXCPU`, failing with only the arming removed. Through the
  binary, round 3's cases: the `text-decoration-inset: -1e20px` hang killed by a cap (its memory
  grows without the recorder, so the heap's comes first: a regression for either cap), with its
  warning (`2px` drawn); a label 20,000 levels deep under an XHTML `<metadata>` left out with its
  warning, no crash; a 2,000,000-pixel dotted border, and a 64 KiB `content`
  on 99,000 elements (100,000 are past their limit before the worker), each aborted at the heap
  cap with its warning; a WOFF whose table is
  stored larger than it unpacks refused alone, the label drawn without it
  (`a_woffs_inconsistent_tables_are_refused`); 10,000 elements clipped by a 1,000-point polygon,
  and a `body::before` wavy underline under 1,000 labels, each drawn or left out within the wall
  budget (measured in the ADR); a `<link>` and an `@import` of font-shaped CSS not applied, as
  committed; a directory of 12 drawings printed with every label; in the pager, a list of two
  labelled drawings starting no worker and its tiles warning of nothing, a draw.io tile with no
  fallback text, and going into one drawing its labels (`Request::tile` checked through the
  viewer, `the_viewer_names_no_backend` still passing); a label nested 1,000 deep
  drawn; the placed, overflow, opacity, rules, picture, hints and links tests as before; each new
  test seen failing with its cap removed (a budget of `usize::MAX`, a wall of an hour; the table
  check with its ranges unchecked).
- [x] **The docs** — ADR-0025 written (Accepted), with the measurements (what macOS enforces,
  spawn, fonts, the pipe) and the alternatives (a footprint watchdog, `RLIMIT_AS`, scenes over
  the pipe, a process a render, the allowlist), and linked where this spec names it; ADR-0024's status "Accepted; its estimate and its
  thread superseded by ADR-0025"; usage's Limits (the worker's rows in, the estimate's out), SVG
  drawings ("in a process of its own"; a list's tiles show drawings without their HTML labels,
  and opening one shows them; `c` on a tile copies it without them) and `## Known issues` (the
  stylesheet case); architecture's
  Files (`worker.rs`), How each opens, Layers (a worker is an extension's; `main.rs` names
  `WORKERS`); CHANGELOG; `CONTRIBUTING.md`'s library rule (`cap` in `worker.rs` alone) and tests;
  README's Plans (a stylesheet's parse bounded, linking `docs/usage.md#known-issues`);
  `CLAUDE.md`'s Layout. *Files:* `docs/adr/0025-…`, `docs/adr/0024-…`, `docs/adr/README.md`,
  `docs/usage.md`, `docs/architecture.md`, `CHANGELOG.md`, `CONTRIBUTING.md`, `README.md`,
  `CLAUDE.md`. *Verify:* `cargo test -j 2 --test docs`, and the manual checks in Done.

## Done

- [x] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass, with
  `--features svg-html` alone and `--no-default-features --features pdf,svg` too.
- [x] Tests added, each seen failing without its change, the change reverted named beside it.
- [x] **Cost**, on a release build under `/usr/bin/time -l`, on the machine the ADR names (an Apple
  M3 Max, as ADR-0003's): the 1,024 labels under 3 MiB of styles drawn, none left out, the labels'
  work (from the request to the worker to its answer, measured about 0.6 s) in at most 3 s, and
  500 MB (viewer and worker together: `-l` on the viewer, the worker's maximum resident set from
  `ps` while it lives); the whole open takes some 8 s, usvg's stylesheet parse twice, a known
  issue in usage. 50 labels sharing a 400 KB font drawn in at most 1 s; the hang killed within
  its budget (by whichever cap it reaches first) and the bomb within a second, each with its
  warning; a drawing with one label shown
  within 100 ms more than the same drawing without it.
- [x] A worker's death leaves no process behind: after the hang and the bomb, `pgrep -f
  'clv --worker'` finds none; after `clv` is killed with SIGKILL while a worker lives, the worker
  exits.
- [ ] Nothing a worker writes reaches the terminal: in kitty, a drawing whose label panics Blitz
  (none known; a worker made to panic by hand while testing) shows the warning and nothing
  else.
- [ ] Manual, in kitty with a window over 6,000 pixels wide, and again zoomed in with `+`: a tldraw
  drawing shows every label in its own font; a draw.io 21 drawing (labels `100%` wide, set in
  `<font face>`) shows them as Chrome does, with no "Text is not SVG" line; a Mermaid flowchart
  shows its labels but those in `display: table`. Built with `--no-default-features --features svg`,
  the same tldraw drawing says how many labels it didn't draw.
- [x] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- 2026-11-09 — Ready (spec-new).
- 2026-11-09 — Codex's review: the limits a stage each, measured first (task 1); one eligibility
  rule; the XML bounded before every parse; the viewport stack; markup written from the tree, in
  its context; hints of no specificity; labels drawn over the drawing, opacity approximated; a
  shared failure leaves out all labels; decoders brought by `svg-html`; the feature default only
  once bounded; ADR-0024; the 32 MiB fallback's omissions and the 50-label fixture made honest.
- 2026-11-09 — Implementing ([notes](0004-draw-an-svgs-html-labels-impl-20261109_1432.md)).
- 2026-11-09 — Task 1, measured (ADR-0024, its tool in `docs/adr/0024-measure/`): one document a
  drawing holds; the labels' elements held to 100,000, their commands to 200,000, their overflow
  to 4,096 CSS pixels; the nesting limit 64, with Blitz on a thread of its own with an 8 MiB stack,
  since a debug build overflows 2 MiB at 58 nested grids.
- 2026-11-09 — Codex's review of task 1: commands counted as they are painted, a recorder keeping
  none past 200,000, and the paint work estimated after layout, before painting, past 20 million
  all labels left out (ADR-0024: counting alone bounds memory, not time); a style resolved twice
  for the pictures it asks for; the styles fixture of fewer, larger rules, within 50 million;
  `stylo` a direct dependency from task 3, approved by the owner.
- 2026-11-09 — Codex's review of task 2: labels through `<use>`s counted by the one rule, nested
  uses too, in one pass; a fallback an entity makes kept (a known difference); the child taken
  loses its `requiredFeatures`; markup refused before it is written past its budget.
- 2026-11-09 — Codex's review, round 2: the estimate counts the document's `html` and `body` once a
  label, a dotted border's dots, a decoration's pixels of line and a gradient's stops; an element
  or a line of text past 24,576 pixels leaves its label out; a percentage of a drawing sized by what
  it draws is of that size.
- 2026-11-09 — Task 3: the stack measured again as clv's tests build it (dependencies optimized):
  392 nested grids hold on 2 MiB, so the 8 MiB thread is margin for an unoptimized build, not what
  clv's tests need; kept. `peniko` a direct dependency, for the recorder, approved.
- 2026-11-09 — Task 4, the docs: usage's SVG drawings (which labels, how they look, where they differ
  from a browser) and Limits, architecture, CHANGELOG, README's credit, `CLAUDE.md`; ADR-0024
  Accepted. README's Plans gains bounding what an SVG's stylesheets take to parse, at the owner's
  word (a long declaration takes some 45 seconds, labels or not).
- 2026-11-09 — Codex's review, round 3: no stylesheet or document fetched (`LOADING`,
  `without_imports`), a label's own opacity applied once, the styles fixture of rules some 750
  bytes; its estimate fixes set aside.
- 2026-11-09 — Worker process, owner's decision, after three reviews of the estimate: Blitz into a
  worker process, capped on CPU time (10 s to lay out, 5 s a render, three times that on the
  wall) and heap (1 GiB), killed past either, at most four alive; the estimate, the recorder's
  budget, the 8 MiB thread, `catch_unwind`, the nesting limit and the version guard deleted,
  round 3's generated-text count not taken up; its WOFF table check kept; `peniko` out, `cap` in
  (approved by the owner); ADR-0025. Task 5 and a docs task added; tasks 2 to 4 stand as built.
  The Cost item's 3 s covers the labels' work, the whole open some 8 s (usvg's stylesheet parse,
  a known issue).
- 2026-11-09 — Codex's review of the worker: a nesting limit kept in the viewer's writer, 1,024
  levels, since the drawing's scan skips `<metadata>` and `<defs>`; one deadline for a request's
  sending and its answer, enforced by killing the worker; at most one reply held, an unasked one
  killing the worker; the CPU cap tested by a worker that spins without allocating, the
  `-1e20px` hang a regression for either cap. Six workers, not four, as many as the pager's
  pools (owner's decision): the worst case 6 GiB.
- 2026-11-09 — No labels in a list's tiles, owner's decision: a tile's render (`Request::tile`, set
  by the pager) starts no worker and warns of no labels, its fallbacks still cut; a drawing opened
  or printed draws them. Six workers stay, a backstop no longer reached.
- 2026-11-09 — Task 5, the worker ([notes](0004-draw-an-svgs-html-labels-impl-20261109_1432.md#task-5-blitz-in-a-worker)):
  built; Codex's P2 folded in, `tile` carried to a document's pictures; the labels' links told
  as strings, hashed with the drawing's, since a count said one link twice; the `content` bomb at
  99,000 elements (100,000 are past their limit before the worker); the writer's 1,024 levels
  reachable only under a name the scan skips, plain nesting being the drawing's to refuse first.
- 2026-11-09 — Codex's review of task 5: a worker that ends between requests is seen ending by its
  reader, which takes each frame's first byte as it comes, and reaped at once, its place given up;
  all of a page's labels painted before any is drawn, so a worker gone on one leaves none on the
  page; `--worker`'s error names no name.
- 2026-11-09 — Task 6, the docs: ADR-0025 Accepted, with the measurements on the release build
  (the styles drawing's worker 0.32 s of CPU and 95 MB, the whole open 8 s; the font drawing
  0.10 s; the hang and the bombs killed in 0.2–0.3 s; a label some 30–90 ms over none);
  ADR-0024's status; usage's SVG drawings, Limits and a new Known issues; architecture;
  CHANGELOG; CONTRIBUTING; README's Plans; `CLAUDE.md`. Done's manual checks in kitty are the
  owner's: the tldraw, draw.io and Mermaid drawings, and that nothing a worker writes reaches the
  terminal.
- 2026-11-09 — Codex's review of the branch: the labels' stylesheets found in one walk, as the
  labels are (each `<style>` asked for its ancestors was the square of a nest the scan doesn't
  bound: 100,000 under a `<metadata>`, 11 s in a debug build, before anything is drawn). Its other
  finding, that Blitz's `system-fonts` link fontconfig at build time, which the musl release builds
  can't, is the owner's to decide: the notes have the options.
- 2026-11-09 — Fonts via fontdb, owner's decision: the worker registers the font files fontdb
  finds for resvg into Blitz's collection, mapped by fontique, with the generic families as
  resvg's and each script's fallbacks from a list; Blitz's `system-fonts` off, so fontconfig
  leaves the build and the musl release builds pass (ADR-0025's update).
- 2026-11-10 — Labels in `display: table` drawn, owner's decision: Blitz 0.3.0-beta.4 (and its
  git main) lays out none of a table box's children that aren't table parts, so Mermaid's edge
  labels came out blank; the worker wraps each run of them in the anonymous row and cell CSS gives
  them, once the styles resolve. A repro and issue draft for Blitz's tracker went to the owner;
  `tests/data/blitz-table.html` is its case, and `blitz_still_drops_inline_content_in_a_table`
  fails the day Blitz draws it. CONTRIBUTING gains a "Moving Blitz" checklist.
- 2026-11-10 — Codex's review of the workaround: cells and the content between them share one
  anonymous row, the content alone in anonymous cells, as CSS has it; the moved nodes keep the
  rules that matched them where they were written (the wrappers are `clv-row` and `clv-cell`
  elements, which only a rule on `*` reaches: usage's known difference); and the tree is edited
  in place, each box's children listed anew once, not a scan per node moved.
