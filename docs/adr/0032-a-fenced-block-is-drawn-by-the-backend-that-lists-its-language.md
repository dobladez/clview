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
7. **HTML's blocks by a scanner of clv's own.** Before wordcraft imports HTML, `blocks_drawn`
   finds each `<pre>` that says a listed language and writes `<img src="KEY">` in its place. It
   says it as what writes HTML from Markdown says it (Evidence): a class `language-x`, or `x`
   alone, of the `<pre>` or of the `<code>` it starts with; the `<pre>`'s `lang="x"`; or a class
   `highlight-source-x` of the `<div>` right around it. Said, never guessed, as in Markdown. The
   scanner cuts the text as wordcraft's lexer does, so that what is a `<pre>` to one is to the
   other. Markdown's own blocks never go through it: pulldown-cmark gives them whole.
8. **As code, by a setting that lays the document out again.** The documents' backend declares
   `fences`, `drawn` or `code`, on `s` and `--fences`
   ([ADR-0021](0021-a-backend-declares-its-settings-and-their-options-hold-for-any-file.md)). Its
   effect is a relayout, as `width`'s is: a picture and its code take other room, so the pages
   fall otherwise. It is the document's: one block alone can't be turned. The reader's place is
   kept by a paragraph's number with each block counted as one, since a block is one paragraph
   drawn and a paragraph a line as code. A block shown as code is written by clv from its text,
   with an invisible anchor on its first line and on its last, named afresh as a block's key is:
   its paragraphs are those between the two, however the importer splits lines.
9. **A block follows its document's paper, told through the request.** `Request` has `on_dark`:
   set for a picture a document shows on dark paper, whose colour is `Request::dark`. A backend
   with a dark form of what it draws draws that, with nothing behind it; one with none draws as
   ever. `picture.rs` sets it for a block's promise alone, and keeps what it holds of a block for
   one paper at a time, drawn again for the other. Mermaid answers with its own dark theme,
   merman's site theme `dark`, with no white behind it, each theme's SVG kept apart in its cache;
   never the light drawing's colours inverted. `d` stays a redraw: the document is laid out once,
   around the light drawing.
10. **The dark drawing in the light one's room.** Mermaid's dark theme lays some diagrams out
    larger than its default (Evidence), so the dark drawing is rendered at the scale that fits the
    light one's size, its shape kept, in the middle of a frame that size. A dark diagram is at most
    a tenth smaller than its light twin, and nothing on the page moves.
11. **A diagram's own theme stays its own.** The dark theme is the site's default, as on a web
    page whose theme is dark: a diagram's `%%{init}%%` theme and colours still apply. One made
    for white, on dark paper, is as hard to read as it is on such a page.
12. **The listing backend keeps what it made.** Each reading asks the backend again, with the same
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

**How HTML written from Markdown says a block's language,** each asked for a ` ```mermaid ` block
on 2026-11-13:

| Written by | What it wrote |
|---|---|
| pulldown-cmark 0.13 (clv's own, and CommonMark's renderers alike) | `<pre><code class="language-mermaid">` |
| GitHub, a README's page (`curl https://github.com/mermaid-js/mermaid/blob/develop/README.md`) | `<pre lang="mermaid" aria-label="Raw mermaid code">`, the text alone in it, in a `<div class="render-plaintext-hidden">` under the frame it draws the diagram in |
| GitHub's Markdown API (`curl https://api.github.com/markdown -d '{"text":…,"mode":"gfm"}'`) | `<div class="highlight highlight-source-mermaid"><pre class="notranslate">`, the text in `<span class="pl-…">`s; for a language it doesn't highlight, `<pre lang="nolang" class="notranslate"><code class="notranslate">` |
| pandoc 3.14 (its server, `https://pandoc.org/cgi-bin/pandoc-server.cgi`, `from` markdown `to` html) | `<div class="sourceCode" id="cb1"><pre class="sourceCode mermaid"><code class="sourceCode mermaid">`, each line a `<span id="cb1-1">` starting with an empty `<a>`, its words in `<span class="kw">`s |

That GitHub page, saved as it came (405 KB, ten `mermaid` blocks), shown by clv: each
`<pre lang="mermaid">` drawn as its diagram, in under a second. Mermaid's own way in a web page,
`<pre class="mermaid">`, is from its documentation, as are highlight.js's and Prism's
`language-x` classes.

**A diagram's size in Mermaid's default theme and in its dark one,** merman 0.10.0, the
`viewBox` of its SVG for a small diagram of each kind (2026-11-13,
`a_diagram_for_dark_paper_is_in_the_dark_theme_on_nothing_and_about_as_large` in
`src/source/mermaid.rs` holds the bounds):

| Diagram | Default theme | Dark theme |
|---|---|---|
| Flowchart, three nodes | 610.74 × 183 | 615.56 × 185 |
| Class diagram, two classes | 133.52 × 266 | 146.88 × 274 |
| ER diagram, two entities | 123.20 × 271 | 132.80 × 275 |
| State diagram | 152 × 196 | 152 × 200 |
| Sequence diagram, pie, gantt chart | the same | the same |

[Spec-0009](../specs/0009-draw-fenced-blocks-as-pictures.md) had taken a theme not to change a
diagram's size. It does, by up to a tenth. The owner chose the dark drawing shrunk to fit
(Decision 10) over the alternatives below, on 2026-11-13.

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
- **Only `language-x`** in HTML, as the spec first had it: GitHub's own pages and pandoc's output,
  the two most likely to be saved and read, say it otherwise.
- **Running Markdown's HTML through the scanner too**, in place of reading pulldown-cmark's
  events: one mechanism, but a block's text would go through HTML's escaping and back, and a
  `<pre>` written as HTML inside Markdown would be drawn, which GitHub doesn't.
- **The document laid out again on `d`**, each theme's diagrams at their own size: `d` would stop
  being a redraw, the pages could fall otherwise on each paper, and where the reader is would
  have to be found again, for a difference of a tenth at most.
- **The dark drawing stretched into the light one's room**: a class diagram some 7% out of shape.
- **The paper read from the documents' `paper` setting by name**: Mermaid's backend would name a
  setting of another backend's. A flag on the request names nothing.
- **Mermaid's dark theme forced over a diagram's own**: it would throw away the colours a diagram
  chose, which mermaid.js doesn't.
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
  open, within the document's own size. Two blocks of the same text are two pictures. A block on
  dark paper may be up to a tenth smaller than on light, and is drawn again each time `d` turns the
  page, from an SVG kept for each theme.
- **Follow-ups:** other diagram languages, each a backend and a spec of its own
  ([README's Plans](../../README.md#plans)). A bound on what a document's blocks cost in all, in
  time or in text, if the minutes above turn out to matter
  ([the backlog](../backlog.md#not-yet-planned)).
- **Revisit when:** wordcraft's importer can be given a hook for a `<pre>`'s language, or clv reads
  another format with fenced blocks (Org, AsciiDoc), which would ask `fenced` the same way.
