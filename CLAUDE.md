# clview — instructions for agents

A terminal viewer: pages drawn through the kitty graphics protocol, a pager over them. PDFs are
rendered by pdfcraft (a git dependency pinned in `Cargo.toml`). `src/view/` began as
`pdfcraft-cli view` in storytold/pdfcraft; its README explains how it works.

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

## Quality gates before every commit
`cargo fmt --check`, `cargo clippy --all-targets -j 2 -- -D warnings`, `cargo test -j 2`, under a
`timeout`. The pdfcraft engine is a large build: keep `-j 2` unless the machine is free.

## Looking at it
Output is images in a terminal, so look at the result in kitty, Ghostty or WezTerm. Without a
terminal, `--protocol kitty` writes the escape codes to stdout (the integration tests read them
back), and `script(1)` gives a test a pseudo-terminal (`tests/it/view_pager.rs`).
