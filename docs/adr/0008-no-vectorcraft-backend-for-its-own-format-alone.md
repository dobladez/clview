# ADR-0008: No vectorcraft backend for its own format alone

- **Status:** Accepted
- **Date:** 2026-10-31

## Context

[ADR-0007](0007-vectorcraft-drawings-through-its-format-and-renderer.md) added a backend for
[vectorcraft](https://github.com/storytold/vectorcraft)'s drawings (commit `07d3719`), through
its format, document and renderer crates and not its engine, whose PDF export needs patches. That
shows one kind of file, `.vectorcraft` (and `.drawcraft`, its name before), which only vectorcraft
makes. What else vectorcraft reads, it reads through its engine or through importer crates of
their own.

## Decision

- **No vectorcraft backend.** It is removed in the commit after `07d3719`, which holds the code,
  tests and docs should it come back.

## Evidence

vectorcraft at `1660cd1b41be81a7d00cdf0d7ce889305ed63b44`, its formats as the engine lists them
(`read: true` in `crates/engine/src/cmd/fileio/formats.rs`), and each importer crate's
`[dependencies]`:

| Formats | Read through | In clv |
|---|---|---|
| vectorcraft, drawcraft, vctemplate | `vectorcraft-format` | the backend removed here |
| SVG, svgz; PDF; PNG, JPEG, GIF, WebP, TIFF, BMP, TGA | the engine | the `svg`, `pdf` and `image` backends |
| DXF | `vectorcraft-cad` | none; the crate needs no engine and no patches |
| EMF, WMF | `vectorcraft-metafile` | none; the same |
| EPS, AI, AIT, PostScript | `vectorcraft-eps`, `vectorcraft-pdf`, on craft-pdf | none; the engine's patches |
| Affinity (af, afdesign, afpub) | `vectorcraft-affinity`, then the engine, which turns its model into a drawing | none |
| PSD, PSB, PCX | the engine | none |

The cost, from ADR-0007: `cargo build --release --locked --no-default-features --features vector`
at `07d3719` makes a `clv` of 17.2 MB, against 1.0 MB with `--no-default-features`, some of it
shared with the Word and image backends. Besides the binary, a pin and its test, a CI job, and a
format that moves with vectorcraft's releases.

## Alternatives considered

- **Keep it, as a default backend**: about 16 MB for a format few have.
- **Keep it, off by default**: the pin, tests and CI job still to keep up, for a backend hardly
  anyone builds.
- **Widen it to DXF, EMF and WMF** through `vectorcraft-cad` and `vectorcraft-metafile`: formats
  people have that no backend shows, for little more binary; but how well those importers draw
  real files is untried.

## Consequences

- **Positive:** a smaller binary and build, one pin fewer.
- **Accepted:** vectorcraft's drawings, plain or gzipped, aren't shown.
- **Revisit when:** DXF, EMF or WMF are wanted (try vectorcraft's importers on real files first);
  or vectorcraft's format becomes one people send each other.
