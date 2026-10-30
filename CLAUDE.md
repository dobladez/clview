# clview — instructions for agents

A terminal viewer: pages drawn through the kitty graphics protocol, a pager over them.
`src/view/`'s README explains how it works.

## Layout
- `src/view/`: the viewer. It knows what it shows only through the traits in `src/source.rs`
  (`Source`, `Render`, `Pool`, `Frame`, `Container`), never a backend; `the_viewer_names_no_backend` checks it.
- `src/source/<backend>.rs`: one backend each, behind a cargo feature, listed in
  `source::BACKENDS`. A backend says whether it opens a file (from its first bytes) and which
  options it takes; the first that opens a file opens it. Options that belong to one kind of file
  (a PDF's `--password`) are the backend's, never the viewer's. `pdfcraft.rs` (feature `pdf`,
  default) renders PDFs through pdfcraft, a git dependency pinned in `Cargo.toml`; `image.rs`
  (feature `image`, default) decodes images through the `image` crate and jxl-oxide; `zip.rs`
  (feature `zip`, default) opens zip archives through the `zip` crate, to a `Container` rather
  than pages (`Opened::Files`), its files read from it on demand, never unpacked to disk.
- **A backend's library is its own.** pdfcraft is named only in `src/source/pdfcraft.rs`, its
  tests (`tests/it/pdfcraft*.rs`), `Cargo.toml`, the line listing it in `source::BACKENDS`, and
  the backend tables in the README and `docs/cli-view.md`; swapping the PDF renderer must touch
  nothing else. `tests/it/pdfcraft.rs` checks the code. The generic tests write their PDFs by hand
  (`tests/it/fixtures.rs`), so any PDF backend reads them.
- A backend brings its own render threads, honouring `Pool`'s contract: `set_queue` replaces the
  work wanted, a result arrives only while its request is listed, `try_recv` never blocks. A
  backend with nothing better (images) lends its render function to `source/threads.rs`.
- `source/dir.rs`: a directory as a `Container`, which lists files and the containers in it and
  opens each file as an `Input`: its files and subdirectories, in name order (`-r` for its
  subdirectories' files instead). Several files, or a container, are a list (`view/mod.rs`'s
  `Item`): a tile each, its first page, or a folder (`source/folder.rs`) to go into, and Enter
  opens one as a level of its own in the pager. `source/joined.rs`: the tiles as one source. It
  renders a few files at a time and lets go of what the others render with, so a source must hold
  little while idle: an image holds its header, not its pixels.
- Test pictures are made by the tests, or kept in `tests/data/`, synthetic, with how each was
  made in its README.

## Rules
- **Never panic.** Every file, every byte the terminal sends back and every argument is untrusted
  input. In non-test code: no `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!` unless provably
  infallible with a comment saying why; `get()` rather than indexing with input-derived positions,
  and string slicing only at char boundaries; checked or saturating arithmetic on input-derived
  numbers; allocations sized by input are capped. `unsafe_code = "forbid"`.
- **Nothing raw reaches the terminal.** File names, messages and anything a document can shape go
  through `printable` first.
- **Prove a test fails before the fix** by actually reverting the fix.
- **Rust only.**
- **The pdfcraft pin:** every `rev` in `Cargo.toml` moves together, the `[patch.crates-io]` ones
  included; `tests/it/pdfcraft_pin.rs` checks it.
- **Clean room.** Behaviour comes from specifications (the kitty graphics protocol, the tmux
  manual) and black-box observation. Never read the source of another terminal-graphics program
  or PDF viewer (kitty, tdf, timg, chafa, viu, MuPDF…): most are GPL or AGPL, and this is MIT or
  Apache-2.0.

## Quality gates before every commit
`cargo fmt --check`, `cargo clippy --all-targets -j 2 -- -D warnings`, `cargo test -j 2`, and
the viewer without backends: `cargo clippy --no-default-features --all-targets -j 2 -- -D warnings`
and `cargo test --no-default-features -j 2`. After touching `cfg` attributes, each backend alone
too (`--no-default-features --features image`, then `pdf`, then `zip`). Run them under a `timeout`. The PDF
backend is a large build: keep `-j 2` unless the machine is free. CI
(`.github/workflows/ci.yml`) runs them all on Linux and macOS, with Rust 1.95.0, the `rust-version`.

## Releases
Bump `version` in `Cargo.toml`, commit, tag `v<version>` and push the tag:
`.github/workflows/release.yml` builds `clv` for macOS and Linux (musl) and publishes the archives
as a GitHub release. It refuses a tag that disagrees with `Cargo.toml`. Run by hand, it only builds.

## Looking at it
Output is images in a terminal, so look at the result in kitty, Ghostty or WezTerm. Without a
terminal, `--protocol kitty` writes the escape codes to stdout (the integration tests read them
back), and `script(1)` gives a test a pseudo-terminal (`tests/it/view_pager.rs`).
