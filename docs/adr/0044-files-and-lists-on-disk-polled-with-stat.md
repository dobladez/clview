# ADR-0044: Files and lists on disk polled with `stat`, not watched through the OS's notices

- **Status:** Accepted
- **Date:** 2026-11-25

## Context

clv warns when what it shows changed on disk since it read it
([Files and lists changed on disk](../architecture.md#files-and-lists-changed-on-disk)). A file's
pages warn when the file changes. A list warns when a file is added to its directory, removed or
renamed, or when the file of a tile on screen changes.
[Spec-0016](../specs/0016-watch-lists-on-disk-and-refresh-by-themselves.md) adds the lists.

clv can learn of a change in two ways. It can ask the OS to tell it: inotify on Linux, FSEvents or
kqueue on macOS, through a crate such as `notify`. Or it can look itself, with a `stat` of each
path, at a fixed interval. The pager already runs a loop that wakes at least every 200 ms, so a
look once a second costs no new thread.

A list holds at most 4,096 files ([Limits](../usage.md#limits)). With `-r`, a list can read many
directories.

## Decision

clv polls. Once a second, the pager stats what is on screen: the file of a file's pages, or a
list's directories and the files of its tiles on screen. It compares each stamp (the modification
time and the size) with the stamp taken before it was read. clv looks at 1,024 directories at most
for a list, the first ones read, and says so when a list has more.

clv adds no dependency for this.

## Evidence

The cost of one look, on an Apple M3 Max, macOS 26.6.2, APFS, Rust 1.95.0, best of 20 looks:

| What one look stats | Time |
|---|---|
| 1 directory and 24 tiles' files | 0.03 ms |
| 1,024 directories and 24 tiles' files | 1.3 ms |
| 1,024 directories and 4,096 files | 6.7 ms |

clv stats the tiles on screen, not every file of the list, so the second row is its worst case:
1.3 ms each second, on the main thread. To measure it again:

```sh
rustc -O docs/adr/0044-measure/stat.rs -o /tmp/stat
/tmp/stat 1 24 && /tmp/stat 1024 24 && /tmp/stat 1024 4096
```

## Alternatives considered

- **The OS's notices, through `notify`**: a new dependency, with a backend for each OS and their
  differences to test. inotify takes a watch for each directory, under a limit per user that other
  programs share. FSEvents delivers its events late, in batches, and coalesces them. For what clv
  looks at, the poll costs little, and it behaves the same everywhere.
- **Every file of a list, not only the tiles on screen**: up to 4,096 stats a second for changes
  that the reader can't see. A file off screen that changed is opened again when the list is made
  again (`r`), or when the reader goes into it.
- **No limit on the directories**: `-r` on a home directory reads thousands of them, and a look at
  each one every second would cost more than the rest of the loop.

## Consequences

- **Positive:** no dependency, and the same behaviour on Linux and macOS. The stamp is the one
  test, for a file and for a directory alike.
- **Accepted:** a change shows up to a second late. A change that keeps the time and the size the
  same is not seen (a symlink pointed at another file with the same time and size). On a network
  file system, the client caches the stamps, so a change can show some seconds late. Past 1,024
  directories, a change in the others is not seen.
- **Revisit when:** lists hold far more than 4,096 files, or a reader needs to see a change sooner
  than a second.
