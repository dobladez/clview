# ADR-0032: A fenced block is drawn by the backend that lists its language

- **Status:** Accepted
- **Date:** 2026-11-13

## Context

On GitHub, a ` ```mermaid ` block in Markdown shows as its diagram. clv showed it as code, though it
draws a Mermaid file ([ADR-0031](0031-mermaid-diagrams-through-merman-in-a-worker-process.md)) and
draws a document's pictures by path through the backends that show them
([ADR-0013](0013-a-documents-pictures-are-drawn-by-the-backends.md)).
[Spec-0009](../specs/0009-draw-fenced-blocks-as-pictures.md) draws such a block as a picture.

What constrains how:

- The documents' backend may name no other backend
  ([Layers](../architecture.md#layers)), so it can't know that `mermaid` is Mermaid's.
- A block's text is untrusted, as a file is, and travels in the document: there is no file whose
  name or first bytes a backend could be asked about.
- wordcraft's HTML importer, which Markdown goes through too
  ([ADR-0014](0014-markdown-through-pulldown-cmark-into-wordcrafts-html-importer.md)), takes
  pictures only by an `<img src>` it hands to a callback, and drops a `<code>`'s class. It stays
  at its pin ([ADR-0018](0018-wordcraft-stays-before-its-proofing-needs-a-checkout-to-build.md)),
  so it can't be given a hook for blocks.
- A document is read again and again: once to open, again for its first render, again for every
  relayout (a width, a font size, the window's size).

## Decision

1. **A format lists its fences.** `Format` (`src/source.rs`) has `fences`, beside `extensions`:
   the languages a document's fenced block of it is written in, in lowercase. Mermaid lists
   `mermaid`. `source::fenced(language, backends)` finds the backend that lists a language, among
   those that say `draws_pictures`, without regard to case; the documents' backend asks it, and
   names no one. `--help` lists a format's fences after its extensions. A test over the backends
   built holds that only a backend that draws pictures lists any, and no language twice.
2. **Listed, never sniffed, never taken from the extensions.** A block is drawn only if its
   language is listed: ` ```svg ` stays code, as on GitHub, though clv shows SVG, and so does a
   block with no language, whatever its text. A fence's name need not be an extension (` ```dot `
   for `.gv`), nor an extension a fence (` ```mmd ` is nothing).
3. **Straight to `open`, never through `opens`.** The block's text goes to the listing backend's
   `open` as a file held in another (`Input::held_bounded`), under a name of the format's first
   extension (`block.mmd`). `opens` isn't asked: it goes by a file's name and first bytes, and a
   block has said what it is. The backend's own limits on a file hold (merman's 1 MiB).
4. **Drawn as a picture by path is.** `picture.rs` promises a picture from a block's text as it
   does from a path: sized at layout by the backend's first page, drawn when its page first
   renders, let go of and drawn again within the document's 256 MiB. Blocks count among a
   document's 1,024 pictures, a block that fails included.
5. **A block that can't be drawn is its code.** It is sized before the HTML for it is written, so
   one its backend refuses is written as the code block it would have been, never a grey box, and
   the first page's note counts them. One that opens and later fails to draw is a grey box, as any
   picture.
6. **Keys a document can't write.** A block reaches wordcraft as `<img src="KEY">`, and its
   callback gets every `<img src>` of the document, the document's own among them. A key is
   `fenced:` followed by 64 bits made afresh each time the document is read (std's `RandomState`,
   keyed from the system's randomness), then the block's number. A document can't know them, so it
   can't name a block's picture in an `<img>` of its own; and with a scheme's colon, a key is
   never a path (`local` refuses it), so one that isn't a block's reads no file.
7. **The listing backend keeps what it made.** Each reading asks the backend again, with the same
   text. Mermaid's cache answers without its worker
   ([ADR-0031](0031-mermaid-diagrams-through-merman-in-a-worker-process.md)); `Format::fences`
   says that a backend listing one keeps what it made the same way.

## Evidence

GitHub's own rule, as its documentation gives it
([Creating diagrams](https://docs.github.com/en/get-started/writing-on-github/working-with-advanced-formatting/creating-diagrams),
read 2026-11-13): a fenced block is drawn by its language identifier alone, and four are listed,
`mermaid`, `geojson`, `topojson` and `stl`. `svg` isn't among them, though GitHub shows an `.svg`
file as its drawing.

What wordcraft's importer does with an `<img>`, at its pin (`bbe3547`,
`crates/formats/src/html/builder.rs`): a `data:` URL is decoded, any other `src` goes to the
caller's loader, trimmed; with no picture back, the `alt` is shown as text. A `<code>` only sets
the code font: its class is dropped.

**What a document's blocks cost to open,** measured 2026-11-13 on an Apple M3 Max (14 cores,
36 GiB), macOS 26.6.2, Rust 1.95.0, with `target/debug/clv` (its dependencies at `opt-level = 2`):
`/usr/bin/time -l clv <file>.md --protocol kitty --pages 1 > /dev/null`, which reads the document
twice, to open it and to render its first page. Each file is ` ```mermaid ` blocks and nothing
else:

| Blocks | The file | Time | Maximum resident set |
|---|---|---|---|
| 1,025 of one small pie | 30 KB | 1.3 s | |
| 1,024 flowcharts of two nodes, each its own | 46 KB | 10.1 s | |
| 8 flowcharts of one label of 1 MB, each its own | 8 MB | 12.5 s | 464 MB |
| 8 flowcharts of 15,000 nodes, each its own (ADR-0031's costliest in CPU and SVG) | 2.6 MB | 67 s | 545 MB |

The first is Mermaid's cache answering all but one. The last two are past it: eight SVGs of 9.7 MB
or 22 MB don't fit its 64 MiB, so the second reading draws them again, and the SVG backend opens
each one's SVG both times. A document's own limits bound how many such blocks it holds: 500,000
lines and marks are eleven diagrams of 15,000 nodes, and 64 MiB sixty labels of 1 MB.

## Alternatives considered

- **Sniffing the block's text** (asking each backend's `opens` with the text's first bytes): a
  block of SVG, of HTML or of CSV would turn into a picture or a page of its own, where the writer
  showed code on purpose; and `opens` goes by names a block doesn't have.
- **Fences from the extensions** (` ```svg ` because `.svg`): the same, and it couldn't say
  ` ```dot `.
- **The languages listed in the documents' backend**: it would name Mermaid, and every later
  diagram language would touch it.
- **Drawing at read time, the picture embedded as a `data:` URL**: wordcraft takes those without a
  callback, but every block of a long document would be drawn to open it, and held; ADR-0013's
  stand-ins draw a page's own when it first shows.
- **Keys by the block's number alone** (`fenced:0`): a document could show a block's picture
  again in an `<img>` of its own. Harmless as far as is known, since it is the document's own
  block; but a key the document can write is a name it shares with clv, and the next use of a key
  would have to remember that.
- **A grey box for a broken block**, as for a picture that fails: the reader would lose the text
  of the diagram, which is what they have to fix it by.

## Consequences

- **Positive:** a diagram language is a backend that lists a fence, in one line, and the
  documents' backend doesn't change. A block is drawn through a pipe and in an archive, where a
  picture by path can't be reached.
- **Accepted:** every block a backend lists is opened to size it when the document opens: a
  document of many diagrams starts Mermaid's worker and waits for each, within its caps
  ([Limits](../usage.md#limits)). A document made of the costliest diagrams merman accepts takes
  minutes to open where the costliest Markdown took 20 seconds
  ([ADR-0012](0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md)), and as long
  again each time it is laid out, though the keys still work then; a document that refers to as many
  `.mmd` files by path already did. A block's text is kept with its promise while the document is
  open, within the document's own size. Two blocks of the same text are two pictures.
- **Follow-ups:** other diagram languages, each a backend and a spec of its own
  ([README's Plans](../../README.md#plans)). A bound on what a document's blocks cost in all, in
  time or in text, if the minutes above turn out to matter
  ([the backlog](../backlog.md#not-yet-planned)).
- **Revisit when:** wordcraft's importer can be given a hook for a `<pre>`'s language, or clv reads
  another format with fenced blocks (Org, AsciiDoc), which would ask `fenced` the same way.
