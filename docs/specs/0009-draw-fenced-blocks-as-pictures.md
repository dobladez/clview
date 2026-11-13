# Spec-0009: Draw a document's fenced blocks as pictures

- **Status:** Ready
- **Date:** 2026-11-13
- **Related:** [Spec-0008](0008-show-mermaid-diagrams.md) (Mermaid's backend and its cache), which
  must be Done first; [ADR-0030](../adr/0030-mermaid-diagrams-through-merman-in-a-worker-process.md)
  (merman, its limits and its cache's bounds); [ADR-0013](../adr/0013-a-documents-pictures-are-drawn-by-the-backends.md) (a
  document's pictures, drawn by the backends);
  [ADR-0014](../adr/0014-markdown-through-pulldown-cmark-into-wordcrafts-html-importer.md) (Markdown
  through pulldown-cmark); [ADR-0018](../adr/0018-wordcraft-stays-before-its-proofing-needs-a-checkout-to-build.md)
  (wordcraft's pin); [ADR-0021](../adr/0021-a-backend-declares-its-settings-and-their-options-hold-for-any-file.md)
  (a backend's settings); [Spec-0007](0007-search-markdown-html-and-word-documents.md) (searching a
  document's text); [Documents](../usage.md#documents); [Backends](../architecture.md#backends).
  With Spec-0008, it delivers [README's Plans](../../README.md#plans)' "Mermaid diagrams, their files
  and a Markdown `mermaid` block": this spec its blocks; the bullet comes out once both are Done.

## Summary

On GitHub, a ` ```mermaid ` block in Markdown shows as its diagram; in clv it shows as code. This spec
lets a format whose backend draws pictures list the languages its fenced blocks are written in (a
`fences` list on `Format`, beside `extensions`), and has the documents' backend turn a block whose
language a format lists into a picture of it: ` ```lang ` in Markdown, `<pre><code
class="language-lang">` in HTML. It is drawn through `picture.rs` as a picture the document refers
to is, from the block's text rather than a file, and the backend that draws it keeps what it made of
it, so reading the document again for a render or a relayout costs nothing more (Mermaid's cache,
[Spec-0008](0008-show-mermaid-diagrams.md)). The documents' backend names no other backend: it asks
which lists the language. A language is listed, never sniffed nor taken from an extension, so
` ```svg ` stays code, as on GitHub. A block that can't be drawn shows as its code. A setting the
documents' backend declares shows such blocks as code instead. Mermaid is the first to list one,
`mermaid`. Since a block's text travels in the document, this works through a pipe and in archives
too, where pictures by path can't be reached.

## Requirements

- **The contract:** a `fences` list on `Format` (`src/source.rs`): the languages a document's fenced
  block of it is written in, matched without regard to case against the info string's first word
  (` ```Mermaid title="x" ` is `mermaid`). Empty for most formats; only one whose backend
  `draws_pictures` lists any. `source.rs` finds the backend for a language (`fenced`), and `--help`
  lists the languages beside the extensions. A test in `source.rs` checks that only picture-drawing
  backends list fences, and no language twice among the backends built.
- **Markdown:** in `markdown_as_html`, a `CodeBlock(Fenced(info))` whose language a built backend
  lists becomes a picture of its text. An indented block, a fence with no language or one none
  lists, stays code as now: ` ```svg ` stays code, since no format lists `svg`.
- **HTML, and so an epub's chapters:** a `<pre>` whose `<code>` (or itself) has a class
  `language-x`, GitHub's and most highlighters' way, `x` listed, becomes a picture of its text, its
  tags dropped and its entities decoded. Any other `<pre>` stays as it is.
- **Drawn as pictures are:** measured by the listing backend opening the block's text (`Input::held`,
  under a name from its format's first extension, `block.mermaid`), straight through its `open`, not
  `opens`, for its first page's size; drawn when its page first renders; at its own size or smaller to
  fit the column; let go of and drawn again as any picture
  ([ADR-0013](../adr/0013-a-documents-pictures-are-drawn-by-the-backends.md)). With no directory too:
  through a pipe, in an archive, in an epub.
- **Within the pictures' limits:** blocks count among a document's 1,024 pictures and its 256 MiB held.
  A block's text is held to the listing backend's own limit on a file of it (merman's 1 MiB source limit, Spec-0008); past
  it, the block stays code.
- **Keys a document can't forge:** the key a block's picture is known by has a prefix made afresh
  each time the document is read, which the document's own `<img src>` can't guess. A picture the
  document names goes through `local()` as now, and never reaches a block.
- **A failed block shows as its code:** one the backend can't open (a broken diagram, the worker
  killed, no worker to be had) is known when it is measured, before the HTML is written, so it is the
  code block it would have been, not a grey box; the first page's note says how many blocks weren't
  drawn, beside the pictures left out. One that opens and fails to draw later is a grey box with the
  page's note, as any picture.
- **As code, by a setting:** the documents' backend declares `fences`, `drawn` (the default) or
  `code`, effect `Relayout`, on `s` (a `Keys::Cycle`), its option `--fences drawn|code`, in `?`'s
  list; on a file that has it not, `s` says so (`only Markdown and HTML draw their blocks`).
  `a_settings_keys_are_its_own_in_every_backend` and `the_viewers_keys_are_those_listed` check the
  key.
- **The cache it relies on:** reading the document again (a render, a relayout, a picture drawn
  again) asks the listing backend again. Mermaid's cache answers without its worker
  ([Spec-0008](0008-show-mermaid-diagrams.md)); a backend that lists a fence keeps what it made the
  same way, as `Format::fences`'s comment says.
- **On the document's paper:** a drawn block follows its document's `d`
  ([ADR-0015](../adr/0015-markdown-and-html-on-dark-paper-the-terminals-own-colour.md)). On light
  paper, Mermaid's default theme on white, as a `.mmd` file is ([Spec-0008](0008-show-mermaid-diagrams.md));
  on dark paper, drawn again in **Mermaid's own dark theme** on the paper's colour, the white left
  out, never the light drawing's colours inverted. `d` draws the blocks again with the rest of the
  page (a redraw, no relayout: a theme doesn't change a diagram's size, which the test checks). The
  paper reaches the listing backend through the contract, with the request a block is drawn for,
  never by the documents' backend's setting's name; Mermaid's cache keeps each theme's SVG apart.
  A picture by path and a `.mmd` file shown alone stay as they are now (README's Plans).
- **Search** ([Spec-0007](0007-search-markdown-html-and-word-documents.md)): a drawn block's text
  isn't searched; shown as code, it is, being the document's text then.

## Out of scope

- **Other diagram languages** (` ```dot `, ` ```d2 `, math): each a backend of its own listing its
  fences, and a spec of its own ([README's Plans](../../README.md#plans)). PlantUML needs Java.
- **` ```svg ` and the other formats clv shows, drawn from a block**: none lists a fence; one could
  later, in a line.
- **Org-mode's `#+begin_src`, AsciiDoc's `[source,x]`, reStructuredText's `.. code::`**: no backend
  reads them yet; one that does asks `fenced` as Markdown does (README's Plans).
- **Word documents**: they have no fenced blocks.
- **Highlighting** the blocks that stay code (README's Plans).
- **A size or alignment from the info string** (` ```mermaid width=400 `).
- **One block shown as code**, or a block switched by a click: the setting is the document's.
- **Searching a drawn block's text.**
- **A block's picture in a list's tile drawn otherwise** than a picture by path: the same rules,
  `svg-html` labels left out of tiles included.
- **`d` for drawings outside blocks**: SVG and Mermaid files shown alone, and a document's pictures
  by path, following the paper; the `paper` setting moved into the contract for them
  ([README's Plans](../../README.md#plans)).

## Approach

**Files:**

- `src/source.rs` — `Format`'s `fences`, built through `Format::named(name, extensions)` and
  `.fenced(&[…])`, so the formats without fences say nothing new; `fenced(lang, backends)` beside
  `shown_by`; the test.
- `src/source/{gridcraft,pdfcraft,tar,image,zip,svg,wordcraft,later}.rs`, and the test backends in
  `src/view/mod.rs` — their `Format`s through the constructor. `src/source/mermaid.rs` —
  `.fenced(&["mermaid"])`.
- `src/view/mod.rs` (where `--help` lists formats) — the fences beside the extensions. It reads
  `Format`, and names no backend.
- `src/source/picture.rs` — a `Promise` drawn from a path, as now, or from a block (its backend, its
  name, its text); `Pictures::fenced(lang, text, fit)`, the key and stand-in of one measured, or none;
  blocks among `MAX_PICTURES`; the keys' prefix, from std's `RandomState`, no new dependency.
- `src/source/wordcraft.rs` — `markdown_as_html` given `pictures`: a listed fence measured first, then
  `<img src=KEY alt=…>` if it measured, the code block as now if not. For HTML, a scanner of clv's
  own over the text before it is imported: a `<pre>` whose `<code>`, or itself, has a `language-x`
  class, to its `</pre>`; comments, `<script>`, `<style>` and `<textarea>` skipped; its text with tags
  dropped and entities decoded (`wordcraft_formats::html::unescape`); bounded as `MAX_MARKS` bounds
  the rest. wordcraft's parser drops the class, and wordcraft stays at its pin
  ([ADR-0018](../adr/0018-wordcraft-stays-before-its-proofing-needs-a-checkout-to-build.md)), so it
  can't be given a hook. The `fences` setting beside `paper`; blocks not drawn counted in the first
  page's note.
- Tests: unit tests in `wordcraft.rs` and `picture.rs`; `tests/it/mermaid.rs` (blocks through the
  viewer, a pipe, an epub); `tests/it/view_pager.rs` (`s`); the fences test in `src/source.rs`.
- Docs: `docs/usage.md` (Documents: blocks drawn; Keys and Options: `s`, `--fences`; Limits: blocks
  among the pictures); `docs/architecture.md` (`Format`'s fences, pictures from blocks, the scanner);
  an ADR (fences listed on `Format`, by name, never sniffed nor taken from the extensions; keys a
  document can't forge); `CHANGELOG.md`; README's Plans bullet out, with Spec-0008 Done.

**Patterns to follow:**

- A setting the documents' backend declares, `Relayout`: `width` in `src/source/wordcraft.rs`;
  `paper` for a `Keys::Cycle`.
- A promise made at layout, drawn at render: `Pictures::promised` and `Promise::draw` in
  `src/source/picture.rs`.
- A function over the backends given: `shown_by` in `src/source.rs`.
- A rule every backend built keeps, checked: `the_extensions_a_backend_lists_are_the_names_it_opens_by`.

**Decided:**

- Fences on `Format`, listed, never taken from its extensions: ` ```svg ` stays code, and a fence's
  name may differ from an extension (` ```dot `, `.gv`).
- A block goes straight to its backend's `open`, never through `opens`.
- A block that doesn't open shows as its code, decided before the HTML is written.
- Keys with a prefix made afresh each read.
- The setting `fences`, `drawn|code`, `Relayout`, `--fences`, on `s`.
- The listing backend's cache keeps re-reads cheap (Mermaid's, Spec-0008).
- A block follows its document's paper: on dark, Mermaid's dark theme on the paper's colour, through
  the request it is drawn for (`src/source.rs` says how a picture is told it is on dark paper, and
  `picture.rs` keeps a block drawn for each paper apart); the paper's colour is `Request::dark`'s.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- 🔴 **Spec-0008 Done first** — Mermaid is the first, and so far the only, format to list a fence:
  without it, nothing tests this through the viewer.
- 🟡 **The HTML scanner on real output** — GitHub's rendered Markdown, pandoc's, and highlighters'
  (a `<pre>` of nested `<span>`s, `class="highlight-source-mermaid"`): try each before settling which
  classes count. It stands alone as the second task, so it can be cut without the rest.

## Tasks

- [ ] **Fenced blocks in Markdown** — `Format`'s fences and `fenced`, the constructor through every
  backend, `--help`, pictures from blocks, `markdown_as_html`, Mermaid's ` ```mermaid `, the ADR.
  *Files:* `src/source.rs`, the backends' `formats()`, `src/view/mod.rs`, `src/source/picture.rs`,
  `src/source/wordcraft.rs`, `src/source/mermaid.rs`, `tests/it/mermaid.rs`, docs. *Verify:*
  `cargo test -j 2 --lib fence` and `cargo test -j 2 --test it mermaid`: a ` ```mermaid ` block drawn
  (ink where its nodes are), ` ```Mermaid title=x ` too; ` ```svg ` and an indented block stay code;
  a broken diagram shows as its code and the first page says so; Markdown through a pipe and in a zip
  draws its block; an `<img src>` naming a block's key, written in the document, shows as its alt
  text; a document of 1,025 blocks leaves the last out; the document rendered page after page without
  the diagram made again; the fences test. Manual: in kitty, a README with
  diagrams, `]` and `+`.
- [ ] **HTML's blocks** — the scanner. *Files:* `src/source/wordcraft.rs`, `tests/it/mermaid.rs`,
  usage. *Verify:* `cargo test -j 2 --lib scanner`: `<pre><code class="language-mermaid">` and
  `<pre class="language-mermaid">` drawn, entities decoded, nested `<span>`s' text kept; one in a
  comment, a `<script>`, a `<style>` or a `<textarea>` left alone; one never closed, code; cut
  anywhere, never a panic (a property test). `cargo test -j 2 --test it mermaid`: an epub's chapter
  draws its block. Manual: in kitty, GitHub's HTML of a README with a diagram, saved from the browser.
- [ ] **The `fences` setting** — `s`, `--fences`, `?`. *Files:* `src/source/wordcraft.rs`,
  `tests/it/view_pager.rs`, usage. *Verify:* `cargo test -j 2 --test it view_pager` on `s`: the block
  as code, then drawn again; `--fences code` starting so; `s` on a PDF saying so; in a list mixing
  them; `a_settings_keys_are_its_own_in_every_backend`. Manual: in kitty, `s` on a README with a
  diagram, and inside a private `tmux -L` server.
- [ ] **Blocks on the document's paper** — the paper told to a block through the request it is drawn
  for, Mermaid's dark theme, each theme cached apart. *Files:* `src/source.rs`,
  `src/source/picture.rs`, `src/source/wordcraft.rs`, `src/source/mermaid.rs`, `tests/it/mermaid.rs`,
  `tests/it/view_pager.rs`, usage, the ADR. *Verify:* `cargo test -j 2 --test it mermaid`: a block on
  `--theme dark` drawn in the dark theme on the paper's colour, with no white behind it; on
  `--theme light`, on white; the two of the same size; a picture by path unchanged on dark paper;
  `cargo test -j 2 --test it view_pager`: `d` on a README with a diagram draws the block again, dark,
  then light, without the diagram laid out again more than once a theme. Manual: in kitty with a dark
  background, a README with diagrams, `d` back and forth.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [ ] Tests added, each seen failing without the change: name them.
- [ ] Manual: in kitty, a README with diagrams, through a pipe, `s`, `]`; GitHub's HTML of it; inside
  tmux.
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- 2026-11-13 — Ready (spec-new), blocked by Spec-0008.
- 2026-11-13 — Reconciled with Spec-0008's spike: ADR-0030 linked, merman's 1 MiB named (spec-update).
- 2026-11-13 — The owner's: a drawn block follows its document's `d`, Mermaid's dark theme on dark
  paper; a fourth task. `d` for drawings outside blocks left to README's Plans.
