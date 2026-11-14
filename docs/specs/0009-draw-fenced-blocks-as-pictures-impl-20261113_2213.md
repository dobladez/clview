# Spec-0009: implementation notes (2026-11-13)

Notes made while implementing [Spec-0009](0009-draw-fenced-blocks-as-pictures.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 1: Fenced blocks in Markdown

**Changes from the plan:**

- **The constructor is `Format::of(name, extensions)`**, not `Format::named` as the Approach has it:
  `Format::named(&self, path)` is already the test of a file's name. `.fenced(&[…])` as planned.
- **A block's name is `block.mmd`**, the format's first extension, as the requirement says; its
  example (`block.mermaid`) had the second. Nothing reads the name but messages.
- **`Input::held_bounded`, not `Input::held`:** the block's text is handed over with a bound, so a
  backend that reads only so much of a file (`Input::at_most`, Mermaid's 1 MiB) is given no copy of
  a longer text to refuse. `held_bounded`'s `dead_code` expectation takes `word` in.
- **`Pictures::draws(language)`** beside `Pictures::fenced`: `markdown_as_html` gathers a block's
  text only for a language some backend lists, and writes every other block's events through as
  they come.

**Files beyond the plan's:** `tests/it/view.rs` (`--help`'s Mermaid row now has its fence),
`tests/it/view_pager.rs` (the block's turn at "turned into SVG once", beside the picture by path's,
where `ps` is watched), `docs/adr/README.md`, `CLAUDE.md` and `CONTRIBUTING.md` (what `picture.rs`
is, and what `tests/it/mermaid.rs` covers).

**Decided here:**

- **The key** is `fenced:<16 hex digits>:<n>`: 64 bits from one `RandomState` for each
  `Pictures`, so for each reading, and the block's number among those drawn. With a scheme's colon
  it is never a path: `local` refuses it, so a key that isn't a block's reads no file. `measured`
  looks a reference up among the blocks' keys before it asks for a directory, which is what draws a
  block through a pipe.
- **What counts among the 1,024:** every block in a listed language, the ones that fail too, as
  every path does. A block past them isn't opened. Two blocks of the same text are two pictures:
  the listing backend's cache makes the second cheap, and a document holds one PNG a key.
- **The note:** `one of its blocks could not be drawn, and is shown as its code`, or `N of its
  blocks could not be drawn, and are shown as their code`, on the first page, after the pictures
  left out. It doesn't say why: merman's message for each would be a line a block.
- **The info string's first word** is split at whitespace (`language_of`), so ` ```mermaid,x ` is
  no `mermaid`. A language in braces (` ```{mermaid} `, as R Markdown writes) isn't one either.
- **The picture's paragraph** is `<p><img src="KEY" alt=""></p>`: an empty `alt`, so a search
  finds nothing of a drawn block, and a block that somehow came back without a picture would show
  nothing rather than its key.
- **`--help`** lists a format's fences after its extensions, as the block is written:
  ``Mermaid (.mmd .mermaid, ```mermaid)``.
- **A test backend, `source::tests::Sketches`**: it lists fences, draws pictures or not, and opens
  a block of two numbers to a page that size. `picture.rs`'s and `wordcraft.rs`'s unit tests draw
  blocks through it (`Pictures::among`), so neither names Mermaid, and the 1,025 blocks of the
  limit's test cost no worker. Through the viewer, `tests/it/mermaid.rs` uses the real one.

**Surprises:**

- **Markdown through a pipe** can only be a pipe named as Markdown, since Markdown is known by its
  name alone: `/dev/stdin` is refused before any block matters. The test makes a fifo, `piped.md`.
  HTML says what it is by how it starts, so task 2's blocks do come through `/dev/stdin`.
- **1,025 blocks of one diagram open in 1.3 s** in the debug build, since Mermaid's cache
  answers all but the first; so the limit is tested through the viewer too.
- **What the costliest blocks cost** (ADR-0032's Evidence): a 2.6 MB Markdown file of eight
  diagrams of 15,000 nodes takes 67 s to print its first page, and one of 1,024 small diagrams
  10 s. A document's marks and bytes bound how many it holds, so it is bounded, at minutes; a
  document referring to as many `.mmd` files by path cost the same before this spec, but now one
  file does it alone. Raised with the owner, and put in the backlog: a bound on what a document's
  blocks cost in all.

**Each test seen failing**, the file copied aside, changed, and copied back (`cmp` the same):

| Test | Reverted | How it failed |
|---|---|---|
| `a_fence_is_listed_once_and_only_by_a_backend_that_draws_pictures` | Markdown's format given `.fenced(&["mermaid"])` | `wordcraft lists mermaid, and draws no pictures`, `mermaid is listed twice` |
| `a_fences_language_finds_the_backend_that_draws_it` | `fenced` without its `draws_pictures` filter | `held` found |
| `a_fenced_block_is_a_picture_of_its_text_by_the_backend_that_lists_its_language` | the same; and `measured` asking for a directory before the blocks' keys | `draws("held")`; no stand-in for its key |
| `a_fenced_blocks_key_is_nothing_a_document_can_write`, `a_mermaid_blocks_key_written_in_the_document_is_no_picture` | `afresh` returning 0 | `fenced:0000000000000000:0` gave the block's stand-in; the page showed the diagram twice |
| `fenced_blocks_count_among_the_pictures_a_document_has`, `a_mermaid_block_past_the_most_pictures_a_document_has_stays_code` | `within` always true | the 1,024th key given; no warning |
| `a_fenced_block_a_backend_draws_is_written_as_its_picture` | the fence's arm in `markdown_as_html` never taken | no `<img>` written |
| `a_mermaid_block_in_markdown_is_drawn_as_its_diagram` | Mermaid's format without `.fenced(&["mermaid"])` | no red node on the page |
| `a_mermaid_block_that_cant_be_drawn_shows_as_its_code_and_the_first_page_says_so` | the first page's note not written | nothing said |
| `a_mermaid_block_is_drawn_through_a_pipe_and_in_an_archive` | `measured` asking for a directory first | no diagram through the pipe |
| `help_lists_the_options_each_backend_takes_…` (`listed`), `help_and_version` | `listed` without the fences; Mermaid's format without its fence | `Diagram (.dg .diagram)`; no such row |
| `a_documents_mermaid_block_is_turned_into_svg_once_whatever_it_is_laid_out_at` | Mermaid's cache never answering | a worker at `40]` |

**Verify:** the spec's `cargo test -j 2 --lib fence` is `cargo test -j 2 --bin clv fence` here (clv
is a binary, as `project.md` says). The manual check, in kitty, is the owner's: the box stays open
until it is confirmed.

## Task 2: HTML's blocks

**The open question, settled by trying** (🟡 "the HTML scanner on real output"): what writes HTML
from Markdown was asked for a ` ```mermaid ` block, and what each wrote is in ADR-0032's Evidence.
Only pulldown-cmark's, and the CommonMark renderers', is the `language-x` class the requirement
names. So a `<pre>` says its language in four ways, all of them said and none guessed:

- a class `language-x`, of the `<pre>` or of the `<code>` it starts with (the requirement's);
- a class `x` alone, there too: pandoc (`<pre class="sourceCode mermaid">`), and Mermaid's own
  `<pre class="mermaid">`;
- the `<pre>`'s `lang="x"`: GitHub's pages, which the task's manual check reads. Without it that
  check couldn't pass: a README's page has `<pre lang="mermaid" aria-label="Raw mermaid code">`
  and no class at all;
- a class `highlight-source-x` of the `<div>` right around the `<pre>`: GitHub's Markdown API.

**This goes past the requirement as written** (a `language-x` class only): raised with the owner.
Any of the last three comes out by taking its line out of `blocks_drawn`.

**Decided here:**

- **The scanner cuts HTML as wordcraft's lexer does**, read at its pin
  (`crates/formats/src/html/lexer.rs`, `import.rs`): a tag's name, its attributes with their
  quotes (an unquoted value takes a `/`, an unclosed quote runs to the end of the text), comments
  (`<!-->` and `<!--->` are whole ones), `<!…>` and `<?…>`, an end tag to its `>`. What it skips
  whole is the importer's list, `script`, `style`, `noscript`, `template` and `textarea`: two
  more than the spec named. A `<pre>` to one is a `<pre>` to the other, so a picture is never
  written where the importer would have seen text.
- **A `<pre>` with another in it stays code**, as one never closed does: `pre_text` stops at the
  inner one, which then has its own turn. That also keeps the scan linear: a `<pre>`'s text is
  read once more only when it says a drawn language, and never past the next `<pre>`.
- **A `<br>` in a block is a line's end**; a class or `lang` is read as written, its entities not
  decoded (`class="language-&#109;ermaid"` is no Mermaid).
- **Markdown's HTML doesn't go through the scanner:** its blocks come whole from pulldown-cmark,
  and a `<pre>` written as HTML inside Markdown stays code.
- **The picture** replaces the `<pre>` alone: a `<div>` around it stays (GitHub's, pandoc's).

**Tried on the real thing:** GitHub's page of Mermaid's README, fetched with `curl` (405 KB, ten
`mermaid` blocks), shown by `clv … --protocol kitty`: 29 pages in 0.9 s, no block left as code,
the pie and the gantt chart looked at as PNGs. GitHub's "Loading" under each diagram is its own
placeholder for the frame it draws in, which is text to clv.

**Each test seen failing:**

| Test | Reverted | How it failed |
|---|---|---|
| `a_mermaid_block_in_html_is_drawn_as_its_diagram`, `a_mermaid_block_in_html_is_drawn_through_a_pipe_and_in_an_epub` | `html` importing the text as it came | no flow on the page, nor in the tile |
| `the_scanner_reads_a_blocks_text_without_its_tags` | a run of text taken without `unescape` | `a --&gt; b &amp; c …` |
| `the_scanner_cuts_html_as_the_importer_does`, `the_scanner_never_panics_wherever_html_is_cut` | a start tag's length not held to the text's | a piece ending past the text; shrunk to `40 20`, `<pre>`, `<pre class="language-sketch">`, cut |
| `the_scanner_leaves_every_other_pre_as_it_is` | `SKIPPED` not skipped; a `<div>`'s language kept past an end tag | a `<pre>` inside `<script>` drawn |
| `the_scanner_draws_a_pre_that_says_its_language` | the `<pre>`'s `lang` not read | GitHub's `<pre lang>` left as it was |

The reverted property test left a `proptest-regressions/` file, which the repository doesn't keep:
removed.

**Files beyond the plan's:** `CONTRIBUTING.md` (the tests), `docs/architecture.md`, ADR-0032,
`CHANGELOG.md`.

## Task 3: The `fences` setting

As planned: `FENCES` beside `PAPER` in `src/source/wordcraft.rs`, `drawn` or `code`,
`Keys::Cycle('s')`, `--fences drawn|code`, `Effect::Relayout`. The viewer needed nothing: it cycles
a setting's named values and lays a document out again for any relayout setting, and
`a_settings_keys_are_its_own_in_every_backend` and `the_viewers_keys_are_those_listed` pass with
`s` as they are.

**Decided here:**

- **`At::fences`**, with the width, the font size and the margins, since it is what a layout is
  made at. As code, `laid_out` reads the document with `Pictures::blocks_as_code`, pictures that
  list no backend for a block: Markdown's events and HTML's scanner then find no language to draw,
  with nothing to tell either. No block is counted as left undrawn, so nothing is said of a broken
  one.
- **What the status line says:** `blocks as code` on going there and, as for any relayout setting
  off its default, on the right while it lasts; `blocks drawn` on going back.
- **A search across `s` starts over** (the requirement's "a drawn block's text isn't searched;
  shown as code, it is"). A search kept across a relayout keeps what it read when the text's
  fingerprint and its paragraphs' count are the same, and the fingerprint was the bytes' alone. A
  block of one line is one paragraph, as its picture is, so the count didn't tell them apart and
  the search went on saying `no match` over code that had the word. `fingerprint_of` now hashes
  how many blocks are shown as pictures too (`Promised::fenced`). With none drawn either way, a
  document's text is the same and its search is kept.
- **In a list** `s` says what the width keys say there, the viewer's `open a document to reflow
  it`, and a document gone into takes it. The words fit a width better than blocks; they are the
  viewer's, for every relayout setting, and are left as they are.

**Files beyond the plan's:** `src/source/picture.rs` (`blocks_as_code`, `Promised::fenced`),
`tests/it/mermaid.rs` (`--fences code` printed), `docs/architecture.md`, ADR-0032, `CHANGELOG.md`,
`CONTRIBUTING.md`.

**Each test seen failing:**

| Test | Reverted | How it failed |
|---|---|---|
| `s_shows_a_documents_blocks_as_code_and_draws_them_again`, `s_on_a_pdf_…_and_a_list_says_to_open_one` (its list), `a_mermaid_block_is_shown_as_code_with_fences_code`, `fenced_blocks_shown_as_code_are_none_of_the_pictures` | `blocks_as_code` giving the pictures back as they were | the block still drawn after `s`, and with `--fences code` |
| `markdown_and_html_say_their_width_and_room_and_lay_out_again_at_another` (its `fences` lines) | `laid_at` not saying `fences` | `None`, not `drawn` |
| `s_on_a_pdf_…` (its PDF) | `FENCES` not among the backend's settings | `s` said nothing |
| `a_search_reads_a_block_shown_as_code_and_not_one_drawn` | `fingerprint_of` hashing the bytes alone | `no match` kept over the one-line block's code |

The last first passed with the fix reverted: a block of several lines changes the paragraphs' count,
which restarts a search by itself. The one-line block was added for it.

**Verify:** the pager's tests ran with their 60 s of patience cut to 8 s while they were seen
failing, and put back. The manual check, in kitty and inside a private tmux, is the owner's.

## Task 4: Blocks on the document's paper

**An assumption of the spec was false.** It had `d` a redraw with no relayout because "a theme
doesn't change a diagram's size, which the test checks". The test said otherwise: merman's dark
theme lays a flowchart out 1% larger, an ER diagram 8% wider and a class diagram 10% wider and 3%
taller; sequence diagrams, pies and gantt charts are the same (ADR-0032's Evidence). Raised with
the owner the moment it showed, with three ways on: fit the dark drawing into the light one's room,
lay the document out again on `d`, or stretch it. The first is built, as the one that keeps every
decision the spec made (a redraw, Mermaid's own dark theme, on the paper's colour) and gives up
only the assumption; the spec's requirement and task 4's Verify are revised to say so, in its
Changelog, **for the owner to confirm**.

**Decided here:**

- **`Request::on_dark`**, a flag, beside `Request::dark`, the paper's colour: how a picture is
  told it is on dark paper. It names no setting. Asked of a `Source::renderer` alone, which is how
  `picture.rs` draws; a pool is never asked so, and Mermaid's hands its pool to the light drawing.
- **Mermaid opens to a source of its own, `Diagram`**: the light SVG's source, the diagram's text
  and its path. Everything is the light drawing's but a render with `on_dark`, which draws the
  dark SVG, asked of the worker as `DRAW_DARK` when the first such request comes and kept in the
  cache under its `Theme`. A `.mmd` shown by itself, in a list, or by path goes through the same
  source and is never asked for dark.
- **The dark drawing in the light one's room** (`in_the_room_of`, `centred`): rendered at the
  request's scale times what fits the light drawing's size, never more than 1, then put in the
  middle of a transparent frame the light one's size. No distortion, nothing on the page moves,
  and a dark class diagram is a tenth smaller than its light twin. The contract says it: a dark
  form comes "at the size of its light form".
- **A diagram's own theme stays its own** on dark paper: `dark` is merman's site theme, which a
  diagram's `%%{init}%%` overrides, as on a web page whose theme is dark. A diagram themed for
  white is then dark ink on dark paper. Forcing the dark theme would throw away the colours a
  diagram chose. Also for the owner to confirm.
- **What a document holds of a block is for one paper** (`Held::for_dark`): the other paper's
  takes its place, as a page's takes a tile's, so the 256 MiB hold one PNG a block. A failure is
  kept by paper, so a block that fails in the dark theme is still tried on light paper.
- **A picture drawn for dark paper is never put on white:** `thin_and_near_black` is for ink made
  for white paper, and Mermaid's dark nodes (`#1f2020`) on nothing would have passed for it.
- **A page checks its pictures under the read lock** (`Promised::settled`, at most `MAX_DRAWS`
  rounds). With a picture drawn for one paper at a time, two pages rendering side by side for
  different papers (a stale request after `d`) could each take the other's; and it also closes
  the older gap where a picture is let go of between a page holding it and rendering.

**A surprise in the tests:** in the unit tests a worker is a thread of the test process, and all of
them share the six a viewer may have at once, the SVG backend's labels' among them. Two new tests
that each kept a Mermaid worker idle for its two seconds made two SVG label tests fail (`too many`,
so their labels were left out). The tests that draw several diagrams now take turns (`TURN`) and let
go of their worker at the end, the older `what_the_worker_answers_…` included, and the new test
puts what its own worker drew into the backend's cache before it opens a diagram, so that `open`
starts none. The unit suite passed three runs in a row after.

**Files beyond the plan's:** `src/view/mod.rs` and `src/source/wordcraft.rs`'s test (a `Request`
written whole), `docs/architecture.md`, `docs/backlog.md` (the plan for `d` on drawings: blocks
already do), `CHANGELOG.md`, `CONTRIBUTING.md`.

**Each test seen failing:**

| Test | Reverted | How it failed |
|---|---|---|
| `a_fenced_block_is_drawn_again_for_the_other_paper`, `a_mermaid_block_follows_its_documents_paper`, `d_draws_a_documents_block_again_for_its_paper_…` | `Promise::draw` never setting `on_dark` | red where blue was due; 44,811 white pixels on the dark page; the pager's page not dark |
| `a_picture_by_path_is_the_same_on_either_paper`, `a_mermaid_picture_by_path_is_unchanged_on_dark_paper` | every promise following its paper | not held for light paper; no white behind the picture |
| `the_cache_keeps_each_themes_svg_apart` | `Cache::get` not looking at the theme | the light SVG for dark |
| `a_diagram_for_dark_paper_is_in_the_dark_theme_on_nothing_in_the_light_ones_room` | the site's `theme` not set; then `in_the_room_of` left out | no `#1f2020` in the flowchart; frames of 1221 × 366 and 1231 × 370 |
| `a_frame_is_centred_in_a_larger_one` | `centred` putting the frame at the left | the pixels at the left |
| `d_draws_…_and_lays_the_diagram_out_once_a_theme` | the cache not answering for the dark theme | a worker on the second turn to dark |

The by-path test first passed with its fix reverted, because the cache's revert was in the same
build and gave it the light SVG; alone, it failed.

**Verify:** the manual check, in kitty with a dark background, is the owner's.

## What is left

- Every task's code, tests and docs are committed, and each task's box is open: all four end
  with a manual check in kitty (the third in tmux too), which only the owner can do. The spec
  stays `Implementing` until they are confirmed.
- For the owner to confirm: the four ways HTML says a block's language (task 2), the dark drawing
  fitted into the light one's room, and a diagram's own theme kept on dark paper (task 4).
- In the backlog: a bound on what a document's blocks cost in all.
- Noticed, left alone: in a list, `s` says `open a document to reflow it`, the viewer's words for
  every relayout setting, which fit a width better than blocks.
