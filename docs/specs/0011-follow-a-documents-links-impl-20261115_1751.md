# Spec-0011: implementation notes (2026-11-15)

Notes made while implementing [Spec-0011](0011-follow-a-documents-links.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 1: the contract and PDF links

- **The contract** (`source.rs`): `Source::links() -> Option<Box<dyn Links>>`; `Links` takes `Ask`
  (`Page(page)`, `Name(name)`) and gives `Told` (`Page { page, links, more }`,
  `Name { name, at }`). `more` is how many links a page has past the 1,024 given, which the spec's
  Limits want said and a list of links alone can't say. The limits are constants there
  (`MAX_LINKS`, `MAX_LINK_RECTS`, `MAX_ADDRESS`, `MAX_NAME`): a source keeps to them as it makes
  its answer, and the viewer checks them again on what it is given.
- **`Target::Other` is the whole message**, in the backend's words (`a script, which clv doesn't
  run`), not a kind for the viewer to put into a sentence: the viewer knows no format, and a place
  the document doesn't have (`no #usage in this document`) is said through it too, so `Target`
  keeps the spec's four variants.
- **The queue** (`texts.rs`) is generic over what it gives (`Queue<G: Reply = Got>`): a `Reply`
  says what it answers and what it weighs. `Queue::serve` takes a closure; `Queue::start`, for
  `Got`, stays what the two text readers call. `Ask` holds a `String`, so what is asked is `Clone`,
  not `Copy`.
- **`link.rs` holds the check of what a source gives too** (`link::checked`: a page of the
  document, finite rectangles clipped to it, the caps), beside the checks on a web address and a
  path: the spec puts it "as a search's is", and the search's is in the viewer.
- **A web address with a space in it is refused**, with the control, bidirectional and invisible
  characters the spec lists: a space of any kind (`char::is_whitespace`) doesn't show either. `http`
  and `https` must have `//` and a host's first character after them.
- **A `mailto`'s parameters** are what follows its first `?`, apart at `&`, `;` and any further `?`,
  named by what comes before `=`, blanks before it left out; one still percent-encoded after eight
  layers is refused unread, since what it names can't be known.
- **A local path** goes through `source::real` (which refuses what is given or resolves under
  `/dev` or `/proc`, as `o` does), after a `canonicalize` of its own for the words of one that
  isn't there. **An absolute path is followed as it is** (`Path::join`): the spec's "relative" says
  what a document usually writes, and an absolute path leads nowhere a `../../..` doesn't. **Taken
  as written**: no `~`, no percent-decoding (`my%20file.md` isn't `my file.md`); raised below.
- **pdfcraft**: `Pdf` keeps the `PageInfo`s and the links of the `inspect` it already ran
  (`Linked`), a page's links found by a sorted index. `/FitV` is the page's top on every page, as
  the spec has it, though on a page turned a quarter its `left` runs down the page shown; only
  `/XYZ` goes by `left` there. `/FitR` goes by the top of its rectangle *as shown*
  (`user_to_view` on both corners). `resolve_remote_document_path` is asked against a document
  that stands for any (`/a/document.pdf`): it does no I/O, and only says whether the file
  specification is a local path at all.
- **The fixture** (`fixtures::linked()`) is read by the backend's unit tests too, through
  `#[path]`, since the task's Verify is `--bin clv links` and the fixture is the integration
  tests': one PDF written by hand, not two.
- **Until `f` is a key**, nothing calls `Source::links`, so `Source::links`, `Links`, `Told`'s `more`
  and `mod link` carry `expect(dead_code)` outside tests, and `fixtures::linked` an `allow` (it is
  used where the unit tests include it, unused in the integration tests): the next task takes them
  off. An `expect(dead_code)` makes what it is on a root for the lint, so the backend's code
  needed none.
- **Seen failing**, the fixes reverted together in copies of the three files (`cp` aside, `cmp`
  after), `cargo test -j 2 --bin clv link`, 13 failed and the 4 older tests the filter also runs
  passed:
  - `link::checked` keeping every rectangle: `a_pages_links_are_checked_and_a_malformed_rectangle_dropped`;
    without its two `take`s: `a_pages_links_and_a_links_rectangles_are_held_to_their_caps`; the
    target passed on unchecked: `where_a_pages_links_lead_is_checked`;
  - `link::web` taking any scheme: `a_web_address_is_http_https_or_mailto`; without the check for
    hidden characters: `a_web_address_is_refused_for_its_length_or_for_hidden_characters`; a
    `mailto` never read for its parameters: `a_mailto_that_names_a_file_to_read_is_refused`;
  - `link::local` taking whatever the path resolves to as a file:
    `a_links_path_leads_to_a_regular_file_or_a_directory_only` (the FIFO and `/dev/null` opened as
    files);
  - pdfcraft's `shown` giving user space as it is: `a_pdfs_links_are_where_they_are_active_on_their_pages_as_shown`;
    `down` always the page's top: `a_pdfs_links_lead_to_places_addresses_and_files` and
    `links_destinations_go_down_the_page_as_shown`; `Linked::on` without its `take`:
    `links_past_the_caps_are_left_out_and_said`; a name never looked up:
    `links_named_destinations_are_looked_up_in_the_file`;
  - `Told`'s weight 0: `a_queue_of_links_answers_pages_and_names_and_weighs_what_waits`.

## Task 2: `f` directly, a place, and `u`

- **Where the state is.** `Hints` (the codes, what is typed, the wait, each label's place) is in
  `Nav`, as the search is, and takes the keys there; the pager keeps what is drawn (`Labels`: the
  level and the screen they were made for, the image's id) and where the pages are on screen
  (`Pager::spots`, written by `Pager::place`). `Step` stays `Copy`: `Step::Follow` carries nothing,
  and the pager takes the link chosen from `Nav::chosen`.
- **While the links are read, any key drops the labels and does nothing else**, not only `Esc`:
  a key that moved would label another screen, and one that typed ahead can't know the codes.
  `Ctrl-C` still quits.
- **One label left is followed at once**, as the spec has it, so with 15 links `a` alone follows
  the fifteenth (`as`): the codes count through the letters, and the last few share a first
  letter with no other.
- **The labels wait for the pages too**: they are made once every page has answered *and* the
  pages asked for are on screen (`Pager::ready`), or the 2 s are over, since a label goes where
  its page was last placed.
- **Reading order is by the page's unit**, not by the label's pixel: two links a fraction of a
  pixel apart on screen keep their order on the page.
- **A place always comes a third of the way down** a page taller than the window, whether or not
  it was on screen: a link followed should show that it was (a match on screen leaves the view
  where it is; `Nav::land` is the link's, beside `Nav::reveal`).
- **Going back restores the scroll** (`Nav::scroll_to`), not a third of the way down: it is where
  the reader was.
- **The letters**: capitals, as Vimium shows them, typed in lower case. The box is as tall as
  seven rows of the letter's pixels and half of one above and below, the largest whole number of
  pixels a pixel that fits the cell's height (16 px in a 20 px cell, 32 in 40); a rule a pixel
  high along its top and bottom, none at its sides, so two letters side by side read as one label.
- **The labels' placements have their keys in another order than a page's** (`a=p,q=1,C=1,z=1,i=`
  against `a=p,q=1,i=`), so that the tests' helpers that look for the page placed don't find a
  label.
- **Inside tmux and in text cells `f` says the labels aren't drawn there yet**, until task 3;
  **a link to another file and a web address say they aren't followed yet**, until task 5. Usage
  and the changelog say what this commit does, and those tasks rewrite them.
- **`Esc` while the links are read** is tested on `Nav` (`hints_f_asks_for_the_labels_and_esc_…`),
  not on a pseudo-terminal: a PDF's page answers before a second key can arrive, so no end-to-end
  test can hold the labels unread. The end-to-end test drops labels shown.
- **A relayout handing the jumps on** is in the code (`Pager::replace`) but no PDF is laid out
  again: its test comes with documents' links, task 4.
- **kitty 0.49.2 was asked** (hidden, `--config NONE`): it answers `OK` to the labels' placements
  and `ENOENT` once the image is deleted (ADR-0035's Evidence). Ghostty and WezTerm weren't
  watched: the manual check is the owner's.
- **Seen failing**, the fixes reverted in copies (`cp` aside, `cmp` after):
  - `cargo test --bin clv -- hints u_goes a_part_of`, 12 failed: `codes` always one letter
    (`hints_codes_…`); the labels unsorted (`hints_label_links_in_reading_order_…`); a link's
    rectangle not clipped to the part on screen (`hints_label_the_part_of_a_tall_page_…`); a key in
    no label not dropping them (`hints_typing_narrows_…`); `due` without the wait
    (`hints_wait_for_the_pages_…`); no cap of 1,000 (`hints_label_a_thousand_links_…`); a page with
    no room labelled (`hints_never_panic_…`); a letter typed in the labels' ink
    (`hints_letters_are_drawn_…`); `f` no key (`hints_f_asks_for_the_labels_…`); the labels not
    taking the keys (`hints_take_the_keys_…`); `u` always `Step::Back`, and no `land`
    (`u_goes_back_through_the_links_followed_…`); `place_part` without `z=1`
    (`a_part_of_an_image_is_placed_…`);
  - `cargo test --test it view_pager::links`, 5 failed: `jump_back` going nowhere
    (`links_are_labelled_from_one_image_…`, `links_followed_in_a_file_from_a_list_…`,
    `links_to_a_page_not_among_pages_…`); a letter typed never drawn paler
    (`links_on_two_pages_side_by_side_…`); a screen with no links saying nothing
    (`links_where_there_are_none_are_said`).
- **The box stays open** at this commit: its commands pass, and its manual check (kitty, Ghostty and
  WezTerm: `f`, a label, its page, `u` back) was asked of the owner and not yet answered.

## Task 3: the labels inside tmux and in text cells

- **The labels are part of their rows' lines** (`Pager::place`): each run of label cells is written
  at the end of the line of the row it is on, after the page's cells, so `on_glass` sees a row
  with a label as another row than without, and writes exactly those rows, when the labels show,
  change and go. Nothing else keeps track of which rows carry labels.
- **`Hints::runs`** (pure, in `hints.rs`) says which cells: a label starts at its link's cell and
  is moved left where it would pass its page's last cell; labels side by side are one run; of two
  over the same cell the later one's letter shows. The pixel offsets `Hints::settle` gives are
  left unused there.
- **`kitty::placeholder_at`** writes one placeholder cell with its row, its column and the id's
  high byte: the cell after a run, when that cell is still a page's (found among `Pager::spots`,
  with each page's image id kept beside them while `place` runs).
- **`cells::label`** writes the letters: capitals, as the image has them; dark on yellow in 24-bit
  colour or in xterm's 256 (221, 234, 100, the nearest to the image's colours), and with no colours
  reverse video, dim for a letter typed. Inside tmux the colours follow how the pane names its
  images (`IdColor::Rgb`: 24-bit; otherwise the palette).
- **`tmux.rs` is untouched**, though the task lists it: nothing is asked of tmux for the labels.
- **A surprise: a second terminal attaching dropped the labels.** tmux saying a terminal may lack
  the images makes the pager send every page again; for a moment no page is placed, which the
  labels took for a screen that changed, so they went, and the test's next `Esc` quit the pager.
  Now, while a page on screen is still to come, the labels stay as they are
  (`Pager::labels_fit`, `Pager::ready` set before the pages are placed); once the pages are back,
  the screen is compared again. The same holds directly, after a resize: the labels go when the
  new pages are placed, not before.
- **The fixture** `fixtures::linked_at_its_edges`: a page with a link at its first cell, one in its
  middle and one at its last cell, for the row cases the task names.
- **Seen failing**, the fixes reverted in copies (`cp` aside, `cmp` after):
  - `Hints::runs` not keeping a label within its page: `hints_runs_are_the_cells_the_labels_cover_within_their_pages`;
    `cells::label` writing the letters as typed: `a_links_label_is_capitals_on_yellow_and_a_letter_typed_is_paler`;
    `placeholder_at` naming column 0: `a_placeholder_cell_names_its_row_and_column_outright`, and
    with it `view_tmux::links_inside_tmux_…` (the cell after `S` named the first column);
  - the runs not written into the rows: `view_tmux::links_inside_tmux_…` and
    `view_pager::links_in_text_cells_are_written_over_the_pages_cells`;
  - the labels dropped while the pages are sent again: `view_tmux::links_inside_tmux_…`, as first
    run (above).
