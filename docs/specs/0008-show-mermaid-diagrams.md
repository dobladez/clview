# Spec-0008: Show Mermaid diagrams

- **Status:** Implementing
- **Date:** 2026-11-13
- **Related:** [Spec-0009](0009-draw-fenced-blocks-as-pictures.md) (a document's fenced blocks drawn as pictures), which builds on this and on
  its cache of what diagrams were turned into;
  [ADR-0030](../adr/0030-mermaid-diagrams-through-merman-in-a-worker-process.md) (merman, its
  config and the caps: the spike's, Proposed until this ships);
  [ADR-0026](../adr/0026-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md) and
  [ADR-0027](../adr/0027-a-pdfs-text-read-in-a-worker-process-for-its-heap-cap.md) (the workers this
  one follows); [ADR-0013](../adr/0013-a-documents-pictures-are-drawn-by-the-backends.md) (a
  document's pictures); [ADR-0015](../adr/0015-markdown-and-html-on-dark-paper-the-terminals-own-colour.md)
  (pictures on dark paper); [ADR-0019](../adr/0019-a-list-opens-its-files-when-their-tiles-are-wanted.md)
  (a list's files opened when their tiles are wanted); [Backends](../architecture.md#backends);
  [Limits](../usage.md#limits). With [Spec-0009](0009-draw-fenced-blocks-as-pictures.md), it delivers [README's Plans](../../README.md#plans)'
  "Mermaid diagrams, their files and a Markdown `mermaid` block": this spec its files, which takes
  the bullet out only once Spec-0009 is Done too.

## Summary

A Mermaid diagram is text that a browser turns into a drawing: today clv opens no `.mmd` file, since
no backend takes it. This spec adds a backend for Mermaid files, behind a `mermaid` feature: it tells
a diagram by its name (`.mmd`, `.mermaid`) and that it is text, turns it into SVG through merman, a
pure-Rust Mermaid crate (chosen by a time-boxed spike among merman, mermaid-rs-renderer and ariel-rs,
and recorded in [ADR-0030](../adr/0030-mermaid-diagrams-through-merman-in-a-worker-process.md)), and
hands that SVG to whichever backend opens it, the SVG backend, never by name. The crate's work runs
in a worker process of clv's own, capped on CPU time and heap as the SVG labels' is
([ADR-0026](../adr/0026-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md)),
since a diagram is untrusted input and its layout can cost more than its size suggests, and what it
answers is kept, so a diagram is turned into SVG once however often it is opened. A diagram is one
page, drawn as a drawing is, and may be a document's picture (`![](flow.mmd)`). The first half of
Mermaid in Markdown: Spec-0009 brings fenced blocks to it.

The spike settled what this left to it (the crate, the kinds it draws, how it labels, and the numbers
once marked *(spike)*), and the owner approved merman as a dependency, on 2026-11-13.

## Requirements

- **Opens:**
  - `.mermaid` by its name, if it is text.
  - `.mmd` by its name, when its first line past a BOM, blanks, `%%` comments and front matter
    (`---` to `---`) starts with a kind of diagram merman draws (the first word of a label in
    `merman::diagram_header_facts()`: `flowchart`, `graph`, `sequenceDiagram`, `stateDiagram-v2`…,
    47 headers), matched case as Mermaid matches it and followed by a space, a `-`
    (`stateDiagram-v2`, `flowchart-elk`) or the line's end; or is an `%%{init: …}%%` directive.
    `.mmd` is MultiMarkdown's too, which no backend opens now and which stays so. `opens` sees only
    the first 1024 bytes: a `.mmd` whose first such line isn't within them is declined, as one whose
    head is cut inside a character is. The kinds are merman's own list, so the two can't disagree.
  - Not through a pipe (`-`, `/dev/stdin`): a diagram has no signature, as Markdown has none. A named
    pipe that is named `.mmd` is read as any file is, within the cap below.
  - Asked before the PDF backend, as the SVG one is: a diagram's text may hold `%PDF-`.
  - `formats()` lists "Mermaid diagrams" with both extensions, and `credits()` the crate, for
    `--help`; `the_extensions_a_backend_lists_are_the_names_it_opens_by` (`src/source.rs`) gains a
    Mermaid head among its heads, so it tries `.mmd` as it tries the others.
- **Shows:** one page, the diagram as merman draws it to SVG, handed to the first other backend
  in `BACKENDS` that opens that SVG. Its size, sharpness, zoom, `c` and text cells are what that
  backend gives a drawing. Where no backend opens it, the diagram is refused, saying why (never a
  panic, though the feature brings `svg`).
- **Its words, in a tile too:** merman's site config sets `htmlLabels` false, flowcharts' too, and
  lists it among Mermaid's `secure` keys with Mermaid's own (`secure`, `securityLevel`,
  `startOnLoad`, `maxTextSize`, `suppressErrorRendering`, `maxEdges`), so a diagram's `%%{init}%%`
  can't turn it back on: labels are SVG `<text>`, and a list's tile, which starts no HTML labels'
  worker ([ADR-0026](../adr/0026-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md)),
  shows the diagram's words. A user journey keeps `<foreignObject>` for its sections and tasks, as
  mermaid.js draws them: those are the `svg-html` worker's, and its tile shows them not, which usage
  says.
- **A document's picture:** `draws_pictures` is true, so `![a flow](flow.mmd)` in Markdown or HTML
  draws it ([ADR-0013](../adr/0013-a-documents-pictures-are-drawn-by-the-backends.md)).
- **Theme:** Mermaid 12's default, as merman draws it, on white, as a drawing is; on dark paper a
  picture's thin near-black ink comes with white behind it, as any picture's does
  ([ADR-0015](../adr/0015-markdown-and-html-on-dark-paper-the-terminals-own-colour.md)).
- **Turned into SVG once:** a document opens its pictures to size them and again to draw them, and is
  read again for every render, every relayout and every picture let go of and drawn again
  (`picture.rs`, `wordcraft.rs`); a list's tile opens its file when wanted. So what the worker
  answers is kept, **in a cache in `mermaid.rs`**, by the diagram's bytes (a hash to find it, the
  bytes compared to take it): the SVG, or why it failed, so a diagram that killed the worker isn't
  given to it again. At most 64 MiB of SVG and 4,096 diagrams (a typical diagram's SVG is 5 to
  65 KB), those used longest ago let go of first. A failure for want of a worker slot isn't kept: it
  may pass.
- **merman's own limits first:** its `Constrained` resource profile (1 MiB of source, 16,000 model
  items, 1 MiB of model text, 128 levels deep) and Mermaid's 500 edges, which refuse every hostile
  diagram the spike tried in milliseconds, with merman's message; and its `OperationControl` given a
  deadline of 1.5 s, so it stops with its own message before the kernel kills it.
- **The worker:**
  - merman parses, lays out and writes SVG in a worker process (`clv --worker mermaid`, through
    `worker.rs`, listed in `WORKERS`): **2 s of CPU time a diagram, 6 s on the clock** (`worker.rs`'s
    deadline, three times the CPU's, the backstop), **512 MiB of heap** for the process, about ten
    times the CPU and twice the heap of the costliest diagram merman accepts
    ([ADR-0030](../adr/0030-mermaid-diagrams-through-merman-in-a-worker-process.md)). Past a cap, the
    worker is killed and that diagram fails, saying which.
  - **One worker serves the backend's diagrams in turn**: held by the backend behind a lock, so
    diagrams opened at once (a pool's threads, a list's tiles) wait their turn; started when one is
    first wanted; started again for the next after it dies, which fails only the diagram it was on;
    **killed once idle**, 2 seconds after its last answer, by a timer of the backend's own (`Link`
    has none). After three killed in a row, no worker is started for 60 seconds, and a diagram
    wanting one meanwhile fails, saying so: a document of hostile diagrams costs a few caps (at most
    6 s of CPU), not one for each.
  - Six workers alive at once, of every kind, is `worker.rs`'s: where none can be started
    (`TooMany`), the diagram fails, saying so, as the PDF's text reader's does. While it lives, the
    Mermaid worker holds one of the six; the SVG it hands on may start an `svg-html` worker of its
    own, held by that drawing as any drawing's is.
  - A worker of another clv (`HELLO`), one that can't be started, one that dies, or one whose answer
    isn't one (its kind, its length, its request's number), fails the diagram it was asked, and is
    killed and dropped.
- **Bounded, as untrusted input:**
  - A diagram is read at most 1 MiB, merman's own limit, through a reader that stops there, never
    `Input::bytes` whole; past it, refused before anything parses it. Not UTF-8, refused.
  - The worker is given the diagram's text alone, and nothing it does reads another file or the
    network: merman built without the features that read the system's fonts or the clock, nor ELK,
    and nothing on its render path reads a file, the environment or the network (ADR-0030); its
    environment cleared as `worker.rs` clears it.
  - Its SVG answer is held to 64 MiB, the SVG backend's own limit on a file (the largest seen, 22 MB,
    from a 1 MB label), and then to that backend's limits; why it failed, to 256 bytes, cut at a
    character, through `printable`.
- **When:** a diagram is turned into SVG when it is opened, or found in the cache; renders at other
  sizes are the SVG backend's. Listing a list starts no worker: the file is deferred, as a drawing is
  (`Backend::deferred`), and opened when its tile is wanted
  ([ADR-0019](../adr/0019-a-list-opens-its-files-when-their-tiles-are-wanted.md)), which may start the
  worker, unlike an SVG's labels.
- **Errors:** a diagram merman can't read, or refuses past one of its limits, fails with merman's
  message, as above.
- **Its library is its own:** merman is named only in `src/source/mermaid.rs`,
  `tests/it/mermaid.rs` (which checks it) and `Cargo.toml`.

## Out of scope

- **Fenced ` ```mermaid ` blocks in Markdown and HTML**: Spec-0009.
- **A theme of the reader's choosing** (Mermaid's dark theme, a theme for every diagram): it would be
  nice to make it configurable once clv has a config file ([README's Plans](../../README.md#plans)).
  A diagram's own `%%{init: {theme}}%%` applies, as merman honours it.
- **Following dark paper with Mermaid's dark theme**: it would draw again on `d`, and a picture isn't
  told the paper.
- **A diagram through a pipe**: no signature to tell it by.
- **Kinds of diagram merman doesn't draw**, and parity with mermaid.js past what it gives: its
  differences go in usage only where they would mislead.
- **Edges routed as mermaid.js 12 routes them**: that is ELK's, under EPL-2.0, left out; edges bend
  as merman's Cytoscape layout bends them (ADR-0030).
- **Diagrams past merman's limits** (500 edges, 16,000 items, 128 levels): refused, as mermaid.js
  refuses past 500 edges.
- **Math in labels (KaTeX), and icons from the web** (`fa:`, `icon` shapes): nothing is fetched
  ([ADR-0013](../adr/0013-a-documents-pictures-are-drawn-by-the-backends.md)); they show as merman
  leaves them.
- **Searching a diagram's text.**
- **Clickable nodes** (`click`): clv follows no links.
- **A cache across runs**: the cache lives with the process.

## Approach

**Files:**

- `src/source/mermaid.rs` (new) — the backend (`opens`, `formats`, `credits`, `deferred`,
  `draws_pictures`, `open`), its cache, its worker's service (the lock, the idle timer, the pause
  after kills) and both sides of the worker, as `svg/html.rs` is both sides of the labels'. The only
  file naming merman.
- `src/source.rs` — `mermaid` in `BACKENDS`, second, after `svg` and before `tar`, `wordcraft` and
  `pdfcraft`; its worker in `WORKERS`; a Mermaid head in
  `the_extensions_a_backend_lists_are_the_names_it_opens_by`.
- `src/source/worker.rs` — only the `cfg_attr` saying which features start a worker.
- `Cargo.toml` — a `mermaid` feature, on by default, bringing `svg` as `sheet` brings `pdf`; merman
  0.10.0 from crates.io, `default-features = false`, features `all-diagrams`, `svg` and
  `layout-cytoscape` (not `layout-elk`, `system-fonts`, `native-fonts`, `system-clock`,
  `system-timing`, `png`, `pdf`, `ascii`, `math`), with its comment.
- `tests/it/mermaid.rs` (new, listed in `tests/it/main.rs`); fixtures in `tests/data/`
  (`flow.mmd`, `sequence.mermaid`, a MultiMarkdown `notes.mmd`, `broken.mmd`), each in its README;
  hostile diagrams written by the tests (`docs/adr/0030-measure/gen` shows which).
- `.github/workflows/ci.yml`, `CONTRIBUTING.md` — `mermaid` among the features built alone; the rule
  naming where merman may be named; Tests; merman's pin among the pinned crates, its caps measured
  again with `docs/adr/0030-measure/` when it moves.
- `docs/usage.md` — a "Mermaid diagrams" section under Which files it shows (the kinds drawn, the
  differences that mislead, edges' routing, a user journey's tile); the row for `.mmd` and `.mermaid`
  in its table of what tells a file; Limits' rows (a diagram's size, merman's limits, the worker's
  caps, the pause after kills, the cache, Mermaid's worker among the six). `docs/architecture.md` —
  Backends' table and How each opens (the cache, the service, why a picture's opening costs nothing
  twice). `CHANGELOG.md`, `README.md` (what it shows). ADR-0030 turned Accepted and listed in
  `docs/adr/README.md`, in the commit that adds merman.

**Patterns to follow:**

- merman's site config, its resource profile and its deadline:
  `docs/adr/0030-measure/candidates/merman/src/main.rs`.
- Bytes made by one backend handed to another without its name: `src/source/gridcraft.rs` `open`,
  through `Input::held` and the first other backend in `BACKENDS` that `opens` them. Legitimate: the
  rule that only `main.rs` names `BACKENDS` is the core's
  ([Layers](../architecture.md#layers)).
- A worker started again after a kill, which costs only the request that killed it, and `TooMany`
  met: the PDF's text reader (`TextWorker` in `src/source/pdfcraft.rs`,
  [ADR-0027](../adr/0027-a-pdfs-text-read-in-a-worker-process-for-its-heap-cap.md)).
- Opening by name and that it is text, `formats()` from the same table: `MARKDOWN` in
  `src/source/wordcraft.rs`.
- No other file naming the library: `nothing_but_svg_html_names_blitz` in `tests/it/resvg.rs`.

**Decided:**

- merman 0.10.0, its features and its config as above
  ([ADR-0030](../adr/0030-mermaid-diagrams-through-merman-in-a-worker-process.md)).
- merman's limits first (`Constrained`, 500 edges, a 1.5 s deadline); the worker's caps past them
  (2 s of CPU, 6 s on the clock, 512 MiB).
- One worker for the backend, serving diagrams in turn behind a lock, started again after a kill,
  killed 2 seconds after its last answer, paused 60 s after three kills in a row.
- What the worker answers is cached by the diagram's bytes (64 MiB, 4,096 diagrams), failures
  included but for `TooMany`; Spec-0009's fenced blocks reach the same cache.
- `.mermaid` by its name; `.mmd` by its name and how it starts, within the first 1024 bytes, by
  merman's own headers.
- Labels as SVG `<text>`, `htmlLabels` false and secure.
- The `mermaid` feature brings `svg`.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- ✅ **Which crate, at what caps** (it blocked this) — resolved: merman, at the caps above
  ([ADR-0030](../adr/0030-mermaid-diagrams-through-merman-in-a-worker-process.md)); the owner's word
  given 2026-11-13.
- ✅ **Whose fonts measure the text** — resolved: merman sizes boxes for Mermaid 12's font
  and resvg draws in the system's Arial; labels fit their boxes, but for a sequence note too long for
  its box, which mermaid.js draws overflowing the same way (ADR-0030).
- ✅ **The kinds `.mmd` is told by** — resolved: `merman::diagram_header_facts()`.
- 🟡 **The caps off the spike's machine** — measured on macOS as peak resident set, not as the
  `cap` allocator counts the heap, and not on Linux: task 2 draws the costliest diagrams merman
  accepts through clv under the worker's caps, and CI runs them on Linux.

## Tasks

- [x] **The spike: which crate, at what caps** — at most one session, before this is Ready. Each
  candidate, with its default features off: its licence and dependency tree (no second resvg, nothing
  from git, nothing that fetches or loads files), the diagram kinds it draws and whether it lists
  them, whether it can label with SVG `<text>`, its output in kitty against Chrome's of mermaid.js on
  a dozen real diagrams (flowchart, sequence, class, state, ER, gantt, pie), and its time and memory on
  hostile diagrams (thousands of nodes and edges, deep subgraphs, long labels, 1 MiB of text). The
  caps, the cache's bounds and the pause from those. *Files:* the ADR and its `measure/` directory.
  *Verify:* the ADR's commands give its numbers again; the owner's word on the crate; `spec-update`
  lifts this to Ready with the numbers. Done:
  [ADR-0030](../adr/0030-mermaid-diagrams-through-merman-in-a-worker-process.md) (Proposed) and
  `docs/adr/0030-measure/`, the PNGs drawn through resvg 0.49 in a program of its own, not in kitty;
  the owner's word on 2026-11-13.
- [ ] **A Mermaid file shown** — the feature and merman, `mermaid.rs`'s opening, its worker's
  both sides, its service and its cache, `BACKENDS`, `WORKERS`; usage, architecture, the changelog,
  CONTRIBUTING, CI, and ADR-0030 Accepted and listed. *Verify:* `cargo test -j 2 --test it mermaid`:
  `flow.mmd` drawn (ink where its nodes are) and `sequence.mermaid`; `notes.mmd` and a `.mmd` whose
  diagram starts past 1024 bytes left unopened; `-` and `/dev/stdin` refused; `broken.mmd`'s message,
  printable and short; a diagram past 1 MiB, one not UTF-8, one past 500 edges (merman's message),
  one past the CPU cap and one past the heap cap each failing, and the next drawn; the costliest
  diagrams merman accepts (ADR-0030's `gen within` and `gen worst`) drawn under the caps; three killed
  in a row pausing the worker; unit tests in `mermaid.rs` for the cache (bounded, a failure kept,
  `TooMany` not) and the idle timer (the worker gone 2 s after its answer); nothing else naming
  merman; `the_extensions_a_backend_lists_are_the_names_it_opens_by`; each feature alone. Manual: in
  kitty, `clv tests/data/flow.mmd`.
- [ ] **A diagram in a list and in a document** — `deferred` and `draws_pictures`, and what they
  need of the cache. *Files:* `src/source/mermaid.rs`, `tests/it/mermaid.rs`, `tests/it/view_pager.rs`,
  `tests/it/wordcraft.rs`, usage. *Verify:* `cargo test -j 2 --test it mermaid`: a list of diagrams
  starts no worker until a tile is wanted, then one; `![](flow.mmd)` in Markdown drawn, and its
  document rendered page after page and laid out again (`]`) without the diagram turned into SVG
  again (the worker answers once, by its request count); a broken diagram's tile kept; `c` on a
  diagram; text cells. Manual: in kitty, a list of `.mmd` files, their tiles with their words (a user
  journey's without); a Markdown file's `![](x.mmd)`, `]` and `+`; the same inside a private
  `tmux -L` server.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass, each
  backend alone too.
- [ ] Tests added, each seen failing without the change: name them.
- [ ] Manual: in kitty, a diagram of each kind merman draws, beside mermaid.js's; a list's tiles; a
  Markdown file's `![](x.mmd)`; inside tmux.
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- 2026-11-13 — Ready (spec-new).
- 2026-11-13 — Back to Draft after a review (Codex): the spike settles the crate and the numbers;
  added the cache, the worker's service and its failures, the input's bounds, `.mmd`'s head within
  1024 bytes, tiles' words, and a third task.
- 2026-11-13 — Ready: the spike's findings (ADR-0030), merman approved by the owner; the numbers
  marked *(spike)* filled in, merman's limits and labels' config added, ELK's routing and diagrams
  past merman's limits out of scope (spec-update).
- 2026-11-13 — Implementing ([notes](0008-show-mermaid-diagrams-impl-20261113_1933.md)).
