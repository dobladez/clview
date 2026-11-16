# Architecture Decision Records

An ADR records why clv does something one way and not another, as it stood when it was decided:
the forces, the options, what was measured, and what would make it worth deciding again. How
clv works today is in [`docs/architecture.md`](../architecture.md) and
[`docs/usage.md`](../usage.md); an ADR doesn't repeat them, it links to them, and they link
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
| 0007 | [vectorcraft drawings through its format and renderer](0007-vectorcraft-drawings-through-its-format-and-renderer.md) | Superseded by ADR-0008 |
| 0008 | [No vectorcraft backend for its own format alone](0008-no-vectorcraft-backend-for-its-own-format-alone.md) | Accepted |
| 0009 | [Copy a page through the terminal's clipboard protocol, else the system's clipboard](0009-copy-a-page-through-the-terminals-clipboard-protocol-else-the-systems.md) | Accepted |
| 0010 | [A filtered list keeps the whole list's tile size](0010-a-filtered-list-keeps-the-whole-lists-tile-size.md) | Accepted |
| 0011 | [Text cells where the terminal has no graphics protocol](0011-text-cells-where-the-terminal-has-no-graphics-protocol.md) | Accepted |
| 0012 | [Bound what Markdown and HTML cost by their marks and their text](0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md) | Accepted |
| 0013 | [A document's pictures are drawn by the backends, and only from near it](0013-a-documents-pictures-are-drawn-by-the-backends.md) | Accepted; a document held in an archive having nothing near it superseded by ADR-0037 |
| 0014 | [Markdown through pulldown-cmark into wordcraft's HTML importer](0014-markdown-through-pulldown-cmark-into-wordcrafts-html-importer.md) | Accepted |
| 0015 | [Markdown and HTML on dark paper, the terminal's own colour](0015-markdown-and-html-on-dark-paper-the-terminals-own-colour.md) | Accepted; `--theme` being the viewer's superseded by ADR-0021; its key `d` moved to `D` by ADR-0036 |
| 0016 | [How Markdown and HTML look](0016-how-markdown-and-html-look.md) | Accepted |
| 0017 | [HTML's style rules apply, as far as wordcraft reads them](0017-htmls-style-rules-apply-as-far-as-wordcraft-reads-them.md) | Accepted |
| 0018 | [wordcraft stays before its proofing needs a checkout to build](0018-wordcraft-stays-before-its-proofing-needs-a-checkout-to-build.md) | Accepted |
| 0019 | [A list opens its files when their tiles are wanted](0019-a-list-opens-its-files-when-their-tiles-are-wanted.md) | Accepted |
| 0020 | [New versions through a Homebrew tap, not a check inside clv](0020-new-versions-through-a-homebrew-tap-not-a-check-inside-clv.md) | Accepted |
| 0021 | [A backend declares its settings, and their options hold for any file](0021-a-backend-declares-its-settings-and-their-options-hold-for-any-file.md) | Accepted; the viewer's keys first set aside for the zoom keys by ADR-0023; a setting's sentences put together by the viewer by ADR-0033; its keys suggestions the keymaps take by ADR-0036 |
| 0022 | [Markdown and HTML reflow to a width in characters, as a new level](0022-markdown-and-html-reflow-to-a-width-in-characters-as-a-new-level.md) | Accepted; its US Letter page superseded by ADR-0023 |
| 0023 | [The window is the page of Markdown and HTML](0023-the-window-is-the-page-of-markdown-and-html.md) | Accepted; its pages to a row given keys of their own by ADR-0024 |
| 0024 | [Pages to a row of Markdown and HTML are columns, and the last key wins](0024-pages-to-a-row-of-markdown-and-html-are-columns-and-the-last-key-wins.md) | Accepted |
| 0025 | [Draw an SVG's HTML labels with Blitz, one document a drawing, each stage bounded](0025-draw-an-svgs-html-labels-with-blitz-one-document-a-drawing.md) | Accepted; its estimate and its thread superseded by ADR-0026 |
| 0026 | [An SVG's HTML labels in a worker process, capped on CPU time and heap](0026-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md) | Accepted |
| 0027 | [A PDF's text read in a worker process, for its heap cap](0027-a-pdfs-text-read-in-a-worker-process-for-its-heap-cap.md) | Accepted |
| 0028 | [A search's matches tinted into the page's pixels, the page rendered again when its tint changes](0028-matches-tinted-into-the-pages-pixels.md) | Accepted |
| 0029 | [The pager keeps 256 MiB of pages in the terminal](0029-the-pager-keeps-256-mib-of-pages-in-the-terminal.md) | Accepted |
| 0030 | [A document's text read from the pool's layout, on a thread](0030-a-documents-text-read-from-the-pools-layout-on-a-thread.md) | Accepted |
| 0031 | [Mermaid diagrams through merman, in a worker process capped at 2 s and 512 MiB](0031-mermaid-diagrams-through-merman-in-a-worker-process.md) | Accepted |
| 0032 | [A fenced block is drawn by the backend that lists its language](0032-a-fenced-block-is-drawn-by-the-backend-that-lists-its-language.md) | Accepted |
| 0033 | [Backends share a setting by declaring its name, and the viewer says of it only what they declare](0033-backends-share-a-setting-by-declaring-its-name.md) | Accepted; what a key says where its setting isn't, updated 2026-11-16 |
| 0034 | [Open a file through the platform's opener, by its real path, only where its name agrees](0034-open-a-file-through-the-platforms-opener-by-its-real-path.md) | Accepted |
| 0035 | [The links' labels are placements of one image of letters, over the page](0035-link-labels-as-placements-of-one-image-of-letters.md) | Accepted |
| 0036 | [Keys are bound to named commands](0036-keys-are-bound-to-named-commands.md) | Accepted |
| 0037 | [What is beside a held file is read from its holder, a picture measured from its first 64 KiB](0037-what-is-beside-a-held-file-is-read-from-its-holder.md) | Accepted |
| 0038 | [An epub is one document through the backend that opens HTML, its package read by rbook in a worker process capped at 2 s and 256 MiB](0038-an-epub-as-one-document-its-package-read-by-rbook-in-a-worker.md) | Accepted; the Contents page last and a key to the contents, updated 2026-11-16 |
| 0039 | [A backend sure of a file wins over one that finds a mark in it](0039-a-backend-sure-of-a-file-wins-over-one-that-finds-a-mark-in-it.md) | Accepted |

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
  delete one. Correcting a link whose target has moved isn't rewriting.
- Dates are absolute (`2026-10-30`).
- Add each new ADR to the index above.
