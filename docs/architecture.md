# How clv works

This page is for working on the code: how a file becomes pages, how pages get to the terminal, what
is kept there, what can go wrong on the way, and why each choice was made. What clv does, as someone
using it sees it, is in [`docs/usage.md`](usage.md). How to build and test it, and the rules its code
keeps, are in [`CONTRIBUTING.md`](../CONTRIBUTING.md). The alternatives rejected, with their
evidence, are in [`docs/adr/`](adr/README.md).

The numbers were measured on the macOS laptop it was developed on (kitty 0.45, tmux 3.5a), by
black-box observation. The same probes, run again against kitty 0.49.2, found the same behaviour:
temporary files read and deleted, directly and through tmux, the same answers, the 320 MB quota and
the cell size.

## The crate

A file, named on the command line, found in a directory or read from an archive, goes three steps:

1. **A backend opens it.** `source::BACKENDS` lists this build's, and the first that says it opens
   the file, from its first bytes, gives a `Source` of pages or, for an archive, a `Container` of
   files.
2. **The viewer asks for pages.** The pager or the printer asks the source for a page at a scale, and
   gets a `Frame` of pixels back. It knows what it shows only through the traits in `src/source.rs`.
3. **The pages reach the terminal**, as PNGs in the stream or raw pixels in a temporary file,
   directly or through tmux, and the pager keeps track of what the terminal still holds; or, without
   the graphics protocol, as [text cells](#text-cells-cellsrs).

### Layers

The code is three layers, and what each may depend on is a rule
([CONTRIBUTING](../CONTRIBUTING.md#rules), "The core never depends on an extension"):

| Layer | Where | What it is |
|---|---|---|
| **Core** | `src/main.rs`, `src/args.rs`, `src/encode.rs`, `src/view/` | The program: the entry point, the options, the pager and the printer, the terminal. It knows no file format |
| **Contract** | `src/source.rs` | How the core sees what it shows: the traits (`Backend`, `Source`, `Render`, `Pool`, `Frame`, `Container`), what passes between them (`Room`, `Bounds`, `Setting`), the generic sources it re-exports (`Directory`, `Joined`, `Later`, `Folder`) and `BACKENDS`, the one list of this build's backends |
| **Extensions** | `src/source/*.rs` | The backends, one each behind a cargo feature ([Backends](#backends)), and the modules that serve them |

- **The core depends on the contract alone.** Nothing in it names a module of `src/source/`:
  whatever the viewer needs of a source, `source.rs` declares or re-exports. The viewer doesn't
  even name `BACKENDS`: `main.rs`, wiring it all up, is the one core file that does, and hands the
  list to `view::run` and `view::print_help`. So the core builds and its tests run with no backend
  at all (`cargo clippy --no-default-features`).
- **The contract names the extensions in one place**, `BACKENDS`, each line behind its feature,
  and uses the core only for its options' types (`args`); its tests also check no backend claims
  one of the viewer's keys.
- **An extension depends on the contract, and on the core's utilities**: `args` for its options,
  `encode` for a picture a document holds. Never on `src/view/`. Never on another backend by name:
  the spreadsheet backend prints a PDF and asks `BACKENDS` which backend opens it, through the
  `Backend` trait. A backend's library is named in its file alone (CONTRIBUTING's rule "A backend's
  library is its own"), so swapping a renderer touches one file.

`source::tests::the_viewer_names_no_backend` checks the first rule over every core file, and
the per-backend tests (`tests/it/pdfcraft.rs`, `resvg.rs`, `tar.rs`, `wordcraft.rs`,
`gridcraft.rs`) that no file but the backend's names its libraries.

### Files

| Where | What it does |
|---|---|
| `src/main.rs`, `src/args.rs` | The entry point, which hands `source::BACKENDS` to the viewer; command-line options and page lists |
| `src/encode.rs` | Rendered pixels into a PNG, fast or small, with straight alpha. Pure. For the viewer, and for a document to hold a picture |
| `src/view/` | The viewer, below |
| `src/source.rs` | The traits (`Source`, `Render`, `Pool`, `Frame`, `Backend`, `Container`) and `BACKENDS` |
| `src/source/<backend>.rs` | One backend each ([Backends](#backends)) |
| `src/source/svg/html.rs` | An SVG's HTML labels, laid out, recorded and replayed by Blitz for `svg.rs`: the one file that names Blitz |
| `src/source/threads.rs` | Render threads for a backend with nothing better |
| `src/source/picture.rs` | The pictures a document refers to, drawn by the backends that show them |
| `src/source/dir.rs` | A directory as a `Container`: its files and subdirectories, in name order (`-r` for its subdirectories' files instead) |
| `src/source/joined.rs` | Several sources as one: a list's tiles |
| `src/source/later.rs` | A list's file not opened until its tile is wanted, and the threads that open them |
| `src/source/folder.rs` | A container as a list shows it, to be opened: one page, a folder drawn in code |
| `src/source/head.rs` | What the first bytes of a zip or a compound file say, for the Word and spreadsheet backends |
| `src/source/tree.rs` | The directories an archive's names make, for zip and tar |
| `assets/` | The icon `--help` shows: its drawing, and the PNG built into the binary |
| `build.rs` | The version `--help` and `--version` show, as `CLV_VERSION`: `Cargo.toml`'s, and past a release the commit, from `git describe` (`0.3.0+50.g3f677cb`, `.dirty` when tracked files changed). It reruns when a commit, a tag or a staged file changes the answer, and gives the version alone without git or outside clview's checkout |

## The viewer (`src/view/`)

| File | What it does |
|---|---|
| `mod.rs` | Arguments, the checks before drawing, printing pages without the pager (`--no-pager`, a single page), in rows as zoomed, choosing PNG compression and whether to try temporary files |
| `detect.rs` | Whether the terminal speaks the protocol, from the environment and its answer to a query, and the terminal's answers about its cell size and its background. Pure |
| `tty.rs` | `/dev/tty`: size, raw mode, and reads with a timeout through a reader thread (on macOS, `poll(2)` on `/dev/tty` answers `POLLNVAL` at once) |
| `keys.rs` | Bytes from the terminal into keys, and the terminal's answers to graphics commands and about its clipboard. Pure, total over any bytes |
| `fit.rs` | The render scale and the cells a page covers, the zoom, and how many pages share the window. Pure arithmetic |
| `kitty.rs` | The protocol's escape codes, and the Unicode placeholders used inside tmux. Pure |
| `cells.rs` | Rendered pixels into lines of text cells, for a terminal without the protocol ([Text cells](#text-cells-cellsrs)). Pure |
| `tmux.rs` | Asking tmux: its client's terminal, `allow-passthrough`, SSH, who shows the pane, what a pane holds, and the server's options. It knows nothing of images |
| `sweep.rs` | Pages printed inside tmux: recorded in the tmux server, and deleted from the terminal once no pane holds them. The rule is its own, the cells are read through `kitty.rs`, and tmux is asked only through `tmux.rs`: this pane, the panes, what one holds, and the records |
| `pager.rs` | The pager: navigation (`Nav`, no IO), rendering and sending ahead, what is kept in the terminal, redraws |
| `files.rs` | Temporary files for a terminal on this machine, on a thread of their own |
| `icon.rs` | The icon beside `--help`'s first lines: whether it can show, where it goes, and its escape codes ([The icon in `--help`](#the-icon-in---help-iconrs)) |
| `clip.rs` | Copying a page to the clipboard (`c`): the clipboard protocol's escape codes, the scale a page is copied at, and a copy's way from the pool to the terminal's clipboard or the system's, on threads of its own |

The viewer is Unix only (macOS, Linux, FreeBSD): it needs `/dev/tty`. Nothing in it is `unsafe`.

## Sources (`src/source.rs`)

The pager and the printer never see a PDF. They see a `Source`: how many pages it has, each
page's size in the source's own unit (points, for a PDF), a renderer on this thread for printing,
and a pool of render threads for the pager. A render is asked for as a page and a scale (output
pixels per unit) and comes back as a `Frame`: premultiplied RGBA in the backend's own buffer, so
the pixels reach the terminal without a copy, or the error, and the warnings for the reader.

Each backend brings its own threads, because only it knows how to stop a page. The PDF
backend's pool tells a page that takes too long to stop at its next drawing operator, retries a
page that had only waited behind another page's decode, and bounds the threads it replaces; a
pool in the viewer could only abandon the thread. What the pager relies on is that pool's contract:
`set_queue` replaces the renders wanted, most urgent first; a result is delivered only while its
request is still listed; `try_recv` never blocks. A backend that has no better way, such as
images and drawings, gets plain threads from `source/threads.rs`: a render once begun runs to its
end, and one no longer wanted is dropped when it is done; a render that panics comes back as a
failed page.

## Backends

A `Backend` opens sources. `source::BACKENDS` lists this build's, and a file goes to the first
that says it opens it, going by the file's first 1024 bytes (`Backend::opens`). Those bytes are
read once and handed on with the rest of the file (`Input`), so a pipe opens as well as a file. A
backend opens a file to pages (`Opened::Pages`) or, for an archive, to the files it holds
(`Opened::Files`, a `Container`).

All are on by default, and asked in this order, the ones that go by how a file starts before the
PDF one, since a PDF's header may come anywhere in its first 1024 bytes:

| Asked | File | Feature | How it tells its files | Why asked there |
|---|---|---|---|---|
| 1 | `svg.rs` (`svg/html.rs`: its HTML labels, with `svg-html`) | `svg` (`svg-html`) | XML whose root element is `<svg>`, after whatever may come first (a declaration, comments, a doctype), and svgz, gzipped, by what it unpacks to; a file whose first 1024 bytes end before its root element, by its name | Those bytes may hold a PDF's header, in a drawing's text |
| 2 | `tar.rs` | `tar` | A POSIX or GNU header first (`ustar`, its checksum right); gzip, bzip2 and xz by their signature: a compressed tar as an archive, anything else as the file it unpacks to | As for SVG: a tar's first file starts 512 bytes in |
| 3 | `wordcraft.rs` | `word` | An RTF by its first bytes (`{\rtf`); an odt, a zip, by its first file (`mimetype`, saying OpenDocument text); a docx, a zip, by a file in `word/` among those its first 1024 bytes hold, or by a Word name; a Word 97-2003 document, a compound file, by such a name only; HTML by how it starts (a doctype or an `html` element), or by its name and that it is text; Markdown by its name only, and that it is text | As for SVG (an RTF's text), and before zip, since a docx is a zip |
| 4 | `gridcraft.rs` | `sheet`, which brings `pdf` | An ods, a zip, by its first file (`mimetype`, saying OpenDocument spreadsheet); an Excel workbook, a zip, by a file in `xl/` among those its first 1024 bytes hold, or by an Excel name; an Excel 97-2003 workbook, a compound file, by such a name only; values separated by commas or tabs by their name only, and text | Before zip, since an xlsx is a zip |
| 5 | `pdfcraft.rs` | `pdf` | `%PDF-` in its first 1024 bytes | After those above, whose files may hold that |
| 6 | `image.rs` | `image` | Their signatures; TGA, which has none, by its name | |
| 7 | `zip.rs` | `zip` | Its signature (`PK`), whatever its name | Last, for the zips the Word and spreadsheet backends don't take |

`source/head.rs` reads what a zip's first files say, for the Word and spreadsheet backends.

### What a backend says of itself

Each backend declares its own options (`Backend::options`): the viewer parses the command line
against its own and every backend's, lists them in `--help` under the backends that take some,
and refuses one the chosen backend doesn't take. The backend's `open` gets the parsed command line
and reads its own options from it, so an option that belongs to one kind of file (a PDF's
`--password`) is the backend's, never the viewer's. The viewer opens every file through
`Backend::open_at`, which adds the `Room` the file is shown in ([The room a document is laid out
for](#the-room-a-document-is-laid-out-for)): a default method calling `open`, which only the
documents' backend overrides, for the files that have no page of their own.

A backend declares its settings the same way (`Backend::settings`): what the reader may change while
its files are shown, each a `Setting` with its named values and what each says, its numbers
(`Numbers`: least, most, step, and the unit a number is said with, `%` or nothing), its default,
where a count of 0 sends it (`zero`: `0]` is `window`, `0+` 100%), `auto` by the terminal's
background, its keys (`Keys::Cycle`, `Keys::Step`, or `Keys::Zoom`, the viewer's own zoom keys, below),
its option, its line in `?`, its message for a file that hasn't it, and its `Effect`: `Redraw`, the
pages drawn again with the value, or `Relayout`, the document laid out again at it
([Relayout](#relayout)). The documents' backend declares three, `paper` (`d`, `--theme`, a redraw),
`width` (`[` and `]`, `--width`, a relayout) and `font` (`+` and `-`, `--font`, a relayout). Which a file has is its source's
(`Source::settings`; a list's, `Joined`'s, those of any of its files). Where a setting's values sit
for a file depends on the room it is shown in, so the source says (`Source::bounds`, a `Bounds`): where
its named value sits among its numbers (`named_at`: `window`, at the characters a line of the
window's page holds) and the most a number is laid out at there (`most`: the longest line the room
holds). The level asks when it is made and keeps the answer on its `Nav` (`Nav::bounds`), and `turned`
takes it. `view::settings` makes the
build's table, each setting once. The viewer takes a setting's option for any file (`takes_all`),
starts each at what its option says or its default, resolving `auto` by the background (`starting`),
and asks the terminal for its background only when what is shown has a setting with `auto`, or is a
list. Every request carries the values (`Request::values`, a `Values` of at most `MAX_SETTINGS`, so a
request stays `Copy`) and dark paper's colour (`Request::dark`), and the backend draws by them. The
pager's `Nav` takes a key the viewer's keys don't take to the build's settings (`setting_key`), then
says the setting's message if what is shown hasn't it; a redraw value changed goes the way `r` goes. A `Cycle` key goes to the next named
value; `Step` keys to the next among the steps down or up (`stepped`): the multiples of the step from
the least up to the most here (the setting's, or the room's where that is under it), and the named
value set among them (`Bounds::named_at`), after the number equal to it (`window` at 94 in a window
of 100 columns: from 92 `]` goes to 94, then to `window`, which says `the most here`; in a window
that holds a book, `window` sits at a page's characters and the steps go on past it to the window's
width), staying at the least or the most and saying so; a number typed before a numbered setting's
key sets it, clamped to its range, used up whatever happens, and 0 to the setting's `zero`
(`turned`). A number past the most here is kept, laid out at the most, and said with what it is laid
out at, `width 120 (window)`, `font 300% (170%)` (`said_in`): `]` from it says so, `[` goes to the last
step under it. `Keys::Zoom` steps the same way on `+`/`=` and `-`/`_` (`Keys::up`), but those are the
viewer's keys: `Nav::key` asks first (`zoom_setting`) whether this build declares a setting on them
that the document shown has, shown by itself (not a list, not pages picked by `--pages`), and only then
hands them to it; everywhere else they zoom as before, so the viewer's keys still come first but for
these, on such a document. The one setting `?` doesn't list after the viewer's keys: `help_lines` shows
one `+ -` line, the setting's in place of the zoom's where the keys go to it, and the status line's
first reminder says `+ - font` there (`KEYS`, `OPENED_KEYS`). `a_settings_keys_are_its_own_in_every_backend`
lets `Keys::Zoom` through, alone among the viewer's keys ([ADR-0023](adr/0023-the-window-is-the-page-of-markdown-and-html.md)).
The values are shared by every level (`pager::Settings`), and so is the change, so a level left before
the next draw (`d` then `u`, read together) doesn't keep the parent's pages on the old value. `source::tests::a_settings_keys_are_its_own_in_every_backend`
checks that no setting has one of the viewer's keys (`VIEWER_KEYS`, which the pager's
`the_viewers_keys_are_those_listed` checks) and that a key means one setting
([ADR-0021](adr/0021-a-backend-declares-its-settings-and-their-options-hold-for-any-file.md)).

A backend whose `open` costs more than a list should spend on a file the reader may never come to says
so, going by the file's name and first bytes (`Backend::deferred`), with what a list needs unopened:
the settings it has. Documents, PDFs, drawings and spreadsheets defer. Images don't, as
their `open` reads a header, and archives don't, as a tar may turn out not to hold files. A backend that
defers promises pages, never a container
([ADR-0019](adr/0019-a-list-opens-its-files-when-their-tiles-are-wanted.md)).

A backend says what it shows in a word or two (`Backend::shows`: "PDFs", "documents"), which
messages and `--help` give, and which formats (`Backend::formats`): each a `Format`, its name and
the extensions that tell a file of it where its first bytes can't. `--help` lists them last, under
what their backend shows, nothing about formats written in the viewer. A backend takes them from
the tables its `opens` goes by (the image backend's `DECODED`, the extensions a document's or a
workbook's kind is told by, `Codec`'s compressions in tar's), so what `--help` lists and what opens
can't disagree; `the_extensions_a_backend_lists_are_the_names_it_opens_by` (`source.rs`) checks
every backend built. A backend also names the libraries it reads and draws its files through
(`Backend::credits`), which `--help` credits in its row of formats, beside what it shows: the names
are written in the backend's own file, so the viewer still names none.

Nothing in `src/view/` names a backend ([Layers](#layers)): `source::tests::the_viewer_names_no_backend`
checks the files, and `cargo clippy --no-default-features` builds the viewer with no backend at all.

### How each opens, and what it keeps

- **A PDF** renders through pdfcraft, the way its app draws it, a git dependency pinned in
  `Cargo.toml`.
- **A drawing** renders through resvg
  ([ADR-0002](adr/0002-render-svg-with-resvg.md)), within limits of clv's own on what it costs to
  parse and render, since resvg's are few. It parses once to open and keeps only where its bytes are; its
  renderer or pool parses it again on its first render and keeps the tree for renders at other
  sizes, each of which is estimated before it starts
  ([ADR-0003](adr/0003-bound-what-a-drawing-costs-before-usvg-and-resvg-spend-it.md)). The system's
  fonts are looked for only once a drawing has text (about 15 ms on macOS). Its HTML labels
  (`<foreignObject>`, the `svg-html` feature, `svg/html.rs`) are found by `svg.rs` in one walk
  (`drawn`), placed through the drawing's viewports and written out from the parsed tree inside
  copies of their context (`found_labels`); what follows the child a `<switch>` takes is cut from
  the text before usvg parses it (`without_fallbacks`). The render's parse, not the open's, lays them
  out with Blitz in one document a drawing, each label a slot placed by a transform, on a thread of
  its own with an 8 MiB stack (`prepare`); estimates the painting (`paint_work`), and records each
  label once into an anyrender `Scene` through a `Recorder` that keeps at most 200,000 commands.
  Each render replays a label into the part of the page it lands on, that part and its commands in
  the page's estimate, and draws it over resvg's pixels at its opacity
  ([ADR-0024](adr/0024-draw-an-svgs-html-labels-with-blitz-one-document-a-drawing.md)). Its net
  provider (`DataOnly`) answers `data:` fonts and pictures only, each sized from its header.
  Blitz asks it for a URL without saying what for, so the labels' markup holds no element that
  loads a stylesheet or a document (`LOADING`), and a label's `<style>` joins the drawing's
  stylesheets, which Stylo reads and writes out again without `@import` (`without_imports`).
  The system's fonts are found once a run.
- **A document** goes through wordcraft's readers, layout and renderer, not its engine, whose PDF
  export needs patches. It is read and laid out to open, and keeps only where its bytes are and its
  pages' sizes; its renderer or pool reads and lays it out again on its first render, and keeps
  that ([ADR-0005](adr/0005-word-documents-through-wordcrafts-readers-layout-and-renderer.md)).
  HTML comes through wordcraft's importer as a fragment (`html::import_with`), whose text takes the
  document's styles, not as a web page (`import_on`), which sets Times New Roman over them
  ([ADR-0017](adr/0017-htmls-style-rules-apply-as-far-as-wordcraft-reads-them.md)), on a page the
  window's size, since it gives none ([The room a document is laid out for](#the-room-a-document-is-laid-out-for)),
  and Markdown through pulldown-cmark into HTML for it
  ([ADR-0014](adr/0014-markdown-through-pulldown-cmark-into-wordcrafts-html-importer.md)). What they
  cost is bounded before it is spent, by their marks before they are imported and their text before
  they are laid out ([ADR-0012](adr/0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md)).
  How they look is set after they are imported, by `styled`, in the document's styles where those can
  say it, and on its paragraphs and runs where they can't
  ([ADR-0016](adr/0016-how-markdown-and-html-look.md)).
  The pictures they refer to are laid out as stand-ins, PNGs saying only the size to show one at, from
  `Source::size`, so neither reading nor laying out reads a picture's pixels. wordcraft's reader holds
  a picture to a 468-point column whatever the page, and to its stand-in's whole pixels, so `fitted`
  gives one the document gives no size to the size it was measured for (`Pictures::shown`), and
  shrinks any other to the column and the page. A page's own are drawn
  when it first renders, by the first backend that says `draws_pictures` and opens the file, and
  replace their stand-ins in the document's `media`, under a lock that pages rendering share
  ([ADR-0013](adr/0013-a-documents-pictures-are-drawn-by-the-backends.md)).
  Markdown and HTML have the `paper` setting: asked for `dark` in a request's values, they render
  through wordcraft's dark page, on the grey as bright as the request's dark colour, then stretched to
  it (`on_paper`). The viewer holds the value and puts it in each request, so `d` needs only what `r`
  does ([ADR-0015](adr/0015-markdown-and-html-on-dark-paper-the-terminals-own-colour.md)). They have
  the `width` setting too, which is the source's own (`Width`, from `--width` when it opens, `window`
  or a number of characters), and `font`, a percentage (`Document::font`, from `--font`), and are
  laid out for the room they are shown in (`Document::room`): the page the three give (`page_in`,
  below) goes into wordcraft's conversion of the fragment
  (`html::parse_with`, then `model::to_doc_in`), where tables take their widths from it.
  `Source::laid_at` says the width, the font size and the room, exactly as given, and `Source::relayout`
  gives a job that reads the bytes again from where they are kept (`Document::at`) and lays out at
  another width or font size, or for another room, as a new `Paged`
  ([ADR-0022](adr/0022-markdown-and-html-reflow-to-a-width-in-characters-as-a-new-level.md),
  [ADR-0023](adr/0023-the-window-is-the-page-of-markdown-and-html.md)).
- **A spreadsheet** is printed to a PDF when it opens, through gridcraft's engine, and opened as that
  PDF by the backend that opens it, found the way any file's backend is, not by name; what gridcraft
  said on reading the workbook rides with the first page's warnings
  ([ADR-0006](adr/0006-spreadsheets-printed-by-gridcraft-to-a-pdf-the-pdf-backend-shows.md)).
- **An image** opens by its header alone and decodes on its renderer's or pool's first render (on a
  render thread, so the keys work while a large photo decodes), which keeps the pixels for its
  renders at other sizes and lets them go with it; a picture on disk is read again to decode, one
  from a pipe kept as it came, one in an archive read from it again.
- **A zip** has its central directory parsed once, and a file read from it on demand: the first
  bytes, for `Input`, and the whole file through the `Input`'s `Again`, on a render thread. It holds
  no open file while idle, so a directory of archives doesn't run out of file descriptors.
- **A tar** is listed through `Container::ready`: a compressed one is read through once, on a thread
  of its own, into a temporary file removed as soon as it is made
  ([ADR-0004](adr/0004-read-a-compressed-tar-through-once-into-a-file-nothing-names.md)). A file
  compressed alone is opened as what it unpacks to.

### The room a document is laid out for

Markdown and HTML have no page of their own, so the window is their page
([ADR-0023](adr/0023-the-window-is-the-page-of-markdown-and-html.md)). What the backend knows of the
window is a `Room` (`source.rs`), all of it the viewer's to say: the pages area's columns and rows (the
window less the status line's row), a cell's width and height in pixels as `f32`, exactly the cell the
grid is fitted with (`Output::fitting` included: for text cells a sampling cell, 1.25 × 2 for a 10 × 16
terminal cell, which rounding would reshape), the cells between two pages side by side (`fit::GAP`),
and `per_row`, the pages to a row the reader asked for, as the grid draws them (`None` at `auto`).
It isn't `fit::Area`: the contract takes nothing from the core. Rooms are compared only through
`Room::same`: the same columns, rows, gap and `per_row`, and a cell within 2% of the other's each way, so a window
dragged a few pixels, which changes a cell's size by a fraction, lays nothing out again, and a font
change in the terminal, which changes it by more and the columns with it, does.

- **Where a room comes from.** One function, `view::area`, makes the cells pages go into from a
  window's size, the cell tmux or the terminal reported where the tty reports no pixels, and the
  output, and `view::room` turns that into a `Room`: the pager's `draw` lays its grid out in the one,
  `print_pages` prints in it, and a document is laid out for the other, so that what is laid out and
  what is shown agree. Files open where they did before, before the terminal is asked anything and
  before raw mode, so Ctrl-C stops a long layout and leaves the terminal as it was
  (`first_rooms`): a file alone is laid out for the window the tty says without a question, its cell
  from the pixels it reports, or tmux's client's cell inside tmux, or `fit::FALLBACK_CELL`, with the
  output guessed from the protocol asked for (text cells by name sample; inside tmux the placeholders
  bound the columns). A list's documents are laid out for `PREVIEW`, 60 × 40 cells at the cell the
  run started with, a portrait page, as tiles are square; they keep it, as previews, and `Later`'s
  `Slot` opens with it. Printing with no terminal lays out for the preview's room too. The `Opener`
  carries both rooms, and `Backend::open_at` takes the one that fits (`Opener::file`).
- **The count to a row** ([ADR-0024](adr/0024-pages-to-a-row-of-markdown-and-html-are-columns-and-the-last-key-wins.md)).
  `first_rooms` gives a file alone `--per-row`'s count (`view::columns`: one for `width`), clamped to
  `fit::max_per_row` of the room, unless `--pages` picks the pages or a `--width` of a number disagrees,
  which wins with a warning once the file is known to reflow; a list's preview room has none. The
  pager keeps its `room` at `auto` and adds the reader's count where it lays out and asks bounds
  (`Pager::room_for`, from `Nav::columns_in`: the count asked, on a document that `Nav::reflows`, as
  far as that room takes it, `fit::max_per_row` of its columns), so a count is laid out for at once,
  while `Hold` still waits for a change of size: a window that changes how many to a row it takes
  changes the count laid out in only with the room, once it has held, and in the new room. A level
  starts at its zoom's count (`Nav::new`), so a relayout's level is bounded in it before its first draw. A count resets the
  settings that say how wide a page is (`Setting::sets_width`, the documents' `width`) to their
  default (`Nav::width_gives_way`); a key of one sets the count back to `auto` and returns
  `Step::Again`, which the pager presses again after `refresh_bounds`, so it steps from the bounds at
  `auto`. A layout that fails puts the count back to its `laid_at` room's with the values
  (`Pager::put_back`), and the failed pair, its room's count included, is refused as any is.
- **The page** (`wordcraft.rs`'s `page_in`), in points such that a character of the code face
  (`CODE_WIDTH`, `CODE_SIZE` × `CODE_ADVANCE` = 5.64 pt) is `font`/100 of a cell of the room when the
  page is shown as tall as it: a cell of the room is 5.64 × 100/`font` pt wide and that × the cell's
  shape tall (`cell_points`, the shape taken within reason, since a room is input), so at 200% the page
  is half as large in points and its text twice as large on screen. The page is always as tall as the
  room; at `window` as wide as it, or half of it less the gap where each half holds `BOOK_LEAST` (60)
  characters of text between its margins, as an open book, or with a count its share of the room's
  columns less the gaps (`page_cols`, checked, a cell at least), the book rule off (`window_cols`); at a
  number, the characters between the margins, the page growing by them, never wider than the room's
  share. The side margins are the `margin` setting's (`MARGIN`, 0 to 20, `MARGIN_DEFAULT` 3), in
  characters of the code face, so `margin` × `CODE_WIDTH` points at any font size (`side`); above and
  below, `TOP_MARGIN` (1) row at the text's size whatever the font size (`top_margin`). The bounds are
  worked out in order, each from the values as asked
  ([ADR-0024](adr/0024-pages-to-a-row-of-markdown-and-html-are-columns-and-the-last-key-wins.md)): the
  page's cells; `font`'s most, the largest of its steps at which that page holds `LEAST_COLUMNS` ×
  `LEAST_LINES` (20 × 10) of text within margins of a column, whatever the margin asked (`fits`,
  `font_most`), the least where none does; `margin`'s most, the largest at which a line holds 20
  characters at the font laid out at (`margin_most`), so the margin yields to the font. A value past
  its most is laid out at it, the value kept, and the status line says so even for a default
  (`pager::laid_at`: `font 100% (80%)` in a narrow column); where even no margin leaves 20 characters,
  the margin is `min(asked, 1)` and the page grows past the room (and is shown smaller). Sides are
  capped at `MAX_PAGE` (14,400 pt).
  `Source::bounds` answers each of them: for `width`, `named_at` the columns of the `window` page's text
  (`columns_of`) and `most` those of a page as wide as the room's share; for `font` and `margin`, `most`
  as above (a column, for a margin where none holds 20 characters).
- **Shown at its size.** `Source::largest_scale` is a default method answering `None`; the documents'
  backend answers, for Markdown and HTML, with the scale at which a page is as tall as its room
  (`rows × cell_h / page_h`). `fit::layout` takes it in `Size::largest` as it takes the 1.0 a natural
  page is held to: at `Fit::Page`, never past it, alone or beside another; at `Fit::Width` and at N to
  a row the page is any page. So a page is never shown larger than its font size, but may be shown
  smaller: two to a row at 80% of their size alone (`TWO_UP_SHARE`) still happens for a width between
  about half and five eighths of the window, a known limitation the ADR records. A document's pages
  aren't natural, so text cells oversample them as a PDF's (`Output::rendering`). `Joined` and `Later`
  pass the scale through for a list's tiles.

## Lists and levels

Several files, or a container's files and the containers in it, are a list: each an `Item`
(`mod.rs`), a file's pages or a container. A `source::Container` lists what it holds and opens
each file as an `Input`, which goes to the first backend that opens it, as a file named on the
command line does; the viewer knows a container only through the trait. A directory is one
(`source::Directory`, in `source/dir.rs`: in name order, subdirectories' files instead with
`-r`).

The list's tiles are one source, `source::Joined`: one page from each item, a file's first, or a
folder (`source::Folder`, drawn in code, with the badge its container gives it: `Container::badge`,
a parcel for an archive, none for a directory) for a container, each part with a name for its
errors. The pager shows one level at a time (`pager.rs`'s `Level`): a list, or a file's pages opened
from it, each with its own source and pool; going back drops the level and its images in the
terminal, and the list's are still there.

A container is listed when it is gone into, by the `Opener` `mod.rs` hands the pager, without
stopping the pager: first `Container::ready`, for a container that has to read itself before it
lists (a compressed tar, read through on a thread of its own), then its files opened a few at a time
(`Opening`), in the order the container says is quickest (`Listing::quickest`: a tar's, as its files
come in it), the status line saying how far each has got, and Esc stopping it. Stopping, or leaving
the level, tells the container (`Container::release`), which lets go of what it read its files with.
Given on the command line, a container is listed the same way before anything shows, the progress
on stderr.

A part's renderer or pool is made when one of its pages is first wanted. Printing keeps only the last
part's renderer, as pages print in order. The pager's pool hands each request to its part's pool, and
only for the most urgent parts, as many at once as the pool has threads, so a screen of 48 tiles from
48 files renders two files at a time, not 48 at once. It keeps the pools of the four parts wanted
most recently, then lets them go, and an image's decoded pixels with them. So a source must hold
little while idle: an image holds its header, not its pixels.

`/` filters a list (`pager.rs`'s `Filter`): the tiles `Nav` moves among become those whose names
(`Item::name`, in lower case) hold what is typed, in their order, so only they are on screen,
rendered and sent ahead, and the parts of the others are let go of as an off-screen tile's are.
The tiles keep the size the whole list gives them, so the matches are placements of what is in
the terminal ([ADR-0010](adr/0010-a-filtered-list-keeps-the-whole-lists-tile-size.md)).
A list shows once all its files are found, whether or not they are opened yet, so no tile arrives
under a filter later. For the filter, `keys.rs` decodes UTF-8: a character past ASCII is one key, one cut between reads waits
for the rest as an escape sequence does, and bytes that aren't UTF-8, or a control character,
are dropped.

### Relayout

A relayout setting ([`Effect::Relayout`](#backends)) applies only to a document on screen: a level
that shows one file's pages, opened alone or gone into from a list. Its keys don't set the shared
`changed` flag, which is for redraw settings and deletes every page sent; in a list they change nothing
and say to open a document, and with `--pages` (`Pages::picked`) that pages are picked as laid out.
Instead, on each pass of its loop, after the keys and after a file gone into shows, the pager compares
what the level's source is laid out at and for (`Source::laid_at`: the values and the room) with the
values wanted and the room it shows in (`Pager::room`), for the relayout settings the source has
(`relayout::wanted`: the values, and the room through `Room::same`), and when either differs asks the
source for a job (`Source::relayout`, a `Job`: a `Send` closure owning the bytes the source keeps, the
values and the room, which gives a new source laid out at them). So `]` then `u`, read together,
reflows nothing in the list, and the next document gone into is laid out at the new value, and for the
pager's room rather than the preview's it was opened for, as it shows. A layout is asked for, and its
result taken in, by its values and its room together.

The pager's room follows the window (`Pager::watch_room`, each pass of the loop): the room the window's
pages area makes now, from the same `area` the draw lays its grid out in (so its columns and rows, or
its cell where the tty reports pixels; a cell asked or told once holds for the run), becomes the room a
document is laid out for once it has differed from that and held still for `relayout::HOLD` (500 ms),
which `relayout::Hold` keeps, with no IO: a window still being dragged starts the wait again, one
dragged back to the document's room ends it, and the loop wakes when the hold is due (`Hold::due`).
Until then the old pages are fitted to the new window as any page is. So a terminal that reports no
pixels, whose first layout took the cell assumed, is laid out once more as the pager starts, where the
cell it answers makes another room. The status line says what is being laid out that isn't the default
(`width 72…`; `relayout::describe` drops the defaults), or `laying out…` when nothing is (`LAYING_OUT`,
a change of size at the defaults).

The job runs on `Openers`' threads (`Openers::run`, first in their queue, since the reader is waiting
on it), a panic in it costing the relayout alone, and the pager polls its `Handle` on each pass
(`Pager::relayout`; `Handle::try_take` never blocks), waking every `POLL` while one runs, so the new
pages show without a key and the keys work meanwhile. `view/relayout.rs` keeps the bookkeeping, with no
IO: which values and room want a layout (`relayout::wanted`, the relayout settings' alone, so a redraw
setting's change tells no two apart), the values and the room the job under way is for (`Relayout`),
and whether its result, when it ends, is for the values and the room wanted now (`Ended::Taken`) or
stale, dropped whether it succeeded or failed. At most one job is under way: a change meanwhile waits in the shared values and starts when it
ends, and the status line says the value wanted with `…` after it (`Nav::relaying`) until then. Leaving
the level (`enter`, `back`) forgets the job, which runs to its end for nobody. A new source makes a new
`Level` in place of the one on screen (`Pager::replace`): a new id and pool, the same zoom, `nested`
and help screen, its parents untouched. It opens where the reader was: the old source says which
paragraph is at the top of the screen (`Source::anchor`, from the first page on screen as the reader
has it now, a jump read in the same pass counted, and how far down it is scrolled, as a share of the
rows the last draw gave that page, `place`; an `Anchor` the viewer only hands back), the new says
where that paragraph starts in its layout (`Source::find`: the page, and how far down), and the level
goes to that page, scrolled there once the draw knows the page's rows (`Nav::scroll_to`), which only a
page as wide as the window has to scroll; a source that doesn't say leaves the reader's page, or the
last if there are fewer. The documents' backend keeps each page's body paragraphs for it, by their
order in the document, with their top and height on the page (`paragraphs_of`, from the layout's
`Placed::Lines`), for Markdown and HTML alone; a paragraph split across pages is found on the one it
starts on, and below a page's last paragraph the anchor is that one. The old level
goes as one gone back from does (`Pager::let_go`, which `back` shares): a copy still rendering on its
pool stops and says so (`clip::Copier::left`), its pages sent are deleted by its id, its renders
wanted dropped. A failure keeps the old level, says why through `printable`, once, puts the shared value
back to the document's `laid_at` (`Pager::put_back`), so the next step starts from what is on screen,
and remembers the pair that failed (`Relayout::fail`): it isn't asked for again (`Relayout::refused`)
until the values or the room change, or `r` (`Relayout::retry`), and leaving the level forgets it, so a
room a document can't be laid out for doesn't start a job on every pass.
The status line's right side shows the value the document is laid out at when it isn't the default
(`Nav::laid`, from `laid_at` when the level is made), padded to the widest it is said, dropped first
in a narrow window
([ADR-0022](adr/0022-markdown-and-html-reflow-to-a-width-in-characters-as-a-new-level.md)).

### Files opened when their tiles are wanted

The files "opened" above are only read as far as their first bytes for a backend that says `Backend::deferred`: the file, among
others or in a container, becomes an `Item::Later`, closed, with `source::Later` as its part
([ADR-0019](adr/0019-a-list-opens-its-files-when-their-tiles-are-wanted.md)). `Input::later` says how to
come back to it, by its path or its `Again`; a pipe can't be come back to, nor anything named in `/dev`
(redirected, `/dev/stdin` is a regular file, but on macOS opening it again goes on from where it was
read to), so those open as they are reached. A
file named alone never defers, as it is shown at once. So what the list knows from the start is its
tiles' number and names, each file's backend (for `takes_all`), and the settings each has.

`Later` reports one page, as a tile. It opens on `source::Openers`, four threads for the whole run,
each with a stack of 8 MiB, what the main thread has had to open files with, through
`Backend::open_at` for the preview's room ([The room a document is laid out for](#the-room-a-document-is-laid-out-for)). A `Later`'s pool asks for it
when given requests, and forgets it when given none or dropped, if no thread has begun it; one begun
goes on, and what it opened to is kept for when the reader comes back. The pool for the opened source
is made once it is known. So what renders stays what `Joined` already bounds: `Pools` gives requests
only to the most urgent parts, which limits how many open too.

Until it is open `Later::size` is a stand-in, US Letter, and the pager's layouts come from `source.size`,
so the first request made for a tile is at a scale from nothing:

1. The pager's `draw` lays out the tile and requests it, which only starts the file opening.
2. On finishing, the pool answers each request it holds with `Frame::asked_again`, no page and no error,
   and `Pager::receive` drops those `Wanted`s and asks for a draw. That is a redraw whether the tile
   is on screen or not, since it has to be asked for again either way.
3. `draw` lays the tile out again. `Later::size` and `Later::natural` only look: the result moves from
   the thread to `Later` when the pool answers in `try_recv`, or on `Later::outcome`, neither of them
   during a layout. So no thread finishing can change a size between `draw` reading it and listing what
   it worked out from it, and whatever is listed before the size is known is answered as above. A
   `Wanted` kept from before (one `Pools` hadn't
   passed on) has its layout compared each `draw`, and if that has changed it goes under a new tag.
4. The pool passes the new request to the opened source's pool. A file that failed answers it with
   `Frame::failed`, as a render does, so a failed tile takes the path any failed page takes: `fail`,
   the note when its tile is chosen, and a `!` before its label in `place`.

Printing can wait, so `print_pages` calls `Source::settle` before its layout, which opens the file on
the spot, and a `Later`'s renderer is the opened source's. The pager never calls it. Going into a tile
(`Pager::go_on_into`) and copying one (`Pager::copy`) ask `Later::outcome`, which also moves the file to
the front of the queue. Going in, that is asked again each `POLL`, keys meaning what they do while a
container is read; copying says it is still opening, and copies once it is, after a `draw`, so that what
is listed is from the true size.

## Getting pages to the terminal

A rendered page is premultiplied RGBA. The protocol offers three ways to send it:

| Way | Escape code | Used |
|---|---|---|
| PNG in the stream | `a=t,f=100,t=d`: base64 in 4096-byte chunks | Over SSH, when printing, and until the terminal takes a temporary file |
| Raw pixels in a temporary file | `a=t,f=32,t=t`: only the file's path, base64 | The pager, on a terminal on this machine that takes files |
| Shared memory | `t=s` | Never: it needs `mmap`, and the crate forbids `unsafe` |

Temporary files are much faster for the terminal. kitty takes a full-window page in about 10 ms
from a file, against 52 ms as a PNG in the stream (64–66 ms through tmux). Nine pages sent back to
back through tmux took 495 ms as PNGs and 55–90 ms as files. A PNG also has to be made first: on
the same laptop, 35 ms for a text page and 110 ms for a full-screen page of pictures at the
smaller compression. Without that, everything `clv` does between a key and the escape codes
takes under a millisecond; the render takes about 50 ms for a text page.

`--compression auto` (the default) picks the way (`Compression::choose` in `mod.rs`):

| Where the terminal is | PNG effort | Temporary files |
|---|---|---|
| This machine | fast (about twice the bytes for a text page and a third more for pictures, much less time) | tried by the pager |
| Over SSH (`SSH_CONNECTION`, `SSH_CLIENT`, `SSH_TTY`; inside tmux, `tmux show-environment SSH_CONNECTION`, which tmux updates at each attach) | small | never |

`--compression fast` or `small` sends PNGs, without trying files.

## Temporary files (`files.rs`)

- **The probe.** At the start, the pager sends a one-pixel image through a file. A terminal on
  this machine reads it and deletes it, as the protocol has it delete temporary files once read.
  Until it does, pages go as PNGs, and for good if it hasn't within 2 s.
- **Where.** `TMPDIR`, else `/dev/shm`, else `/tmp`, and in it a directory of the user's own,
  `tty-graphics-protocol-clv-<uid>`. The terminal deletes only files with
  `tty-graphics-protocol` in their path. The directory must be the user's, mode 0700 and not a
  symlink. The directory around it must be the user's or root's, and sticky if others can write
  to it, so no one else can move it away. Otherwise there are no files.
- **Each file** is `<pid>-<n>`, created new with mode 0600 (never an existing file, nor through a
  symlink), and written as straight RGBA in 256 KiB chunks.
- **A file the terminal leaves** is removed after 5 s, and pages go as PNGs from then on: the
  terminal doesn't take files after all, or the escape code never reached it. At most 8 files wait
  at once; pages go as PNGs until the terminal catches up.
- **Cleaning up.** The pager removes its files when it ends. A killed pager leaves them, and a
  later run removes them: files named `<pid>-<n>`, older than 60 s, whose process is gone
  (`kill(pid, 0)` says `ESRCH`), looking through at most 10,000 entries. Only the private
  directory is ever listed, never `TMPDIR` itself: on one macOS account that held 326,191 entries
  and took 26 s to list cold, with the pager on a black screen meanwhile.
- **A thread of their own** (`Files`). Every file operation runs on it, and the pager waits at
  most 1 s for an answer. After one that doesn't come (a hung network mount), files are over for
  the run: pages go as PNGs, and the pages whose files the terminal hadn't taken yet are sent
  again. The keys keep working throughout. At the end, once the terminal is put back, the pager
  waits up to 1 s more for the thread to remove its files, stalled or not: a loaded machine
  makes a thread slow far more often than a mount hangs it.

## The size of a cell

Pages are rendered for the pixels they cover, so `clv` needs the cell size in pixels. The tty
gives it with the window's size (`TIOCGWINSZ`), and SSH carries it. `docker exec` and
`kubectl exec` carry only rows and columns, so the pixels read 0. Taking 10 × 20 px cells then
rendered a page for a kitty with 18 × 46 px cells at under half its resolution, upscaled, and
22% too narrow. So, with no pixels, `clv` asks the terminal once at the start (`ask_cell` in
`mod.rs`): `ESC [ 16 t`, which xterm's control sequences define, and kitty answers with
`ESC [ 6 ; <height> ; <width> t` in about 3 ms. The query is followed by the device attributes
query, which every terminal answers, so one that doesn't know the first ends the wait at once;
then the 10 × 20 assumed stands. An answer arriving late reaches the pager as an unknown CSI
sequence, which is no key, even cut in two (see the terminal's answers below). Inside tmux,
`#{client_cell_width}` and `#{client_cell_height}` give the cell instead. The answer isn't asked
again, so changing the font size in a session that reports no pixels shows only on the next
start. The room a document is laid out for as it opens, before the question, takes the cell
assumed where the tty reports no pixels, and the pager lays it out once more for the cell answered,
through a change of size ([Relayout](#relayout)).

## Directly, or inside tmux

Directly, the pager transmits each page as an image of its own and shows it with a placement
(`a=p`), the cursor left where it is (`C=1`). Each place on screen has a placement id (`p=1` for
the first, `p=2` for the next and so on, row by row), so a page given twice shows twice. A page that
leaves the screen loses its placement (`a=d,d=i`) but keeps its data, so showing it again is only
a placement.

Inside tmux, tmux keeps its own copy of the screen and knows nothing of images:

- **Passthrough.** Every escape code goes inside `ESC P tmux; … ESC \`, with each ESC doubled.
  tmux passes it on only when the pane's `allow-passthrough` is `on` or `all`. `clv` only asks
  (`show-options -Ap`): the option lets any program in the pane write to the terminal past tmux,
  so it is the user's. When it is off, `clv` stops with the command that turns it on, naming the
  pane: run from the shell without `-t`, tmux sets it on the session's active pane, which may be
  another. Output to a file only gets a warning, since it may be shown where it is on.
- **Unicode placeholders.** A page is a virtual placement (`U=1`) plus rows of U+10EEEE cells that
  tmux stores as text, so pages scroll with the pane and survive window switches. The image id is
  in the cells' foreground colour: 24-bit when tmux reports `RGB` for the terminal, the 256-colour
  palette otherwise, with the id's high byte in a third diacritic. Only the first cell of each row
  carries the row and column diacritics, as the protocol allows; full marks made a screenful big
  enough for tmux to throw output away. Placeholders number at most 297 rows and columns, so
  pages inside tmux are never bigger.
- **The terminal.** `TERM` names tmux, so `clv` asks tmux for its client's terminal
  (`#{client_termname}`) and cell size (`#{client_cell_width}`, `#{client_cell_height}`).

## Text cells (`cells.rs`)

`Output::Text` is the third way a page reaches the screen, beside `Direct` and `Tmux`: `--protocol
blocks` or `ascii` ask for it, and `--protocol auto` falls back to blocks when `check_terminal` or
`tmux_terminal` find the terminal doesn't speak the protocol
([ADR-0011](adr/0011-text-cells-where-the-terminal-has-no-graphics-protocol.md)). Nothing goes out
but SGR colours and characters `cells.rs` writes, so nothing a document shapes reaches the terminal.

- **What a render is asked for.** `Output::fitting` gives `fit.rs` a cell of the terminal's cell's
  shape (clamped to between 1:1 and 1:4) with one pixel for each sample (`cells::sampling_cell`):
  1 × 2 for a 10 × 20 cell. So the arithmetic in `fit.rs` holds as it is, a tile is still square, and
  a picture whose natural size fits gets a column for each pixel across. A page with no natural size
  then renders four times over in each direction (`cells::OVERSAMPLE`, `Output::rendering`), within
  `MAX_PIXELS` and `MAX_AREA`, since a backend's picture is point-sampled at so small a scale.
- **From pixels to cells** (`cells::lines`). Each sample is the box of pixels it covers, averaged,
  which is any frame into any cells. A box under half opaque is no colour, as the terminal's
  background is unknown. A half-block cell writes `▀` for its two colours, a space when they are the
  same, and `▄` when only its bottom has one. A cell of ink picks a character by coverage: over a
  dark background, its brightest channel, in the colour at that brightness; over a light one, one
  less its darkest channel, in what is left of the colour. Mixed by how much of the cell the
  character covers, each comes back to about the colour it started from. Colours are written only
  when they change, and every line ends with them put back.
- **The background** matters only to ink: `ascii`, and `blocks` without colours (`Style::inks`).
  `ask_light` sends OSC 11 from xterm's control sequences, followed by the device attributes query
  as the cell size's question is, and goes by its answer's luminance. Half-blocks colour the whole
  cell, so they ask nothing.
- **Printing** (`print_row`) writes a line of text for each row of cells, as tmux's placeholder rows
  are written (`lines_of_cells`).
- **The pager.** `Sent` keeps a page's text (`Sent::text`) rather than an id the terminal knows, and
  `make_room` counts its bytes. `place` writes it where it would place an image, and `scroll` is a
  row into it. Nothing is kept in the terminal, so temporary files, `tmux::Watch`, `sweep.rs` and
  passthrough don't come into it: `Output::wrap` is `Wrap::None` even inside tmux, for the clipboard's
  question too, as the pane may not pass it.
- **Redraws write what changed.** A key usually changes little, and a full window of colours is
  hundreds of kilobytes. `place` builds each row in `text_rows`, and writes only those that differ
  from `Pager::on_glass`, which is cleared whenever something else may have written to them (the
  keys' help, `r`, a new window size). A row is erased in the same breath as it is written, not all
  rows first: synchronized output hides that where a terminal has it, and elsewhere a terminal that
  paints midway shows the screen tearing rather than going blank.
- **Limits.** `cells::MAX_CELLS` (4 million) bounds a page's text; a cell takes 39 bytes at most.

## The icon in `--help` (`icon.rs`)

The help has to work in any terminal and through any pipe, and the icon is a decoration, so it may
cost the help nothing:

- **Nothing is asked of the terminal.** Stdout must be a terminal, and `detect::env_says_kitty`
  must say so from the environment; inside tmux, `#{client_termname}` and `allow-passthrough` say.
  The query pages get would mean waiting up to `QUERY_TIMEOUT` on a terminal that never answers,
  and showing the query on one that prints what it doesn't know, for every `--help` there. So
  WezTerm, known only by its answer, gets no icon.
- **The lines go first.** They are written, so any scrolling is done; the cursor goes back up, the
  icon is drawn, and the cursor comes back for the rest. Directly it is shown with `C=1`; inside
  tmux it is a virtual placement and placeholder cells, written over the blanks to the right of the
  lines, which tmux keeps along with them.
- **Its size.** Its right edge is at `HELP_WIDTH`, where the text ends, or at the window's edge. It
  covers 5 rows, the help's lines down to its usage, or 4 or 3 when that doesn't leave 2 columns
  free beside the longest of them; below that, none. The PNG is square and the columns are rounded
  from the cell's shape, so it is at most half a column off square, at the terminal's stretching.
- **One image inside tmux.** `sweep.rs` deletes what it recorded once no pane holds it, but that
  reads every recorded pane, too much for `--help`. The icon has a fixed id instead (`ID_INDEX`):
  sending it again replaces it, so the terminal keeps one, recorded nowhere. Outside tmux it has no
  id, as a printed page has none, and goes when its lines do.

## Zoom and the grid (`fit.rs`, `pager.rs`)

The zoom (`fit::Zoom`) is a number of pages to a row, or the width. At N to a row, the page at
the start of the reader's row is fitted into the window's width shared N ways, gaps of 2 columns
between, and the cells it covers are the place every page on screen is fitted into, centred. As
many rows of those places as fit go one under another, a row between them. So pages never shrink
to make room for a row, and in a document of one page size every page renders at one scale, which
keeps what was sent usable as the reader moves on a row at a time. The same page in a place of
another size is another render (`View` has the place's size).

`Zoom::Auto` is two to a row when there are two pages and `fit::two_up_fits` says each keeps 80%
of its size alone, otherwise one, decided at every draw until a key zooms. `--per-row` sets it
(`--zoom`, its old name, through `OptionSpec::alias`, which `Parsed` records to warn of), and the
pager's `-` and `+`, and `,` and `.` (`Nav::per_row_key`, which takes a count), step it from what the
last draw showed. Zooming out stops at
`MAX_PER_ROW` (8), or before a page would be under `MIN_PAGE_COLS` (8) columns, however few pages
there are: N to a row is a size, so one page at four to a row is a quarter of the width, and a row
with fewer pages than that, when it is the only row, is centred. A screen holds at most
`MAX_ON_SCREEN` (48) pages, which bounds the rows of a strip of short pages.

A page whose size is natural (`Source::natural`: an image's, in pixels) is fitted at a scale of 1
at most, so a small picture keeps its size; `Fit::Width` still enlarges it. Its place in the row
is computed without that bound, since the place is shared by the pages after it. A page laid out
for a room (`Source::largest_scale`, a Markdown or HTML page) is fitted at the scale that makes it
as tall as that room at most, the same way ([The room a document is laid out
for](#the-room-a-document-is-laid-out-for)).

A screen starts at a multiple of the pages to a row (so two to a row pair pages the way a book's
spreads do), or higher up near the end, so the last row stays at the bottom. Either way the
reader's page, which is where zooming and typed pages anchor, is on screen; a property test
checks it over any keys and draws.

Printing (`print_pages`) uses the same `fit::grid`: the pages to a row come from the first page
(so `auto` decides once and every row has as many), and each row is fitted as the pager fits the
reader's row. A page alone in its row is shown with `a=T`, which moves the cursor like text, so a
page taller than the window (at the width) scrolls by. Pages side by side go into a band: as many
newlines as the row is tall, scrolling the screen if need be, then the cursor back up to the
band's top, and each page shown at its column with the cursor left where it is (`C=1`). The band
is never taller than the window, so going back up always reaches it. Inside tmux, each line of a
row is the pages' placeholder cells side by side.

## What is kept in the terminal (`pager.rs`)

The pager works ahead. Two render threads render the pages on screen first, then the next two
screens and the one before. Each page is sent as soon as it is ready, so turning to it is only a
placement. The pages on screen stay up until the next ones are ready, so the screen never goes
blank in between.

The terminal keeps at most 12 pages, and 192 MiB of pixels (`MAX_SENT`, `BUDGET`). When a screen
shows more than 3 pages, the count goes up to the screen and the screens worked on around it:
with 12, the pages of the screens ahead pushed each other out as they arrived, and turning to
them rendered them again. Rendering ahead stops at the first page that wouldn't fit, and the
least recently shown pages not on screen are deleted (`a=d,d=I`) to make room. The byte budget matters. kitty keeps 320 MB of images per
screen (the protocol's example quota), and past it, it deletes nearly every image without a
placement at once, not just the oldest. Measured in a hidden kitty with raw images:

| Images sent | What kitty kept |
|---|---|
| 13 × 24.6 MB (319 MB) | all 13 |
| 14 × 24.6 MB (344 MB) | 1 (3 when two had placements: those survive) |
| 30 × 12.1 MB (364 MB) | 3 |

At 12 pages and no byte budget, fit-width pages on a 5K window (about 64 MB each) went past the
quota: kitty dropped 8 of the 12 pages the pager kept, and turning back showed them black.

When kitty drops a page anyway (another program's images, a terminal with a smaller quota),
placements tell. They ask for errors (`q=1`; every other code says `q=2`, no answers), and kitty
answers a placement of an image it doesn't have with `ESC _ G i=<id>,p=<p>;ENOENT:… ESC \`.
`keys.rs` turns that into `Key::Missing`, and the pager forgets the page, renders and sends it
again, and halves its budget, at most 8 times (`MAX_RESENDS`; `r` still sends everything again).
kitty takes a temporary file before it reads the next escape code, so a placement right after
one never gets this answer by mistake.

What the pager deletes, kitty frees, and so are pictures printed at the cursor, PNG or raw, once
they leave the scrollback. Inside tmux, a printed page has to outlive `clv`, its placeholder
cells being tmux's text, and nothing tells kitty when tmux lets go of them, so kitty keeps it
until its quota pushes it out: about 400 MB of GPU memory a window, through `clear-history` and
the tmux server's exit alike. `sweep.rs` deletes them instead: each page printed inside tmux is
recorded in the server's `@clv-printed` option (`%pane:id`, the newest 2048), and every run
inside tmux, before it draws, reads each recorded pane (`capture-pane -e` of its history and
screen, and with `-a` the screen saved behind a full-screen program) and deletes the images
whose cells are gone, or whose pane is. A pane it can't read, or one holding more than 32 MiB,
keeps its records. The deletes go through the running pane, to the terminals showing it then.
[ADR-0001](adr/0001-delete-pages-printed-inside-tmux-once-no-pane-holds-them.md) has
the measurements and the alternatives.

The clipboard's answers are read the same way: whether the terminal speaks its protocol (a CSI
sequence), and how a write went (an OSC sequence, `ESC ] 5522 ; … ESC \`, or ended by BEL), for
the copy under way ([Copying to the clipboard](#copying-to-the-clipboard-cliprs)). Any other
answer is dropped, OSC ones included. An answer must never be read as keys: its `ESC` would be the Esc
key, which quits, and its letters keys. One cut off between reads waits for the rest, up to 512
bytes and 1 s, and is dropped whole if the rest doesn't come. So does any escape sequence cut off
past its `ESC`, a late cell size or a key over SSH alike. A rest that comes later than that is
read as keys: no wait could tell it from typing, and terminals send each reply in one write. Only
an `ESC` alone, with nothing after it for 50 ms, is the Esc key. At the end, the pager waits up to 100 ms after its last placements,
so no late answer lands on the shell's command line.

## What tmux loses, and how the pager notices

tmux delivers what goes through passthrough once, to the terminals showing the pane at that
moment, and never again: it redraws its own text, not images. The pager asks tmux about the pane
150 ms after it writes, and every second anyway, in one command:

```sh
tmux display-message -p -t "$TMUX_PANE" '#{window_id}|#{window_zoomed_flag}|#{pane_active}' \; \
     list-clients -F '#{client_name}|#{client_pid}|#{client_discarded}|#{window_id}'
```

`tmux::Watch` keeps, for every client seen showing the pane, how many images had been sent when it
last did. The pager acts on what changed:

| What tmux says | What happened | What the pager does |
|---|---|---|
| No client shows the pane (another window, or zoomed on another pane) | With `allow-passthrough on`, tmux throws passthrough away | Sends nothing until the pane shows; removes waiting files without giving files up |
| A client shows it again after images went out without it | It missed them | Sends everything again |
| A client it hasn't seen (new, or attached again: the terminal clears the alternate screen's images when tmux returns to it) | It has none of the images | Sends everything again |
| More than one client shows it | Each gets every escape code, and the first to read a file deletes it | No more files; pages go as PNGs |
| A client's `#{client_discarded}` rose | tmux threw output away because the terminal fell behind | Sends everything again, at most 8 times |

A client is its `client_name` and `client_pid`, so attaching again from the same tty is a new
client. Temporary files start only inside a tmux that reports exactly one client showing the
pane.

## Copying to the clipboard (`clip.rs`)

`c` copies the page the reader is on (`Nav::at`), or the tile chosen, as a picture. Where it goes
is [ADR-0009](adr/0009-copy-a-page-through-the-terminals-clipboard-protocol-else-the-systems.md):
the terminal's own clipboard through kitty's clipboard protocol (OSC 5522) when the terminal
speaks it, otherwise the system's (arboard) when the terminal is on this machine, otherwise none.
The first `c` asks with DECRQM for mode 5522 (wrapped for tmux); an answer of 0 or 4, or none in
a second, is no. The pager never waits for any of it:

- **The render.** A copy is a `Request` of its own (its own tag) on the level's pool, at the
  page's own pixels for a picture and twice its unit otherwise, within `MAX_PIXELS` and
  `MAX_AREA`. It goes first in every `set_queue` while it is wanted, as the pool's contract asks,
  and its result is taken out of `try_recv` before the pages'. Leaving the level while it renders
  stops it, and says so.
- **The PNG** is made on a thread of its own (the pager's PNG effort: fast on this machine, small
  over SSH); a PNG over 64 MiB, what the protocol has every terminal take, isn't sent.
- **The write**: a packet to start, a packet for each 4096 bytes of the PNG (the protocol's most,
  before base64), a packet to end, each its own escape code and, inside tmux, its own passthrough
  sequence. They go 128 KiB a round of the pager's loop, which reads keys between rounds without
  waiting, and stops while tmux hides the pane. The write carries an id (`clv-<n>`) that the
  terminal echoes: its `status=DONE` says "copied", an error says why not, and an answer to an
  older write or to someone else's is dropped. Without an answer for five seconds (inside tmux,
  the answer goes to the pane tmux shows), the status line says so; a late one is still said.
  When tmux reports output thrown away during a write, the copy is said to be lost.
- **The system's clipboard** has a thread of its own for the pager's run, which opens it on the
  first copy and keeps it: on X11, a picture is on the clipboard only while the program that put
  it there is there to hand it over. At the end, once the terminal is put back, the pager waits up
  to half a second for the thread to let it go.

Quitting while an answer is due (the question, or a write's end) waits up to a second for it, so
it doesn't land on the shell's command line.

## The terminal's state

The pager runs on the alternate screen, in raw mode, with the cursor hidden and line wrap off.
`Screen` restores all of it when it drops, on return, on an error through `?`, and while a panic
unwinds; nothing else is printed to stderr meanwhile (`QUIET`). The images are deleted before
that, and the temporary files removed after it, so a slow filesystem can't hold the terminal
in raw mode. File names, error messages and warnings are written with their control characters
replaced (`printable`), so a document can't send escape sequences through them.

When it will page, `clv` takes raw mode before it asks the terminal anything (whether it speaks
the protocol, its cell size), and keeps it until the pager ends, never back in canonical mode
between the two. On FreeBSD a read begun in canonical mode stays a line read after raw mode
returns: had the reader thread begun its next read in such a gap, no key would reach the pager
until Enter. Found by CI on FreeBSD, where two pager tests hung after `q`; the race shows there
only.

## What sixel would take

Sixel isn't supported yet, and is a possible later addition. It is DEC's older image format, the
one Windows Terminal speaks (as do xterm, foot, WezTerm and others), so it is the way to bring
`clv` to Windows and to more terminals. It would be a second output beside `kitty.rs` more than a
change to it: sixel keeps no images in the terminal by id, and has no temporary files and no
Unicode placeholders, so every page turn sends the page's pixels again, and pictures go through a
palette (commonly 256 colours). [Text cells](#text-cells-cellsrs) are the same shape in the pager: a
page kept here and written whenever it goes on screen. On Windows, `tty.rs` would also need the console's own API for raw
mode and the window size, through a crate, since the crate forbids `unsafe`.
