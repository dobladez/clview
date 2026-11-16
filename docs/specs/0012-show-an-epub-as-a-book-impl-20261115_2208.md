# Spec-0012: implementation notes (2026-11-15)

Notes made while implementing [Spec-0012](0012-show-an-epub-as-a-book.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 1: pictures beside a held file

**Changed from the plan**

- **`Bounded` gives no prefix**, so "measured through its `Bounded` (64 KiB)" can't measure a
  larger picture. The contract got a third reader, `Start` (a held file's first bytes alone,
  `Input::starting`, `Input::start`), which zip and tar give. ADR-0037 says so.
- **The `image` crate reads a JPEG whole before parsing it** (528 of the 710 JPEGs of 22 real
  books are past 64 KiB). So a JPEG is given its first 64 KiB as if they were the file, and read
  whole when that fails; every other format through `image::First`, which notes a read past them.
  Measured with `docs/adr/0037-measure/`: every header of 1,999 real pictures is within 26 KiB,
  which answers the spec's open question on 64 KiB for books' pictures. A camera's photograph was
  not measured, only written by a test.
- **The held input is a variant with named fields** (`Rest::Held { again, bounded, start, beside }`),
  and `Beside` a tuple behind an `Arc`, not a struct: the no-backend and `pdf,svg` builds, never
  built locally, then need no dead-code expectation on fields, and `Input` stays under clippy's 128
  bytes for an `Err`. `Input::held` got an expectation: `later.rs` no longer calls it. Those two
  builds are still to be run by CI (`gh workflow run CI`).
- **Hidden names** (`.github/demo.png`) aren't found in an archive: its `Tree` never held them.
  In usage's Archives and the ADR's Accepted.
- **Tar's head** for a file past the kept heads is now its first 1,024 bytes (`Shared::first`),
  not the file read whole and cut: a file past 512 MiB is refused when read, not when opened.
- **The ADR is 0037**: Spec-0013's branch had taken 0036.
- Files beyond the task's list: `docs/adr/0037-measure/` (the measure), ADR-0013 and the ADR
  index (its status), `docs/backlog.md`, `CHANGELOG.md`.

**Tests, each seen failing** with its fix reverted (the file copied aside, changed, the test run,
copied back and compared):

| Reverted | Failed |
|---|---|
| `from_the_top` no longer refusing a `..` above the top | `picture::tests::a_held_documents_picture_is_a_path_from_its_holders_top`, `…a_held_documents_pictures_are_read_from_its_holder` |
| `Image::held` never asking for the first bytes | `image::tests::a_held_picture_is_measured_from_its_first_bytes` |
| a JPEG read as the other formats are | the same |
| no whole read once the first bytes are passed | `image::tests::a_held_picture_whose_header_comes_later_is_read_whole` |
| zip's `Directory::open` without `held_near` | `zip::tests::a_file_in_an_archive_gives_its_first_bytes_and_what_is_beside_it` |
| tar's the same | `tar::tests::a_file_in_an_archive_gives_its_first_bytes_and_what_is_beside_it` |
| `Reopen::input` dropping what is beside the file | `later::tests::a_file_is_closed_until_wanted_unless_it_couldnt_be_opened_again` |
| `Document::beside` giving nowhere for a held document | `zip::a_documents_pictures_are_drawn_from_its_zip`, `tar::a_documents_pictures_are_drawn_from_its_tar` |
| zip's held file without its `Start` | `zip::a_documents_pictures_are_measured_without_being_read_whole` |

The tests' own first mistakes, for whoever reads them: a GIF of one frame of noise is 26 KB (its
256 colours), so the test writes four; a name seen twice is left out once (`seen`), so three
names above the top count two; a document opened from a list says `page 1/1`, not its name's
count; and a zip whose picture fails its checksum prints that picture's tile as an error, so the
print exits unsuccessfully while the document's tile is drawn.

An existing test changed with the behaviour: `wordcraft::what_has_no_directory_has_no_pictures`
held a document in a zip to saying nothing of a picture it couldn't reach; it now says one is left
out, as a document on disk does, and still draws nothing from beside the archive on disk.

Gates on 2026-11-16: `cargo fmt --check`, `cargo clippy --all-targets -j 2 -- -D warnings`,
`cargo test -j 2` (565 unit, 13 docs, 279 integration), all passing.

## Task 2: an epub's chapters as a book, its package read in a worker

**The spec's open questions, answered**

- **rbook on a zip of the package alone**: yes. With `strict(false)` and `skip_toc(true)` it gave
  the spine, the manifest and the cover of 22 real books from a zip holding only the container
  and the OPF. The worker is never sent the epub.
- **The worker's caps**: 2 s of CPU and 256 MiB of heap. A real package takes under a quarter of
  a millisecond and 175 KiB; the costliest 4 MiB package found (599,000 empty `<meta/>`) takes
  189 ms and 564 MiB, and is refused by the cap, on purpose (ADR-0038's decision 4).
- **A page break on a `<div>`**: it reaches the first paragraph in it, so each chapter is a
  `<div style="page-break-before: always">`. Not when the `<div>`'s first content is a table: the
  join writes a paragraph of one zero-width space, a point high, before such a table.

**Changed from the plan**

- **The worker writes the container itself**, naming the package the backend found, and is sent
  the package's path and bytes, not the book's `container.xml`: rbook parses one hostile file
  less, and its reading of the container can't disagree with the backend's.
- **`application/fontè` doesn't refuse the book.** rbook panics on it only when asked whether an
  item is a font, which the worker never asks: the book shows. The spec expected it refused.
- **A chain of `refines` doesn't recurse past the stack** at rbook 0.7.13: it is followed 255
  deep and then refused as a cycle (in a debug build, a panic on an overflowing addition, which
  the worker catches). Either way the book is refused; the test takes both wordings.
- **`skip_toc` skips the EPUB 2 guide too**, so the reading order's answer can't say which
  chapter is the book's own contents page: that comes with the table of contents, in task 5.
- **The lock moved further than the spec said**: zip 9.1.3 to 9.5.1 brings `lzma-rust2` 0.25.0
  beside the tar backend's 0.23, and `constant_time_eq`. Cargo would have taken zip 9.6.0; the
  lock is held at 9.5.1, as the spec names.
- **A link to a file is marked when the book is assembled** (`clv-in-the-book/`,
  `clv-not-in-the-book/`), and `Book` answers by the mark: answering by looking in the zip would
  read the book from `Links::try_recv`, which never blocks.
- **Links between chapters wait for task 4**: until then a link to another chapter says `a file
  in the book, which clv doesn't open alone`, and `#s3` in a chapter says the place is missing,
  its id having its chapter's number before it.
- **`Kind`, the cover, fallbacks and protected files** aren't in the worker's answer yet: tasks 3
  and 5 add to its format, which the viewer and the worker, one binary, always agree on.
- **`markup.rs`'s attributes are an iterator** over the tag's text (`StartTag::attributes`), not
  a list kept in every tag: the documents' passes go through half a million tags and want three
  attributes of each.
- Files beyond the task's list: `CLAUDE.md` (its Layout names the two new modules),
  `docs/adr/0038-measure/`.

**Tests, each seen failing** with its fix reverted:

| Reverted | Failed |
|---|---|
| a chapter's path as the manifest writes it, not from the book's top | `epub::tests::the_worker_answers_a_packages_reading_order` |
| a package with no chapters accepted | `epub::tests::a_book_whose_package_cant_be_read_is_refused_with_why`, `epub::a_book_that_cant_be_read_is_refused_not_shown_as_a_zip` |
| an answer of more chapters than a book has accepted | `epub::tests::a_worker_that_dies_or_answers_too_much_costs_the_book` |
| ids left as written | `epub::tests::chapters_are_assembled_in_order_each_on_a_page` |
| any kind of file read as a chapter | `epub::tests::a_chapter_that_cant_be_read_is_left_out_and_counted` |
| no bounds on the book assembled | `epub::tests::a_book_is_cut_at_the_chapter_that_would_pass_its_bounds` |
| a picture's path as the chapter writes it | `epub::tests::a_chapters_references_are_paths_from_the_books_top` |
| a link to a file left one | `epub::tests::a_books_link_to_a_file_is_a_message_never_a_file`, `view_pager::epub_a_link_to_a_file_is_said_and_no_file_beside_the_book_is_opened` |
| a `.epub` that doesn't say so no book | `epub::tests::a_book_is_a_zip_that_says_so_or_is_named_so` |
| the container's path not unescaped | `epub::tests::a_container_names_its_package` |
| a field cut short read as far as it goes | `epub::tests::fields_are_read_as_they_were_written_and_no_further` |
| a chapter that isn't UTF-8 unwrapped | `epub::tests::any_chapter_is_assembled_or_left_out` |
| a field past the answer's end sliced | `epub::tests::any_answer_is_a_plan_or_none` |
| no page break on a chapter's `<div>` | `epub::an_epub_is_a_book_its_chapters_in_order_each_on_a_new_page` |
| nothing found beside the book's document | `epub::an_epub_is_read_through_a_pipe_and_from_an_archive` |
| the worker without its heap cap | `epub::a_hostile_package_costs_the_book_and_never_the_viewer` |
| the chapters left out not said | `epub::a_chapter_missing_is_left_out_and_said` |
| the options not handed on | `epub::a_book_is_laid_out_to_a_width_and_drawn_on_dark_paper` |
| rbook not credited | `epub::help_lists_books_and_credits_rbook` |
| rbook named in `zipped.rs` | `epub::nothing_but_this_backend_names_rbook` |
| a chapter's classes dropped | `mermaid::a_mermaid_block_in_html_is_drawn_through_a_pipe_and_in_an_epub` |
| a book without its text | `view_pager::epub_a_search_finds_a_word_in_a_later_chapter_and_the_worker_is_gone` |
| the worker left alive after its answer | the same |

**Existing tests changed with the behaviour**, each because a sentence now names books:
`view::a_file_no_backend_opens_is_refused_and_a_pdf_is_known_by_its_header` (what clv shows),
`view::help_and_version` (a heading for the books' options, and their format), and four of the
pager's that quote which formats have a setting (`only Markdown, HTML and EPUB reflow`, `… draw
their blocks`, `only Mermaid, Markdown, HTML and EPUB go dark`, and `?`'s lines). Three of those
ran in 80 columns, where the status line now shortens the sentence in its middle (`only Markdown,
HTML …PUB draw their blocks`): their windows are 110 columns now, to read it whole.

Gates on 2026-11-16: `cargo fmt --check`, `cargo clippy --all-targets -j 2 -- -D warnings`,
`cargo test -j 2` (580 unit, 13 docs, 289 integration), all passing; `cargo deny check
advisories` too, with rbook and the newer zip in the lock.

**Discoveries, raised**

- The last nightly CI run on `main` (2026-11-15, schedule) failed in under two minutes, before
  this branch: told to the lead.
- **A setting's sentence no longer fits 80 columns** with one more format in it, and is
  shortened in its middle there. The spec asked for the format to list the settings; whether the
  status line should give such a sentence more room, or say it shorter, is the owner's: told to
  the lead.
- `view_pager::keys_work_while_the_document_is_laid_out_again_and_the_new_pages_show_with_no_key`
  failed once in a full run, under load, and passed alone three times and in the runs after: it
  times a layout against a key.
- Two versions of `lzma-rust2` are built now. The tar backend's could move to 0.25 with zip's:
  told to the lead, not done here.

## Task 3: covers, fallbacks and DRM

**The spec's open question, measured**

- **The cost of a book's tile**: the 22 real books open to their first page in 0.11 to 1.13 s each
  (ADR-0038's table), and all 22 printed as a list take 10 s and 655 MiB, half a second a book,
  each laid out whole for its tile. In the pager they open four at a time, as their tiles are
  wanted. Whether that is too slow for a shelf is the owner's to judge in kitty (the task's manual
  check); a tile from the cover alone is in the backlog's More documents.

**Changed from the plan**

- **A fixed-layout book says so on its first page** (`a fixed-layout book: its pages are laid out
  as its text flows, not as they were made`), besides usage's known issue: the worker answers the
  layout, and a reader of a comic laid out as text should be told why.
- **The cover is found twice by its path**: "the first chapter doesn't show it already" is whether
  that chapter's body, as written into the book, refers to the cover's path from the book's top.
  A chapter that shows the same picture under another file's name gets the cover page too.
- **A protected picture** is refused by the book's `Near`, so its chapter leaves it out and
  counts it, as a picture that is missing: a document has no grey box for a picture it couldn't
  measure.
- **16,384 protected files** at most, a limit the spec didn't name: past it the book is refused.
  A list of 1 MiB can't name many more.
- **The table before a chapter** (task 2) and **`<x/>` as `<x></x>`** are in `Written`; the
  void elements are HTML's fourteen.

**Tests, each seen failing** with its fix reverted:

| Reverted | Failed |
|---|---|
| no cover page | `epub::tests::the_cover_comes_first_unless_the_first_chapter_shows_it`, `epub::a_books_cover_is_its_first_page_and_its_tile_in_a_list` |
| the cover shown though the first chapter shows it | `epub::tests::the_cover_comes_first_unless_the_first_chapter_shows_it` |
| an `<svg>` that wraps a picture left a drawing | `epub::tests::an_svg_that_only_wraps_a_picture_is_that_picture`, `epub::a_books_cover_is_its_first_page_and_its_tile_in_a_list` |
| an element written short left open | `epub::tests::an_element_written_short_is_read_as_empty` |
| no fallback followed | `epub::tests::a_spines_item_is_shown_as_its_fallback_and_a_picture_as_its_page` |
| a picture in the spine no page | the same |
| a font's obfuscation taken for protection | `epub::tests::a_file_encrypted_by_anything_but_a_fonts_obfuscation_is_protected` |
| any number of protected files | the same |
| the list of protected files read past 1 MiB | the same |
| a protected chapter read | `epub::tests::a_protected_chapter_is_left_out_and_a_protected_picture_not_read`, `epub::a_protected_chapter_is_left_out_and_a_book_all_protected_refused` |
| a protected picture read | `epub::tests::a_protected_chapter_is_left_out_and_a_protected_picture_not_read` |
| a book all protected shown empty | `epub::a_protected_chapter_is_left_out_and_a_book_all_protected_refused` |
| fixed layout not said | both of those |

**Looked at, not in kitty**: *Alice's Adventures in Wonderland* (Gutenberg's 11, EPUB 3) printed
with `--protocol kitty` and its pages read back as pictures: its cover is the first page, shown
once, from the drawing that wraps it; Standard Ebooks' *Alice* has its cover first, which its
title page doesn't show, and is its tile in a list of the 22. The task's manual check, in kitty,
is the owner's, and its box stays open until it is confirmed.

Gates on 2026-11-16: `cargo fmt --check`, `cargo clippy --all-targets -j 2 -- -D warnings`,
`cargo test -j 2` (586 unit, 13 docs, 291 integration), all passing.

A trap met on the way, for whoever reverts a fix to see a test fail: a file copied back with its
old time (`cp -p`, `shutil.copy2`) is older than the binary built from the revert, and cargo then
builds nothing, so the next run tests the reverted code. Copy it back with a new time.

## Task 4: a book's links

**Changed from the plan**

- **A link to an id too long to be a place says `no such place in this book`**, through a third
  mark (`clv-no-such-place/`) that `Book` answers, as it answers a link to a file: left as a
  `#name` of more than 256 bytes, the documents' backend would say its name is too long to follow,
  not that the place is missing.
- **A link to a chapter the reading order has and the book doesn't** (missing, protected, cut) is
  still written as that chapter's place: the document then says it has no such place, as it does
  of any `#name` it hasn't.
- **A link's `?query` is dropped** with its path, when the path is a chapter's.
- The spec's test names are kept: `epub::tests::links_…` and `view_pager::epub_links_…`.

**Tests, each seen failing** with its fix reverted:

| Reverted | Failed |
|---|---|
| a link to a chapter left a link to a file | `epub::tests::links_between_chapters_are_places_in_the_book`, `…links_lead_to_ids_made_the_books_own`, `view_pager::epub_links_between_chapters_are_followed_and_u_comes_back` |
| a `#name` alone not its own chapter's | all three of `epub::tests::links_…` |
| a name of any length a place | `epub::tests::links_to_an_id_too_long_once_made_the_books_own_say_so` |
| a place too long answered as a missing file | the same |

Gates on 2026-11-16: `cargo fmt --check`, `cargo clippy --all-targets -j 2 -- -D warnings`,
`cargo test -j 2` (589 unit, 13 docs, 292 integration), all passing.

The task's manual check (in kitty, a Gutenberg epub's contents page, `f`, a label: its chapter)
is the owner's, and its box stays open until it is confirmed.
