# ADR-0035: The links' labels are placements of one image of letters, over the page

- **Status:** Accepted
- **Date:** 2026-11-15

## Context

`f` labels every link on screen with a short code, as the Vimium browser extension does, and a
code typed follows its link ([Spec-0011](../specs/0011-follow-a-documents-links.md),
[Following links](../usage.md#following-links)). The labels have to show at once, change with
every letter typed (the ones that no longer match go, the letter typed pales in the rest), and go
without a trace. clv's pages are pictures in the terminal: placed by cell through the kitty
graphics protocol, kept there under an image id each, at most 12 of them and 256 MiB of pixels
([What is kept in the terminal](../architecture.md#what-is-kept-in-the-terminal-pagerrs)). A
search's matches are shown by tinting the page's own pixels and sending the page again
([ADR-0028](0028-matches-tinted-into-the-pages-pixels.md)), which costs a render and a PNG a page
for each change.

Where a link leads is the document's to say, and a document may be anyone's: a web address goes
to the desktop's opener, and a path opens a file.

## Decision

1. **Directly, a label is drawn over the page, never into it.** One image holds the fourteen
   letters, each in a yellow box of its own, once as a label shows it and once paler, as a letter
   typed (`hints::Sheet`, drawn from a bitmap font of the viewer's own, five pixels by seven,
   enlarged by a whole number to fit the cell's height). It is sent when the labels first show.
   Each letter of each label is a placement of its box of that image (`x`, `y`, `w`, `h`), at its
   own size, at a cell and a pixel offset into it (`X`, `Y`), above the pages (`z=1`)
   (`kitty::place_part`). A letter typed takes every placement off (`a=d,d=i`) and places what
   still matches again; the labels gone, the image is deleted. The page is never rendered again.
2. **One image, not one a label.** What the pager keeps in the terminal is counted in images
   (`MAX_SENT`, 12, more only for more pages on screen): a thousand labels as a thousand images
   would push the pages out. The one image is counted against that and the budget as it is sent
   (`make_room`), and sent again, under another id, if the terminal says it no longer has it
   (`Key::Missing`).
3. **The checks on where a link leads are the viewer's** (`link.rs`), whatever backend gave the
   link: what a source gives is held to the document's pages and to the limits, a web address to
   `http`, `https` and `mailto` with nothing hidden in it, a path to a regular file or a directory
   made real. A backend says what its document says; none decides what is opened.
4. **Markdown's heading slugs and HTML's `id`s as places are clv's own**: neither pulldown-cmark
   nor wordcraft's HTML importer gives them
   ([ADR-0014](0014-markdown-through-pulldown-cmark-into-wordcrafts-html-importer.md)).
5. **A web address opens with no key to confirm it:** the status line names it as it goes
   (`opening https://…`). Chosen to try; the label typed is the reader's choice of that link.
6. **An address is copied to the clipboard that can say it took it.** Through kitty's clipboard
   protocol (OSC 5522, as `text/plain`) where the terminal speaks it; otherwise to the system's
   clipboard when the terminal runs on this machine; and through OSC 52 over SSH, or where the
   system's clipboard can't be had. OSC 52 is never answered, so it is the last resort, said as
   sent and not as copied, with the address on the status line. On this machine the system's
   clipboard and the one a terminal sets through OSC 52 are the same one.

## Evidence

kitty 0.49.2 on macOS 26.6.2, started hidden with no configuration (`kitty --start-as=hidden
--config NONE`, `TMUX` unset), running a script that writes each code with its answers asked for
and reads them back: a 168 × 32 PNG transmitted (`a=t,f=100,i=902`), then
`a=p,C=1,z=1,i=902,p=1,x=24,y=16,w=12,h=16,X=5,Y=2` over a placement of another image, the same
placement again with another `y` (it moves), a second placement, `a=d,d=i,i=902` and the first
placed again were each answered `OK`; after `a=d,d=I,i=902` the placement was answered
`ENOENT`, with `i=902,p=1`, which is what the pager reads as the image missing. An `X` past the
cell's width (`X=500`) was answered `OK` too; the pager keeps it under the cell's all the same.

Measured the same day on an Apple M3 Max, clv's `dev` build (optimized), in the pager on a
pseudo-terminal of 10 × 20 pixel cells that takes no temporary files, on the tests' PDF with links
(`tests/it/fixtures.rs`'s `linked`), five times each: from `f` to the labels' placements and the
status line written.

| Window | Links on screen | Letters placed | From `f` to the labels | Written in all |
|---|---|---|---|---|
| 100 × 60 cells, one page | 14 | 14 | 21 to 25 ms, the harness's own 20 ms poll among them | 5.3 KB, the image's 2.6 KB among them |
| 200 × 60 cells, two pages | 15 | 30 | 21 to 25 ms | 6.8 KB |

A page sent again with a tint is 22 to 61 ms of render and PNG a page and 30 to 560 KiB written
([ADR-0028](0028-matches-tinted-into-the-pages-pixels.md)'s Evidence), each time what is typed
changes.

**Not watched:** how Ghostty and WezTerm take a placement of part of an image at `z=1` with pixel
offsets over another placement. Spec-0011 leaves it to a look in each.

## Alternatives considered

- **Tinted into the page's pixels, as a search's matches are:** one mechanism for kitty, tmux and
  text cells, but a render and a send a page for the labels to show, again for every letter typed,
  and again for them to go; and two copies of each page on screen held in the terminal meanwhile.
  The labels are a moment's overlay, and the reader is waiting for them.
- **An image a label:** simplest to place, but a screen of links is hundreds of images against a
  count of twelve.
- **The pages placed under the text (`z<0`), the labels as characters over them:** every page's
  placement would change for all of the pager, and how terminals other than kitty take a negative
  `z` hasn't been watched.
- **The labels as characters over the page's cells everywhere:** what tmux and text cells get
  ([Following links](../usage.md#following-links)). Directly, a cell-sized label covers more of a
  page shown small and can only start at a cell's corner; a placement starts at the link's own
  pixel.
- **pdfcraft's own checks for a link's target** (`pdfcraft_engine::links`): they are a PDF
  backend's, and Markdown's links go through the same door.
- **A key to confirm a web address:** Vimium asks none. Revisit after use.
- **OSC 52 before the system's clipboard**, as Spec-0011 lists them: on this machine both reach
  the same clipboard, and only the system's says whether the text got there; a terminal that
  ignores OSC 52 (macOS's Terminal, a tmux with `set-clipboard` off and no passthrough) would
  leave the reader told it was sent, with nothing copied.

## Consequences

- **Positive:** the labels show without a render, the same pages staying as they are; typing
  costs a few hundred bytes; nothing of the labels outlives them in the terminal.
- **Accepted:** a bitmap font of fourteen letters in the viewer. Labels may overlap one another,
  a later one on top. Inside tmux and in text cells the labels are characters, a cell each, since
  neither has placements at pixels.
- **Follow-ups:** a mouse click on a link would test against the same rectangles.
- **Revisit when:** Ghostty or WezTerm turn out not to draw such a placement as kitty does (the
  labels as characters there, as inside tmux); or a web address opened without a confirmation
  proves a mistake.
