# ADR-0042: A PDF's tile is the file itself, its pages measured by hayro-syntax

- **Status:** Accepted
- **Date:** 2026-11-22

## Context

A list opens a PDF when its tile is wanted
([ADR-0019](0019-a-list-opens-its-files-when-their-tiles-are-wanted.md)). Opening a PDF did three
things before its first render:

1. pdfcraft's `display_bytes_for` parsed the whole file to make the display copy. The copy draws
   the comments that have no appearance of their own, and the form fields that the file leaves to
   the viewer. To decide whether a copy is needed, it lists the file's signatures, which reads
   every object.
2. pdfcraft's `inspect` measured each page through hayro-syntax. Then it read every page's
   annotations, links, bookmarks and fields.
3. The renderer parsed the file again for its first render.

A tile shows page 1 and needs only the pages' sizes. pdfcraft has no call that gives the sizes
without the pass over the annotations. A book's tile already lays out only its first page
([ADR-0041](0041-a-books-tile-is-its-first-page-alone.md)), through `Backend::open_tile` and
`Source::tile`.

## Decision

- The PDF backend opens a tile with `Pdf::tile`. It renders the file itself, with no display copy.
- It measures each page's size, crop box and turn through hayro-syntax, as `inspect` measures them,
  and reads no annotations. Its links have the pages and no link annotations.
- clv depends on `hayro-syntax` directly, at the version that pdfcraft's `[patch.crates-io]` gives.
  It builds nothing that pdfcraft didn't build already. Only `src/source/pdfcraft.rs` names it.
- A file that hayro-syntax can't read, such as one that needs a password, opens whole, so its
  error is the same as before.
- When the reader goes into the tile, the pager asks for a relayout, though a PDF has no relayout
  setting. `Pdf::relayout` opens the file whole. Labels that `f` asked for meanwhile wait for the
  whole file.

## Evidence

clv 0.6.0 (before) and this change (after), release builds, macOS 26.6.2, Apple M3 Max. The list
is a real directory of 107 PDFs, 863 MB in all, printed as a list:

```sh
/usr/bin/time -l clv --protocol kitty ~/books/*.pdf > /dev/null
```

| | Time | CPU time | Peak memory |
|---|---|---|---|
| Before | 4.40 s | 4.32 s | 1,516 MB |
| After | 1.78 s | 1.53 s | 1,267 MB |

Three runs each; the table shows the median. Before, `sample` of the main thread put 35% in
`display_bytes_for`, most of it in `pdfcraft_sign`'s listing of signatures, 26% in `inspect`, and
37% in the render of page 1. The 107 tiles were the same PNGs before and after, byte for byte, and
the warnings were the same.

## Alternatives considered

- **Skip the display copy only**: about 35% quicker, with no new dependency. But `inspect` would
  still read every page's annotations for sizes alone.
- **A call in pdfcraft that gives the pages' sizes**: the cleanest. But it is a change to pdfcraft
  first, and the pin then moves. Worth asking for: this backend would then name hayro-syntax no
  more.
- **Render page 1 to learn its size**: a render at a known scale gives the size in pixels, rounded.
  It costs a render more, and a page's size must be known before its first render.
- **Keep opening the whole file**: a tile of a PDF with a comment that has no appearance would
  show the comment. Of 107 real PDFs, none had a tile that changed.

## Consequences

- **Positive:** a list of PDFs shows its tiles more than twice as soon. A PDF gone into costs one
  whole open, as before.
- **Accepted:**
  - A tile shows no comment without an appearance, no form field that the file leaves to the
    viewer, and no signature's status. The tile of a dynamic XFA form shows its placeholder page.
  - clv names a crate that pdfcraft brings, and its version moves with pdfcraft's pin.
  - A tile still holds the whole file in memory, as an opened PDF does.
- **Follow-ups:** ask pdfcraft for a call that measures the pages without the annotations.
- **Revisit when:** pdfcraft gives such a call, or makes its display copy without reading every
  object.
