# clview

`clv` shows PDF pages in the terminal, through the
[kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/): kitty, Ghostty and
WezTerm, inside tmux too, on macOS, Linux and FreeBSD.

```sh
clv report.pdf             # page through it, like less
clv report.pdf --zoom 4    # four pages to a row
clv report.pdf --no-pager  # print every page and exit, like cat
```

More than a page opens a pager: one page at a time, two side by side when the window is wide
enough; `-` and `+` zoom out to rows of pages and back in to the window's width. Everything else
(the options, the keys, tmux, SSH) is in [`docs/cli-view.md`](docs/cli-view.md), and how it works
inside in [`src/view/README.md`](src/view/README.md).

Pages are rendered by [pdfcraft](https://github.com/storytold/pdfcraft), the way its app draws
them: comments without an appearance of their own are drawn, dynamic XFA forms are laid out, and
either password opens a protected file. This began as `pdfcraft-cli view`.

## Building

```sh
cargo install --path .      # installs `clv`
```

pdfcraft is not on crates.io, so it comes from git, pinned to one commit of its `main` in
[`Cargo.toml`](Cargo.toml). To move the pin, change every `rev` there together, the
`[patch.crates-io]` ones included (they are pdfcraft's patched hayro and lopdf), then run the
tests: `tests/it/pdfcraft_pin.rs` fails if a crate pdfcraft patches would be built from crates.io
instead.

## Plans

- A source abstraction under the pager, so PDFs are one kind of thing it shows, with more than
  one PDF backend to choose from.
- Images.
- Directories, as a gallery: a tile for each file it can show, and Enter opens one.

## License

MIT or Apache-2.0, at your option.
