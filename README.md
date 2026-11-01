# clview

`clv` shows PDFs, images, SVG drawings, Word documents and spreadsheets in the terminal, through the
[kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/): kitty, Ghostty and
WezTerm, inside tmux too, on macOS, Linux and FreeBSD.

https://github.com/user-attachments/assets/1d3d8132-5dd3-40d7-bf62-e6b25327dbaa


```sh
clv report.pdf             # page through it, like less
clv --zoom 4 report.pdf    # four pages to a row
clv --no-pager report.pdf  # print every page and exit, like cat
clv photo.jpg              # an image, printed at the cursor
clv diagram.svg            # a drawing, as large as the window and sharp at that size
clv letter.docx            # a Word document's pages, laid out as Word prints them (odt, rtf too)
clv budget.xlsx            # a workbook's sheets, printed as its app prints them (xls, ods, csv too)
clv *.pdf                  # several files: a tile each, Enter opens one
clv ~/Photos               # a directory: its files, and its subdirectories as folders
clv -r ~/Photos            # its subdirectories' files too, as tiles of their own
clv comics.zip             # an archive: its files and directories, as a directory's
clv photos.tar.xz          # a tar too, plain or compressed, read through once with progress
clv paper.pdf.gz           # a file compressed alone: the PDF it unpacks to
```

More than a page opens a pager: one page at a time, two side by side when the window is wide
enough; `-` and `+` zoom out to rows of pages and back in to the window's width. Several files,
a directory or an archive, are a list: a tile for each, by its first page, its name under it. The arrows
choose one, Enter opens it (a file to its pages, a folder to its own list), and Esc or `u` goes back. `c`
copies the page, or the tile, to the clipboard as a picture. `?` lists the keys. Everything else
(the options, the keys, tmux, SSH) is in [`docs/cli-view.md`](docs/cli-view.md), and how it works
inside in [`src/view/README.md`](src/view/README.md), and why some choices were made in
[`docs/adr/`](docs/adr/README.md).

What it shows comes from backends, one for each kind of file, each behind a cargo feature and
none of them known to the viewer by name; `clv --help` lists the formats a build's backends show
and the libraries each shows them through, and the options of those that take some:

| Feature | Shows | Rendered by |
|---|---|---|
| `pdf` | PDFs | [pdfcraft](https://github.com/storytold/pdfcraft), the way its app draws them: comments without an appearance of their own are drawn, dynamic XFA forms are laid out, and either password opens a protected file |
| `image` | PNG, JPEG, GIF, WebP, TIFF, BMP, ICO, QOI, JPEG XL, OpenEXR, HDR, TGA and more, the right way up by their EXIF orientation | the [`image`](https://crates.io/crates/image) crate, and JPEG XL [jxl-oxide](https://crates.io/crates/jxl-oxide), in Rust |
| `svg` | SVG drawings and svgz, drawn at the size shown: text in the system's fonts, embedded pictures, filters; nothing they link to outside the file | [resvg](https://github.com/linebender/resvg), in Rust, within limits of clv's own on what a drawing may cost ([ADR-0002](docs/adr/0002-render-svg-with-resvg.md), [ADR-0003](docs/adr/0003-bound-what-a-drawing-costs-before-usvg-and-resvg-spend-it.md)) |
| `zip` | Zip archives (and what is a zip inside: jar, cbz, epub…), as directories: the files in one that another backend opens, and its directories as folders | the [`zip`](https://crates.io/crates/zip) crate, in Rust: deflate, deflate64, bzip2, LZMA and XZ, not zstd or encrypted entries |
| `word` | Word documents (docx, doc), OpenDocument text (odt) and RTF, laid out into pages as their app prints them: headers and footers, tables, pictures, text in the system's fonts or bundled ones; a password opens a protected Word document | [wordcraft](https://github.com/storytold/wordcraft)'s readers, layout and renderer, in Rust (not its editing engine), with fonts of its own under the SIL Open Font License |
| `sheet` | Spreadsheets: Excel workbooks (xlsx, xlsm, xlsb, xls), OpenDocument spreadsheets (ods) and values separated by commas or tabs (csv, tsv), each visible sheet as the pages it prints to, its formulas recalculated; a password opens a protected workbook. Text outside Western European letters prints blank, with a warning | [gridcraft](https://github.com/storytold/gridcraft)'s engine, in Rust, which prints a workbook to a PDF that the `pdf` backend then shows, so `sheet` brings `pdf` too |
| `tar` | Tar archives, plain or compressed with gzip, bzip2 or xz (tgz, cbt…), as directories, read through once on a thread of their own while the pager says how far it has got; and a file compressed alone (`paper.pdf.gz`) as the file it unpacks to | in Rust: [flate2](https://crates.io/crates/flate2) (gzip), the [`bzip2`](https://crates.io/crates/bzip2) crate and [lzma-rust2](https://crates.io/crates/lzma-rust2) (xz), and a tar reader of clv's own; unpacked into a temporary file that nothing names ([ADR-0004](docs/adr/0004-read-a-compressed-tar-through-once-into-a-file-nothing-names.md)). Not zstd |

This began as `view` in pdfcraft's command line.

## Installing

Binaries for macOS and Linux are on the [releases page](https://github.com/dobladez/clview/releases):
unpack one and put `clv` on your `PATH`. They aren't signed, so on macOS one downloaded by a
browser wants `xattr -d com.apple.quarantine clv` before it runs. Or build it, with Rust 1.95 or
later:

```sh
cargo install --locked --git https://github.com/dobladez/clview    # installs `clv`
```

## Building

```sh
cargo install --path .      # installs `clv`
```

All seven features are on by default; `cargo build --no-default-features --features image`
builds the viewer with images alone, and `--no-default-features` alone without any backend.

The PDF backend's pdfcraft is not on crates.io, so it comes from git, pinned to one commit of its
`main` in [`Cargo.toml`](Cargo.toml). To move the pin, change every `rev` there together, the
`[patch.crates-io]` ones included (they are pdfcraft's patched hayro and lopdf), then run the
tests: `tests/it/pdfcraft_pin.rs` fails if a crate pdfcraft patches would be built from crates.io
instead. Only the backend (`src/source/pdfcraft.rs`) and its tests name pdfcraft, so another PDF
renderer would replace that file and the dependencies, and nothing else.

The Word backend's wordcraft comes from git the same way, every `rev` of its crates at one commit;
it needs no patches, since clv builds its readers, layout and renderer but not its engine, whose
PDF export does. `tests/it/wordcraft.rs` fails if the crates disagree, or if a crate wordcraft
patches comes to be built here. Only `src/source/wordcraft.rs` and its tests name wordcraft.
The spreadsheet backend's gridcraft likewise, from `tests/it/gridcraft.rs` and
`src/source/gridcraft.rs`; it reaches the PDF backend only by asking which backend opens the PDF
it prints, never by name.

## Plans

The pager shows sources (`src/source.rs`): anything with pages of a known size that a backend
renders. A file goes to the first backend that opens it, going by its first bytes, and each
backend brings its own options (`--password` is the PDF, Word and spreadsheet backends'). A backend may open a file to
files instead of pages, as an archive holds them, which the viewer lists as it lists a directory.
There are seven so far: PDFs, images, SVG drawings, Word documents, spreadsheets, and zip and tar
archives.

- Open a list's files only when their tiles come on screen, and let go of them after: today
  every file is opened before anything shows, which for a directory of large PDFs takes a while.
- More than one PDF backend to choose from.
- AVIF, colour profiles, and an animation's frames as its pages.
- More archives: zstd, 7z; a comic book archive (cbz) as a book, its pictures
  its pages.

## License

MIT or Apache-2.0, at your option.
