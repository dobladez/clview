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

## Auto-reload

- Its state on and off is `Settings::auto_reload`, a `Cell` that every level shares, as the
  settings' values are, so that `Nav` can put `auto` on the status line. `pager::run` takes the
  option's value.
- The stamps held are those of what `watch_disk` looks at (`Pager::stamps_now`): a file's, or a
  list's directories and its tiles on screen. A reload that failed, and a list made again with
  nothing left in it, keep them in `Pager::auto_failed`: auto-reload leaves that change alone, and
  the warning shows, until they move.
- While the reader is busy, the hold's time moves on by `SETTLE`, rather than the loop waking
  at once over and over.
- `Pager::reload` set the warning itself for a change (`abe33f3`). With auto-reload on, it goes
  through `Pager::said`, which says nothing of a change: `auto_reload_reloads_a_list` caught the
  warning shown while the list was made again.
- `auto` sits before `? help`, in the longest form of the right side only, so it goes first. At 44
  columns it still fits beside two pages; the test narrows to 40.
- `config_auto_reload_starts_it_on` is in `tests/it/view_pager.rs`, not `tests/it/view.rs`: only
  the pager shows that auto-reload is on, and a pipe prints. It reads the config through `--config`,
  as the pager's other config tests do.
- The names' rules grow to four (`toggle-`), in `commands.rs`'s docs and architecture's. ADR-0043,
  which listed three, is left as the record of its moment.
- Added beyond *Verify:* `auto_reload_tries_a_reload_that_failed_again_only_once_the_file_changes`,
  for a requirement no test named: seen failing with `auto_failed` never set, when it tried again.
- Each test seen failing with its fix reverted: `R` bound to nothing (`capital_r_…`); the hold not
  started over when the stamps move (`…once_its_writes_pause`, which saw a reload while the file was
  written); `settle_on` never reloading (`…reloads_a_list`, and the file's); `busy` blind to a search
  typed (`…waits_while_a_search_is_typed`); the option's value dropped (`config_auto_reload_…`).
- `CONTRIBUTING.md`'s Tests says what the new pager, zip and tar tests cover.

## Done checklist

- The gates, the tests each seen failing, and the docs: checked (2026-11-25).
- The three manual checks, in kitty and in a private tmux, wait for the owner: the tests play a
  terminal on a pseudo-terminal, and can't show what kitty draws.

## Codex review fixes

- Two files named alike lost their places in a list made again: `relist` found an old tile's new
  tile by its name alone, always the first of that name, against the spec's "known by its path
  and its stamp". `retiled` now maps each old tile to its new tile once, by the real path of its
  file, else by its name's occurrence (the second `report.pdf` to the second). The tile chosen, the
  filter's tiles and the tiles `--pages` picked all go through that map. Test:
  `r_keeps_the_tile_of_two_files_named_alike`, seen failing with `pager.rs` as it was (the reader
  moved to the first), and its `--pages 2` half alone failing too.
- A file named on the command line that couldn't be opened again kept its old item silently, so a
  list made again reported success, cleared `auto_failed`, and auto-reload reloaded it every
  second. `Opener::piece` now gives `Piece::Kept` with the reason, which goes to
  `Found::unreopened`; `relist` then says `not reloaded: …` and counts it as a failed reload.
  Test: `auto_reload_holds_a_file_given_that_cant_be_opened_again`, seen failing with the reason
  dropped.
- That test showed that `auto_failed` as the stamps of the tiles on screen didn't hold: it was taken
  as the new level replaced the old one, before its first draw, when fewer tiles counted as on
  screen. It now holds what had changed, by path, with its stamp, over every tile
  (`Pager::changes`), and auto-reload leaves a change alone while every change on screen is one of
  those (`Pager::failed_before`).

## Codex review fixes (2)

- A reload that failed recorded the stamps when it ended, so a file written again while it ran was
  held back though never tried. `settle_on` now takes `Pager::changes(true)` as it starts the reload
  (`Pager::auto_tried`), and `auto_reloaded` keeps those. Test:
  `auto_reload_tries_again_a_file_changed_while_a_reload_that_failed_ran` in `tests/it/tar.rs`: a
  broken `a.pdf` is read first as the list is made again, a slow tar (3 GiB of zeros) makes it
  last, and a valid
  `a.pdf` is written meanwhile. Seen failing with the stamps taken at the end. A broken PNG
  doesn't fail there: the image backend takes a `.png` by its name, and defers it.
- A path given that can't be read twice (`/dev/stdin` from a pipe) had no stamp, so no entry in
  `Old`, and a list made again read the spent pipe and dropped its tile. `GivenPath::unstamped` now
  keeps such an item, by its place among the paths, and the path is never read again. Test:
  `r_keeps_a_file_given_through_a_pipe`, seen failing with the kept item passed over.
- `Old` was keyed by the real path alone, so two names for one file shared one item.
  It is keyed by the real path and the name the file was opened under now, and `retiled` the same.
  Test: `r_keeps_each_name_of_one_file_as_its_own_tile`, `notes.md` and `notes.org` linked to one
  text: three distinct tiles before and after `r`. Seen failing (two distinct) with the map and
  its lookup keyed by the path alone. A first revert that changed only the map passed, since
  nothing was found to keep.
- With a 6 GiB tar, the first full run failed
  `past_its_cpu_time_or_its_heap_the_labels_worker_is_killed_and_they_are_left_out` (its worker not
  killed for its memory) while the tar was read twice in parallel; it passed alone. 3 GiB is enough
  for the test, and the full run passes with it.

## Codex review fixes (3)

- Round 2 kept a piped file's item, but not a piped archive: a folder, or with `-r` the archive's
  files, came from `Piece::Item` of a container or `Piece::List`, which it passed over, so a list
  made again read the spent pipe. A path given that `read_once` says can be read only once (there,
  not a directory, and no regular file of its own) now keeps everything it added to the list in
  `GivenPath::once`, whatever it opened as, and is never read again. `Opener::open` takes it after
  each path; `make_on` takes it once the path's container is all listed (`Making::once`). Test:
  `r_keeps_an_archive_given_through_a_pipe`, a zip through `/dev/stdin` beside a PDF, as a folder
  and with `-r`. Seen failing with round 2's rule put back (a single file that isn't a container).
- Round 2 keyed reuse by the real path and the base name, so `one/notes.md` and `two/notes.md`
  linked to one file shared a key, though each draws the picture beside its own path. `OnDisk`
  keeps the path the file was opened by (`OnDisk::opened`, the `Input`'s path); `Old`, `kept` and
  `retiled` key by the real path and that. Test: `r_keeps_each_path_to_one_file_as_its_own_tile`,
  a red and a blue `pic.png` beside the two paths: three distinct tiles before and after `r`. Seen
  failing (two distinct) with the map and its lookup keyed by the name again.
- `tests/it/zip.rs`'s `archive` and `png` are `pub(crate)` now, for the pager's test of a piped zip.
- A full run on a busy machine (the unit tests took 79 s, not 4) failed four of the new tests, each
  passing alone. They assumed timings: the tiles sent first after `r` were the ones to compare; the
  tiles of the list made at a failure were sent within 800 ms; and a 300 ms window after
  `reloading…` was enough for `a.pdf` to be read. The two tile tests now wait for three distinct
  tiles, the held-reload test looks for `reloading` and ` reloaded ` alone (`not reloaded:` stays
  said, and a redraw repeats it), and the tar test waits
  1 s with a 4 GiB tar. Each still fails with its fix reverted, checked again.
