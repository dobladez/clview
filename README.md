# clview

`clv` shows PDFs and images in the terminal, through the
[kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/): kitty, Ghostty and
WezTerm, inside tmux too, on macOS, Linux and FreeBSD.

```sh
clv report.pdf             # page through it, like less
clv report.pdf --zoom 4    # four pages to a row
clv report.pdf --no-pager  # print every page and exit, like cat
clv photo.jpg              # an image, printed at the cursor
clv *.pdf                  # several files: a tile each, Enter opens one
clv ~/Photos               # a directory: its files, and its subdirectories as folders
clv -r ~/Photos            # its subdirectories' files too, as tiles of their own
clv comics.zip             # an archive: its files and directories, as a directory's
```

More than a page opens a pager: one page at a time, two side by side when the window is wide
enough; `-` and `+` zoom out to rows of pages and back in to the window's width. Several files,
a directory or an archive, are a list: a tile for each, by its first page, its name under it. The arrows
choose one, Enter opens it (a file to its pages, a folder to its own list), and Esc goes back. `?`
lists the keys. Everything else
(the options, the keys, tmux, SSH) is in [`docs/cli-view.md`](docs/cli-view.md), and how it works
inside in [`src/view/README.md`](src/view/README.md), and why some choices were made in
[`docs/adr/`](docs/adr/README.md).

What it shows comes from backends, one for each kind of file, each behind a cargo feature and
none of them known to the viewer by name:

| Feature | Shows | Rendered by |
|---|---|---|
| `pdf` | PDFs | [pdfcraft](https://github.com/storytold/pdfcraft), the way its app draws them: comments without an appearance of their own are drawn, dynamic XFA forms are laid out, and either password opens a protected file |
| `image` | PNG, JPEG, GIF, WebP, TIFF, BMP, ICO, QOI, JPEG XL, OpenEXR, HDR, TGA and more, the right way up by their EXIF orientation | the [`image`](https://crates.io/crates/image) crate, and JPEG XL [jxl-oxide](https://crates.io/crates/jxl-oxide), in Rust |
| `zip` | Zip archives (and what is a zip inside: jar, cbz, epub…), as directories: the files in one that another backend opens, and its directories as folders | the [`zip`](https://crates.io/crates/zip) crate, in Rust: deflate, deflate64, bzip2, LZMA and XZ, not zstd or encrypted entries |

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

All three features are on by default; `cargo build --no-default-features --features image`
builds the viewer with images alone, and `--no-default-features` alone without any backend.

The PDF backend's pdfcraft is not on crates.io, so it comes from git, pinned to one commit of its
`main` in [`Cargo.toml`](Cargo.toml). To move the pin, change every `rev` there together, the
`[patch.crates-io]` ones included (they are pdfcraft's patched hayro and lopdf), then run the
tests: `tests/it/pdfcraft_pin.rs` fails if a crate pdfcraft patches would be built from crates.io
instead. Only the backend (`src/source/pdfcraft.rs`) and its tests name pdfcraft, so another PDF
renderer would replace that file and the dependencies, and nothing else.

## Plans

The pager shows sources (`src/source.rs`): anything with pages of a known size that a backend
renders. A file goes to the first backend that opens it, going by its first bytes, and each
backend brings its own options (`--password` is the PDF backend's). A backend may open a file to
files instead of pages, as an archive holds them, which the viewer lists as it lists a directory.
There are three so far: PDFs, images and zip archives.

- Open a list's files only when their tiles come on screen, and let go of them after: today
  every file is opened before anything shows, which for a directory of large PDFs takes a while.
- More than one PDF backend to choose from.
- AVIF, colour profiles, and an animation's frames as its pages.
- More archives: tar (gzip, xz, zstd), 7z; a comic book archive (cbz) as a book, its pictures
  its pages.

## License

MIT or Apache-2.0, at your option.
