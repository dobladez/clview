# Spec-0006: Search a PDF's text

- **Status:** Ready
- **Date:** 2026-11-13
- **Related:** [README's Plans](../../README.md#plans) ("Search!": PDFs and spreadsheets here,
  Markdown, HTML and Word in [Spec-0007](0007-search-markdown-html-and-word-documents.md));
  [Filtering a list](../usage.md#filtering-a-list), whose prompt this mirrors;
  [Sources](../architecture.md#sources-srcsourcers),
  [Getting pages to the terminal](../architecture.md#getting-pages-to-the-terminal),
  [What is kept in the terminal](../architecture.md#what-is-kept-in-the-terminal-pagerrs) and
  [Text cells](../architecture.md#text-cells-cellsrs);
  [ADR-0026](../adr/0026-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md)
  (the worker process this reads text in);
  [ADR-0006](../adr/0006-spreadsheets-printed-by-gridcraft-to-a-pdf-the-pdf-backend-shows.md)
  (spreadsheets as a PDF)

## Summary

clv shows a document's pages as pictures, so there is no finding a word in one: `/` in a file's
pages only says that it filters lists. This slice searches PDFs, and spreadsheets, which reach the
screen as a PDF. `/` opens a prompt in the status line, and the pages' text starts to be read then,
in a worker process of clv's own, outward from the page you are on. While you type, the status line
counts the matches in what has been read (`/word 17…`); `Enter` searches, as in less: the view goes
to the first match from the top of the screen, and the matches on screen are tinted, the one you are
on more strongly. `n` and `N` go from match to match while the search is kept, and `Esc` clears it,
after which `n` turns pages again.

The work is split along the code's layers. A backend gives its text in chunks, in the document's
order (a PDF's chunk is a page), and the rectangles a range of a chunk covers when asked, through a
new optional `Source::texts`; the viewer folds case, matches, and tints what it is given, the same
for every backend with text. The PDF backend reads text in a worker, as an SVG's labels are drawn
(ADR-0026), for memory more than time: a page with millions of glyphs takes gigabytes to read,
which a worker's heap cap stops and a thread's doesn't. No dependency and no algorithm of our own:
`str::find` on folded text, a millisecond a megabyte. [Spec-0007](0007-search-markdown-html-and-word-documents.md)
gives Markdown, HTML and Word documents their text through the same contract, a paragraph a chunk.

Tinting as you type is left out: each page resent costs 35 to 110 ms of PNG encoding on the
pager's thread where the terminal takes no temporary files, and more inside tmux, so a key at a time
it would hold the keys up (the measurements are under Open questions).

## Requirements

**Typing**

- `/` in a file's pages opens a prompt in the status line (`/word`), as a list's filter does: at
  most 256 characters (`MAX_FILTER`), `Backspace` takes one back, every key that types a character
  is part of it, `Ctrl-C` still quits, other keys (arrows, `PgUp`) do nothing. A number typed
  before `/` is dropped. The view doesn't move, nor is anything tinted, while typing.
- While typing, the status line counts the matches in the text read so far: `/word 17…` while text
  is still being read, `/word 17` once all is. Counting never holds a key up: a character added
  narrows the matches already found, and anything else counts again between keys, the count saying
  `…` until it is done.
- Upper and lower case match letter by letter, by the filter's rule (`É` matches `é`, `e` doesn't
  match `é`). A run of whitespace in the query or the text matches a run in the other. A query of
  whitespace alone matches nothing. Matches don't overlap, are found left to right, and never run
  from one chunk onto the next: in a PDF, a phrase across a page break isn't found.
- `Enter` searches and keeps the search. With a match known at or after the top of the screen, past
  the end round to the start, the view goes to it; with none known yet, the first one read at or
  after the start brings the view to it, once, and no match found later moves it. `Esc` while
  typing leaves no search. `Enter` or `Backspace` with nothing typed leave no search.

**A kept search**

- `n` goes to the next match, `N` to the one before, past the ends round to the other, saying
  `search wrapped`; a number first goes that many matches on (`3n`). From the match you are on
  while it is on screen; once you have paged away, from the top of the screen. A match on a page not
  shown brings that page on screen; zoomed in to a page that scrolls, the match is scrolled into
  view. `→` and `←` still turn pages.
- A number being typed is cleared by `Esc` or `Backspace` first, as now. Then `Esc`, `Backspace` or
  `u` clear a kept search, before they go back to the list or quit; `n` turns pages again after.
  `/` again edits the query kept.
- `N` becomes one of the viewer's keys (`VIEWER_KEYS`), and `?` lists `/`, `n` and `N` in a file's
  pages.
- The status line says `/word 3/17`, the match you are on and how many, with `…` after while text is
  still being read, or `no match`; its right side keeps its place, as with the filter. The query is
  shown through `printable`.

**Highlights**

- After `Enter`, `n`, `N` or a match arriving on a page on screen, its matches are tinted, the one
  you are on more strongly, with colours that read on a PDF's white and on Markdown's dark paper
  alike. Clearing the search takes the tint off.
- A page whose tint changes is asked of the render pool again and tinted as it arrives, before it
  is encoded or made into text cells; the page as it was stays on screen until then. What was sent
  is known by its tint, so a page whose tint didn't change is only placed again. Keys read together
  tint once, for where they end: holding `n` doesn't queue a resend a key.
- The terminal holds no more than `MAX_SENT` and `BUDGET` allow while tints change: a page sent
  again under a new id, the old image deleted once the new one is placed. A resend for a tint is not
  one of the `MAX_RESENDS` after kitty loses a page.
- In text cells the tint shows as far as the cells' colours carry it; without colour (`NO_COLOR`,
  ASCII) it doesn't, and the status line still says where the match is.
- `c` copies the page untinted, as rendered; printing (`--no-pager`) has no search.

**Reading the text**

- A source with no text to search (an image, an SVG) says `no text to search` at `/`, and opens no
  prompt. A PDF whose pages hold no text (a scan without OCR) says `no text in this file` once all
  are read; one with text but no match, `no match`.
- A PDF is searched for all the text pdfcraft reads in it: an invisible OCR layer, form fields'
  values and comments' appearances too. A word hyphenated across a line's end is two words, as the
  PDF draws them: a known difference in usage. Text pdfcraft reads beyond a clip inside the page is
  dropped or documented (Open questions).
- The text is read once, from `/` on, whatever is typed, in a worker process (`clv --worker
  pdf-text`) given the PDF's bytes once: the copy the viewer shows and its password, not the file
  read again. It reads outward from the page on screen. Each page has a CPU budget, and the document
  one too; the worker has a heap cap of its own. A page past them kills the worker and costs only
  that page (`/word 3/17 · page 40 unread`), a new worker going on with the next; past three kills,
  or the document's budget, the rest is left unread and the status line says how far it read. A PDF
  larger than the worker takes says so at `/`. A worker that can't start (`MAX_WORKERS` alive)
  leaves the search to what was read, and says so.
- Leaving the file or clearing the search kills the worker without waiting for it; the keys never
  wait on it.
- The viewer keeps each chunk's text folded once, with what it takes to map a match back to the
  original's characters: at most 64 MiB in all and 1 MiB a chunk. Past 64 MiB, reading stops and
  the status line says how far it read. The rectangles of a match are asked for when its page is on
  screen, and the backend keeps or reads them again as it sees fit.
- At most 10,000 matches are kept and counted (`10000+`), the first in document order; the cap
  never stops the reading.
- Whatever a backend gives is checked: a chunk one of its own, text within the cap; a rectangle on a
  page of the source's, finite, clipped to the page. What fails costs that chunk or rectangle.
- Spreadsheets are searched through their PDF, a workbook that opened with warnings too.

## Out of scope

- Markdown, HTML and Word documents: [Spec-0007](0007-search-markdown-html-and-word-documents.md).
- Tinting as you type, with the view moving on a pause: a later task, once the latency from a key
  to the tinted screen is measured under some 50 ms in kitty with temporary files, in kitty without
  them, and inside tmux.
- Plain text (`.txt`), not opened at all yet: its own item in
  [README's Plans](../../README.md#plans).
- An SVG's `<text>` and labels; OCR of our own.
- Searching a list's files (which hold the word), and `/` as a search in lists, the filter on a key
  of its own (in [README's Plans](../../README.md#plans)).
- Options: regular expressions, whole words, case that matters (smartcase), accents folded, a
  search from the command line (less's `+/word`).
- Joining a word hyphenated across a PDF's line end: a guess (`well-` / `known`), left out.
- Remembering a search across files, or after going back to the list.

## Approach

**Files:**

- `src/source.rs`: the contract. `Source::texts(&self) -> Option<Box<dyn Texts>>`, `None` by
  default. `Texts` says how many chunks there are and which chunk a page starts with, and keeps
  `Pool`'s contract for the work: `set_queue(wanted)` replaces what is wanted, nearest first, each
  a chunk's text or the rectangles of a character range of a chunk; `try_recv()` never blocks, and
  gives a chunk's text (`Result<String, String>`) or a range's rectangles (`Result<Vec<(page,
  [f32; 4])>, String>`, in the page's unit, top left, y down, as `Source::size` measures it, one
  for each line it covers); a result arrives only while it is wanted; dropping it ends its work
  without waiting. The doc comment says a chunk's text is the text as it should be searched: what a
  line's end, a hyphen there or a ligature becomes is the backend's to decide, and the viewer never
  corrects it. A chunk is whatever the backend's text comes in; a match never crosses one.
  `WORKERS` lists the PDF backend's worker, behind `pdf`.
- `src/source/pdfcraft.rs`: the `pdf-text` `Worker`: a `Session` that takes the shown PDF's bytes
  and password once and keeps a `PageRenderer`, then answers a page with its text, or a range's
  rectangles, from `filtered_text(page, TextFilter::default())`: the glyphs' text joined with a
  space where `space_before` says and between lines (`line_of`), a ligature's letters sharing its
  box; a range's glyphs' `rect`s merged along each line. `Pdf::texts`: a thread that spawns the
  worker, asks for what is queued, starts another past a kill, and sends what comes back. The pages'
  text is chunk for page. pdfcraft stays named only here.
- `src/source/gridcraft.rs`: `Noted` forwards `texts()` to the PDF source it wraps, by the contract,
  never naming the PDF backend (ADR-0006).
- `src/source/worker.rs`: its `cfg_attr` for builds without `svg-html`; a handle that kills a
  worker while another thread waits on its answer (`Link::request` holds the `Link`, and only
  `Drop` kills today); a heap cap for each kind of worker, where `MAX_HEAP` is one for all; a
  longest request large enough for the PDFs searched, which `takes()` says; `MAX_WORKERS` shared
  with the labels' workers.
- `src/view/search.rs`, new, pure, no IO: the query and the text folded a character at a time (the
  filter's `fold`, moved here from `pager.rs`), whitespace runs made one, and the map from a folded
  match back to the original's characters. The chunks' text kept and its caps, the matches by chunk
  in document order, narrowed as the query grows, the match you are on (by chunk and character, so a
  later chunk's arrival leaves it where it is), the cap on matches, and `tint(pixels, width, height,
  scale, rects, strong)`. Unit tests, and property tests: no panic on any text, query or rectangle;
  matches within their text; rectangles within the page.
- `src/view/pager.rs`: the prompt in a file's pages, beside `Filter` (`start_filter`, `filter_key`);
  `n`, `N`, a number before them, and the keys that clear; the status line; `NO_SEARCH` goes, `N`
  joins `VIEWER_KEYS` and `?`'s list. The tint: a generation in `Sent`, pages asked of the pool
  again, `tint` in `receive` before `payload`, the old image deleted once the new one is placed.

**Patterns to follow:**

- `svg/html.rs`'s labels worker and its tests in `tests/it/resvg.rs` for the worker, its caps and
  its kills; `Pool` and `threads.rs` for `Texts` and its tests.
- The list's `Filter` in `pager.rs` for the prompt, its keys and its status line; the pager's fake
  sources in its tests (`LaidAt`) for a fake `Texts`.
- Tests where their kind is: `tests/it/view_pager.rs` for the pager, on PDFs from
  `tests/it/fixtures.rs`, whose pages say `<title>, page N` in Helvetica, and for a worker's kill
  seen with `ps`, as its tests of the labels' workers do; `tests/it/view_tmux.rs` for a tinted page
  resent inside tmux; `tests/it/gridcraft.rs` for a workbook.

**Decided:**

- Backends give text and rectangles, the viewer matches and tints.
- `Enter` searches; nothing is tinted, nor does the view move, while typing.
- PDF text in a worker process, for its heap cap: pdfcraft's `RenderPool` reads text cancellably
  (`RequestKind::Text`), but one page of 3 million glyphs peaked at 1.3 GB, and of 10 million at
  3.5 GB. An ADR, in the worker's commit, with those measurements and the caps chosen.
- Matches tinted into the pixels, not an overlay image: kitty can place an image at a pixel offset
  within a cell, but tmux's placeholders are cells, and one mechanism serves both, and text cells.
  An ADR, in the tint's commit.
- A tint changed is a page rendered again, not a copy kept: what the pager keeps is the terminal's
  budget, and the copies would be clv's memory, outside it.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- 🟡 **The worker's caps** — measured at pdfcraft `168a3cb`, release, one core, on five books of 308
  to 688 pages: parsing 1–15 ms, a page's text 1.0–3.7 ms median, 25 ms at most, a whole book
  0.7–2.9 s and 0.3–1.4 MB of text. The heap cap, the CPU budgets (a page's and the document's) and
  the largest PDF sent are chosen from these and the hostile pages above, and the ADR records them.
- 🟡 **Text clipped away** — pdfcraft drops glyphs outside the page (a million run off it left 542),
  but its text device ignores clips inside it (`push_clip_path` does nothing): a spreadsheet's
  string cut off by the next cell matches, and its tint falls on that cell. Spike it in the worker's
  task (workbooks with a clipped string, a `###` number, a hidden column, a formula), then drop
  glyphs outside their clip in the backend or say so in usage.
- 🟡 **The tint's colours** — read on white and on the terminal's dark, the match you are on told
  from the others, and in text cells.

## Tasks

One checkbox each: a session's work, and a commit that builds and passes on its own.

- [ ] **The contract and the search, without a backend** — `Source::texts`, `Texts`, `search.rs`,
  the prompt, the count while typing, `Enter`, `n`/`N` and their numbers, the keys that clear,
  `no text to search`, the status line. *Files:* `src/source.rs`, `src/view/search.rs`,
  `src/view/pager.rs`, `src/view/mod.rs`. *Verify:* `cargo test -j 2 --lib search`; `cargo test -j 2
  --lib pager` with a fake `Texts` of five chunks: `/page 3⏎` lands on page 3 and says `1/1`;
  `/page⏎ n n` lands on page 3, `N` back, `n` past the last says `search wrapped`; `3n`; text
  arriving out of order leaves the match you are on; `Esc` clears, then `n` turns a page; a source
  without text says `no text to search`.
- [ ] **A PDF's text, read in a worker** — the `pdf-text` worker and `Pdf::texts`, the changes to
  `worker.rs`, `Noted` forwarding, the caps measured, the clipping spike, the ADR, and usage's
  section on searching. *Files:* `src/source.rs` (`WORKERS`), `src/source/pdfcraft.rs`,
  `src/source/gridcraft.rs`, `src/source/worker.rs`, `tests/it/view_pager.rs`,
  `tests/it/gridcraft.rs`, `docs/adr/`, `docs/usage.md`, `docs/architecture.md`, `CHANGELOG.md`.
  *Verify:* `cargo test -j 2 --test it view_pager`: on a five-page fixture, `/page 3⏎` shows page 3
  and `1/1`; a PDF opened from a list searches; clearing the search leaves no worker (`ps`); `cargo
  test -j 2 --test it gridcraft`: a workbook with warnings searches its cells; each backend alone,
  `pdf,svg`, and none.
- [ ] **Matches tinted** — the tint's generation in `Sent`, the pages rendered again, `tint` in
  `receive`, the old image deleted, keys read together tinting once, text cells; the ADR. *Files:*
  `src/view/search.rs`, `src/view/pager.rs`, `src/view/cells.rs` if needed,
  `tests/it/view_pager.rs`, `tests/it/view_tmux.rs`, `docs/architecture.md`, `docs/adr/`,
  `docs/usage.md`. *Verify:* `cargo test -j 2 --test it view_pager`: through a terminal without
  temporary files, the PNG of a page with a match is tinted inside the match's rectangle and nowhere
  else, the match you are on more strongly, and after `Esc` comes again untinted; twenty `n` then
  `Esc` leave at most `MAX_SENT` images, each deleted once replaced; `view_tmux`: a tinted page shows
  in a second client. Manual: in kitty, a 10-page PDF, `/the⏎`, `n` through the matches, `Esc`;
  then inside tmux, and with `--protocol ascii`.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass, each
  backend alone, `pdf,svg`, and none.
- [ ] Tests added, each seen failing without the change: `search.rs`'s matching and property
  tests, the pager's prompt, `n`/`N` and clearing on a fake `Texts`, a PDF searched and its worker
  gone with the search, `Noted`'s forwarding, the tint's pixels and the images it leaves.
- [ ] Manual: in kitty, a 500-page PDF: `/` and a word, the count growing as it is read, `Enter` on
  the first match within a second, the keys answering throughout; the same inside tmux and over SSH.
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)): usage's
  keys, a section on searching, the known difference on hyphens; architecture's contract, worker and
  tint; two ADRs; the changelog. README's "Search!" stays until Spec-0007 is Done.

## Changelog

- 2026-11-12 — Ready (spec-new), as "Search a document's text".
- 2026-11-13 — Split after Codex's review: PDFs here, wordcraft's documents in Spec-0007.
- 2026-11-13 — After a second review: `Enter` searches (measured), the contract in chunks and
  rectangles asked for, tasks reordered, a tint changed is a page rendered again, the worker for its
  heap, the caps fixed.
