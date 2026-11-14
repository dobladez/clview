# Working on clview

How to build, test and release clview, and the rules its code keeps. How it works is in
[`docs/architecture.md`](docs/architecture.md), and why some choices were made in
[`docs/adr/`](docs/adr/README.md). A change larger than a commit starts as a spec in
[`docs/specs/`](docs/specs/README.md), written and implemented through the `spec-*` skills.

## Building

```sh
cargo install --path .      # installs `clv`
```

It takes Rust 1.95 or later, the `rust-version`. All nine features are on by default;
`cargo build --no-default-features --features image` builds the viewer with images alone, and
`--no-default-features` alone without any backend.
[Backends](docs/architecture.md#backends) lists the features.

## Pinned dependencies

The PDF backend's pdfcraft is not on crates.io, so it comes from git, pinned to one commit of its
`main` in [`Cargo.toml`](Cargo.toml). To move the pin, change every `rev` there together, the
`[patch.crates-io]` ones included (they are pdfcraft's patched hayro, lopdf and vello), then run the
tests: `tests/it/pdfcraft_pin.rs` fails if a crate pdfcraft patches would be built from crates.io
instead. Only the backend (`src/source/pdfcraft.rs`) and its tests name pdfcraft, so another PDF
renderer would replace that file and the dependencies, and nothing else. The caps of the worker
that reads a PDF's text for a search rest on what pdfcraft's text costs at the pin
([ADR-0027](docs/adr/0027-a-pdfs-text-read-in-a-worker-process-for-its-heap-cap.md)): measure it
again with `docs/adr/0027-measure/`, its `rev`s moved with these.

The Word backend's wordcraft comes from git the same way, every `rev` of its crates at one commit;
it needs no patches, since clv builds its readers, layout and renderer but not its engine, whose
PDF export does. No crate wordcraft patches may be built here, since its patches don't reach us:
`tests/it/wordcraft.rs` fails if the crates disagree, or if such a crate comes to be built. Only
`src/source/wordcraft.rs` and its tests name wordcraft. It stays at `bbe3547` for now: past that, hyphenation
takes a checkout at build time ([ADR-0018](docs/adr/0018-wordcraft-stays-before-its-proofing-needs-a-checkout-to-build.md)), and those tests fail without it.

The spreadsheet backend's gridcraft likewise, from `tests/it/gridcraft.rs` and
`src/source/gridcraft.rs`; it reaches the PDF backend only by asking which backend opens the PDF
it prints, never by name.

The Mermaid backend's merman comes from crates.io, pinned to one version (`=`) in `Cargo.toml`,
without its default features: not ELK's layout (EPL-2.0), nor what reads the system's fonts or
the clock. Only `src/source/mermaid.rs` and `tests/it/mermaid.rs` name it, and it reaches the SVG
backend only by asking which backend opens the SVG it draws, never by name. The caps of the worker
it runs in rest on what its costliest diagrams cost at the pin
([ADR-0031](docs/adr/0031-mermaid-diagrams-through-merman-in-a-worker-process.md)): when it moves,
measure them again with `docs/adr/0031-measure/` (`gen within`, `gen worst`, through `clv --worker
mermaid`), and run `tests/it/mermaid.rs`, whose costliest diagrams must still be drawn under them.

**Moving Blitz** (the SVG backend's HTML labels, `src/source/svg/html.rs`): every `=` pin in
`Cargo.toml` moves together (`blitz-dom`, `blitz-html`, `blitz-paint`, `blitz-traits`, `anyrender`,
`anyrender_vello_cpu`, `stylo`, `parley`; `wuff` and `kurbo` as they need). Then:

1. Read the canaries, tests that assert a Blitz bug clv works around is still there, and fail with
   what to remove when it isn't: `blitz_still_drops_inline_content_in_a_table` in `html.rs` (its
   fixture `tests/data/blitz-table.html`; the workaround `anonymous_cells`; upstream, Blitz's issue
   #1430, fixed for text and inline alike by PR #783's anonymous table boxes and by PR #1370's
   table layout, measured on their branches on 2026-11-10, while #781 alone leaves the text out).
   The three tests beside it (`cells_and_the_content_between_them_share_a_row`,
   `wrapping_leaves_the_tables_rules_as_they_matched`, `anonymous_cells_cost_their_elements`) go
   with the workaround; so does usage's known difference on `clv-row` and `clv-cell`. The
   workaround leans on `set_style_property`, `set_incremental_layout` and on Stylo visiting an
   unstyled element whose ancestors are marked dirty: check them if it moves. When the workaround
   comes out, close [dobladez/clview#1](https://github.com/dobladez/clview/issues/1), which tracks it.
   The hang test
   `past_its_cpu_time_or_its_heap_the_labels_worker_is_killed_and_they_are_left_out` in
   `tests/it/resvg.rs` leans on blitz-paint's `text-decoration-inset` loop and its dotted border
   paths: if it fails, the caps still hold, and the fixtures need another hostile drawing.
2. Measure the worker's caps again: `docs/adr/0025-measure/`'s drawings through the release binary
   as [ADR-0026](docs/adr/0026-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md)'s
   Evidence says, and its `fonts()` against the system's fonts (the same families as before).
3. Check usage's known differences for the labels against real drawings: the tldraw, draw.io 21
   and Mermaid files the spike's `NOTES.md` names (`spike/blitz-foreignobject` branch), or the
   synthetic stand-ins in `tests/it/resvg.rs`, in kitty, zoomed, against Chrome.
4. `tests/it/resvg.rs`'s `nothing_but_svg_html_names_blitz` says which crates may be named where.

## Rules

- **Never panic.** Every file, every byte the terminal sends back and every argument is untrusted
  input. In non-test code: no `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!` unless provably
  infallible with a comment saying why; `get()` rather than indexing with input-derived positions,
  and string slicing only at char boundaries; checked or saturating arithmetic on input-derived
  numbers; allocations sized by input are capped. `unsafe_code = "forbid"`.
- **Nothing raw reaches the terminal.** File names, messages and anything a document can shape go
  through `printable` first.
- **The core never depends on an extension.** The code is three layers
  ([Layers](docs/architecture.md#layers)): the core (`main.rs`, `args.rs`, `encode.rs`, `src/view/`),
  the contract (`src/source.rs`) and the extensions (`src/source/`: the backends, and the modules
  that serve them). The core names nothing under `src/source/`; what the viewer needs of a source,
  `source.rs` declares or re-exports, and `main.rs` alone names `source::BACKENDS`, to hand the list
  to the viewer. An extension may use the core the other way (`args`, `encode`), never `src/view/`,
  and never another backend by name. `source::tests::the_viewer_names_no_backend` checks the core's
  files, and CI's nightly run builds the core with no backend at all (`cargo clippy --no-default-features`).
- **A backend's library is its own.** pdfcraft is named in code only in `src/source/pdfcraft.rs`,
  its tests (`tests/it/pdfcraft*.rs`), `Cargo.toml` and the line listing it in `source::BACKENDS`;
  swapping the PDF renderer must touch nothing else in the code, and in the docs only what credits
  or describes it. `tests/it/pdfcraft.rs` checks the code. Likewise resvg and what it renders with
  (usvg, tiny-skia, roxmltree, fontdb): only in `src/source/svg.rs`, `tests/it/resvg.rs` (which
  checks it) and `Cargo.toml`; bzip2 and lzma-rust2 only in `src/source/tar.rs`, `tests/it/tar.rs`
  (which checks it) and `Cargo.toml` (flate2 there and in the SVG backend, for svgz); wordcraft and
  pulldown-cmark only in `src/source/wordcraft.rs`, `tests/it/wordcraft.rs` (which checks it) and
  `Cargo.toml`; gridcraft only in `src/source/gridcraft.rs`, `tests/it/gridcraft.rs` (which checks
  it) and `Cargo.toml`; Blitz and what it paints an SVG's HTML labels with (blitz, anyrender,
  kurbo, stylo, parley, taffy, wuff) only in `src/source/svg/html.rs`, `tests/it/resvg.rs` (which
  checks it) and `Cargo.toml`; merman only in `src/source/mermaid.rs`, `tests/it/mermaid.rs` (which checks it)
  and `Cargo.toml`; the allocator that caps a worker process's heap (cap) only in
  `src/source/worker.rs`, `tests/it/view.rs` (which checks it) and `Cargo.toml`.
- **A backend brings its own render threads**, honouring `Pool`'s contract: `set_queue` replaces the
  work wanted, a result arrives only while its request is listed, `try_recv` never blocks. A backend
  with nothing better (images, drawings) lends its render function to `source/threads.rs`.
- **Prove a test fails before the fix** by actually reverting the fix.
- **Rust only.**
- **Record decisions in an ADR** (`docs/adr/`, its README says when and how): a choice between
  real alternatives that a later reader would otherwise undo or redo, above all one resting on a
  measurement or an observed terminal behaviour, or a fix measured and rejected. Write it in the
  commit that makes the choice, with the versions, setup and commands that get the evidence
  again, and link it from the doc that describes the behaviour.
- **Clean room.** Behaviour comes from specifications (the kitty graphics protocol, the tmux
  manual) and black-box observation of terminals and of other programs' output. Never read the
  source of another terminal-graphics program or PDF viewer (kitty, tdf, timg, chafa, viu, MuPDF
  and so on): most are GPL or AGPL, and this code is MIT or Apache-2.0.

## Where a fact goes

Each fact has one home, and the other docs link to it. When a change needs documenting:

| Doc | Reader | What goes there |
|---|---|---|
| `README.md` | Someone deciding to try it | What clv is, how to install it, what it shows and through what, plans, a line each. Never the only home of a behaviour |
| `docs/backlog.md` | Someone choosing what to build next | The details behind the Plans, and ideas not yet planned. Never promised, never the home of a behaviour |
| `CHANGELOG.md` | Someone deciding to update | A line under `## Unreleased`, in the commit that makes the change, for anything a user would notice. What changed, never how it works now: that is the manual's |
| `docs/usage.md` | Someone using it | Anything a user would notice changing: options, keys, what each format shows, what terminals need, limits |
| `docs/architecture.md` | Someone working on the code | Anything naming a function, an escape code or a thread, and the numbers a design rests on |
| `docs/adr/` | Someone about to undo a choice | Why this way and not another: the alternatives rejected, and the evidence |
| `docs/specs/` | Someone about to build a change | What one change set out to do and how, before its code: a record of that moment, frozen once `Done`. Never the only home of a behaviour |
| This file | Someone working on the code | How to build, test and release, and the rules |

## Quality gates

Before every commit, under a `timeout`:

```sh
cargo fmt --check
cargo clippy --all-targets -j 2 -- -D warnings
cargo test -j 2
```

That is the build clv ships, every feature on, and the only one tested before a commit: other
feature sets, each backend alone among them, aren't built locally, even after touching `cfg`
attributes. The PDF backend is a large build: keep `-j 2` unless the machine is free. CI (`.github/workflows/ci.yml`) runs on Linux
and macOS, with Rust 1.95.0, the `rust-version`. Every push tests the binary shipped, all features:
`fmt`, clippy and every test. Nightly, if `main` moved since the last nightly run, it builds and
tests the viewer without backends (which keeps the core free of the extensions,
[Layers](docs/architecture.md#layers)), and `pdf,svg`; `gh workflow run CI` runs both at once, worth
it after a change to `cfg` attributes. Each backend alone is tested nowhere.

## Tests

- **Unit tests** in each module, among them property tests for the key decoder (answers cut
  anywhere among keys are never keys), the fit and the grid, that the reader's page stays on
  screen, and a search's matches and rectangles (within their text, within their page). The
  pager's search is tested on a fake text (`search::tests::Fake`), which gives what it is asked
  when the test lets it.
- **`tests/it/`** is one test binary, its modules sharing the fixtures. Its tests show PDFs, so it
  needs the `pdf` feature. The generic tests write their PDFs by hand (`tests/it/fixtures.rs`), so
  any PDF backend reads them.
- **`tests/it/view.rs`**: printing, through a pipe with `--protocol kitty`, and text cells, which a
  pipe takes as they are. A list's files opened as they are reached, too: at their true size, and a
  broken one costing only its tile.
- **`tests/it/view_pager.rs`**: the pager on a pseudo-terminal from script(1), keys in through its
  stdin. It plays a terminal that takes temporary files (it reads each file named, checks its
  size, deletes it) or one that doesn't, and sends kitty's answers, its clipboard's too. It plays one
  without the protocol as well, for text cells. A list's file opened on demand is sent at its true size,
  and one that turns out broken keeps its tile. `--help`'s icon is tested here too, since it depends
  on stdout being a terminal. A PDF's search too: its text read in a worker seen with `ps`, gone
  with the search, the file and the pager, and a page past the worker's heap, written by the test,
  costing the search that page; and its tint, read back from the PNGs sent: on the matches and
  nowhere else, the page before deleted once the new one is placed, gone with `Esc`, absent from
  a copy, and in text cells' colours.
- **`tests/it/view_tmux.rs`**: the pager in a private tmux server (`tmux -L`, killed at the end), its
  clients on script(1) pseudo-terminals. It covers flips, a hidden pane, a second client, a page
  tinted for a search reaching a client that attaches later, and
  pages printed from a shell, deleted once no pane holds them, `--help`'s icon, on a client whose
  terminfo entry says it is kitty, and text cells in a pane.
- **`tests/it/pdfcraft.rs`** and **`tests/it/pdfcraft_pin.rs`**: the PDF backend's own (passwords,
  XFA forms, its warnings, its pinned dependency), and that nothing else names its library. What
  its text's worker answers (a page's text as searched, a range's rectangles) is unit-tested in
  `src/source/pdfcraft.rs`, the worker's code run on a thread.
- **`tests/it/resvg.rs`**: the SVG backend through the viewer: what it draws, the limits a drawing is
  held to, its HTML labels and the worker process they are drawn in (killed past its CPU time or
  its heap, with the drawings that reach each), and that nothing else names its libraries or
  Blitz's. The worker's own tests, with `sleep`, `sh` and `yes` as workers, are unit tests in
  `src/source/worker.rs`; the pager's, that a list's tiles start none, in `tests/it/view_pager.rs`.
- **`tests/it/wordcraft.rs`** and **`tests/it/gridcraft.rs`**: documents and workbooks as pages, a
  protected one opened with its password, a workbook searched through its PDF, their pins, and that
  nothing else names their libraries. A document searched in the pager, on a pseudo-terminal: a
  word hyphenated at a line's end, a table's cell, a field's result, a paragraph across a page
  break, one too long to search, and the tint on dark paper. What a document's text answers, and
  that the pool and a search lay it out once, is unit-tested in `src/source/wordcraft.rs`.
- **`tests/it/mermaid.rs`**: Mermaid diagrams through the viewer: which `.mmd` files it opens and
  which it leaves to MultiMarkdown, what merman draws of one, its limits and messages, the costliest
  diagrams merman accepts drawn under the worker's caps, a list's diagrams opened as their tiles are
  wanted, and that nothing else names merman; in `tests/it/view_pager.rs`, that a list's diagram
  starts the worker only once its tile is wanted, and a document's diagram is turned into SVG once
  however it is laid out, watched with `ps`; in `tests/it/wordcraft.rs`, a diagram as a document's
  picture. The
  worker's service (one worker in turn, gone once idle, paused after kills, with fake workers that
  die) and the cache of what it answered are unit-tested in `src/source/mermaid.rs`.
- **`tests/it/zip.rs`** and **`tests/it/tar.rs`**: archives through the viewer, as lists and as
  folders to go into; for tar, read on a thread of its own, a file compressed alone, and that nothing
  else names its decompressors.

- **`tests/docs.rs`**: that every relative link in the docs leads to a file, and its anchor to a
  heading, that the changelog has this version, and that the specs' Index says what their headers
  say (through `examples/spec_list.rs`, which it builds as a module). A binary of its own, so it runs without backends too.

The terminal tests skip with a warning where tmux or script(1) can't run, so CI machines without
them pass.

Test pictures are made by the tests, or kept in `tests/data/`, synthetic, with how each was made in
its README.

## Looking at it

Output is images in a terminal, so look at the result in kitty, Ghostty or WezTerm. Without a
terminal, `--protocol kitty` writes the escape codes to stdout (the integration tests read them
back), `--protocol ascii` writes text you can read as it is, and `script(1)` gives a test a
pseudo-terminal (`tests/it/view_pager.rs`).

To try a real kitty without touching your own, run `kitty --start-as=hidden --config NONE` with
`TMUX` and `TMUX_PANE` unset (an inherited `TMUX` makes `clv` think it is inside your tmux: it
asks your tmux server about your pane, and wraps its escape codes for a tmux that isn't there),
and use a private `tmux -L <name>` server for tmux. A hidden kitty answers as a visible one does,
but doesn't free images the same way: there, cleared PNGs stayed while raw pixels went, which a
visible window doesn't do. Measure memory in a visible window.

## Releases

Bump `version` in `Cargo.toml`, turn `CHANGELOG.md`'s `## Unreleased` into `## <version> (<date>)`
under a new empty `## Unreleased`, commit, tag `v<version>` and push the tag:
`.github/workflows/release.yml` builds `clv` for macOS and Linux (musl) and publishes the archives
as a GitHub release, that section its notes. It refuses a tag that disagrees with `Cargo.toml`, or a
binary whose `--version` names a commit after that version ([`build.rs`](build.rs) says how), and
`tests/docs.rs` fails on a version the changelog has nothing for. Run by hand, it only builds.

Once the release is published, its `homebrew` job writes the formula (`.github/homebrew-formula.sh`)
from the release's checksums and puts it in the tap, [dobladez/homebrew-tap](https://github.com/dobladez/homebrew-tap),
as `Formula/clv.rb` ([ADR-0020](docs/adr/0020-new-versions-through-a-homebrew-tap-not-a-check-inside-clv.md)).
It writes with the `HOMEBREW_TAP_TOKEN` secret: a fine-grained token whose only repository is the
tap, with Contents read and write. When the token has expired, or is missing, the job fails and the
release stands: make a new token, `gh secret set HOMEBREW_TAP_TOKEN`, and `gh run rerun <run> --failed`.
