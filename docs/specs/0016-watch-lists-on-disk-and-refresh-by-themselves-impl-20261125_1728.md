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
