# Spec-0016: Warn when a list's directory changes, and let pages and lists refresh by themselves

- **Status:** Ready
- **Date:** 2026-11-25
- **Related:** commit `abe33f3`, which this builds on (a file's pages warn when the file changes,
  and `r` reloads it); [A file changed on disk](../architecture.md#a-file-changed-on-disk),
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
config), and `R` to turn it on and off, make files and lists reload by themselves once a change
has stopped, with no warning. While it is on, the status line shows `auto`, and after a reload it
says `reloaded` for about 1.5 s. clv goes on polling with `stat`, and adds no dependency on the
OS's file notices: ADR-0044 says why.

## Requirements

### A list warns

- These lists are looked at:
  - A list of a directory: named alone on the command line, gone into from a list, or reached by
    a link. Files added and removed are noticed only in a directory.
  - Any list whose tiles are files on disk, the files named on the command line included: a
    tile's file whose content changed is noticed.
- Once a second, clv looks at these:
  - The stamp of each directory that the listing read: its modification time and size, which a
    file added, removed or renamed in it changes. Without `-r`, that is the directory itself.
    With `-r`, it is each subdirectory read too, at most 1,024.
  - The stamps of the files whose tiles are on screen, never every file of the list.
- When a directory or a tile's file changed, the status line shows the file's warning:
  `changed on disk, r reloads`, with the key bound to `rerender`, shorter in a narrow window. A
  directory that is gone shows `gone from disk`.
- An archive's list (zip, tar), a file in an archive, and a pipe are not looked at, as for a file.

### `r` in a list

- `r` lists the directory again. The old tiles stay until the new list is ready, and the status
  line says `reloading…`. Esc stops it, and the old list stays.
- The new list keeps these as they were:
  - Each file whose name and stamp are the same: its opened source, and its tiles already in the
    terminal.
  - The filter, the zoom and the count to a row.
- It opens again each file that changed, adds each new file at its place in the order, and drops
  each file that is gone.
- The reader stays on the same tile, by its name. If that file is gone, the reader goes to the
  tile that is now at its place, or the last one.
- When the reader goes into a tile whose file changed since it was opened, clv opens the file
  again first.

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
  - A change reloads the file or the list once the stamp has held still for 500 ms. So a file
    still being written is not read half-way, and a file that an editor saves by a rename is
    seen changed, not gone.
  - No warning shows for a change. After a reload, the status line says `reloaded` for about
    1.5 s.
  - The status line's right side shows `auto`, dropped first in a narrow window.
  - A reload waits while `?`, the link labels, or a number typed are on screen.
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

## Approach

**Files:**

- `src/view/pager.rs` — most of the work, as the file's case in `abe33f3` did it.
  - `Pager::watch_disk` also looks at a list's directories and at the stamps of its tiles on
    screen.
  - `Pager::reload` in a list starts a listing again, through the `Reading` and `Opening` path
    that going into a container takes, so Esc stops it and `reloading…` shows.
  - A new `Pager::relist` builds the new list level from the new listing and the old items.
  - The state of auto-reload, its wait of 500 ms, the `auto` mark and the `reloaded` notice.
- `src/view/mod.rs`
  - `Given` and `Shown::list` keep the top-level directory's `Container`, so `clv dir/` can list
    it again. Today only a folder gone into keeps one, in `Level::from`.
  - `Opener::finish` takes the old items, and keeps an `Item` whose name and stamp are the same.
    `Item::Later` and `Item::File` hold an `OnDisk` with its `Stamp` already.
  - `--auto-reload` in `OPTIONS`, as a `flag` that a config may set.
- `src/source.rs` — `Listing` gains `directories: Vec<PathBuf>`: the real paths of the directories
  that the listing read. Empty means that nothing is looked at.
- `src/source/dir.rs` — `list_within` fills `directories`: the top directory, and with `-r` each
  subdirectory it reads, at most 1,024. The archives' `tree.rs` builds `Listing::default()`, so it
  gives none and changes nothing.
- `src/view/commands.rs` — `Command::ToggleAutoReload` (`toggle-auto-reload`): `R` in `PAGES` and
  `LIST`, and a `Line` in both.
- `tests/it/view_pager.rs` — the pseudo-terminal tests, beside
  `r_reloads_a_file_changed_on_disk_which_the_status_line_says`.
- `tests/it/view.rs` — `auto-reload = true` from a config.
- Docs:
  - `docs/usage.md`: "The status line", the keys and the options.
  - `docs/architecture.md`: "A file changed on disk" becomes "Files and lists changed on disk".
    "Commands and keymaps" gets the `toggle-` rule.
  - `docs/adr/0044-…`: polling with `stat`, not the OS's notices.
  - `CHANGELOG.md`.

**Patterns to follow:**

- `Pager::reload` and `Pager::reload_on`: a job polled each pass, the old level kept until the new
  one is ready, and `Pager::unreloaded` for a reload that failed.
- `relayout::Hold`: a wait until a value holds still, with no IO, the loop waking when it is due.
- `Nav::notice` and `Moment`: a message that has the whole status line for a moment.
- `Nav::laid`: a part of the status line's right side, dropped first in a narrow window.
- `Pager::replace_with`: what a new level takes from the old one. A list takes the filter, the
  zoom, and the tile by its name.

**Decided:**

- **Polling with `stat`, not the OS's notices.** A directory's stamp is its modification time and
  size, which `Stamp::of` takes already. Only the tiles on screen are stat-ed. ADR-0044 records
  it.
- **The directories come from the contract.** The core stats the paths that `Listing::directories`
  gives and never names `Directory`. The alternatives are rejected: `Container::on_disk` alone
  misses a subdirectory with `-r`, and paths made from the tiles' names miss a subdirectory with
  no tile, and assume that a name is a path.
- **The top-level list keeps its container.**
- **`R` and `toggle-auto-reload`**: the first command that turns a flag on and off. `toggle-`
  joins the three rules for names in "Commands and keymaps".
- **What is gone is not reloaded**, whether auto-reload is on or off.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- None.

## Tasks

One checkbox each: a session's work, and a commit that builds and passes on its own.

- [ ] **Noun phrase** — what gets built. *Files:* `src/...`, `tests/it/...`. *Verify:* `cargo test -j 2 ...`, or `Manual: in kitty, press X, see Y`.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [ ] Tests added, each seen failing without the change: name them.
- [ ] Manual: …
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- 2026-11-25 — Ready (spec-new).
