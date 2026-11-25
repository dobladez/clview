# Spec-0016: Warn when a list's directory changes, and let pages and lists refresh by themselves

- **Status:** Implementing
- **Date:** 2026-11-25
- **Related:** commit `abe33f3`, which this builds on (a file's pages warn when the file changes,
  and `r` reloads it); [Files and lists changed on disk](../architecture.md#files-and-lists-changed-on-disk),
  [Lists and levels](../architecture.md#lists-and-levels) and
  [Commands and keymaps](../architecture.md#commands-and-keymaps-commandsrs-pagerrs) in the
  architecture; [The status line](../usage.md#the-status-line), [Keys](../usage.md#keys) and
  [Options](../usage.md#options) in the manual;
  [ADR-0019](../adr/0019-a-list-opens-its-files-when-their-tiles-are-wanted.md) (a list opens its
  files when their tiles are wanted); [ADR-0036](../adr/0036-keys-are-bound-to-named-commands.md)
  (keys bound to named commands); the backlog's
  [A badge on a tile whose file changed](../backlog.md#a-badge-on-a-tile-whose-file-changed) and
  [A document that notices its pictures changed](../backlog.md#a-document-that-notices-its-pictures-changed).
  It delivers none of README's Plans.

## Summary

Since `abe33f3`, a file's pages warn when the file changes on disk, and `r` reloads it. A list
doesn't. Files added to or removed from its directory, or a tile's file rewritten, show only after
the reader leaves and goes back in. This spec makes a list of a directory do what a file already
does. Once a second, clv looks at the directory and at the files of the tiles on screen. When
either changed, the status line shows the same yellow warning. The key bound to `rerender` (`r`
by default) lists the directory again. It keeps the tile the reader is on, by its name, and
opens again only the files that changed. Nothing on screen changes until the reader presses the
key, unless the reader asks for that. The option `--auto-reload` (`auto-reload = true` in the
config), and `R` to turn it on and off, make files and lists reload by themselves once the writes
pause, with no warning. While it is on, the status line shows `auto`, and after a reload it says
`reloaded` for about 1.5 s. clv goes on polling with `stat`, and adds no dependency on the OS's
file notices: ADR-0044 says why.

## Requirements

### A list warns

- These lists are looked at:
  - A list of a directory: named alone on the command line, among other paths, gone into from a
    list, or reached by a link. Files added and removed are noticed only in a directory.
  - Any list whose tiles are files on disk, the files named on the command line included: a
    tile's file whose content changed is noticed.
- Once a second, clv looks at these:
  - The stamp of each directory that the listing read: its modification time and size, which a
    file added, removed or renamed in it changes. Without `-r`, that is the directory itself.
    With `-r`, it is each subdirectory read too.
  - The stamps of the files whose tiles are on screen, never every file of the list.
- A directory's stamp is taken before the directory is read, as a file's is taken before its
  bytes are. So a file added while the listing runs shows as a change, never as nothing.
- clv looks at 1,024 directories at most, for the whole list. Past that, the listing goes on, and
  clv looks at the first 1,024 read and says so once, as a listing cut short is said.
- When a directory or a tile's file changed, the status line shows the file's warning:
  `changed on disk, r reloads`, with the key bound to `rerender`, shorter in a narrow window. A
  directory that is gone shows `gone from disk`.
- An archive's list (zip, tar), a file in an archive, and a pipe are not looked at, as for a file.

### `r` in a list

- `r` makes the list again, the way it was made:
  - A list from the command line goes through its paths again, with `-r` as it was given: each
    container listed again with its prefix, and each file named opened again only if it changed.
  - A list gone into, or reached by a link to a directory, lists its container again, without
    `-r`, as going into it does.
- While the list is made again, the old tiles stay and the status line says `reloading…`. The
  keys do what they do while a container gone into is read: Esc stops it and keeps the old list,
  `back` stops it and goes back, and `q` quits.
- An item is known by the path the list opened it by, and its stamp. Each file whose path and
  stamp are the same is not opened again: its source, or its `Later` still opening, is kept. Its
  tiles are sent to the terminal again, as `r` sends every page again.
- Each file that changed is opened again, as a new file is, deferred where its backend defers
  ([ADR-0019](../adr/0019-a-list-opens-its-files-when-their-tiles-are-wanted.md)). Each new file
  is added at its place in the order, and each file that is gone is dropped. A file renamed is one
  gone and one new.
- A file named on the command line is never dropped. Gone, its tile stays as it was.
- The new list keeps these by name, never by a tile's number:
  - The tile the reader is on. If that file is gone, the reader goes to the tile now at its place,
    or the last one.
  - The filter: its text, its tile chosen last, and what Esc puts back. With no match, it behaves
    as a filter with no match does now.
  - With `--pages`, the tiles picked that are still there, in their order. A new file is not
    added, since it wasn't picked. If none is left, every tile shows, as for a file's pages.
  - The zoom and the count to a row.
- A list made again with nothing in it keeps the old tiles and says `nothing left in <name> that
  clv shows`, as going into an empty container says it. There is no empty list.
- When the reader goes into a tile whose file changed since it was opened, clv opens the file
  again first.
- A list made again belongs to its level, as a file's reload does. If the level is not on screen
  when it ends, it is dropped. An item dropped while its file opens finishes for nobody.

### Gone from disk

- A file or a directory on screen that is deleted or moved keeps its pages or its tiles, with
  `gone from disk`. Neither `r` nor auto-reload takes them away.
- A file deleted in a directory that is listed is not "gone" in this sense: the directory
  changed, and a reload drops the file's tile.

### Auto-reload

- The option `--auto-reload`, a flag that a config may set (`auto-reload = true`).
- The command `toggle-auto-reload`, bound to `R` in a file's pages and in a list, with a line in
  `?`. It says `auto-reload on` or `auto-reload off` on the status line.
- While it is on:
  - When clv sees a change, it stats the file or the directories again 500 ms later. If a stamp
    moved, the wait starts over. If not, clv reloads. So clv reloads once the writes pause. That
    is not a promise that a write has finished, but a file that an editor saves by a rename is
    seen changed, not gone.
  - `r` and the reload before a relayout (`abe33f3`) don't wait.
  - No warning shows for a change. After a reload, the status line says `reloaded`: a message,
    not a notice, so the warnings and errors still show. It goes after 1.5 s, at the next key, or
    when another message takes its place.
  - The status line's right side shows `auto`, dropped first in a narrow window.
  - A reload waits while `?`, the link labels, a number typed, or a search or filter being typed
    are on screen.
- A reload that fails says why, once, and is not tried again by itself until the stamp changes.
  `r` still tries it.

### Kept as they are

- No new dependency.
- Never a panic, and every name and message through `printable`.
- The same inside tmux and in text cells.
- Printing (`--no-pager`) doesn't look at the disk.

## Out of scope

- **The OS's file notices** (the `notify` crate: inotify, FSEvents, kqueue). ADR-0044 says why clv
  polls, and what would make notices worth it, such as lists far past 4,096 files.
- **Files that a document draws from**: a Markdown file's pictures, a Mermaid file by path, an Org
  file's includes. In the backlog:
  [A document that notices its pictures changed](../backlog.md#a-document-that-notices-its-pictures-changed).
- **Archives and pipes**: no zip or tar list, no file in one, and no pipe is looked at.
- **What a folder tile holds**: a directory shown as a folder, without `-r`, is looked at only once
  the reader goes into it.
- **Levels not on screen**: the list under a file gone into is looked at once the reader goes back
  to it, and its own check catches the change then.
- **A badge on a changed tile**. In the backlog:
  [A badge on a tile whose file changed](../backlog.md#a-badge-on-a-tile-whose-file-changed).
- **Settings for the timing**: the look once a second and the wait of 500 ms are constants.
- **A network file system's delay**: NFS and SMB cache a file's and a directory's stamps, so a
  change there can show some seconds late. clv can't change that.
- **Tiles kept in the terminal across `r`**: `r` sends every page again (`Nav::rerender`, and
  `draw` deleting what was sent), and a new level deletes the old level's images.
- **A symlink pointed at another file with the same time and size**: it is seen unchanged.

## Approach

**Files:**

- `src/source.rs` — the contract.
  - `Stamp` moves here from `src/view/mod.rs` (its type and `Stamp::of`), since a backend now takes
    one. The core keeps using it from here.
  - `Listing` gains `directories: Vec<(PathBuf, Option<Stamp>)>`: the real path of each directory
    the listing read, with its stamp taken before it was read. Empty means that nothing is looked
    at.
- `src/source/dir.rs` — `list_within` fills `directories`: the top directory, and with `-r` each
  subdirectory it reads, at most 1,024. Past that, it still lists, and sets a flag that the viewer
  says once. The archives' `tree.rs` builds `Listing::default()`, so it gives none and changes
  nothing.
- `src/view/mod.rs`
  - A list keeps how it was made: the paths given and `-r` for the top level (`Given`, from
    `Opener`'s first listing), or its container for one gone into or reached by a link.
  - `Opener::begin` and `Opener::step` take the old items, by path. `step` stats an entry before
    it opens it, and puts the old `Item` in its slot when the stamp is the same. `Item::Later` and
    `Item::File` hold an `OnDisk` with its `Stamp` already. `Opener::finish` only puts the results
    together, as now.
  - `--auto-reload` in `OPTIONS`, as a `flag` that a config may set.
- `src/view/pager.rs`
  - `Pager::watch_disk` also looks at a list's directories and at the stamps of its tiles on
    screen.
  - `Pager::reload` in a list starts making the list again through the `Reading` and `Opening`
    path that `Pager::read_on` takes to go into a container, with the list's recipe in place of
    `recurse = false`. A new `Pager::relist` builds the new list level from the result, in place
    of the old one, tied to its level id as `Pager::reloading` is.
  - `Filter` maps `given`, `last` and `chosen_before` to the new tiles by name.
  - The state of auto-reload, its own hold (a stamp and when it was seen, stat-ed again at the
    deadline, the loop waking then), the `auto` mark, and the `reloaded` message with its own
    deadline of 1.5 s.
- `src/view/commands.rs` — `Command::ToggleAutoReload` (`toggle-auto-reload`): `R` in `PAGES` and
  `LIST`, and a `Line` in both.
- `tests/it/view_pager.rs` — the pseudo-terminal tests, beside
  `r_reloads_a_file_changed_on_disk_which_the_status_line_says`.
- `tests/it/view.rs` — `auto-reload = true` from a config.
- Docs: `docs/usage.md` ("The status line", the keys, the options), `docs/architecture.md` ("A
  file changed on disk" becomes "Files and lists changed on disk"; "Commands and keymaps" gets the
  `toggle-` rule), `docs/adr/0044-…`, `CHANGELOG.md`.

**Patterns to follow:**

- `Pager::reload` and `Pager::reload_on`: a job polled each pass, the old level kept until the new
  one is ready, and `Pager::unreloaded` for a reload that failed.
- `Pager::read_on`: a container read on the main thread a step at a time, the keys that work
  meanwhile, and what an empty result says.
- The test container `Ordered` in `src/view/mod.rs`'s tests, which records the files it opens: a
  test that an unchanged file isn't opened again.
- `relayout::Hold`: a wait until a value holds still, with no IO, the loop waking when it is due.
  The stamp's hold is its own, not `Hold`, which holds a `Room`.
- `Nav::laid`: a part of the status line's right side, dropped first in a narrow window.
- `Pager::replace_with`: what a new level takes from the old one.

**Decided:**

- **Polling with `stat`, not the OS's notices.** Only the directories and the tiles on screen are
  stat-ed. ADR-0044 records it.
- **The directories come from the contract**, with their stamps. The core stats the paths that
  `Listing::directories` gives and never names `Directory`. The alternatives are rejected:
  `Container::on_disk` alone misses a subdirectory with `-r`, and paths made from the tiles' names
  miss a subdirectory with no tile, and assume that a name is a path.
- **A list is made again from its recipe**, not from one container: the top level can come from
  several paths.
- **An item is known by its path and its stamp**, never by its name alone: two files named on
  the command line can have the same name.
- **`R` and `toggle-auto-reload`**: the first command that turns a flag on and off. `toggle-`
  joins the three rules for names in "Commands and keymaps".
- **What is gone is not reloaded**, whether auto-reload is on or off.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- None.

## Tasks

One checkbox each: a session's work, and a commit that builds and passes on its own.

- [x] **A list looks at its directories and its tiles, and warns** — `Stamp` into `src/source.rs`;
  `Listing::directories` with stamps taken before each read, filled by `dir.rs` (1,024 at most for
  the whole list, said once past that); a list keeps its recipe; `Pager::watch_disk` for a list;
  ADR-0044; usage's status line, architecture's section renamed, a CHANGELOG line. *Files:*
  `src/source.rs`, `src/source/dir.rs`, `src/view/mod.rs`, `src/view/pager.rs`,
  `tests/it/view_pager.rs`, `docs/`. *Verify:* `cargo test -j 2 --bin clv dir::` (the directories
  listed, with and without `-r`, and the limit); `cargo test -j 2 --test it
  a_list_warns_of_a_file_added_to_its_directory`, `…a_list_warns_of_a_tile_rewritten_on_screen`,
  `…a_list_given_with_r_warns_of_a_file_added_to_a_subdirectory`, `…a_zips_list_is_not_looked_at`.
- [x] **`r` makes a list again** — `Opener::begin` and `step` reusing an item by path and stamp;
  `Pager::relist` through `read_on`'s path with the recipe, tied to its level; the tile, the
  filter and `--pages` by name; an empty result; a changed tile opened again when gone into;
  usage and architecture. *Files:* `src/view/mod.rs`, `src/view/pager.rs`,
  `tests/it/view_pager.rs`, `docs/`. *Verify:* `cargo test -j 2 --bin clv
  an_unchanged_file_is_not_opened_again` (with `Ordered`); `cargo test -j 2 --test it
  r_lists_a_directory_again_and_keeps_the_tile_by_name`, `…r_keeps_a_lists_filter_by_name`,
  `…r_on_a_list_emptied_keeps_its_tiles_and_says_so`,
  `…r_keeps_a_file_named_on_the_command_line_that_is_gone`,
  `…going_into_a_tile_changed_on_disk_opens_it_again`, `…esc_stops_a_list_made_again`.
- [x] **Auto-reload** — `--auto-reload`, `toggle-auto-reload` on `R`, the stamp's hold, the `auto`
  mark, the `reloaded` message, the waits; for a file's pages and for a list; usage's options and
  keys, architecture's `toggle-` rule, a CHANGELOG line. *Files:* `src/view/mod.rs`,
  `src/view/pager.rs`, `src/view/commands.rs`, `tests/it/view_pager.rs`, `tests/it/view.rs`,
  `docs/`. *Verify:* `cargo test -j 2 --test it capital_r_turns_auto_reload_on_and_off`,
  `…auto_reload_reloads_a_file_once_its_writes_pause`, `…auto_reload_reloads_a_list`,
  `…auto_reload_waits_while_a_search_is_typed`, `…config_auto_reload_starts_it_on`.

## Done

- [x] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [x] Tests added, each seen failing without the change: the ones the Tasks name.
- [ ] Manual: in kitty, run `clv -r <dir>` on a directory with a subdirectory of pictures. Copy a
  picture into the subdirectory: the warning shows within a second. Press `r`: the new tile shows,
  and the tile you were on is still chosen.
- [ ] Manual: in kitty, open a Markdown file, press `R` (`auto` shows), and save the file from an
  editor: the pages reload, `reloaded` shows, and the paragraph at the top stays.
- [ ] Manual: in a private tmux (`tmux -L`), the warning and `r` in a list work as in kitty.
- [x] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- 2026-11-25 — Ready (spec-new).
- 2026-11-25 — Codex's review folded in: a list made again from its recipe, items known by path
  and stamp, reuse decided in `Opener::step`, directory stamps through the contract, the filter
  and `--pages` by name, an empty result, the stamp's own hold, `reloaded` as a message, the limit
  of 1,024 directories, a relist tied to its level. The promise to keep unchanged tiles in the
  terminal is dropped, since `r` sends every page again. Tasks and Done written (spec-update).
- 2026-11-25 — Implementing ([notes](0016-watch-lists-on-disk-and-refresh-by-themselves-impl-20261125_1728.md)).
