# Change specs

A spec is one slice of buildable work, written before its code: what it needs to do, how it
will be built, and how we will know it's done. Specs are a record of changes, in order: once one
is `Done` it describes a moment, and isn't rewritten. What clv does now is in the docs a change
updates as it lands ([Where a fact goes](../../CONTRIBUTING.md#where-a-fact-goes)): a spec links
to them, never stands in for them. Why a choice was made is an [ADR](../adr/README.md)'s.

What gets a spec: a new capability, a change to what clv does, a refactor large enough to take
more than one commit. A one-commit fix needs none.

## Working here

Start from [`template.md`](template.md). [`project.md`](project.md) holds this project's
commands, code roots and conventions, the one file the skills read for them.

The skills are in `.agents/skills/` (`.claude/skills` and `.codex/skills` link there), and are
invoked however the agent offers them (`/spec-new` in Claude Code, `$spec-new` in Codex):

- `spec-new <description>` writes one through a short interview.
- `spec-new-auto <description>` researches and writes one unattended, as a `Draft`.
- `spec-implement <NNNN>` claims a `Ready` spec and takes it to `Done`, a commit per task.
- `spec-update <NNNN>` reviews and tightens one not yet `Done`, without writing code.
- `spec-list [bucket]` shows the queue.

`spec-list` runs `cargo run -q --no-default-features --example spec_list`
([`examples/spec_list.rs`](../../examples/spec_list.rs)), which takes a bucket
(`implementing`, `ready`, `draft`, `shelved`, `done`), or `--index` to rewrite the Index below.
`cargo specs` is the same, an alias in [`.cargo/config.toml`](../../.cargo/config.toml):
`cargo specs ready`, `cargo specs --index`.

## Index

Made from the specs' headers: never edit it by hand. `--index` rewrites it, and `tests/docs.rs`
fails while it disagrees with them.

| # | Title | Status |
|---|---|---|
| 0001 | [Let a file declare its settings and their keys](0001-settings-a-file-declares.md) | Done |
| 0002 | [Reflow Markdown and HTML to a width in columns](0002-reflow-markdown-and-html-to-a-width.md) | Done |
| 0003 | [Markdown and HTML on pages the window's size, and a font size](0003-lay-markdown-and-html-out-to-the-window.md) | Done |
| 0004 | [Draw an SVG's HTML labels](0004-draw-an-svgs-html-labels.md) | Done |
| 0005 | [Pages to a row on their own keys, and a document's margins](0005-set-pages-to-a-row-and-margins.md) | Done |
| 0006 | [Search a PDF's text](0006-search-a-pdfs-text.md) | Done |
| 0007 | [Search Markdown, HTML and Word documents](0007-search-markdown-html-and-word-documents.md) | Done |
| 0008 | [Show Mermaid diagrams](0008-show-mermaid-diagrams.md) | Ready |
| 0009 | [Draw a document's fenced blocks as pictures](0009-draw-fenced-blocks-as-pictures.md) | Ready |

## Lifecycle

`Ready → Implementing → Done`, with `Draft` before `Ready` now and then, and `Shelved` or
`Superseded by Spec-NNNN` out of the queue.

- **Ready**: written and committed, for anyone to pick up. A reader can tell the slice
  (Summary), what's left out (Out of scope), the first task (Tasks), the evidence it's done
  (Done), and what blocks it (🔴 in Open questions). If any of those is missing or vague, it's a
  `Draft`.
- **Implementing**: `spec-implement` has claimed it, so no one else picks it up.
- **Done**: shipped, its `Done` checklist passed. `spec-implement` makes it so.
- **Draft**: committed before its scope was settled. Lifted to `Ready` before it is implemented.
- **Superseded by Spec-NNNN**: replaced by a later spec, linked; the file stays.
- **Shelved**: not to be built now. The file stays, with why and, where it matters, what leaving
  it costs. Lifted back to `Draft` or `Ready` to pick it up again. `spec-list` still lists shelved
  and superseded specs, under their own heading.

## When a spec is Done

Its `Done` checklist covers what the spec itself promised. The change also updates what any
commit would, by [Where a fact goes](../../CONTRIBUTING.md#where-a-fact-goes):

- `CHANGELOG.md`: a line under `## Unreleased`, if a user would notice.
- `docs/usage.md`: what a user would notice, rewritten to say what clv does now.
- `docs/architecture.md`: what it names of the code, and the numbers a design rests on.
- `docs/adr/`: an ADR for a choice between real alternatives, in the commit that makes it.
- README's Plans: the bullet this delivered comes out.

## Capturing discoveries

Writing or implementing a spec turns up things that belong elsewhere. Say so when they come up,
and propose where they go; don't leave them in the notes or the conversation.

| What turned up | Where it goes |
|---|---|
| Something clv does that no doc says | `docs/usage.md`, or `docs/architecture.md` if only the code would notice |
| A future feature | README's Plans, a line, with its details in `docs/backlog.md`; or the backlog alone, not yet planned; or `spec-new-auto` for a `Draft` |
| A choice between real alternatives, made | An ADR |
| A rule the code should keep | `CONTRIBUTING.md`'s Rules |
| A decision not yet made | The spec's Open questions, or README's Plans |
| What only this spec's later readers need | Its implementation notes |

## Implementation notes

While a spec is being implemented, its notes are kept beside it, in
`{spec}-impl-{YYYYMMDD_HHMM}.md`: for `0003-show-avif-pictures.md`,
`0003-show-avif-pictures-impl-20261110_1430.md`, so they sort together. They record decisions the
spec didn't make, changes from its plan and why, and surprises. `spec-implement` opens the file
and links it from the spec's Changelog.

## Conventions

- Numbered in order, four digits, never reused. File name: `NNNN-change-in-kebab-case.md`.
- One slice per spec. Two entangled changes are two specs, linked.
- The header is bullets, as in the ADRs, not front matter.
- Link what a spec rests on by path: `[ADR-0019](../adr/0019-a-list-opens-its-files-when-their-tiles-are-wanted.md)`,
  `[Limits](../usage.md#limits)`. `tests/docs.rs` checks every link leads somewhere.
- Dates are absolute (`2026-11-06`).
- Superseding one: its status line becomes `Superseded by Spec-NNNN`, linked. Never delete one.
