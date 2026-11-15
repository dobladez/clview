# Spec-0011: Follow a document's links with hints

- **Status:** Ready
- **Date:** 2026-11-15
- **Related:** [Spec-0006](0006-search-a-pdfs-text.md) and [Spec-0007](0007-search-markdown-html-and-word-documents.md)
  (a backend's text and its rectangles, the pattern `Links` follows);
  [Spec-0010](0010-open-file-in-default-app.md) and
  [ADR-0034](../adr/0034-open-a-file-through-the-platforms-opener-by-its-real-path.md) (`o`'s
  launchers, which open a web address too);
  [ADR-0028](../adr/0028-matches-tinted-into-the-pages-pixels.md) (the tint into the pixels, which
  the labels are not);
  [ADR-0030](../adr/0030-a-documents-text-read-from-the-pools-layout-on-a-thread.md) (a document's
  text read from the pool's layout, as its links are);
  [Layers](../architecture.md#layers) (the viewer knows no format);
  [Keys](../usage.md#keys), [Searching a document's text](../usage.md#searching-a-documents-text),
  [Opening in another app](../usage.md#opening-in-another-app), [Inside tmux](../usage.md#inside-tmux),
  [Text cells](../usage.md#text-cells);
  the backlog's [`f` in a list](../backlog.md#f-in-a-list-labels-on-tiles) and
  [a table of contents](../backlog.md#a-table-of-contents-to-browse), which come after it.
  It delivers none of README's Plans.

## Summary

In a file's pages, `f` labels every link on screen with a short code of home-row letters, as the
Vimium browser extension does, and typing a code follows that link; `Esc` drops the labels. A link
to a place in the document (a PDF's go-to-page, its table of contents, a Markdown `[see](#usage)`,
a Word cross-reference) goes there, as `n` goes to a match. A relative link to a local file
(`[guide](docs/guide.md)`), or a PDF's link to another PDF, opens that file in `clv`, as a level of
its own that `u` comes back from. A web address (`http`, `https`, `mailto` only) goes to the
browser through the desktop's opener `o` already uses, never a shell, the status line saying
`opening https://…` and then that it did; over SSH or with no desktop, it is copied to the
clipboard instead. `F` labels the same links and copies the chosen one's address. `u` goes back
through the links followed, then to the list. Backends say where each page's links are and where
they lead, through an optional `Source` method beside `texts()`, so the viewer stays without
formats. PDFs, Word and OpenDocument documents, RTF, Markdown and HTML have links; Markdown's
headings get GitHub's ids, so `#heading` links land. Mouse clicks and OSC 8 hyperlinks come later.

## Requirements

**The hints**

- **`f`** in a file's pages labels every link at least partly on screen: on every page shown side
  by side, and on a page taller than the window, in the part shown.
- **The labels** look like Vimium's: a small yellow box, dark letters, at each link's top left,
  over the page. Their letters are `sadfjklewcmpgh` (Vimium's default), every label the same
  length, the shortest that covers the links on screen (one letter up to 14, two up to 196). At
  most 1,000 links are labelled; past them the status line says how many were left out. The labels
  show at once, without rendering the pages again, directly in kitty, inside tmux, and in text
  cells.
- **Typing:** each letter hides the labels that no longer match and greys out what was typed in
  the others; when one is left, its link is followed. `Backspace` takes a letter back. `Esc`, or a
  key in no label, drops the labels and does nothing else. Anything that changes the screen (a
  resize, a relayout, a key that moves) drops them too.
- **`F`** labels the same links, and copies the chosen link's address to the clipboard instead of
  following it (Vimium's `yf`): a web address as it is, a local file's path as written, a place in
  the document as its `#name` where it has one.
- **Nothing to label:** a file with no links (a picture, a drawing), or a screen with none on it,
  says so on the status line (`no links on screen`), as `/` says a file has no text.

**Following a link**

- **A place in the document** (a PDF's go-to or named destination, a Word cross-reference, HTML's
  `#name`, a Markdown `#heading`): its page, scrolled so the place is a third of the way down a
  page taller than the window, as a match is. A place the document doesn't have is said
  (`no #usage in this document`).
- **Back:** `u` (and `Backspace`) goes back to where the reader was before the last link followed,
  the newest first, up to 100 jumps; with none left, back to the list as now. A kept search is
  cleared first, as now. `Esc` doesn't step through the jumps: it clears a kept search or the
  labels, then leaves as now. The jumps belong to the file open: they outlive a relayout (kept by
  anchor) and go with going back to the list. Nothing goes forward.
- **Another local file:** a document's relative link (`docs/guide.md`, `../README.md#install`), or a
  PDF's link to another PDF, opens that file in `clv` as a level of its own, as a list's tile is
  gone into, and `u` comes back to the reader's place. The path resolves beside the document;
  then the link's `#fragment`, or the page it names, is gone to. A directory opens as a list. A
  file missing or unreadable is said. A document from stdin or held in an archive can't follow a
  relative link, and says so.
- **A web address:** `http`, `https` and `mailto` only, through `o`'s launchers as a single
  argument, never a shell. The status line says `opening https://…` (through `printable`, cut in
  the middle to fit), then `opened`, or why not. Where `o` refuses (over SSH, no desktop), the
  address is copied to the clipboard instead, and the status line says so.
- **Anything else** (`javascript:`, `file:`, a PDF's launch of a program, an unknown scheme) isn't
  followed; the status line says what kind of link it was.

**Which files have links**

- **PDFs:** their link annotations, with their exact active areas (quads, or else the rectangle).
  A **spreadsheet**'s are whatever the PDF it prints carries.
- **Word, OpenDocument, RTF, Markdown and HTML:** links in the body, tables included; not in
  headers, footers, notes or text boxes, as a search leaves them out.
- **Markdown's headings get GitHub's ids** (lower case, punctuation dropped, spaces to `-`, `-1`,
  `-2` for repeats), so `[see](#install)` lands on `## Install`; an id the document writes itself
  wins.
- **The pages are never changed:** no underline or colour beyond what the document draws.

**Elsewhere**

- `f` and `F` do nothing in printing (`--no-pager`), and in a list. `?` lists them and `u`'s jumps.

## Out of scope

- **Mouse clicks** on a link (SGR mouse reporting): taking the mouse takes plain drag-to-select
  from the terminal, a choice of its own. The links the backends give are what a click would test
  against, so it comes on top of this.
- **OSC 8 hyperlinks** written under a page, for kitty's own click and its hints kitten: whether a
  hyperlink on a cell a picture covers is honoured needs watching in kitty first.
- **`f` in a list**, labelling tiles to open ([backlog](../backlog.md#f-in-a-list-labels-on-tiles)).
- **Going forward** after `u`.
- **Links in an SVG** (`<a href>`) or a Mermaid diagram (`click`), and in a Word document's headers,
  footers, notes and text boxes.
- **Relative links in an archive** (a zip of docs linking one another) or from stdin: said, not
  followed.
- **A local link opened in the desktop's app** instead of `clv`: `o` does it once the file is open.
- **A table of contents to browse** ([backlog](../backlog.md#a-table-of-contents-to-browse)).
- **Where a link goes, shown before following it** (a preview, the address on hover).
- **The labels' letters configured:** the config file's.

## Approach

**Files:**

- `src/source.rs` (the contract): `Source::links() -> Option<Box<dyn Links>>`, `None` by default,
  beside `texts()`. `Links` keeps `Pool`'s contract (`set_queue`, `try_recv` never blocks), asked
  for a page's links, or where a `#name` is (a fragment, once a linked file is open). A `Link` is
  its page, its rectangles in the page's unit from its top left (as `Got::Rects`), and a `Target`:
  `Place { page, y }`, `File { path, fragment, page }` (the path as written, relative), `Web(String)`,
  or `Other(kind)`, never followed, what it was in words. Whatever a source gives is checked as a
  search's is: a page of the document, finite rectangles clipped to it, a cap on links a page.
- `src/source/texts.rs`: its `Queue` serves `Links` too, made generic over what is asked rather
  than copied.
- `src/source/pdfcraft.rs`: keep `DocInfo.links` and the `PageInfo`s of the `inspect` that
  `displayed()` already runs (`Pdf::open` keeps only the sizes today); no worker process. Rectangles
  from the quads, or else the rect, through `PageInfo::user_to_view`. `LinkTarget::Page` to `Place`
  (its `DestView`'s top as `y`), `Uri` to `Web`, `RemoteGoTo` to `File` (its path checked with
  `pdfcraft_engine::links::resolve_remote_document_path`). A `#name` answered by `named_destination`
  on the queue's thread, since it parses the file again.
- `src/source/wordcraft.rs`: a page's links from the shared layout (`Document::shared`, as the
  search reads it): each body paragraph on the page (`Document::on`) cut into runs of the same
  `props_of_char(off).link`, or `field_link` for a `REF \h`, each run's rectangles from
  `selection_rects` as `Reader::rects` has them. `#name` through `doc.bookmarks()` (exact, then
  case-insensitive, as wordcraft's PDF exporter does) and `layout.caret`; a relative path beside
  the document (`beside()`). `markdown_as_html` puts a heading's GitHub slug in it as
  `<a id="…"></a>`, which wordcraft's HTML importer makes a bookmark. `gridcraft.rs`'s `Noted` hands
  `links` on to its PDF, as it does `texts`.
- `src/view/hints.rs` (new): the labels' codes (`sadfjklewcmpgh`, all one length), what has been
  typed, and each label's place on screen from its link's rectangle and its page's place;
  unit-tested alone.
- **Drawing the labels:** directly, a small PNG each, placed with `z=1` over the page, at a cell and
  a pixel offset (`X=`, `Y=`), its letters from a bitmap font of the 14 letters in the core, scaled
  to the cell's height; deleted when the labels go. Inside tmux and in text cells, the label written
  as characters in SGR colours over the page's cells, and the rows written again after, as tmux's
  placeholder rows and text cells already are (`on_glass`). Label images count against `MAX_SENT`
  and `BUDGET` as any image.
- `src/view/pager.rs`: `f` and `F` start the hints, which take the keys while shown, as the search's
  prompt does; `Step::Follow(Target)`; a stack of jumps a level (an `Anchor` each where there is one,
  else a page and `y`; 100 at most), which `u` and `Backspace` pop after a kept search and before
  `Step::Back`; `PAGE_HELP`.
- `src/view/mod.rs`: a file opened by its path as a new level, as a tile is gone into, and its
  fragment or page gone to.
- `src/view/launch.rs`: a web address launched beside a file, through the same launchers (`open
  URL`, `xdg-open URL`), its scheme checked first.
- `src/view/clip.rs`: text copied (`text/plain` through OSC 5522, or the system's clipboard), for
  `F` and the fallback over SSH; it copies only PNGs today.
- Tests: `tests/it/fixtures.rs`, a PDF written by hand with a link to a page, one to a URI, one
  `/GoToR`; `tests/it/view_pager.rs`, `f` on it and on Markdown (`#heading`, a relative `.md`, a
  missing file), `u` back through the jumps then to the list, a web address handed to a fake `open`
  on `PATH` (as Spec-0010's tests), `F` copying; `tests/it/view_tmux.rs`, the labels inside tmux;
  `tests/it/wordcraft.rs`, a DOCX's internal link and a field's.
- Docs: usage (Keys; a section "Following links"), architecture (a section beside the search's),
  `CHANGELOG.md`, ADR-0035.

**Patterns to follow:**

- `Texts` and `search.rs`: a backend's answers arriving later, checked as input.
- `copy()` and `launch()` in `pager.rs`: a message at once, the outcome polled.
- Spec-0010's fake openers on `PATH`, so no test reaches a real browser.

**Decided** (ADR-0035):

- The labels as images or cells over the page, not tinted into its pixels: a tint costs a render a
  page, and the labels show at once.
- Not the pages placed under the text (`z<0`) with the labels as text: every placement would
  change, and how terminals other than kitty take a negative `z` hasn't been watched.
- The slugs are `clv`'s, since neither pulldown-cmark nor wordcraft gives headings ids.
- A web address opens with no key to confirm it; the status line names it.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- 🟡 **Ghostty and WezTerm** — whether they place a `z=1` image with pixel offsets over another
  placement as kitty does: watch it before task 1's drawing, and fall back to cells where they
  don't.
- 🟡 **No confirmation for a web address** — chosen to try; revisit after use.

## Tasks

- [ ] **The contract, PDF links, `f` directly** — `Links`, the queue made generic, pdfcraft's links
  and named destinations, `hints.rs`, the label images in kitty, following a place in the
  document, `u` through the jumps, `?`; ADR-0035, usage, architecture and the changelog. *Files:*
  `src/source.rs`, `src/source/texts.rs`, `src/source/pdfcraft.rs`, `src/view/hints.rs`,
  `src/view/kitty.rs`, `src/view/pager.rs`, `tests/it/fixtures.rs`, `tests/it/view_pager.rs`,
  `docs/`, `CHANGELOG.md`. *Verify:* `cargo test -j 2 --bin clv hints`, `cargo test -j 2 --test it
  view_pager::links`, and `Manual: in kitty, clv a PDF with a table of contents, press f, type a
  label: its page; u: back.`
- [ ] **The labels inside tmux and in text cells** — written over the page's cells, the rows
  written again after. *Files:* `src/view/pager.rs`, `src/view/cells.rs`, `src/view/tmux.rs`,
  `tests/it/view_tmux.rs`, `tests/it/view_pager.rs`. *Verify:* `cargo test -j 2 --test it
  view_tmux::links` and `view_pager::links_in_text_cells`.
- [ ] **Documents' links** — Word, OpenDocument, RTF, Markdown and HTML; bookmarks; Markdown's
  slugs; `Noted` handing on. *Files:* `src/source/wordcraft.rs`, `src/source/gridcraft.rs`,
  `tests/it/wordcraft.rs`, `tests/it/view_pager.rs`, `docs/usage.md`. *Verify:* `cargo test -j 2
  --bin clv slug`, `cargo test -j 2 --test it wordcraft::links`.
- [ ] **Other files, the web, and `F`** — a local file opened as a level with its fragment, a PDF's
  `/GoToR`, a web address through the launchers or copied, `F`, text copied. *Files:*
  `src/view/mod.rs`, `src/view/pager.rs`, `src/view/launch.rs`, `src/view/clip.rs`,
  `tests/it/view_pager.rs`, `docs/`. *Verify:* `cargo test -j 2 --bin clv launch`, `cargo test -j 2
  --test it view_pager::links`, and `Manual: in kitty on macOS, f on a Markdown file's web link:
  the browser opens it; over ssh localhost: copied.`

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [ ] Tests added, each seen failing without the change: name them.
- [ ] Manual: in kitty, Ghostty and WezTerm, `f` on a PDF and a Markdown file, following a place,
  a local file and a web address, `u` back; inside tmux and with `--protocol blocks`, the labels.
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- 2026-11-15 — Ready (spec-new).
