# Specs: this project

The one file the `spec-*` skills read to learn this project: its commands, where its code is,
and its conventions, so the skills say nothing of their own about it. Read on every invocation:
keep it short.

## Commands

| Purpose | Command |
|---|---|
| The quality gates, before every commit | The five in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates), each under a `timeout` |
| Other feature sets (no backends, `pdf,svg`) | CI's nightly run, or `gh workflow run CI`: never locally |
| One test, while working | `cargo test -j 2 --test it <name>`, or `--bin clv <name>` for a unit test |
| The docs' links and the specs' Index | `cargo test -j 2 --test docs` |
| The queue | `cargo specs [bucket]`, an alias of `cargo run -q --no-default-features --example spec_list` |
| Rewrite the Index | `cargo specs --index` |

Keep `-j 2`: the PDF backend is a large build.

## Code

- **Rust only**, 1.95 or later. What each dependency is for is said beside it in `Cargo.toml`,
  and why it was chosen in its ADR.
- **Where to look** when researching a spec: `src/` (`src/view/` the viewer, `src/source/` the
  backends; [`CLAUDE.md`](../../CLAUDE.md)'s Layout says which is which), `tests/it/`, and
  [`docs/architecture.md`](../architecture.md).
- **Generated, never edited by hand:** this directory's Index.

## Docs

Skip any a project doesn't have.

| Role | Path |
|---|---|
| What clv does, for a user | `docs/usage.md` |
| How it works, for the code | `docs/architecture.md` |
| Why one way and not another | `docs/adr/` |
| What changed, by release | `CHANGELOG.md` |
| What is planned | `README.md`'s Plans, a line each; their details, and ideas not yet planned, in `docs/backlog.md` |
| Rules, gates, tests, and where a fact goes | `CONTRIBUTING.md` |
| For agents | `CLAUDE.md` |

## Conventions

- **The rules in `CONTRIBUTING.md` hold for every task**: never panic, nothing raw reaches the
  terminal, a backend's library named only in its own file, a backend bringing its own render
  threads.
- **A test is seen failing before its fix**, by actually reverting the fix. Copy the file aside
  first and copy it back: `git checkout` restores what was there before the task, not before the
  revert.
- **Clean room.** Research reads specifications and watches terminals and programs from outside,
  never the source of another terminal-graphics program or viewer (kitty, tdf, timg, chafa, viu,
  MuPDF and so on).
- **No new dependency** without the owner's word. Proposing one in a spec is fine; adding it
  comes with its `Cargo.toml` comment and its ADR.
- **Looking at it** is in kitty, Ghostty or WezTerm. A manual check says which, and what to see.
- **A commit per task**, on a `spec-NNNN` branch made from `main`, never pushed.
- **Titles** are short and imperative, from the description, not the slug: "let a list let go of
  its files" makes "Let go of a list's files once the reader moves on".
