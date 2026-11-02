# clview

`clv` shows PDFs, images, SVG drawings, Word documents and spreadsheets in the terminal, through the
[kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/): kitty, Ghostty and
WezTerm, inside tmux too, on macOS, Linux and FreeBSD.

https://github.com/user-attachments/assets/bee14c3d-781f-49f4-8f0f-df44e3300eb3

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
copies the page, or the tile, to the clipboard as a picture. `?` lists the keys.

**Everything else is in the manual, [`docs/usage.md`](docs/usage.md)**: the options, the keys, lists
and archives, what each format shows, tmux, SSH and the limits.

## What it shows

`clv --help` lists the formats your build shows and the options of those that take some.

| Shows | Through |
|---|---|
| PDFs | [pdfcraft](https://github.com/storytold/pdfcraft) |
| PNG, JPEG, GIF, WebP, TIFF, BMP, ICO, QOI, JPEG XL, OpenEXR, HDR, TGA and more | the [`image`](https://crates.io/crates/image) crate, and JPEG XL [jxl-oxide](https://crates.io/crates/jxl-oxide) |
| SVG drawings and svgz | [resvg](https://github.com/linebender/resvg) |
| Word documents (docx, doc), OpenDocument text (odt) and RTF | [wordcraft](https://github.com/storytold/wordcraft)'s readers, layout and renderer, with fonts of its own under the SIL Open Font License |
| Spreadsheets (xlsx, xlsm, xlsb, xls, ods, csv, tsv) | [gridcraft](https://github.com/storytold/gridcraft)'s engine, which prints a workbook to a PDF that clv then shows |
| Zip archives (and what is a zip inside: jar, cbz, epub…), as directories | the [`zip`](https://crates.io/crates/zip) crate |
| Tar archives, plain or compressed with gzip, bzip2 or xz (tgz, cbt…), as directories; and a file compressed alone (`paper.pdf.gz`) | [flate2](https://crates.io/crates/flate2), the [`bzip2`](https://crates.io/crates/bzip2) crate and [lzma-rust2](https://crates.io/crates/lzma-rust2), and a tar reader of clv's own |

This began as `view` in pdfcraft's command line.

## Installing

Binaries for macOS and Linux are on the [releases page](https://github.com/dobladez/clview/releases):
unpack one and put `clv` on your `PATH`. They aren't signed, so on macOS one downloaded by a
browser wants `xattr -d com.apple.quarantine clv` before it runs. Or build it, with Rust 1.95 or
later:

```sh
cargo install --locked --git https://github.com/dobladez/clview    # installs `clv`
```

## Plans

- Open a list's files only when their tiles come on screen, and let go of them after: today
  every file is opened before anything shows, which for a directory of large PDFs takes a while.
- More than one PDF backend to choose from.
- AVIF, colour profiles, and an animation's frames as its pages.
- More archives: zstd, 7z; a comic book archive (cbz) as a book, its pictures
  its pages.
- Sixel, which would bring Windows Terminal and the terminals that speak only sixel.

## Working on clview

How to build and test it, and the rules its code keeps, are in [`CONTRIBUTING.md`](CONTRIBUTING.md).
How it works inside is in [`docs/architecture.md`](docs/architecture.md), and why some choices were
made in [`docs/adr/`](docs/adr/README.md).

## License

MIT or Apache-2.0, at your option.
