# Using clv

Everything `clv` does that you can see, and how to control it. What it is and how to install it
are in the [README](../README.md). `clv --help`, or `clv` alone, lists the options and the formats
of the build you have. How it works inside is in [`docs/architecture.md`](architecture.md).

## Paged or printed

On a terminal, more than one page opens the pager: one page at a time, two side by side when the
window is wide enough, or zoomed out, rows of them. A single page (a one-page document, or
`--pages 3`) is printed at the cursor instead, and `clv` exits, the way `less -F` prints text that
fits.

Printed pages are drawn in rows one under another and stay in the terminal's scrollback, the same
as text that `cat` prints.

Output to a file or a pipe takes `--protocol kitty`, `blocks` or `ascii` (without one, `clv` stops
with an error). Then `clv` doesn't page: with `kitty` it writes every page's escape codes one after
another, and `cat` shows them later in a terminal that speaks the protocol; with the others it
writes [text cells](#text-cells).

A file is read once, so `cat report.pdf | clv /dev/stdin` works too.

## Options

```sh
clv --pages 1,3-5 report.pdf # page through just these
clv --zoom width report.pdf  # as wide as the window
clv --password PW secret.pdf
```

| Option | Meaning |
|---|---|
| `--pages 1,3-5` | the pages to show, single pages and ranges, in the order given (default: all of them); in a list, the tiles |
| `-r`, `--recurse` | in a directory, its subdirectories' files as tiles of their own, not folders |
| `--zoom auto` | two pages side by side when the window is wide enough, otherwise one (the default) |
| `--zoom width` | one page as wide as the window; a tall page runs past the bottom |
| `--zoom 3` | that many pages to a row, 1 to 8, in as many rows as fit the window: each page a third of the width, even when there is only one |
| `--pager` | always page, even through a single page |
| `--no-pager` | never page: print the pages and exit |
| `--theme auto` | [Markdown and HTML](#documents) light on dark on a terminal with a dark background, dark on light otherwise (the default); taken whatever the files, and doing nothing to the rest. With documents, which `--help` lists it under |
| `--theme light`, `--theme dark` | Markdown and HTML that way, whatever the terminal |
| `--width window` | Markdown and HTML on pages as wide as the window, or two to a row where it is wide (the default); taken whatever the files, like `--theme`. With documents |
| `--width 72` | Markdown and HTML laid out to lines of that many characters of code, 20 to 200, the pages as tall as the window ([Documents](#documents)) |
| `--font 150` | Markdown and HTML text at that share of its size, 50 to 400 percent; at 100, the default, a character of code is a cell ([Documents](#documents)). With documents |
| `--compression auto` | on this machine, temporary files when the terminal takes them and `fast` otherwise; `small` over SSH (the default) |
| `--compression fast` | PNGs with the least work: pages ready sooner, about twice the bytes |
| `--compression small` | PNGs with the fewest bytes, for a slow link |
| `--protocol auto` | images if the terminal can show them, otherwise [text cells](#text-cells) and a warning (the default) |
| `--protocol kitty` | skip the check and send the escape codes anyway, even to a file or a pipe |
| `--protocol blocks` | [text cells](#text-cells) of half-blocks and colours, even where images would show |
| `--protocol ascii` | [text cells](#text-cells) of ASCII characters |

Printed without the pager, pages follow the zoom too, a row of them under another: `--zoom 3`
prints three to a row, `--zoom width` one to a row as wide as the window, and `auto` two to a row
when the window is wide enough, as the pager first shows them.

`clv --help`, or `clv` alone, lists these, then the options a backend takes of its own
(`--password`, for PDFs, documents and spreadsheets: see
[Which files it shows](#which-files-it-shows)), each under what that backend shows, then the
formats this build shows, with the libraries each shows them through. It starts with clview's
version and what clview is, in a line each, and ends with a link to this manual and where to report
issues and ideas: <https://github.com/dobladez/clview>.

In kitty and Ghostty, [inside tmux](#inside-tmux) too, clv's icon sits to the right of the help's
first lines, smaller or not at all when the window is narrow. Everywhere else, and through a pipe
or into a file, the help is the same text without it. `clv` asks the terminal nothing for it, so a
terminal the environment doesn't name gets no icon, WezTerm among them, though it shows pages.

## Keys

`?` lists them in the pager too, a backend's after the viewer's: `d` is the documents', and does
nothing in a `clv` built without them. `+` and `-` are listed once, as what they do where you are:
the font size on a Markdown or HTML file, the zoom elsewhere, and the first status line's reminder
says which (`+ - font`, `+ - zoom`).

In a file's pages:

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
| `-`, `_` | zoom out: one more page to a row. On a Markdown or HTML file, its text 10% smaller ([Documents](#documents)) |
| `+`, `=` | zoom in: one page fewer to a row, then the page as wide as the window. On a Markdown or HTML file, its text 10% larger; a number, then `+` or `-`, to that percentage, and `0+` back to 100% |
| `r`, `Ctrl-L` | render the pages again |
| `c` | copy the page you are on to the clipboard ([Copying a page](#copying-a-page)) |
| `d` | Markdown and HTML light on dark, or dark on light ([Documents](#documents)); with documents |
| `[`, `]` | Markdown and HTML laid out again, 4 characters a line narrower or wider; a number, then `]` or `[`, to that many, and `0]` back to the window's ([Documents](#documents)); with documents |
| `?` | these keys, in place of the pages; any key goes back |
| `Esc`, `Backspace`, `u` | back to the list, for a file opened from one |
| `q`, `Esc`, `Ctrl-C` | quit |

A Markdown or HTML page is as tall as the window ([Documents](#documents)), so it has nothing to
scroll: `j`, `k` and `Enter` turn its pages as `n` and `p` do.

In a list:

| Keys | Action |
|---|---|
| `←` `→`, `h` `l` | the tile before or after |
| `↑` `↓`, `k` `j` | the tile above or below; below the last row's tiles, the last |
| `space`, `b`, `n`, `p`, `PgDn`, `PgUp` | a screen on or back |
| `g`, `G`, `Home`, `End` | the first or the last tile |
| a number, then `Enter` | that tile |
| `Enter` | open it: a file to its pages, a folder to its own list |
| `/` | filter the tiles by name, as you type ([Filtering a list](#filtering-a-list)) |
| `+`, `-` | bigger tiles, fewer to a row; or smaller, more to a row |
| `c` | copy the tile's picture to the clipboard: a file's first page (a folder has none) |
| `d` | Markdown and HTML light on dark, or dark on light; the rest of the list stays as it is |
| `[`, `]` | nothing: only a document opened from the list reflows, and they say so |
| `Esc`, `Backspace`, `u` | clear the filter, if one is kept; otherwise back (up), in a folder gone into, or stop opening one; at the first list, `Esc` quits, the others do nothing |
| `q`, `Ctrl-C` | quit |

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
of the screen. `+` zooms in: one page fewer to a row, and from one page, the page as wide as the
window, which scrolls; that is as far in as it goes. `=` and `_` work too: the same keys, with
Shift or without.

Rows start the way a book's spreads do: at three to a row, pages 1–3, 4–6 and so on. The page
you are on stays on screen as you zoom, and the last row of the document stops at the bottom of
the screen, the way `less` stops at the last line. The page at the start of your row sets the
size of every page on screen, and a page is never made smaller to fit another row: at one page
to a row, a tall window shows landscape pages two or three at a time, one under another.

The pager follows the window: after a resize, the pages are drawn again for the new size, as many
to a row as you zoomed to, or as the new size allows; a Markdown or HTML document is laid out again
for it too ([Documents](#a-page-the-windows-size)). Pages are rendered when they are shown, on a
thread of their own: the keys keep working while a slow page renders (the status line's `…` says it
is rendering), moving on cancels it, and the renderer's watchdog gives up on a page that takes too
long (20 seconds a try) and says so.

The pages around the one on screen are prepared ahead, so turning to them is instant. Jumping
further away waits for the page to render.

Quitting always puts the terminal back as it was, including after an error. A pager killed by a
signal (`kill`, a closed SSH session) can leave the terminal in raw mode; `reset` fixes that.

### The status line

The status line keeps its right side in place whatever you do: where you are (`page 3/12`,
`pages 3–4/12`), padded to the widest it gets in the document, what a Markdown or HTML file is laid
out to when it isn't the window's width or its text isn't at 100% (`width 72`, `font 120%`, with `…`
after the value it is being laid out to again, and `width 120 (window)` for a width the window doesn't
hold), the zoom, and `? help`, with
`…` before them while the pages on screen render. A narrow window drops the width, then the zoom,
then `? help`. The
name is on the left, cut in its middle when it is too long (`IMG_2…41.jpg`, so names that differ
at the end still differ), and a message (a page that failed, `fully zoomed in`, a page number
being typed) sits just left of the right side, cutting the name shorter while it is there. Until
the first key, the message is a reminder of the main keys.

### Copying a page

`c` copies the page you are on to the clipboard as a picture, ready to paste into a document or a
chat: with pages side by side, the first of them, or the one whose number you typed; in a list,
the tile chosen. The status line says `copying page 3…`, then `copied page 3, 1224 × 1584
pixels` (a picture, or a document of one page, by its name), or why not; the keys keep working
meanwhile. A picture is copied at its own size, and anything else at twice the size of its unit:
a PDF's page at 144 pixels per inch (US Letter is 1224 × 1584), an SVG drawing at twice its size.
Either way, at most 8192 pixels a side and 16 megapixels, as a page on screen is, so a large
photograph may be made smaller (`copied photo.jpg, 4899 × 3266 pixels of its 6000 × 4000`).

Where it goes depends on the terminal ([ADR-0009](adr/0009-copy-a-page-through-the-terminals-clipboard-protocol-else-the-systems.md)).
The first `c` asks the terminal whether it has a clipboard for pictures
([kitty's clipboard protocol](https://sw.kovidgoyal.net/kitty/clipboard/)). kitty does: the page
goes to it as a PNG, over SSH and through tmux too, and kitty says when it has it; a terminal
told not to let programs write its clipboard (kitty's `clipboard_control`) says so, and nothing is
copied. Ghostty and WezTerm have only a clipboard for text, so the page goes to this machine's
clipboard instead (the macOS pasteboard, X11), when the terminal runs on this machine; over SSH
it can't reach yours, and the status line says so. On X11 the picture stays on the clipboard only
while `clv` runs, unless a clipboard manager keeps it.

## Several files

`clv *.png` and `clv a.pdf b.pdf` show the files as a list: a tile for each, its first page, its
name under it, in the order given. The arrows choose a tile, its name highlighted and on the
status line with its place (`3/12`), and Enter opens it: its pages, paged through as if it had
been given alone. Esc, Backspace or `u` goes back to the list as you left it. `/` narrows the
list to the files whose names hold what you type ([Filtering a list](#filtering-a-list)).
Printed, a list is its tiles, in rows, each row's names under it.

A list's tiles are all the same size, square in pixels, so moving through it moves nothing else:
up to four to a row until you zoom (fewer when there are fewer), as many rows as fit. The screen
moves only when the tile you choose leaves it, and the tiles show as they render, as a gallery's
do. `--zoom` sets the list's tiles to a row; a file opened from it starts at `auto`. A Markdown or
HTML file's tile is a preview on a portrait page ([Documents](#a-page-the-windows-size)).

`--pages` applies to what is shown, and a list's pages are its tiles: `clv --pages 2-3 *.pdf`
shows the second and third files, each still by its first page and opened to all its pages. The
tiles given keep the list's numbers, as a document's pages given keep theirs: the status line
says `3/12 · 2 of 2`, and a number typed is one of the list's. A number past the end of the list
is an error, as a page past the end of a file is. A pattern that matches a single file gives
that file's pages, as naming it alone does.

A file nothing shows, or a broken picture, among others, is left out: why is said on stderr before
anything is shown, the pager's status line names it at first, and printed pages end with an error
that says how many were left out. With none left, nothing is shown. An option goes to the files that
take it: `--password` to the PDFs among them, and it is refused only when none of them takes it.

Opening a file reads what its backend needs to know its pages: an image's header, but a PDF's whole
file, a document's whole layout, a spreadsheet printed. So a list shows as soon as each file's first
bytes are read (518 Markdown files, in about a tenth of a second on an M3 Max), and a PDF, a document,
a drawing or a spreadsheet is opened only when its tile nears the screen, its tile empty until then,
while the keys keep working. Enter on one still opening says `opening notes.md…` until it can go in,
Esc stops waiting, and `c` on one says it is still opening. Printed, each is opened as it is reached.
Pictures and archives are opened as the list is listed, as that costs little.

One of those that turns out not to open (a broken file, a PDF that needs a password) keeps its tile,
empty, so that nothing moves, its name marked `! notes.pdf`. The status line says why when it is
chosen, and Enter says so and stays in the list. Printed, the rest are printed and the error at the end
names it. One never reached says nothing, as nothing has opened it.

Files are rendered only when their tiles are wanted, and let go of once you are elsewhere, so hundreds
of photos open at once hold little more than their sizes. What a file opened to stays until you leave
the list: for a PDF that is its whole file, however many of them you have come past.

## Directories

A directory is a list too, like a gallery: a tile for each of its files that a backend opens, by
its first page (a directory of PDFs shows their covers), and a folder for each subdirectory,
which Enter goes into, to a list of its own; Esc, Backspace or `u` (up) comes back. A folder with
many files says how far opening them has got (`opening photos/… 120 of 500`), and Esc stops
it. They come in the order of their names, numbers by their value (`img2` before `img10`), letters
regardless of case, subdirectories among the files. Hidden files are left out, and so, without a
word, are files no backend opens, since a directory holds all sorts. A broken picture is left out as
one given by name is, with a warning: before anything shows for the directory given, and once the
screen is back for one gone into. A PDF that needs a password, or anything else that is only opened
when it is reached, keeps its tile and [says why there](#several-files). The status line names each
file from the directory given (`2024/beach.jpg`).

With `-r` (`--recurse`), each subdirectory's files are tiles in the list instead of a folder, at
its place among the names, depth first. A link to a file is followed; a link to a directory is
not, so links can't make a loop. A listing stops at its [limits](#limits), so `clv -r ~` stops in
time, and a warning says when it stopped early.

A directory and files can be given together, and so can several directories: each directory is a
folder at its place among the arguments (with `-r`, its files, named from it: `photos/a.jpg`).

## Archives

A zip archive is shown as a directory is: a list of the files in it that a backend opens, read
from the archive, and a folder for each directory in it. Given alone it is that list; among other
files, or in a directory, it is a folder, tied up as a parcel (tape down and across it, and a bow)
where a directory's is plain, which Enter goes into, and the status line names its files from it
(`trip.zip/2024/beach.jpg`). `-r` lists its directories' files instead of folders when the archive
is given; an archive in a directory stays a folder even then, since `-r` goes through directories,
not into every archive (a jar, a document that is a zip inside). An archive in an archive is a
folder too.

From a zip, nothing is unpacked to disk: a file in it is read from it when it is drawn, so an
archive open holds its list of names, not its files. Hidden names are left out, as on disk, and so
is macOS's `__MACOSX` directory. It reads deflate, deflate64, bzip2, LZMA and XZ. A file past the
[limits](#limits), an encrypted one, or one packed in a way this build doesn't read (zstd) is left
out with a warning, as a broken picture in a directory is.

A tar archive, plain or compressed with gzip, bzip2 or xz (`.tar.gz`, `.tgz`, `.tar.bz2`,
`.tar.xz`, `.cbt`…), is shown the same way. A compressed tar has no list of its files and can't be
read from the middle, so its files are found by reading it through, once, when it is given or gone
into: the status line says how far (`reading photos.tar.xz/… 40%`), then how many of its files are
opened, and Esc stops it (`q` still quits). Given on the command line, it says so on stderr
instead, where Ctrl-C stops it. That reading takes as long as unpacking the archive does: on an M3
Max, gzip unpacks some 2,500 MB a second, xz 260 to 420, and bzip2 only 30 to 110. It unpacks the
archive into a temporary file that nothing names, removed as soon as it is made, which goes when
you leave the archive, and in any case with clv, however clv ends; files are read from there, in
any order. A file past what that holds is unpacked again from the start each time it is read, which
is slow, and so is every file when there is no temporary directory to write to. A plain tar is read
where it is, with nothing unpacked. Links, devices and GNU's sparse files are left out. Zstd isn't
read.

A file compressed alone with gzip, bzip2 or xz, not a tar, is shown as the file it unpacks to:
`paper.pdf.gz` as the PDF. A file compressed twice is not shown, and neither is one that unpacks
to what nothing shows, or one only its name would tell (`notes.md.gz`, `data.csv.gz`). An option only the unpacked file's backend takes (`--password` for a
`.pdf.gz`) is refused, as an option no backend takes is.

## Filtering a list

`/` filters a list by name: as you type, only the tiles whose names hold what you typed stay, in
their order, upper and lower case alike. The name is the one under the tile: `beach` keeps
`Beach.jpg` and the folder `beach/`, and with `-r`, `2024/` keeps that directory's files. The
status line shows `/beach` as you type, and where you are among the tiles that match:
`7/40 · 2 of 3`, the tile's number in the list, then its place among the matches; or
`no match`. Every key that types a character is part of the filter, `q`, `j` and digits too
(`Ctrl-C` still quits). `Backspace` takes the last one back, and the arrows, `PgUp`, `PgDn`,
`Home` and `End` move among the matches.

`Enter` keeps the filter, and the list's keys move among the matches as they do among all the
tiles: `Enter` again opens the tile chosen, and `/` edits the filter. `Esc` while typing puts the
list back as it was before `/`. With a filter kept, `Esc`, `Backspace` or `u` clears it before
they go back or quit. With nothing typed, `Enter` or `Backspace` leaves no filter. The tile chosen
stays chosen while it matches, and otherwise the first after it that matches is chosen; when the
filter changes or is cleared, it stays in its row of the screen where the list allows. A number
is still the list's: `12 Enter` chooses the 12th tile if it matches, and says it doesn't
otherwise, as it says of a tile `--pages` left out.

The tiles keep the size the whole list gives them, so the ones that match come together at once,
from what is already in the terminal, and only they are rendered and sent ahead
([ADR-0010](adr/0010-a-filtered-list-keeps-the-whole-lists-tile-size.md)); `+` makes them bigger.
A filter belongs to its list: a folder gone into starts with all its files, and going back finds
the list filtered as you left it. With `--pages`, the filter narrows the tiles given.

Letters past ASCII (`café`) are typed as the terminal sends them, in UTF-8. Upper and lower case
match letter by letter (`É` matches `é`), and that is all: `e` doesn't match `é`, nor does an `é`
typed as one letter match a name that stores it as `e` and an accent, as macOS's HFS+ did. In a
file's pages, `/` only says that it filters lists: there is no search in a document's text.

## Which files it shows

A file is shown by the first backend that opens it, going by how the file starts, and by its name
only where that can't tell. `clv --help` ends with the formats this build shows, under what each
backend shows and the libraries it shows them through, and the extensions of those that go by their
name, so a build without a backend lists none of its formats. Before them come the options a
backend takes besides the ones above, under it; a backend that takes none has no heading. An option
that only another backend takes is refused, since it would do nothing for the file.

| Files | Known by | Their options |
|---|---|---|
| [SVG drawings](#svg-drawings), and svgz, gzipped | XML whose root element is `<svg>`, after whatever may come first (a declaration, comments, a doctype), and svgz by what it unpacks to; a `.svg` or `.svgz` name when the first 1024 bytes end before the root element. Other XML is not one | none |
| [Word documents, OpenDocument text, RTF, Markdown and HTML](#documents) | An RTF by how it starts (`{\rtf`), whatever its name; an odt by what the zip it is says it holds; a docx by what its zip holds, or a Word name (`.docx`, `.docm`, `.dotx`, `.dotm`, `.doc`, `.dot`); a Word 97-2003 document by such a name only, since spreadsheets and slides are compound files too; HTML by how it starts (a doctype or an `<html>` element), whatever its name, or a name (`.html`, `.htm`, `.xhtml`) and that it is text; Markdown by its name only (`.md`, `.markdown`, `.mdown`, `.mkd`), and that it is text | `--password PW`: the password of a protected Word document |
| [Spreadsheets](#spreadsheets) | An ods by what the zip it is says it holds; an Excel workbook by what its zip holds, or an Excel name (`.xlsx`, `.xlsm`, `.xltx`, `.xltm`, `.xlsb`, `.xls`, `.xlt`); an Excel 97-2003 workbook by such a name only; values separated by commas or tabs by their name only (`.csv`, `.tsv`, `.tab`), and that they are text | `--password PW`: the password of a protected workbook |
| [PDFs](#pdfs) | `%PDF-` in their first 1024 bytes | `--password PW`: the user or the owner password of an encrypted PDF |
| [Images](#images) | Their signature: PNG, JPEG, GIF, WebP, TIFF, BMP, ICO, QOI, JPEG XL, OpenEXR, Radiance HDR, PNM and farbfeld; and TGA, which has no signature, by its `.tga` name | none |
| [Zip archives](#archives) | Their signature (`PK`), whatever their name: jars, cbz comics, epubs and other zips that aren't documents or workbooks, as directories | none |
| [Tar archives and compressed files](#archives) | A POSIX or GNU tar header, as directories; files compressed with gzip, bzip2 or xz by their signature: a compressed tar as an archive, anything else as the file it unpacks to | none |

### PDFs

PDFs are rendered by [pdfcraft](https://github.com/storytold/pdfcraft), the way its app draws them:
comments without an appearance of their own are drawn too, and a dynamic XFA form shows the pages
its template lays out into, not the placeholder page the file carries for viewers that can't. A
page whose content runs past pdfcraft's safety budget is drawn without the rest of it, and a colour
it can't compute (a colour space it can't read, a tint transform it can't evaluate) is painted in a
fallback colour, each with a warning.

### Images

An image is one page, its size its own in pixels, turned the way its EXIF orientation says (a
photo taken with the phone on its side shows upright). It is fitted to the window like a page,
but never made larger than it is: a pixel of the picture to a pixel of the screen at most, so an
icon shows as small as it is. `--zoom width` (and `+` to the width) still makes it as wide as the
window. The first frame of an animation is shown. Colour profiles are not applied yet: a picture
in a wide-gamut space (Display P3, Adobe RGB) shows a little duller than it should. AVIF and DDS
files are recognised and refused with their format's name: the `image` crate's AVIF decoder gets
most AVIFs' colours wrong in 0.25.10, and it has no DDS decoder.

### SVG drawings

An SVG drawing is one page, rendered by [resvg](https://github.com/linebender/resvg) at the size
it is shown, so it stays sharp however far it is zoomed. Its size is its own (`width` and
`height`, in CSS pixels at 96 per inch), or its `viewBox`'s; one that says neither is as large as
what it draws reaches from its origin, as resvg sizes it, and 100 × 100 if it draws nothing.
Unlike a picture, it is fitted to the window as a PDF's page is, made larger as well as smaller:
a 24-pixel icon fills the window. Text is drawn with the system's fonts; a `font-family` the system
lacks falls back to its serif font, and a drawing with no fonts to draw its text with is drawn
without it, with a warning. Pictures embedded in it (JPEG, PNG, GIF and WebP `data:` URLs) are
drawn, and so are SVGs embedded in it.

A drawing reads nothing but itself: a picture, a stylesheet, a font or another drawing it links
to by a path or a URL is left out, and a warning on the page says how many links weren't
followed. An entity declared outside it (`SYSTEM`) is never read.

What a drawing costs is bounded before it is spent
([ADR-0003](adr/0003-bound-what-a-drawing-costs-before-usvg-and-resvg-spend-it.md)): past its
[limits](#limits), it is refused, what is too much of it is left out with a warning, or a page
fails with why. A render once begun can't be stopped: a page within the limits but slow still takes
its time on a render thread, while the keys work.

### Documents

A Word document (docx, or a Word 97-2003 doc), an OpenDocument text (odt) or an RTF is laid out
into pages by [wordcraft](https://github.com/storytold/wordcraft), as its app prints them: each
page its own size, in points, fitted to the window as a PDF's is; headers and footers, tables,
lists, pictures and shapes; tracked changes marked up, as Word prints them unless told not to
(what was deleted struck through, what was inserted underlined, a bar beside each changed line);
no comment balloons, no proofing marks. Text is set in the fonts it names when the system has
them, and otherwise in the ones wordcraft substitutes, its own included (Source Serif, Source
Sans, Inter, JetBrains Mono). Fonts a document embeds are not used. A protected Word document opens
with `--password`; without it, clv says it needs one.

Markdown and HTML are laid out the same way, on pages the window's size, since they give no page size
of their own ([A page the window's size](#a-page-the-windows-size)), or to a width of your own.
Markdown is read as GitHub writes it, with its tables, task lists, strikethrough, footnotes and alerts
(`> [!NOTE]`), and the HTML in it ([ADR-0014](adr/0014-markdown-through-pulldown-cmark-into-wordcrafts-html-importer.md));
front matter (`---`) is left out. HTML is read for its text and what its tags and CSS say of it
(headings, lists, tables, colours, margins): its `style` attributes, and the simple rules of its `style`
elements (`p`, `.note`). Scripts and linked stylesheets are skipped, so a page designed with CSS shows as
its words in order, not as a browser draws it, and one that counts on rules that aren't read can come out
oddly: a dark theme's colours on white paper, say
([ADR-0017](adr/0017-htmls-style-rules-apply-as-far-as-wordcraft-reads-them.md)). An epub's chapters, being XHTML, open from
its list the same way. What may cost too much to lay out is refused, at its [limits](#limits)
([ADR-0012](adr/0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md)).

They say little of how they should look, so clv gives them a look of its own, the same on any machine:
text in Source Sans and code in JetBrains Mono, both of which come with it; headings falling in size, the
first two above a line; code blocks on a faint wash, where 80 columns fit on a line; quotes beside a bar;
tables in fine lines, under a shaded header; links in blue; and an alert's kind, in its colour, at the
top of its quote. What HTML's own CSS says of its text comes first
([ADR-0016](adr/0016-how-markdown-and-html-look.md)).

The pictures Markdown and HTML refer to are drawn by whatever shows them: anything `clv` shows as an
image, an SVG drawing, or a PDF's first page, at its own size, or smaller to fit the column and the
page. They are read when a page they are on first shows, so opening a document, or a list of them,
reads none.

Only what is near the file is read: a path from its directory, `..` included. **Nothing is fetched**:
a picture on the web shows as its alt text. So does an absolute path, which on the web means from a
site's root; one that is missing; and a file that is no picture, such as another document. What comes
through a pipe or from an archive has no directory, so none of its pictures can be reached, an epub's
included. A picture in a `data:` URL is drawn, if it is a PNG, JPEG, GIF, WebP or BMP. The first page
says how many pictures on this machine were left out, and a page says if one of its own can't be drawn,
a grey box in its place ([ADR-0013](adr/0013-a-documents-pictures-are-drawn-by-the-backends.md)).

Markdown and HTML have no paper of their own, so on a terminal with a dark background they go light
on dark, the page exactly the terminal's colour, so that it is no rectangle at all. `d` turns them to
dark on light and back, `--theme` says how they start, and a terminal that doesn't say what its
background is gets them as printed, with `d` still going to a dark grey. Pictures stay as they are,
but for thin near-black ink, a logo or a line drawing made for white paper, which would vanish on dark:
it comes with white behind it. A Word document, an RTF or a PDF is a page as printed, and stays so;
`d` says so. What `c` copies is the page as it is shown
([ADR-0015](adr/0015-markdown-and-html-on-dark-paper-the-terminals-own-colour.md)).

Opening a document reads and lays it out whole, to know its pages: a quarter of a second for 500
pages, on an M3 Max, and one slow to lay out holds up its opening. Shown, those 500 pages hold about
200 MB. Why it is read this way, and not through wordcraft's engine, is in
[ADR-0005](adr/0005-word-documents-through-wordcrafts-readers-layout-and-renderer.md).

#### A page the window's size

A Markdown or HTML page is the window, as on an e-reader: as tall as the pages area (the window less
the status line), and as wide, or, where the window holds two pages of 60 characters each, two side by
side like an open book, each half the window. A character of the code face (JetBrains Mono at 9.4 pt)
is a cell of the window, so a window 100 columns wide puts 96 characters of code on a line, between
margins of two columns at each side and a row above and below. A page fills the window at that size,
and is never shown larger: zoomed out, two or more to a row, it is smaller, as any page. In a window
too small for 20 characters a line and 10 lines of text, the page holds those all the same, shown
smaller to fit. Through a pipe or to a file, the pages are laid out for the terminal the command
runs in; with no terminal at all, on the preview's page below. Inside tmux, the pane is the window
([Inside tmux](#inside-tmux)).

The pages are laid out when the file opens, so a large file takes its time before anything shows
(`Ctrl-C` stops it, the terminal as it was). When the window changes size, the document is laid out
again for it once the size has held for half a second: until then the old pages are fitted to the
new window, and while it is laid out the status line says `laying out…` (or the width to come, when
it isn't the window's), the old pages staying until the new ones are ready. A window still being
dragged is waited for; one dragged back to its size lays out nothing. On a terminal that doesn't
report its cells' size in pixels, the first layout takes the cell assumed, and the document is laid
out once more as the pager starts, once `clv` has asked the terminal, where the cell it says makes
another page.

**The size of the text.** `+` and `-` make a Markdown or HTML file's text larger or smaller, in steps
of 10% from 50% to 400%, laying the document out again: larger text puts fewer characters on a line
and fewer lines on a page, so there are more pages, each still the window's size. At 100%, the
default, a character of the code face is a cell wide. A number, then `+` or `-`, goes straight to it
(`150+`), `0+` back to 100%, and `--font 150` says where to start. Scaling the picture, what `+` and
`-` do elsewhere, has no use on a page the screen's size, so on these files they are the font size;
on a PDF, a Word document, in a list, and with `--pages`, they zoom as they always did, and `?` says
which where you are. A small window can't reach the larger sizes, since a page holds 20 characters a
line and 10 lines at least: `+` stops there and says so (`font 170%, the most here`), and a size the
window can't hold (given, or left so by a smaller window) is kept, laid out at the most it holds, and
said as `font 300% (170%)`. The status line shows the size while it isn't 100% (`font 120%`). The
font size is a most, not a promise: two pages share a row where each keeps 80% of its size alone, so a
width between about half and five eighths of the window shows two pages a little smaller than the
size asked, while `window` never does.

**A width of your own.** `]` and `[` lay a Markdown or HTML file out again to lines 4 characters
wider or narrower, counted in the code face, from 20 up to the longest line the window holds, with
`window` among the steps where it sits: in a window of 100 columns, 92, 96, `window`; in one wide
enough for a book, `window` sits at a page's 60-odd characters, and the steps go on to the window's
width. A number, then `]` or `[`, goes straight to it (`72]`), `0]` back to `window`, and
`--width window|72` says where to start. The text area is that wide, between the same margins, and
every page stays as tall as the window; a code block, indented a little on each side, holds about 3
characters fewer; tables and pictures fit the width. Narrow pages go two to a row when the window
takes them, as any page does. A width the window doesn't hold (typed, given, or left so by a smaller
window) is kept, laid out at the window's width, and grows back when the window does; the status
line says `width 120 (window)` meanwhile. The status line shows the width while it isn't the
window's (`width 72`), and `c` copies the page as it is laid out. You keep your place: the paragraph
at the top of the screen is on the page you land on. Laying out again costs what opening did: the
old pages stay until the new ones are ready, the status line shows the width to come with `…` after
it (`width 84…`), and the keys keep working meanwhile. Another step while one runs waits its turn,
and only the last is laid out. While it runs, the old layout is held too (see [Limits](#limits)); a
layout that fails keeps the pages as they are, says why, and leaves the width at what is on screen.

Only the document on screen reflows. A list's tiles are previews, laid out once on a portrait page
of 60 × 40 cells whatever the window, since tiles are square; `[` and `]` say so there, and a
document opened from the list is laid out for the window, at the width and font size wanted then, as
it opens. With `--pages`, which picks pages as laid out, `[` and `]` say so and change nothing. A
Word document, an RTF or a PDF has a page of its own, and `[` and `]` say so on one
([ADR-0022](adr/0022-markdown-and-html-reflow-to-a-width-in-characters-as-a-new-level.md),
[ADR-0023](adr/0023-the-window-is-the-page-of-markdown-and-html.md)).

### Spreadsheets

A workbook (xlsx, xlsm, xlsb or xls), an OpenDocument spreadsheet (ods) or a file of values
separated by commas or tabs (csv, tsv) is shown as [gridcraft](https://github.com/storytold/gridcraft)
prints it: each visible sheet's pages, laid out by its print area, page setup, title rows,
headers and footers, gridlines and charts, in the order of its sheets; a hidden sheet prints
none. Its formulas are recalculated first, as gridcraft's app does when it opens a workbook, so a
formula saved without its value shows one. gridcraft prints the pages to a PDF, which the PDF
backend shows like any other, fitted to the window. A CSV's columns are made as wide as what they
hold, as gridcraft's app makes them on importing one. A protected workbook opens with
`--password`; without it, clv says it needs one.

What gridcraft says on reading a workbook comes as a warning with its first page: a sheet cut
short at one of its [limits](#limits), and that an ods or an xlsb is shown by its values alone
(gridcraft reads their formatting, charts and pictures from neither). So does text in letters other
than Western European ones (Greek, Cyrillic, Chinese…, and symbols such as `≤`): the PDF gridcraft
prints to sets it in a font it doesn't carry, and the PDF backend has none to put in its place, so
that text is blank.

Opening a workbook reads it, recalculates it and prints every page, then lets go of it: what is
kept is the PDF. On an M3 Max that took 6 ms for a styled workbook of four sheets, and 1.5 s for
a 7.6 MB one of 37,000 rows that prints to 1,648 pages. Why spreadsheets are printed this way is in
[ADR-0006](adr/0006-spreadsheets-printed-by-gridcraft-to-a-pdf-the-pdf-backend-shows.md).

## Which terminals work

kitty, Ghostty and WezTerm speak the
[kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/), among others. It
works on macOS, Linux and FreeBSD, and [inside tmux](#inside-tmux). Inside Zellij too: pages showed
in a quick try of 0.45.1, though no test covers it.

`--protocol auto` trusts the environment for kitty (`TERM=xterm-kitty` or `KITTY_WINDOW_ID`) and
Ghostty (`TERM=xterm-ghostty` or `TERM_PROGRAM=ghostty`). Any other terminal is asked with the
query the protocol recommends, and has half a second to answer, unless `TERM` says it takes no
escape codes at all. A terminal that doesn't speak the protocol gets [text cells](#text-cells) instead, and is told so: with a warning on stderr when
printing, and on the status line in the pager. `--protocol blocks` asks for them without the
warning.

Windows isn't supported yet: Windows Terminal speaks sixel, not the kitty protocol. Sixel may come
later, which would also bring the terminals that speak only sixel.

## Text cells

Without images, a page is drawn as text: any terminal shows it, tmux keeps it as it keeps other
text, and it can go to a file. It is a preview: a cell holds very little, so pictures and drawings
show what they are, and a document shows its layout, but its body text won't be readable, even as
wide as the window.

| `--protocol` | Each cell | Holds | Needs |
|---|---|---|---|
| `blocks` | `▀`, its foreground above and its background below | two colours | a terminal with colours, and a font with the block characters |
| `ascii` | one of ` .:-=+*#%@`, more ink for more brightness | one sample, in its colour | nothing |

What else to know:

- **Colours** go by the environment. `NO_COLOR`, set and not empty, means none. `COLORTERM=truecolor`
  or `24bit` means 24-bit, as does tmux saying its terminal takes them, or a `TERM` that says so
  (`xterm-kitty`, `xterm-ghostty`, one ending in `-direct`). `TERM` unset, `dumb` or `vt…` means none,
  and anything else xterm's 256.
- **Without colours**, `blocks` shades each cell, with `░▒▓█`, one sample to a cell, and `ascii` is
  those characters and nothing else.
- **To a file or a pipe**, `ascii` is plain text, whatever the environment says: nothing but ASCII
  characters and line ends. `blocks` keeps its colours, as it has to be shown on a terminal anyway.
- **The background.** What is transparent shows the terminal's background. Characters show against
  it too, so `ascii`, and `blocks` without colours, ask the terminal what it is and draw light on a
  dark one and dark on a light one. A terminal that doesn't say, or output to a file or a pipe, is
  taken to be dark, and a light one would then show a negative.
- **Their size.** A page fits the window as it would as an image, and in the terminal's cell's
  shape. A picture's natural size is about a column for each of its pixels across, as long as the
  window takes it, so a small one is larger on screen than it would be as an image.
- **`c`** copies the page as it is, not as the cells show it.
- **`--compression`** only applies to what `c` copies.
- **A terminal that takes no escape codes**, as `TERM` says (`dumb`, `vt…`, or none), isn't asked
  anything and doesn't get the pager: every page is printed, in characters alone unless `COLORTERM`
  says otherwise.
- **Double-width blocks.** Some terminals set up for Chinese, Japanese or Korean draw `▀` two columns
  wide, which breaks up the page; `--protocol ascii` doesn't have that problem.

## Size and sharpness

Each page is rendered for the window it is shown in, at the terminal's own pixel density, so text
stays sharp on high-DPI screens. One row is left free for the shell prompt. Pages are never
rendered larger than 8192 pixels on a side or 16 megapixels in all (more than a 4K screen shows),
which bounds the memory one page can take, however its document shapes it.

The size in pixels usually comes with the window's size, as it does over SSH. Some ways in pass
on only rows and columns, such as `docker exec` and `kubectl exec`; then `clv` asks the terminal
how big its cells are, once, when it starts, and assumes 10 × 20 pixel cells if the terminal
doesn't say. Inside tmux, tmux says. When there is no terminal to ask (output to a file with
`--protocol kitty`, no controlling terminal), `clv` assumes an 80 × 24 window.

## On this machine or over SSH

When the terminal runs on the same machine, the pager hands it pages through temporary files, as
the protocol allows, which kitty takes much faster than PNGs. They are new files that only you can read, in a directory of your own,
`tty-graphics-protocol-clv-<uid>`, inside `TMPDIR`, else `/dev/shm`, else `/tmp`; the terminal
deletes each as soon as it has read it, and any it leaves are removed when the pager ends, or, if
it was killed, by the next one. Whether the terminal reads files from this machine is found out by
trying, at the start. Until it does, and for good when it doesn't, pages travel as PNGs inside the
escape codes, as they do when printing, so saved or piped output still works.

Over SSH the image travels inside the escape codes too, so `clv` works there as well: the bytes
cross the network, so it compresses small and doesn't try temporary files. It goes by
`SSH_CONNECTION`, `SSH_CLIENT` and `SSH_TTY`, and inside tmux by how the tmux client attached.
`--compression fast` or `--compression small` overrides the guess, and sends PNGs even on this
machine. A small PNG takes longer to make and a fast one takes more bytes; the pixels are the same
either way.

## Inside tmux

`clv` works inside tmux 3.3 and later when tmux runs in a terminal that speaks the protocol:

- tmux doesn't know about images, so the escape codes go through tmux's passthrough. tmux only
  lets them through when the pane's `allow-passthrough` option is `on` or `all`. The option lets
  any program in the pane send escape codes past tmux, so `clv` never changes it. If it is off,
  `clv` stops and says how to turn it on: for every pane, add `set -g allow-passthrough on` to
  `~/.tmux.conf`; for one pane, run the `tmux set -p -t <pane> allow-passthrough on` it shows.
- Pages are drawn with the protocol's *Unicode placeholders*: special characters that tmux stores
  as text, which the terminal draws as the image. The page scrolls with the pane, and survives
  switching windows and panes. Placeholders number at most 297 rows and columns, so inside tmux a
  page is never drawn larger than that.
- Inside tmux the pane is the window: a Markdown or HTML page is the pane's size, and the pane
  resized lays the document out again. tmux says the terminal's cell size once, at the start, so a
  font change in the terminal while `clv` runs isn't seen; the next run takes it.
- Inside tmux, `TERM` describes tmux, not the terminal, so `clv` asks tmux which terminal it runs
  in. `xterm-kitty` and `xterm-ghostty` are trusted. Anything else gets [text cells](#text-cells)
  and a warning, unless `--protocol kitty` is given. With `allow-passthrough` off, a trusted terminal
  doesn't get them: `clv` stops, as above, since one command gets the images.
- [Text cells](#text-cells) need none of this, or what follows: they are text in the pane.
- Pages turned while the pane is hidden (another window in front, or the window zoomed on another
  pane) arrive when it shows again. A terminal that attaches to the session later gets the pages
  too.
- With more than one terminal showing the pane, pages go as PNGs, not temporary files.
- tmux throws output away when it decides the terminal can't keep up, images with it. `clv`
  notices and sends the pages again. If tmux keeps dropping them, the status line says so, and
  `r` sends them again.
- A printed page stays in the terminal after `clv` exits, so its placeholders show it again when
  they scroll back, and the terminal holds its pixels until something deletes it. Each later run
  inside tmux deletes the pages no pane holds any more: after `clear-history`, a closed pane, or
  the history past `history-limit`. Until the next run they stay. The deletes reach the terminals
  showing the pane that run is in, so a terminal attached only to another session keeps its copies.
  Closing the terminal's tab frees them all.

The terminal itself must support Unicode placeholders. kitty does. Other terminals may draw nothing
inside tmux even though they draw pages outside it.

## Limits

Any file may be large, or made to cost a lot, so what `clv` reads and holds is bounded. Past a
limit, it does as the last column says. How large a page is drawn is under
[Size and sharpness](#size-and-sharpness).

| What | Limit | Past it |
|---|---|---|
| A directory, or a directory in an archive | 4,096 files listed | the listing stops, with a warning |
| A directory's entries read, or an archive's | 100,000 | the listing stops, with a warning |
| Directories down, with `-r` or in an archive | 32 | what is deeper is left out, with a warning |
| A file in an archive, unpacked | 512 MiB | left out, with a warning |
| What a compressed tar unpacks into its temporary file | 4 GiB | a file past that is unpacked again each time it is read |
| A compressed tar, read through | 32 GiB | the listing stops, with a warning |
| A file compressed alone, unpacked | 512 MiB | refused |
| An image's pixels | 512 MiB (about 134 megapixels) | refused when it is opened |
| A page in [text cells](#text-cells) | 4,194,304 cells | the page fails with why |
| An SVG file, or what an svgz unpacks to | 64 MiB | refused |
| An SVG's content, what it repeats counted: its entities expanded, each `use` copied, a marker for each vertex it marks, and the clip path, mask, filter or paint a shape refers to for each shape | 128 MiB, a million elements, 1024 levels deep, or stylesheet rules times elements past 50 million | refused before it is parsed |
| An SVG's text, its `use`s counted | 200,000 characters, or about three seconds to lay out (a `<text>` of 70,000 characters, or of 20,000 in 1,300 `<tspan>`s) | that text is left out, with a warning, and the rest drawn |
| What an SVG embeds | a picture of 64 MiB decoded; a drawing four deep | past that, or a drawing that is broken, it is left out and the rest drawn |
| What an SVG's page takes to render: a layer for each group with opacity, a clip path, a mask or a filter, a filter's buffers, a pattern's tile, and the pixels painted | 1 GiB held at once; 4,000 million pixels painted (about seven seconds) | the page fails with why; the same drawing smaller, its tile in a list, still renders |
| A Word document, OpenDocument text or RTF | 256 MiB | refused; what else it may cost is bounded by wordcraft's own limits (how much a docx unpacks to, how deep it nests) |
| A Markdown or HTML file | 64 MiB; half a million lines and marks (HTML's tags, Markdown's punctuation); 8 MiB of text, some 5,000 pages, which take 20 seconds to open and 1.6 GB | refused. Within them, a file made to cost the most takes about 6 GB and 15 seconds, at 20 characters a line as at the window's width, at the largest font size a window holds as at the smallest; laid out again (`]`, or the window resized), with the old layout held until the new one shows, that file peaks at 9.5 GB |
| A document's pictures, in Markdown or HTML | 1,024 referred to; each held at 1,600 pixels at most on its longer side; 256 MiB held in all | past 1,024, left out, with a warning; past 256 MiB, those drawn longest ago are let go of, and drawn again when wanted |
| A document's page | 64 million pixels | the page fails with why |
| A spreadsheet's file | 64 MiB | refused |
| An xlsx's sheets, unpacked, together | 128 MiB, which would take some 850 MB once read (one of 97 MiB took 649 MB) | refused |
| A sheet's XML, as gridcraft reads it | a million elements (about 37,000 rows of twelve columns) | the sheet is cut short, with a warning |
| A workbook's pages, as gridcraft prints them | 2,000, down a sheet's rows before across its columns | a very wide sheet loses its right-hand pages |

## Errors

- A page number past the end of the document fails before anything is drawn.
- A page that can't be rendered is reported at the end. The other pages are still drawn, and
  `clv` exits with an error.
- What a backend warns about on a page (content it left out, a colour it had to guess) is said
  with the page's number: on stderr when printed, on the status line in the pager.
- File names and messages are written with their control characters replaced, so neither a file
  name nor a document can send escape sequences (a clipboard write, a window title) to the
  terminal through them.
