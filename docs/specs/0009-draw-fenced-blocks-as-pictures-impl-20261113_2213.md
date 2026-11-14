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
