# Spec-0011: Follow a document's links with hints

- **Status:** Implementing
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
  [ADR-0018](../adr/0018-wordcraft-stays-before-its-proofing-needs-a-checkout-to-build.md)
  (wordcraft's pin, which stays);
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
its own that `u` comes back from. A web address (`http`, `https`, `mailto` only, checked) goes to
the browser through the desktop's opener `o` already uses, never a shell, the status line saying
`opening https://…` and then that it did; over SSH or with no desktop, it is copied to the
clipboard instead. `F` labels the same links and copies the chosen one's address. `u` goes back
through the links followed, then to the list. Backends say where each page's links are and where
they lead, through an optional `Source` method beside `texts()`, so the viewer stays without
formats. PDFs, Word and OpenDocument documents, RTF, Markdown and HTML have links; Markdown's
headings get GitHub's ids, and HTML's elements with an `id` are places, so `#heading` links land.
Mouse clicks and OSC 8 hyperlinks come later.

## Requirements

**The hints**

- **`f`** in a file's pages labels every link at least partly on screen: on every page shown side
  by side, and on a page taller than the window, in the part shown.
- **When they show.** `f` asks the source for the links of the pages on screen, and the status line
  says `reading links…` until every one has answered or failed, at most 2 s; `Esc` meanwhile drops
  it. The labels are then made once, from what came: never renumbered. A page that answered late,
  or failed, has no labels, and the status line names it (`links on page 40 unread`).
- **The labels** look like Vimium's: a small yellow box, dark letters, at the top left of the part
  of its link on screen, over the page; labels may overlap, a later one on top. Their letters are
  `sadfjklewcmpgh` (Vimium's default), every label the same length, the shortest that covers the
  links on screen (one letter up to 14, two up to 196, three past). At most 1,000 links are
  labelled, in reading order (page, then top, then left); past them the status line says how many
  were left out. The labels show without rendering the pages again, directly in kitty, inside tmux,
  and in text cells.
- **Typing:** each letter hides the labels that no longer match and greys out what was typed in
  the others; when one is left, its link is followed. `Backspace` takes a letter back. `Esc`, or a
  key in no label, drops the labels and does nothing else. Anything that changes the screen (a
  resize, a relayout, a key that moves) drops them too.
- **`F`** labels the same links, and copies the chosen link's address to the clipboard instead of
  following it (Vimium's `yf`). What it copies is what the backend gives as the link's address: a
  web address as it is, a local file's path as written with its `#fragment`, a place in the
  document as its `#name` where it has one. A link with no address (a PDF's link to a page, which
  pdfcraft has already resolved) says where it goes instead (`goes to page 12`); one never followed
  (`Other`) says what it is. Neither is copied.
- **Nothing to label:** a file with no links (a picture, a drawing), or a screen with none on it,
  says so on the status line (`no links on screen`), as `/` says a file has no text.

**Following a link**

- **A place in the document** (a PDF's go-to or named destination, a Word cross-reference, HTML's
  `#name`, a Markdown `#heading`): its page, scrolled so the place is a third of the way down a
  page taller than the window, as a match is. A place the document doesn't have is said
  (`no #usage in this document`). With `--pages`, a place on a page not given is said (`page 12
  isn't among --pages`) and not gone to; a page given twice is gone to where it is first.
- **Back:** `u` (and `Backspace`) goes back to where the reader was before the last link followed,
  the newest first, up to 100 jumps; with none left, back to the list as now. A kept search is
  cleared first, as now. `Esc` doesn't step through the jumps: it clears a kept search or the
  labels, then leaves as now. The jumps belong to the file open: they outlive a relayout (handed to
  the new level as the search is, kept by anchor) and go with going back to the list. Nothing goes
  forward.
- **Another local file:** a document's relative link (`docs/guide.md`, `../README.md#install`), or a
  PDF's link to another PDF, opens that file in `clv` as a level of its own, as a list's tile is
  gone into, and `u` comes back to the reader's place. The path resolves beside the document and
  is made real (`canonicalize`: symbolic links followed); only a regular file or a directory is
  opened, a directory as a list: a FIFO, a socket or a device is refused before anything opens it,
  as is a path that doesn't resolve. Then the link's `#fragment`, or the page it names, is gone
  to. Links followed into files go at most 16 levels deep, `A` to `B` to `A` counting each; past
  that the status line says so. A document from stdin or held in an archive can't follow a
  relative link, and says so.
- **A web address** is checked in the core before any launch: `http`, `https` or `mailto`, at most
  4 KiB, with no control character, no bidirectional mark and no other invisible character; and a
  `mailto` whose parameters name a file to read (`attach`, `insert`, matched as prefixes, so
  `attachment` too; looked for through up to 8 layers of percent-encoding) is refused, since some
  mail apps attach or insert the file it names. It goes through `o`'s launchers as a single
  argument, never a shell. The status line says `opening https://…` (through `printable`, cut in
  the middle to fit), then `opened`, or why not. Where `o` refuses (over SSH, no desktop), the
  address is copied to the clipboard instead, and the status line says so.
- **Anything else** (`javascript:`, `file:`, a PDF's launch of a program, an unknown scheme) isn't
  followed; the status line says what kind of link it was.

**The copy**, for `F` and for a web address over SSH: OSC 5522's `text/plain` where the terminal
speaks it; else OSC 52 (xterm's, which carries text, and which tmux passes with `set-clipboard`);
else, on this machine, the system's clipboard. Where none applies, the status line shows the
address and says it couldn't be copied.

**Which files have links**

- **PDFs:** their link annotations, with their exact active areas (quads, or else the rectangle).
  A **spreadsheet**'s are whatever the PDF it prints carries.
- **Word, OpenDocument, RTF, Markdown and HTML:** links in the body, tables included, and a Word
  cross-reference field with `\h` (`REF` or `PAGEREF`); not in headers, footers, notes or text
  boxes, as a search leaves them out.
- **Places in HTML and Markdown:** an element with an `id` is a place (`<h2 id="usage">`,
  `<div id="usage">`), not only `<a id>` and `<a name>`. Markdown's headings get GitHub's ids (lower
  case, punctuation dropped, spaces to `-`, `-1`, `-2` for repeats), so `[see](#install)` lands on
  `## Install`; an id the document writes itself wins.
- **The pages are never changed:** no underline or colour beyond what the document draws.

**Limits**, each checked before what it bounds is allocated, past it what is over left out and
said: 1,024 links a page; 64 rectangles a link; 4 KiB an address or a path; 256 bytes a `#name`;
the answers waiting held to the queue's 4 MiB (`MAX_WAITING`); 100 jumps; 16 levels of files
followed into.

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
- **Labels kept from overlapping** one another: a later one is drawn on top.

## Approach

**Files:**

- `src/source.rs` (the contract): `Source::links() -> Option<Box<dyn Links>>`, `None` by default,
  beside `texts()`. `Links` keeps `Pool`'s contract (`set_queue`, `try_recv` never blocks), asked
  for a page's links, or where a `#name` is (a fragment, once a linked file is open). A `Link` is
  its page, its rectangles in the page's unit from its top left (as `Got::Rects`), its `address`
  (`Option<String>`, what `F` copies), and a `Target`: `Place { page, y }`,
  `File { path, fragment, page }` (the path as written, relative), `Web(String)`, or `Other(kind)`,
  never followed, what it was in words. Whatever a source gives is checked as a search's is,
  against the Limits: a page of the document, finite rectangles clipped to it.
- `src/source/texts.rs`: its `Queue` serves `Links` too, made generic over what is asked rather
  than copied; its `MAX_WAITING` holds the answers waiting.
- `src/source/pdfcraft.rs`: keep `DocInfo.links` and the `PageInfo`s of the `inspect` that
  `displayed()` already runs (`Pdf::open` keeps only the sizes today); no worker process. Rectangles
  from the quads, or else the rect, through `PageInfo::user_to_view`. `LinkTarget::Page` to `Place`
  through the destination page's own `PageInfo`: each `DestView` to a point in user space, then to
  the displayed page (`user_to_view`, or `dest_fraction` with its rotation), so that a page turned a
  quarter goes by `left`; `Top`, `Fit`, `FitV` and a missing operand to the page's top; `Xyz` and
  `FitH` by their `top`, `FitR` by its rectangle's top. `Uri` to `Web`, `RemoteGoTo` to `File` (its
  path checked with `pdfcraft_engine::links::resolve_remote_document_path`, which does no I/O: the
  core's checks on the file come after). A `#name` answered by `named_destination` on the queue's
  thread, since it parses the file again.
- `src/source/wordcraft.rs`: a page's links from the shared layout (`Document::shared`, as the
  search reads it): each body paragraph on the page (`Document::on`) cut into runs of the same
  `props_of_char(off).link`, each run's rectangles from `selection_rects` as `Reader::rects` has
  them. A cross-reference field (`InlineObject::Field { instr, .. }`) through a parser of the
  backend's own, bounded, for `REF` and `PAGEREF` with `\h`: wordcraft's `field_link` is its UI's
  (`pub(super)`, through `wordcraft_engine`, which `clv` doesn't build) and its layout's
  `reference_link` is `pub(crate)`. `#name` through `doc.bookmarks()` (exact, then
  case-insensitive, as wordcraft's PDF exporter does) and `layout.caret`; a relative path beside
  the document (`beside()`). Since wordcraft's HTML importer makes a bookmark of an `id` only on
  `<a>`, a pass of the backend's own over the HTML, as the fenced blocks' scanner is
  (`blocks_drawn`), puts an `<a id="…"></a>` after the opening tag of any other element with an
  `id`, and `markdown_as_html` one for a heading's GitHub slug. `gridcraft.rs`'s `Noted` hands
  `links` on to its PDF, as it does `texts`.
- `src/view/hints.rs` (new): the labels' codes (`sadfjklewcmpgh`, all one length), what has been
  typed, the wait for the pages' answers, and each label's place on screen from the part of its
  link on screen and its page's place; unit-tested alone.
- `src/view/link.rs` (new): a web address checked (its scheme, length, characters, a `mailto`'s
  parameters, decoded up to 8 times); and a local path resolved beside its document, made real, and
  taken only as a regular file or a directory. The core's own, not pdfcraft's, since every backend's
  links go through it.
- **Drawing the labels:** directly, one image holding the 14 letters twice, plain and greyed, in
  yellow boxes, drawn from a bitmap font of the core's at the cell's height, sent when `f` shows the
  labels; each letter of a label is a placement of its part of that image (`x`, `y`, `w`, `h`, as
  `kitty.rs` already places part of a page) with `z=1` over the page, at a cell and a pixel offset
  (`X=`, `Y=`). One image against `MAX_SENT` and `BUDGET`, deleted when the labels go, sent again if
  the terminal says it is missing (`Key::Missing`). Inside tmux and in text cells, the label written
  as characters in SGR colours over the page's cells, the rows written again after, as tmux's
  placeholder rows and text cells already are (`on_glass`, which knows which rows carry labels).
  Inside tmux, the placeholder cell after a label names its row and column outright, as the
  protocol lets any placeholder, since a run of them takes those from its first cell.
- `src/view/pager.rs`: `f` and `F` start the hints, which take the keys while shown, as the search's
  prompt does; `Step::Follow(Target)`; a stack of jumps a level (an `Anchor` each where there is one,
  else a page and `y`; 100 at most), which `u` and `Backspace` pop after a kept search and before
  `Step::Back`, and which `replace` hands to the new level as `take_search` does; `PAGE_HELP`.
- `src/view/mod.rs`: a file opened by its path as a new level, as a tile is gone into, and its
  fragment or page gone to; the levels followed into counted.
- `src/view/launch.rs`: a web address launched beside a file, through the same launchers (`open
  URL`, `xdg-open URL`), once `link.rs` has passed it.
- `src/view/clip.rs`: text copied: OSC 5522's `text/plain`, else OSC 52, else the system's
  clipboard on this machine; it copies only PNGs today.
- Docs: usage (Keys; a section "Following links"), architecture (a section beside the search's),
  `CHANGELOG.md`, ADR-0035.

**Patterns to follow:**

- `Texts` and `search.rs`: a backend's answers arriving later, checked as input.
- `copy()` and `launch()` in `pager.rs`: a message at once, the outcome polled.
- Spec-0010's fake openers on `PATH`, so no test reaches a real browser.

**Decided** (ADR-0035):

- The labels as placements of one image of letters, or as cells, over the page, not tinted into
  its pixels: a tint costs a render a page, and the labels show at once. One image, not one a
  label, since `MAX_SENT` counts images (12, more only for more pages on screen).
- Not the pages placed under the text (`z<0`) with the labels as text: every placement would
  change, and how terminals other than kitty take a negative `z` hasn't been watched.
- The slugs and HTML's ids as bookmarks are `clv`'s, since neither pulldown-cmark nor wordcraft's
  importer gives them.
- The checks on a link's target are the core's (`link.rs`), whatever backend gave it.
- A web address opens with no key to confirm it; the status line names it.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- 🟡 **Ghostty and WezTerm** — whether they place part of an image at `z=1` with pixel offsets over
  another placement as kitty does: watch it before task 2's drawing, and fall back to cells where
  they don't.
- 🟡 **No confirmation for a web address** — chosen to try; revisit after use.

## Tasks

- [x] **The contract and PDF links** — `Links`, `Link`, `Target`, the queue made generic, the
  checks on what a source gives; pdfcraft's links, their destinations and named destinations;
  `link.rs`'s checks. No key yet. *Files:* `src/source.rs`, `src/source/texts.rs`,
  `src/source/pdfcraft.rs`, `src/view/link.rs`, `tests/it/fixtures.rs` (a PDF written by hand: a link
  to a page, a named destination, a URI, a `/GoToR`, a page turned a quarter, a crop box, a
  UserUnit). *Verify:* `cargo test -j 2 --bin clv links` (each `DestView` on a turned and cropped
  page, the caps, a malformed rectangle dropped) and `cargo test -j 2 --bin clv link::` (a `mailto`
  with `attach` and with it percent-encoded twice refused, hidden characters, 4 KiB, a FIFO and a
  device refused, a symbolic link to one too).
- [ ] **`f` directly, a place, and `u`** — `hints.rs`, the image of letters and its placements, the
  wait for the pages' answers, following a place in the document, the jumps through `u` and
  `Backspace` and through a relayout, `--pages`, `?`; ADR-0035, usage, architecture, the changelog.
  *Files:* `src/view/hints.rs`, `src/view/kitty.rs`, `src/view/pager.rs`, `tests/it/view_pager.rs`,
  `docs/`, `CHANGELOG.md`. *Verify:* `cargo test -j 2 --bin clv hints` (codes, typing, a page late),
  `cargo test -j 2 --test it view_pager::links` (labels sent as one image, a place gone to, `u`
  back then to the list, a page not in `--pages`, `Esc` while reading), and `Manual: in kitty,
  Ghostty and WezTerm, clv a PDF with a table of contents, press f, type a label: its page; u: back.`
- [x] **The labels inside tmux and in text cells** — written over the page's cells, the placeholder
  after each label naming its row and column, the rows written again after. *Files:*
  `src/view/pager.rs`, `src/view/cells.rs`, `src/view/tmux.rs`, `src/view/kitty.rs`,
  `tests/it/view_tmux.rs`, `tests/it/view_pager.rs`. *Verify:* `cargo test -j 2 --test it
  view_tmux::links` (a label over a row's first cell and a middle one, cancelled, a client
  attaching later) and `view_pager::links_in_text_cells`.
- [ ] **Documents' links** — Word, OpenDocument, RTF, Markdown and HTML; the field parser;
  bookmarks; HTML's ids and Markdown's slugs as bookmarks; `Noted` handing on. *Files:*
  `src/source/wordcraft.rs`, `src/source/gridcraft.rs`, `tests/it/wordcraft.rs`,
  `tests/it/view_pager.rs`, `docs/usage.md`. *Verify:* `cargo test -j 2 --bin clv slug`,
  `cargo test -j 2 --bin clv field_link`, `cargo test -j 2 --test it wordcraft::links` (a DOCX's
  internal link and `REF \h`, `<h2 id>`, `<div id>`, a Markdown heading and its repeat).
- [ ] **Other files, the web, and `F`** — a local file opened as a level with its fragment, a PDF's
  `/GoToR`, the 16 levels, a web address through the launchers or copied, `F` and what it copies,
  text through OSC 5522, OSC 52 or the system's clipboard. *Files:* `src/view/mod.rs`,
  `src/view/pager.rs`, `src/view/launch.rs`, `src/view/clip.rs`, `tests/it/view_pager.rs`, `docs/`.
  *Verify:* `cargo test -j 2 --bin clv launch`, `cargo test -j 2 --bin clv clip`, `cargo test -j 2
  --test it view_pager::links` (list → file → linked file → `u` → `u`, a missing file, a FIFO, from
  stdin, a web address to a fake `open` on `PATH`, `F` on each kind, over SSH copied through OSC
  52), and `Manual: in kitty on macOS, f on a Markdown file's web link: the browser opens it; over
  ssh localhost: copied.`

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [ ] Tests added, each seen failing without the change: name them.
- [ ] Manual: in kitty, Ghostty and WezTerm, `f` on a PDF and a Markdown file, following a place,
  a local file and a web address, `u` back; inside tmux and with `--protocol blocks`, the labels.
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- 2026-11-15 — Ready (spec-new).
- 2026-11-15 — Revised after a Codex review: a web address checked in the core (a `mailto`'s
  file parameters among it), a linked file only a regular file or a directory, the limits in
  numbers, the labels as placements of one image, Word's fields parsed by the backend, HTML's ids
  as places, a PDF's destinations through their page, what `F` copies, `--pages` and relayout, OSC
  52, when the labels show, tmux's placeholders after a label, five tasks (spec-update).
- 2026-11-15 — Implementing ([notes](0011-follow-a-documents-links-impl-20261115_1751.md)).
