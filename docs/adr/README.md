# Architecture Decision Records

An ADR records why clv does something one way and not another, as it stood when it was decided:
the forces, the options, what was measured, and what would make it worth deciding again. How
clv works today is in [`src/view/README.md`](../../src/view/README.md) and
[`docs/cli-view.md`](../cli-view.md); an ADR doesn't repeat them, it links to them, and they link
back to it where the choice shows.

Use [`template.md`](template.md) as the starting point.

## Index

| # | Title | Status |
|---|---|---|
| 0001 | [Delete pages printed inside tmux once no pane holds them](0001-delete-pages-printed-inside-tmux-once-no-pane-holds-them.md) | Accepted |
| 0002 | [Render SVG with resvg](0002-render-svg-with-resvg.md) | Accepted |
| 0003 | [Bound what a drawing costs before usvg and resvg spend it](0003-bound-what-a-drawing-costs-before-usvg-and-resvg-spend-it.md) | Accepted |
| 0004 | [Read a compressed tar through once, into a file nothing names](0004-read-a-compressed-tar-through-once-into-a-file-nothing-names.md) | Accepted |
| 0005 | [Word documents through wordcraft's readers, layout and renderer](0005-word-documents-through-wordcrafts-readers-layout-and-renderer.md) | Accepted |
| 0006 | [Spreadsheets printed by gridcraft to a PDF the PDF backend shows](0006-spreadsheets-printed-by-gridcraft-to-a-pdf-the-pdf-backend-shows.md) | Accepted |

## When to write one

Write an ADR when a choice between real alternatives is made, and a later reader (or agent) would
otherwise undo it, or redo the work that settled it:

- **A choice resting on measurement or observation**: a terminal's behaviour, a cost, a limit's
  number. Above all when the obvious fix was tried, or measured, and rejected.
- **A dependency or backend**: a library picked over others, a feature left out (zstd, AES).
- **A rule that constrains later work**: the clean room, never panicking, a backend's library
  named only in its own file.
- **A format or protocol choice**: how pages reach the terminal, what goes through tmux.

Don't write one for:

- a decision not yet made: say so in the conversation, or in the README's Plans;
- keeping what clv already does because a suspected problem turned out not to be one: what was
  learned goes in the doc that describes the behaviour, where the next person will look;
- what a test already pins and its name explains;
- how something works rather than why.

## Conventions

- Number sequentially, four digits, never reuse. File name: `NNNN-decision-in-kebab-case.md`.
- Statuses: `Proposed`, `Accepted`, `Superseded by ADR-NNNN`, `Deprecated`.
- One decision per ADR. If two decisions are entangled, write two and link them.
- Keep it short: context, decision, evidence, alternatives, consequences.
- **Evidence says how to get it again**: the versions (kitty 0.49.2, macOS 26), the setup (a
  visible window, not a hidden one), and the commands or escape codes, so it can be rerun rather
  than trusted. Numbers that weren't measured are said to be estimates.
- **Revisit when** names what would reopen it: a new terminal version, a fixed upstream bug, a
  new need.
- Accepted ADRs aren't rewritten. A later finding goes in a dated `Update` at the end; a changed
  decision is a new ADR, and the old one's status line says `Superseded by ADR-NNNN`. Never
  delete one.
- Dates are absolute (`2026-10-30`).
- Add each new ADR to the index above.
