# ADR-0010: A filtered list keeps the whole list's tile size

- **Status:** Accepted
- **Date:** 2026-10-31

## Context

`/` filters a list (several files, a directory, an archive) to the tiles whose names hold what is
typed, as it is typed ([Filtering a list](../usage.md#filtering-a-list); `Nav` and `Filter` in
`src/view/pager.rs`). How big a list's tiles are depends on how many there are: until the reader
zooms, up to four to a row, fewer when there are fewer, and one alone as wide as the window
(`fit::tiles`). A filter changes how many show at every key, so it decides whether the tiles
change size as the reader types.

A tile is rendered for the place it is shown in, and a page already in the terminal is used again
only for a place of the same size (`View` in `pager.rs`): a tile of another size is rendered and
sent again.

## Decision

- While a list is filtered, its tiles keep the size the whole list gives them (or the tiles
  `--pages` gives): `fit::tiles` gets how many tiles the filter filters (`Nav::listed`), not how
  many match. The matches come together in the list's places, a short last row centred as in a
  short list. `+` makes them bigger, as in any list.

## Evidence

clv at the commit that adds the filter, a dev build, on macOS 26.6.2 (Apple M3 Max). The pager on a
script(1) pseudo-terminal of 100 × 60 cells, which reports no pixel size (10 × 20 px cells
assumed), over four SVG drawings, each 400 × 300 with a rectangle and a circle, named `alpha.svg`,
`beta.svg`, `Alphabet.svg` and `café.svg`. Typed `/alpha.` a character at a time, 0.4 s apart,
then Backspace back to nothing, then `q`, and counted the images sent and the base64 bytes of
their PNGs:

```sh
( sleep 2; for k in / a l p h a .; do printf '%s' "$k"; sleep 0.4; done
  for i in 1 2 3 4 5 6 7; do printf '\177'; sleep 0.4; done; sleep 0.5; printf q ) |
  script -q /dev/null sh -c 'stty rows 60 cols 100 &&
    exec clv alpha.svg beta.svg Alphabet.svg café.svg --protocol kitty --compression fast' > out
grep -ao 'a=t,f=100,t=d,q=2,i=' out | wc -l
ESC=$(printf '\033')
LC_ALL=C grep -ao "${ESC}_G[^;${ESC}]*;[A-Za-z0-9+/=]*" out | sed 's/^[^;]*;//' | tr -d '\n' | wc -c
```

| While filtered, the tiles are | Images sent | PNG bytes, base64 |
|---|---|---|
| the whole list's size (this decision) | 4 | 30,064 |
| laid out for the matches (`fit::tiles` given `nav.pages.len()`) | 7 | 129,772 |

Laid out for the matches, `/al` (two match) rendered both again at two to a row, and `/alpha.`
(one) rendered it again as wide as the window: each a render, and a larger image in the terminal.
Back at four, the first images were still in the terminal, so nothing more was sent. In a
gallery of photos or PDFs, each of those is a decode and a render between a key and the matches;
kept at the list's size, the matches are placements of images already there.
`a_list_filters_by_name_as_you_type` in `tests/it/view_pager.rs` checks that filtering sends no
tile again.

## Alternatives considered

- **Lay the tiles out for the matches**, as a list of that many files would show them: a key that
  takes the matches to four, three, two or one renders every match again at another size, and
  the layout jumps while the reader types. Measured above.
- **Lay them out for the matches once the filter is kept** (Enter): steady while typing, but Enter
  then renders them all again, and they change size under the reader's eyes all the same.

## Consequences

- **Positive:** filtering places what is in the terminal: at once, nothing rendered again for a
  tile already shown, and no tile changes size while the reader types.
- **Accepted:** one match out of forty shows as small as it does among forty, centred; `+`
  enlarges it.
- **Revisit when:** the filter should fill the screen with its matches (a gallery mode), or
  rendering a tile again costs next to nothing.
