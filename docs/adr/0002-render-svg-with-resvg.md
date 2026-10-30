# ADR-0002: Render SVG with resvg

- **Status:** Accepted
- **Date:** 2026-10-30

## Context

clv shows PDFs, images and zip archives; SVG drawings were wanted too, one page each, rendered at
the size shown so they stay sharp ([Which files it shows](../cli-view.md#which-files-it-shows)).
The project's rules constrain the library: Rust only, MIT or Apache-2.0 compatible, a backend's
library named only in its own files, nothing read but the file shown, and a source holding
little while idle, since a list of hundreds of files keeps them all open
([`source/joined.rs`](../../src/source/joined.rs)). A drawing is untrusted input like any file.

## Decision

The `svg` feature (on by default) is `src/source/svg.rs`, which renders drawings with resvg
0.49.0: usvg parses a drawing into a tree, resvg paints the tree with tiny-skia. Its features are
`text`, `system-fonts`, `memmap-fonts` and `raster-images`; not `svgz`:

- **svgz is clv's own.** usvg's gunzip (`usvg::decompress_svgz`) reads to the end with no cap, so
  clv unpacks an svgz itself through flate2, which the always-on `png` crate already builds, to
  at most 64 MiB. A gzip file is a drawing when what its first 1024 bytes unpack to is one.
- **clv parses the XML.** It parses with roxmltree (usvg's own parser, re-exported) under options
  of its own and hands the document to `usvg::Tree::from_xmltree`, so it can count what a
  drawing will cost first ([ADR-0003](0003-bound-what-a-drawing-costs-before-usvg-and-resvg-spend-it.md)).
- **Nothing outside the file is read.** usvg's default resolver reads any file an `<image>`
  names; clv's refuses every link that isn't a `data:` URL, and the page says how many links
  weren't followed. usvg follows no other link (`use`, paint and CSS references are to `#id`
  only) and reads no network.
- **Text is drawn with the system's fonts**, found once per process by the first drawing that has
  text (fontdb reads the names in every font file), with the generic families (serif,
  sans-serif, monospace…) pointed at fonts the system has, since fontdb's defaults (Times New
  Roman, Arial…) are often missing on Linux. A drawing without text never looks.
- **A drawing scales** (`Source::natural` is false): fitted to the window as a PDF's page is. Its
  size is usvg's: its own, its `viewBox`'s, or, saying neither, as far as what it draws reaches
  from its origin, 100 × 100 when it draws nothing.
- **It holds its bytes' place, not its tree**, as an image holds its header: opening parses once,
  to know the drawing opens and its size, and a renderer or pool parses it again on its first
  render and keeps the tree until it goes.
- **SVG is asked first** among the backends: it goes by how a file starts, while a PDF's header
  may be anywhere in the first 1024 bytes, which a drawing's text may hold.

## Evidence

Measured 2026-10-30 on an Apple M3 Max (14 cores, 36 GiB), macOS 26.6.2, Rust 1.95.0, at commit
136450e plus this change, the machine otherwise lightly used.

**Licences.** resvg and usvg are `Apache-2.0 OR MIT`; tiny-skia is `BSD-3-Clause`. Everything
the feature adds is permissive: MIT, Apache-2.0, BSD-2-Clause (arrayref), BSD-3-Clause, Zlib
(slotmap), Unlicense-or-MIT; on Linux, fontconfig-parser is MIT. Nothing GPL, LGPL or MPL. Again
with `cargo tree --locked -e normal --no-default-features --features svg --prefix none -f '{p}|{l}'`
(and `--target x86_64-unknown-linux-musl`).

**What it adds.** 21 crates join `Cargo.lock`: resvg, usvg, tiny-skia, tiny-skia-path,
roxmltree, svgtypes, simplecss, fontdb, fontconfig-parser, memmap2, imagesize, data-url,
strict-num, float-cmp, arrayref, rgb, slotmap, unicode-script, unicode-vo, xmlwriter and
pico-args (which resvg depends on for its command line). Text shaping (harfrust, skrifa), kurbo,
flate2, the GIF, WebP and JPEG decoders and bytemuck were in the default build already.

**Size and clean build time**, release (`lto = "thin"`, `codegen-units = 1`), each in an empty
target directory: `CARGO_TARGET_DIR=<empty> cargo build --locked --release -j 2 <features>`,
then `stat -f %z <dir>/release/clv`.

| Features | Clean build | `clv` |
|---|---|---|
| `--no-default-features` (no backend) | 10 s | 981,888 bytes |
| `--no-default-features --features svg` | 42 s | 3,993,616 bytes |
| `--no-default-features --features pdf,image,zip` (the default before) | 328 s | 22,875,600 bytes |
| default (`pdf`, `image`, `svg`, `zip`) | 345 s | 25,509,152 bytes |

In the default build the feature costs 2.6 MB (11.5%) and 17 seconds, less than alone: text
shaping and the picture decoders are shared. The build times are to about ten seconds; other
work ran on the machine meanwhile.

**Finding the fonts** costs about 14 ms here (about 360 font files in `/System/Library/Fonts`,
`/Library/Fonts` and `~/Library/Fonts`): the mean of 20 runs of
`env -u TMUX TERM=dumb clv <file> --protocol kitty >/dev/null`, with the release build above,
was 6.2 ms for a 300 × 200 drawing of a rectangle and 20.6 ms for the same with one word of text.

## Alternatives considered

- **librsvg** (crates.io `librsvg` 2.63.4, GNOME's): LGPL-2.1-or-later, and it draws through
  cairo and pango, which are C libraries: against the rule of Rust only, and the licence clv
  would rather not carry.
- **nanosvg** through a binding (`nsvg`): C, and it has no text, filters, masks or embedded
  pictures.
- **usvg's tree painted by vello_cpu**, which the PDF backend already builds (through hayro): no
  maintained bridge from a usvg tree to vello_cpu exists (vello_svg is for vello, on the GPU), so
  clv would write and keep one; resvg is that bridge for tiny-skia.
- **resvg's own svgz:** its gunzip has no cap, so a few kilobytes can unpack without end.
- **Fonts loaded at every start:** 14 ms on every run, a PDF's or a photo's too, for nothing.
  **A font of clv's own in the binary:** megabytes more for every build, and a drawing would
  not get the fonts it names that the system has. **No text:** most diagrams' labels gone.

## Consequences

- **Positive:** drawings, svgz and the SVGs in directories and archives show, sharp at any zoom,
  in Rust, with the system's fonts.
- **Accepted:**
  - 2.6 MB of binary and 17 seconds of clean build in the default build (3 MB and 32 seconds
    alone).
  - resvg and usvg bound little of what a drawing costs; clv bounds it instead, by counting the
    way they work ([ADR-0003](0003-bound-what-a-drawing-costs-before-usvg-and-resvg-spend-it.md)).
  - A render once begun can't be stopped: resvg has no cancellation, so the drawing renders on
    `source/threads.rs`'s threads as a picture decodes.
  - A drawing in UTF-16, or another encoding than UTF-8, is refused: usvg reads UTF-8 only.
  - A drawing is parsed twice, to open and to render: a few milliseconds for most, and the tree,
    which can be several times the file, isn't held for every file in a list.
- **Revisit when:** resvg gains a render budget, cancellation or limits of its own; a usvg front
  end for vello_cpu appears; or a drawing format comes along that resvg doesn't read.
