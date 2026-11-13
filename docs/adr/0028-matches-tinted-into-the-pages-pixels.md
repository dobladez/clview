# ADR-0028: A search's matches tinted into the page's pixels, the page rendered again when its tint changes

- **Status:** Accepted
- **Date:** 2026-11-13

## Context

A search ([Spec-0006](../specs/0006-search-a-pdfs-text.md)) has to show where its matches are on
a page, the one the reader is on told from the others. clv's pages are pictures: an image in the
terminal through the kitty graphics protocol, placed by cell; inside tmux, an image shown through
Unicode placeholder cells; and where the terminal has no protocol, text cells sampled from the
pixels ([Getting pages to the terminal](../architecture.md#getting-pages-to-the-terminal),
[Text cells](../architecture.md#text-cells-cellsrs)). What the pager keeps of a page is in the
terminal, not in clv: at most 12 pages and 192 MiB of pixels
([What is kept in the terminal](../architecture.md#what-is-kept-in-the-terminal-pagerrs)). A PDF's
paper is white; Markdown's may be the terminal's dark.

## Decision

1. **The matches are tinted into the page's own pixels,** as it arrives from the render pool and
   before it is encoded as a PNG, written to a temporary file or made into text cells
   (`search::tint`, in `Pager::receive`). One mechanism for kitty, tmux and text cells.
2. **A page in the terminal is known by its tint** (`Sent::tint`, a hash of its matches'
   rectangles and of which is the reader's; 0 for none). A page whose tint is another by now is
   asked of the render pool again, like a page never sent, and sent under a new image id; the one
   in the terminal stays on screen until the new one is placed, and is deleted then. A page whose
   tint didn't change is only placed. Nothing of a page's pixels is kept in clv for it. The two
   copies count against the terminal's budget while both are held: a page is asked for again only
   if it fits beside its old copy with what the screen holds, and otherwise waits for another
   page's old copy to go. A copy nobody sees (off screen, or under the keys) goes as its
   replacement is sent; and a page that doesn't fit beside itself, past half the budget, goes
   first, its place empty until the new one comes: the one case where the screen isn't kept whole.
3. **A render under way keeps going when the tint changes,** and gets the tint of the moment it
   arrives: a page is the same pixels whatever its tint. So keys read together, or a key held,
   cost at most one render and one send a page, for where they end.
4. **A page's tint waits for all its matches' rectangles** (`Search::retint`): until those asked
   for have come, a page already in the terminal keeps the tint it has, so a page with a hundred
   matches is sent again once, not a hundred times. The pages rendered ahead of the reader have
   their tints too, so turning to one is still only a placement.
5. **The colours** are two for each kind of match, by how light the pixel is: a colour multiplied
   into light pixels and one screened onto dark ones, mixed by the pixel's luma. Paper takes the
   colour and ink stays ink, on white paper as on dark: a yellow for the matches (`#FFEC82`
   multiplied, `#463A00` screened), an orange for the reader's (`#FFAA3C`, `#783C00`). Each pixel
   is tinted once, however many matches' rectangles cover it, as the reader's if any is.
6. **Nothing is tinted while the query is typed,** and a page copied (`c`) is the page as
   rendered: a copy is a render of its own, which the tint never touches.

## Evidence

Measured 2026-11-13 on an Apple M3 Max, macOS 26.6.2, clv's build at this commit (its `dev`
profile, which is optimized), in the pager on a pseudo-terminal that takes no temporary files, so
every page goes as a PNG in the stream: from a key to the placement (`a=p`) of the page's new
image, one page to the window. The pseudo-terminal is a throwaway script's (`forkpty`, its window
set to the cells below at 10 × 20 pixels each, `TERM=xterm-kitty`, no `TMUX`), running
`clv <file> --protocol kitty --per-row 1`; it writes a key, reads what clv writes, and notes
when a `\x1b_Ga=p,q=1,i=<id>,p=1` names an id not placed before. The files: the tests' five-page
fixture (`tests/it/fixtures.rs`'s `letter`), searched for `e`, and a book of 641 pages, for `the`.

| What | Window of 120 × 50 cells (a page of some 0.7 megapixels) | 240 × 100 cells (some 3 megapixels) |
|---|---|---|
| `Enter`: the first tint of the page on screen, its matches' rectangles asked of the worker first | 35 to 41 ms | 70 ms |
| `n` to a match on the same page: rendered again, tinted, encoded, written | 22 to 38 ms | 56 to 61 ms |
| `N` round to the last page of 641, not in the terminal | 44 ms | |
| `Esc`: the page untinted again | 81 to 105 ms, 50 of them the wait that tells `Esc` from an escape sequence | 170 ms |
| What one `n` writes | 30 to 230 KiB (the page's PNG) | 560 KiB |

With seven `n` in one write, two pages were sent (the one that lost the reader's match and the
one that gained it), not fourteen. Tinting itself is a pass over the rows a match is on, after a
copy of the page's pixels (the backend's buffer isn't the viewer's to write).

The colours, computed from `search::tinted` (contrast as WCAG 2 measures it):

| Paper and ink | A match: its ground, its ink, their contrast | The reader's match |
|---|---|---|
| White paper, black ink (21 to 1 untinted) | `#FFEC82`, `#463A00`, 9.4 to 1 | `#FFAA3C`, `#783C00`, 4.5 to 1 |
| Paper of `#1E1E1E`, ink of `#E6E6E6` (13.4 to 1) | `#544B1C`, `#E6D680`, 6.0 to 1 | `#7B4B1B`, `#E7A147`, 3.3 to 1 |

A grey ramp tinted has no step larger than 3 levels between neighbours, so anti-aliased edges
show no halo. The reader's match on dark paper is the weakest of the four: light ink turns orange
on a brown ground.

Spec-0006's review measured what tinting as the reader types would cost, and left it out: each
page sent again is 35 to 110 ms of PNG encoding on the pager's thread where the terminal takes no
temporary files, more inside tmux, a key at a time.

## Alternatives considered

- **An image over the page for each match:** kitty can place an image at a pixel offset within a
  cell, above another. tmux's placeholders are cells: an overlay can't be put between them. Text
  cells have no layers at all. Two mechanisms, and the second (tinting) needed anyway.
- **An untinted copy of each page kept in clv,** tinted and sent again without rendering: the
  pages near the screen at four bytes a pixel are tens to hundreds of megabytes of clv's own
  memory, outside the budget that bounds what the pager holds. Rendering a page again costs about
  as much as encoding it, and happens on the pool's threads.
- **One colour blended over the pixels at some opacity:** it greys the ink it covers, on white,
  and washes light ink out on dark paper. Multiplying alone is invisible on dark paper; screening
  alone on white.
- **Inverting the match:** shows on any paper, but turns a white page's match into a black bar
  around light letters, the strongest thing on the page for every match.
- **Tinting as the reader types:** left out for its latency (above); `Enter` searches.
- **Sending a page again for each rectangle as it arrives:** as many sends as matches.

## Consequences

- **Positive:** the same tint in kitty, inside tmux, and in text cells as far as their colours go;
  what the terminal holds stays within `MAX_SENT` and `BUDGET`, a page sent again taking the
  place of the one before; a resend for a tint is no resend for a page lost (`MAX_RESENDS`).
- **Accepted:** each step to a match on another part of the screen sends one or two pages again
  (the one that lost the reader's colour, the one that gained it): 25 to 60 ms and a PNG each
  where there are no temporary files. For a moment after a key the page shows the tint before.
  A page with matches is copied (`to_vec`) once to be tinted. Text cells without colours don't
  show the tint; the status line still says which match the reader is on.
- **Follow-ups:** the colours are a choice to look at on real terminals, light and dark;
  [Spec-0007](../specs/0007-search-markdown-html-and-word-documents.md) brings dark paper.
- **Revisit when:** the latency from a key to the tinted screen is under some 50 ms everywhere
  (kitty with and without temporary files, inside tmux): then tinting as the reader types.
