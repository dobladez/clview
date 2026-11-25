# Spec-0016: implementation notes (2026-11-25)

Notes made while implementing [Spec-0016](0016-watch-lists-on-disk-and-refresh-by-themselves.md):
decisions, changes from its plan, surprises, and what a later reader would want.

## A list looks at its directories and its tiles, and warns

- `MAX_WATCHED` (1,024) is in `source.rs`, beside `Stamp`, not in `dir.rs`: the viewer caps the
  directories for the whole list (`Found::watch`), and the core can't name `dir.rs`. `dir.rs` caps
  each listing by it too, and sets `Listing::unwatched` when it reads more.
- `Shown` carries the directories to the level (`Shown::directories`, a `Watched`), and
  `Shown::file` builds a file's `Shown`, so that the four places that built one by hand name no
  list field.
- A directory without a real path (`real` fails: under `/dev`, or gone) is listed but not looked
  at.
- The recipe that a list keeps (the paths given and `-r`) moved to the next task, where `r` first
  uses it: here it would be dead code, which clippy refuses.
- Between this commit and the next, a list's warning says `r reloads`, but `r` in a list only
  renders again.
- `list_on_disk_now` puts `Changed` before `Gone`: a subdirectory deleted makes its parent
  changed, and that is what `r` acts on.
- ADR-0044's numbers come from `docs/adr/0044-measure/stat.rs`, a single file built with `rustc`,
  since it needs no crate.
- Tests added beyond *Verify:* `a_list_looks_at_the_first_directories_read_and_says_once_there_were_more`
  (`mod.rs`), for the cap of the whole list. `a_zips_list_is_not_looked_at` sits in
  `tests/it/zip.rs`, beside the zip's other tests through the pager.
- Each test seen failing with its fix reverted: `list_on_disk_now` made to answer `Same` (the three
  pager tests time out waiting for the warning); `dir.rs` reading a subdirectory without stamping it
  (the `-r` test, and the unit test's paths); `dir.rs`'s cap lifted (the unit test); `Found::watch`
  keeping every directory (its unit test); `Opener::finish` looking at the archive's own path too
  (the zip test sees the warning).

## `r` makes a list again

- **A change to the contract the spec didn't list:** `Container::file_on_disk(name)`, a default
  method (`None`), which `Directory` answers with its path joined to the name. `Opener::step` must
  stat an entry before it opens it, and the core can't make a path from a container's name, nor
  name `Directory`. Without it, `step` would open each file's `Input` to learn its path, and
  `Ordered` would record every file as opened. An archive's files have no path, so a relist of an
  archive opens them all again; archives aren't looked at anyway.
- The recipe (`Recipe::Given`, `Recipe::Container`) moved here from the first task. It travels on
  `Shown::recipe` to `Level::recipe`. `Opener::open` sets `Recipe::Given`; `read_on` sets
  `Recipe::Container` for a container gone into or reached by a link.
- `Opener::open`'s loop over the paths given became `Opener::piece`, which a list made again uses
  too (`Opener::make_on`). The single file shown alone, and its error, stay in `Opener::open`.
- `Making` replaces `Reading`'s container and `Opening`: going into a container is a `Making` of
  `Recipe::Container` with no old items, so one path reads a container gone into and a list made
  again. `Opener::begin`, `step` and `finish` became private to `mod.rs`.
- A kept item goes in its own slot (`Slot::Kept`), since it has no backend to give `Found::used`,
  which only `Opener::open` reads for its check of the options.
- A path given that can't be opened now keeps its old item whatever the error, not only when it is
  gone: the spec says a file named on the command line is never dropped. `GivenPath::real` finds
  it, the real path that the path led to when the list was last made.
- `r` makes a list again only when it is `Changed`, as `r` on a file reloads only a changed file.
  A `Gone` list is only rendered again.
- `Esc` during a relist says `stopped reloading`. It releases only the containers that the relist
  itself opened from the paths given, never the level's own container.
- What `Opener::again` does is mostly done already for a document: going into a tile lays it out
  for the window, and `abe33f3` reloads a changed file before a relayout. A picture isn't laid out
  again, so `going_into_a_tile_changed_on_disk_opens_it_again` uses PNGs. With PDFs it passed
  without the fix.
- `esc_stops_a_list_made_again` is in `tests/it/tar.rs`, beside `slow`, the tar that takes seconds
  to read: a tar given with `-r` is rewritten as slow, and a file beside it changed, so `r` waits on
  the tar.
- `?` in a list says `render again, and list again a directory changed on disk`, and usage's table
  of a list's keys gets its `r` row, which it lacked.
- Each test seen failing with its fix reverted (`mutate.sh`: a file copied aside, one change, the
  test run, the file copied back and `cmp`-ed): `step` never keeping an item
  (`an_unchanged_file_is_not_opened_again`); the tile chosen by its place, not its name
  (`r_lists_…`); the filter dropped (`r_keeps_a_lists_filter_by_name`); the empty result's message
  left out (`r_on_a_list_emptied_…`); Esc ignored while a list is made again
  (`esc_stops_a_list_made_again`); a path given that can't be opened left out
  (`r_keeps_a_file_named_…`); `Opener::again` never asked (`going_into_…`).
