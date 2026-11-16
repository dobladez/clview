# Spec-0012: implementation notes (2026-11-15)

Notes made while implementing [Spec-0012](0012-show-an-epub-as-a-book.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 1: pictures beside a held file

**Changed from the plan**

- **`Bounded` gives no prefix**, so "measured through its `Bounded` (64 KiB)" can't measure a
  larger picture. The contract got a third reader, `Start` (a held file's first bytes alone,
  `Input::starting`, `Input::start`), which zip and tar give. ADR-0037 says so.
- **The `image` crate reads a JPEG whole before parsing it** (528 of the 710 JPEGs of 22 real
  books are past 64 KiB). So a JPEG is given its first 64 KiB as if they were the file, and read
  whole when that fails; every other format through `image::First`, which notes a read past them.
  Measured with `docs/adr/0037-measure/`: every header of 1,999 real pictures is within 26 KiB,
  which answers the spec's open question on 64 KiB for books' pictures. A camera's photograph was
  not measured, only written by a test.
- **The held input is a variant with named fields** (`Rest::Held { again, bounded, start, beside }`),
  and `Beside` a tuple behind an `Arc`, not a struct: the no-backend and `pdf,svg` builds, never
  built locally, then need no dead-code expectation on fields, and `Input` stays under clippy's 128
  bytes for an `Err`. `Input::held` got an expectation: `later.rs` no longer calls it. Those two
  builds are still to be run by CI (`gh workflow run CI`).
- **Hidden names** (`.github/demo.png`) aren't found in an archive: its `Tree` never held them.
  In usage's Archives and the ADR's Accepted.
- **Tar's head** for a file past the kept heads is now its first 1,024 bytes (`Shared::first`),
  not the file read whole and cut: a file past 512 MiB is refused when read, not when opened.
- **The ADR is 0037**: Spec-0013's branch had taken 0036.
- Files beyond the task's list: `docs/adr/0037-measure/` (the measure), ADR-0013 and the ADR
  index (its status), `docs/backlog.md`, `CHANGELOG.md`.

**Tests, each seen failing** with its fix reverted (the file copied aside, changed, the test run,
copied back and compared):

| Reverted | Failed |
|---|---|
| `from_the_top` no longer refusing a `..` above the top | `picture::tests::a_held_documents_picture_is_a_path_from_its_holders_top`, `…a_held_documents_pictures_are_read_from_its_holder` |
| `Image::held` never asking for the first bytes | `image::tests::a_held_picture_is_measured_from_its_first_bytes` |
| a JPEG read as the other formats are | the same |
| no whole read once the first bytes are passed | `image::tests::a_held_picture_whose_header_comes_later_is_read_whole` |
| zip's `Directory::open` without `held_near` | `zip::tests::a_file_in_an_archive_gives_its_first_bytes_and_what_is_beside_it` |
| tar's the same | `tar::tests::a_file_in_an_archive_gives_its_first_bytes_and_what_is_beside_it` |
| `Reopen::input` dropping what is beside the file | `later::tests::a_file_is_closed_until_wanted_unless_it_couldnt_be_opened_again` |
| `Document::beside` giving nowhere for a held document | `zip::a_documents_pictures_are_drawn_from_its_zip`, `tar::a_documents_pictures_are_drawn_from_its_tar` |
| zip's held file without its `Start` | `zip::a_documents_pictures_are_measured_without_being_read_whole` |

The tests' own first mistakes, for whoever reads them: a GIF of one frame of noise is 26 KB (its
256 colours), so the test writes four; a name seen twice is left out once (`seen`), so three
names above the top count two; a document opened from a list says `page 1/1`, not its name's
count; and a zip whose picture fails its checksum prints that picture's tile as an error, so the
print exits unsuccessfully while the document's tile is drawn.

An existing test changed with the behaviour: `wordcraft::what_has_no_directory_has_no_pictures`
held a document in a zip to saying nothing of a picture it couldn't reach; it now says one is left
out, as a document on disk does, and still draws nothing from beside the archive on disk.

Gates on 2026-11-16: `cargo fmt --check`, `cargo clippy --all-targets -j 2 -- -D warnings`,
`cargo test -j 2` (565 unit, 13 docs, 279 integration), all passing.

## For task 2, found while waiting

- rbook 0.7.13's `Archive` trait is private: the worker feeds it a zip in memory, as planned.
- 22 real epubs (Gutenberg ×20, Standard Ebooks ×2) are in the session's scratchpad, not the repo.
- The last nightly CI run (2026-11-15, schedule) failed in under two minutes: worth a look.
