# clview

I hate leaving the terminal, so `clv` was born: it shows images, Markdown, PDFs, Word documents,
spreadsheets, SVGs, and HTML right in your kitty-compatible terminal.

[demo](https://github.com/user-attachments/assets/4f8b4590-9c21-498c-b72c-ad7c5dcf1371)

🤖 Disclaimer: clview is almost entirely coded with AI tools: mostly Claude Code, some Codex, OpenCode,
Pi and others, always from my kitty terminal, of course.

`clv` draws using the [kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
when available: kitty, Ghostty and WezTerm, locally or over SSH, and inside tmux or Zellij too. It
runs on macOS, Linux and FreeBSD. Other terminals get a preview in text cells.

```sh
clv report.pdf             # page through it, like less
clv --per-row 4 report.pdf # four pages to a row
clv --no-pager report.pdf  # print every page and exit, like cat
clv photo.jpg              # an image, printed at the cursor
clv diagram.svg            # a drawing, as large as the window and sharp at that size
clv flow.mmd               # a Mermaid diagram, drawn as Mermaid draws it (.mermaid too)
clv letter.docx            # a Word document's pages, laid out as Word prints them (odt, rtf too)
clv notes.md               # Markdown, laid out into pages (HTML too)
clv budget.xlsx            # a workbook's sheets, printed as its app prints them (xls, ods, csv too)
clv *.pdf                  # several files: a tile each, Enter opens one
clv ~/Photos               # a directory: its files, and its subdirectories as folders
clv -r ~/Photos            # its subdirectories' files too, as tiles of their own
clv comics.zip             # an archive: its files and directories, as a directory's
clv photos.tar.xz          # a tar too, plain or compressed, read through once with progress
clv paper.pdf.gz           # a file compressed alone: the PDF it unpacks to
clv --protocol ascii photo.jpg > photo.txt  # as plain text, for a log or a mail
```

A file with more than one page opens in a pager. The pager shows one page, or two pages side by
side when the window is wide enough. Use `-` to zoom out to rows of pages. Use `+` to zoom in. Use
`?` to show the keys.

Two or more files, a directory or an archive open as a list of tiles. A file's tile shows its first
page and its name. Use the arrow keys to select a tile. Use `Enter` to open it. A file opens to its
pages, and a folder opens to its own list. Use `Esc` or `u` (up) to go back.

Use `c` to copy the current page or tile to the clipboard as a picture.

Markdown and HTML go light on dark on a dark terminal. Use `d` to switch them between dark and light.

See [`docs/usage.md`](docs/usage.md) for all options and more details.

## What it shows

`clv --help` lists the formats your build shows and the options of those that take some.

| Shows | Through |
|---|---|
| PDFs | [pdfcraft](https://github.com/storytold/pdfcraft) |
| PNG, JPEG, GIF, WebP, TIFF, BMP, ICO, QOI, JPEG XL, OpenEXR, HDR, TGA and more | the [`image`](https://crates.io/crates/image) crate, and JPEG XL [jxl-oxide](https://crates.io/crates/jxl-oxide) |
| SVG drawings and svgz | [resvg](https://github.com/linebender/resvg), and [Blitz](https://github.com/DioxusLabs/blitz) for their HTML labels |
| Mermaid diagrams (.mmd, .mermaid) | [merman](https://github.com/Latias94/merman), a Mermaid in Rust, into SVG that clv then shows |
| Word documents (docx, doc), OpenDocument text (odt) and RTF | [wordcraft](https://github.com/storytold/wordcraft)'s readers, layout and renderer, with fonts of its own under the SIL Open Font License |
| Markdown and HTML, with the pictures they refer to on disk | [wordcraft](https://github.com/storytold/wordcraft) again, and [pulldown-cmark](https://crates.io/crates/pulldown-cmark) for Markdown |
| Spreadsheets (xlsx, xlsm, xlsb, xls, ods, csv, tsv) | [gridcraft](https://github.com/storytold/gridcraft)'s engine, which prints a workbook to a PDF that clv then shows |
| Zip archives (and what is a zip inside: jar, cbz, epub…), as directories | the [`zip`](https://crates.io/crates/zip) crate |
| Tar archives, plain or compressed with gzip, bzip2 or xz (tgz, cbt…), as directories; and a file compressed alone (`paper.pdf.gz`) | [flate2](https://crates.io/crates/flate2), the [`bzip2`](https://crates.io/crates/bzip2) crate and [lzma-rust2](https://crates.io/crates/lzma-rust2), and a tar reader of clv's own |

Missing a format you use? [Open an issue](https://github.com/dobladez/clview/issues) saying which.

## Installing

With [Homebrew](https://brew.sh), on macOS or Linux:

```sh
brew install dobladez/tap/clv
brew upgrade clv      # when a new version is out
```

Or take a binary for macOS or Linux from the [releases page](https://github.com/dobladez/clview/releases):
unpack one and put `clv` on your `PATH`. They aren't signed, so on macOS one downloaded by a
browser wants `xattr -d com.apple.quarantine clv` before it runs. Or build it, with Rust 1.95 or
later:

```sh
cargo install --locked --git https://github.com/dobladez/clview    # installs `clv`
```

To hear of each new version, watch the repository's releases (Watch, then Custom, then Releases) or
follow [their feed](https://github.com/dobladez/clview/releases.atom). What each changed is in the
[changelog](https://github.com/dobladez/clview/blob/main/CHANGELOG.md).

## Plans

High level, roughly in order; the details of each, and ideas not yet planned, are in the
[backlog](docs/backlog.md).

- Plain text files (yes, not yet: ironic).
- Mermaid diagrams, their files and a Markdown `mermaid` block drawn as its diagram
  ([Spec-0008](docs/specs/0008-show-mermaid-diagrams.md), then [Spec-0009](docs/specs/0009-draw-fenced-blocks-as-pictures.md)).
- More diagram languages: Graphviz, D2, math.
- Markdown's code blocks highlighted by their language.
- `d` for drawings and a document's pictures too, not only Markdown and HTML.
- More documents: Org-mode, AsciiDoc and reStructuredText; ebooks (epub) as books.
- A config file: customization, settings and positions remembered.
- `/` as a search in lists too.
- More pictures: AVIF, colour profiles, animations.
- More archives: zstd, 7z; comic book archives as books.
- Sixel, for Windows Terminal and the terminals that speak only it.
- Lighter on memory and steadier: a list letting go of its files, relayouts without a shrunk grid,
  an SVG's stylesheets bounded.
- Maybe: HTML documents through Blitz.

## Working on clview

How to build and test it, and the rules its code keeps, are in [`CONTRIBUTING.md`](CONTRIBUTING.md).
How it works inside is in [`docs/architecture.md`](docs/architecture.md), and why some choices were
made in [`docs/adr/`](docs/adr/README.md).

## Short History

`clv` began as the `view` subcommand in pdfcraft's command line, in
[pull request #2920](https://github.com/storytold/pdfcraft/pull/2920). I immediately realized I
wanted to support all other types of docs and it had to be a separate tool.

Prior to `clv` I was using `icat` for images, `tdf` for PDFs, `mdv` or `mdcat` for Markdown, and didn't
have anything for most other doc types. So I imagined a "display anything" command, and here we are.

Find me on X: [@dobladez](https://x.com/dobladez).

## License

MIT or Apache-2.0, at your option.
