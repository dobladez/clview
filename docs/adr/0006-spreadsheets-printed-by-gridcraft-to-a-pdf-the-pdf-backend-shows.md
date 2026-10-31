# ADR-0006: Spreadsheets printed by gridcraft to a PDF the PDF backend shows

- **Status:** Accepted
- **Date:** 2026-10-31

## Context

Spreadsheets were wanted beside documents: xlsx and its kin, the older xls, ods, and CSV
([Spreadsheets](../cli-view.md#spreadsheets)). A sheet has no pages of its own; on paper it is cut
into pages by its print area, page setup, title rows, and the order its app prints them in.
[gridcraft](https://github.com/storytold/gridcraft), from the same authors as pdfcraft and
wordcraft, is a spreadsheet in Rust (MIT or Apache-2.0): readers, a formula engine, and an
engine crate holding the print layout (`cmd::print`) and a PDF writer of its own. It has no
renderer of cells to pixels outside its egui interface.

The project's rules: a backend's library is named only in its own file, and swapping the PDF
renderer must touch nothing else ([`CLAUDE.md`](../../CLAUDE.md)).

## Decision

- **Print to a PDF, and show that**: read the workbook, recalculate its formulas, lay out every
  visible sheet's print pages and write them as a PDF with gridcraft's engine, then hand the PDF,
  as a file held in memory (`Input::held`), to the first backend that opens it. The spreadsheet
  backend finds the PDF one through `BACKENDS`, as any file finds its backend, and never names it;
  the `sheet` feature brings `pdf`.
- **Pinned to one commit of gridcraft-engine**, as pdfcraft is; it builds none of the crates
  gridcraft patches (only its app's).
- **Recalculate**, as gridcraft's app does on opening: a formula saved without its value (as some
  libraries write them) shows one.
- **Fit a CSV's columns** with gridcraft's own AutoFit command, as its app does on importing one:
  at the width columns start with, a date shows as `#######` and text is cut off.
- **Say what gridcraft says on reading** (a sheet cut short, an ods or xlsb read for its values
  alone) as warnings with the first page, and **say when text will be blank**
  (`blanked` in `src/source/gridcraft.rs`).
- **Bound what a workbook may cost** beyond gridcraft's per-sheet limits: 64 MiB of file, and
  128 MiB of an xlsx's sheet XML together, from its central directory, before gridcraft reads it.

## Evidence

Apple M3 Max, macOS 26.6.2, gridcraft at `1f4c67ef0a0200e48d832c7cbbb5570b743b7837`, pdfcraft at
clview's pin. A release build of a test program (`lto = "thin"`, `codegen-units = 1`) calling
`io::open_bytes`, `Calc::recalc_all`, `cmd::print::layouts` and `render_pdf`, peak memory from
`getrusage`, on workbooks written by a Python script with `zipfile`:

| Workbook | Read | Recalculate | PDF of every page | Pages | Peak memory |
|---|---|---|---|---|---|
| styled.xlsx, 4 sheets | 2 ms | 0.6 ms | 3.6 ms | 6 | 37 MB |
| wide.csv, 2,000 × 60 | 18 ms | 0.4 ms | 267 ms (10 MB) | 270 | 61 MB |
| big.xlsx, 7.6 MB | 407 ms | 125 ms | 1.0 s (38 MB) | 1,648 | 314 MB |
| many.xlsx, 15 MB, 6 sheets, 97 MiB of sheet XML | 1.35 s | 31 ms | 1.2 s | 2,000 (gridcraft's cap) | 645 MB |

big.xlsx holds 100,000 rows of twelve columns; gridcraft read 37,036 of them, stopping at a
million XML elements, and said so. pdfcraft draws one of those pages in 1–5 ms. 750 workbooks
and CSVs made by mutating these went through reading, recalculating, laying out and printing
without a panic; 22 were refused, the slowest took 104 ms.

Text outside Windows-1252: a CSV of Greek, Russian, Polish, Chinese and `≤ ∞`, through
`clv --protocol kitty`, shows only the Western European letters; the rest is blank, the Polish
name half drawn. gridcraft's PDF writer sets such characters in a non-embedded Adobe-GB1 font
(`STSong-Light`), which pdfcraft doesn't substitute.

The binary: `cargo build --release --locked --no-default-features --features sheet` makes a
`clv` of 29.4 MB, against 18.7 MB with `--features pdf` alone: 10.7 MB more, gridcraft's engine
and readers, and the zip crate, which the default build has anyway.

## Alternatives considered

- **Draw cells to pixels in clv**: a second spreadsheet renderer (number formats, fonts,
  borders, merged cells, charts) to keep in step with gridcraft's, where its print layout already
  does it as its app prints.
- **Name the PDF backend, or render the PDF with pdfcraft directly**: simpler, but it breaks the
  rule that only `src/source/pdfcraft.rs` names pdfcraft, and swapping the PDF renderer would
  touch the spreadsheet backend.
- **Each sheet as one tall page, like a screen**: possible with hand-made layouts, but a sheet of
  100,000 rows is a page no window shows, and a workbook's print setup says how its author wants
  it on paper.
- **The values the file saved (`Calc::rebuild`)**: what the author last saw, with no formula to
  evaluate; but a file written without values shows empty cells, and gridcraft's app shows the
  recalculated ones.
- **Claim `.txt` as tab-separated values, as gridcraft reads it**: most text files aren't tables.

## Consequences

- **Positive:** xlsx, xlsm, xlsb, xls, ods and CSV, printed as their app prints them, through a
  renderer clv already has; nothing of the workbook is held once printed.
- **Accepted:**
  - Up to 11 MB more binary.
  - Text outside Windows-1252 prints blank, with a warning, until gridcraft embeds a font for it
    or pdfcraft substitutes one.
  - Opening is synchronous: about 1.5 s for a 7.6 MB workbook, holding up the pager meanwhile.
  - gridcraft's print code (`cmd::print`) is command code, not an API it promises to keep.
  - An ods and an xlsb show their values alone, without formatting or charts.
- **Follow-ups:** report the blank text to gridcraft; a total-cells limit of gridcraft's own.
- **Revisit when:** gridcraft's PDF embeds fonts for every script, or it gains a renderer of cells
  to pixels outside its interface.
