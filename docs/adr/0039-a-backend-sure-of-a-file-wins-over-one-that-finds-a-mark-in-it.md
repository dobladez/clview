# ADR-0039: A backend sure of a file wins over one that finds a mark in it

- **Status:** Accepted
- **Date:** 2026-11-17

## Context

A file goes to a backend by its first 1024 bytes ([Backends](../architecture.md#backends)). Each
backend answered yes or no, and the first in `source::BACKENDS` to say yes opened the file, so the
order carried every judgement between two claims. The PDF backend says yes to `%PDF-` anywhere in
those bytes, since ISO 32000-2 §7.5.2 lets readers allow bytes before the header; the order kept
it after the backends whose files may hold such bytes (a drawing's text, a tar's first file), and
each new backend had to find its place against that.

The zip backend was last, after the PDF one. A zip of scanned PDFs, 24 files, would not open:
its first file, a PDF, was deflated as stored blocks, since a compressed PDF compresses no
further, so the PDF's header stood at byte 139, and the PDF backend took the zip and refused it as
a PDF it couldn't read. A zip whose first file is stored outright fares the same, or shows that
first PDF alone where it reads.

## Decision

A backend says how sure it is. `Backend::opens` is sure: a signature where its format puts it
(`PK`, an image's, `%PDF-` first), what a file of its kind holds (a docx's `word/`, a tar's
header), or a name and that the file is text. `Backend::opens_loosely`, false by default, is a
mark another format's bytes may hold too: today only a PDF's header past its first byte.
`source::opener` chooses: the first backend sure of the file; only when none is, of those that
open it loosely, the first whose formats go by its name, else the first. The pick, a container's
files, and the backends that ask the others (the spreadsheet's PDF, the epub's HTML, Mermaid's
SVG) all go through it. The order settles only claims as sure, and the backends' order is kept.

## Evidence

Prior art, from the specifications, 2026-11-17:

- WHATWG's [MIME Sniffing](https://mimesniff.spec.whatwg.org/) matches `%PDF-`, zip's `PK\x03\x04`
  and gzip's signature only at byte 0, in the first 1445 bytes, and never sniffs by name.
- [Shared MIME-info](https://specifications.freedesktop.org/shared-mime-info/latest-single)'s
  checking order: globs (names) first, magic when they fail or disagree, and a name's type kept
  only where it is the magic's type or a subclass of it (a `.doc` over OLE storage); magic has a
  priority, 50 by default, higher for more specific types.
- Apache Tika's [detection](https://tika.apache.org/2.3.0/detection.html): magic on the start of
  the file first, the name then used to refine it (text to CSV), since names are easily wrong.

All three let content at a fixed place outweigh a name, and use the name for what content leaves
open. clv's sure claims are mostly at byte 0 or of a container's structure, and only the PDF
backend searches, so two levels are enough.

To see it again: `cargo test -j 2 --test it zip::an_archive_starting_with_a_stored_pdf_is_an_archive`
(a zip whose first file is a PDF, stored), and `--bin clv a_backend_sure_of_a_file` for the
choice among fake backends. Before this, the first test shows the zip as one PDF page.

## Alternatives considered

- **The zip backend before the PDF one** (the first fix, never released): fixes this file, but
  leaves the order to judge every pair, and the next format whose bytes hold a PDF's header trips
  the same way.
- **The PDF backend refusing a file that starts with another format's signature**: a list of
  other formats kept in the PDF backend, against the rule that a backend knows only its own.
- **A score for each claim, as shared MIME-info's priorities or libmagic's strengths**: more than
  two levels for a viewer with nine backends, one of which searches.
- **The name first**: a pipe has none, files are misnamed (a `.pdf` that is an HTML error page),
  and the specifications above trust a signature over it.
- **`%PDF-` only at byte 0**: refuses real PDFs with bytes before the header, which the standard
  and readers allow.

## Consequences

- **Positive:** a backend that searches can't take a file from one that knows its signature,
  whatever the order; the order says only what claims as sure overlap (an svgz before tar, a
  docx before zip).
- **Accepted:** a file of another format with `%PDF-` past its first byte, that no backend is
  sure of, still goes to the PDF backend, as before.
- **Revisit when:** a second backend opens loosely, or a format's claims as sure overlap where
  the order can't settle them.
