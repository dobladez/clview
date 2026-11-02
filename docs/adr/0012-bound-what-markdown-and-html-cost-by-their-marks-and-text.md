# ADR-0012: Bound what Markdown and HTML cost by their marks and their text

- **Status:** Accepted
- **Date:** 2026-11-02

## Context

wordcraft's importers crate, already built for odt and RTF
([ADR-0005](0005-word-documents-through-wordcrafts-readers-layout-and-renderer.md)), reads Markdown
and HTML too, so showing them takes little code ([Documents](../usage.md#documents)). They read
untrusted text, and nothing else: the importers cap how deep it nests (a million `>` or `<div>` is
safe), and do no file or network I/O unless given a loader. But the Word backend's limit is 256 MiB,
for files that are mostly compressed, and in a text file every byte may become something. Neither
importing nor laying out can be stopped once started.

## Decision

- **Three limits, in the order the costs fall** (`marked_up` in `src/source/wordcraft.rs`):
  - a file of at most 64 MiB, as an SVG's or a spreadsheet's;
  - at most half a million **marks**, counted over its bytes before it is imported: what may start
    a run, a cell or a paragraph of its own. For Markdown, ASCII punctuation and line ends. For
    HTML, `<` and line ends (a `pre` has a paragraph for each line);
  - at most 8 MiB of **text**, counted in what the importer made, before it is laid out.
- **Pictures they link to are left out**: `import_on`, with no loader, so a document reads nothing
  but itself, as a drawing does. Pictures in `data:` URLs are drawn, within wordcraft's limit on
  what a picture decodes to.
- **HTML by how it starts** (a doctype or an `html` element), whatever its name, so one comes through
  a pipe; or by its name, and that it is text, for a fragment. Markdown has only its name.
  wordcraft's `txt::is_text` says what is text, and refuses a PDF.

## Evidence

Apple M3 Max, macOS 26.6.2, wordcraft at `5b5810fe1f6f815acc3fda27619f61d9bc08b615`, clv's dev
profile (optimized). `/usr/bin/time -l clv FILE --protocol kitty --pages 1` gives the whole run, which
lays the document out twice; the import and one layout were timed by temporary lines in `laid_out`
that exited after each. The files, written by a script:

- **plain**: sections of a heading, a paragraph of 370 characters and a two-item list;
- **stars**: `*a`, repeated;
- **pipes**: a Markdown table of 2,500 columns, repeating its rows;
- **cells**: an HTML table, rows of 200 `<td>x`;
- **items**: `<ul>`, then `<li>a`, repeating.

Before these limits, 8 MiB each (and a real single-file page):

| File | Text | Import | Import and one layout | Whole run |
|---|---|---|---|---|
| plain, Markdown | 8.1 MB | 0.44 s, 361 MB | 9.3 s, 1.6 GB | 20 s, 1.6 GB |
| stars | 4.2 MB | 2.2 s, 4.3 GB | 5.1 s, 5.3 GB | 15 s, 4.5 GB |
| pipes | 0.1 MB | 2.7 s, 5.2 GB | 1.1 s, 6.0 GB | 9.4 s, 5.4 GB |
| cells | 0.5 MB | 1.3 s, 4.1 GB | 5.2 s, 6.2 GB | 17 s, 6.3 GB |
| a real single-file page of 19 MiB | 41 KB | 0.04 s, 72 MB | 0.07 s, 346 MB | 0.20 s, 379 MB |

Memory grows with the bytes: stars of 1, 2, 4 and 8 MiB peak at 1.1, 1.9, 2.9 and 4.5 GB, cells at
1.0, 2.1, 3.7 and 6.3 GB. Time grows faster for plain text: 0.38 s to lay out 1 MiB, 9.3 s for 8;
32 MiB didn't open within two minutes.

Files made to cost the most, within the limits, the whole run:

| File | Bytes | Whole run |
|---|---|---|
| `<ul>`, then half a million `<li>`, sharing out 8 MiB of text | 9.0 MiB | 15 s, 4.9 GB |
| `<ul>`, then half a million `<li>a` | 2.4 MiB | 13 s, 4.4 GB |
| half a million `<p>` each with a page break | 17 MiB | 8.9 s, 3.8 GB |
| half a million `<p>a` | 1.9 MiB | 7.6 s, 3.3 GB |
| half a million lines in a code block, Markdown | 0.9 MiB | 7.6 s, 3.3 GB |
| 8 MiB of text in one paragraph | 8 MiB | 3.5 s, 1.4 GB |

At a million marks the second of these was 7.3 GB and 30 s.

What real files have, counted in the 4,428 Markdown and HTML files of one developer's checkouts
(changelogs, documentation, coverage reports, single-file pages): at most 40,360 marks in a Markdown
file (of 170 KB) and 263,964 in an HTML one (a coverage report of 4.4 MiB, which opens in 0.87 s and
301 MB). One has more than 100,000.

Nesting a million levels, a `colspan` of a million, unclosed tags and attributes, and a PNG of
30,000 × 30,000 pixels in a `data:` URL (which wordcraft doesn't decode: 33 MB at most) all cost
little.

## Alternatives considered

- **A limit on the file's bytes alone**: 8 MiB would still let through 6 GB, and refuse the 19 MiB
  page that opens in 0.2 s, only 41 KB of which is text. Real pages show why: one of 3 MiB is 99%
  scripts and styles and takes 36 MB, and the 1.4 MiB of one that is 95% `data:` URLs cost what its
  pictures do.
- **Counting what the importer made, alone**: the costly files have spent most of it by then, 4.3 GB
  of the 5.3 for stars.
- **A million marks**: nothing real needs it, and it costs twice as much at worst.
- **Marks and text as one number**: they cost differently, marks memory and text the time to lay out.
- **Reading the pictures they link to**: a README's pictures would show. HTML can be given a loader,
  but wordcraft's Markdown importer has no such hook, and a document would then read other files,
  which no other format does.

## Consequences

- **Positive:** real files open, however large their scripts and pictures; a costly one is refused
  with why, before what it would cost is spent.
- **Accepted:**
  - A file made to cost the most still takes about 5 GB and 15 seconds. What a page's text costs
    can't be told from its bytes, and the limits are set to leave real files well inside them.
  - A real page of over half a million tags is refused, though it might open in less than that.
  - Pictures a README links to show as their alt text.
  - HTML is read for its text and structure, not drawn as a browser draws it.
- **Follow-ups:** LaTeX and plain text, which the same importers read.
- **Revisit when:** wordcraft's importers or layout cost less, or can be stopped; or its Markdown
  importer can load pictures, and they are wanted.
