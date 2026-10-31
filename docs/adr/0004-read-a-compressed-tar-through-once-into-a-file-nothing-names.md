# ADR-0004: Read a compressed tar through once, into a file nothing names

- **Status:** Accepted
- **Date:** 2026-10-30

## Context

A zip has a central directory: its files are listed from it and each is read on its own, so
nothing of it is unpacked ahead ([Archives](../cli-view.md#archives)). A compressed tar
(`.tar.gz`, `.tar.bz2`, `.tar.xz`) has neither: its files are found only by unpacking it from the
start, and a file in the middle only by unpacking everything before it. And the viewer reads each
file of a list more than once, in name order: whole, to open it (an image's backend for its
header, a PDF's to parse it), then again for each render of its tile, while the files come in
the archive in whatever order `tar` found them (on APFS, `photos/img119.png`, `img125.png`, …).

Reading an archive through also takes time, from a fraction of a second to minutes, and the
pager used to list a container on its own thread, frozen until it was done.

## Decision

- **Read through once, on a thread of its own** (`Container::ready`), the pager saying how far it
  has got on the status line (on stderr, for an archive on the command line), and Esc stopping it
  (`Container::release`, which lets go of what it read). Then the files are opened a few at a time, in the order the container says
  is quickest (`Listing::quickest`: as they come in the archive), again with progress and Esc. No
  question is asked before reading, however large the archive.
- **That pass unpacks the archive into a temporary file that nothing names** (`Spill` in
  `src/source/tar.rs`): created in the temporary directory, readable by the user only, and removed
  at once, so the system frees it when the last handle closes, however clv ends. Files are read
  from it at their place (`pread`), in any order, by any render thread.
- **At most 4 GiB go there** (`MAX_SPILL`). A file past them, or every file when no temporary file
  can be had, is read by one decompressor kept going forward through the archive, which starts
  over only for a file before where it is.
- **Leaving the archive, or Esc while going into it, lets go of it all** (`Container::release`):
  the file, the decompressor and the list, which going back in reads through again. Otherwise they go with the last thing that
  can read the archive.
- A plain tar is read where it is, nothing unpacked.

## Evidence

Apple M3 Max, macOS 26.6.2; flate2 1.1.10 (zlib-rs), bzip2 0.6.1 (libbz2-rs-sys), lzma-rust2
0.23.0, the decoders the tar backend builds.

**How fast each unpacks**, in a release build: 256 MiB of xorshift noise (as photos and PDFs
are, already compressed) and of numbered lines of text, compressed at each tool's default level,
then read through in 64 KiB reads.

| Unpacked MiB a second | gzip -6 | bzip2 -9 | xz -6 |
|---|---|---|---|
| noise | 2,679 | 27 | 423 |
| text | 2,520 | 111 | 261 |

**What reading a list of an archive's files costs**: 200 PNGs of noise, 1000 × 1000 (573 MB),
made with Python's `zlib`, archived with macOS's `tar -cf`, `-czf` and `-cJf`, then
`/usr/bin/time -l target/debug/clv photos.tar.xz -r --protocol kitty --zoom 8 > /dev/null`
(a dev build: clv at `opt-level` 1, its dependencies at 2), which opens and draws every tile.

| Files read with | tar.gz | tar.xz | Peak memory |
|---|---|---|---|
| the decompressor going forward, 256 MiB of files kept in memory | 12.9 s | 195 s | 278–292 MB |
| the file nothing names | 3.1 s | 4.7 s | 24–32 MB |

The same pictures in a directory took 2.9 s (23 MB), and in a plain tar 2.7 s (25 MB). The
first design kept files in memory: past the 256 MiB it held, nearly every tile, in name order,
was before the decompressor in archive order, and it started over, unpacking up to the whole
archive again for each.

## Alternatives considered

- **Ask before reading a large archive:** the cost is time only, nothing stays once it is done,
  and progress with Esc lets the reader decide while seeing how long it takes. Printing, with no
  pager, has nobody to answer.
- **Only a decompressor going forward, or several at different places, with files kept in
  memory:** measured above. More decompressors would start over less often but still would, and
  each xz decoder takes up to its dictionary's size (65 MiB for `xz -9`).
- **Unpack into a temporary directory of named files:** a crash or `kill -9` leaves them behind,
  and the archive's names would become names on disk.
- **Points to start unpacking from, kept as the pass goes (zlib's `zran`):** needs priming a
  decoder with a window and a bit offset, which flate2 doesn't offer, and a way of its own for each
  format.
- **List the files in the archive's order:** a list is in name order everywhere else.

## Consequences

- **Positive:** an archive's files read in any order at the speed of a directory's, after one
  pass; memory stays small; the pager no longer freezes while a container is listed, a large directory
  included (one file being opened still holds it up while it opens).
- **Accepted:**
  - **The disk:** up to 4 GiB of the temporary directory per archive gone into, while the reader
    is in it, and for the whole session for an archive given on the command line.
  - **Past 4 GiB unpacked, or with no temporary directory to write to,** reading out of order
    starts over for each file: slow, above all for xz and bzip2.
  - **bzip2 is slow to read through:** about 40 s for a gigabyte of photos here.
  - **Going back into an archive reads it through again.**
  - **A crash between making the file and removing it**, a few microseconds, would leave an empty
    `.clv-tar-<pid>-<n>` in the temporary directory.
- **Follow-ups:** zstd (`.tar.zst`), left out as for zip, has a decoder in Rust (ruzstd).
- **Revisit when:** archives commonly carry an index of where to start unpacking (xz's blocks,
  zstd's seekable format), or a Rust decoder can start from a saved point.
