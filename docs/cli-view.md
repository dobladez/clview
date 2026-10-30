# Showing pages in the terminal

`clv` draws PDF pages and images straight into the terminal, through the
[kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/). kitty, Ghostty
and WezTerm speak it, among others. It works on macOS, Linux and FreeBSD, and inside tmux.

```sh
clv report.pdf               # page through it, like less
clv report.pdf --pages 1,3-5 # page through just these
clv report.pdf --zoom width  # as wide as the window
clv report.pdf --zoom 4      # four pages to a row
clv report.pdf --no-pager    # print every page and exit, like cat
clv secret.pdf --password PW
clv *.png                    # several files: a tile each, Enter opens one
clv ~/Photos                 # a directory: each file by its first page
clv -r ~/Photos              # its subdirectories' files too
clv comics.zip               # an archive, as a directory
```

On a terminal, more than one page opens the pager: one page at a time, two side by side when
the window is wide enough, or zoomed out, rows of them. A single page (a one-page document, or `--pages 3`) is printed
at the cursor instead, and `view` exits, the way `less -F` prints text that fits.

Output to a file or a pipe takes `--protocol kitty` (without it, `view` stops with an error).
Then `view` doesn't page: it writes every page's escape codes one after another, and `cat` shows
them later in a terminal that speaks the protocol.

Printed pages are drawn in rows one under another and stay in the terminal's scrollback, the
same as text that `cat` prints. `--pages` takes single pages and ranges, in
the order given. Without it, `view` shows the whole document.

How `view` works inside (the transports, what the terminal keeps, what tmux loses and the numbers
behind each choice) is in [`src/view/README.md`](../src/view/README.md).

## Options

| Option | Meaning |
|---|---|
| `--pages 1,3-5` | the pages to show, in order (default: all of them); in a list, the tiles |
| `-r`, `--recurse` | in a directory, its subdirectories' files as tiles of their own, not folders |
| `--zoom auto` | two pages side by side when the window is wide enough, otherwise one (the default) |
| `--zoom width` | one page as wide as the window; a tall page runs past the bottom |
| `--zoom 3` | that many pages to a row, 1 to 8, in as many rows as fit the window: each page a third of the width, even when there is only one |
| `--pager` | always page, even through a single page |
| `--no-pager` | never page: print the pages and exit |
| `--compression auto` | on this machine, temporary files when the terminal takes them and `fast` otherwise; `small` over SSH (the default) |
| `--compression fast` | PNGs with the least work: pages ready sooner, about twice the bytes |
| `--compression small` | PNGs with the fewest bytes, for a slow link |
| `--protocol auto` | check that the terminal can show images first (the default) |
| `--protocol kitty` | skip the check and send the escape codes anyway, even to a file or a pipe |

Printed without the pager, pages follow the zoom too, a row of them under another: `--zoom 3`
prints three to a row, `--zoom width` one to a row as wide as the window, and `auto` two to a row
when the window is wide enough, as the pager first shows them.

## Several files

`clv *.png` and `clv a.pdf b.pdf` show the files as a list: a tile for each, its first page, its
name under it, in the order given. The arrows choose a tile, its name highlighted and on the
status line with its place (`3/12`), and Enter opens it: its pages, paged through as if it had
been given alone. Esc or Backspace goes back to the list as you left it. Printed, a list is its
tiles, in rows, each row's names under it.

`--pages` applies to what is shown, and a list's pages are its tiles: `clv *.pdf --pages 2-3`
shows the second and third files, each still by its first page and opened to all its pages. The
tiles given keep the list's numbers, as a document's pages given keep theirs: the status line
says `3/12 · 2 of 2`, and a number typed is one of the list's. A number past the end of the list
is an error, as a page past the end of a file is. A pattern that matches a single file gives
that file's pages, as naming it alone does.

A file that can't be shown, among others, is left out: why is said on stderr before anything is
shown, the pager's status line names it at first, and printed pages end with an error that says
how many were left out. With none left, nothing is shown. An option goes to the files that take
it: `--password` to the PDFs among them, and it is refused only when none of them takes it.

Opening a file reads what its backend needs to know its pages: an image's header, a PDF's whole
file. Every file given is opened before anything shows, which for many large PDFs takes a while
(20 of an 11 MB, 3,000-page manual take 16 seconds and half a gigabyte); opening only the files
on screen is planned. Files are rendered when their tiles are wanted, two at a time in the pager,
and what a file renders with is let go once the reader is elsewhere, so hundreds of photos open
at once hold little more than their sizes.

## Directories

A directory is a list too, like a gallery: a tile for each of its files that a backend opens, by
its first page (a directory of PDFs shows their covers), and a folder for each subdirectory,
which Enter goes into, to a list of its own; Esc or Backspace comes back. They come in the order
of their names, numbers by their value (`img2` before `img10`), letters regardless of case,
subdirectories among the files. Hidden files are left out, and so, without a word, are files no
backend opens, since a directory holds all sorts. A file a backend opens and can't show (a broken
picture, a PDF that needs a password) is left out as one given by name is, with a warning: before
anything shows for the directory given, and once the screen is back for one gone into. The status
line names each file from the directory given (`2024/beach.jpg`).

With `-r` (`--recurse`), each subdirectory's files are tiles in the list instead of a folder,
at its place among the names, depth first, down to 32 directories. A link to a file is followed; a link to
a directory is not, so links can't make a loop. At most 4,096 files are listed from a directory,
and at most 100,000 entries read, so `clv -r ~` stops in time; a warning says when the listing
stopped early.

A directory and files can be given together, and so can several directories: each directory is a
folder at its place among the arguments (with `-r`, its files, named from it: `photos/a.jpg`).

## Archives

A zip archive is shown as a directory is: a list of the files in it that a backend opens, read
from the archive, and a folder for each directory in it. Given alone it is that list; among other
files, or in a directory, it is a folder, which Enter goes into, and the status line names its
files from it (`trip.zip/2024/beach.jpg`). `-r` lists its directories' files instead of folders
when the archive is given; an archive in a directory stays a folder even then, since `-r` goes
through directories, not into every archive (a jar, a document that is a zip inside). An archive
in an archive is a folder too.

Nothing is unpacked to disk: a file in an archive is read from it to choose its backend and again,
whole, when it is drawn, so an archive open holds its list of names, not its files. Hidden names
are left out, as on disk, and so is macOS's `__MACOSX` directory. A file larger than 512 MiB
unpacked, an encrypted one, or one packed in a way this build doesn't read (zstd) is left out
with a warning, as a broken file in a directory is. At most 100,000 entries are read from an
archive, 4,096 files listed from a directory in it, and names 32 directories deep.

## Which files it shows

A file is shown by the first backend that opens it, going by the file's first bytes, not its
name. This build's backends, and the options each takes besides the ones above, are listed at the
end of `clv --help`. An option that only another backend takes is refused, since it would do
nothing for the file. The file is read once, so `cat report.pdf | clv /dev/stdin` works too.

| Backend | Shows | Its options |
|---|---|---|
| `pdfcraft` (the `pdf` feature, on by default) | PDFs: a file with `%PDF-` in its first 1024 bytes | `--password PW`: the user or the owner password of an encrypted PDF |
| `image` (the `image` feature, on by default) | Images, known by their signature: PNG, JPEG, GIF, WebP, TIFF, BMP, ICO, QOI, JPEG XL, OpenEXR, Radiance HDR, PNM and farbfeld, and TGA, which has no signature, by its `.tga` name | none |
| `zip` (the `zip` feature, on by default) | Zip archives, known by their signature (`PK`), whatever their name: jars, cbz comics, epubs and documents that are zips inside too, as directories ([Archives](#archives)) | none |

PDFs are rendered by [pdfcraft](https://github.com/storytold/pdfcraft), the way its app and
`pdfcraft-cli render` draw them: comments without an appearance of their own are drawn too, and a
dynamic XFA form shows the pages its template lays out into, not the placeholder page the file
carries for viewers that can't. A page whose content runs past pdfcraft's safety budget is drawn
without the rest of it, and a colour it can't compute (a colour space it can't read, a tint
transform it can't evaluate) is painted in a fallback colour, each with a warning. To write an
image file instead, use `pdfcraft-cli render report.pdf --page 1 --out page.png`.

An image is one page, its size its own in pixels, turned the way its EXIF orientation says (a
photo taken with the phone on its side shows upright). It is fitted to the window like a page,
but never made larger than it is: a pixel of the picture to a pixel of the screen at most, so an
icon shows as small as it is. `--zoom width` (and `+` to the width) still makes it as wide as the
window. The first frame of an animation is shown. Colour profiles are not
applied yet: a picture in a wide-gamut space (Display P3, Adobe RGB) shows a little duller than
it should. AVIF and DDS files are recognised and refused with their format's name: the `image`
crate's AVIF decoder gets most AVIFs' colours wrong in 0.25.10, and it has no DDS decoder. A
picture whose pixels would take more than 512 MiB (about 134 megapixels) is refused when it is
opened.

## Paging through a document

The pager shows pages on the alternate screen, like `less`. The shell's scrollback is left as it
was, and quitting returns to it. With `--pages`, the pager goes through those pages only, in
their order.

When the window is wide enough, two pages share it side by side, like an open book: pages 1–2,
then 3–4, and the keys that turn a page turn both. Two pages go side by side when each keeps at
least 80% of the size it would have alone, as portrait pages in a wide window do. Until you zoom,
the pager decides this again whenever the window changes size.

`-` zooms out: one more page to a row each time, in as many rows as fit the window, so zoomed
far out the screen is an overview of the document. It stops at 8 pages to a row, or before a page
would be narrower than 8 columns. The zoom is a size, whatever the number of pages: at four to a
row, a single page takes a quarter of the width, and fewer pages than a row takes sit in the middle
of the screen. `+` zooms in: one page fewer
to a row, and from one page, the page as wide as the window, which scrolls; that is as far in as
it goes. `=` and `_` work too: the same keys, with Shift or without.

Rows start the way a book's spreads do: at three to a row, pages 1–3, 4–6 and so on. The page
you are on stays on screen as you zoom, and the last row of the document stops at the bottom of
the screen, the way `less` stops at the last line. The page at the start of your row sets the
size of every page on screen, and a page is never made smaller to fit another row: at one page
to a row, a tall window shows landscape pages two or three at a time, one under another.

| Keys | Action |
|---|---|
| `space`, `PgDn` | down a screen; at the bottom of a page, on to the next page (or pages) |
| `b`, `PgUp` | up a screen; at the top of a page, back to the previous page (or pages) |
| `j`, `↓`, `Enter` | down a few rows (then on to the next page); with pages side by side, down a row of pages |
| `k`, `↑` | up a few rows (then back to the previous page); with pages side by side, up a row of pages |
| `n`, `→` | next page (or screen of pages) |
| `p`, `←` | previous page (or screen of pages) |
| `g`, `Home` | first page |
| `G`, `End` | last page |
| a number, then `Enter` or `g` | that page |
| `-`, `_` | zoom out: one more page to a row |
| `+`, `=` | zoom in: one page fewer to a row, then the page as wide as the window |
| `r`, `Ctrl-L` | render the pages again |
| `?` | these keys, in place of the pages; any key goes back |
| `Esc`, `Backspace` | back to the list, for a file opened from one |
| `q`, `Esc`, `Ctrl-C` | quit |

In a list:

| Keys | Action |
|---|---|
| `←` `→`, `h` `l` | the tile before or after |
| `↑` `↓`, `k` `j` | the tile above or below; below the last row's tiles, the last |
| `space`, `b`, `n`, `p`, `PgDn`, `PgUp` | a screen on or back |
| `g`, `G`, `Home`, `End` | the first or the last tile |
| a number, then `Enter` | that tile |
| `Enter` | open it: a file to its pages, a folder to its own list |
| `+`, `-` | bigger tiles, fewer to a row; or smaller, more to a row |
| `Esc`, `Backspace` | back, in a folder gone into; at the first list, `Esc` quits |
| `q`, `Ctrl-C` | quit |

A list's tiles are all the same size, square in pixels, so moving through it moves nothing else:
up to four to a row until you zoom (fewer when there are fewer), as many rows as fit. The screen
moves only when the tile you choose leaves it, and the tiles show as they render, as a gallery's
do. `--zoom` sets the list's tiles to a row; a file opened from it starts at `auto`.

The status line keeps its right side in place whatever you do: where you are (`page 3/12`,
`pages 3–4/12`), padded to the widest it gets in the document, the zoom, and `? help`, with `…`
before them while the pages on screen render. A narrow window drops the zoom, then `? help`. The
name is on the left, cut in its middle when it is too long (`IMG_2…41.jpg`, so names that differ
at the end still differ), and a message (a page that failed, `fully zoomed in`, a page number
being typed) sits just left of the right side, cutting the name shorter while it is there. Until
the first key, the message is a reminder of the main keys.

The pager follows the window: after a resize, the pages are drawn again for the new size, as many
to a row as you zoomed to, or as the new size allows. Pages are rendered when they are shown, on a thread
of their own: the keys keep working while a slow page renders (the status line's `…` says it
is rendering), moving on cancels it, and the renderer's watchdog gives up on a page that takes too
long (20 seconds a try) and says so.

The pages around the one on screen are prepared ahead, so turning to them is instant. Jumping
further away waits for the page to render.

Quitting always puts the terminal back as it was, including after an error. A pager killed by a
signal (`kill`, a closed SSH session) can leave the terminal in raw mode; `reset` fixes that.

## Size and sharpness

Each page is rendered for the window it is shown in, at the terminal's own pixel density, so text
stays sharp on high-DPI screens. One row is left free for the shell prompt. Pages are never
rendered larger than 8192 pixels on a side or 16 megapixels in all (more than a 4K screen shows),
which bounds the memory one page can take, however its document shapes it.

The size in pixels usually comes with the window's size, as it does over SSH. Some ways in pass
on only rows and columns, such as `docker exec` and `kubectl exec`; then `view` asks the terminal
how big its cells are, once, when it starts, and assumes 10 × 20 pixel cells if the terminal
doesn't say. Inside tmux, tmux says. When there is no terminal to ask (output to a file with
`--protocol kitty`, no controlling terminal), `view` assumes an 80 × 24 window.

## On this machine or over SSH

When the terminal runs on the same machine, the pager hands it pages through temporary files, as
the protocol allows: kitty takes a page this way much faster than as a PNG inside the escape
codes. Each page's pixels go into a new file that only you can read, which the terminal
deletes as soon as it has read it. The files are in a directory of your own,
`tty-graphics-protocol-clv-<uid>`, inside `TMPDIR`, else `/dev/shm`, else `/tmp`. Any the
terminal leaves are removed when the pager ends, or, if it was killed, by the next one.

Whether the terminal reads files from this machine is found out by trying, at the start. Until it
does, and for good when it doesn't (a terminal without temporary files, or one on another
machine), pages travel as PNGs inside the escape codes. They also do when the temporary directory
isn't safe from other users, and when printing without the pager, so saved or piped output still
works.

A small PNG takes longer to make, and a fast one takes more bytes; the pixels are the same either
way. On the same machine, bytes are cheap and the time is what shows, so `view` compresses fast.
Over SSH, the bytes cross the network, so it compresses small and doesn't try temporary files. It
goes by `SSH_CONNECTION`, `SSH_CLIENT` and `SSH_TTY`, and inside tmux by how the tmux client
attached. `--compression fast` or `--compression small` overrides the guess, and sends PNGs even
on this machine.

## Which terminals work

`--protocol auto` trusts the environment for kitty (`TERM=xterm-kitty` or `KITTY_WINDOW_ID`) and
Ghostty (`TERM=xterm-ghostty` or `TERM_PROGRAM=ghostty`). Any other terminal is asked with the
query the protocol recommends, and has half a second to answer. A terminal that doesn't speak the
protocol gets an error that says so, and nothing is drawn.

Over SSH, the image is sent inside the escape codes themselves, so `view` works there too.

`view` fails when stdout isn't a terminal, unless `--protocol kitty` is given. Windows isn't
supported yet: Windows Terminal speaks sixel, not the kitty protocol. Sixel may come later, which
would also bring the terminals that speak only sixel.

## Inside tmux

`view` works inside tmux 3.3 and later when tmux runs in a terminal that speaks the protocol:

- tmux doesn't know about images, so the escape codes go through tmux's passthrough. tmux only
  lets them through when the pane's `allow-passthrough` option is `on` or `all`. The option lets
  any program in the pane send escape codes past tmux, so `view` never changes it. If it is off,
  `view` stops and says how to turn it on: for every pane, add `set -g allow-passthrough on` to
  `~/.tmux.conf`; for one pane, run the `tmux set -p -t <pane> allow-passthrough on` it shows.
- Pages are drawn with the protocol's *Unicode placeholders*: special characters that tmux stores
  as text, which the terminal draws as the image. The page scrolls with the pane, and survives
  switching windows and panes. Placeholders number at most 297 rows and columns, so inside tmux a
  page is never drawn larger than that.
- Inside tmux, `TERM` describes tmux, not the terminal, so `view` asks tmux which terminal it runs
  in. `xterm-kitty` and `xterm-ghostty` are trusted. For anything else, `view` refuses unless
  `--protocol kitty` is given.
- Pages turned while the pane is hidden (another window in front, or the window zoomed on another
  pane) arrive when it shows again: tmux throws away images sent to a hidden pane. A terminal that
  attaches to the session later gets the pages too.
- With more than one terminal showing the pane, pages go as PNGs, not temporary files: the first
  terminal to read a file deletes it before the others can.
- tmux throws output away when it decides the terminal can't keep up, images with it. `view`
  notices and sends the pages again. If tmux keeps dropping them, the status line says so, and
  `r` sends them again.
- A printed page stays in the terminal after `view` exits, so its placeholders show it again when
  they scroll back, and the terminal holds its pixels until something deletes it. `view` records
  each page it prints in the tmux server's `@clv-printed` option, and each later run deletes the
  pages no pane holds any more: after `clear-history`, a closed pane, or the history past
  `history-limit`. Until the next run they stay. The deletes reach the terminals showing the pane
  that run is in, so a terminal attached only to another session keeps its copies. Closing the
  terminal's tab frees them all.

The terminal itself must support Unicode placeholders. kitty does. Other terminals may draw nothing
inside tmux even though they draw pages outside it.

## Errors

- A page number past the end of the document fails before anything is drawn.
- A page that can't be rendered is reported at the end. The other pages are still drawn, and
  `view` exits with an error.
- What a backend warns about on a page (content it left out, a colour it had to guess) is said
  with the page's number: on stderr when printed, on the status line in the pager.
- File names and messages are written with their control characters replaced, so neither a file
  name nor a document can send escape sequences (a clipboard write, a window title) to the
  terminal through them.
