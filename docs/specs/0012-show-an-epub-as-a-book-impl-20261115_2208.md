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
| a link to a chapter left a link to a file | `epub::tests::links_between_chapters_are_places_in_the_book`, `…links_lead_to_ids_made_the_books_own`, `view_pager::epub_links_between_chapters_are_followed_and_backspace_comes_back` |
| a `#name` alone not its own chapter's | all three of `epub::tests::links_…` |
| a name of any length a place | `epub::tests::links_to_an_id_too_long_once_made_the_books_own_say_so` |
| a place too long answered as a missing file | the same |

Gates on 2026-11-16: `cargo fmt --check`, `cargo clippy --all-targets -j 2 -- -D warnings`,
`cargo test -j 2` (589 unit, 13 docs, 292 integration), all passing.

The task's manual check (in kitty, a Gutenberg epub's contents page, `f`, a label: its chapter)
is the owner's, and its box stays open until it is confirmed.

## Task 5: a book's Contents page

**The spec's open question, looked at on real books**

- **A book's own contents page, told**: of the 22 books, none has its nav document in its spine.
  7 of the 10 Gutenberg EPUB 2 editions name a contents chapter in their guide, and get no
  Contents page. The 10 Gutenberg EPUB 3 editions have no landmarks at all, so those of them
  whose text has its own contents get clv's page besides it: two. Standard Ebooks' two have none
  of their own, and get the page, nested as their books are. The rule is the spec's, kept as
  written; whether two contents in Gutenberg's EPUB 3 books is worth a guess at a chapter headed
  `Contents` is the owner's: told to the lead.

**Changed from the plan**

- **An NCX nested 100,000 deep doesn't kill the worker: it is never given to it**, being past the
  1 MiB read of a table of contents (8 MiB of it). And nothing within 1 MiB does either, at rbook
  0.7.13: a nav nested 20,000 deep is read in 10 ms and 20 MiB, on a stack of 8 MiB (ADR-0038).
  So the integration test has the book whole and saying so for the first, and a Contents page
  for a nav 20,000 deep; a worker that dies on a table of contents is a fake one, in the unit
  tests.
- **The NCX is found by its media type**, not by the spine's `toc` attribute, which rbook doesn't
  give once the table of contents is skipped. A book whose spine doesn't name its NCX is then
  asked for it, and rbook, which goes by the spine, says it has none: no Contents page, and the
  first page says its table of contents couldn't be read.
- **An entry is a link where its chapter can be shown** (in the book, not protected, XHTML or a
  picture), decided before the chapters are written: one whose chapter is then left out for
  another reason (unreadable, past the bounds) is still a link, to a place the document then says
  it hasn't.
- **A list never opens more than a level deeper than the entry before it**, whatever depths a
  worker answers: `<ul>` in `<ul>` is no list.
- **An entry without a label** shows as `…`.
- **Two workers a book** that has a table of contents to read, one after the other.

**Tests, each seen failing** with its fix reverted:

| Reverted | Failed |
|---|---|
| no table of contents asked for | `epub::tests::contents_are_read_from_a_nav_and_from_an_ncx`, `…contents_are_kept_within_their_limits`, `…contents_are_not_made_twice`, `…contents_that_cant_be_read_leave_the_book_whole`, `epub::a_books_contents_page_comes_after_its_cover_and_isnt_made_twice`, `view_pager::epub_contents_page_leads_to_its_chapters_and_backspace_comes_back` |
| a page though the nav document is a chapter | `epub::tests::contents_are_not_made_twice`, `epub::a_books_contents_page_comes_after_its_cover_and_isnt_made_twice` |
| a page though the landmarks, or the guide, name a chapter | `epub::tests::contents_are_not_made_twice` |
| the worker keeping every entry | `epub::tests::contents_are_kept_within_their_limits` |
| a depth kept as the worker answers it | the same |
| a label written as markup | `epub::tests::contents_are_a_page_of_links_after_the_cover` |
| an entry linked whatever it leads to | the same |
| a table of contents read whatever its size; a protected one read | `epub::tests::contents_that_cant_be_read_leave_the_book_whole` |
| a table of contents unread not said | the same, and `epub::a_books_contents_page_comes_after_its_cover_and_isnt_made_twice` |
| the Contents page before the cover | `epub::tests::contents_are_a_page_of_links_after_the_cover` |
| lists as deep as the entries say | the same |

The size test first held a file of blank space past 1 MiB, which no worker could read either: with
the limit taken away it still passed. It now holds a table of contents that reads, a byte past the
limit, and one exactly at it, which is listed.

Gates on 2026-11-16: `cargo fmt --check`, `cargo clippy --all-targets -j 2 -- -D warnings`,
`cargo test -j 2` (594 unit, 13 docs, 294 integration), all passing; `cargo deny check
advisories` too.

**Looked at, not in kitty**: Standard Ebooks' *Alice* and *Frankenstein* printed with `--protocol
kitty`, their pages read back as pictures: the cover, then `Contents`, a list nested two deep, its
entries in the links' blue. The task's manual check, in kitty, is the owner's, and its box stays
open until it is confirmed.

## The Done checklist

- **The gates** pass at the last commit: `cargo fmt --check`, `cargo clippy --all-targets -j 2 --
  -D warnings`, `cargo test -j 2` (594 unit, 13 docs, 294 integration). The two other feature sets
  (no backends; `pdf,svg`) are CI's, never built here: `src/source.rs` got dead-code expectations
  for them (`Input::held`, `starting`, `held_near`, `near`, `start`), `src/source/zipped.rs` one
  (`index_of`) and `src/source/markup.rs` one (`attributes`), each reasoned out, none compiled.
  Run `gh workflow run CI` once this is pushed.
- **Each test was seen failing**, in the tables above; the lexer's two in `markup.rs` too
  (`a_start_tags_attributes_come_as_written_with_their_places`,
  `any_text_is_cut_into_pieces_and_attributes_within_it`), with an attribute's place cut to its
  name, its value a byte short, and its place past its tag.
- **The docs** were read against what shipped: `CHANGELOG.md` (two lines under Unreleased),
  usage (Books, Archives, Documents, Which files it shows, Limits, Known issues, the options' and
  keys' tables), architecture (Sources, Backends' table, Files, How each opens), ADR-0037 and
  ADR-0038 with their measures and the ADR index, ADR-0013's status, `CONTRIBUTING.md` (features,
  pins, the rule on rbook, Tests), `CLAUDE.md`'s Layout, README (what it shows; Plans without its
  epub half), the backlog.
- **The manual item** is open, with tasks 3 to 5's: in kitty, a Gutenberg epub with pictures and
  a contents page (the cover first, chapters a page each, its pictures, `/` across chapters, `d`,
  `+`, `f` to a chapter); a book without a contents page of its own (the Contents page after the
  cover, `f` to a chapter); a directory of 20 epubs as a list, timed, its tiles their covers; a
  Markdown README in a zip with its screenshots.

The spec stays `Implementing` until those are confirmed.

## Task 6: a shorter sentence where a setting isn't, on the whole status line

**The owner's decision, 2026-11-16**, through the lead, on the sentence that no longer fitted 80
columns (task 2's discovery): do both. A shorter sentence of the shape `this format doesn't reflow
(X, Y and others do)`, X and Y from what the backends declare at run time, never written in the
core; and while it shows, it has the whole status bar, for about a second, then the status line
comes back. The three tests' windows back to 80 columns; tested there.

**Choices made on the way**, as the lead asked, to be told:

- **A key pressed during the second takes the line back at once, and does what it does**: `Nav::key`
  drops the message as it drops any, before the key acts. The same key says it again, for a
  second of its own.
- **How the loop wakes**: the pager keeps when the message's moment is over (`Moment`), counts it
  into the wait before the next read, as it does a room that changed or tmux's check, and redraws
  when it is over. The moment starts when the loop first sees the message, not when the key was
  decoded: the same within a few milliseconds.
- **It is kept apart from the status line's ordinary message** (`Nav::notice`, not `Nav::message`):
  a page's warning that arrives during the second isn't shown on the whole line, nor lost when the
  second ends.
- **Only this sentence takes the whole line.** Every other message keeps its place beside the
  rest, cut where it is too long, as before.
- **In a list none of whose files has the setting, `these files don't…`**: `this format` names
  nothing there.
- **`draw fenced blocks`**, where the documents' and the books' `fences` setting said `draw their
  blocks`: the phrase is now said of a file that doesn't (`doesn't draw…`) and of those that do.
- **One format alone is `X does`, two `X and Y do`, more `X, Y and others do`.**
- **`?` is left as it was**: every format that has a setting, after its keys. Its `d` line is now
  cut in its middle at 80 columns (`Mermaid, Markdown,…nd EPUB light on dark, or dark on light`),
  as its `[ ]` and `{ }` lines already were. Whether `?` should name two formats too is the
  owner's: told to the lead.

**Tests, each seen failing** with its fix reverted:

| Reverted | Failed |
|---|---|
| said as any message, in its place beside the rest | `view::pager::tests::a_settings_key_cycles_its_values_where_what_is_shown_has_it`, `view_pager::s_on_a_pdf_says_only_markdown_and_html_draw_their_blocks_and_a_list_says_to_open_one` |
| the status line as ever while it is said | `view::pager::tests::a_settings_key_cycles_its_values_where_what_is_shown_has_it` |
| a moment that is never over | `view::pager::tests::a_message_on_the_whole_line_has_it_for_a_moment`, `view_pager::on_a_pdf_the_width_keys_say_only_markdown_and_html_reflow` |
| a key that starts no moment over | `view::pager::tests::a_message_on_the_whole_line_has_it_for_a_moment` |
| every format named | `view::tests::the_formats_that_have_a_setting_are_those_their_backends_list` |
| said of a format in a list too | `view::pager::tests::a_settings_key_cycles_its_values_where_what_is_shown_has_it` and the list's |
| the old phrase for fenced blocks | `view_pager::s_on_a_pdf_says_only_markdown_and_html_draw_their_blocks_and_a_list_says_to_open_one` |
| a key that leaves the message on the line | `view::pager::tests::a_settings_key_cycles_its_values_where_what_is_shown_has_it`, `view_pager::what_has_no_dark_form_isnt_asked_and_d_says_which_formats_go_dark` |

Not held by a test that fails without it: the loop's call to `Moment::key` and its wake at the
moment's end. Without the first, a sentence said twice within a second would go with the first's
second; without the second, it would go at the loop's next tick, a fifth of a second late at most.
`Moment` itself is unit-tested for both.

Gates on 2026-11-16: `cargo fmt --check`, `cargo clippy --all-targets -j 2 -- -D warnings`,
`cargo test -j 2` (595 unit, 13 docs, 294 integration), all passing. In kitty, for the owner: on a
PDF in a window of 80 columns, `]`: `this format doesn't reflow (Markdown, HTML and others do)`
alone on the status line, the line back after a second; `d` then at once `j`: the line back with
the key.

## Rebased onto main, 2026-11-16

The lead's word: once the status-line change was committed, rebase onto main, where Spec-0013 (keys
bound to named commands) had landed. The branch had never been pushed; its commits before the
rebase are kept on `backup-spec-0012-before-rebase`.

- **Conflicts**: the ADR index (0036 is main's, 0037 and 0038 this branch's, both kept), the
  backlog's line on a table of contents, and the pager and its tests where both had changed the
  sentence a key says (`Settings::of`, which main brought, now takes each setting's formats as a
  list, as `view::having` gives them).
- **Main's keys, where this branch named the old ones**: the paper's key is `D`, in the book's
  `paper` setting too, which must be declared as the documents' is; and what comes back from a
  link is Backspace, `u` being a screen up. The two epub tests in the pager press Backspace and
  are named for it (`…_and_backspace_comes_back`); usage, the changelog, ADR-0038 and
  CONTRIBUTING say `D` and Backspace. The spec's own text still says `d` and `u`, as it was
  written before Spec-0013: its Changelog says so.
- Three tests failed after the rebase, all for those keys, and pass with them.

## Task 7 (and task 5 revised): the Contents page last, and a key that goes to the contents

**The owner's decisions, 2026-11-16**, through the lead, in three messages, the last standing:

1. clv's Contents page always goes at the end of the book, its last page, so that it never moves
   the book's own; made whenever the book has a table of contents, whether or not the book has a
   contents chapter; labelled as made by clv from the book's table of contents; its limits and
   the worker's handling as they were. (First it was: leave clv's page out where a chapter is
   guessed to be the contents. Replaced the same day.)
2. A key goes to the contents: the book's own contents chapter when there is one, marked in the
   package or guessed (one of the first few spine chapters mostly links to other chapters, with
   bounded thresholds and tests), otherwise clv's page; Backspace comes back (`u`, as the message
   said, before Spec-0013). General in the contract: a source may declare a contents place, and
   the viewer offers the key only when one is declared.
3. The status bar says the key when a book opens, for about a second, only when a place is
   declared, the wording put together from what the source declares.
4. The key is `T`, and the command is built through Spec-0013's named commands, after a rebase
   onto main.

**Spec-0013's overlap.** Main had Spec-0013 merged by then, so nothing is left to merge by hand:
the command is `Command::Contents`, named `contents`, in `VIEWER`, bound to `T` in `PAGES` (a
file's pages only; a list has no such place), its line in `?` a `Line` like any other
(`CONTENTS_LINE`), and what names its key reads the keymap (`Nav::offer`). `T` was bound to
nothing, in either keymap, and no setting suggests it. A number typed before it is dropped
(`Count::Dropped`), as `f`'s is.

**Choices made on the way**:

- **The contract's declaration is a `Landmark`**: what the place is called, for the reader
  (`Contents`), and its name among the source's links (`Ask::Name`), which the documents' backend
  already answers. The viewer's `Level::seeking` already waited for such a name, for a link into
  a file; it now also keeps where the reader was (`Level::leaving`), so that going to the
  contents is a jump `back` returns from. No new thread, no new request of a backend.
- **The status line's words are `{called}: {key}`**, the source's word and the keymap's key:
  `Contents: T`. Said through `Nav::notice`, task 6's message that has the whole line for a
  second. A relayout doesn't say it again (it makes a new `Level`): `Pager::replace` keeps the
  old level's notice, said or not.
- **Where no place is declared, `T` says `no contents to go to here`**, an ordinary message, as
  `N` without a search says there is none; `?` doesn't list it, and nothing is said at opening.
- **The guess** is by links alone: among the book's first 8 chapters, the first whose links lead
  to 3 other chapters or more and most of whose links (more than half) lead to another chapter.
  Measured on the 22 books (ADR-0038's update has the table): the 18 that have a page of
  contents have 12 to 155 such links there, 76% of the chapter's links or more, and no other
  chapter has more than 2, but one title's list of illustrations, which comes after its
  contents. The 7 EPUB 2 editions that mark a contents chapter in their guide mark the chapter
  the guess finds. The share of a chapter's text that is links was measured too and tells
  nothing: Gutenberg's files hold the contents with the chapters after them.
- **`T` lands at the chapter's first link to another chapter**, where one is written in the
  first 8 chapters (an empty `<a id="contents-in-c2">`), not at the chapter's start: Gutenberg's
  second file is its title page, then its contents. The chapter a package marks is gone to the
  same way where it has such a link, at its start otherwise (past the first 8, or a page of
  contents that links nowhere).
- **What the book marks comes before the guess**, and a marked chapter the book doesn't show
  (protected, cut) is no place: then the guess, then clv's page.
- **The Contents page's room is kept first**: it is counted against the book's bounds before any
  chapter, so a book cut short still ends with it. (It was counted with the first chapter.)
- **The nav document in the spine no longer saves the second worker**: the table of contents is
  read for every book that has one, since the page is made for every book.

**Tests, each seen failing** with its fix reverted:

| Reverted | Failed |
|---|---|
| the Contents page first again | `epub::tests::contents_are_a_page_of_links_at_the_books_end`, `epub::a_books_contents_page_is_its_last_page_whatever_the_book_has_of_its_own`, `view_pager::epub_t_goes_to_the_contents_page_at_the_books_end_and_backspace_comes_back` |
| no room kept for the page | `epub::tests::contents_are_a_page_of_links_at_the_books_end` |
| no line saying who made the page | the same |
| only a nav in the reading order is the book's own; or only the landmarks' and the guide's | `epub::tests::a_book_marks_its_own_page_of_contents` |
| no chapter marked | `view_pager::epub_t_goes_to_the_books_own_contents_marked_or_guessed` (once its marked book listed two chapters: listing three, it was guessed all the same, and the test passed) |
| no guess | `epub::tests::the_contents_are_the_books_own_page_marked_or_guessed_else_the_page_made_here`, `view_pager::epub_t_goes_to_the_books_own_contents_marked_or_guessed` |
| one other chapter is enough; whatever share of its links; guessed anywhere in the book; the last guess, not the first; a place in every chapter; the chapter's start, never its first link | `epub::tests::the_contents_are_the_books_own_page_…` |
| every chapter led to is kept | `epub::tests::a_chapter_of_links_costs_the_guess_no_more_than_a_few_of_them` |
| a place before every link away | both of those |
| the book declares no place | both `view_pager::epub_t_goes_…` |
| the command wherever, declared or not; its line in `?` whatever is shown | `view::pager::tests::the_contents_command_is_offered_only_where_a_place_is_declared`, `view_pager::t_is_offered_only_where_contents_are_declared_never_on_a_pdf` |
| nothing said when the file opens | `view::pager::tests::the_contents_command_…`, `view_pager::epub_t_goes_to_the_contents_page_…` |
| no place kept to come back to; said again for every layout | `view_pager::epub_t_goes_to_the_contents_page_…` |

One revert failed nothing and was a condition too many: `Nav::offer` asked whether the reader is
in a list, where no key is bound to the command anyway. Taken out.

**Seen**, printed through `--protocol kitty` and read back as a PNG: Standard Ebooks' *Alice* at
`--width 60`, its Contents page on pages 189 and 190 of 190, the heading, `Made by clv from the
book's table of contents.` in italics under it, the entries as nested links.

**In kitty, for the owner** (task 7's Manual): a Project Gutenberg EPUB 3 book: `Contents: T` for
a second when it opens; `T` from a chapter lands on the book's own contents, a third of the way
down the screen, Backspace back. A Standard Ebooks book: `T` lands on the last page, `f` and a
label on a chapter, Backspace twice back to where `T` was pressed.

Gates on 2026-11-16, after the rebase and with this task: `cargo fmt --check`, `cargo clippy
--all-targets -j 2 -- -D warnings`, `cargo test -j 2` (607 unit, 13 docs, 297 integration), all
passing. Tasks 5 and 7 stay open for their checks in kitty.

## Codex review, round 1 (of 354cb19)

Three findings, relayed by the lead, each fixed with a test seen failing without its fix.

1. **[P1] A chapter rewritten was built whole before the book's limits were checked**: paths are
   written from the book's top, so 20,000 `<img src="x"/>` in a directory 4,000 bytes deep, 280
   KiB, come to 80 MB, and a chapter within its 16 MiB to gigabytes, in the viewer's process.
   `Written::body` now takes the bytes the book has left (the Contents page's kept) and stops at
   the piece that passes them; and a path that comes to more than `MAX_PATH` from the top is none
   of the book's, so one piece never writes more than a few of them. The chapter is where the book
   is cut, with those after it (`Left::past`), as a chapter past the book's bounds already was:
   Codex suggested counting it as too large, which is what a chapter past 16 MiB as it is read is
   called; this one is within that, and it is the book that is too long with it.
2. **[P2] A picture's `alt` wasn't counted as text**, and the importer writes it where it has no
   picture: 9 MiB of it passed assembly and the documents' backend refused the whole book. It is
   counted now. Which attributes are text was probed through the documents' backend, 8 MiB in each
   of some 40 attributes and elements (`alt`, `title`, `value`, `placeholder`, `label`, `summary`,
   `abbr`, `standby`, `content`, `cite`, `aria-label`, `data-*`, `name`, `href`, `src`…): an
   `<img>`'s `alt` alone is, with a picture or without. The probe was a test of the moment and is
   not kept.
3. **[P2] Paths were resolved with their XML entities**: `a&amp;b.ppm` beside `a&b.ppm` was said
   to be missing. An `id`, a `src` and an `href` are now read with `unescaped`, which was there for
   the package's path (bounded: an entity is at most 10 bytes, and what is none stays as written),
   and written with `&`, `<` and `"` as entities, always between double quotes. No double escape:
   what is rewritten is unescaped first, and what isn't is written as the chapter wrote it, with
   the quote it had. The same for a cover's `<svg><image>` path. Found on the way: the Contents
   page wrote an entry's place into its `href` as it came from the worker; it is escaped too.

| Reverted | Failed |
|---|---|
| its room checked once the chapter is whole; a path of any length | `epub::tests::a_chapter_stops_being_written_where_it_passes_the_books_room` |
| a picture's `alt` no text | `epub::tests::a_pictures_alt_is_text_of_its_chapter`, `epub::a_book_with_more_alt_text_than_a_document_holds_is_cut_at_that_chapter` |
| any element's `alt` | `epub::tests::a_pictures_alt_is_text_of_its_chapter` |
| a picture's path with its entities | `epub::tests::references_are_read_as_their_text_and_written_as_text_again`, `epub::a_path_written_with_an_entity_is_the_file_it_names` |
| a link, an id, a wrapped picture's path with their entities; written back with no entity; the Contents page's link as it is | `epub::tests::references_are_read_as_their_text_and_written_as_text_again` |

Not held by a test that fails without it: that `assembled` hands `body` the room left. With all
the room in the world the book is cut at the same chapter, by the check after it, and only the
memory spent differs; `body` itself is tested to stop within a piece of its room.

Dismissed by the lead, not this branch's: a piped book read without a bound in
`Input::reopenable`, which every backend shares.

Gates on 2026-11-16 with the review's fixes: `cargo fmt --check`, `cargo clippy --all-targets -j 2
-- -D warnings`, `cargo test -j 2` (610 unit, 13 docs, 299 integration), all passing.

## Task 8: `?` with two formats at a line's end, and the keys that do nothing here faint

**The owner's decision, 2026-11-16**, through the lead, on the question task 6 left (`?`'s `D`
line cut in its middle at 80 columns): each line that names formats names a couple, at its end
(`D  light on dark, or dark on light (Mermaid, Markdown and others)`), the `[ ]` and `{ }` lines
the same, from what the backends declare, as on the status line; and the keys that do nothing on
the current document are greyed out, told from what the viewer knows of the source (its settings,
its contents place), never from a format's name, legible on light and dark and right as text
cells. Tested at 80 columns: no line cut, a PDF's `D`, `[`, `]` dim, a Markdown file's not.

**Choices made on the way**:

- **Faint is SGR 2**, undone by SGR 22, around each row: the terminal's own dimming of its own
  foreground. No colour is picked, so none can be wrong for a theme: a fixed grey would vanish on
  a background near it, and "bright black" is the background's twin in some themes. The keys are
  text in every output (kitty's protocol, text cells, a pane of tmux), so nothing differs there. A
  terminal with no faint shows the keys as the rest, which loses the hint and no key.
- **Which keys**: a setting the source hasn't; a setting a document is laid out again for (the
  width, the margins) in a list or among `--pages`, where its keys only say to open a document;
  and `T` where the source declares no contents. The same test `Nav::turn` goes by, now one
  function (`Nav::idle`). Not `/` on a file with no text, nor `f` on one with no links: the viewer
  knows neither until it asks the source for them, which starts their work.
- **`T` is listed where there are no contents, faint**, where task 7 left it out of `?`: the
  owner's example names it.
- **No line cut means lines go over rows.** Shortening to two formats wasn't enough at 80
  columns: the width's line says 68 characters before any format, in 59 of room. What a line says
  now wraps under its column, where the window has rows for the whole of `?` (34 rows at 80
  columns on a PDF, which a window of 40 rows holds); where it hasn't (80 × 24, where `?` had a
  line more than the rows already), each line keeps to one row, cut as before, so that no key
  falls off the bottom for another's second row. The viewer's own long lines (`/`, `f`, `n N p`)
  are whole at 80 columns by the same means; they were cut on main too.
- **`view::together` is gone**: nothing names every format any more.

**Tests, each seen failing** with its fix reverted:

| Reverted | Failed |
|---|---|
| the formats first again | `view::pager::tests::the_keys_list_the_settings_after_the_viewers`, `…a_count_sets_a_numbered_setting_and_is_used_up`, `view_pager::the_keys_at_80_columns_are_whole_and_those_that_do_nothing_here_are_faint` |
| every format named | `view::pager::tests::the_keys_list_the_settings_after_the_viewers` |
| no setting's keys idle | `view::pager::tests::the_keys_that_do_nothing_on_what_is_shown_are_listed_as_idle`, `view_pager::the_keys_at_80_…` |
| idle only where the file hasn't the setting | `view::pager::tests::the_keys_that_do_nothing_…` |
| the contents never idle | that, `view::pager::tests::the_contents_command_is_offered_only_where_a_place_is_declared`, `view_pager::t_is_offered_only_where_contents_are_declared_never_on_a_pdf` |
| idle keys written as the rest | `view_pager::the_keys_at_80_…`, `view_pager::what_has_no_dark_form_isnt_asked_and_d_says_which_formats_go_dark` |
| a row a line, always | `view::pager::tests::the_keys_lines_go_over_rows_where_the_window_has_them`, `view_pager::the_keys_at_80_…` |
| over rows, whatever rows there are | `view::pager::tests::the_keys_lines_go_over_rows_…` |

**In kitty, for the owner**: `?` on a PDF at 80 × 40: `D`, `[ ]`, `{ }`, `s` and `T` faint and
readable, on a light and on a dark background, their lines ending `(Markdown, HTML and others)`,
no line cut; on a Markdown file only `T` faint; at 80 × 24 a row a line, cut in the middle.

Gates on 2026-11-16 with this task: `cargo fmt --check`, `cargo clippy --all-targets -j 2 -- -D
warnings`, `cargo test -j 2` (612 unit, 13 docs, 300 integration), all passing.

## Codex review, round 2 (of 08a3b16)

Two findings, relayed by the lead, and a third found while looking for the second's pattern, as
the lead asked.

1. **[P1] The book's room was checked between pieces, so one tag could pass it**: 20,000 `src="x"`
   in one `<img>`, 160 KiB, in a directory 4,000 bytes deep, wrote 80 MB before it was refused.
   `Written::start_tag` is given the room and stops at the attribute that passes it.
2. **[P1] `unescaped` looked for an entity's `;` in all the rest**, for each `&`, and only then
   asked whether it was near: a run of `&` took the square of its length, 20 s for 1 MiB here, in
   the viewer's process. It looks in the next 11 bytes, as bytes, since a slice of 11 may end
   within a character; what is sliced is up to a `;` found, which is a character's start.
3. **Found on the way, the same shape in `wrapped_picture`**: to see whether an `<svg>` only wraps
   a picture, its `<title>` or `<desc>` was passed over with `past_end_tag`, to the chapter's end
   where it has no end tag; `<svg><desc>` 80,000 times, 880 KiB, took 30 s. It is passed over
   within 4 KiB (`MAX_LOOK`: a cover says what it is in a line), and a chapter's drawings are
   looked into for nothing 1 MiB at most together (`MAX_LOOKED`), after which the chapter's
   drawings stay drawings. A drawing of anything else costs its first tag, so 50,000 of them leave
   a wrapped picture after them its look (tested). The limit is on the look, not on the picture: an
   `<image>` whose reference is megabytes of `data:` is read as before.

Looked at for the same and found linear: the lexer (`Piece::at`, `Attributes`, `past_end_tag`
called from `Written::body`, each of which uses up what it reads), `resolved`, `from_the_top`,
the percent coders, `escaped`. `picture.rs`'s `unescaped` is the percent decoder, one pass.

| Reverted | Failed |
|---|---|
| a tag's room checked between tags alone | `epub::tests::a_tag_stops_being_written_where_it_passes_the_books_room` |
| an entity's end looked for in all the rest | `epub::tests::a_chapter_made_to_be_read_slowly_isnt` (20 s where it has 3) |
| a drawing's `<desc>` passed over to the chapter's end | `epub::tests::a_drawing_is_looked_into_only_so_far_for_its_picture` |
| a chapter's drawings looked into without end | the same |
| both of those | `epub::tests::a_chapter_made_to_be_read_slowly_isnt` (30 s where it has 3); with either alone it passes, the other bounding the time |

The timed test's bounds are 3 s for what takes 40 ms with the fixes, and 20 and 30 s without.

Gates on 2026-11-16 with round 2's fixes: `cargo fmt --check`, `cargo clippy --all-targets -j 2 --
-D warnings`, `cargo test -j 2` (615 unit, 13 docs, 300 integration), all passing.

## Codex review, round 3 (of cd19a36)

One finding, relayed by the lead.

**[P1] Every `href="#x"` took the chapter's directory apart and put it together again**
(`resolved`, through `from_the_top`), for a result `linked` then threw away: under a directory
4,000 bytes deep, 1,600,000 of them in one `<a>` took Codex 34 s, where the book's top takes 0.41
s, and no budget stops it, since so little is written. Two changes:

- `Written::linked` answers a link that is empty, a place alone (`#x`) or a query alone before any
  path is asked about.
- `from_the_top` no longer takes the directory apart: the directory is kept as it is written, its
  end cut back for each `..` that leads up it, so a path costs what the reference is long. That
  is the answer to the lead's second question: `src` and `href` both did work as the directory
  is deep, for each attribute, where the result is short (a link to a chapter is written `#c2`) or
  thrown away (a path past `MAX_PATH`, which `from_the_top_within` now finds by its length without
  putting it together). What is left as the path is long is the look for a chapter by its whole
  path, a hash of it: 800,000 links to a chapter 2,000 directories deep are read in under 2 s.

**The test, seen failing before the fix**: `epub::tests::a_reference_costs_nothing_for_its_chapters_depth`,
timed, 3 s for each of `href="#x"`, `href=""` and `href="two.xhtml"`, 800,000 times in one tag
under a directory 2,000 deep: the first took 22.9 s on the code as it was, and the test failed
there. It holds what `from_the_top` answers too (up the directory, from the top, above it).

**Left undone, for want of time** (the session's usage limit was reached here): the two reverts
one at a time (the early answer alone; the directory taken apart again alone), a property test
holding the new `from_the_top` to the plain one name by name, which was written and not put in,
and a line in `docs/architecture.md` on `from_the_top_within`.

Gates on 2026-11-16: `cargo fmt --check`, `cargo clippy --all-targets -j 2 -- -D warnings`,
`cargo test -j 2` (616 unit, 13 docs, 300 integration), all passing.
