# ADR-0014: Markdown through pulldown-cmark into wordcraft's HTML importer

- **Status:** Accepted
- **Date:** 2026-11-02

## Context

Markdown first went through wordcraft's own Markdown importer
([ADR-0012](0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md)). Pictures were then
wanted ([ADR-0013](0013-a-documents-pictures-are-drawn-by-the-backends.md)), and that importer can't
be given any: it makes a picture of a `data:` URL only, and anything else its alt text. Its HTML
importer takes a loader. Looking at real files also showed that the Markdown importer reads little of
the HTML a README holds: `<p align="center"><img src=…>` was shown as its tags.

## Decision

- **Markdown goes through [pulldown-cmark](https://crates.io/crates/pulldown-cmark) into HTML, and that
  through wordcraft's HTML importer** (`markdown_as_html`, `marked_up` in
  `src/source/wordcraft.rs`), with GitHub's tables, strikethrough, task lists and footnotes.
- **What the importer would leave out is written otherwise**: a task's checkbox as `☑` or `☐` (it
  skips `<input>`), and a table with `border="1"` (without it, it has no grid).
- **Front matter (`---`) is left out**, which pulldown-cmark does when asked.
- **The limits of ADR-0012 stay, counted on the Markdown** before it is converted.

## Evidence

pulldown-cmark 0.13.6 (MIT, pure Rust), wordcraft at `5b5810fe1f6f815acc3fda27619f61d9bc08b615`, Apple
M3 Max, macOS 26.6.2, clv's dev profile (optimized).

Among the 2,204 Markdown files of one developer's checkouts (leaving out dependencies), 104 refer to a
picture, and 43 of those use `<img>` (226 of 1,391 references): a regular expression over each file,
fenced code removed.

The same Markdown, each way, by eye on `clv … --protocol kitty` (headings, emphasis, code, a nested
list, a quote, a code block, a table, a footnote, `<details>`, a centred `<img>`, a task list):

| | Markdown importer | pulldown-cmark and the HTML importer |
|---|---|---|
| Raw HTML | Shown as its tags | Read: `align="center"` centres, `<details>` shows its text |
| A footnote | `[^1]` as written | A superscript, and the note at the end |
| Front matter | A rule and a heading | Left out |
| A task list | `[x]` as written | With `☑` and `☐` written for it, nothing without |
| A table | A grid | With `border="1"` written for it, a grid and a bold header; nothing without |

What hostile Markdown costs is about the same. For 16 files of 100,000 marks (`*a`, `|`, `[`, `>`,
nested quotes, footnotes and others), `/usr/bin/time -l clv FILE --protocol kitty --pages 1` with the
Markdown importer, and with what pulldown-cmark made of each given to `clv` as an HTML file: converting
each took 0.01 s at most, and 11 of them were within 0.1 s and 45 MB either way. The importer cost more
on `deeplist`, a list nested 1,000 deep (0.60 s and 228 MB, against 0.08 s and 45 MB), `backticks`
(0.45 s and 269 MB, against 0.07 s and 32 MB) and `stars` (258 MB, against 204). This way cost more on
`footnotes` (295 MB, against 204) and `headings` (2.73 s, against 2.51).

At ADR-0012's limits, through the code as it now is, its Markdown files peak within 5% of where they
did, but for three: a table of half a million `|`, 0.9 s and 1.1 GB, was 0.7 s and 0.8 GB; half a
million lines, 336 MB, was 258; half a million links' marks, 122 MB, was 139. The worst is still 15 s
and 4.9 GB, an HTML file.

A program of nothing but a call to it, `cargo build --release`, is 769 KB, against 433 KB for one that
prints a file: about 335 KB, an estimate of what it adds to `clv`.

## Alternatives considered

- **A loader in wordcraft's Markdown importer**, a small change upstream: `<img>` would still show as
  its tags, a change wanted elsewhere too. It isn't there on its `main` (`242e850aaa`, 2026-11-02),
  the pin is 70 commits and 130 files behind it, and moving the pin takes all of that.
- **Rewriting each picture in the Markdown into a `data:` URL before importing**: a Markdown parser
  to find them anyway (a code block's aren't pictures), a third more bytes for each, and `<img>` as
  above.
- **Leaving `<details>` and the like to show as tags**, and handling `<img>` alone: raw HTML is common
  in READMEs, whatever it holds.

## Consequences

- **Positive:** a README shows as it is meant to; pictures have one path, whichever format refers to
  them.
- **Accepted:** a new dependency, and three more crates in the lockfile counting its own; how Markdown
  looks is now how wordcraft's HTML importer styles those tags.
- **Revisit when:** wordcraft's Markdown importer reads raw HTML and takes a loader.
