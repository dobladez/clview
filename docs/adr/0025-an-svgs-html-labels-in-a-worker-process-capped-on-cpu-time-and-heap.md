# ADR-0025: An SVG's HTML labels in a worker process, capped on CPU time and heap

- **Status:** Accepted
- **Date:** 2026-11-09

## Context

[ADR-0024](0024-draw-an-svgs-html-labels-with-blitz-one-document-a-drawing.md) draws an SVG's
HTML labels with Blitz, one document a drawing, each stage bounded before it is spent: counts
before Blitz, and an estimate of its painting (shadows, layers' tiles, a dotted border's dots,
a decoration's lines) after layout, before it was done, since a painting once begun can't be
stopped. Three reviews each found CSS the estimate missed: a `text-decoration-inset` of
`-1e20px` loops for ever in blitz-paint, its step lost to the float; `content` strings are
copied into a text node for each element they reach, before layout; a clip path is made anew for
each element it clips; a decoration on the document's own `body` is painted once a label; a
dotted border a million pixels long has its dots built into one path before anything counts
them; a WOFF whose table is stored larger than it unpacks takes wuff past what it sized. An
allowlist of CSS was weighed, and would have been a denylist in disguise.

In the viewer's process, neither time nor memory can be capped: a Rust thread can't be stopped,
Blitz has no cancellation, and an allocation refused aborts the viewer. A drawing is untrusted
input ([ADR-0003](0003-bound-what-a-drawing-costs-before-usvg-and-resvg-spend-it.md)); the rules
are never to panic and never to let anything raw reach the terminal; `unsafe_code` is forbidden,
so `pre_exec` and `libc` are out.

## Decision

1. **Blitz runs in a worker process of clv's own:** clv run again as `clv --worker svg-html`
   (`std::env::current_exe`, a hidden argument), its stdin and stdout pipes of frames, its stderr
   null, its environment cleared but for `HOME`, `XDG_*` and `FONTCONFIG_*`, which fontique's
   Linux backend needs through libfontconfig (`fonts.conf`, its cache). The first frame says the
   protocol and clv's version, so a binary replaced under a running viewer answers that it is
   another's and the labels are left out. Nothing Blitz, Stylo, Parley or vello_cpu does on a
   drawing's bytes runs in the viewer. `src/source/worker.rs` is the worker machinery for any
   backend; `src/source/svg/html.rs` alone names Blitz, and is both sides of this worker.
2. **The worker caps itself where the kernel and the allocator enforce it.** CPU time by
   `RLIMIT_CPU`, which it arms before each request to the CPU it has used plus the request's
   budget (10 s to lay the labels out and record them, 5 s a render), through rustix's safe
   `setrlimit`; the kernel stops it with `SIGXCPU`. Heap by the `cap` allocator, the process's
   `#[global_allocator]`, given a limit of 1 GiB in the worker alone (`usize::MAX` in the
   viewer): the allocation that crosses it fails and the worker aborts. The viewer's deadline,
   three times the CPU budget on the wall clock, covers a request's sending and its answer, and
   kills the worker past it: the backstop for a worker that isn't running.
3. **One worker a drawing,** started by the first render that isn't a list's tile's, owned by
   the parsed drawing (a pool's, or the printer's renderer's) and killed when it goes; at most
   six alive, a backstop (the pager keeps at most that many drawings' pools), past which a
   drawing's labels are left out rather than wait. A dead worker, for whatever reason, isn't
   started again for that drawing: a deterministic death isn't repeated every render. A list's
   tiles start none (`Request::tile`), nor do the pictures a document's page that is a tile refers
   to.
4. **What crosses the pipe is pixels.** The viewer sends the drawing's stylesheets and each
   label's markup, size and overflow; the worker lays them out as ADR-0024 decided, keeps each
   label's recording, and answers each render with the premultiplied RGBA of a label's part of
   the page, exactly the size asked. Everything read from the worker is untrusted: each frame's
   kind and length against a cap, counts against what was sent, a reason as a code the viewer
   puts words to, the links it didn't follow cut to 256 bytes and hashed, never shown. The viewer
   holds at most one answer: its reader takes a frame's first byte as it comes, so a worker that
   ends between requests is seen ending and reaped at once, and the rest only when an answer is
   awaited.
5. **The estimate goes,** with the recorder's budget, the 8 MiB thread, `catch_unwind` and the
   version guard. The cheap bounds before the worker stay (labels, elements, markup, text,
   styles, what is fetched and how large), since they are exact and spare the worker for a real
   drawing with one bad label; so do the rules that aren't bounds: no stylesheet or document is
   fetched (`LOADING`, `without_imports`), and a WOFF's tables are checked against its file
   before wuff unpacks them. The nesting limit is the writer's own, 1,024 levels, since the
   drawing's scan skips some elements by name; the worker lays out on a thread of 64 MiB.
6. **A page's labels are all painted before any is drawn:** a worker gone on one leaves none on
   the page, now or again, so every render agrees with its warning.

## Evidence

Measured 2026-11-09 on an Apple M3 Max (14 cores, 36 GiB), macOS 26.6.2, Rust 1.95.0, with two
throwaway crates (`rustix` 1.1 with `process` and `time`, `cap` 0.1.2; `parley` 0.12.0) and the
release build of clv at this commit.

**What macOS enforces.** A child process set each limit on itself, then spent:

| Mechanism | macOS 26.6.2 | Linux |
|---|---|---|
| `RLIMIT_CPU` 1 s, a busy loop | killed by `SIGXCPU` at 1.00 s; the soft limit raised again by the process from 1 to 2 s (the hard limit infinite) and honoured | the same, `SIGKILL` at the hard limit |
| `RLIMIT_AS` 256 MiB | `setrlimit` fails, `EINVAL`; 1 GiB allocated and touched | enforced, on virtual memory |
| `RLIMIT_DATA` 256 MiB | `EINVAL`; 1 GiB touched | enforced |
| `RLIMIT_RSS` 256 MiB | `EINVAL`; 1 GiB touched | not enforced |
| `cap::Cap<System>` at 256 MiB as the global allocator | the allocation that crosses it fails; Rust aborts, `SIGABRT`, "memory allocation of N bytes failed"; `try_reserve` past it returns `Err` | the same: it is the allocator |

So no rlimit caps a worker's memory on macOS. A watchdog reading the footprint (libproc,
sysinfo, memory-stats) would poll, and a page-touching loop here grows at some 15 GB/s: 150 MB
past the cap in a 10 ms interval. The allocator's cap is exact, instant and portable, and counts
the heap alone: thread stacks (bounded by their size) and fontique's memory-mapped system font
files (clean, shared pages) are outside it, so the footprint exceeds the cap by those and
malloc's slack.

**What a worker costs to start and to answer:**

| What | Measured |
|---|---|
| Spawn and exit of `clv --version` (the 67 MB release binary), stdio piped, environment cleared, median of 20 | 4.7 ms |
| `FontContext::default()`: CoreText enumerates 551 families | 30–39 ms; the same under `env -i` |
| 64 MiB of pixels (a 4,096 × 4,096 part) through a pipe to `cat`, and back | 21 ms one way, 22 ms the round trip |
| A thread of 64 MiB | holds 100,000 frames of some 600 bytes |

**The caps, on the drawings ADR-0024 measured and round 3's cases**, through the release binary
with `--protocol kitty` under `/usr/bin/time -l`, whose maximum resident set is the larger of the
viewer's and the worker's (a reaped child's counts), the worker's own set and CPU time sampled
from `ps` every 10 ms while it lives (`measure gen styles 1024 3072` and `font 50 400` from
ADR-0024's tool; the rest written by hand as `tests/it/resvg.rs` writes them):

| Drawing | What happens | Time, memory |
|---|---|---|
| `styles 1024 3072`: 1,024 labels under 3 MiB of styles | drawn, none left out | 8.0 s to open, of which the worker spends 0.32 s of CPU; 97 MB in all, the worker 95 MB (the rest is usvg's stylesheet parse, a known issue) |
| `font 50 400`: 50 labels sharing a 400 KB font | drawn | 0.10 s, 35 MB |
| a drawing with one label, against the same without | drawn | 0.07–0.13 s against 0.04 s: some 30–90 ms more, the worker 17 MB |
| `text-decoration-inset: -1e20px` | the heap cap, in under a second: the loop grows its path as it goes | 0.20 s; the worker reached 1 GiB |
| a box 2,000,000 pixels wide with a 1-pixel dotted border | the heap cap: the dots built into one path | 0.27 s; 1 GiB |
| a `content` of 64 KiB on 99,000 elements | the heap cap: a text node an element | 0.30 s; 1 GiB |
| 10,000 elements each clipped by a 1,000-point polygon | drawn | 0.19 s, the worker 37 MB |
| a wavy underline under 3,000 pixels of the `body`, painted with each of 1,000 labels | drawn | 1.23 s, the worker 1.15 s of CPU and 57 MB |
| a WOFF whose table is stored larger than it unpacks | the font refused, the label drawn without it | |

After the hang and the bombs no `clv --worker` process is left; `clv` killed with `SIGKILL` while
its worker lives, the worker is gone within three seconds (its stdin ends).

The tests in `tests/it/resvg.rs` run each case through the debug binary; `worker.rs`'s own
tests run the viewer's side against `sleep`, `sh` and `yes`, and the CPU cap against the test
binary itself spinning as a worker.

## Alternatives considered

- **An allowlist of CSS:** a denylist in disguise; three reviews found new holes each time.
- **A footprint watchdog**, in the worker (memory-stats, libproc) or in the viewer (libproc,
  sysinfo): a new dependency all the same, platform code, a poll interval's overshoot, a race
  with the kill. The allocator's cap has none of them.
- **`RLIMIT_AS` on Linux besides the cap:** it counts virtual memory (glibc's arenas, the 64 MiB
  stack, the mapped fonts), a platform difference, a flake risk, nothing the cap doesn't do.
- **Wall-clock limits alone:** flaky under load; CPU time is the deterministic one, with the wall
  clock as a backstop for a worker that isn't running.
- **A test-only override of the limits:** a knob in the product for the tests' sake; the tests fit
  the real limits (the hang dies by the heap cap in under a second; the bombs at once).
- **Scenes over the pipe, replayed in the viewer:** anyrender's scenes serialize (its `serde`
  feature), but a glyph run carries its whole font file (Apple Color Emoji is some 190 MB) and a
  fill its decoded picture, and the viewer would replay them with vello_cpu and skrifa on
  untrusted bytes, which is what the worker is for.
- **A process a render**, laying out each time: the simplest, but 0.6 s more at every zoom step
  of the styles drawing; kept as the fallback.
- **A worker shared between drawings:** one hostile drawing's death would take every drawing's
  labels, and their renders would serialize.
- **Restarting a dead worker:** a deterministic death repeated every render.
- **The recorder's command budget kept for memory:** the heap cap is exact and covers it.
- **`catch_unwind` in the worker, to survive one label's replay panic:** a second failure path
  for a hypothetical; a dead worker is one path.

## Consequences

- **Positive:** a hang, a bomb, a panic, a crash and an abort in Blitz all look the same to the
  viewer, which runs none of it; the limits are two numbers and a count, not an estimate that
  follows blitz-paint's painting; a label stays sharp at any zoom and costs its part of the page,
  as before.
- **Accepted:**
  - A drawing with labels costs a process: some 5 ms to start, some 35 ms to find the fonts,
    once a drawing opened or printed; a list's tiles show drawings without their labels.
  - Two scales of one drawing replay in turn, not at once: a pool's threads share the worker.
  - A worker that dies takes all of the drawing's labels with it until the drawing is opened
    again; a stack overflow in it would read as memory (both abort), though none can be made
    from input.
  - `cap` wraps the viewer's allocator too, at no limit: one relaxed atomic a call.
  - Blitz stays pinned at `=0.3.0-beta.4` for its API; the bomb test's drawing grows by
    blitz-paint's shapes.
- **Follow-ups:** usvg and resvg in a worker too, which would bound a stylesheet's parse
  (simplecss takes time as the square of a declaration's length), in README's Plans.
- **Revisit when:** Blitz gains cancellation or bounds of its own; `RLIMIT_AS` or a memory
  limit reaches macOS; the owner wants a drawing's labels back after a transient failure.

## Update 2026-11-09: the worker's fonts through fontdb, not fontconfig

**Context.** Blitz's `system-fonts` feature is parley's `system`, which is fontique's, which on
Linux is `yeslogic-fontconfig-sys`: its build script runs `pkg_config::find_library("fontconfig")
.unwrap()` unless its `dlopen` feature is on or `RUST_FONTCONFIG_DLOPEN` is set. The release
builds (`.github/workflows/release.yml`) cross-compile from GNU Linux to musl with nothing set
up for pkg-config, so both Linux artifacts failed at that script: reproduced here with
`cargo check --target x86_64-unknown-linux-musl -p yeslogic-fontconfig-sys` ("pkg-config has
not been configured to support cross-compilation"), and seen passing with
`RUST_FONTCONFIG_DLOPEN=1`. With `dlopen`, a static musl binary can't load fontconfig at run
time: fontique's `SystemFonts::new` sees the library missing and holds no fonts, so labels
would draw their boxes, pictures and embedded fonts but no text in the system's fonts, while
draw.io's fallback text is cut all the same.

**Decision (the owner's: option 5 of those weighed).** The worker gets the system's fonts
from fontdb, as the drawing's own text has them: `svg.rs` loads them once a process
(`system_fonts`, fontdb's own `fonts.conf` parser on Linux, its directory scan elsewhere) and
hands the worker each font file's path, or the bytes fontdb holds of it (`label_fonts`); the
worker maps each path through fontique's own source cache (`SourceCache::get`, a memory map
inside fontique: clv forbids `unsafe`, and fontdb's `make_shared_face_data` is an `unsafe fn`)
and registers it (`Collection::register_fonts`), never reading a file into its heap. The
generic families are set as `svg.rs` sets them for resvg, and `system-ui` with `sans-serif`;
emoji to Apple Color Emoji, Noto Color Emoji or Segoe UI Emoji; and since a collection built
from files has no script fallback (CoreText's cascade, fontconfig's matching), each script
falls back to a list of the usual families the system has (`FALLBACKS`: PingFang, Hiragino,
Noto Sans CJK and the rest for Han, kana and hangul; Geeza Pro and Noto for Arabic; and so on).
Blitz's `system-fonts` feature is off, parley a direct dependency without its `system` feature:
`yeslogic-fontconfig-sys`, `dlib`, `libloading` and `objc2-core-text` leave the lock, for every
feature set and target (`cargo tree -i yeslogic-fontconfig-sys` names nothing).

**Evidence.** On this machine (1,222 faces in 644 files), fontdb's load and fontique's
registration take 18–23 ms in all (fontdb 10–15 ms, mapping and registering 8 ms), against
CoreText's own enumeration in fontique at 33–39 ms; the same 408 families come out, Helvetica,
Arial, Times New Roman, Menlo, Comic Sans MS and Apple Color Emoji among them (PingFang SC is in
neither: it lives in an asset catalogue). Sixteen lines of text in named families (Helvetica,
Arial, Times New Roman, Courier New, Menlo, Comic Sans MS, Georgia, Verdana, draw.io's
`trebuchet ms, verdana, arial, sans-serif`) and in the generic ones (`sans-serif`, `serif`,
`monospace`, `cursive`, `fantasy`, `system-ui`, a name no font has) render pixel for pixel as
before. What differs: CJK, emoji, Greek and Cyrillic text falls back to a family from the list
(Hiragino Sans for Han here, where CoreText cascaded to PingFang), so its glyphs may differ, and
a script not in the list falls back to nothing, as the drawing's own text wouldn't either.
`cargo check --target x86_64-unknown-linux-musl --no-default-features --features svg-html`
passes with the change (the full musl build needs a musl linker, which CI has).

**Alternatives.** `dlopen` alone (fonts lost in a static binary); `svg-html` off in the Linux
release (labels lost there); fontconfig built static for musl (heavy, and fontconfig's files
still needed at run time); glibc builds (one binary no longer runs on any distribution); reading
font files into the worker's heap (some 700 MB of fonts on this machine, against a 1 GiB cap).

## Update 2026-11-10: inline content in a `display: table` box, wrapped for Blitz

Blitz 0.3.0-beta.4 lays out none of a `display: table` box's children that aren't table parts
(`blitz-dom/src/layout/table.rs`, `collect_table_cells`: a text child returns at once, a flow,
flow-root, flex or grid child is dropped as "probably a table caption"), where CSS 2.1 §17.2.1
gives each run of them an anonymous `table-row` and `table-cell`. Mermaid's edge labels are such a
box (`display: table` holding a `<span>`), and came out blank: six of a 44 KB flowchart's. Its git
`main` (4647f4a2, 2026-11-10) does the same. The owner reversed the spec's "it is Blitz's to fix":
the worker wraps each such run in a row and a cell of its own (`anonymous_cells`), decided by
Stylo's computed `display` once the styles resolve, before layout: the structure a browser makes,
so the layout is the same, not a different one (a `block` in its place would lose the table's
shrink-to-fit and `vertical-align`). The repro (a red square and text in such a box, three
controls that draw) is `tests/data/blitz-table.html`; `blitz_still_drops_inline_content_in_a_table`
renders it through Blitz without the workaround and fails, saying to remove the workaround, the
day Blitz draws it. CONTRIBUTING's "Moving Blitz" says what else to read then.
