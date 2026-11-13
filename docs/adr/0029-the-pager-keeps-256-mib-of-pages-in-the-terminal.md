# ADR-0029: The pager keeps 256 MiB of pages in the terminal

- **Status:** Accepted
- **Date:** 2026-11-13
- **Deciders:** the owner

## Context

What the pager keeps of its pages is in the terminal, not in clv, within a count and a number of
bytes ([What is kept in the terminal](../architecture.md#what-is-kept-in-the-terminal-pagerrs)).
The bytes matter because kitty keeps a quota of images per screen and, past it, drops nearly
every image without a placement at once. The budget was 192 MiB, set against a quota read as
"320 MB".

A search made the budget tighter than paging did: a page sent again with its tint is held beside
its old copy until it is placed, and where the two don't fit beside the rest of the screen, the
page is blank until its new copy arrives
([ADR-0028](0028-matches-tinted-into-the-pages-pixels.md)). At 192 MiB that began at three
large pages to a row. The owner asked for more room.

## Decision

`BUDGET` is 256 MiB (268 MB). `MIN_BUDGET`, 32 MiB, stays, and so does the rest: the budget is
halved each time the terminal turns out to have dropped a page all the same.

It leaves 64 MiB of kitty's quota for what the pager doesn't count: a page of the largest size
a page is rendered at (16 megapixels, 61 MiB) fits in it.

## Evidence

Measured 2026-11-13 in kitty 0.49.2 on macOS 26.6.2, started hidden with no configuration and
`TMUX` unset (`kitty --start-as=hidden --config NONE <program>`), by a throwaway program run as
its window's: on the alternate screen, it sends raw RGBA images of nothing but zeros through
temporary files (`a=t,f=32,t=t,q=2`), none of them placed; asks for the device attributes and
waits for the answer, so all of it has been taken; then places each one (`a=p`, answers on) and
notes `OK` or `ENOENT`; then deletes everything (`a=d,d=A`) before the next row.

| Images sent, none placed | Kept |
|---|---|
| 67 of 4 MB: 268 MB (255.6 MiB), what the budget is | all |
| 80 of 4 MB: 320 MB (305.2 MiB) | all |
| 83 of 4 MB: 332 MB (316.6 MiB) | all |
| 80 of 4.19 MB (1024 × 1024): 335.5 MB (320.0 MiB) | all |
| 81 of them: 339.7 MB (324.0 MiB) | 1, the last |
| 84 of 4 MB: 336 MB (320.4 MiB) | 1 |
| 5 of 64 MB (4000 × 4000, a page of the largest size): 320 MB | all |
| 4 of 64 MB and 19 of 4 MB: 332 MB (316.6 MiB) | all |
| 4 of 64 MB and 21 of 4 MB: 340 MB (324.2 MiB) | 2, the last |
| 4 of 67 MB (4096 × 4096) and 16 of 4.19 MB: 320.0 MiB | all |
| The same and one more: 324.0 MiB | 1 |

So the quota is 320 MiB (335.5 MB), not 320 MB, to the image: at it everything stays, and 4 MiB
past it nearly everything goes. [Architecture](../architecture.md#what-is-kept-in-the-terminal-pagerrs)'s
older table (13 images of 24.6 MB kept, 14 dropped) agrees.

What the budget holds, by the largest page (`fit.rs`'s `MAX_AREA`, 16 megapixels: 61 MiB):

| Budget | Pages of the largest size it holds | Left of the quota |
|---|---|---|
| 192 MiB | 3 (183 MiB) | 128 MiB |
| 256 MiB | 4 (244 MiB) | 64 MiB |
| 288 MiB | 4 (a fifth is 305 MiB) | 32 MiB |

`tests/it/view_pager.rs` holds the pager to it: three pages of 56 MiB sent again for their tints
never pass 256 MiB in the terminal, and four to a row, where a fifth image doesn't fit, each go
first.

## Alternatives considered

- **192 MiB, as it was:** three large pages to a row already left no room for one tinted copy.
- **More than 256 MiB:** nothing more fits until 305 MiB, a fifth page of the largest size, and
  that leaves 15 MiB of the quota. What is left over is all that stands between another pane's
  pictures and kitty dropping the pager's pages.
- **Counting what else the screen holds:** the pager can't know. Inside tmux every pane's images
  are on kitty's one screen, another clv's among them.

## Consequences

- **Positive:** a third more pages stay ready in large windows, four of the largest size where
  three did; a screen has to hold more before a page sent again for its tint is blank meanwhile
  (four pages of 56 MiB to a row, where three of 51 MiB did it).
- **Accepted:** 64 MiB of headroom under kitty's quota where there were 128. Inside tmux, other
  panes' images count against the same 320 MiB: with more than 64 MiB of them (pictures printed
  in another pane, or another clv paging), kitty drops the pages the pager keeps off screen. The
  pager finds out when it places one (`ENOENT`), sends it again and halves its budget, to 128
  MiB, then 64, then 32. Two clv pagers in one tmux window could pass the quota before, too.
  The screen's own pages are sent whatever they take, as before.
- **Revisit when:** kitty's quota changes, or another terminal's is known to be smaller; or
  pages come to be dropped in ordinary use inside tmux.
