# ADR-0040: Org-mode documents parsed by orgize, in a worker process, into the backend that opens HTML

- **Status:** Accepted
- **Date:** 2026-11-17
- **Deciders:** the owner chose a parser library over one of clv's own, and orgize or
  org-rust-parser by an evaluation, patched if neither were good enough, on 2026-11-17
  ([Spec-0014](../specs/0014-show-org-mode-documents.md))

## Context

[Spec-0014](../specs/0014-show-org-mode-documents.md) shows Org-mode files (`.org`) as Markdown is
shown: written as HTML that the documents' backend lays out
([ADR-0014](0014-markdown-through-pulldown-cmark-into-wordcrafts-html-importer.md)), by a backend
of their own that hands that HTML to whichever backend opens it, as the epub's does
([ADR-0038](0038-an-epub-as-one-document-its-package-read-by-rbook-in-a-worker.md)). The HTML is
clv's to write; the parsing is a library's. Two Rust crates parse Org: [orgize](https://crates.io/crates/orgize)
0.10.0-alpha.10 (a lossless tree of rowan's, built with nom; its last release 2024-06-11, its
repository quiet since 2024-07) and [org-rust-parser](https://crates.io/crates/org-rust-parser)
0.1.8 (an arena of nodes; released 2026-05-07). An Org file is untrusted input, and clv never
panics on one ([Rules](../../CONTRIBUTING.md#rules)) and bounds what one costs.

The spec left open whether the parser runs inside `caught`, if it is bounded in time and depth on
hostile input, or in a worker process capped on CPU time and heap
([ADR-0026](0026-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md)), as
merman and rbook do.

## Decision

1. **orgize 0.10.0-alpha.10**, from crates.io, pinned with `=`, its default features (none). It
   adds ten crates to clv's build (orgize, rowan, nom 7, minimal-lexical, jetscii, bytecount,
   countme, text-size, memoffset, rustc-hash 1), all MIT or Apache-2.0.
2. **In a worker process of clv's own**, `clv --worker org`: orgize is not bounded in time on
   hostile input (Evidence), and overflows a stack on input nested deep, which a `catch_unwind`
   can't catch. The worker is given the Org text, already held to the documents' limits
   ([ADR-0012](0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md): 64 MiB,
   500,000 lines and marks, counted as Markdown's are), and answers the HTML clv writes of it, or
   why not: the parse and the writing both happen there, so nothing of orgize's tree crosses the
   pipe. Capped at **2 s of CPU and 512 MiB of heap**, one worker a conversion, in turn.
3. **Two of orgize's mistakes are worked around in clv's converter, never by a parser of clv's
   own**, each by orgize parsing the text again, and each with a canary test that fails once
   orgize stops making it, saying what to remove:
   - a table after affiliated keywords (`#+NAME:`, `#+CAPTION:`, `#+ATTR_HTML:`, `#+TBLNAME:`) is
     read as a paragraph, its rule lines as strike-through: the text after the keywords is parsed
     again on its own, and what it parses to replaces the paragraph, the keywords kept with it;
   - a bracket link whose path goes on past a line's end (`[[*A Cleaner⏎Outline View]]`, as
     filling a paragraph leaves it) is no link: within such a paragraph, a line's end inside a
     link's path is made a space, and the paragraph parsed again. Upstream has the fix unmerged
     ([PoiScript/orgize#94](https://github.com/PoiScript/orgize/pull/94)).

   Bare `http`, `https` and `mailto` addresses and angle links (`<https://…>`), which orgize
   doesn't parse at all, are found in its text by the converter, as GitHub's autolinks are.

## Evidence

Apple M3 Max, macOS 26.6.2, Rust 1.95, orgize 0.10.0-alpha.10 and org-rust-parser 0.1.8 as
[`0040-measure/`](0040-measure/Cargo.toml) pins them: `cargo build --release` there, then
`target/release/measure`, whose subcommands its `src/main.rs` lists.

**The corpus**: the Org manual's sources and worg, `git clone --depth 1` of
`https://git.savannah.gnu.org/git/emacs/org-mode.git` and `https://git.sr.ht/~bzg/worg` on
2026-11-17: the 298 `.org` files under `org-mode/doc` and `worg`, 7.1 MiB. **What Org makes of
them** is the reference: Emacs 31.1's own `org-element-parse-buffer`, through
[`reference.el`](0040-measure/reference.el) (`emacs -Q --batch -l reference.el FILE…`, each file in
a buffer in `org-mode`, its `#+SETUPFILE` not read, as clv reads none), counted kind by kind; then
`measure count orgize|orgrust FILE…` counts the same of each parser's tree, and `measure compare`
puts them side by side ([`corpus-results.txt`](0040-measure/corpus-results.txt)):

| Kind | Org | orgize | org-rust-parser |
|---|---|---|---|
| Files parsed | 298 | 298 | 296: it panics on two |
| Headlines | 5,527 | 5,527, every file | 5,504: none past level 6 |
| … with a TODO keyword, with tags | 153, 104 | 153, 104 | 136 (it knows no `#+TODO:`), 99 |
| Planning lines | 24 | 24 | none |
| Property drawers, drawers | 1,147, 6 | 1,147, 6 | 1,143, 10 |
| Bracket links | 5,080 | 4,938 (97%): 89 of the manual's 508 wrap a line | 5,025 |
| Plain and angle links | 504 | none: it doesn't parse them | 586 |
| Footnote references, definitions | 378, 193 | 376, 193 | 377, 193 |
| Source, example, quote, special blocks | 1,379, 945, 97, 80 | 1,379, 945, 97, 79 | 1,362, 945, 97, 79 |
| Tables, their rows | 328, 8,100 | 305, 6,376: 23 after affiliated keywords | 328, 8,100 |
| Lists, items, checkboxes | 2,196, 9,594, 625 | 2,214, 9,594, 625 | 2,815, 9,605, 625 |
| Fixed-width lines, timestamps | 1,265, 52 | 1,264, 77 | none, none |
| Bold, italic, verbatim, code | 886, 1,197, 11,389, 3,891 | 882, 1,179, 11,207, 3,873 | 725, 1,126, 10,636, 3,095 |
| Strike-through | 6 | 45: the tables read as paragraphs | 176 |

So orgize agrees with Org on every file for headlines and their decorations, planning, drawers,
blocks and checkboxes, and its misses come from the two mistakes worked around (tables and the
strike-through in them; links that wrap) and from plain links, which the converter finds. org-rust-parser
misses whole kinds Org has (planning, timestamps, fixed-width, a file's own TODO keywords,
headlines past level 6), and writes to stderr while it parses (`dbg!`: `caption applied to
invalid object`), which in a pager would reach the terminal.

**Panics**: `measure cut <parser> <seed> 20000 FILE…`, a property test of the measure's own: each
round a file of the corpus or of `org-mode/testing` (336 files) cut at two places anywhere, its
prefix or the slice between kept, or two files' pieces joined, or up to eight bytes changed to
Org's marks; parsed under `catch_unwind`.

| Parser | Rounds | Panicked |
|---|---|---|
| orgize, release (seeds 1 and 7, with debug assertions as clv's tests build it: `--profile checked`) | 20,000 each | 0 |
| org-rust-parser, release (seed 1) | 2,179 before it was killed | 20, the first 20 kept; then the process grew until macOS killed it (SIGKILL) |

**Time and heap**: `measure time orgize FILE…`; the Org manual, 0.9 MiB and 121,175 lines and
marks, parses in 13 ms and 5.4 MiB (org-rust-parser: 19 ms and 25 MiB); the corpus's largest file,
1 MiB, in 2.5 ms; the manual four times over, 3.4 MiB and 485,000 lines and marks, about as much as
the documents' limit lets through, in 50 ms and 21 MiB.

**Hostile input**, [`hostile.sh orgize`](0040-measure/hostile.sh), each kind at 1,000 to 1,000,000,
a process each, on a thread with the main thread's 8 MiB stack, killed at 60 s
([`hostile-results.txt`](0040-measure/hostile-results.txt)):

| Kind | Within the documents' limits | Time | Heap |
|---|---|---|---|
| Emphasis nested 10,000 deep, `*/_+…x…+_/*` | 19 KiB | 1.6 s | 1 MiB |
| Quote blocks nested 10,000 deep | 253 KiB | 1.3 s | 4 MiB |
| Special blocks nested 10,000 deep, each its own name | 251 KiB | 6.8 s | 4 MiB |
| 10,000 `#+begin_src` never closed | 117 KiB | 1.3 s | 2 MiB |
| 10,000 drawers opened, never closed | 87 KiB | 1.2 s | 0 |
| Emphasis opened and never closed, one line of 10,000 | 29 KiB | 1.2 s | 0 |
| `[[` 100,000 times on a line | 195 KiB | 12 s | 0 |
| Footnotes nested 100,000 deep, `[fn:: [fn:: …]]` | 683 KiB | stack overflow, abort | |
| A list nested 10,000 deep, headlines 100,000 deep | over 64 MiB | stack overflow, abort | |
| A table of 41,000 rows of 10 cells, about the limit's marks | 1.6 MiB | 86 ms | 97 MiB |
| Each of 100,000 lines of prose `/x/ *y* [[l][d]] =c=` | 6.6 MiB | 113 ms | 119 MiB |

Time grows as the square of what is nested or left open, so the documents' limits, which bound
Markdown's parse by its marks, don't bound orgize's: 19 KiB takes 1.6 s, and the half million marks
allowed would take hours. Real files are four orders below the CPU cap (50 ms at the limit); the
costliest heap within the limits, a table, is a fifth of the heap cap, which also holds the HTML
written (up to 64 MiB).

**How much larger the HTML is**: `measure html FILE…`, through orgize's own exporter, writes the
corpus as 0.82 times its size (drawers and keywords left out), a file at 1.06 times its size at the
median and 1.37 at most. clv's own, with its ids, styles and title block, measured through the
backend (`ORG_CORPUS=<a list of the corpus's paths> cargo test -j 2 --bin clv
org::tests::how_much_larger_the_html_is -- --ignored --nocapture`): 0.75 times the corpus's size, a
file over 4 KiB at 1.33 times at most (worg's `contributors.org`, a list of links); and 0.34 times
its lines and marks, as HTML counts them (each `<` and line's end). So an Org file within the
documents' limits comes to HTML within HTML's, but for one made of little else than a table's
cells: a row of ten cells is twelve lines and marks of Org and twenty-two of HTML, and 40,000 such
rows are refused for their HTML (`an_org_file_is_held_to_the_documents_limits_and_its_html_to_htmls`).

**Through clv** (its dev profile, the dependencies optimized; `/usr/bin/time -l clv x.org --protocol
kitty --pages 1`): the Org manual to its first page, worker, parse, HTML and layout of the whole,
in 0.83 s and 186 MB; the manual four times over, at the limit's marks, in 3.2 s and 655 MB, most of
it the documents' backend's layout. Special blocks nested 10,000 deep, and footnotes nested
90,000 deep, are refused in under 7 s each, the worker killed past its CPU time
(`a_hostile_org_file_costs_the_document_and_never_the_viewer`).

## Alternatives considered

- **A parser of clv's own**: the owner's decision is a library, and a parser of the corners Org
  has (affiliated keywords, drawers, planning, footnotes of three kinds, the TODO keywords a file
  declares) is a project of its own.
- **org-rust-parser**: it panics on real files and on one in a hundred of the cuts, grows without a
  bound on some, writes to stderr while parsing, and leaves out kinds Org has.
- **orgize inside `caught`, in the viewer's process**: a panic is caught, but a stack overflow
  aborts the viewer, and nothing bounds a parse that takes seconds; the worker's kernel limit on
  CPU time and allocator's cap on heap do.
- **orgize patched, from a fork pinned by `rev`** (as pdfcraft is pinned), with the two fixes:
  a fork to keep, and a pin that can't follow crates.io. The workarounds are small, use orgize to
  parse, and their canaries say when they can go.
- **orgize's own HTML exporter**: it writes what Org hides (drawers' contents aside, keywords'
  values in places, comments as HTML comments), knows no places or TODO colours, sends `file:`
  links as they are and inlines remote images; clv's converter writes what the spec says Org
  shows.

## Consequences

- **Positive:** Org files show with what Markdown has (layout, dark paper, search, pictures,
  links), orgize's tree read for every kind Org has; nothing orgize does can cost more than the
  document.
- **Accepted:**
  - A worker process is started for every Org file opened, and laid out again (a few
    milliseconds each), one at a time.
  - A file nested past what orgize parses in 2 s is refused, with a message, where Emacs would
    show it.
  - The two workarounds parse a paragraph twice where they apply.
- **Follow-ups:** the workarounds come out when orgize fixes its mistakes (their canaries fail);
  a newer orgize is measured again with `0040-measure/`.
- **Revisit when:** orgize moves (measure again: the corpus, the cuts and the hostile kinds), or
  is abandoned for good; or another Org parser in Rust appears.
