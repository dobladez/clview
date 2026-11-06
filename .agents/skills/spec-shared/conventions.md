# Spec skills: shared conventions

Read this before the skill's own recipe. It holds what the `spec-*` skills have in common; each
recipe carries only what is its own.

**Nothing about this project lives here.** Its commands, code and docs are in
`docs/specs/project.md`: read that too, and trust it over any path you remember. The lifecycle,
what to update at `Done` and where discoveries go are in `docs/specs/README.md`.

## Host adaptations

These skills serve more than one agent host (Claude Code, Codex, …):

- The user's text after the skill's name is the recipe's `$ARGUMENTS`.
- `AskUserQuestion` means the host's structured question (Claude's `AskUserQuestion`, Codex's
  `request_user_input`), or else a short plain question, and **waiting for the answer**. A draft
  of a section to react to is always plain text, never a multiple choice.
- `/spec-implement` and the rest mean the shared skill, however this host invokes it.

## Finding specs

Specs are `NNNN-<kebab-slug>.md` in `docs/specs/`. `README.md`, `template.md`, `project.md` and
implementation notes (`*-impl-*.md`) are not specs. A spec's `- **Status:**` line says where it
stands; `cargo run -q --no-default-features --example spec_list` lists them all by it.

- **Number**: the highest there + 1, four digits. Never reuse one. `0001` if there are none.
- **Slug**: kebab-case from the description (lower case, digits, hyphens), without filler ("the",
  "a", "support for").

Statuses: `Draft`, `Ready`, `Implementing`, `Done`, `Shelved`, `Superseded by Spec-NNNN`.

**The Index** in `docs/specs/README.md` is made from the specs' headers. After writing a spec or
changing its status or title, rewrite it with `--index` (`project.md` has the command); never by
hand.

## References

Every link must lead somewhere before a spec is written: an ADR to its file, a spec to its file,
a doc section to its heading (`../usage.md#limits`). `tests/docs.rs` fails on one that doesn't.
Each recipe says whether to ask, correct or drop one that doesn't.

## Filling the template

**Tasks.** One checkbox per task: a session's work, and a commit that builds and passes the
gates on its own. What must ship together is one task. If a task might not fit a session, it's too
big.

**Verify.** Prefer a command from `project.md`, naming the test. A manual check is specific: which
terminal, what to press, what to see ("in kitty, open `tests/data/x.pdf`, press `n`: page 2"),
never "check it works". Include the unhappy path when it matters: a broken file, a terminal
without the protocol, a pipe.

**Tests.** clview tests what its users see: printed escape codes through a pipe (`tests/it/view.rs`),
the pager on a pseudo-terminal (`tests/it/view_pager.rs`), tmux (`tests/it/view_tmux.rs`), a
backend through the viewer. A task's test sits where its kind already is; `CONTRIBUTING.md`'s
Tests says which.

**Approach.** Only the files the implementer will touch or read. A file that shows the pattern
beats a description of it. Record what is decided, so it isn't argued again.

**Size.** Small (1–3 files): Summary and one task, no Approach. Medium (4–20 files): the whole
template, 2–4 tasks. Large (20+ files): say it should be split, rather than writing it. A bug fix:
Summary and one task. A spike: Summary and a time box.

Never pad Tasks or Done with filler. Work you can't break down is a gap in the research, not in
the formatting.

## Capturing discoveries

Work on a spec turns up what belongs elsewhere: a later feature, an unanswered question, a rule
worth keeping, something clv does that no doc says. **Raise it when it comes up**, and propose
where it goes, by the table in `docs/specs/README.md`. Not in the notes, nor the spec's Out of
scope, nor the conversation.

## Constraints

- `CONTRIBUTING.md`'s rules hold: never panic, nothing raw reaches the terminal, a backend's
  library named only in its own file, Rust only, a test seen failing before its fix.
- **Clean room.** Research reads specifications (the kitty graphics protocol, the tmux manual) and
  watches terminals and programs from outside. Never read the source of another terminal-graphics
  program or viewer, nor fetch it from the web.
- Proposing a dependency is fine; **adding one without the owner's word is not**.
- Follow the code's conventions over your own.
- Each spec focused and shippable on its own.
- Absolute dates (`YYYY-MM-DD`).
