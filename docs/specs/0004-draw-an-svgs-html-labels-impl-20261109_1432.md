# Spec-0004: implementation notes (2026-11-09)

Notes made while implementing [Spec-0004](0004-draw-an-svgs-html-labels.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 1: measured

- **Where the tool lives:** the owner put it in `docs/adr/0024-measure/` on this branch, a crate of
  its own with its own `[workspace]`, rather than on the spike's branch as the task's wording has
  it; nothing was committed to the spike. It isn't in the task's *Files:*. Its fixtures are
  generated (`measure gen`), never kept; the real drawings it was compared on are clients' files,
  named in the spike's `spike/NOTES.md`.
- **One document holds,** with one change from the spec's sketch: the slots are placed by
  `transform: translate()`, 8 CSS pixels apart. Placed by `left`/`top`, each label's recording
  carried an empty clip layer for every slot between half its position and its own: Blitz's paint
  culls by `scrollable_overflow ∪ border_box`, and the border box is in its parent's coordinates
  while the transform it goes through already holds the position. Slots that touched weren't
  culled either, hence the gap. Seen with `COUNTS=1` and `DUMP=<i>`.
- **The packing** first held rows to 65,536 CSS pixels; with `overflow: visible` labels (slots
  8,192 pixels larger) that made the document 1.2 million pixels tall, where an f32 is a tenth
  of a pixel apart. Rows are now about as wide as all the slots would be square.
- **A transparent fill** the size of the whole document starts every label's recording (the root's
  background, painted whatever the viewport). It draws nothing; task 3 may drop it.
- **Limits that moved** from the spec's starting table: label elements 100,000 (new), commands
  200,000 (from a million), overflow 4,096 CSS pixels (the spec named no number); and the nesting
  limit's thread. The owner chose a thread of 8 MiB for Blitz (option C) over 64 on 2 MiB sized
  by release builds, or 32 on 2 MiB. Each is reasoned in ADR-0024.
- **One document a label** for 1,024 labels under 3 MiB of styles was stopped after 10 minutes:
  this stylesheet costs 1.7 s a label (16 labels: 27 s), more than the spike's (111 s for 1,024).
- **wuff** sizes a WOFF 1 table's output by its header's `origLength`, unbounded: the hostile font
  is a WOFF 1. WOFF 2 it caps itself (128 MiB, a ratio of 100). The header must say the file's
  padded length, or wuff refuses it before unpacking.
- **Blitz requests a repeated `data:` URL once** a document (1,024 labels of 256 distinct pictures
  made 256 requests).
- **Verify:** every row of the ADR's tables was run again from `docs/adr/0024-measure/` under
  `/usr/bin/time -l`; the numbers are the second run's, within a few percent of the first.
