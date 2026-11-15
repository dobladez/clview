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
