# ADR-0037: What is beside a held file is read from its holder, a picture measured from its first 64 KiB

- **Status:** Accepted
- **Date:** 2026-11-15

## Context

A document shows the pictures it refers to only from near it
([ADR-0013](0013-a-documents-pictures-are-drawn-by-the-backends.md)), and near meant its directory:
a document held in another file (a README in a zip of a repository, a chapter of an epub) had
none, so its pictures showed as their alt text. ADR-0013 left that as its follow-up, "a way for
what is held in an archive to reach what is beside it".

A held file reaches the viewer as an `Input` with its first bytes and two ways to read the rest
from its holder: whole (`Again`), or whole within a bound and not at all past it (`Bounded`)
([Sources](../architecture.md#sources-srcsourcers)). Neither says where the file is in its holder,
nor reads another file there.

A document is laid out around a stand-in for each picture, which takes the picture's size, and is
laid out twice, to open and on its first render; a list lays out every document whose tile is
wanted. A picture on disk gives its size from its header, read through a seekable file. A held
picture was read whole for it (`Image::held`): fine for a list of an archive's pictures, each
drawn at once anyway, but a document of a hundred pictures would unpack them all to open, and a
book ([Spec-0012](../specs/0012-show-an-epub-as-a-book.md)) on every relayout.

## Decision

1. **The contract lets a held file say what is beside it.** An `Input` held in another may carry
   a `Near`, a function from a path from the holder's top to a held `Input` of its own, and the
   file's own path there (`Input::held_near`, `Input::near`). It is the holder's to give: zip's and
   tar's containers do, for every file opened from them. A file compressed alone, a pipe, and what
   one backend hands another (a spreadsheet's PDF, a diagram's SVG) have nothing beside them.
2. **The reader brings a path to the holder's top; the holder finds only what it lists.** The
   documents' pictures (`picture::from_the_top`) resolve a picture's path against the document's
   own, each `..` taken with the name before it, and leave out one that leads above the top,
   without asking. The archives look a path up in the tree their listing is made from
   (`Tree::file`), where `..` is only a name and a hidden name was never added: so a path can't
   lead out of an archive whatever the reader does, and nothing beside the archive on disk is ever
   read for a file in it. `picture::local`'s checks hold as on disk: a URL, an absolute path or
   `%2F` is no path from the document.
3. **What `Near` gives is opened as a file on disk is**, by the first backend that draws
   pictures and opens it by its first bytes. The documents' backend names no archive, and an
   archive no document.
4. **It rides with the file.** The documents' backend keeps it with where the bytes are
   (`Stored::Held`), so a relayout reads pictures as the first layout did; a list's file opened
   late keeps it too (`later::Reopen::Held`).
5. **A held file may give its first bytes alone** (`Start`, `Input::start`): as many as asked for,
   or all of a shorter file, unpacking no more. `Bounded` can't serve: it gives a file whole or
   not at all. Zip's and tar's files give it.
6. **A held picture is measured from its first 64 KiB** (`image::MAX_HEADER`), never unpacked
   whole for its size, where its holder gives them. They are read as the file would be up to where
   they end (`image::First`): a read or a seek to there or past it, or from the file's end, fails
   and is noted, and the header is then read from the whole file, so no decoder's answer is taken
   from a file it believed ended there. A JPEG alone is given its first bytes as if they were the
   file: the `image` crate reads a JPEG whole before parsing it, then walks its segments in order
   to its first scan, for its size and its EXIF alike, so what it makes of the first bytes, when it
   makes anything, is what it makes of the file; when it makes nothing, the file is read whole.

## Evidence

Apple M3 Max, macOS 26.6.2, `image` 0.25.10, [`0037-measure/`](0037-measure/Cargo.toml):
`cargo build --release` there, then `target/release/measure <directories>`, which reads each
picture's size and orientation as clv's `header` does, through a file that notes the furthest byte
read; for a JPEG, which the crate reads whole, it walks the segments to the first scan instead.

| Pictures | Format | How many | Header within 64 KiB | Furthest read |
|---|---|---|---|---|
| 22 books unpacked: Project Gutenberg's 11, 74, 84, 98, 1342, 1400, 1661, 2701, 5200 and 28885, each as EPUB 2 and EPUB 3 with images, and Standard Ebooks' *Alice's Adventures in Wonderland* and *Frankenstein* | JPEG | 710 | 710 | 623 bytes |
| the same | PNG | 262 | 262 | 1,089 bytes |
| 10 checkouts' pictures (aws-icons-for-plantuml, clojure-polylith-realworld-example-app, Clerk, Emmy, FlowStorm, d3-graphviz, highlight.js, Biff, Edge, clv's own) | PNG | 1,027 | 1,027 | 25,454 bytes |
| the same | JPEG | 2 | 2 | 13,084 bytes |

Of the books' 710 JPEGs, 528 are larger than 64 KiB, up to 843,147 bytes: read through the
`image` crate's decoder as a file, every one is read to its end before its header is parsed. That
is why a JPEG is given its first bytes as the file.

No photograph straight from a camera was measured: its EXIF, with its thumbnail, is one segment,
at most 64 KiB, and a colour profile may add segments of its own. `a_held_picture_whose_header_comes_later_is_read_whole`
(`src/source/image.rs`) writes one whose EXIF starts past 64 KiB, and it is turned as that EXIF
says, read whole; `a_held_picture_is_measured_from_its_first_bytes` holds nine formats to being
measured without a whole read, and a TIFF, whose directory the encoder puts at the file's end, to
being read whole.

End to end, `a_documents_pictures_are_measured_without_being_read_whole` (`tests/it/zip.rs`): a
document in a zip beside a 10 MB picture whose bytes a megabyte in fail the archive's checksum
still gives the picture its place on the page.

## Alternatives considered

- **The documents' backend reading the zip itself**, for an epub's chapters: it would name the
  archive's library, or the epub; and a README in a tar would still have no pictures.
- **A directory unpacked to disk, and the document opened there**: nothing is written out from an
  archive today ([Archives](../usage.md#archives)), a name in one being only ever a name; and a
  picture's path could then be made to lead anywhere the temporary directory does.
- **The holder resolving `..`**: each holder would do it again, and differently. The reader
  resolves once, and the holders' lookup refuses what isn't a name they list, which is all the
  safety there is to have.
- **Measuring through `Bounded` at 64 KiB**, as the spec first said: it gives nothing of a larger
  file, and three quarters of the books' JPEGs are larger.
- **A size read by clv's own parser of each format's header**: one more reader of untrusted
  bytes for each of thirteen formats, to agree with the decoder's on size and orientation both.
- **Giving every format its first bytes as the file**, as a JPEG is: a decoder that scans to the
  file's end for a chunk (WebP's EXIF, which comes after the pixels) would then answer without
  it, and a picture would be measured unturned and drawn turned.

## Consequences

- **Positive:** a README in a zip or a tar shows its screenshots, and an epub's chapters can show
  their pictures; opening a document in an archive unpacks no picture whole, and neither does
  laying it out again.
- **Accepted:**
  - A picture under a hidden name (`.github/demo.png`) isn't found in an archive, where on disk it
    would be: the archives' trees never held hidden names.
  - Each time a picture is measured or drawn, the holder is asked for it again: its first 1 KiB to
    choose a backend, its first 64 KiB to measure, the whole to draw. In a compressed tar past
    what its temporary file holds, each is unpacked from the archive's start.
  - A picture whose header is past its first 64 KiB is unpacked twice to be measured, its first
    bytes and then the whole.
  - A document's links to other files in an archive are still not followed
    ([Spec-0011](../specs/0011-follow-a-documents-links.md)'s Out of scope), though `Near` would
    serve them.
- **Follow-ups:** an epub as a book gives its chapters a `Near` of its own
  ([Spec-0012](../specs/0012-show-an-epub-as-a-book.md)).
- **Revisit when:** the `image` crate's JPEG decoder stops reading its input whole, or reads past
  the first scan for a picture's size or orientation (the two tests above fail then); or camera
  photographs in documents turn out to be common and slow.
