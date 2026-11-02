# ADR-0013: A document's pictures are drawn by the backends, and only from near it

- **Status:** Accepted
- **Date:** 2026-11-02

## Context

Markdown and HTML refer to pictures by a path or a URL, and left them out until now
([ADR-0012](0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md)), which leaves a README
without its screenshots. wordcraft's importer can be given a loader
([ADR-0014](0014-markdown-through-pulldown-cmark-into-wordcrafts-html-importer.md)), for bytes it then
decodes itself: PNG, JPEG, GIF, WebP and BMP. clv's backends already draw far more than that, within
limits of their own.

A list holds many documents at once, so a source must hold little while idle
([`docs/architecture.md`](../architecture.md)), and a document is laid out twice: to open, for its
pages' sizes, and again on its first render.

## Decision

- **A picture is drawn by the backend that shows it**, whichever says it opens the file among those
  that say they may be pictures (`Backend::draws_pictures`: images, drawings, PDFs), page 0, and
  handed to the document as a PNG (`src/source/picture.rs`). A document can't come to draw itself: a
  document, a workbook or an archive is no picture, nor a file compressed alone, which could unpack
  to a document.
- **Only a regular file, named by a path from the document's directory**. `..` may lead out of it.
  - **Nothing is fetched**: a URL of any scheme shows as its alt text. clv has no network code, and
    opening a file shouldn't tell anyone that it was opened.
  - **An absolute path shows as its alt text**: on the web it means from the site's root, not the
    machine's. So does `%2F`, unescaped.
  - **A pipe or a device isn't opened**: it might never end.
  - **A document through a pipe or held in an archive has no directory**, so none of its pictures can
    be reached.
- **A document is laid out around stand-ins** (`stand_in`): a small PNG of the size to show the
  picture at, its pixels never read, taken from `Source::size`: the picture's own at three quarters
  of a point a unit, or smaller to fit the page's text, keeping its shape.
- **A picture is drawn when a page it is on first renders**, replacing its stand-in in the document's
  `media` (`Promised::draw`, `pictures_on`). It is drawn at a resolution of its own: a picture at its
  own pixels, within 1,600 on its longer side; what scales, at three pixels a point of what it is
  shown at, within the same.
- **Limits**: 1,024 pictures a document, each opened to learn its size; 256 MiB of PNGs held, past
  which those drawn longest ago are let go of, and drawn again when wanted.
- **The first page says how many left out on this machine, and a page which of its own failed**: a URL
  isn't counted, since it isn't a mistake.

## Evidence

Apple M3 Max, macOS 26.6.2, wordcraft at `5b5810fe1f6f815acc3fda27619f61d9bc08b615`, clv's dev profile
(optimized), `/usr/bin/time -l clv FILE --protocol kitty` with and without `--pages 1`, on real
documents with their pictures in place:

| Document | Pictures | Pages | First page | All pages |
|---|---|---|---|---|
| aws-icons-for-plantuml's `AWSSymbols.md` | 887, 1 MiB | 114 | 0.32 s, 101 MB | 0.61 s, 110 MB |
| Myna's `README.md` | 23, 4 MiB | 7 | 0.20 s, 317 MB | 2.88 s, 636 MB |
| pdfcraft's `README.md` | 15, 6 MiB | 21 | 0.09 s, 63 MB | 0.42 s, 148 MB |
| wordcraft's `README.md` | 10, 5 MiB | 14 | 0.08 s, 261 MB | 1.01 s, 382 MB |
| clojure-polylith-realworld-example-app's `how-to.md` | 10, 6 MiB | 9 | 0.05 s, 37 MB | 1.07 s, 252 MB |

Pictures made to cost more than their bytes, 1-bit PNGs of zeros on disk: one of 30,000 × 30,000 is
past the image backend's limit, so it is left out, the document taking 29 MB. One of 8,000 × 8,000
takes 1 s and 558 MB. A document of 300 hard links to that one, one to a page: its first six pages take
6.1 s and 646 MB, so memory doesn't grow with them, and all of them about a second each, past the 280 s
this was given.

What real Markdown refers to: 1,374 references with a place to go, in the checkouts of ADR-0014, a
regular expression over each file:

| Where | References | Shown as |
|---|---|---|
| A path, beside or below | 1,044 | The picture |
| A path with `..` | 8 | The picture |
| `http(s)` | 300 | Alt text |
| Absolute | 22 | Alt text |

And their types, of the 1,052 paths: PNG 1,023, SVG 18, JPEG 3, GIF 2, WebP 2, PDF 1, no extension 3.

The stand-ins, in wordcraft at that commit: layout reads a picture's bytes only to wrap text around
one (`Ctx::wrap_outline`), not for an inline one, and `draw_image` scales whatever it decodes into the
rectangle layout gave it. An HTML file of 18 pictures of 6 sizes among text lays out to 13 pages with
real pictures and with PNGs of only their size, and one of six at 800 × 1100 to 12 with either. Such a
PNG must measure as one: with no `IDAT`, the importer gets no size, and that file lays out to 2 pages.
So a stand-in is a whole PNG, under a kilobyte at the largest.

Of the 300 `http(s)` references, 199 look like badges by their URLs (shields.io alone is 126).

## Alternatives considered

- **Handing wordcraft the file's bytes**, for what it decodes: no SVG, no PDF, no JPEG XL or OpenEXR,
  and no EXIF orientation. Simpler for the 98% that are PNG and JPEG, but a second path for those, not
  a shorter one.
- **Drawing every picture on the first render, not page by page**: in the table above that is the
  right-hand column on the first page, 2.88 s for Myna's, and a list of documents would pay it for
  each tile, which shows one page.
- **A real picture when laid out for rendering, and a stand-in only to open**: the two layouts may
  then differ, with a rounding, by a page. Any picture's size would also be its pixels': a 24-pixel
  icon drawn sharply would be shown large.
- **Fetching what a URL names**: network code in a viewer, and a document that tells whoever serves
  its pictures when it was opened, and where. 22% of references, two thirds of them badges, whose alt
  text says what they do.
- **Keeping to the document's directory and below**: a smaller reach, but `docs/usage.md` couldn't show
  `../assets/`. A document could already show what is below it, and nothing it reads leaves the machine.

## Consequences

- **Positive:** what clv shows, a document can show; opening one stays cheap, and a list holds no
  picture until it draws it.
- **Accepted:**
  - A document can show any picture the reader can read, by `..`, on the reader's own screen.
  - What scales is drawn once for each page, at a size good for three pixels a point: zoomed past
    that, it softens, as a photograph does.
  - A picture a document holds in a `data:` URL goes through wordcraft's decoder alone, and a very tall
    one (over 1,584 pt) loses its shape to wordcraft's own limit before it is fitted.
  - The pictures of a document in an archive, and so an epub's, show as their alt text.
- **Follow-ups:** a way for what is held in an archive to reach what is beside it.
- **Revisit when:** wordcraft can be given a picture's pixels and a size directly, without a PNG.
