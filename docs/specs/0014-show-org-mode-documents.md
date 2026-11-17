# Spec-0014: Show Org-mode documents

- **Status:** Ready
- **Date:** 2026-11-17
- **Related:** [ADR-0014](../adr/0014-markdown-through-pulldown-cmark-into-wordcrafts-html-importer.md)
  (Markdown through HTML into wordcraft's importer, the path Org takes);
  [ADR-0038](../adr/0038-an-epub-as-one-document-its-package-read-by-rbook-in-a-worker.md) (a
  backend that writes HTML and hands it to the backend that opens HTML, and a library that can't be
  bounded run in a worker); [ADR-0031](../adr/0031-mermaid-diagrams-through-merman-in-a-worker-process.md)
  (likewise, for SVG); [ADR-0039](../adr/0039-a-backend-sure-of-a-file-wins-over-one-that-finds-a-mark-in-it.md)
  (which backend opens a file, and what makes it sure); [ADR-0033](../adr/0033-backends-share-a-setting-by-declaring-its-name.md)
  (settings shared by name); [ADR-0037](../adr/0037-what-is-beside-a-held-file-is-read-from-its-holder.md)
  (what is beside a held file); [ADR-0012](../adr/0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md)
  (what a document costs, bounded); [ADR-0013](../adr/0013-a-documents-pictures-are-drawn-by-the-backends.md)
  (a document's pictures); [ADR-0015](../adr/0015-markdown-and-html-on-dark-paper-the-terminals-own-colour.md)
  and [ADR-0016](../adr/0016-how-markdown-and-html-look.md) (paper and look);
  [Spec-0009](0009-draw-fenced-blocks-as-pictures.md) (blocks drawn as pictures, HTML's `<pre>`
  among them); [Spec-0011](0011-follow-a-documents-links.md) (a document's links);
  [Spec-0012](0012-show-an-epub-as-a-book.md) (the epub, the pattern this follows);
  [Documents](../usage.md#documents), [Following links](../usage.md#following-links),
  [Limits](../usage.md#limits), [Backends](../architecture.md#backends); the backlog's
  [More documents](../backlog.md#more-documents). It delivers the Org-mode part of README's Plans
  bullet "More documents: Org-mode, AsciiDoc and reStructuredText", leaving the other two.

## Summary

Org-mode files (`.org`), Emacs users' notes, plans and documentation, aren't shown today: alone
they open as nothing, and in a list as a file clv can't show. This shows them as Markdown is shown,
by writing them as HTML that the documents' backend lays out
([ADR-0014](../adr/0014-markdown-through-pulldown-cmark-into-wordcrafts-html-importer.md)), so Org
gets everything that path gives: pages the window's size or a width of the reader's (`]`,
`--width`, `--font`, `--margins`), dark paper (`D`), search (`/`), pictures drawn by the backends
that show them, `#+begin_src mermaid` blocks drawn as diagrams (`s` for their code), and links
followed with `f` ([Spec-0011](0011-follow-a-documents-links.md)). Org is told by its name
(`.org`, `.org_archive`), or by an Emacs mode line on its first line, and that it is text.

Org is a backend of its own, as the epub is
([ADR-0038](../adr/0038-an-epub-as-one-document-its-package-read-by-rbook-in-a-worker.md)): it
writes HTML and hands it to whichever backend opens HTML, never naming it. The parsing is a
library's, not clv's own: the first task chooses one of two existing libraries,
[orgize](https://crates.io/crates/orgize) (0.10.0-alpha.10) and
[org-rust-parser](https://crates.io/crates/org-rust-parser) (0.1.8), weighing each on real Org
files for what it gets right, and on hostile ones for panics and parse time, its choice recorded
in an ADR; if neither is good enough, the better one is patched, never replaced by a parser of
clv's own. The HTML written from the parser's tree is clv's: what Org shows, what it hides, and
how its links and places become `<a href>` and `id`s.

## Requirements

### Telling and opening

- **By name:** a file named `.org` or `.org_archive`, in any case, that is text.
- **By its mode line:** a file whose first line, or second when the first starts with `#!`, holds
  an Emacs mode line naming Org (`-*- org -*-`, or `mode: org` among its variables, `;`-separated,
  in any case), as the Emacs manual's "Specifying File Variables" writes them, and that is text,
  whatever its name: a `.txt`, a file with no extension, and **a pipe**. A sure claim
  ([ADR-0039](../adr/0039-a-backend-sure-of-a-file-wins-over-one-that-finds-a-mark-in-it.md)):
  "what a file of its kind holds", like a `.mmd`'s first line.
- Asked before the documents' backend, so a `.md` whose first line says Org is Org, as Emacs would
  have it. HTML that starts with a doctype or an `html` element has that on its first line, not a
  mode line, so it stays HTML.
- Read as Markdown is (`txt::decode`): UTF-8, and UTF-16 by its byte order mark.
- From the command line, in a list (opened when its tile is wanted, as documents are), in a
  directory, in a zip and in a tar.
- `--help` lists Org among documents, with its two extensions, and credits the parser.

### What it has, as Markdown has it

- The five settings Markdown has (`paper`, `width`, `font`, `margins`, `fences`), declared by name
  ([ADR-0033](../adr/0033-backends-share-a-setting-by-declaring-its-name.md)): `D`, `]` and `[`,
  `s`, `--theme`, `--width`, `--font`, `--margins`, `--fences`; and Markdown's look
  ([ADR-0016](../adr/0016-how-markdown-and-html-look.md)).
- `/` searches its body as shown: nothing hidden is searched, nor a block drawn as a picture.
- Pictures drawn by the backends that show them, from near the file only, nothing fetched
  ([ADR-0013](../adr/0013-a-documents-pictures-are-drawn-by-the-backends.md)); through a pipe, none.
- **Limits.** The Org source is held to ADR-0012's limits (its size, and its lines and marks
  counted before it is parsed, which bounds the parse too); the HTML it makes is then held to
  HTML's, as any HTML is. An Org file may be refused for the HTML it makes, and the message says
  which.

### What Org shows

- **Headlines** as headings, levels past 6 as 6. A TODO keyword in colour, not-done red and done
  green, the in-file `#+TODO:`, `#+SEQ_TODO:` and `#+TYP_TODO:` keywords honoured; a priority
  (`[#A]`) as written; tags after the title, in grey.
- **Planning lines** (`SCHEDULED`, `DEADLINE`, `CLOSED`) small and grey under their heading.
- **A title block** from `#+TITLE`, `#+SUBTITLE`, `#+AUTHOR` and `#+DATE`, at the top.
- **Inline:** `*bold*`, `/italic/`, `_underline_`, `+strike+`, `=verbatim=` and `~code~`; `\\`
  a line break; `-----` a rule. As written: timestamps, macros, LaTeX fragments, and `_`, `^`
  (never sub- or superscript); inline `src_x{…}` as code.
- **Lists:** ordered, unordered and description lists; checkboxes `[ ]`, `[X]` and `[-]` as ☐, ☑
  and ☒, counters (`[1/3]`) as written.
- **Tables** in a grid, as Markdown's: the first row a header when a rule (`|---|`) follows it,
  rules where they are; `#+TBLFM` lines, and rows of alignment or width cookies (`<l>`, `<10>`),
  hidden; nothing evaluated. table.el tables (`+---+`) as code.
- **Blocks:** `src` as code, or as a picture when a backend lists its language, as Markdown's
  fenced blocks are ([Spec-0009](0009-draw-fenced-blocks-as-pictures.md)); `example` and
  fixed-width lines (`: `) as code; `quote` as a quote; `verse` with its lines kept; `center`
  centred; a special block (`#+begin_note`) as its text; `#+RESULTS:` and what follows it shown.
- **Footnotes** (`[fn:1]`, `[fn:name]`, `[fn:: inline]`, `[fn:name: inline]`) as numbered
  superscripts, the notes at the end, as Markdown's.
- **Pictures:** a link with no description to a file of an extension the Org manual says is inlined
  as an image (png, jpg, jpeg, gif, svg, webp, …) is that picture; a link whose description is such
  a link is that picture, linked.
- **Shown, though Org's export leaves them out:** `COMMENT` and `:noexport:` subtrees. A viewer of
  one's own notes shows what was written.

### What Org hides

- Drawers (`:PROPERTIES:`, `:LOGBOOK:`, any `:NAME:` … `:END:`); `#+KEYWORD:` lines but the title
  block's; comments (`# …`, `#+begin_comment`); export blocks but HTML's. `#+begin_export html` is
  read as the HTML in Markdown is. `#+INCLUDE:` is never read, its line left out.

### Links and places

Each followed with `f` and copied with `F`, under [Spec-0011](0011-follow-a-documents-links.md)'s
checks:

- **Web:** `[[https://…][…]]`, `<https://…>`, and bare `http`, `https` and `mailto` addresses in
  the text; `doi:X` as `https://doi.org/X`.
- **Files:** `file:`, `file+sys:`, `file+emacs:`, and `./x`, `../x`, `/x` and `~/x` (`~` from
  `$HOME`; `~user` refused). After `::`, `*Heading`, `#custom-id` and `target` go to that place in
  that file; a line number or a `/regex/` opens it at its top.
- **In the document:** `[[*Heading]]`, `[[#custom-id]]`, `[[target]]`, and `id:X` whose `:ID:` is
  in this file; an `id:` that isn't is said not to be in it. A footnote's reference goes to its
  note.
- `#+LINK:` abbreviations expanded, with `%s` and `%h`.
- `elisp:`, `shell:`, `help:`, `info:` and the rest are never run: the status line says what each
  is.
- `F` copies the address as clv writes it (`other.org#Heading`), which is what is followed.
- **Places:** `:CUSTOM_ID:`, `:ID:`, `<<target>>` and `#+NAME:` are the document's own. A heading is
  a place by its title, as Org matches `[[*Heading]]`: without its TODO keyword, priority, tags and
  counters; and by GitHub's slug of it, so a Markdown file's `notes.org#some-heading` finds it. A
  name the document gives itself wins over a heading's, and for `[[target]]` a target or a name
  before a heading, as Org has it.

### Never panic

- A panic in the parser means the file isn't shown, with a message, never a crash: inside `caught`
  if the evaluation finds the parser bounded in time and depth on hostile input, else in a worker
  process capped on CPU time and heap, as merman and rbook are.

## Out of scope

- **AsciiDoc and reStructuredText**: the rest of the Plans bullet, each its own spec; the backlog's
  [More documents](../backlog.md#more-documents) has the way.
- **A local-variables block** at a file's end (`Local Variables: … mode: org`): past the first
  1024 bytes a file is told by.
- **Running anything:** Babel (`#+begin_src` evaluated, `#+CALL:`), table formulas, `#+INCLUDE:`
  and `#+SETUPFILE:` (each reads another file), macros expanded, dynamic blocks updated
  (`#+BEGIN: clocktable`; their text shows as last written).
- **LaTeX typeset**: fragments and environments as written; Org's entities (`\alpha`) as the
  parser gives them.
- **Folding and the agenda**: every heading shown with what is under it; no TODO views across
  files.
- **`id:` links across files**: they need Emacs's org-id database.
- **Radio targets as links**: `<<<x>>>` is a place, but `x` elsewhere doesn't link to it.
- **A line number or a `/regex/` after `::`**: the file opens at its top.
- **Captions and `#+ATTR_*`**: `#+CAPTION:` isn't laid out as one, `#+ATTR_HTML: :width` doesn't
  size a picture.
- **Org's export options**: `#+OPTIONS:`, and a src block's `:exports` and `:results`, are ignored.
- **Highlighting src blocks**: README's Plans, for Markdown too.
- **Editing**, or writing anything back.

## Approach

**Files:**

- `src/source/org.rs` (new): the Org backend, the one file naming the parser. `opens` by the name or
  the mode line, and that it is text; `formats` lists Org (`org`, `org_archive`), having the five
  settings by name; Org converted to HTML, a doctype first so the HTML is a sure claim; handed on as
  a held file whose pictures are read beside the `.org`
  ([ADR-0037](../adr/0037-what-is-beside-a-held-file-is-read-from-its-holder.md)), through
  `source::opener` over the other backends; its `Again` converts again for a relayout.
- `src/source.rs`: Org in `BACKENDS`, before `wordcraft.rs`, behind an `org` feature that brings
  `word`; and in `WORKERS` if the evaluation puts the parser in a worker.
- `src/source/wordcraft.rs`: read, ideally untouched. A src block in a listed language is written
  as `<pre><code class="language-mermaid">`, which the HTML scanner (`blocks_drawn`) draws, `s` and
  `D` included; places as `id`s, which `ids_as_anchors` makes places; links as `href`s, which
  `Linker::leads` reads.
- `Cargo.toml`: the parser, pinned with `=`, its comment; the `org` feature.
- `tests/it/org.rs` (new): Org through the viewer, and that nothing else names the parser;
  `tests/it/view_pager.rs`: `f`, `/` and `s` on an Org file.
- Docs: `docs/usage.md` (Documents, Following links, Limits, Which files it shows),
  `docs/architecture.md` (Backends' table and how Org opens), the parser's ADR, `CHANGELOG.md`,
  `CONTRIBUTING.md` (the library rule, Tests), README's Plans and the backlog's More documents.

**Patterns to follow:**

- `src/source/epub.rs`'s `opened`: a backend that makes HTML and hands it to `opener`, and `Book`,
  its wrapper of what comes back.
- `src/source/mermaid.rs`'s `opens`: a name and a first line.
- `markdown_as_html` in `src/source/wordcraft.rs`: checkboxes, footnotes, `border="1"` on tables,
  what the importer would leave out written otherwise.
- `tests/it/wordcraft.rs`'s `links_in_markdown_go_to_a_heading_and_to_its_repeat`.

**Decided:**

- A backend of its own, writing HTML for the backend that opens HTML, as the epub
  ([ADR-0038](../adr/0038-an-epub-as-one-document-its-package-read-by-rbook-in-a-worker.md)), not a
  `Kind` of the documents' backend: the parser is named in one file, and `wordcraft.rs` doesn't grow.
- A parser library, not clv's own. If neither candidate is good enough, the better one is patched.
- Places resolved by the converter: it sees the whole file, writes each place one `id`, and
  rewrites `[[*Heading]]`, `[[target]]` and `id:X` to it, so Org's precedence is its own, and
  `Linker` needs nothing new. A heading's title and slug ids are left out where the document's own
  name already has them.
- Both bounds: the Org source by ADR-0012's limits before it is parsed, the HTML by HTML's after.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- 🟡 **Which parser, and in `caught` or a worker** — task 1's evaluation decides, in its ADR.
- 🟡 **An `id` with spaces** (a heading's title) — whether wordcraft's importer keeps it as a
  bookmark as written, and `Linker::place` finds it percent-decoded; if not, the title id is
  written otherwise, and the converter rewrites `::*Heading` the same way.
- 🟡 **How much larger the HTML is than the Org** — measured in the evaluation; it says how near
  HTML's limits an Org file under its own comes.

## Tasks

- [ ] **Choose the parser, and its ADR** — evaluate orgize and org-rust-parser, existing
  libraries, never one of our own, on a corpus (the Org manual's sources, worg, real notes) for
  fidelity (headings and their decorations, links and places,
  footnotes, drawers, blocks, tables), panics under a property test cutting input anywhere, and
  parse time and depth on large and hostile files (deep nesting, unclosed emphasis), with how much
  larger the HTML is. Time box: one session. *Files:* `docs/adr/`. *Verify:* the ADR's commands
  give its numbers again.
- [ ] **The backend** — telling Org by name and mode line, its format and settings, the conversion
  but for links, handed on through `opener`, both bounds, the parser isolated as the ADR says.
  *Files:* `src/source/org.rs`, `src/source.rs`, `Cargo.toml`, `tests/it/org.rs`,
  `tests/it/view_pager.rs`. *Verify:* `cargo test -j 2 --test it org::`, and
  `Manual: in kitty, open an Org file with a TODO heading, a table and a #+begin_src mermaid block:
  see them; press D, s and ]`.
- [ ] **Links and places** — every link form and place above, the converter resolving its own.
  *Files:* `src/source/org.rs`, `tests/it/org.rs`, `tests/it/view_pager.rs`. *Verify:*
  `cargo test -j 2 --test it org::links`, and `Manual: in kitty, press f on an Org file and follow
  [[*Heading]], file:other.org::*Heading and a web link; u comes back`.
- [ ] **Docs** — usage, architecture, `CONTRIBUTING.md`, `CHANGELOG.md`, README's Plans, the
  backlog. *Verify:* `cargo test -j 2 --test docs`.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [ ] Tests added, each seen failing without the change: name them.
- [ ] Manual: in kitty, an Org file of real notes, alone and in a list, through a pipe with a mode
  line, light and dark, at a width, searched, its links followed.
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- 2026-11-17 — Ready (spec-new).
