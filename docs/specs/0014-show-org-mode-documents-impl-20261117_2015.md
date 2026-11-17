# Spec-0014: implementation notes (2026-11-17)

Notes made while implementing [Spec-0014](0014-show-org-mode-documents.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 1: the parser and its ADR

[ADR-0040](../adr/0040-org-mode-through-orgize-in-a-worker-process.md); its harness and results in
`docs/adr/0040-measure/`.

- **orgize**, as the spec's evaluation expected; org-rust-parser lost on every count (panics on two
  corpus files and one cut in a hundred, then runs away in memory; `dbg!` to stderr; no planning,
  timestamps, fixed-width, nor a file's `#+TODO:`).
- **The reference was Emacs itself**: `emacs` is installed (31.1), so `reference.el` counts what
  `org-element-parse-buffer` makes of each corpus file, and each parser is compared kind by kind.
  Not in the spec's plan, and the strongest measure of fidelity available.
- **A worker process, not `caught`**: orgize's time grows as the square of what is nested or left
  open (19 KiB of nested emphasis takes 1.6 s), and nested footnotes or lists overflow a stack.
  The documents' limits bound Markdown's parse, not orgize's. Caps chosen at 2 s and 512 MiB from
  the measure; confirmed through the backend in task 2.
- **Two orgize mistakes, worked around rather than patched in a fork**: a table after affiliated
  keywords read as a paragraph (23 of the corpus's 328 tables), and a bracket link whose path
  wraps a line (89 of the manual's 508 links; upstream PR #94, unmerged). How to host a patch was
  put to the owner (through the team lead) with a recommendation for workarounds with canaries,
  as Blitz's; the ADR says workarounds. Revisit if the owner prefers a fork.
- **Plain and angle links** aren't parsed by orgize at all: the converter finds them in its text.
- The cut test found a bug in the harness's own table of marks first (an index past its end);
  fixed before the numbers were taken.
