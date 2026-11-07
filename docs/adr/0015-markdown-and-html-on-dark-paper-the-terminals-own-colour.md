# ADR-0015: Markdown and HTML on dark paper, the terminal's own colour

- **Status:** Accepted; `--theme` and `d` being the viewer's superseded by [ADR-0021](0021-a-backend-declares-its-settings-and-their-options-hold-for-any-file.md)
- **Date:** 2026-11-02

## Context

A white page on a terminal with a dark background is a bright rectangle in a dark window. Markdown and
HTML are text with no paper of their own, so there is no reason to show them on white; a PDF or a Word
document is a page as printed, designed on white
([Documents](../usage.md#documents)). wordcraft's renderer has a dark page (`RenderOptions::dark`): the
lightness of every colour inverted, pictures left alone, the paper a grey of one's choosing (`#333` by
default). The reader wants to choose, by an option and a key, since a dark page isn't always what is
wanted.

## Decision

- **Only what has no paper of its own goes dark**: a source says so (`Source::dark_form`), and of the
  sources, only Markdown and HTML do. A list has a dark form if any file in it does, and only those
  files change.
- **What a page is on travels with the request** (`Request::paper`), so a pool's threads have it with
  everything else, nothing is kept per paper, and a source with no dark form has nothing to do. `d`
  turns the viewer's paper and goes the way `r` goes: all that was sent goes, and is rendered again.
- **`--theme auto|light|dark`, the viewer's own option, `auto` by default**: dark on a dark terminal.
  It is the viewer's, not the backend's, since the key must be, and in a list of many kinds of file it
  says the same of each that has a dark form; a PDF given `--theme dark` is shown as ever, not refused.
- **The terminal is asked for its background (OSC 11) only when what is shown has a dark form, or is
  a list, which may hold some**, and never with `--protocol kitty`, which says not to ask, or when the
  output is no terminal's. One that doesn't answer gets a white page.
- **The page is exactly the terminal's colour**, so it isn't a rectangle at all: wordcraft's dark page
  on the grey as bright as that colour (`grey_of`), then each channel stretched to meet it at that grey
  and keep its white (`on_paper`). A light terminal, or one that doesn't say, has `#1E1E1E` for `d` and
  `--theme dark` to go to.
- **A picture stays what it is, except thin near-black ink**, which is given white behind it
  (`thin_and_near_black`): at least 5% of it clear, under half of it opaque, what is opaque of a mean
  luma under 48. Decided when it is drawn, once, since white behind it can't be seen on white paper.
- **`c` copies the page as it is shown**, and text cells go dark as images do.

## Evidence

Terminals: kitty 0.49.2 and tmux 3.5a, Apple M3 Max, macOS 26.6.2, a program in the terminal that
writes `ESC ] 11 ; ? ESC \ ESC [ c` in raw mode and records the answer, under
`kitty --start-as=hidden --config NONE -o background='#1e2e3e'`:

| Asked | Answer |
|---|---|
| kitty | `ESC ] 11 ; rgb:1e1e/2e2e/3e3e ESC \ ESC [ ? 62 ; 52 ; c` |
| tmux in that kitty, a private server, no configuration | `ESC ] 11 ; rgb:1e1e/2e2e/3e3e ESC \ ESC [ ? 1 ; 2 ; 4 c` |

Ghostty and WezTerm weren't tried.

Stretching a page, in a loop over the same code, `rustc -O`: 1.3 ms for 1224 × 1584 pixels, 3.4 ms for
2000 × 2600, and nothing at all for a terminal whose background is a grey.

Pictures on dark paper, among the 1,030 distinct PNGs and SVGs real Markdown refers to in one developer's
checkouts, 671 of them with an alpha channel to go by, measuring what `clv` draws of each:

| Picture | How many | On dark paper |
|---|---|---|
| At least 5% clear, mean luma of what is opaque under 96 | 205 | Too many to back: 191 are one icon set, most of which show, and the rest include screenshots |
| The same, under 48, opaque on under half (the rule) | 52 | Next to nothing to see: a logo of luma 11 in four repositories' docs, 47 icons of luma 45, a "light" logo |
| Dark windows' screenshots, their shadows' margins clear | 4 | Show as they are: luma 44–46, but 64–75% opaque, which the rule spares |

The rule's 50% sits between the logo's 44% and the screenshots' 64%. Looking at the pages decided it:
a first rule, of dark ink whatever it covered, caught 119 PNGs at a mean luma under 80, nearly all of
them that icon set but those screenshots too, which a white card would have spoiled.

## Alternatives considered

- **wordcraft's dark paper as it is**, `#333`: a lighter rectangle on most dark terminals, and a page
  that doesn't look like the terminal it is in.
- **`dark_paper` alone, the terminal's grey**: right on a grey terminal, a visible rectangle on a tinted
  one (`#282C34`, `#002B36`), since it takes one byte. A page's edge is what this is to remove.
- **Rendering a transparent page**, for the terminal to show through: wordcraft paints its paper, and
  taking it out again from anti-aliased text is what the stretch avoids.
- **Inverting a page's pixels after rendering**, for any source: pictures inverted too, and a PDF's
  figures with them. The reader decided on Markdown and HTML alone.
- **Paper as state a source keeps** (`set_paper`): a pool's threads would have to share it, and a render
  under way could come back on the wrong paper; a request says which it was for.
- **Always asking the terminal**: a round trip for what a PDF doesn't need, over SSH too.
- **A backend's option**, as `--password` is: a key can't belong to a backend, and a list of Markdown and
  PDFs would refuse it.
- **White behind everything transparent**: a card round every icon and screenshot, most of which show
  as they are.
- **Inverting thin black ink instead of backing it**: a logo in another colour than its own.

## Consequences

- **Positive:** on a dark terminal, Markdown looks like the terminal, with a key to see it as printed.
- **Accepted:**
  - A picture on a tinted terminal's page shifts by as little as the colour is from grey.
  - `#1E1E1E` for a terminal that doesn't say, which may not be its colour.
  - A picture a document holds in a `data:` URL goes through wordcraft's decoder alone, so thin black
    ink there isn't backed.
  - A `<picture>` with a dark `<source>` for `prefers-color-scheme` shows its `<img>`, the light one.
- **Follow-ups:** Markdown's own styling, seen both ways.
- **Revisit when:** wordcraft takes a paper colour, not a grey, or can leave its paper unpainted.

## Update (2026-11-07)

`--theme` and `d` are no longer the viewer's but the documents' backend's `paper` setting, which a
backend declares as it does its options; `Source::dark_form` and `Request::paper` gave way to
`Source::settings` and `Request::values`. The option is still taken for any file, so a PDF given
`--theme dark` is shown as ever
([ADR-0021](0021-a-backend-declares-its-settings-and-their-options-hold-for-any-file.md)). The rest of
this decision stands.
