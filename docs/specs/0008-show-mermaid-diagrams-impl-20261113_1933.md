# Spec-0008: implementation notes (2026-11-13)

Notes made while implementing [Spec-0008](0008-show-mermaid-diagrams.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 2: A Mermaid file shown

**Files beyond the plan's:** `tests/it/view.rs` (`--help`'s row and the list of what clv shows, which
name every backend), `tests/data/README.md` (the fixtures), `docs/adr/README.md` (ADR-0030's row),
`docs/adr/0030-measure/worker.sh` (measuring through clv's own worker, below). `.github/workflows/ci.yml`
is unchanged: since 071da53, CI builds each push with all features (`mermaid` among them, its
costliest diagrams run on Linux there), and each feature alone is a local gate, which
`CONTRIBUTING.md` now lists `mermaid` in.

**Decided along the way:**

- `Input::at_most` (`src/source.rs`) reads a file to a cap and stops, for a file and a named pipe
  alike; a file held in another is read whole within the holder's limits, then checked.
- The `mermaid` feature turns on `serde_json`, already clv's dependency (`sheet`) and in merman's
  tree, to build merman's site config (`MermaidConfig::from_value` takes a `serde_json::Value`).
  The svgtypes lock moved 0.16.1 → 0.16.2, which merman requires: a patch release usvg takes.
- The site config sets `maxEdges` to 500 itself rather than trusting merman's default, and lists it
  secure. merman's message for it is "Flowchart edge limit exceeded: … maxEdges budget", which the
  test matches.
- Telling a `.mmd`: after the kind, any ASCII whitespace counts as the spec's "space" (a tab; `\r`
  in a CRLF file); kinds are matched in their case; `%%{init` and `%%{initialize` both count; a line
  counts only when whole within the head (followed by a newline, or the file shorter than the head),
  and a character cut at the head's end ends what is read. Text is "no NUL in the head"; a `.mmd`
  whose header is ASCII but whose later bytes aren't UTF-8 opens, and is then refused as not UTF-8.
- The cache counts a diagram's bytes with its SVG against the 64 MiB, since it keeps them to compare:
  4,096 diagrams of 1 MiB would otherwise be 4 GiB. An entry larger than the whole cache isn't kept.
- The service holds its lock for the whole request (the turn), and checks the cache again once it
  has it, so diagrams opened at once are drawn once. Every death of the worker on a diagram counts
  toward the three in a row (killed past a cap, timed out, ended, malformed); a worker that couldn't
  be started (`TooMany`, `Unstarted`, `Version`) and the pause aren't kept in the cache, the rest
  are. The idle timer is a thread a worker, which ends with it; if it can't be started, the worker is
  let go of after its diagram. The service is given how to start a worker, so the tests can give it
  one that dies.
- `-` is no name clv gives stdin at all: it fails as a file not found, before any backend is asked.
  The test says so.
- merman's `Constrained` profile caps its SVG at 24 MiB itself (`max_svg_bytes`), so the 64 MiB on
  the worker's answer is a backstop.

**Surprises:**

- **No diagram reaches the worker's caps.** Through `clv --worker mermaid` (`worker.sh`), the
  costliest accepted diagram peaks at 0.23 s of CPU and 232 MiB, and 24 further hostile diagrams for
  the kinds the spike left out (packet, block, architecture, sankey, kanban, quadrant, radar,
  treemap, git graph, ER, class, timeline, `%%{init}%%` spacing and wrapping) were drawn under
  0.32 s or refused by merman's resource limits in milliseconds (ADR-0030's Evidence has the list).
  So the Verify items "one past the CPU cap and one past the heap cap each failing, and the next
  drawn" and "three killed in a row pausing the worker" can't be met by real diagrams through the
  binary. Asked the owner (through the team lead), who chose unit tests with a fake worker that
  dies, with `worker.rs`'s own tests of the kernel's and the allocator's kills; the spec's Verify
  says so now. Meanwhile a heap cap
  lowered by hand to 64 MiB shows the real kill through the binary: `the_costliest_…` then fails
  with "merman took too much memory on the diagram" for 15,000 nodes, and the next diagram is drawn.
- **A dense diagram merman draws can be refused by the SVG backend:** 32 nodes each linked to every
  other (496 edges, 1.2 MB of SVG) comes to more than its 128 MiB once its arrowheads (markers) are
  repeated; 24 nodes (276 edges) is drawn. Said in usage's Limits.
- **Kanban keeps `<foreignObject>`** for its cards with `htmlLabels` false, as a user journey does,
  so its tile shows no words either: usage says both. merman has a best-effort fallback that writes
  a `<foreignObject>`'s text as SVG `<text>` for rasterizers (`svg::fallback`); using it would change
  what the spec decided, so it's raised as a discovery, not used.
- **A diagram's background.** clv draws an SVG on transparency (the terminal's background shows), and
  merman's SVG has none, so a diagram on a dark terminal loses Mermaid's #333 edges and texts; the
  spec said "on white, as a drawing is", which a drawing isn't. The owner chose white: `on_white`
  puts a white `<rect>` over merman's viewBox (its viewport without one) as the first child of its
  `<svg>`, after the worker's answer is checked against its cap and before it is cached, reading the
  start tag alone (its `>` found outside quotes within 4 KiB). A sequence diagram's viewBox starts
  left of and above 0 (`-50 -10 …`), a timeline's, a journey's and a styled flowchart's too, so a
  rect at a default origin would leave a strip transparent: the integration test checks every pixel
  of `flow.mmd` and `sequence.mermaid` opaque and their corners white. The spec's
  Theme line, ADR-0030's Decision and usage say so.

**Each test seen failing**, its fix reverted (the file copied aside, changed, the test run, copied
back, `cmp`):

| Test | Reverted | How it failed |
|---|---|---|
| `mermaid::tests::a_mmd_starts_as_a_diagram_merman_draws` | `opens` taking any `.mmd` | MultiMarkdown's heads opened |
| `it mermaid::a_mmd_is_opened_when_it_starts_as_a_diagram` | the same | `notes.mmd` given to merman, its error not "not a file clv shows" |
| `mermaid::tests::the_cache_is_bounded_and_lets_go_of_the_least_used` | no eviction | a fourth diagram kept |
| `mermaid::tests::what_the_worker_answers_is_kept_but_for_no_worker` | `TooMany` put in the cache; then, apart, both cache lookups taken out | "not kept" found kept; the second drawing not the first's `Arc` |
| `mermaid::tests::the_worker_is_gone_two_seconds_after_its_answer` | no idle timer started | still alive at 4 s |
| `mermaid::tests::three_workers_killed_in_a_row_pause_the_service` | the pause's test turned (`>`) | a fourth worker started |
| `mermaid::tests::a_diagram_is_put_on_white_over_its_view_box` | the rect at its default, whatever the viewBox | the viewBox's origin and size not in the rect |
| `it mermaid::a_diagram_is_drawn_as_merman_draws_it` (on white) | no rect; then, apart, the rect at its default | transparent at 0, 0 of `flow.mmd`; of `sequence.mermaid`, whose viewBox starts at -50, -10 |
| `mermaid::tests::a_reason_is_cut_at_a_character` | cut at a byte | a panic on `é` |
| `it mermaid::a_diagram_is_drawn_as_merman_draws_it`, `…diagrams_in_a_directory_are_tiles` | the backend taken out of `BACKENDS` | refused; no tiles |
| `it mermaid::a_diagram_through_a_pipe_is_refused` | `opens` not asking the name | `/dev/stdin` opened as a diagram |
| `it mermaid::a_broken_diagram_is_refused_with_merman_s_message` | the reason without `merman: ` | its prefix missing |
| `it mermaid::a_diagram_past_its_limits_is_refused_with_why` | `Input::bytes` for `at_most`; then, apart, the UTF-8 check | merman's own message for 1 MiB; "merman failed on the diagram" for Latin-1 |
| `it mermaid::the_costliest_diagrams_merman_accepts_are_drawn_under_the_caps` | the heap cap at 64 MiB | 15,000 nodes: "merman took too much memory on the diagram", the rest drawn |
| `it mermaid::nothing_but_this_backend_names_merman` | a comment naming merman in `source.rs` | named outside its files |
| `source::tests::a_file_is_read_whole_only_within_a_cap` (`Input::at_most`) | its cap's check taken out | a file past the cap given whole |
| `source::tests::the_extensions_a_backend_lists_are_the_names_it_opens_by` | the Mermaid head taken out | `.mmd` told by no head |

**Gates:** the five, then each feature alone (`image`, `pdf`, `svg`, `svg-html`, `mermaid`, `zip`,
`tar`, `word`, `sheet`) and `pdf,svg`, all passing, before the white rect; after it, which touched
no `cfg`, the five and `mermaid` alone again. Clippy without `mermaid` first found `Input::at_most`
unused in test builds: it has a unit test of its own now. Once, under the load of two builds,
`view_pager::a_copy_still_rendering_stops_when_the_document_is_laid_out_again` failed on its
timing; alone, and in the next full run, it passed.

**Manual check:** confirmed by the owner on 2026-11-13, in kitty, from the worktree's
`target/debug/clv`: `flow.mmd` Start to Finish on white and readable, `sequence.mermaid`'s
lifelines and messages on white edge to edge, `notes.mmd` refused. Task 2 ticked then.

**Gates, from 2026-11-13 on:** the owner's rule makes them the build that ships alone (`fmt`,
`clippy --all-targets`, `test`, every feature on); each feature alone and the build without backends
are CI's nightly run's. The runs above were made before it.

## Task 3: A diagram in a list and in a document

As planned: `deferred` (no settings) and `draws_pictures`, and nothing more of the cache, which
task 2 already made answer every opening of the same bytes.

**Tests:**

- `tests/it/view_pager.rs` watches the workers with `ps` (`descendants_running`). A worker lives 2 s
  past its last answer, so one that drew a diagram just now is still there: checking right after a
  step's pages are drawn tells whether that step turned the diagram into SVG, without racing.
  `a_lists_diagram_starts_a_worker_only_when_its_tile_is_wanted` has twelve PDFs at one to a row
  before the diagram, so its tile isn't among those wanted first. It then opens the tile and copies
  it (`c`), with no worker again: the page is drawn from the cache.
  `a_documents_diagram_is_turned_into_svg_once_whatever_it_is_laid_out_at` pages through a
  Markdown document with the diagram twice in it (`n`, `n`, `g`) and lays it out again (`40]`,
  `]`). The spec's Verify says "the worker answers once, by its request count": the request count
  isn't seen from outside the process, so the test goes by the worker's life instead, which tells
  the same: a second answer would leave a worker alive.
- A list's diagram that fails is said at the end by its page (`clv: page 2: b.mmd: merman: …`), as
  a list's PDFs are, once it defers; before, it was left out at listing.
- Text cells: `--protocol ascii` and `blocks` print `flow.mmd`.
- A wait in the pager test first sliced the output from the status line's words, losing the
  `  page ` before them, so no step ever matched; and at width 40 the pager shows two pages to a
  row (`pages 1–2/5`). `rendered` takes both forms.

**Each test seen failing:**

| Test | Reverted | How it failed |
|---|---|---|
| `view_pager::a_lists_diagram_starts_a_worker_only_when_its_tile_is_wanted` | `deferred` → `None` | a worker before the diagram's tile was wanted |
| `mermaid::a_lists_diagram_is_opened_when_its_tile_is_wanted_and_a_broken_one_costs_its_tile` | the same | `--pages 1` opened the broken diagram at listing, and said so |
| `wordcraft::a_mermaid_diagram_is_a_picture` | `draws_pictures` → `false` | no red, no blue on the page |
| `mermaid::a_diagram_in_text_cells` | the backend out of `BACKENDS` | refused |
| `view_pager::a_documents_diagram_is_turned_into_svg_once_whatever_it_is_laid_out_at` | both cache lookups taken out | a worker after the next page was drawn |

**Manual check pending:** in kitty, a list of `.mmd` files with their words in the tiles (a user
journey's and a kanban's without); a Markdown file's `![](x.mmd)`, `]` and `+`; the same inside a
private `tmux -L` server.

**Gates:** the five, each feature alone and `pdf,svg`, all passing.
