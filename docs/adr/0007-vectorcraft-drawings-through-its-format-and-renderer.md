# ADR-0007: vectorcraft drawings through its format and renderer

- **Status:** Superseded by ADR-0008
- **Date:** 2026-10-30

## Context

[vectorcraft](https://github.com/storytold/vectorcraft) is a vector drawing app in Rust (MIT or
Apache-2.0), from the same authors as pdfcraft and wordcraft. It saves drawings as JSON
(`.vectorcraft`, `.drawcraft` before its rename), gzipped when the app's Use Compression is on, its
pictures after the JSON. A drawing has artboards, each a rectangle in points that an export
draws, as a PDF has pages ([vectorcraft drawings](../cli-view.md#vectorcraft-drawings)).

Its workspace splits the file format, the document model and the renderer (vello_cpu, as
wordcraft's) from an engine (the app's commands, importers for SVG, PDF, EPS and others, PDF
export) and the app. A drawing is untrusted, and vectorcraft's limits suit an editor: a picture
may decode to 2 GiB, its shared picture cache takes an eighth of the machine's memory (up to
1 GiB), and the renderer cuts a side at 32,768 pixels, but bounds no render's time.

## Decision

- **The format, document and renderer crates, not the engine**: `vectorcraft-format`,
  `-doc` and `-render`, pinned to one commit, as pdfcraft and wordcraft are. The engine and the
  importers bring PDF export, whose crates vectorcraft patches through a `[patch.crates-io]` that
  Cargo honours only in the root manifest. vectorcraft's bundled fonts are left out (3.4 MB): the
  system's stand in for them.
- **Each artboard a page**, drawn as the app's PNG export draws it (no guides or template layers,
  white where the document's background is), at the scale the viewer asks; not supersampled
  (`precise`, `supersample`), which costs about three times as long for edges a terminal's
  cells don't show.
- **Known by its first bytes, strictly**: `{`, then the key `"format"` first, saying
  `"vectorcraft"` or `"drawcraft"`, as vectorcraft's writer always puts it; not vectorcraft's own
  `sniff`, which takes any JSON naming vectorcraft in its first 256 bytes (a `package.json`).
  Asked before the PDF backend, whose `%PDF-` may be anywhere in a drawing's first 1024 bytes. A
  gzipped one is the tar backend's, which unpacks it and hands it on.
- **Parse to open, keep only the artboards' sizes**; a renderer or pool parses it again on its
  first render and keeps it, and a vectorcraft `Renderer` for each thread that draws (single
  threaded: the pool's threads draw the pages side by side), whose caches make the next render
  of a page quicker.
- **Limits of clv's own**: a file of at most 256 MiB, a page of at most 16,384 pixels a side and 64
  million in all, the shared picture cache at 64 MiB, and a render stopped after 10 seconds, through
  vectorcraft's cancel token (checked at each object), by a watchdog thread.

## Evidence

Apple M3 Max, macOS 26.6.2, vectorcraft at `1660cd1b41be81a7d00cdf0d7ce889305ed63b44`, a release
build (`lto = "thin"`, `codegen-units = 1`) of a test program calling
`vectorcraft_format::load_file` and `Renderer::render_region_with` at scale 2, single-threaded,
peak memory from `getrusage`, on vectorcraft's `examples/` and on drawings it made itself (20,000
rectangles, ellipses and stars on a 1000 pt artboard; 2000 artboards).

| Drawing | File | Parse | Draw an artboard (first, again) | Peak memory |
|---|---|---|---|---|
| bracket-drawing | 129 KB | 3.4 ms | 117 ms, 1.6 ms | 71 MB |
| feature-sheet, 4 artboards | 184 KB | 0.9 ms | 1–33 ms | 61 MB |
| neon-drive, 2160 × 2700 px | 364 KB | 0.7 ms | 167 ms, 15 ms | 180 MB |
| still-life, 2800 × 1800 px | 56 KB | 0.2 ms | 131 ms, 73 ms | 407 MB |
| 20,000 shapes | 12 MB | 47 ms | 167 ms, 122 ms | 504 MB |
| 20,000 shapes, gzipped | 1.9 MB | 64 ms | the same | 528 MB |
| 2000 artboards | 284 KB | 0.6 ms | 0.1 ms | 6 MB |

neon-drive drawn as the export does (`precise`, `supersample`): 463 ms, 451 MB. A gzipped file
padded to 200 MiB of JSON is refused after 189 ms, having held 405 MB. A binary of the format,
document and renderer crates is 17.3 MB, against 1.5 MB for one with the `image` crate's decoders
alone; 20.7 MB with the bundled fonts. `cargo build --release --locked --no-default-features
--features vector` makes a `clv` of 17.2 MB, against 1.0 MB with `--no-default-features`: 16.2 MB
more, some of it (vello_cpu, skrifa, harfrust, the `image` crate's decoders) built for the Word
and image backends anyway.

## Alternatives considered

- **The engine**: what the app opens files with, and its importers (DXF, EMF, EPS, legacy AI)
  for free, but with the patches above, and formats clv has backends for (SVG, PDF) or doesn't
  mean to read.
- **vectorcraft's `sniff`**: looser than its own writer, so JSON that merely names it would be
  taken, and then fail to open, rather than be passed on.
- **Keep the parsed drawing**: no second parse when shown, but every drawing of a list would hold
  its document while idle, and parsing costs milliseconds.
- **A fresh renderer for each render**: holds nothing between renders, but a page drawn again (at
  another zoom) costs as much as the first: neon-drive 132 ms against 15.
- **No time limit, as for SVG**: resvg's costs can be estimated before a render (ADR-0003);
  vectorcraft's effects, blends and live shapes can't from outside, but it can be told to stop.
- **The bundled fonts**: truer text for drawings set in vectorcraft's default fonts, 3.4 MB more.

## Consequences

- **Positive:** vectorcraft's drawings, as its export draws them, quick to open; a slow page is
  given up on.
- **Accepted:**
  - About 16 MB more binary (some of it shared with the Word backend: vello_cpu, skrifa,
    harfrust).
  - What vectorcraft holds while it draws a page is its own: a few hundred MB for a complex one at
    a large size, kept in its renderers' caches while the drawing is shown.
  - A picture a drawing embeds may decode to vectorcraft's 2 GiB before clv's limits see it.
  - Text set in vectorcraft's own fonts shows in the system's.
  - The pin must move as vectorcraft's format does: a file saved in a newer version is refused
    ("too new"), with what vectorcraft says.
- **Follow-ups:** the importers that need no engine (DXF, EMF and WMF) as files of their own; the
  preview a file carries (at most 256 pixels) for a list's tiles.
- **Revisit when:** vectorcraft's engine no longer needs patches; or its limits become settable
  (a picture's decode).
