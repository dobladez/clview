# ADR-0005: Word documents through wordcraft's readers, layout and renderer

- **Status:** Accepted
- **Date:** 2026-10-30

## Context

Word documents were wanted beside PDFs: docx, the older binary doc, and the OpenDocument text and
RTF that word processors also save ([Documents](../cli-view.md#documents)). A document has no
pages until it is laid out: fonts, line breaks, tables, floating pictures, headers and footers,
all of which a word processor does. [wordcraft](https://github.com/storytold/wordcraft), from the
same authors as pdfcraft, is one in Rust (MIT or Apache-2.0), split into crates: a document
model, readers (docx, doc, and odt, rtf and others in one importers crate), layout, a renderer on
vello_cpu, an engine (the app's session, commands and file dispatch, with PDF export), and the
app.

A list shows many files at once, so a source must hold little while idle
([`src/view/README.md`](../../src/view/README.md)), and a docx and an odt are zips, which the zip
backend would show as folders.

## Decision

- **The readers, layout and renderer, not the engine**: `wordcraft-docx`, `-docbin`, `-formats`,
  `-layout`, `-render` and `-doc`, pinned to one commit as pdfcraft is. The engine's open does
  more (content sniffing for misnamed files, plain text, HTML and Markdown, fonts a document
  embeds, `DATE` fields brought up to now, pictures fitted to the text width), but depends on
  wordcraft's PDF export, whose `krilla` must be patched through a `[patch.crates-io]` that Cargo
  honours only in the root manifest: ours would have to repeat it, as it repeats pdfcraft's.
- **Read and lay out the whole document to open it, keep only its pages' sizes**, and read and lay
  it out again on a renderer's or pool's first render, kept while it lives, as a drawing keeps
  its tree (`src/source/wordcraft.rs`).
- **Asked before zip, and known by what it holds** where the first 1024 bytes tell: an RTF by
  `{\rtf`, an odt by its first file, a docx by a file in `word/` among the first. By name where
  they don't: a zip named as a Word document, and a compound file (`D0 CF 11 E0`), which
  spreadsheets and slides are too.
- **Fonts a document embeds are not loaded**: wordcraft adds them to its process-wide font
  database, where they stay until clv exits, and a list may open hundreds of documents.

## Evidence

Apple M3 Max, macOS 26.6.2, wordcraft at `5b5810fe1f6f815acc3fda27619f61d9bc08b615`, a release
build (`lto = "thin"`, `codegen-units = 1`) of a test program calling `wordcraft_docx::read`,
`wordcraft_layout::layout` and `wordcraft_render::render_page` at scale 2 (1.5 for the longest),
peak memory from `getrusage`. The documents were written by macOS's `textutil -convert docx`
(and `odt`, `rtf`) from an HTML page of 40 chapters of four paragraphs, a table every third, a
list every fourth; the long one is 14 copies of the chapters.

| Document | Read | Lay out | Draw a page | Peak memory |
|---|---|---|---|---|
| docx, 38 pages | 12 ms | 110 ms | 1–6 ms | 50 MB |
| docx, 519 pages | 67 ms | 247 ms | 1–4 ms | 219 MB |
| odt, 26 pages | 4 ms | 31 ms | 2 ms | 40 MB |
| rtf, 27 pages | 2 ms | 30 ms | 2–3 ms | 41 MB |

The 519 pages' layout is most of that memory: kept by every document of a list, idle, it would
be about 180 MB each. `clv long.docx --protocol kitty --pages 1-2` (a dev build) took 0.66 s and
235 MB at most, laying it out twice. A `.doc` written by `textutil -convert doc` is refused by the
compound-file reader wordcraft uses ("Malformed MiniFAT"); none written by Word was tried.

The binary: `cargo build --release --locked --no-default-features --features word` makes a `clv`
of 17.2 MB, against 1.0 MB with `--no-default-features`: 16.2 MB more, 4.1 MiB of it the fonts
wordcraft bundles, and some of it the `image` crate's decoders, which the image backend builds
anyway.

## Alternatives considered

- **The engine (`wordcraft_engine::io::open_bytes_with`)**: what wordcraft's app opens files
  with, and more formats for free, but its PDF export comes with it and needs the patch above;
  and plain text, HTML and Markdown aren't files clv means to page through as printed documents.
- **Keep the layout from opening**: no second layout when shown, but every document in a list
  would hold it while idle (180 MB for 519 pages).
- **Lay out only the first page to open**: a tile needs only that, but wordcraft lays out a
  document whole, and the page count is needed to page through it.
- **Go by name alone, as the engine does**: a docx renamed or held without its name (piped in,
  a `.docx.gz`) would show as a folder; going by the first bytes is what every other backend does.
- **Load embedded fonts**: truer text where a document embeds a font the system lacks, but memory
  that is never given back.

## Consequences

- **Positive:** docx, doc, odt and rtf in one backend, quick to lay out; a protected Word
  document opens with `--password`; no patches to carry.
- **Accepted:**
  - Up to 16 MB more binary, a quarter of it fonts.
  - A document is read and laid out twice to be shown, and a slow one holds up its opening:
    clv bounds only the file's size (256 MiB) and a page's pixels (64 million); the rest is
    wordcraft's own limits.
  - Pictures in an odt or rtf wider than the text aren't shrunk to it, as the engine would, and
    `DATE` fields show what the document saved.
  - A `.doc` that is a docx, an RTF or a web page inside is read only when its first bytes say
    so (a zip with `word/` first, `{\rtf`); the engine's other sniffing is not here.
- **Follow-ups:** report the `textutil` `.doc` to wordcraft; flat XML Word documents (`.xml`).
- **Revisit when:** wordcraft's engine no longer needs patches, or its layout can stop after a
  page or be cut short; or a list keeps documents open only while their tiles show.
