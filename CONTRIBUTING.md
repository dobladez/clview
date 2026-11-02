# Working on clview

How to build, test and release clview, and the rules its code keeps. How it works is in
[`docs/architecture.md`](docs/architecture.md), and why some choices were made in
[`docs/adr/`](docs/adr/README.md).

## Building

```sh
cargo install --path .      # installs `clv`
```

It takes Rust 1.95 or later, the `rust-version`. All seven features are on by default;
`cargo build --no-default-features --features image` builds the viewer with images alone, and
`--no-default-features` alone without any backend.
[Backends](docs/architecture.md#backends) lists the features.

## Pinned dependencies

The PDF backend's pdfcraft is not on crates.io, so it comes from git, pinned to one commit of its
`main` in [`Cargo.toml`](Cargo.toml). To move the pin, change every `rev` there together, the
`[patch.crates-io]` ones included (they are pdfcraft's patched hayro, lopdf and vello), then run the
tests: `tests/it/pdfcraft_pin.rs` fails if a crate pdfcraft patches would be built from crates.io
instead. Only the backend (`src/source/pdfcraft.rs`) and its tests name pdfcraft, so another PDF
renderer would replace that file and the dependencies, and nothing else.

The Word backend's wordcraft comes from git the same way, every `rev` of its crates at one commit;
it needs no patches, since clv builds its readers, layout and renderer but not its engine, whose
PDF export does. No crate wordcraft patches may be built here, since its patches don't reach us:
`tests/it/wordcraft.rs` fails if the crates disagree, or if such a crate comes to be built. Only
`src/source/wordcraft.rs` and its tests name wordcraft. It stays at `bbe3547` for now: past that, hyphenation
takes a checkout at build time ([ADR-0018](docs/adr/0018-wordcraft-stays-before-its-proofing-needs-a-checkout-to-build.md)), and those tests fail without it.

The spreadsheet backend's gridcraft likewise, from `tests/it/gridcraft.rs` and
`src/source/gridcraft.rs`; it reaches the PDF backend only by asking which backend opens the PDF
it prints, never by name.

## Rules

- **Never panic.** Every file, every byte the terminal sends back and every argument is untrusted
  input. In non-test code: no `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!` unless provably
  infallible with a comment saying why; `get()` rather than indexing with input-derived positions,
  and string slicing only at char boundaries; checked or saturating arithmetic on input-derived
  numbers; allocations sized by input are capped. `unsafe_code = "forbid"`.
- **Nothing raw reaches the terminal.** File names, messages and anything a document can shape go
  through `printable` first.
- **A backend's library is its own.** pdfcraft is named in code only in `src/source/pdfcraft.rs`,
  its tests (`tests/it/pdfcraft*.rs`), `Cargo.toml` and the line listing it in `source::BACKENDS`;
  swapping the PDF renderer must touch nothing else in the code, and in the docs only what credits
  or describes it. `tests/it/pdfcraft.rs` checks the code. Likewise resvg and what it renders with
  (usvg, tiny-skia, roxmltree, fontdb): only in `src/source/svg.rs`, `tests/it/resvg.rs` (which
  checks it) and `Cargo.toml`; bzip2 and lzma-rust2 only in `src/source/tar.rs`, `tests/it/tar.rs`
  (which checks it) and `Cargo.toml` (flate2 there and in the SVG backend, for svgz); wordcraft and
  pulldown-cmark only in `src/source/wordcraft.rs`, `tests/it/wordcraft.rs` (which checks it) and
  `Cargo.toml`; gridcraft
  only in `src/source/gridcraft.rs`, `tests/it/gridcraft.rs` (which checks it) and `Cargo.toml`.
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
| `README.md` | Someone deciding to try it | What clv is, how to install it, what it shows and through what, plans. Never the only home of a behaviour |
| `docs/usage.md` | Someone using it | Anything a user would notice changing: options, keys, what each format shows, what terminals need, limits |
| `docs/architecture.md` | Someone working on the code | Anything naming a function, an escape code or a thread, and the numbers a design rests on |
| `docs/adr/` | Someone about to undo a choice | Why this way and not another: the alternatives rejected, and the evidence |
| This file | Someone working on the code | How to build, test and release, and the rules |

## Quality gates

Before every commit, under a `timeout`:

```sh
cargo fmt --check
cargo clippy --all-targets -j 2 -- -D warnings
cargo test -j 2
cargo clippy --no-default-features --all-targets -j 2 -- -D warnings   # the viewer without backends
cargo test --no-default-features -j 2
```

After touching `cfg` attributes, each backend alone too (`--no-default-features --features image`,
then `pdf`, then `svg`, then `zip`, then `tar`, then `word`, then `sheet`). The PDF backend is a
large build: keep `-j 2` unless the machine is free. CI (`.github/workflows/ci.yml`) runs them all
on Linux and macOS, with Rust 1.95.0, the `rust-version`.

## Tests

- **Unit tests** in each module, among them property tests for the key decoder (answers cut
  anywhere among keys are never keys), the fit and the grid, and that the reader's page stays on
  screen.
- **`tests/it/`** is one test binary, its modules sharing the fixtures. Its tests show PDFs, so it
  needs the `pdf` feature. The generic tests write their PDFs by hand (`tests/it/fixtures.rs`), so
  any PDF backend reads them.
- **`tests/it/view.rs`**: printing, through a pipe with `--protocol kitty`, and text cells, which a
  pipe takes as they are.
- **`tests/it/view_pager.rs`**: the pager on a pseudo-terminal from script(1), keys in through its
  stdin. It plays a terminal that takes temporary files (it reads each file named, checks its
  size, deletes it) or one that doesn't, and sends kitty's answers, its clipboard's too. It plays one
  without the protocol as well, for text cells. `--help`'s icon is tested here too, since it depends
  on stdout being a terminal.
- **`tests/it/view_tmux.rs`**: the pager in a private tmux server (`tmux -L`, killed at the end), its
  clients on script(1) pseudo-terminals. It covers flips, a hidden pane, a second client, and
  pages printed from a shell, deleted once no pane holds them, `--help`'s icon, on a client whose
  terminfo entry says it is kitty, and text cells in a pane.
- **`tests/it/pdfcraft.rs`** and **`tests/it/pdfcraft_pin.rs`**: the PDF backend's own (passwords,
  XFA forms, its warnings, its pinned dependency), and that nothing else names its library.
- **`tests/it/resvg.rs`**: the SVG backend through the viewer: what it draws, the limits a drawing is
  held to, and that nothing else names its libraries.
- **`tests/it/wordcraft.rs`** and **`tests/it/gridcraft.rs`**: documents and workbooks as pages, a
  protected one opened with its password, their pins, and that nothing else names their libraries.
- **`tests/it/zip.rs`** and **`tests/it/tar.rs`**: archives through the viewer, as lists and as
  folders to go into; for tar, read on a thread of its own, a file compressed alone, and that nothing
  else names its decompressors.

- **`tests/docs.rs`**: that every relative link in the docs leads to a file, and its anchor to a
  heading. A binary of its own, so it runs without backends too.

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

Bump `version` in `Cargo.toml`, commit, tag `v<version>` and push the tag:
`.github/workflows/release.yml` builds `clv` for macOS and Linux (musl) and publishes the archives
as a GitHub release. It refuses a tag that disagrees with `Cargo.toml`. Run by hand, it only builds.
