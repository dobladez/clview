# clview

I hate leaving the terminal, so `clv` was born: it shows images, Markdown, PDFs, Word documents,
spreadsheets, SVGs, and HTML right in your kitty-compatible terminal.

[demo](https://github.com/user-attachments/assets/1d94ccc3-3a8e-441b-8f64-8813622a1909)

🤖 Disclaimer: clview is almost entirely coded with AI tools: mostly Claude Code, some Codex, OpenCode,
Pi and others, always from my kitty terminal, of course.

`clv` draws using the [kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
when available: kitty, Ghostty and WezTerm, locally or over SSH, and inside tmux or Zellij too. It
runs on macOS, Linux and FreeBSD. Other terminals get a preview in text cells.

```sh
clv report.pdf    # page through it, like less
clv notes.md      # Markdown, laid out into pages
clv ~/Photos      # a directory: a tile for each file, Enter opens one
```

After you open a file, you can do these things:

- **Paging.** A file with more than one page opens in a pager. The pager shows one page. It shows
  two pages side by side when the window is wide enough. Use `-` to zoom out to rows of pages. Use
  `+` to zoom in. Use `?` to show the keys.
- **Lists.** Two or more files, a directory or an archive open as a list of tiles. A tile shows
  the first page of a file and its name. Use the arrow keys to select a tile. Use `Enter` to open
  it. Use `Esc` or `Backspace` to go back.
- **Copying.** Use `c` to copy the current page or tile to the clipboard as a picture.
- **Searching.** Use `/` to search the text of a document. Type the text and push `Enter`. Use `n`
  and `N` to go to the next and previous match.
- **Links.** Use `f` to label the links on the screen. Type a label to follow its link: to a place
  in the document, to another file, or to a web address. This works like Vimium in a browser. Use
  `Backspace` to go back. Use `F` to copy the link's address.
- **Dark and light.** On a dark terminal, Markdown, HTML and Mermaid diagrams show light text on a
  dark background. Use `D` to change between dark and light.

Options such as `--no-pager`, `--per-row` and `--protocol ascii`, and every key, are in
[`docs/usage.md`](docs/usage.md).

## What it shows

`clv --help` lists the formats your build shows and the options of those that take some.

| Shows | Through |
|---|---|
| PDFs | [pdfcraft](https://github.com/storytold/pdfcraft) |
| PNG, JPEG, GIF, WebP, TIFF, BMP, ICO, QOI, JPEG XL, OpenEXR, HDR, TGA and more | the [`image`](https://crates.io/crates/image) crate, and JPEG XL [jxl-oxide](https://crates.io/crates/jxl-oxide) |
| SVG drawings and svgz | [resvg](https://github.com/linebender/resvg), and [Blitz](https://github.com/DioxusLabs/blitz) for their HTML labels |
| Mermaid diagrams (.mmd, .mermaid), and a ` ```mermaid ` block of Markdown or HTML | [merman](https://github.com/Latias94/merman), a Mermaid in Rust, into SVG that clv then shows |
| Word documents (docx, doc), OpenDocument text (odt) and RTF | [wordcraft](https://github.com/storytold/wordcraft)'s readers, layout and renderer, with fonts of its own under the SIL Open Font License |
| Markdown and HTML, with the pictures they refer to on disk | [wordcraft](https://github.com/storytold/wordcraft) again, and [pulldown-cmark](https://crates.io/crates/pulldown-cmark) for Markdown |
| Spreadsheets (xlsx, xlsm, xlsb, xls, ods, csv, tsv) | [gridcraft](https://github.com/storytold/gridcraft)'s engine, which prints a workbook to a PDF that clv then shows |
| Ebooks (epub), as books: their chapters in order, with their pictures | [rbook](https://github.com/DevinSterling/rbook) for the book's package, then [wordcraft](https://github.com/storytold/wordcraft) as for HTML |
| Zip archives (and what is a zip inside: jar, cbz…), as directories | the [`zip`](https://crates.io/crates/zip) crate |
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

High level, not in order, no promises; more details and other idesa in the [backlog](docs/backlog.md).

- A config file: customization, settings and positions remembered.
- Plain text files (yes, not yet: ironic).
- More diagram languages: Graphviz, D2, math.
- Markdown's code blocks highlighted by their language.
- `D` for SVG drawings too, as for Markdown, HTML and Mermaid diagrams.
- More documents: Org-mode, AsciiDoc and reStructuredText.
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
