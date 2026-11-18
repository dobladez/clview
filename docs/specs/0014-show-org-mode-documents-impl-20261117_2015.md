# Spec-0014: implementation notes (2026-11-17)

Notes made while implementing [Spec-0014](0014-show-org-mode-documents.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 1: the parser and its ADR

[ADR-0040](../adr/0040-org-mode-through-orgize-in-a-worker-process.md); its harness and results in
`docs/adr/0040-measure/`.

- **orgize**, as the spec's evaluation expected; org-rust-parser lost on every count (panics on two
  corpus files and one cut in a hundred, then runs away in memory; `dbg!` to stderr; no planning,
  timestamps, fixed-width, nor a file's `#+TODO:`).
- **The reference was Emacs itself**: `emacs` is installed (31.1), so `reference.el` counts what
  `org-element-parse-buffer` makes of each corpus file, and each parser is compared kind by kind.
  Not in the spec's plan, and the strongest measure of fidelity available.
- **A worker process, not `caught`**: orgize's time grows as the square of what is nested or left
  open (19 KiB of nested emphasis takes 1.6 s), and nested footnotes or lists overflow a stack.
  The documents' limits bound Markdown's parse, not orgize's. Caps chosen at 2 s and 512 MiB from
  the measure; confirmed through the backend in task 2.
- **Two orgize mistakes, worked around rather than patched in a fork**: a table after affiliated
  keywords read as a paragraph (23 of the corpus's 328 tables), and a bracket link whose path
  wraps a line (89 of the manual's 508 links; upstream PR #94, unmerged). How to host a patch was
  put to the owner (through the team lead) with a recommendation for workarounds with canaries,
  as Blitz's; the ADR says workarounds. Revisit if the owner prefers a fork.
- **Plain and angle links** aren't parsed by orgize at all: the converter finds them in its text.
- The cut test found a bug in the harness's own table of marks first (an index past its end);
  fixed before the numbers were taken.

## Task 2: the backend

- **As planned**: `src/source/org.rs` writes the HTML in the worker (`clv --worker org`, in
  `source::WORKERS`), hands it on through `opener` over the other backends, a doctype and
  1,024 bytes of clv's own first, as the epub's book starts. `wordcraft.rs` untouched.
- **What is beside an Org file on disk**: a held file's pictures are read through a `Near`, so the
  backend gives one for a file on disk too: any file by its path from the file system's root, the
  document's own path from there, so that `../pic.png` reads as a document on disk's does
  (`Where::Directory` joins the path to the directory, `..` included). One held in an archive
  passes its holder's on; through a pipe, none.
- **A cache of what the worker wrote** (64 files, 64 MiB, by the Org bytes): the documents' backend
  reads a held file again on every relayout, and each read would start a worker. Not in the spec;
  what Mermaid's backend does for the same reason.
- **The marks are counted after decoding**, on the UTF-8 text the worker gets, as Markdown's are
  counted on its bytes.
- **The order of the backends shows in what the viewer says**: Org is before the documents'
  backend (the spec's decision, for a `.md` that says Org), and the viewer names the formats that
  have a setting in the backends' order (ADR-0033), the first two: `this format doesn't reflow
  (Org, Markdown and others do)`, where it said `(Markdown, HTML and others do)`; `D`'s says
  `(Mermaid, Org and others do)`. Seven tests' expectations changed with it (`view.rs`,
  `view_pager.rs`). Raised with the owner in the final report.
- **orgize keeps one list across two blank lines**, where Org ends it: seen in a unit test, which
  now tests each list alone. Left as orgize has it.
- **A description list's term in a list that isn't one**: written bold before its text, since
  orgize gives a list its kind by its first item.
- **The hostile files through clv**: special blocks nested 10,000 deep are killed past the CPU
  cap; footnotes nested 90,000 deep likewise (`a_hostile_org_file_costs_the_document_and_never_the_viewer`).
- **How much larger the HTML is** (the spec's 🟡): 0.75 times the corpus's size, 1.33 times at most
  for a file over 4 KiB; 0.34 times its lines and marks. Only a file of little but a table's cells
  passes HTML's limits within Org's; ADR-0040's Evidence, with the ignored test that measures it.
- **Through clv**: the Org manual to its first page in 0.83 s and 186 MB; four of it, at the
  limit's marks, in 3.2 s and 655 MB. The caps stand at 2 s and 512 MiB.
- **Verify**: `cargo test -j 2 --test it org::` passes. The manual check (kitty: a TODO heading, a
  table and a mermaid block; `D`, `s`, `]`) is left to the owner, so the task's box stays open
  until it is confirmed.

**Tests added, each seen failing with the fix it covers reverted** (a script copied the file
aside, changed it, ran the test and copied it back, `cmp` checking): unit tests in
`src/source/org.rs`, `org_is_told_by_its_name_and_its_mode_line_and_must_be_text` (mode line not
read), `org_is_read_as_utf8_and_utf16_by_its_byte_order_mark` (UTF-16 LE not decoded),
`an_org_file_past_the_documents_limits_is_refused_before_it_is_parsed` (marks not counted),
`a_headline_is_a_heading_…` (levels to 7), `the_todo_keywords_a_file_declares_are_honoured`
(Org's own always), `the_title_block_comes_first` (none written), `inline_markup_and_what_is_shown_as_written`
(sub- and superscripts parsed and written as such: the first revert, the parser's option alone,
passed, since the writer writes them as written anyway; the option stays, for orgize's stack on
nested scripts), `lists_with_their_checkboxes_and_descriptions` (☑ as ☐),
`a_table_in_a_grid_…` (cookie rows kept), `blocks_as_code_…` (commas kept),
`footnotes_are_numbered_…` (no notes), `a_link_to_a_picture_is_the_picture` (no pictures),
`what_org_hides_is_left_out` (drawers shown), `a_comment_subtree_and_a_noexport_one_are_shown`
(them left out), `a_table_after_keywords_and_a_link_that_wraps_…` (no mending),
`a_line_end_in_a_links_path_…` (line ends kept), `what_is_nested_past_the_depth_…` (twice the
depth), `a_document_the_worker_fails_on_is_refused_and_said` (one message for all),
`a_document_is_written_by_the_worker_once_and_kept` (nothing kept),
`the_cache_lets_go_of_what_was_used_longest_ago` (nothing let go of), `any_org_text_is_written`
(no doctype); in `tests/it/org.rs`, `an_org_file_is_a_document_…` (nothing beside a file on disk),
`a_mode_line_says_org_…` (mode line not read), `org_files_in_a_list_a_zip_and_a_tar` (the
holder's `Near` dropped), `an_org_document_has_the_settings_markdown_has` (the HTML not HTML, as
the settings are the documents' backend's own), `an_org_documents_mermaid_block_…` (no language
class), `an_org_file_is_held_to_the_documents_limits_…` (the HTML's refusal not said as such),
`a_hostile_org_file_costs_…` (parsed in the viewer's process: the nested blocks shown after
seconds, the footnotes overflowing its stack), `help_lists_org_and_credits_orgize` (no credit),
`nothing_but_this_backend_names_orgize` (a comment naming it in `source.rs`); in
`tests/it/view_pager.rs`, `slash_searches_an_org_documents_body_and_s_its_blocks_as_code`
(drawers shown). The two canaries (`orgize_still_…`) test orgize, not clv: they fail when orgize
changes, which can't be reverted here.

## Task 3: links and places

- **The owner confirmed the workarounds** (2026-11-17, through the team lead): no fork, no vendored
  copy; their canaries named in `CONTRIBUTING.md`'s pinned dependencies (task 4).
- **The slug moved to `src/source/slug.rs`**, out of `wordcraft.rs`, with its table of combining
  marks and that table's test: Org's headings and Markdown's are found by the same GitHub slug,
  and a backend can't name another. `wordcraft.rs` changed only by that (the spec hoped it would
  stay untouched).
- **An id with spaces** (the spec's 🟡): wordcraft keeps it as a bookmark as written, and the
  `Linker` finds it, percent-decoded: `[[*Some Heading]]` and `file:other.org::*Some Heading` both
  land (`links_in_org_go_…`, `links_to_an_org_file_…`). Nothing written otherwise.
- **Ids are claimed in any case**: the `Linker` finds a place in any case after the exact one, so a
  heading's slug `same` beside its title `Same`, or a heading `Clash` beside a target `clash`,
  would be two places for one name. The second is left out; a heading left with no id gets
  `heading`, `heading-1`, … so that `[[*Heading]]` still finds it.
- **wordcraft's importer drops an `href` whose scheme isn't `http`, `https` or `mailto`**
  (`wordcraft_doc::link::is_safe_href`): an `elisp:` link was plain text, with nothing to say what
  it is. Such a link (and `file:~user/…`, and `~/` with no home) is written to a path of clv's own
  (`clv-org-not-followed/…`), which the importer keeps, and the Org backend's `Shown` wraps the
  document's links to say what each is (`a elisp: link, which isn't followed`), its address as
  written, as the epub's `InTheBook` does for a book's files. Not in the spec's plan, which said
  `Linker` would need nothing new: it doesn't, but the backend needed a wrapper.
- **A link with a space in its path** (`elisp:(message "x")`) would be said by the viewer as an
  address with a space, before this wrapper: the path is percent-encoded, and the wrapper decodes
  it for what it says and copies.
- **Verify**: `cargo test -j 2 --test it org::links` passes. The manual check (kitty: `f` on an
  Org file, `[[*Heading]]`, `file:other.org::*Heading`, a web link; Backspace back) is left to the
  owner, so the task's box stays open until it is confirmed.

**Tests added, each seen failing with the fix it covers reverted**: in `src/source/org.rs`,
`links_lead_where_org_has_them` (every path as written), `an_address_in_a_links_description_is_its_text`
(autolinks inside a description), `places_are_written_as_ids_the_documents_own_names_first` (ids
claimed in their case only), `a_url_and_an_id_are_written_as_read_back` (`#` not encoded),
`an_abbreviation_too_long_once_expanded_is_left_as_it_is` (no bound),
`a_link_clv_doesnt_follow_says_what_it_is` (the marker not read); in `tests/it/org.rs`,
`links_in_org_go_to_headings_targets_ids_and_notes` (headings not found by title; and, once it
also followed `[[*clash]]`, headings before targets for `[[clash]]`: the first form of the test,
without that link, passed with that revert, since nothing told the target's page from the
heading's), `links_to_an_org_file_go_to_its_heading_from_org_and_from_markdown` (the place after
`::` dropped), `links_that_run_something_or_lead_nowhere_say_so` (every scheme written as it is,
which the importer drops). `slug::tests::the_table_of_marks_is_in_order_and_knows_a_mark` moved
from `wordcraft.rs` as it was.

## Task 4: docs

- `docs/usage.md`: an Org documents section after Books (which files, what it shows and hides);
  its row in Which files it shows; Org's links and places under Following links; its limits; Org
  in the options' and keys' rows, the search's intro, and the status line's example; a known issue
  (orgize keeps a list across two blank lines).
- `docs/architecture.md`: Org in the Backends table (fourth, the rest renumbered), how it opens and
  how its places and links are resolved, `slug.rs` in Files, the settings' example.
- `CONTRIBUTING.md`: orgize in Pinned dependencies with its two canaries and what to remove when
  each fails (the owner's ask), the library rule, and `tests/it/org.rs` under Tests.
- `CHANGELOG.md`: an Added line; the Unreleased Changed lines' examples of what a key says now
  name Org (`(Org, Markdown and others do)`), since the order of the backends makes it so.
- README's Plans and the backlog's More documents: Org out, AsciiDoc and reStructuredText left.
- `CLAUDE.md`'s Layout: `slug.rs`.
- Gates: `cargo test -j 2 --test docs` passes; no Rust changed since task 3's full gates.
