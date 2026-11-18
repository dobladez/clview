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
