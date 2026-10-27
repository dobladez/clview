# clview — instructions for agents

A terminal viewer: pages drawn through the kitty graphics protocol, a pager over them. `src/view/`
began as `pdfcraft-cli view` in storytold/pdfcraft; its README explains how it works.

## Layout
- `src/view/`: the viewer. It knows what it shows only through the traits in `src/source.rs`
  (`Source`, `Render`, `Pool`, `Frame`), never a backend; `the_viewer_names_no_backend` checks it.
- `src/source/<backend>.rs`: one backend each, behind a cargo feature, opened from
  `source::open`. `pdfcraft.rs` (feature `pdfcraft`, default) renders PDFs through pdfcraft, a git
  dependency pinned in `Cargo.toml`.
- A backend brings its own render threads, honouring `Pool`'s contract: `set_queue` replaces the
  work wanted, a result arrives only while its request is listed, `try_recv` never blocks.

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
and `cargo test --no-default-features -j 2`. Run them under a `timeout`. The pdfcraft engine is a
large build: keep `-j 2` unless the machine is free.

## Looking at it
Output is images in a terminal, so look at the result in kitty, Ghostty or WezTerm. Without a
terminal, `--protocol kitty` writes the escape codes to stdout (the integration tests read them
back), and `script(1)` gives a test a pseudo-terminal (`tests/it/view_pager.rs`).
