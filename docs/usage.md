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
clv --per-row width report.pdf  # as wide as the window
clv --password PW secret.pdf
```

| Option | Meaning |
|---|---|
| `--pages 1,3-5` | the pages to show, single pages and ranges, in the order given (default: all of them); in a list, the tiles |
| `-r`, `--recurse` | in a directory, its subdirectories' files as tiles of their own, not folders |
| `--per-row auto` | two pages side by side when the window is wide enough, otherwise one (the default) |
| `--per-row width` | one page as wide as the window; a tall page runs past the bottom |
| `--per-row 3` | that many pages to a row, 1 to 8, in as many rows as fit the window: each page a third of the width, even when there is only one. In a list, that many tiles to a row. A Markdown or HTML file is laid out in that many columns ([Documents](#a-page-the-windows-size)); a `--width` of a number wins over it there, with a warning ([The config file](#the-config-file) says what happens when the config sets one of the two) |
| `--zoom` | `--per-row`'s old name, with the same values: still taken, with a warning, and to go in a later release. Not with `--per-row` |
| `--pager` | always page, even through a single page |
| `--no-pager` | never page: print the pages and exit |
| `--theme auto` | [Markdown and HTML](#documents), a [book](#books), an [Org document](#org-documents), and a [Mermaid diagram](#mermaid-diagrams) shown by itself, light on dark on a terminal with a dark background, dark on light otherwise (the default); taken whatever the files, and doing nothing to the rest. With documents or Mermaid diagrams, which `--help` lists it under, each in its words |
| `--theme light`, `--theme dark` | Markdown, HTML, books, Org documents and Mermaid diagrams that way, whatever the terminal |
| `--width window` | Markdown, HTML, [books](#books) and [Org documents](#org-documents) on pages as wide as the window, or two to a row where it is wide (the default); taken whatever the files, like `--theme`. With documents |
| `--width 72` | Markdown, HTML, books and Org documents laid out to lines of that many characters of code, 20 to 200, the pages as tall as the window ([Documents](#documents)) |
| `--font 150` | Markdown, HTML, books and Org documents' text at that share of its size, 50 to 400 percent; at 100, the default, a character of code is a cell ([Documents](#documents)). With documents |
| `--margin 5` | Markdown, HTML, books and Org documents' side margins of that many characters of code, 0 to 20 (3 by default) ([Documents](#a-page-the-windows-size)). With documents |
| `--fences drawn` | the [fenced blocks](#documents) of Markdown and HTML, and the source blocks of [Org](#org-documents), that `clv` draws (` ```mermaid `, `#+begin_src mermaid`) shown as pictures of them (the default); taken whatever the files. With documents |
| `--fences code` | those blocks shown as the code they are |
| `--compression auto` | on this machine, temporary files when the terminal takes them and `fast` otherwise; `small` over SSH (the default) |
| `--compression fast` | PNGs with the least work: pages ready sooner, about twice the bytes |
| `--compression small` | PNGs with the fewest bytes, for a slow link |
| `--protocol auto` | images if the terminal can show them, otherwise [text cells](#text-cells) and a warning (the default) |
| `--protocol kitty` | skip the check and send the escape codes anyway, even to a file or a pipe |
| `--protocol blocks` | [text cells](#text-cells) of half-blocks and colours, even where images would show |
| `--protocol ascii` | [text cells](#text-cells) of ASCII characters |
| `--config FILE` | read this file as the config, not your own ([The config file](#the-config-file)). The file must be there |
| `--no-config` | read no config. Not with `--config` |
| `--generate-config` | write a config that sets nothing, each setting commented out, where clv reads one; print its path, whole; and exit ([The config file](#the-config-file)). It takes no file |
| `--print-config` | print the config in effect, from the file and the command line, and exit. It takes no file |

Printed without the pager, pages follow the zoom too, a row of them under another: `--per-row 3`
prints three to a row, `--per-row width` one to a row as wide as the window, and `auto` two to a row
when the window is wide enough, as the pager first shows them.

`clv --help`, or `clv` alone, lists these, then the options a backend takes of its own
(`--password`, for PDFs, documents and spreadsheets: see
[Which files it shows](#which-files-it-shows)), each under what that backend shows, then the
formats this build shows, with the libraries each shows them through. It starts with clview's
version and what clview is, in a line each, and ends with a link to this manual and where to report
issues and ideas: <https://github.com/dobladez/clview>.
A `clv` built from source between releases says which commit in that version, as `--version` does:
`0.3.0+50.g3f677cb` is 50 commits past 0.3.0, at commit `3f677cb`, and `.dirty` after it says it
was built with changes not committed.

In kitty and Ghostty, [inside tmux](#inside-tmux) too, clv's icon sits to the right of the help's
first lines, smaller or not at all when the window is narrow. Everywhere else, and through a pipe
or into a file, the help is the same text without it. `clv` asks the terminal nothing for it, so a
terminal the environment doesn't name gets no icon, WezTerm among them, though it shows pages.

## The config file

clv reads a config file when it starts. The file sets options, so that you do not type them each
time, and it binds keys to commands. clv reads the file when it prints pages, too.

```toml
per-row = "width"
theme = "dark"
width = 72
recurse = true

[keys]
ctrl-d = "screen-down"
x = "none"
"/" = "search"

[keys.list]
J = "list-tile-down"
```

**Where it is.** clv reads `$XDG_CONFIG_HOME/clv/config.toml` when that variable holds an absolute
path. Otherwise clv reads `~/.config/clv/config.toml`, on macOS too. When the file is not there,
clv says nothing and uses its defaults. `--config FILE` reads another file, and `--no-config` reads
none.

**A config to start from.** `clv --generate-config` writes a template where clv reads its config:
your own place, or the file that `--config FILE` names. It makes the directory when there is
none, and prints the path that it wrote, whole. clv never writes over a config: when a file is
there, clv refuses and names it. This is the only time that clv writes a config.

Every line of the template that sets something is commented out, so the file sets nothing as
written, and a default that a later release changes still reaches you. A line that starts with
one `#` is an example, and shows the default: to change a setting, remove the `# ` from its line
and edit the value. A line that starts with `##` is an explanation. A banner starts each part
of the file: a line of `#`, the title of the part, and a line of `#`. The template is made for
your build of clv, and holds these:

- each option that a config may set, with its help lines, what it takes and its default
  (`# width = "window"`)
- the three tables of the keys, `[keys]`, `[keys.pages]` and `[keys.list]`, with every key that
  a config can write (`# ctrl-v = "screen-down"`), under what its command does

The three lines that start the tables are not commented out: leave them as they are. An empty
table sets nothing, and a key works as soon as you take its `# ` away. A key that a file's pages
and a list bind alike is in the template once, under `[keys]`: change `# q = "quit"` to
`x = "quit"`, and `x` quits in both. A key that the two bind to different commands is under each
table: `j` is `scroll-down` under `[keys.pages]` and `list-tile-down` under `[keys.list]`.

**The config in effect.** `clv --print-config` prints the config that clv would use, from the file
and the command line. A line that either sets has its value and no `# `. The other lines keep
their `# ` and their defaults. So `clv --no-config --print-config` prints the template. What clv
prints is a config: you can save it and read it back with `--config`. So clv checks the options
first, as it does before it shows a file: `clv --print-config --protocol sixel` prints nothing
and says what `--protocol` takes. clv writes a number as it read it: `--per-row 03` prints
`per-row = 3`. A `width` of the config that a count on the command line wins over is not in
effect, so clv prints it as not set (`# width = "window"`). It prints the keys as the template
has them: a key that both keymaps bind alike under `[keys]`, once, and any other key under the
table of its keymap. A key that you unbound is `"none"` under the command that had it.

**What it holds.** The file is [TOML](https://toml.io). An option goes at the top level, under its
name without the dashes: `--per-row` is `per-row`. The value has one of these forms:

- A number is a TOML integer or a string: `width = 72`, `width = "72"`.
- A named value is a string: `theme = "dark"`, `width = "window"`.
- A flag is a boolean: `recurse = true`. A flag set to `false` is a flag that is not given.

A config may set `per-row`, `compression`, `protocol`, `recurse`, `pager`, `no-pager`, and the
options that `--help` lists under a kind of file: `theme`, `width`, `font`, `margin` and `fences`.
A config may not set these:

| Option | Why not |
|---|---|
| `pages` | it belongs to one run |
| `password` | a password does not belong in a plain file |
| `config`, `no-config` | they say which config to read |
| `zoom` | it is the old name of `per-row`: write `per-row` |

**The command line wins.** An option on the command line wins over the same option in the config.
`--pager` and `--no-pager` are one option for this: `--no-pager` wins over `pager = true`. clv
refuses a config that sets both to `true`. A flag that has no opposite stays on: with
`recurse = true` in the config, only `--no-config` turns it off.

**A width and a count to a row.** `width` and `per-row` both say how wide a page is, and they
disagree on a document that clv lays out for the window
([A page the window's size](#a-page-the-windows-size)). These rules hold there:

- Both on the command line: the width wins, with a warning.
- One on the command line and the other in the config: the one on the command line wins. clv
  gives no warning.
- Both in the config: the width wins on such a document, and `per-row` holds for the other files.
  clv gives no warning. So a config can set the width for documents and `per-row` for PDFs.

**When the config is wrong.** clv stops before it shows anything. The message names the file, the
line and the problem:

```
clv: ~/.config/clv/config.toml:3: width: expected window or 20–200, got 300
```

These are errors: text that is not TOML, a name that clv does not know, an option that a config
may not set, a value that the option does not take, and a key or a command that cannot be bound.
The message for a value is the same as the command line gives. clv refuses a config that is larger than 1 MiB, one that is not a regular
file, and one that it cannot read. `--config FILE` with no such file is an error too.

A config written for a full build can stop a `clv` built without some backends: `width` is a name
that a build with images alone does not know.

### Keys in the config

The table `[keys]` binds a key to a command, by the name of the command, in every keymap where
the command applies. clv has two keymaps: one for the pages of a file, and one for a list. The
tables `[keys.pages]` and `[keys.list]` bind a key in one keymap, and win over `[keys]` there. The
value `"none"` unbinds a key.

- A key that you bind loses what it was bound to. The command keeps its other keys.
- A key that you bind comes first among the keys of its command. So `?`, the reminders of the
  status line and every message that names a key show your key. Two keys of one command keep
  the order that the file has them in.
- A binding that is the same as the default changes nothing.
- A number typed before a key stays as it is: `12` and `enter` go to page 12, whatever you bind
  `enter` to. `enter = "next"` changes `enter` alone.

**How to write a key.** Write a key as `?` does ([Keys](#keys)): `j`, `J`, `ctrl-d`, `alt-v`,
`space`, `enter`, `esc`, `backspace`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup` and
`pagedown`. A key with letters, digits, `-` and `_` only needs no quotes. A punctuation key does:
`"/"`, `"?"`, `"["`. clv refuses these keys, and says why:

| Key | Why |
|---|---|
| a digit | it starts the number typed before a key |
| `ctrl-c` | it always quits, so a config cannot leave you without a way out |
| `ctrl-D` | a terminal sends the same byte for `ctrl-d`: write `ctrl-d` |
| `ctrl-m`, `ctrl-j` | a terminal sends the same byte for `enter`: write `enter` |
| `ctrl-h` | a terminal sends the same byte for `backspace`: write `backspace` |
| `tab`, `ctrl-i` | clv does not read Tab as a key |

clv also refuses a command that does not exist, and a command that does not apply in the keymap
of its table: `[keys.list] s = "search"`.

A line in the wrong part of the file says where it goes. TOML puts every line under the last
table above it. So a key that you bind above `[keys]` is read as an option, and clv says `unknown
option x: x is a key, and a key is bound under [keys]`. An option that you add at the end of the
file is under a table of keys, and clv says `width is an option, not a key: write it above
[keys], before the first table`.

**The commands.** `clv --generate-config` writes every command with its keys. A command for the
list alone starts with `list-`. The others are for the pages
of a file, or for both.

| Command | Where | What it does |
|---|---|---|
| `screen-down`, `screen-up` | both | a screen on or back |
| `scroll-down`, `scroll-up` | pages | a few rows down or up, or a row of pages |
| `next`, `previous` | pages | the next or the previous screen of pages; with a search kept, the next match or the match before |
| `next-page`, `previous-page` | pages | the next or the previous screen of pages, with a search kept too |
| `previous-match` | pages | with a search kept, the match before |
| `go-to`, `last` | both | the first page or tile (after a number, that one), or the last |
| `zoom-in`, `zoom-out` | both | the zoom; on a document, the size of its text |
| `increase-per-row`, `decrease-per-row` | both | one more page or tile to a row, or one fewer |
| `search` | pages | search the text of the file |
| `follow`, `copy-link` | pages | label the links on screen, to follow one or to copy its address |
| `contents` | pages | to the contents, in a file that has a place for them |
| `copy` | both | copy the page or the tile to the clipboard |
| `open-in-app` | both | open the file in the app that the desktop opens it with |
| `rerender` | both | render the pages again |
| `back` | both | clear the search or the filter; then back through the links followed; then out of the file or the folder |
| `leave` | both | clear the search or the filter; then out of the file or the folder, or quit |
| `quit` | both | quit |
| `help` | both | the keys (`?`) |
| `list-tile-left`, `list-tile-right`, `list-tile-up`, `list-tile-down` | list | the tile before, after, above or below |
| `list-open` | list | open the tile: a file to its pages, a folder to its list |
| `list-filter` | list | filter the tiles by name |
| `cycle-theme` | both | documents and diagrams light on dark, or dark on light |
| `decrease-width`, `increase-width` | both | a document's lines narrower or wider |
| `decrease-margin`, `increase-margin` | both | a document's side margins narrower or wider |
| `cycle-fences` | both | a document's fenced blocks drawn, or as code |

The last four rows are the commands of the settings. A command that steps an option is
`increase-` or `decrease-` and the name of the option, and a command that goes to the next named
value is `cycle-` and that name. A `clv` built without documents does not have them.

## Keys

`?` lists them in the pager too, a backend's after the viewer's: `D` is the documents', and does
nothing in a `clv` built without them. A line about a key that only some formats have says what
the key does and then two of those formats (`D  light on dark, or dark on light (Mermaid, Markdown
and others)`). The keys that do nothing on what you are looking at are faint there: `D`, `[`, `{`
and `s` on a PDF, the width's and the margins' keys in a list, `T` on a file with no contents to
go to. Faint is the terminal's own (SGR 2), made of its colours, so it reads on a light background
and on a dark one, and in [text cells](#text-cells) too; a terminal without it shows those keys
as the rest. What a line says goes on over the next row where the window is too narrow for it, so
that none is cut; in a window too short for that (80 × 24), each line keeps to one row, cut in
its middle where it is too long, so that every key is still listed. Each key does what the
command it is bound to does, and several keys may be bound to one: the keys of `less` and `vi`,
and some of Emacs's.

The keys below are the defaults. [The config file](#keys-in-the-config) binds a key to another
command, or to none, and `?` then lists the keys as you bound them.

A key has one spelling, in lower case, and this manual writes a key as `?` does: `ctrl-v`, `alt-v`,
`enter`, `esc`, `backspace`, `space`, `pageup`, `pagedown`, `home` and `end`. `?` draws the four
arrows (`↑ ↓ ← →`). The case of a character counts: `j` is not `J`, and `alt-v` is not `alt-V`.

For the Alt keys the terminal has to send Alt, or on a Mac Option, as
`ESC` and the key: in kitty on macOS, `macos_option_as_alt yes`. Alt with a character that is
bound to nothing does nothing. Alt with `enter` or with a `Ctrl` key is no Alt key to `clv`: it
reads them as `esc` and then that key, so they leave as `esc` does. `alt-]` and `alt-_` do nothing:
an `ESC` and `]` or `_` is how a terminal's own answers start, so `clv` waits a second for the rest
and then drops them. `+` and `-` are listed once, as what they do where you are:
the font size on a Markdown or HTML file, the zoom elsewhere, and the first status line's reminder
says which (`+ - font`, `+ - zoom`). `,` and `.` set the pages to a row everywhere, on a Markdown or
HTML file too, and the reminder says `, . per row` where it has room.

In a file's pages:

| Keys | Action |
|---|---|
| `space`, `pagedown`, `d`, `ctrl-v` | down a screen; at the bottom of a page, on to the next page (or pages) |
| `b`, `pageup`, `u`, `alt-v` | up a screen; at the top of a page, back to the previous page (or pages) |
| `j`, `↓`, `ctrl-n`, `enter` | down a few rows (then on to the next page); with pages side by side, down a row of pages |
| `k`, `↑`, `ctrl-p` | up a few rows (then back to the previous page); with pages side by side, up a row of pages |
| `n`, `→` | next page (or screen of pages); with a search kept, `n` goes to the next match, and `→` still turns the page |
| `p`, `←` | previous page (or screen of pages); with a search kept, `p` goes to the match before, and `←` still turns the page |
| `/` | search the file's text ([Searching a document's text](#searching-a-documents-text)): type, then `enter` |
| `N` | with a search kept, the match before, as `p`; a number, then `n`, `N` or `p`, goes that many matches on or back |
| `g`, `home` | first page |
| `G`, `end` | last page |
| a number, then `enter`, `g` or `home` | that page |
| `-`, `_` | zoom out: one more page to a row. On a Markdown or HTML file, its text 10% smaller ([Documents](#documents)) |
| `+`, `=` | zoom in: one page fewer to a row, then the page as wide as the window. On a Markdown or HTML file, its text 10% larger; a number, then `+` or `-`, to that percentage, and `0+` back to 100% |
| `,` | one more page to a row, as `-` zooms out, on any file; a number, then `,` or `.`, sets that many (as many as the window takes), and `0.` goes back to `auto`. On a Markdown or HTML file, columns: the document laid out again on that many pages to a row ([Documents](#a-page-the-windows-size)) |
| `.` | one page fewer to a row, then the page as wide as the window, as `+` zooms in, on any file. On a Markdown or HTML file, one column fewer, down to one |
| `f` | label the links on screen; type a label to follow its link ([Following links](#following-links)) |
| `F` | label them, and copy the address of the link whose label you type |
| `T` | to the contents, in a file that has a place for them: a [book](#books). Backspace comes back |
| `r`, `ctrl-l` | render the pages again |
| `c` | copy the page you are on to the clipboard ([Copying a page](#copying-a-page)) |
| `o` | open the file in the app the desktop opens it with ([Opening in another app](#opening-in-another-app)) |
| `D` | Markdown, HTML, [books](#books), [Org documents](#org-documents) and Mermaid diagrams light on dark, or dark on light ([Documents](#documents), [Mermaid diagrams](#mermaid-diagrams)); on any other file it says that this format doesn't, and two that do ([The status line](#the-status-line)); with documents or Mermaid diagrams |
| `[`, `]` | Markdown, HTML, [books](#books) and [Org documents](#org-documents) laid out again, 4 characters a line narrower or wider; a number, then `]` or `[`, to that many, and `0]` back to the window's ([Documents](#documents)); with documents |
| `{`, `}` | Markdown, HTML, books and Org documents laid out again with side margins a character narrower or wider, 0 to 20; a number, then `}` or `{`, to that many, and `0}` back to 3 ([Documents](#a-page-the-windows-size)); with documents |
| `s` | Markdown and HTML's fenced blocks (` ```mermaid `), and Org's source blocks, as the code they are, laid out again, or drawn again ([Documents](#documents)); with documents |
| `?` | these keys, in place of the pages; any key goes back |
| `esc`, `backspace` | clear the search, if one is kept; then `backspace` goes back through the links followed, the last first ([Following links](#following-links)); otherwise back to the list, for a file opened from one, or to the document a link led from |
| `q`, `esc`, `ctrl-c` | quit; with a search kept, `esc` clears it first |

A Markdown or HTML page is as tall as the window ([Documents](#documents)), so it has nothing to
scroll: `j`, `k` and `enter` turn its pages as `n` and `p` do.

In a list:

| Keys | Action |
|---|---|
| `←` `→`, `h` `l`, `ctrl-b` `ctrl-f` | the tile before or after |
| `↑` `↓`, `k` `j`, `ctrl-p` `ctrl-n` | the tile above or below; below the last row's tiles, the last |
| `space` `b`, `n` `p`, `d` `u`, `ctrl-v` `alt-v`, `pagedown` `pageup` | a screen on or back |
| `g`, `G`, `home`, `end` | the first or the last tile |
| a number, then `enter`, `g` or `home` | that tile |
| `enter` | open it: a file to its pages, a folder to its own list |
| `/` | filter the tiles by name, as you type ([Filtering a list](#filtering-a-list)) |
| `+`, `-` | bigger tiles, fewer to a row; or smaller, more to a row |
| `,`, `.` | one more tile to a row, or one fewer, down to one; a number, then `,` or `.`, sets that many, and `0.` goes back to `auto` |
| `c` | copy the tile's picture to the clipboard: a file's first page (a folder has none) |
| `o` | open the tile's file in the app the desktop opens it with, or its folder in the file manager ([Opening in another app](#opening-in-another-app)) |
| `D` | Markdown, HTML and Mermaid diagrams light on dark, or dark on light; the rest of the list stays as it is |
| `[`, `]`, `{`, `}`, `s` | nothing: only a document opened from the list is laid out again, and they say so |
| `esc`, `backspace` | clear the filter, if one is kept; otherwise back (up), in a folder gone into, or stop opening one; at the first list, `esc` quits, and `backspace` does nothing |
| `q`, `ctrl-c` | quit |

## Paging through a document

The pager shows pages on the alternate screen, like `less`. The shell's scrollback is left as it
was, and quitting returns to it. With `--pages`, the pager goes through those pages only, in
their order.

When the window is wide enough, two pages share it side by side, like an open book: pages 1–2,
then 3–4, and the keys that turn a page turn both. Two pages go side by side when each keeps at
least 80% of the size it would have alone, as portrait pages in a wide window do. Until you zoom,
the pager decides this again whenever the window changes size.

`-` zooms out (or `,`): one more page to a row each time, in as many rows as fit the window, so zoomed
far out the screen is an overview of the document. It stops at 8 pages to a row, or before a page
would be narrower than 8 columns. The zoom is a size, whatever the number of pages: at four to a
row, a single page takes a quarter of the width, and fewer pages than a row takes sit in the middle
of the screen. `+` zooms in (or `.`): one page fewer to a row, and from one page, the page as wide as
the window, which scrolls; that is as far in as it goes. `=` and `_` work too: the same keys, with
Shift or without. `3.` (or `3,`) goes straight to three to a row, as far as the window takes, and
`0.` back to `auto`.

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
at the end still differ), and a message (a page that failed, `fully zoomed in`) sits just left of the
right side, cutting the name shorter while it is there. Until the first key, the message is a
reminder of the main keys. A [search](#searching-a-documents-text) is said there too, before any
message: what is typed and its count, then the match you are on.

A key that changes how a file looks (`D`, `[`, `{`, `s`), pressed on a file that has no such thing,
says so on the whole status line: `this format doesn't reflow (Markdown, HTML and others do)`,
naming two of the formats that do, and `these files don't…` in a list none of whose files does.
It has the line for a second, then the status line comes back; any key brings it back at once,
and does what it does.

A file that has a place for its contents, a [book](#books), says the key that goes there the same
way when it opens: `Contents: T`, on the whole status line, for a second.

While you type a number, the status line says only the number and the keys that take it on this
file: `7: g page · . per row` on a PDF, and `· n p match` after them with a search kept; on a Markdown
or HTML file the settings with numbers too, by
name, `7: g page · . per row · + font · ] width · } margin`; in a list `7: enter tile · . per row`.
The name, where you are, the zoom and `? help` give it the room, and come back with the next key; a
narrow window drops the last keys first.

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

### Opening in another app

`o` opens the file you are reading in the app the desktop opens it with, as a double click in the
file manager would: a PDF in Preview to fill in, a Word document in Word to edit, a photo in an
editor. On whatever page you are, the file is opened as a whole; gone into from a list, or laid out
again, it is the same file. In a list, `o` opens the tile chosen: a file, even one `clv` couldn't
show, since its app may; a folder in the file manager (on macOS, shown in Finder, never opened,
since Finder takes some folders for apps). A file compressed alone (`report.pdf.gz`) opens as the
compressed file it is, in the desktop's archive tool. `clv` hands it to the system's
opener (`open` on macOS, `xdg-open` on Linux and the BSDs) and doesn't wait for it: the status line
says `opening report.pdf…`, then `opened report.pdf`, or why not in the opener's words
(`report.pdf: xdg-open lacks a tool it needs (3)`); after five seconds without a word it says the
file was handed over. Whatever the app writes stays off the screen, and quitting `clv` leaves it
open. At most four are opened at a time.

The desktop chooses the app by the file's name, while `clv` goes by what is in it; so a file is
opened only if its real name, links followed, says a format `clv` showed it as: a PDF named
`report.command`, or a `report.pdf` linked to it, would run as a script, and is refused
(`report.command: its name doesn't say PDFs, so it isn't opened`). A `.md` shown as HTML is fine:
either is a document.

What has no file of its own isn't opened, and the status line says why: what came through a pipe
or `/dev/stdin`, even redirected from a file, and a file or a folder in an archive, or the
archive's own. Nor is anything over SSH, where the app would open on
the far machine ([On this machine or over SSH](#on-this-machine-or-over-ssh)), nor on Linux or a BSD
without a desktop session (neither `DISPLAY` nor `WAYLAND_DISPLAY`; inside tmux, as the pane has
them), nor where the opener isn't on `PATH`; on Linux and the BSDs, nor where `setsid` isn't
either, since it is what keeps an app for the terminal off `clv`'s screen.
[ADR-0034](adr/0034-open-a-file-through-the-platforms-opener-by-its-real-path.md) has why.

## Several files

`clv *.png` and `clv a.pdf b.pdf` show the files as a list: a tile for each, its first page, its
name under it, in the order given. The arrows choose a tile, its name highlighted and on the
status line with its place (`3/12`), and Enter opens it: its pages, paged through as if it had
been given alone. Esc or Backspace goes back to the list as you left it. `/` narrows the
list to the files whose names hold what you type ([Filtering a list](#filtering-a-list)).
Printed, a list is its tiles, in rows, each row's names under it.

A list's tiles are all the same size, square in pixels, so moving through it moves nothing else:
up to four to a row until you zoom (fewer when there are fewer), as many rows as fit. The screen
moves only when the tile you choose leaves it, and the tiles show as they render, as a gallery's
do. `--per-row` sets the list's tiles to a row; a file opened from it starts at `auto`. A Markdown or
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
a drawing, a Mermaid diagram or a spreadsheet is opened only when its tile nears the screen, its tile empty until then,
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
which Enter goes into, to a list of its own; Esc or Backspace comes back. A folder with
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
files, or in a directory, it is a folder with a zipper down its front and a plate at its corner
saying which kind it is (`ZIP`), where a directory's is plain, which Enter goes into, and the
status line names its files from it (`trip.zip/2024/beach.jpg`). `-r` lists its directories' files instead of folders when the archive
is given; an archive in a directory stays a folder even then, since `-r` goes through directories,
not into every archive (a jar, a document that is a zip inside). An archive in an archive is a
folder too.

From a zip, nothing is unpacked to disk: a file in it is read from it when it is drawn, so an
archive open holds its list of names, not its files. Hidden names are left out, as on disk, and so
is macOS's `__MACOSX` directory. It reads deflate, deflate64, bzip2, LZMA and XZ. A file past the
[limits](#limits), an encrypted one, or one packed in a way this build doesn't read (zstd) is left
out with a warning, as a broken picture in a directory is.

A Markdown or HTML document in an archive shows the pictures beside it there, as one on disk shows
those in its directory ([Documents](#documents)): a README in a zip of a repository shows its
screenshots, by their paths from it, `..` included, but never from above the archive's top. Only
what the archive lists is found: a picture under a hidden name (`.github/demo.png`) is left out,
as on disk it wouldn't be. Opening the document unpacks none of its pictures whole: each is
measured from its first bytes, and unpacked when a page it is on is drawn. A document's links to
other files in the archive are still not followed ([Following links](#following-links)).

A tar archive, plain or compressed with gzip, bzip2 or xz (`.tar.gz`, `.tgz`, `.tar.bz2`,
`.tar.xz`, `.cbt`…), is shown the same way, its plate saying `TAR`, or `TGZ`, `TBZ` or `TXZ` as
it is compressed, whatever its name. A compressed tar has no list of its files and can't be
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
(`ctrl-c` still quits). `backspace` takes the last one back, and the keys that type nothing still
move among the matches: the arrows, `pageup`, `pagedown`, `home` and `end`, and `ctrl-n`, `ctrl-p`,
`ctrl-f`, `ctrl-b`, `ctrl-v` and `alt-v`.

`enter` keeps the filter, and the list's keys move among the matches as they do among all the
tiles: `enter` again opens the tile chosen, and `/` edits the filter. `esc` while typing puts the
list back as it was before `/`. With a filter kept, `esc` or `backspace` clears it before
they go back or quit. With nothing typed, `enter` or `backspace` leaves no filter. The tile chosen
stays chosen while it matches, and otherwise the first after it that matches is chosen; when the
filter changes or is cleared, it stays in its row of the screen where the list allows. A number
is still the list's: `12 enter` chooses the 12th tile if it matches, and says it doesn't
otherwise, as it says of a tile `--pages` left out.

The tiles keep the size the whole list gives them, so the ones that match come together at once,
from what is already in the terminal, and only they are rendered and sent ahead
([ADR-0010](adr/0010-a-filtered-list-keeps-the-whole-lists-tile-size.md)); `+` makes them bigger.
A filter belongs to its list: a folder gone into starts with all its files, and going back finds
the list filtered as you left it. With `--pages`, the filter narrows the tiles given.

Letters past ASCII (`café`) are typed as the terminal sends them, in UTF-8. Upper and lower case
match letter by letter (`É` matches `é`), and that is all: `e` doesn't match `é`, nor does an `é`
typed as one letter match a name that stores it as `e` and an accent, as macOS's HFS+ did. In a
file's pages, `/` [searches its text](#searching-a-documents-text) instead.

## Searching a document's text

In a file's pages, `/` opens a prompt in the status line, as a list's filter does, and searches
the file's text: a PDF's, a spreadsheet's, which is shown as the PDF it prints to, and a
[document's](#documents): a Word document's, an OpenDocument text's, an RTF's, Markdown's and
HTML's, a [book's](#books) and an [Org document's](#org-documents), as it is shown. Type what
to find, at most 256 characters; `backspace` takes one back, and every key that types a character
is part of it, `q` and `n` too (`ctrl-c` still quits; the arrows and the other keys do nothing
meanwhile). A number typed before `/` is dropped. A picture or a drawing has no text to search,
and `/` says so.

While you type, the status line counts the matches in the text read so far (`/word 17…`, and
`/word 17` once all of it is read and matched), and the view stays where it is. `enter` searches,
as in `less`: the view goes to the first match at or after the top of the screen, or
past the end, to the first in the file, and the search is kept. If no text has been read yet, it
goes there as soon as a match is. `esc` while typing leaves no search, as `enter` or `backspace` do
with nothing typed.

The matches on screen are tinted once `enter` keeps the search, a yellow under them and the one
you are on an orange, on white paper as on dark: the paper takes the colour and the ink stays
ink. A page is sent to the terminal again with its tint, so for a moment after a key it shows
as it was; `n`, `N` and `p` move the orange, and clearing the search sends the pages as they were.
Nothing is tinted while you type. At most 256 matches are tinted on the pages on screen and
those kept ready around them. Where the pages on screen take most of what the terminal is given
to keep (four or more large pages to a row in a very large window), there is no room for a page
beside itself as it was: it goes blank for a moment, then comes back with its tint. `c` copies the
page untinted, as it was rendered. In
[text cells](#text-cells) the tint shows as far as the cells' colours carry it, and without
colours not at all: the status line still says which match you are on.

With a search kept, `n` goes to the next match and `N` or `p` to the one before, round the ends of
the file, where the status line says `search wrapped`; a number first goes that many on or back
(`3n`, `3p`). They
go from the match you are on while it is on screen, and once you have paged or scrolled away, from
the top of the screen: zoomed in to a page that scrolls, that is the top of the part you see. `→`
and `←` still turn pages. A match on a page not shown brings that page on
screen, and zoomed in to a page that scrolls, the match is scrolled into view, a third of the way
down the window. The status line says which match you are on and how many there are (`/word
3/17`, with `…` after while text is still being read), or `no match`; at most 10,000 are kept and
counted, the first in the file, and more say `10000+`. `esc` or `backspace` clear the search
before they go back to the list or quit, after which `n` and `p` turn pages again; a number being typed
is cleared first, as ever. `/` again edits what was searched for. A search belongs to its file: it
ends on going back to the list. It outlives Markdown or HTML laid out again (`[` `]`, `{` `}`, `+`
`-`, `,` `.`, a window resized): being typed, it is typed on; kept, the match you are on stays the
one you are on, tinted on the new pages, and the view goes back to it if it was on screen. A file
changed on disk since its search began starts the search over, what you typed kept.

Upper and lower case match letter by letter, as in a list's filter (`É` matches `é`, and `e`
doesn't). A run of spaces, tabs or line ends matches a run of them, in what you type and in the
text; whitespace alone matches nothing. Matches don't overlap. In a PDF none runs from one page
onto the next: a phrase split by a page break isn't found. In a document none runs from one
paragraph into the next, but a paragraph that runs onto the next page is searched whole, and is
on both pages: with only the second on screen, `enter` goes to a match in the part on it.

The text is read from `/` on, once, outward from the page on screen, so the keys never wait on it. A
PDF's is read in a process of its own: a book of 600 pages is read in about a second. That process
is held to its time and memory, since a page can be made to cost a lot of either. A page that takes
too much of one is left unread, and the status line says which and why (`/word 3/17 · page 40
unread`), the others searched as before; at a fourth such page, or past two minutes of reading, the
rest is left unread and the status line says how far the reading got (`· read 120 of 500 pages`). A
PDF over 512 MiB isn't read at all, and `/` says so. The process ends with the search, and with
`clv`. With `--pages`, only the pages given are searched.

A document's text is read from the layout its pages are drawn from, a paragraph at a time, on a
thread of its own: what a document costs to lay out is bounded when it opens, and its text costs
little beside that ([ADR-0030](adr/0030-a-documents-text-read-from-the-pools-layout-on-a-thread.md)). A paragraph of more than 1 MiB of text is left unread, and with it every page
it is on, the status line saying so (`· read 120 of 500 pages`) and why.

What is searched in a document is its body, each paragraph's own text: a table's cells too, a
field as it shows (an author's name, a drop-down's choice), not its headers, footers, notes, text
boxes or comments. A field whose text is worked out where it lands (a page number, a count of
pages), one drawn as something other than text (a check box, an overstruck symbol), and one with
nothing saved aren't searched, and no match runs across one: `ab`, a page number, then `cd` isn't
found as `abcd`. A table row that runs over several pages is searched with each
page it is on, whole: with `--pages` picking one of them, a match in the row's part on another
page is found and counted too, and while any of the row is unread, so is each of its pages. **A word wordcraft hyphenated at a line's end is found whole**,
and tinted on both lines, the hyphen with it.

What is searched in a PDF is the text pdfcraft reads on each page, in its reading order:

- all of it, an invisible layer too (the text OCR puts under a scan), a form's values, and
  comments drawn on the page; a scan without such a layer has no text, and the status line says
  `no text in this file` once every page is read;
- **a word hyphenated at a line's end is two words**, as the page draws them (`well-` and
  `known`): search for either part;
- **text a page clips away is still found**: a spreadsheet's string cut off by the cell after it
  matches in the part you can't see. A number too wide for its column is searched as it prints
  (`1E+14`), a formula by its value, and a hidden column or sheet not at all, since it isn't
  printed.

Printing (`--no-pager`, or a single page) has no search.

## Following links

In a file's pages, `f` labels every link on screen, as the Vimium browser extension does: a small
yellow box of dark letters, with a little of the yellow around them, its corner at the top left of
its link, on every page shown side by side, and on a page taller than the window, on the part of
it that shows. Type a label and its link is
followed. The letters are Vimium's, the home row first (`sadfjklewcmpgh`), and every label is the
same length, the shortest that gives each link its own: one letter for up to 14 links, two for up
to 196, three past that. Each letter typed hides the labels it rules out and pales in the others,
and the link is followed as soon as one label is left, which may be before its last letter.
`backspace` takes a letter back. `esc`, or any key that is in no label, drops the labels and does
nothing else (`ctrl-c` still quits), and so does anything that changes the screen: the window
resized, the document laid out again.

The labels are in reading order: page by page, down each, then across. At most 1,000 links are
labelled, and the status line says how many were left out. It says `reading links…` until every
page on screen has given its links, for two seconds at the most; the labels are then made once,
and a page that answers later, or can't be read, has none, the status line naming it (`links on
page 40 unread`). Any key meanwhile drops them. A file with no links (a picture, a drawing), or a
screen with none on it, says `no links on screen`.

The labels are drawn over the page, which is never changed nor rendered again: a document's
links look as the document draws them, with no underline or colour of `clv`'s. Directly in kitty,
Ghostty and WezTerm they are drawn at their links' own places, to the pixel. Inside tmux and in
[text cells](#text-cells) a label is characters, a capital to a cell, dark on yellow (reversed on
a terminal without colours), written over the page's cells from the cell its link starts in, and
kept within its page; the cells come back as they were when the labels go.

**A place in the document** (a PDF's link to a page, its table of contents, a named destination;
a Word cross-reference; Markdown's `[see](#usage)`, HTML's `<a href="#usage">`) is gone to: its
page, and on a page taller than the window, scrolled so that the place is a third of the way down
the screen. `backspace` goes back to where you were before the last link followed, the
newest first, for the last 100 of them; with none left it goes back to the list, or
to the document a link led from. A search kept is cleared first, and `esc` never steps back: it
clears the search, or leaves. The places belong to the file: going back to the list forgets them. Nothing goes forward
again. With `--pages`, a link to a page that isn't among those given says so (`page 12 isn't
among --pages`) and goes nowhere, and one to a page given twice goes where it is first. A place the
document doesn't have says so too (`no #usage in this document`). A Markdown or HTML document
laid out again (`]`, the window resized) keeps the places to go back to: each is the paragraph you
were at, found again in the new pages.

**Another file** (`[the guide](docs/guide.md)`, `../README.md#install`, a PDF's link to another
PDF) opens in `clv`, as a level of its own, the way a list's tile is gone into: `backspace`
and `esc` come back to the document, where you left it. Its path is looked for beside the
document, made real (every symbolic link followed), and opened only if it is a regular file or a
directory, a directory as a list: a pipe, a socket or a device is refused before anything opens
it, as is a path that leads nowhere, or to `/dev` or `/proc`. Then the place the link names is
gone to: its `#fragment`, or for a PDF's link the destination or the page it names, and a name the
file doesn't have is said. Links are followed into files 16 deep at the most, two files that link
to each other counting each time; past that the status line says so. A document that is no file
on disk (through a pipe, in an archive) has nothing to look beside, and says so; its links to
places in itself still work.

**A web address** goes to your browser, or your mail app, through the desktop's opener, the one
`o` uses ([Opening in another app](#opening-in-another-app)), as a single argument and never
through a shell. The status line says `opening https://…`, then `opened https://…`, or why not;
nothing asks you to confirm it. Only `http`, `https` and `mailto` addresses are opened, of at most
4 KiB, with no space, no control character, no bidirectional mark and no other character that
doesn't show; and never a `mailto` whose parameters name a file for the mail app to read
(`attach`, `attachment`, `insert`), as written or under up to eight layers of percent-encoding.
Any other is said by its kind (`a file: link, which isn't followed`). Where no app can be opened
for you (over SSH, with no desktop session, with no opener installed), the address is copied to
the clipboard instead, and the status line says so.

**Anything else** a link may do isn't done, and the status line says what it was: a script, a
program to start, layers shown or hidden.

**`F`** labels the same links, and copies the address of the one whose label you type, instead of
following it: a web address as it is, a file's path as the document writes it, with its
`#fragment`, a place in the document as its `#name`. A link with no address of its own (a PDF's
link to a page) says where it goes instead (`goes to page 12`), and one that is never followed
says what it is; neither is copied. The address goes to the terminal's clipboard where it has one
for programs to write (kitty's clipboard protocol, as [a page copied](#copying-a-page) does),
which says when it has it; otherwise to this machine's clipboard, when the terminal runs on this
machine; and over SSH through OSC 52, the escape code most terminals take text for their clipboard
by, which nothing answers: the status line says it was sent, and names the address, since it can't
say whether the terminal took it. Inside tmux it goes through passthrough, as the pages do; in
text cells inside tmux, tmux takes it only with its `set-clipboard` on.

Which files have links:

- **PDFs:** their link annotations, each active where the PDF says, over its quadrilaterals or
  else its rectangle, on a page turned or cropped as on any other. Where a destination puts its
  page is honoured as far as it says how far down: `/XYZ` and `/FitH` by their top, `/FitR` by the
  top of its rectangle, and the rest, or a place left out, at the page's top. A **spreadsheet**'s
  are whatever the PDF it prints to carries.
- **Word documents, OpenDocument texts, RTF, Markdown and HTML:** the links in the body, a
  table's cells included, each over the lines its text is on; and a Word cross-reference that is a
  link (a `REF` or `PAGEREF` field with `\h`). Not those in headers, footers, notes or text boxes,
  which a [search](#searching-a-documents-text) leaves out too. A place is a Word bookmark, by its
  name, in any case if no bookmark has it exactly.
- **Org documents:** the links in their body and their headings, as Org has them, each resolved
  when the file is read: `[[*Heading]]` to the heading of that title (without its TODO keyword,
  priority, tags and cookies, in any case), `[[#custom-id]]` and `[[id:X]]` to the heading whose
  `:CUSTOM_ID:` or `:ID:` it is, `[[target]]` to a `<<target>>`, else a `#+NAME:`, else a heading of
  that title; a footnote's number to its note. An `id:` the file hasn't says `no #id:X in this
  document` (`id:` links across files need Emacs's database). Files: `file:`, `file+sys:`,
  `file+emacs:`, and `./x`, `../x`, `/x` and `~/x`, `~` your home (another user's, `~user`, isn't
  followed); after `::`, `*Heading`, `#custom-id` and `target` are the place in that file, and a
  line's number or a `/regex/` opens it at its top. On the web: `[[https://…][…]]`, `<https://…>`,
  bare `http`, `https` and `mailto` addresses in the text, and `doi:X` as `https://doi.org/X`. The
  abbreviations the file declares (`#+LINK:`, with `%s` and `%h`) are expanded. Anything else
  (`elisp:`, `shell:`, `help:`, `info:`) is never run: the status line says what it is (`an elisp:
  link, which isn't followed`). `F` copies the address as clv writes it: `other.org#*Some%20Heading`.
- **Places in Org documents:** a heading by its title, as `[[*Heading]]` finds it (as written, or
  as it shows: `[[*Greek α]]` for `* Greek \alpha`), and by the id GitHub would give the text it
  shows, so that a Markdown file's `[see](notes.org#some-heading)` finds it;
  a `:CUSTOM_ID:`, an `:ID:`, a `<<target>>` and a `#+NAME:` by their names. A name the file gives
  itself wins over a heading's: a heading whose title or id another place has is found by its
  title through `[[*Heading]]` still. A heading is also `*` and its title, which only a heading is:
  another file's `::*clash` goes to its heading `clash` where it has a `<<clash>>` too, and `F`
  copies such a link as `other.org#*clash`. Nothing hidden is a place, nor a note: a target or a
  footnote's definition in a drawer is left out with it.
- **Places in Markdown and HTML:** in HTML, any element with an `id` (`<h2 id="usage">`,
  `<div id="usage">`), as well as `<a id>` and `<a name>`. Markdown's headings get the ids GitHub
  gives them: lower case, punctuation dropped, each space a hyphen, and `-1`, `-2` after it for a
  heading that repeats, so `[see](#install)` goes to `## Install`. Letters and numbers of any
  script stay, with the marks that combine with them (an accent written apart from its letter, a
  script's vowel signs) and `_`; an id the document writes
  itself (`<a id="install">`) wins over a heading's. A link's path and its `#name` are read as a
  URL's: `my%20file.md` is `my file.md`.

Printing (`--no-pager`, or a single page) has no labels, and `f` and `F` do nothing in a list.

## Which files it shows

A file is shown by the first backend that opens it, going by how the file starts, and by its name
only where that can't tell. `clv --help` ends with the formats this build shows, under what each
backend shows and the libraries it shows them through, the extensions of those that go by their
name, and the languages of the [fenced blocks](#documents) a document shows as pictures of them
(`Mermaid (.mmd .mermaid, ```mermaid)`), so a build without a backend lists none of its formats. Before them come the options a
backend takes besides the ones above, under it; a backend that takes none has no heading. An option
that only another backend takes is refused, since it would do nothing for the file.

| Files | Known by | Their options |
|---|---|---|
| [SVG drawings](#svg-drawings), and svgz, gzipped | XML whose root element is `<svg>`, after whatever may come first (a declaration, comments, a doctype), and svgz by what it unpacks to; a `.svg` or `.svgz` name when the first 1024 bytes end before the root element. Other XML is not one | none |
| [Mermaid diagrams](#mermaid-diagrams) | Their name: `.mermaid`, and that it is text; `.mmd`, and that its first line past blank lines, `%%` comments and front matter, within its first 1024 bytes, names a kind of diagram merman draws (`flowchart`, `graph`, `sequenceDiagram`, `stateDiagram-v2`…) or is an `%%{init: …}%%` directive, since MultiMarkdown names its files `.mmd` too, which clv doesn't open. Not through a pipe, as a diagram has no signature | none |
| [Word documents, OpenDocument text, RTF, Markdown and HTML](#documents) | An RTF by how it starts (`{\rtf`), whatever its name; an odt by what the zip it is says it holds; a docx by what its zip holds, or a Word name (`.docx`, `.docm`, `.dotx`, `.dotm`, `.doc`, `.dot`); a Word 97-2003 document by such a name only, since spreadsheets and slides are compound files too; HTML by how it starts (a doctype or an `<html>` element), whatever its name, or a name (`.html`, `.htm`, `.xhtml`) and that it is text; Markdown by its name only (`.md`, `.markdown`, `.mdown`, `.mkd`), and that it is text | `--password PW`: the password of a protected Word document |
| [Org documents](#org-documents) | Their name, `.org` or `.org_archive`, in any case, and that they are text; or, whatever their name, through a pipe too, an Emacs mode line naming Org on their first line (their second, after a `#!` line): `-*- org -*-`, or `mode: org` among its variables; but a Markdown or HTML file, known first, stays one whatever its first line says | none |
| [Spreadsheets](#spreadsheets) | An ods by what the zip it is says it holds; an Excel workbook by what its zip holds, or an Excel name (`.xlsx`, `.xlsm`, `.xltx`, `.xltm`, `.xlsb`, `.xls`, `.xlt`); an Excel 97-2003 workbook by such a name only; values separated by commas or tabs by their name only (`.csv`, `.tsv`, `.tab`), and that they are text | `--password PW`: the password of a protected workbook |
| [Ebooks (epub)](#books) | A zip whose first file, `mimetype`, says it is an epub, whatever its name; or a `.epub` name, for a zip whose first bytes don't say what it is (a `mimetype` that isn't first, or is compressed, as some tools write it) | none |
| [PDFs](#pdfs) | `%PDF-` in their first 1024 bytes | `--password PW`: the user or the owner password of an encrypted PDF |
| [Images](#images) | Their signature: PNG, JPEG, GIF, WebP, TIFF, BMP, ICO, QOI, JPEG XL, OpenEXR, Radiance HDR, PNM and farbfeld; and TGA, which has no signature, by its `.tga` name | none |
| [Zip archives](#archives) | Their signature (`PK`), whatever their name: jars, cbz comics and other zips that aren't documents, workbooks or books, as directories | none |
| [Tar archives and compressed files](#archives) | A POSIX or GNU tar header, as directories; files compressed with gzip, bzip2 or xz by their signature: a compressed tar as an archive, anything else as the file it unpacks to | none |

### PDFs

PDFs are rendered by [pdfcraft](https://github.com/storytold/pdfcraft), the way its app draws them:
comments without an appearance of their own are drawn too, and a dynamic XFA form shows the pages
its template lays out into, not the placeholder page the file carries for viewers that can't. A
page whose content runs past pdfcraft's safety budget is drawn without the rest of it, and a colour
it can't compute (a colour space it can't read, a tint transform it can't evaluate) is painted in a
fallback colour; an image whose colour table is malformed is drawn as well as it can be, or left
out, and a Type 3 font without its widths is spaced by its glyphs' own; each with a warning. `/` [searches a PDF's text](#searching-a-documents-text).

A PDF's tile in a list shows the file as it is, without those additions, because clv makes them
for the whole file. So a tile shows no comment that has no appearance of its own, and the tile of
a dynamic XFA form shows its placeholder page. When you open the PDF, clv adds them
([ADR-0042](adr/0042-a-pdfs-tile-is-the-file-itself-measured-by-hayro.md)).

### Images

An image is one page, its size its own in pixels, turned the way its EXIF orientation says (a
photo taken with the phone on its side shows upright). It is fitted to the window like a page,
but never made larger than it is: a pixel of the picture to a pixel of the screen at most, so an
icon shows as small as it is. `--per-row width` (and `+` to the width) still makes it as wide as the
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

A drawing's HTML labels (`<foreignObject>`, which tldraw, draw.io and Mermaid label their drawings
with) are drawn by [Blitz](https://github.com/DioxusLabs/blitz), an HTML and CSS engine, over what
resvg draws, at the size shown, so they stay sharp too. The fallback a browser skips after one in
a `<switch>` isn't drawn (draw.io's "[Not supported by viewer]" and "Text is not SVG - cannot
display"). A label is drawn as a browser draws it, as far as Blitz goes:

- Only those a browser shows: none in a `defs`, `symbol`, `clipPath`, `mask`, `pattern` or
  `marker`, none hidden (`display="none"`, `visibility="hidden"`, by attribute or `style`), and in a
  `<switch>` only in the child it takes (`systemLanguage` as English, `requiredExtensions` never
  met, `requiredFeatures` always). One a `<use>` repeats isn't drawn, and the page says how many.
- Where the drawing puts it, through its `viewBox`es, nested `<svg>`s and transforms; its size in
  percent is of its viewport (of the drawing, when it says no size of its own, as large as what it
  draws).
- Styled by the drawing's `<style>`s as if inside its ancestors (their ids, classes, styles, and
  font and colour attributes), so a rule such as Mermaid's `#id .nodeLabel` reaches it; `<font>`'s
  `face`, `color` and `size` as hints, under any rule of the drawing's.
- With the system's fonts, the same ones the drawing's own text has (found the same way, so a
  static Linux build has them too), and those the drawing embeds as `data:` URLs (WOFF, WOFF2,
  TrueType, OpenType); a generic family (`sans-serif`, `monospace`…) is the one the drawing's text
  gets for it. Text in a script the named family lacks (CJK, Arabic, emoji…) falls back to the
  usual families for it that the system has, rather than to whatever the system would pick. With
  the pictures it embeds as `data:` URLs (PNG, JPEG, GIF, WebP). Nothing else: a
  label's link to a picture or a font elsewhere isn't followed, and is counted with the drawing's;
  no stylesheet or document is loaded (`<link>`, `@import`, `<iframe>`, `<object>`), even as a
  `data:` URL. A label's own `<style>` applies as the drawing's do.
- In a process of its own: Blitz runs in a worker clv starts for the drawing (`clv --worker`,
  which `--help` doesn't list), held to its CPU time and its memory ([limits](#limits)); past
  either, the worker is killed and all of the drawing's labels are left out, with a warning,
  until the drawing is opened again, and the drawing is drawn without them. The worker starts
  some 40 ms after the drawing's first page is asked for, and goes with the drawing.
- Not in a list: a list's tiles show drawings without their labels (a draw.io tile shows no
  fallback text either), and `c` on a tile copies it without them. Opening a drawing from the list
  shows its labels, as printing it does.
- Built without the `svg-html` feature, clv draws no labels, and a warning says how many weren't
  drawn.

Where clv draws a label otherwise than a browser:

- Over the whole drawing, not among its shapes: a shape the drawing draws over a label is under it
  here, and a group's opacity around a label applies to the label as a whole.
- Not clipped by its ancestors' clip paths or masks, or by a nested `<svg>`'s bounds.
- A stylesheet's `display: none` on an ancestor isn't seen; nor may a rule on an SVG element's name,
  a namespace, a sibling or a structural pseudo-class (`:nth-child`) match.
- A `<switch>` whose child taken, or fallback, an entity makes keeps its fallback, drawn as resvg
  draws it.
- Content straight in a `display: table` box or row (Mermaid's edge labels), which Blitz alone
  leaves out, is laid out in the anonymous row and cell CSS gives it, as elements named `clv-row`
  and `clv-cell`: a rule on `*` reaches them (its padding on the cell, say), where a browser's
  anonymous boxes take no rule. The content keeps the rules that matched it where it was written.
- What an `overflow: visible` label draws past its box is drawn up to 4,096 pixels each way, and cut
  there.
- Its text can't be selected or copied.

What a drawing costs is bounded before it is spent
([ADR-0003](adr/0003-bound-what-a-drawing-costs-before-usvg-and-resvg-spend-it.md)): past its
[limits](#limits), it is refused, what is too much of it is left out with a warning, or a page
fails with why. A render once begun can't be stopped: a page within the limits but slow still takes
its time on a render thread, while the keys work. Its HTML labels are the exception: their
worker is bounded as it spends, and killed past its time or its memory
([ADR-0026](adr/0026-an-svgs-html-labels-in-a-worker-process-capped-on-cpu-time-and-heap.md)).
One cost the limits don't count is a [known issue](#known-issues): a stylesheet's parse.

### Mermaid diagrams

A Mermaid diagram is one page: [merman](https://github.com/Latias94/merman), a Mermaid in Rust,
turns it into SVG, as Mermaid 12 draws it, and it is shown as an
[SVG drawing](#svg-drawings) is, sharp at any size, `c` and text cells included, but on a paper of
its own, where a drawing shows the terminal's background wherever it is transparent.

A diagram follows the paper, as [Markdown and HTML](#documents) do. On a terminal with a dark
background it is drawn in Mermaid's dark theme, light on dark, on exactly the terminal's colour,
so that it is no rectangle at all; otherwise in Mermaid's default theme on white, as
`mmdc -b white` draws it, since that theme's ink is made for white. `D` turns it to the other and
back, in a list too, `--theme light` or `--theme dark` says how it starts, and a terminal that
doesn't say what its background is gets it on white, with `D` still going to a dark grey. The page
is as large on either paper, the size of the default theme's drawing: where the dark theme lays a
diagram out a little larger (a class diagram, a tenth wider), it is drawn that much smaller, in
the middle. A diagram that can't be drawn in the dark theme fails on dark paper, saying why, and
`D` still shows it on white. A ` ```mermaid ` block of a document, and a diagram a document shows
by path, follow that document's paper the same way.

merman draws every kind of diagram it parses: flowcharts,
sequence, class, state and entity-relationship diagrams, gantt charts, pies, mindmaps, timelines,
user journeys, git graphs, requirement, C4, quadrant, XY, sankey, packet, block, architecture,
kanban, radar and treemap diagrams, and the other kinds Mermaid has in beta. A diagram's own
`%%{init: …}%%` directive and front matter apply (its theme, its colours), but for Mermaid's secure
settings, which it can't change: its labels are drawn as SVG text, never HTML, and its edges are
held to Mermaid's 500.

Where it differs from mermaid.js:

- Edges bend as merman's Cytoscape layout bends them, where Mermaid 12 routes them through ELK.
- merman sizes a label's box by its metrics for Mermaid's font, and the label is drawn in the
  system's fonts (Arial on most): a label may run a little past its box, as a long sequence note does
  in mermaid.js too.
- Math (KaTeX) and icons from the web (`fa:` icons, `icon` shapes) are left as merman leaves them:
  nothing is fetched. `click` does nothing.
- A user journey's sections and tasks, and a kanban board's cards, are HTML labels, as merman draws
  them: drawn as a drawing's are, but not in a list's tile, which shows the diagram without them.

merman runs in a process of its own: a worker clv starts (`clv --worker mermaid`) when a diagram is
first opened, which draws the diagrams opened after it in turn and goes 2 seconds after its last.
A diagram past merman's own limits ([limits](#limits)) is refused with merman's message, in
milliseconds; past the worker's CPU time or memory, the worker is killed and that diagram fails,
saying which, and the next starts another. After three killed in a row, no diagram is drawn for a
minute. What a diagram is drawn to is kept while clv runs, so a diagram opened again is not drawn
again: its tile in a list, then its page; a document's picture or ` ```mermaid ` block, sized, drawn,
and laid out again at another width or font size.

### Documents

A Word document (docx, or a Word 97-2003 doc), an OpenDocument text (odt) or an RTF is laid out
into pages by [wordcraft](https://github.com/storytold/wordcraft), as its app prints them: each
page its own size, in points, fitted to the window as a PDF's is; headers and footers, tables,
lists, pictures and shapes; tracked changes marked up, as Word prints them unless told not to
(what was deleted struck through, what was inserted underlined, a bar beside each changed line);
no comment balloons, no proofing marks. Text is set in the fonts it names when the system has
them, and otherwise in the ones wordcraft substitutes, its own included (Source Serif, Source
Sans, Inter, JetBrains Mono). Fonts a document embeds are not used. A protected Word document opens
with `--password`; without it, clv says it needs one. `/` [searches](#searching-a-documents-text)
a document's body.

Markdown and HTML are laid out the same way, on pages the window's size, since they give no page size
of their own ([A page the window's size](#a-page-the-windows-size)), or to a width of your own.
Markdown is read as GitHub writes it, with its tables, task lists, strikethrough, footnotes and alerts
(`> [!NOTE]`), and the HTML in it ([ADR-0014](adr/0014-markdown-through-pulldown-cmark-into-wordcrafts-html-importer.md));
front matter (`---`) is left out. HTML is read for its text and what its tags and CSS say of it
(headings, lists, tables, colours, margins): its `style` attributes, and the simple rules of its `style`
elements (`p`, `.note`). Scripts and linked stylesheets are skipped, so a page designed with CSS shows as
its words in order, not as a browser draws it, and one that counts on rules that aren't read can come out
oddly: a dark theme's colours on white paper, say
([ADR-0017](adr/0017-htmls-style-rules-apply-as-far-as-wordcraft-reads-them.md)). What may cost too much to lay out is refused, at its [limits](#limits)
([ADR-0012](adr/0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md)).

They say little of how they should look, so clv gives them a look of its own, the same on any machine:
text in Source Sans and code in JetBrains Mono, both of which come with it; headings falling in size, the
first two above a line; code blocks on a faint wash, where 80 columns fit on a line; quotes beside a bar;
tables in fine lines, under a shaded header; links in blue; and an alert's kind, in its colour, at the
top of its quote. What HTML's own CSS says of its text comes first
([ADR-0016](adr/0016-how-markdown-and-html-look.md)).

The pictures Markdown and HTML refer to are drawn by whatever shows them: anything `clv` shows as an
image, an SVG drawing, a Mermaid diagram (`![a flow](flow.mmd)`), or a PDF's first page, at its own size, or smaller to fit the column and the
page. They are read when a page they are on first shows, so opening a document, or a list of them,
reads none.

Only what is near the file is read: a path from its directory, `..` included. **Nothing is fetched**:
a picture on the web shows as its alt text. So does an absolute path, which on the web means from a
site's root; one that is missing; and a file that is no picture, such as another document. A document
in a zip or a tar finds its pictures in that [archive](#archives), by the same paths, `..` included but
never above the archive's top: nothing beside the archive on disk is read for it
([ADR-0037](adr/0037-what-is-beside-a-held-file-is-read-from-its-holder.md)). What comes through a
pipe, or is compressed alone (`README.md.gz`), has nothing beside it, so none of its pictures can be
reached. A picture in a `data:` URL is drawn, if it is a PNG, JPEG, GIF, WebP or BMP. The first page
says how many pictures on this machine were left out, and a page says if one of its own can't be drawn,
a grey box in its place ([ADR-0013](adr/0013-a-documents-pictures-are-drawn-by-the-backends.md)).

A fenced block of Markdown written in a language `clv` draws shows as a picture of it, not as its
code: a ` ```mermaid ` block as its [diagram](#mermaid-diagrams), as on GitHub. The language is the
first word after the fence, in any case (` ```Mermaid title="A flow" `), and `clv --help` lists the
ones this build draws. It goes by that word alone, never by what the block holds: a block in any other
language stays code, ` ```svg ` included, as does one with no language, or one that is indented
rather than fenced.

In HTML, and so in a [book](#books)'s chapters, such a block is a `<pre>` that says its language, as what
writes HTML from Markdown says it:

- a class `language-mermaid`, of the `<pre>` or of the `<code>` it starts with
  (`<pre><code class="language-mermaid">`): CommonMark's renderers, highlight.js, Prism;
- a class of the language alone (`<pre class="mermaid">`), there too: pandoc, and Mermaid's own way
  in a web page;
- the `<pre>`'s `lang="mermaid"`: GitHub's pages, which keep a diagram's text under their drawing
  of it;
- a class `highlight-source-mermaid` of the `<div>` right around the `<pre>`: GitHub's highlighted
  blocks.

Its text is what a browser would show of it: tags dropped, the `<span>`s of a highlighter among
them, and entities decoded. A `<pre>` in a comment, or in a `<script>`, a `<style>` or a
`<textarea>`, is no block, and one that is never closed, or holds another, stays code. A `<pre>`
written as HTML inside Markdown stays code too.

A block is drawn as a picture is, at its own size or smaller to fit the column
and the page, when a page it is on first shows; and since its text comes with the document, it is
drawn through a pipe and from an archive too, where a picture by path can't be reached. A block
that can't be drawn (a broken diagram, or one past what its kind [may be](#limits)) shows as the
code it is, and the first page says how many weren't drawn. Its text isn't
[searched](#searching-a-documents-text) once it is a picture
([ADR-0032](adr/0032-a-fenced-block-is-drawn-by-the-backend-that-lists-its-language.md)).

`s` shows a document's blocks as the code they are, to read a diagram's source or to search it, and
again draws them. The document is laid out again, since a picture and its code take other room,
with the paragraph at the top of the screen kept there, and the status line says `blocks as code`
while they are. As code, a block is its text, as one that can't be drawn is too: in HTML the tags
inside its `<pre>` are dropped, a highlighter's among them, and the blank lines at its start and
end are left out. A search kept meanwhile starts over, on
the text as it is then. `--fences code` starts that way, and `--fences drawn`, the default, is as
without it. On a file that is neither Markdown nor HTML `s` says so; in a list it says to open a
document, and the tiles stay as they were opened: drawn, or as code with `--fences code`.

Markdown and HTML have no paper of their own, so on a terminal with a dark background they go light
on dark, the page exactly the terminal's colour, so that it is no rectangle at all. `D` turns them to
dark on light and back, `--theme` says how they start, and a terminal that doesn't say what its
background is gets them as printed, with `D` still going to a dark grey. Pictures stay as they are,
but for thin near-black ink, a logo or a line drawing made for white paper, which would vanish on dark:
it comes with white behind it. A fenced block drawn as a diagram follows the paper: on dark paper it
is drawn again in Mermaid's own dark theme, on the paper's colour with no white behind it, and `D`
turns it with the page. It takes the room its light drawing took, so nothing on the page moves:
where the dark theme lays a diagram out a little larger (a class diagram, a tenth wider), it is
drawn that much smaller, in the middle of that room. A diagram's own theme (`%%{init: …}%%`) is its
own on either paper. One that can't be drawn for a page's paper is a grey box there, the page
saying so, never its drawing for the other paper; it isn't tried again until the document is laid
out again. A diagram shown by path (`![a flow](flow.mmd)`) follows the paper the same way, and so
does a `.mmd` file shown by itself ([Mermaid diagrams](#mermaid-diagrams)). A Word document, an RTF
or a PDF is a page as printed, and stays so; `D` says that it doesn't go dark, and which formats do. What `c` copies is the
page as it is shown
([ADR-0015](adr/0015-markdown-and-html-on-dark-paper-the-terminals-own-colour.md)).

Opening a document reads and lays it out whole, to know its pages: a quarter of a second for 500
pages, on an M3 Max, and one slow to lay out holds up its opening. Shown, those 500 pages hold about
200 MB. Why it is read this way, and not through wordcraft's engine, is in
[ADR-0005](adr/0005-word-documents-through-wordcrafts-readers-layout-and-renderer.md).

#### A page the window's size

A Markdown or HTML page is the window, as on an e-reader: as tall as the pages area (the window less
the status line), and as wide, or, where the window holds two pages of 60 characters each between
their margins, two side by side like an open book, each half the window. A character of the code face
(JetBrains Mono at 9.4 pt) is a cell of the window, so a window 100 columns wide puts 94 characters of
code on a line, between margins of three characters at each side and a row above and below. A page fills the window at that size,
and is never shown larger: zoomed out, two or more to a row, it is smaller, as any page. In a window
too small for 20 characters a line and 10 lines of text, the page holds those all the same, shown
smaller to fit. Through a pipe or to a file, the pages are laid out for the terminal the command
runs in; with no terminal at all, on the preview's page below. Inside tmux, the pane is the window
([Inside tmux](#inside-tmux)).

The pages are laid out when the file opens, so a large file takes its time before anything shows
(`ctrl-c` stops it, the terminal as it was). When the window changes size, the document is laid out
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

**Columns.** `,` and `.` set the pages to a row on a Markdown or HTML file too, but there they are
columns: `3.` lays the document out again on pages a third of the window wide (less the gaps between
them), each as tall as the window and its text at the font size, three to a row; `,` one more column,
`.` one fewer, down to one, where `.` stops and says `1 per row, the least`: a page is never wider than
the window, and `--per-row width` is one to a row here. `0.` goes back to `auto`, one page or a book of
two. They lay out again at once, as `[` `]` and `+` `-` do. A narrow column holds fewer sizes of text:
the font size's most is the largest at which a column holds 20 characters a line, and a size past it,
100% too, is said with what it is laid out at (`font 100% (80%)`). A count and a width
of your own are both about how wide a page is, so the last you set wins: `3.` puts the width back to
`window`, and `72]`, `[` or `]` put the count back to `auto` first, stepping from where `window` sits
there; `0.` and `0]` change only their own. On the command line, `--width 72` and `--per-row 3` on such
a file disagree, and the width wins, with a warning; with one of the two in
[the config](#the-config-file), the one on the command line wins. A layout that fails puts both back to what is on
screen. In a list and with `--pages`, `,` and `.` are the zoom, as on a PDF; printed, `--per-row 3`
lays the file out in three columns of the terminal's room.

**Margins.** `}` and `{` lay a Markdown or HTML file out again with side margins a character wider or
narrower, from 0 to 20 characters of code at each side, at the font size: 3, the default, is three
cells at 100% and six at 200%. A number, then `}` or `{`, goes straight to it (`5}`), `0}` back to 3,
and `--margin 5` says where to start. At `window` the margins come out of the line, so wider margins
put fewer characters on it, and a book's halves need their 60 characters between them; at a width of
your own the line keeps its length and the page grows by the margins, up to the window. `}` stops at
the most where a line still holds 20 characters at the font size, and says so (`margin 6, the most
here`); a wider margin, typed or given, is kept and laid out at the most (`margin 12 (6)`), or at a
column where even none holds 20: the margin yields to the font size, which stays as asked. The status
line shows the margin while it isn't 3. The top and bottom margins stay a row. In a list and with `--pages` they say why
they do nothing, as `[` and `]` do.

**A width of your own.** `]` and `[` lay a Markdown or HTML file out again to lines 4 characters
wider or narrower, counted in the code face, from 20 up to the longest line the window holds, with
`window` among the steps where it sits: in a window of 100 columns, 92, 94, `window`; in one wide
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

### Books

An ebook (epub) is shown as a book: its cover, then its chapters in its reading order, all of them,
each starting a new page, then a Contents page, as one document. A chapter out of the main flow
(`linear="no"`: a note, an answer) comes where the book lists it. The book's package, which says
that order, is read by
[rbook](https://github.com/DevinSterling/rbook), in a process of its own; its chapters, which are
XHTML, are joined and laid out as [HTML is](#documents), on pages the window's size or to a width
of your own. So a book has what a document has: `[` and `]` for the width of its line, `+` and `-`
for its font size, `{` and `}` for its margins, `D` for dark paper, `s` for its fenced blocks, each
with its option; `/` [searches](#searching-a-documents-text) the whole book, across its chapters;
`--pages` picks pages; and `o` opens the `.epub` itself.

**The cover comes first**, a page of its own: the picture the book's package names as its cover
(EPUB 3's `cover-image`, or EPUB 2's `<meta name="cover">`), unless the first chapter shows that
picture already, as most books' cover pages do; then it isn't shown twice. So a book's tile in a
list is its cover.

**A Contents page** comes last, after the book's chapters, so that the book's own pages are where
they would be without it. It is headed `Contents`, and says under that who made it (`Made by clv
from the book's table of contents.`), since the book didn't: the book's table of contents, EPUB
3's (its `nav`) or else EPUB 2's (its NCX), as nested lists as deep as the book's, each entry a
link to the place it names. `f` and an entry's label go to its chapter, Backspace comes back, and
`/` finds a chapter by its title. An entry that leads to no chapter the book shows is its label
alone. The page is made for any book that has a table of contents, a book with a page of contents
among its chapters too. A table of contents that can't be read leaves the book without the page,
and the first page says so: it never costs the book.

**`T` goes to the contents** from any page, and Backspace comes back to where you were. It goes to
the book's own page of contents where the book has one among its chapters:

- the chapter the book's package says is one: its nav document in its reading order, or the
  chapter its landmarks, or EPUB 2's guide, call `toc`;
- or else a chapter that looks like one, which the package doesn't mark, as in Project
  Gutenberg's EPUB 3 books: among the book's first eight chapters, the first whose links lead to
  three other chapters or more, and most of whose links lead to another chapter. `T` lands at the
  first of those links, since such a chapter may start with the book's title page. A chapter with
  a few links to others, a preface or a page of notes, isn't taken for one.

Only in a book with neither does `T` go to the Contents page at the book's end. When a book
opens, the status line says the key (`Contents: T`), for a second
([The status line](#the-status-line)). A book with no table of contents and no such chapter has
no contents to go to: nothing is said when it opens, `?` lists `T` faint, and `T` says so, as it
does on any file that isn't a book.

Its pictures are read from the book, by their paths from their chapter, `..` included but never
above the book's top, and drawn by whatever shows them, as a document's are from its directory.
Each is measured from its first bytes, so opening a book, or a list of them, unpacks none whole. A
picture on the web is its alt text. A drawing that only wraps a picture (`<svg><image/></svg>`, as
a cover's page is written) shows as that picture.

A chapter that is a picture or an SVG drawing, not XHTML, is a page of that picture. One of any
other kind is shown as the first of its fallbacks, as the package lists them, that is XHTML or a
picture, at most eight along; with none, it is left out.

The book's look is `clv`'s own ([ADR-0016](adr/0016-how-markdown-and-html-look.md)), with what its
markup says on top through `style` attributes. **Its stylesheets are not read**, linked or in a
chapter's `<style>`: a book made to look a certain way comes out plainer, as its words in order.

Its links are [followed](#following-links) as any document's. A link from one chapter to another
(`ch2.xhtml`, `ch2.xhtml#s3`, `../Text/ch2.xhtml#s3`), or to a place in its own (`#s3`), is a place
in the book: `f` and its label go there, Backspace comes back, and `F` copies it as the book's document
names it, `#c2` for the second chapter's start and `#c2-s3` for its `id="s3"`, since two chapters
may each have an `s3`. A web address is followed as in any document.

**No link from a book is followed to a file**, and none is ever looked for beside the `.epub` on
disk. A link to a file in the book that is no chapter (a picture, a stylesheet) says `a file in the
book, which clv doesn't open alone`, one to a file the book doesn't hold says `not in this book`. A
link to a chapter the book lists but doesn't show, or to a place a chapter doesn't have, says the
place isn't in the document, as in any document.

**Protected chapters** (DRM) are left out, and the first page says how many are (`3 chapters are
protected (DRM)`): a chapter the book's `META-INF/encryption.xml` says is encrypted, by anything
but the two ways fonts are obfuscated (IDPF's and Adobe's), which are no matter, since a
document's fonts aren't used. What can be read is shown. A book none of whose chapters can be
read is refused: `it is protected (DRM)`. A protected picture is left out, as one that is missing.

A chapter that is missing, that can't be read (neither UTF-8, nor UTF-16 with its byte-order mark),
that is larger than 16 MiB or that is neither XHTML nor a picture is left out, and the first page
says how many were, and why. A book longer than `clv` lays out ([limits](#limits)) is cut at the
chapter that would pass them, and the first page says how many chapters that left out.

A file that is a book but can't be read as one is refused, with why, and isn't shown as a zip
instead: one without its `META-INF/container.xml` or the package that names, with a package that
lists no chapters, or one rbook refuses, in rbook's words. A package whose reading takes rbook
past the limits of its process is refused as one that `couldn't be read`.

A book in a list is opened when its tile is wanted, and its tile is its first page. For the tile,
clv reads the book's package and lays out only that page: the cover, or else the first chapter. So
the tile's warnings say only what that page and the package tell, such as the chapters that are
missing or protected. When you open the book, clv lays it out whole for the window, and says the
rest ([ADR-0041](adr/0041-a-books-tile-is-its-first-page-alone.md)). One in an archive is read from the archive each time it is laid out or a picture of it is drawn; one through
a pipe is kept whole while it is open, and refused past 256 MiB
([ADR-0038](adr/0038-an-epub-as-one-document-its-package-read-by-rbook-in-a-worker.md)).

### Org documents

An Org-mode file (`.org`: Emacs's notes, plans and documentation) is shown as Markdown is: parsed by
[orgize](https://github.com/PoiScript/orgize), in a process of its own, written as HTML, and laid
out as [HTML is](#documents), on pages the window's size or to a width of your own. So it has what a
document has: `[` and `]` for the width of its line, `+` and `-` for its font size, `{` and `}` for
its margins, `D` for dark paper, `s` for its source blocks, each with its option; `/`
[searches](#searching-a-documents-text) it as it is shown; `f` [follows its links](#following-links);
`--pages` picks pages; and `o` opens the `.org` itself. Its look is Markdown's
([ADR-0016](adr/0016-how-markdown-and-html-look.md)).

**Which files are Org**: text named `.org` or `.org_archive`, in any case; and text, whatever its
name, whose first line (its second, where the first starts with `#!`) holds an Emacs mode line
naming Org, `-*- org -*-` or `-*- mode: org; … -*-` among its variables, in any case, as Emacs
reads it. So an Org file comes through a pipe too, with its mode line. A file Markdown or HTML is
known by first stays one, though: a `.md` whose first line says Org is shown as Markdown, which
Emacs would open as Org. It is read as UTF-8, or UTF-16 by
its byte-order mark. A local-variables block at a file's end (`Local Variables: … mode: org`) isn't
read: it is past what a file is told by.

**What it shows**:

- **Headlines** as headings, levels past 6 as 6: a TODO keyword in colour, red not done and green
  done, the keywords the file declares (`#+TODO:`, `#+SEQ_TODO:`, `#+TYP_TODO:`) honoured; a
  priority (`[#A]`) as written; its tags after the title, in grey. A heading's planning line
  (`SCHEDULED`, `DEADLINE`, `CLOSED`) and a clock line, small and grey, under it.
- **A title block** at the top, from `#+TITLE`, `#+SUBTITLE`, `#+AUTHOR` and `#+DATE`, wherever they
  are in the file.
- **Inline**: `*bold*`, `/italic/`, `_underline_`, `+strike+`, `=verbatim=` and `~code~`; `\\` a line
  break; `-----` a rule; `src_x{…}` as code. As written: timestamps, macros (`{{{x}}}`), LaTeX
  fragments, statistics cookies (`[1/3]`), and `_` and `^`, never a subscript or a superscript.
  Entities (`\alpha`) as the character they name.
- **Lists**, ordered, unordered and of descriptions; a checkbox `[ ]`, `[X]` or `[-]` as ☐, ☑ or ☒.
- **Tables** in a grid, as Markdown's: the rows before the first rule its header, where rows follow
  it; a row of alignment or width cookies (`<l>`, `<10>`) and `#+TBLFM` lines left out, and nothing
  evaluated. A table.el table (`+---+`) as code.
- **Blocks**: `#+begin_src` as code, or as a picture where a backend draws its language, as
  Markdown's fenced blocks are (`#+begin_src mermaid`); `example` and fixed-width lines (`: `) as
  code; `quote` as a quote; `verse` with its lines kept; `center` centred; any other block
  (`#+begin_note`) as its text; `#+RESULTS:` and what follows it shown.
- **Footnotes** of every kind (`[fn:1]`, `[fn:name]`, `[fn:: inline]`, `[fn:name: inline]`) as
  numbers, in the order they are referred to, the notes at the end, as Markdown's.
- **Pictures**: a link with no description to a file whose extension Org's HTML export inlines
  (`avif`, `gif`, `jpeg`, `jpg`, `png`, `svg`, `webp`), `[[file:x.png]]` or `[[./x.png]]`, is that
  picture, drawn as a document's are, from beside the file; a link whose description is such a
  link is that picture, linked. A picture on the web is a link: nothing is fetched.
- **What Org's export leaves out but you wrote**: a `COMMENT` heading and a `:noexport:` one, with
  what is under them.

**What it hides**: drawers (`:PROPERTIES:`, `:LOGBOOK:`, any `:NAME:` … `:END:`); `#+KEYWORD:`
lines but the title block's; comments (`# …`, `#+begin_comment`); export blocks but HTML's, whose
HTML is read as the HTML in Markdown is; `#+INCLUDE:` and `#+SETUPFILE:`, never read. Nothing is
run or evaluated: no Babel, no table formulas, no macros expanded, no dynamic block updated (its
text shows as last written).

Its links, and the places they lead to, are under [Following links](#following-links).

An Org file in a list is opened when its tile is wanted. One in an archive is read from it, its
pictures from beside it there. orgize takes time that grows with what a file nests or leaves open,
so it works in a process capped on time and memory, and an Org file past them is refused, with
`orgize took too long on the document` ([Limits](#limits),
[ADR-0040](adr/0040-org-mode-through-orgize-in-a-worker-process.md)).

### Spreadsheets

A workbook (xlsx, xlsm, xlsb or xls), an OpenDocument spreadsheet (ods) or a file of values
separated by commas or tabs (csv, tsv) is shown as [gridcraft](https://github.com/storytold/gridcraft)
prints it: each visible sheet's pages, laid out by its print area, page setup, title rows,
headers and footers, gridlines and charts, in the order of its sheets; a hidden sheet prints
none. Its formulas are recalculated first, as gridcraft's app does when it opens a workbook, so a
formula saved without its value shows one. gridcraft prints the pages to a PDF, which the PDF
backend shows like any other, fitted to the window, and `/`
[searches](#searching-a-documents-text) as it does any other. A CSV's columns are made as wide as what they
hold, as gridcraft's app makes them on importing one. A protected workbook opens with
`--password`; without it, clv says it needs one.

What gridcraft says on reading a workbook comes as a warning with its first page: a sheet cut
short at one of its [limits](#limits), and that an ods or an xlsb is shown by its values alone
(gridcraft reads their formatting, charts and pictures from neither). So does text in letters other
than Western European ones (Greek, Cyrillic, Chinese…, and symbols such as `≤`): the PDF gridcraft
prints to sets it in a font it doesn't carry, and the PDF backend has none to put in its place, so
that text is blank. Arabic is the exception where Noto Sans Arabic is installed (as
`NotoSansArabic-Regular.ttf`, `-Bold.ttf` or the variable `NotoSansArabic[wght].ttf`, in
`/Library/Fonts`, `~/Library/Fonts`, `~/.local/share/fonts`, `~/.fonts` or `/usr/share/fonts`'s
Noto folders): gridcraft shapes it and embeds that face, so it is drawn. Without it, Arabic is
blank too.

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
- **A search's matches** are tinted in the cells' colours, since the cells are sampled from the
  tinted page ([Searching a document's text](#searching-a-documents-text)).
- **The labels of a page's links** (`f`) are capitals over the page's cells, a cell a letter
  ([Following links](#following-links)).
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
either way. The same guess says whether `c` may use this machine's clipboard
([Copying a page](#copying-a-page)) and whether `o` opens a file in an app here: over SSH it
doesn't, since the app would open on the far machine ([Opening in another app](#opening-in-another-app)).
A web address a link leads to isn't opened over SSH either: it is copied to your clipboard instead
([Following links](#following-links)).

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
- The labels of a page's links (`f`, [Following links](#following-links)) are text over the page's
  placeholders, a cell a letter, so tmux keeps them as it keeps the page: a terminal that attaches
  while they show gets them too.
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
| What an SVG's page takes to render: a layer for each group with opacity, a clip path, a mask or a filter, a filter's buffers, a pattern's tile, each HTML label's part of the page, and the pixels painted | 1 GiB held at once; 4,000 million pixels painted (about seven seconds) | the page fails with why; the same drawing smaller, its tile in a list, still renders |
| An SVG's HTML labels | 1,024 labels; 100,000 elements in all; 8 MiB of markup and 1 MiB of text; 1,024 levels from a `<foreignObject>` down; their stylesheets' rules times their elements, with the drawing's, 50 million | past a count, the rest are left out; nested deeper, that label; past the rules, all; with a warning |
| An HTML label's size | 16,384 pixels a side | that label is left out, with a warning |
| What an HTML label fetches | `data:` URLs, 1,024 of them; a font unpacking to 32 MiB, 64 MiB of them, a WOFF whose tables don't fit its file refused; a picture decoding to 64 MiB, 256 MiB of them | refused: the label is drawn without it |
| The worker that lays a drawing's HTML labels out and paints them, a process of its own: its CPU time laying them out, then each render's; its memory | 10 s, then 5 s a render, or three times that on the clock; 1 GiB | the worker is killed, and all of the drawing's labels are left out, with a warning, until it is opened again |
| Drawings whose HTML labels are shown at once, each with its worker, a search's process and Mermaid's worker with them | 6 | the next drawing's labels are left out, with a warning; a search's text is left unread, and the status line says so; a diagram fails, saying so |
| A Mermaid diagram | 1 MiB, and UTF-8 | refused |
| A Mermaid diagram, as merman reads it | 500 edges; 16,000 items (nodes, messages, members…); 1 MiB of text in them; 128 levels deep; what its layout may work through; 1.5 s of merman's work | refused, with merman's message |
| The worker that turns Mermaid diagrams into SVG, a process of its own: its CPU time on a diagram; its memory | 2 s, or three times that on the clock; 512 MiB | the worker is killed and that diagram fails, saying which; the next diagram starts another |
| Mermaid's workers killed in a row | 3 | no diagram is drawn for 60 s: each fails, saying so |
| The SVG a diagram is drawn to | 24 MiB, merman's own; then an SVG's, above: a diagram near 500 edges with arrowheads (32 nodes each linked to every other) comes to more than an SVG's 128 MiB once its arrowheads are repeated | the diagram fails, with why |
| Diagrams kept, drawn or failed, while clv runs | 4,096, and 64 MiB of them and their SVG | those used longest ago are let go of, and drawn again if opened again |
| A PDF's pages | 100,000 | refused, saying how many it has; a page tree may name one page over and over, so a file of a few megabytes can have millions, and each costs some 2 KB to open |
| A Word document, OpenDocument text or RTF | 256 MiB | refused; what else it may cost is bounded by wordcraft's own limits (how much a docx unpacks to, how deep it nests) |
| An Org file | Markdown's, below, counted before it is parsed: 64 MiB; half a million lines and marks (its punctuation); then the HTML it is written as, as any HTML file's | refused, with which: `as the HTML it is written as, …` for the HTML's |
| The process that parses an Org file and writes it as HTML: its CPU time, and its memory | 2 s, or three times that on the clock; 512 MiB | the process is killed, and the file refused: `orgize took too long on the document` |
| Org files written as HTML, kept while clv runs | 64, and 64 MiB of them and their HTML | those used longest ago are let go of, and written again when laid out again |
| A Markdown or HTML file | 64 MiB; half a million lines and marks (HTML's tags, Markdown's punctuation); 8 MiB of text, some 5,000 pages, which take 20 seconds to open and 1.6 GB | refused. Within them, a file made to cost the most takes about 6 GB and 15 seconds, at 20 characters a line as at the window's width, at the largest font size a window holds as at the smallest; laid out again (`]`, or the window resized), with the old layout held until the new one shows, that file peaks at 9.5 GB |
| A document's pictures, in Markdown or HTML | 1,024 referred to, the fenced blocks it draws among them; each held at 1,600 pixels at most on its longer side; 256 MiB held in all | past 1,024, left out, with a warning, a block as its code; past 256 MiB, those drawn longest ago are let go of, and drawn again when wanted |
| A book (epub): its `container.xml`; its package (OPF); its list of protected files (`encryption.xml`) | 64 KiB; 4 MiB; 1 MiB, and 16,384 files | refused |
| A book's reading order | 4,096 chapters; a file's path of 4 KiB | refused |
| A book's chapter | 16 MiB | that chapter is left out, and the first page says so |
| A book, its chapters joined | 48 MiB; 400,000 lines and tags; 6 MiB of text between its tags, some 3,700 pages: below what any HTML file may be, above | the chapter that would pass one is left out, with those after it, and the first page says how many |
| A book's table of contents (its nav document, or its NCX) | 1 MiB; 4,096 entries; a label of 256 bytes; 8 levels | past 1 MiB, the book has no Contents page, and its first page says so; past 4,096, the rest are left out, and the first page says so; a longer label is cut; a deeper entry is shown at the eighth level |
| A book through a pipe, or in an archive | 256 MiB | refused |
| A place in a book: a chapter's `id`, with its chapter's number before it | 256 bytes | that `id` is no place, and a link to it says `no such place in this book` |
| The process that reads a book's package, and another its table of contents: its CPU time, and its memory | 2 s, or three times that on the clock; 256 MiB | the process is killed; on the package, the book is refused: `its package couldn't be read`; on the table of contents, the book has no Contents page |
| A fenced block drawn as a picture | what a file of its kind may be: a Mermaid diagram's 1 MiB, and merman's limits | it stays code, and the first page says how many did |
| A document's page | 64 million pixels | the page fails with why |
| A spreadsheet's file | 64 MiB | refused |
| An xlsx's sheets, unpacked, together | 128 MiB, which would take some 850 MB once read (one of 97 MiB took 649 MB) | refused |
| A sheet's XML, as gridcraft reads it | a million elements (about 37,000 rows of twelve columns) | the sheet is cut short, with a warning |
| A workbook's pages, as gridcraft prints them | 2,000, down a sheet's rows before across its columns | a very wide sheet loses its right-hand pages |
| A PDF whose text is [searched](#searching-a-documents-text) | 512 MiB | its text isn't read, and `/` says so |
| The process that reads a PDF's text for a search: its CPU time on a page, and its memory | 5 s a page, or three times that on the clock; 768 MiB, the PDF included | the process is killed and that page left unread, the status line saying which and why; another reads the rest |
| A PDF's text, for a search | 1 MiB a page; three processes killed; 120 s of reading in all | past 1 MiB, that page is left unread; at a fourth killed, or past the 120 s, the rest is, and the status line says how far the reading got |
| A document's paragraph, for a search | 1 MiB | that paragraph is left unread, and the pages it is on, the status line saying how far the reading got and why |
| What a search keeps of a file's text, and its matches | 64 MiB; 100,000 pages, or a document's paragraphs; 10,000 matches | past 64 MiB or those, the rest is left unread, and the status line says how far the reading got; past 10,000, the first are kept, and counted as `10000+` |
| The matches tinted at a time, on the pages on screen and those kept ready around them | 256, and 64 lines of one match | the rest aren't tinted; `n`, `N` and `p` still go to them |
| The pages on screen and a page sent again with its tint, together | 256 MiB of pixels, what the terminal is given to keep | the page waits for another's old copy to go; where none will, it is blank until its tinted copy arrives |
| What is typed after `/`, to filter or to search | 256 characters | more are ignored |
| A page's [links](#following-links) | 1,024, each active over 64 rectangles at most; an address or a path of 4 KiB, a place's name of 256 bytes | past 1,024, the rest have no label, and the status line says how many; past 64, the rest of a link's rectangles are left out; a link whose address, path or name is longer isn't followed, and says so |
| The links labelled at once | 1,000, in reading order | the rest have no label, and the status line says how many |
| The wait for the links of the pages on screen | 2 s | a page that hasn't answered has no labels, and the status line names it |
| The places `backspace` goes back to | the last 100 links followed in a file | an older one is forgotten |
| Files followed into through links, each from the one before | 16 | the next link to a file isn't followed, and the status line says so; `backspace` goes back |
| A web address followed, or an address copied | 4 KiB | not followed, saying so; not copied |

## Known issues

- **An SVG's stylesheets take time as the square of a declaration's length to parse.** One long
  declaration in a `<style>` (a `box-shadow` list of 600 KB) takes some 45 seconds to open, labels
  or not, and a stylesheet of 3 MiB some 8; the [limits](#limits) above count rules times elements,
  not that. The parse runs in `clv`'s own process, so it can't be stopped; `q` still quits.

- **A fixed-layout book is laid out as any other.** A book whose package says its pages are made
  one by one (`rendition:layout` `pre-paginated`: a comic, a picture book, a magazine) has its
  chapters' text and pictures flow down pages the window's size, not placed as its maker placed
  them, and its first page says so
  ([backlog](backlog.md#fixed-layout-epubs-through-blitz)).
- **A book's own stylesheets are not read.** An epub is laid out in `clv`'s own look, with only
  what its markup's `style` attributes say on top: a book that counts on its CSS for its headings,
  its indents, its drop caps or its centred lines comes out plainer than a reader app shows it
  ([Books](#books)).
- **An Org list goes on past two blank lines.** Org ends a list at two blank lines in a row;
  orgize, which parses Org for `clv`, keeps the items after them in it, so two lists written so show
  as one ([Org documents](#org-documents)).

## Errors

- A page number past the end of the document fails before anything is drawn.
- A page that can't be rendered is reported at the end. The other pages are still drawn, and
  `clv` exits with an error.
- What a backend warns about on a page (content it left out, a colour it had to guess) is said
  with the page's number: on stderr when printed, on the status line in the pager.
- File names and messages are written with their control characters replaced, so neither a file
  name nor a document can send escape sequences (a clipboard write, a window title) to the
  terminal through them.
