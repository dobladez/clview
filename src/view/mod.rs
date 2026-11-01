//! Pages drawn straight into the terminal through the kitty graphics protocol (kitty, Ghostty,
//! WezTerm and others), from a [`Source`]. Usage and behaviour: `docs/usage.md`.

mod clip;
mod detect;
mod encode;
mod files;
mod fit;
mod keys;
mod kitty;
mod pager;
mod sweep;
mod tmux;
mod tty;

use std::io::{IsTerminal, Write};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use encode::PngEffort;

use crate::CliError;
use crate::args::{OptionSpec, Parsed, page_list};
use crate::source::{Backend, Container, Directory, Folder, Frame, Input, Joined, Listing, Opened, Part, Ready, Request, Source};
use fit::{Area, Cell, Fit, Grid, Layout, Size, Zoom};
use kitty::{IdColor, Wrap};
use tty::{Tty, WindowSize};

/// The viewer's own options, whatever it shows. A backend's come from the backend.
const OPTIONS: &[OptionSpec] = &[
    OptionSpec { name: "--pages", short: None, value: Some("1,3-5"), help: "the pages to show, in this order (default: all); of a list, its tiles" },
    OptionSpec { name: "--recurse", short: Some("-r"), value: None, help: "a directory's subdirectories' files, not folders" },
    OptionSpec { name: "--zoom", short: None, value: Some("auto|width|N"), help: "auto: two side by side if they fit; width; or N pages to a row" },
    OptionSpec { name: "--pager", short: None, value: None, help: "always page, even through one page" },
    OptionSpec { name: "--no-pager", short: None, value: None, help: "never page: print the pages and exit" },
    OptionSpec {
        name: "--compression",
        short: None,
        value: Some("auto|fast|small"),
        help: "fast PNGs or small ones (auto: small over SSH, else fast)",
    },
    OptionSpec { name: "--protocol", short: None, value: Some("auto|kitty"), help: "kitty: send the images without asking the terminal first" },
];

/// What `--help` says before the options.
const ABOUT: &str = "\
clv shows the pages of files in your terminal. It uses the kitty graphics protocol.
  * Terminals: kitty, Ghostty and WezTerm. Also in tmux.
  * Connections: on your computer, or through SSH.
  * Systems: macOS, Linux and FreeBSD.

If a file has more than one page, clv shows it in a pager. If the window is wide, the pager shows
two pages side by side. If you give more than one file, a directory or an archive, clv shows a
list. The list shows each file as a tile, and each subdirectory as a folder.
  * - and +: show more or fewer pages in a row.
  * Enter: open the tile. Esc: go back.
  * /: filter the list by name.
  * c: copy the page or the tile to the clipboard.
  * ?: show all the keys.";

/// The window assumed when there is no terminal to ask (no controlling terminal, as under CI):
/// the classic 80 × 24, at [`fit::FALLBACK_CELL`].
const FALLBACK_SIZE: WindowSize = WindowSize { cols: 80, rows: 24, xpixel: 0, ypixel: 0 };

/// How long the terminal gets to answer a query (support, cell size). Terminals answer within
/// milliseconds, so this only matters for one that never does.
const QUERY_TIMEOUT: Duration = Duration::from_millis(500);

const NOT_A_TERMINAL: &str = "stdout is not a terminal (--protocol kitty sends the escape codes anyway)";
const UNSUPPORTED: &str = "this terminal doesn't show images through the kitty graphics protocol (kitty, Ghostty and WezTerm do)";

/// While the pager owns the screen, panic messages aren't printed over it (see [`run`]).
static QUIET: AtomicBool = AtomicBool::new(false);

/// Which graphics protocol: found out by asking, or taken on trust.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Protocol {
    Auto,
    Kitty,
}

impl Protocol {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "auto" => Ok(Self::Auto),
            "kitty" => Ok(Self::Kitty),
            other => Err(format!("--protocol {other:?}: use auto or kitty")),
        }
    }
}

/// How pages reach the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Output {
    /// Shown at the cursor.
    Direct,
    /// Through tmux's passthrough, and drawn by placeholder cells whose colour names the image.
    Tmux(IdColor),
}

impl Output {
    /// The most columns and rows a page may cover in a window `cols` wide: placeholders number
    /// at most [`kitty::MAX_CELLS`] of either.
    fn limits(self, cols: u16) -> (u16, u16) {
        match self {
            Self::Direct => (cols, u16::MAX),
            Self::Tmux(_) => (cols.min(kitty::MAX_CELLS), kitty::MAX_CELLS),
        }
    }

    fn wrap(self) -> Wrap {
        match self {
            Self::Direct => Wrap::None,
            Self::Tmux(_) => Wrap::Tmux,
        }
    }

    /// The image ids this output can draw with: any outside tmux, as many as placeholder colours
    /// can name inside it.
    fn ids(self) -> IdColor {
        match self {
            Self::Direct => IdColor::Rgb,
            Self::Tmux(colour) => colour,
        }
    }
}

/// Whether pages are paged through or printed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Paging {
    /// The pager when there is more than a page to show, on a terminal it can read keys from;
    /// otherwise printed, as `less -F` does when the text fits.
    Auto,
    /// `--pager`: always the pager, even for one page.
    Always,
    /// `--no-pager`: always printed.
    Never,
}

impl Paging {
    fn from_flags(pager: bool, no_pager: bool) -> Result<Self, String> {
        match (pager, no_pager) {
            (true, true) => Err("give --pager or --no-pager, not both".into()),
            (true, false) => Ok(Self::Always),
            (false, true) => Ok(Self::Never),
            (false, false) => Ok(Self::Auto),
        }
    }

    /// Whether `pages` pages are paged through, `interactive` being whether stdout is a terminal
    /// and keys can be read from one.
    fn pages(self, pages: usize, interactive: bool) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => interactive && pages > 1,
        }
    }
}

/// How hard pages are compressed on their way to the terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Compression {
    /// `small` over SSH. On this machine, `fast`, and the pager hands pages over through
    /// temporary files when the terminal takes them (`files.rs`).
    Auto,
    /// Least CPU: for a terminal on the same machine, where encoding is most of a page's cost.
    Fast,
    /// Fewest bytes: for a terminal across a network, where the bytes are.
    Small,
}

impl Compression {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "auto" => Ok(Self::Auto),
            "fast" => Ok(Self::Fast),
            "small" => Ok(Self::Small),
            other => Err(format!("--compression {other:?}: use auto, fast or small")),
        }
    }

    /// The PNG effort, and whether the pager may try temporary files, asking `over_ssh` only
    /// when it matters. A compression asked for by name means PNGs in the stream.
    fn choose(self, over_ssh: impl FnOnce() -> bool) -> (PngEffort, bool) {
        match self {
            Self::Fast => (PngEffort::Fast, false),
            Self::Small => (PngEffort::Small, false),
            Self::Auto if over_ssh() => (PngEffort::Small, false),
            Self::Auto => (PngEffort::Fast, true),
        }
    }
}

/// Whether the terminal is at the other end of an SSH connection, so the bytes `view` sends
/// cross a network. Inside tmux, the session knows how its client attached (`tmux_says`); the
/// pane's own environment may date from another attach.
fn over_ssh(env_ssh: bool, tmux_says: Option<bool>) -> bool {
    tmux_says.unwrap_or(env_ssh)
}

/// Whether this process's environment comes from an SSH login.
fn env_ssh() -> bool {
    ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"].iter().any(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()))
}

/// The terminal `view` draws on.
struct Terminal {
    output: Output,
    /// The cell size tmux or the terminal reports, for a tty that reports no pixel size.
    cell: Option<Cell>,
    /// How hard pages are compressed for it.
    effort: PngEffort,
    /// Whether the pager may hand pages over through temporary files: the terminal may be on
    /// this machine.
    files: bool,
    /// Whether the terminal runs on this machine, not over SSH: the system's clipboard is the
    /// reader's then.
    local: bool,
}

pub(super) fn run(args: &[String]) -> Result<(), CliError> {
    // A document can make the renderer panic. The renderer catches the panic and reports the page
    // as failed, but the default hook first prints the panic's message, which the document may
    // have shaped, raw to the terminal. This hook prints it made safe, and not over the pager.
    std::panic::set_hook(Box::new(|info| {
        if !QUIET.load(Ordering::Relaxed) {
            note(&format!("{}: {info}", crate::NAME));
        }
    }));
    view(args).map_err(|e| match e {
        CliError::Message(message) => CliError::Message(printable(&message)),
        other => other,
    })
}

/// Which clview this is, then what it is, in a line each: `--help`'s first two.
const IS: &str = concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION"), "\n", env!("CARGO_PKG_DESCRIPTION"));

/// Where to tell of a problem or an idea: `--help`'s last line.
const REPORT: &str = concat!("Report issues and ideas to: ", env!("CARGO_PKG_REPOSITORY"));

/// What `--help` says before the formats this build shows.
const SHOWN: &str = "The files it shows, known by how they start or, where that can't tell, by the names in brackets:";

/// The columns `--help` wraps the formats it lists at.
const HELP_WIDTH: usize = 100;

/// What `--help` prints: what clview is, how it is called, the viewer's options, then those of
/// each of `backends` that takes some, under what it shows, then the formats they show, under what
/// each shows and the libraries it shows them through, and last where clview comes from.
pub(crate) fn usage(backends: &[&dyn Backend]) -> String {
    let name = crate::NAME;
    let mut out = format!("{IS}\n\n{name} [options] <file>…\n{name} --help | --version\n\n{ABOUT}\n\n");
    let width = OPTIONS.iter().chain(backends.iter().flat_map(|b| b.options())).map(|o| o.shown().len()).max().unwrap_or(0);
    let list = |out: &mut String, options: &[OptionSpec]| {
        for option in options {
            out.push_str(&format!("  {:width$}  {}\n", option.shown(), option.help));
        }
    };
    list(&mut out, OPTIONS);
    for backend in backends.iter().filter(|b| !b.options().is_empty()) {
        out.push_str(&format!("\n{}:\n", capitalized(backend.shows())));
        list(&mut out, backend.options());
    }
    if backends.is_empty() {
        out.push_str("\nThis build has no backends, so it shows nothing.\n");
    } else {
        out.push_str(&format!("\n{SHOWN}\n"));
        for Listed { shows, credits, formats } in shown(backends) {
            let named = formats.iter().map(|format| match format.extensions {
                [] => format.name.to_string(),
                extensions => format!("{} (.{})", format.name, extensions.join(" .")),
            });
            wrapped(&mut out, &format!("{}{}:", capitalized(shows), by(&credits)), &named.collect::<Vec<_>>());
        }
    }
    out.push_str(&format!("\n{REPORT}\n"));
    out
}

/// What backends show, as `--help` lists it: the libraries they show it through, and in which
/// formats.
struct Listed {
    shows: &'static str,
    credits: Vec<&'static str>,
    formats: Vec<crate::source::Format>,
}

/// What `backends` show, in their order, each with the libraries and the formats it lists: those
/// of backends that show the same together, each library and each format once.
fn shown(backends: &[&dyn Backend]) -> Vec<Listed> {
    let mut shown: Vec<Listed> = Vec::new();
    for backend in backends {
        let at = match shown.iter().position(|listed| listed.shows == backend.shows()) {
            Some(at) => at,
            None => {
                shown.push(Listed { shows: backend.shows(), credits: Vec::new(), formats: Vec::new() });
                shown.len().saturating_sub(1)
            }
        };
        // Found in `shown`, or just pushed to it: there is one at `at`.
        let Some(Listed { credits, formats, .. }) = shown.get_mut(at) else { continue };
        for &credit in backend.credits() {
            if !credits.contains(&credit) {
                credits.push(credit);
            }
        }
        for format in backend.formats() {
            if !formats.iter().any(|listed| listed.name == format.name) {
                formats.push(format);
            }
        }
    }
    shown.retain(|listed| !listed.formats.is_empty());
    shown
}

/// `, by a, b and c`, for the libraries `credits` names; nothing for none.
fn by(credits: &[&str]) -> String {
    match credits {
        [] => String::new(),
        [one] => format!(", by {one}"),
        [rest @ .., last] => format!(", by {} and {last}", rest.join(", ")),
    }
}

/// `items` after `label`, a comma between each two, wrapped between them at [`HELP_WIDTH`]
/// columns: the lines after the first start under the first item.
fn wrapped(out: &mut String, label: &str, items: &[String]) {
    let indent = " ".repeat(label.chars().count().saturating_add(2));
    let mut line = format!("  {label}");
    for (at, item) in items.iter().enumerate() {
        let item = if at.saturating_add(1) < items.len() { format!("{item},") } else { item.clone() };
        if at > 0 && line.chars().count().saturating_add(1).saturating_add(item.chars().count()) > HELP_WIDTH {
            out.push_str(&line);
            out.push('\n');
            line.clone_from(&indent);
        }
        line.push(' ');
        line.push_str(&item);
    }
    out.push_str(&line);
    out.push('\n');
}

/// `text`, its first letter a capital.
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

fn view(args: &[String]) -> Result<(), CliError> {
    let backends = crate::source::BACKENDS;
    let known: Vec<OptionSpec> = OPTIONS.iter().chain(backends.iter().flat_map(|b| b.options())).copied().collect();
    let options = Parsed::parse(args, &known)?;
    let paths = file_args(&options)?;
    let zoom = options.value("--zoom").map_or(Ok(Zoom::Auto), Zoom::parse)?;
    let protocol = Protocol::parse(options.value("--protocol").unwrap_or("auto"))?;
    let compression = Compression::parse(options.value("--compression").unwrap_or("auto"))?;
    let given = options.value("--pages").map(|list| page_list(list).map_err(|e| format!("--pages: {e}"))).transpose()?;
    let paging = Paging::from_flags(options.has("--pager"), options.has("--no-pager"))?;
    let opener = Opener { backends, options: &options };
    let Given { shown, left_out, warnings } = opener.open(paths, options.has("--recurse"))?;
    warn(&warnings, &left_out);
    // Said first, what was left out may be why there are fewer than asked for.
    let shown = shown.pick(given.as_deref())?;
    let tty = Tty::open().ok();
    let paging = paging.pages(shown.pages.pages.len(), std::io::stdout().is_terminal() && tty.is_some());
    let inside_tmux = tmux::inside();
    // Paging, the terminal stays in raw mode from the first question on (outside tmux, which is
    // asked instead), never back in canonical mode between a question and the pager. On FreeBSD a
    // read begun in canonical mode stays a line read after raw mode returns: the reader thread,
    // its next read begun in between, would hold every key until Enter.
    let _raw = match (&tty, paging && !inside_tmux) {
        (Some(tty), true) => Some(tty.raw()?),
        _ => None,
    };
    let terminal = if inside_tmux {
        tmux_terminal(protocol)?
    } else {
        check_terminal(protocol, tty.as_ref())?;
        let cell = tty.as_ref().and_then(ask_cell);
        Terminal { output: Output::Direct, cell, effort: PngEffort::Fast, files: false, local: true }
    };
    let remote = over_ssh(env_ssh(), inside_tmux.then(tmux::attached_over_ssh).flatten());
    let (effort, files) = compression.choose(|| remote);
    let terminal = Terminal { effort, files, local: !remote, ..terminal };
    // Inside tmux, what this run prints stays in the terminal after it, for tmux's placeholder
    // cells (`sweep.rs`): so it is recorded, and first the pictures no pane holds any more go.
    // Only when the output is this pane's, not a file `cat` shows later.
    let tidies = inside_tmux && std::io::stdout().is_terminal();
    if tidies {
        let mut out = Vec::new();
        sweep::forget(&mut out);
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(&out).and_then(|()| stdout.flush()).map_err(CliError::Stdout)?;
    }
    let first_id = fresh_index();
    let said = left_out_note(&left_out);
    if paging {
        let tty = tty.as_ref().ok_or("--pager needs a terminal to read keys from")?;
        let later = pager::run(tty, shown, said, &terminal, zoom, first_id, &opener)?;
        // Said once the screen is back: what the directories gone into left out.
        warn(&later, &[]);
        return Ok(());
    }
    let size = tty.as_ref().and_then(Tty::size).unwrap_or(FALLBACK_SIZE);
    let names: Option<Vec<String>> = shown.is_list().then(|| shown.items.iter().map(|item| item.name().to_owned()).collect());
    let mut sent = Vec::new();
    let printed = print_pages(&*shown.pages.source, &shown.pages.pages, names.as_deref(), zoom, size, &terminal, first_id, &mut sent);
    if tidies {
        sweep::remember(&sent);
    }
    printed?;
    // Each was said before the pages; the exit status says it too, as for a page that failed.
    said.map_or(Ok(()), |said| Err(said.into()))
}

/// What is shown: pages of a source, and for a list, the items its pages are the tiles of. A
/// file's pages and a list's tiles are both pages, which `--pages` picks from.
pub(super) struct Shown {
    pub(super) pages: Pages,
    /// A list's files and directories, in the order of its tiles; none for a file.
    pub(super) items: Vec<Item>,
}

impl Shown {
    /// A list of `items`, every tile, its items' names following `name`.
    pub(super) fn list(items: Vec<Item>, name: String) -> Self {
        Self { pages: Pages::all(Rc::new(tiles(&items)), name), items }
    }

    /// Whether this is a list: it has items.
    pub(super) fn is_list(&self) -> bool {
        !self.items.is_empty()
    }

    /// The pages `given`, in their order, if any, of all there are; one that isn't there is the
    /// error.
    fn pick(self, given: Option<&[usize]>) -> Result<Self, String> {
        let Some(given) = given else { return Ok(self) };
        let count = self.pages.source.pages();
        let Some(missing) = given.iter().find(|page| **page >= count) else {
            return Ok(Self { pages: Pages { pages: given.to_vec(), ..self.pages }, ..self });
        };
        let (unit, whole) = if self.is_list() { ("tile", "the list") } else { ("page", self.pages.name.as_str()) };
        Err(format!("no {unit} {} ({whole} has {})", missing.saturating_add(1), counted(count, unit)))
    }
}

/// A file's pages to page through, in order, at least one.
#[derive(Clone)]
pub(super) struct Pages {
    pub(super) source: Rc<dyn Source>,
    /// What the status line calls the file.
    pub(super) name: String,
    pub(super) pages: Vec<usize>,
}

impl Pages {
    /// All of `source`'s pages, in order, called `name`.
    fn all(source: Rc<dyn Source>, name: String) -> Self {
        let pages = (0..source.pages()).collect();
        Self { source, name, pages }
    }
}

/// A tile of a list: a file, shown by its first page and opened to its pages, or a container (a
/// directory), shown as a folder and opened to a list of its own.
pub(super) enum Item {
    File(Pages),
    /// Its name ends with `/`.
    Container {
        name: String,
        container: Rc<dyn Container>,
    },
}

impl Item {
    /// As the list names it, from the list's directory: `beach.jpg`, `2024/` (`2024/beach.jpg`,
    /// recursing).
    pub(super) fn name(&self) -> &str {
        match self {
            Self::File(pages) => &pages.name,
            Self::Container { name, .. } => name,
        }
    }
}

/// The tiles of `items` as one source, a page each, in order: a file's first page, a folder for a
/// container.
fn tiles(items: &[Item]) -> Joined {
    let part = |item: &Item| match item {
        Item::File(p) => Part { name: p.name.clone(), source: p.source.clone(), pages: vec![p.pages.first().copied().unwrap_or(0)] },
        Item::Container { name, container } => Part { name: name.clone(), source: Rc::new(Folder(container.badge())), pages: vec![0] },
    };
    Joined::new(items.iter().map(part).collect())
}

/// A container's files being opened, a few at a time ([`Opener::step`]), so that the pager can say
/// how far it has got and the reader can stop it.
pub(super) struct Opening {
    container: Rc<dyn Container>,
    prefix: String,
    listing: Listing,
    /// The order its entries are opened in: the container's quickest, or as listed.
    order: Vec<usize>,
    /// How many of `order` are opened.
    next: usize,
    /// Each entry's file, by its place in the listing, once opened.
    opened: Vec<Option<Chosen>>,
}

/// A file as [`Opener::file`] opens it: with the backend that opened it, a tile; `None` for a file
/// no backend opens; or why it can't be shown.
type Chosen = Result<Option<(&'static dyn Backend, Item)>, String>;

impl Opening {
    /// How many of its entries are opened, and of how many.
    pub(super) fn done(&self) -> (usize, usize) {
        (self.next, self.order.len())
    }
}

/// The file arguments: the arguments that are neither an option nor an option's value.
fn file_args<'o, 'a>(options: &'o Parsed<'a>) -> Result<&'o [&'a str], String> {
    match options.operands.as_slice() {
        [] => Err("missing file".into()),
        files => Ok(files),
    }
}

/// Opens files for the viewer: with the first of `backends` that opens each, with `options`.
pub(super) struct Opener<'o, 'a> {
    backends: &'o [&'static dyn Backend],
    options: &'o Parsed<'a>,
}

/// What the files given show, and what of them couldn't be shown, with why.
struct Given {
    shown: Shown,
    left_out: Vec<(String, String)>,
    /// What else the reader should know first: a listing cut short.
    warnings: Vec<String>,
}

impl Opener<'_, '_> {
    /// What `paths` show: one file's pages, or a list. A file alone that can't be shown is the
    /// error. Several files, or a container (a directory, or a file that holds files, an
    /// archive), are a list: a tile for each file, by its first page, and for each container (a
    /// container alone, its files and the containers in it); recursing, a container's files and
    /// its containers' are tiles instead. Among several, a file that can't be shown is left out,
    /// and only nothing at all to show is an error. In a container, a file that nothing shows is
    /// passed over. An option that none of the files' backends takes is refused, since it would
    /// do nothing.
    fn open(&self, paths: &[&str], recurse: bool) -> Result<Given, String> {
        let is_dir = |path: &str| std::fs::metadata(path).is_ok_and(|m| m.is_dir());
        let mut found = Found::default();
        for path in paths {
            let name = file_name(path);
            let container: Rc<dyn Container> = if is_dir(path) {
                Rc::new(Directory::new(path))
            } else {
                match crate::source::choose(path, self.backends).and_then(|chosen| self.file(Some(chosen), &name)) {
                    Ok(Some((_, Item::Container { container, .. }))) => container,
                    Ok(Some((backend, Item::File(pages)))) if paths.len() == 1 => {
                        takes_all(&[backend], self.options, path)?;
                        return Ok(Given { shown: Shown { pages, items: Vec::new() }, left_out: Vec::new(), warnings: Vec::new() });
                    }
                    Err(why) if paths.len() == 1 => return Err(why),
                    opened => {
                        found.file(opened, name);
                        continue;
                    }
                }
            };
            match (paths.len(), recurse) {
                // Alone, its contents, named from it.
                (1, _) => self.list(&mut found, container, "", recurse),
                // Among others, recursing: its files, named from its own name.
                (_, true) => self.list(&mut found, container, &format!("{name}/"), true),
                (_, false) => found.items.push(Item::Container { name: format!("{name}/"), container }),
            }
        }
        let Found { items, used, left_out, warnings } = found;
        if items.is_empty() {
            if let ([(_, why)], []) = (left_out.as_slice(), warnings.as_slice()) {
                return Err(why.clone());
            }
            warn(&warnings, &left_out);
            return Err(match paths.iter().any(|p| is_dir(p)) {
                false => format!("none of the {} files can be shown", paths.len()),
                true => "nothing given can be shown".into(),
            });
        }
        // A container not gone into yet may hold files that take it.
        if !items.iter().any(|item| matches!(item, Item::Container { .. })) {
            takes_all(&used, self.options, "any of these files")?;
        }
        Ok(Given { shown: Shown::list(items, String::new()), left_out, warnings })
    }

    /// Into `found`, `container`'s files (its containers' too if `recurse`; its containers
    /// otherwise), each named `prefix` and its name in the container: read, listed and opened at
    /// once, saying how far it has got on stderr while that takes a while.
    fn list(&self, found: &mut Found, container: Rc<dyn Container>, prefix: &str, recurse: bool) {
        let mut progress = Progress::new();
        let name = file_name(container.path());
        while let Ready::Reading { read, of } = container.ready() {
            progress.say(&reading(&name, read, of));
            std::thread::sleep(PROGRESS_EVERY);
        }
        match self.begin(container, prefix, recurse) {
            Ok(mut opening) => {
                while !self.step(&mut opening, Some(Instant::now() + PROGRESS_EVERY)) {
                    progress.say(&opened(&name, opening.done()));
                }
                self.finish(found, opening);
            }
            Err(left) => found.left_out.push(left),
        }
        progress.done();
    }

    /// `container` listed, to open its files (its containers' too if `recurse`; its containers
    /// otherwise), each named `prefix` and its name in it; or the container's name and why it
    /// can't be listed.
    pub(super) fn begin(&self, container: Rc<dyn Container>, prefix: &str, recurse: bool) -> Result<Opening, (String, String)> {
        let listing = container.list(recurse).map_err(|why| (file_name(container.path()), why))?;
        let count = listing.entries.len();
        // The container's order, if it is one: each entry once.
        let mut sorted = listing.quickest.clone();
        sorted.sort_unstable();
        let order = match sorted.into_iter().eq(0..count) {
            true => listing.quickest.clone(),
            false => (0..count).collect(),
        };
        Ok(Opening { container, prefix: prefix.to_owned(), opened: (0..count).map(|_| None).collect(), order, next: 0, listing })
    }

    /// Open `opening`'s files until `until`, or all of them: whether all are.
    pub(super) fn step(&self, opening: &mut Opening, until: Option<Instant>) -> bool {
        while let Some(&at) = opening.order.get(opening.next) {
            if until.is_some_and(|until| Instant::now() >= until) {
                return false;
            }
            opening.next = opening.next.saturating_add(1);
            let (Some(entry), Some(slot)) = (opening.listing.entries.get(at), opening.opened.get_mut(at)) else { continue };
            // A file that nothing shows is passed over: a directory holds all sorts.
            if entry.container.is_none() {
                let name = format!("{}{}", opening.prefix, entry.name);
                let shown = opening.container.open(&entry.name).map(|input| crate::source::shown_by(input, self.backends));
                *slot = Some(shown.and_then(|chosen| self.file(chosen, &name)));
            }
        }
        true
    }

    /// Into `found`, in the order listed, the files `opening` opened and the containers in it,
    /// and what it left out.
    pub(super) fn finish(&self, found: &mut Found, opening: Opening) {
        let Opening { container, prefix, listing, opened, .. } = opening;
        let path = container.path();
        let (before, listed) = (found.items.len(), listing.entries.len());
        for (entry, opened) in listing.entries.into_iter().zip(opened) {
            let name = format!("{prefix}{}", entry.name);
            match (entry.container, opened) {
                (Some(inner), _) => found.items.push(Item::Container { name: format!("{name}/"), container: inner }),
                (None, Some(opened)) => found.file(opened, name),
                (None, None) => {}
            }
        }
        found.left_out.extend(listing.unread);
        if listing.cut {
            found.warnings.push(format!("{path}: the listing stopped at {listed} files, as many as {} lists", crate::NAME));
        }
        if found.items.len() == before {
            found.left_out.push((file_name(path), format!("{path}: nothing in it that {} shows", crate::NAME)));
        }
    }

    /// The file `chosen`, with the backend that opens it, as a tile called `name`: opened to all
    /// its pages, or for a file that holds files, to a container of them. A file no backend
    /// opens, `None`, is none.
    fn file(&self, chosen: Option<(&'static dyn Backend, Input)>, name: &str) -> Chosen {
        let Some((backend, input)) = chosen else { return Ok(None) };
        let path = input.path.clone();
        let item = match backend.open(input, self.options)? {
            Opened::Files(container) => Item::Container { name: format!("{name}/"), container },
            Opened::Pages(source) if source.pages() == 0 => return Err(format!("{path}: no pages to show")),
            Opened::Pages(source) => Item::File(Pages::all(Rc::from(source), name.to_owned())),
        };
        Ok(Some((backend, item)))
    }
}

/// `count` of `unit`, in words: `1 page`, `3 pages`.
fn counted(count: usize, unit: &str) -> String {
    format!("{count} {unit}{}", if count == 1 { "" } else { "s" })
}

/// How often a container being read or opened says how far it has got.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// How long listing a container takes before it says how far it has got.
const PROGRESS_AFTER: Duration = Duration::from_millis(300);

/// What a container still reading itself has got to: `reading photos.tar.gz… 40%`.
pub(super) fn reading(name: &str, read: u64, of: Option<u64>) -> String {
    match of.filter(|of| *of > 0) {
        Some(of) => format!("reading {name}… {}%", read.saturating_mul(100).checked_div(of).unwrap_or(0).min(100)),
        None => format!("reading {name}… {} MB", read >> 20),
    }
}

/// What opening a container's files has got to: `opening photos.tar.gz… 120 of 500`.
pub(super) fn opened(name: &str, (done, of): (usize, usize)) -> String {
    format!("opening {name}… {done} of {of}")
}

/// Says on stderr, over and over on one line, how far listing a container has got: once it has
/// taken [`PROGRESS_AFTER`], when stderr is a terminal. Gone once done.
struct Progress {
    since: Instant,
    said: bool,
}

impl Progress {
    fn new() -> Self {
        Self { since: Instant::now(), said: false }
    }

    fn say(&mut self, what: &str) {
        if self.since.elapsed() >= PROGRESS_AFTER && std::io::stderr().is_terminal() {
            let _ = write!(std::io::stderr().lock(), "\r\x1b[K{}: {}", crate::NAME, printable(what));
            self.said = true;
        }
    }

    fn done(&self) {
        if self.said {
            let _ = write!(std::io::stderr().lock(), "\r\x1b[K");
        }
    }
}

/// What [`Opener::open`] has found so far: the tiles, the backends that opened their files, the
/// files left out, with why, and what else to say.
#[derive(Default)]
pub(super) struct Found {
    pub(super) items: Vec<Item>,
    used: Vec<&'static dyn Backend>,
    pub(super) left_out: Vec<(String, String)>,
    pub(super) warnings: Vec<String>,
}

impl Found {
    /// The file called `name`, as [`Opener::file`] opened it.
    fn file(&mut self, opened: Chosen, name: String) {
        match opened {
            Ok(Some((backend, item))) => {
                if !self.used.iter().any(|b| b.name() == backend.name()) {
                    self.used.push(backend);
                }
                self.items.push(item);
            }
            Ok(None) => {}
            Err(why) => self.left_out.push((name, why)),
        }
    }
}

/// The name of the file at `path`, without its directories, as the status line gives it.
fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

/// Says `warnings`, then why each of the files `left_out` was, on stderr.
fn warn(warnings: &[String], left_out: &[(String, String)]) {
    for warning in warnings.iter().chain(left_out.iter().map(|(_, why)| why)) {
        note(&format!("warning: {warning}"));
    }
}

/// What the reader is told about the files left out, if any, once the reasons were said: how
/// many, and the first one's name.
pub(super) fn left_out_note(files: &[(String, String)]) -> Option<String> {
    match files {
        [] => None,
        [(name, _)] => Some(format!("1 file left out: {name}")),
        [(name, _), ..] => Some(format!("{} files left out: {name}, …", files.len())),
    }
}

/// Refuses an option that none of `backends` takes, nor the viewer: it would do nothing for
/// `files`, which those backends open.
fn takes_all(backends: &[&dyn Backend], options: &Parsed<'_>, files: &str) -> Result<(), String> {
    let takes = |name: &str| OPTIONS.iter().chain(backends.iter().flat_map(|b| b.options())).any(|o| o.name == name);
    let Some(name) = options.names().find(|name| !takes(name)) else { return Ok(()) };
    let names: Vec<&str> = backends.iter().map(|b| b.name()).collect();
    let (opens, verb) = match names.split_last() {
        Some((last, [])) => ((*last).to_string(), "opens"),
        Some((last, rest)) => (format!("{} and {last}", rest.join(", ")), "open"),
        None => ("nothing".into(), "opens"),
    };
    Err(format!("{name} doesn't apply to {files}, which {opens} {verb}"))
}

/// Make sure stdout is a terminal that shows kitty images, unless `--protocol kitty` says to
/// trust it. The environment answers for kitty and Ghostty; any other terminal is asked.
fn check_terminal(protocol: Protocol, tty: Option<&Tty>) -> Result<(), CliError> {
    if protocol == Protocol::Kitty {
        return Ok(());
    }
    if !std::io::stdout().is_terminal() {
        return Err(NOT_A_TERMINAL.into());
    }
    let var = |name| std::env::var(name).ok();
    if detect::env_says_kitty(var("TERM").as_deref(), var("TERM_PROGRAM").as_deref(), std::env::var_os("KITTY_WINDOW_ID").is_some()) {
        return Ok(());
    }
    let tty = tty.ok_or(UNSUPPORTED)?;
    if ask_terminal(tty)? { Ok(()) } else { Err(UNSUPPORTED.into()) }
}

/// Send the support query and read the answer.
fn ask_terminal(tty: &Tty) -> Result<bool, String> {
    let reply = ask(tty, detect::QUERY, |reply| detect::parse_reply(reply).done)?;
    Ok(detect::parse_reply(&reply).graphics)
}

/// The cell size, from the terminal, when its tty reports no pixel size: `docker exec` and
/// `kubectl exec` pass on rows and columns only. `None` when the tty reports pixels, stdout is
/// somewhere else, or the terminal doesn't say; tmux says for its client instead
/// ([`tmux_terminal`]).
fn ask_cell(tty: &Tty) -> Option<Cell> {
    let size = tty.size()?;
    if Cell::from_window(size.cols, size.rows, size.xpixel, size.ypixel).is_some() || !std::io::stdout().is_terminal() {
        return None;
    }
    let reply = ask(tty, detect::CELL_QUERY, |reply| detect::parse_cell_reply(reply).done).ok()?;
    detect::parse_cell_reply(&reply).cell.map(|(w, h)| Cell { w: f32::from(w), h: f32::from(h) })
}

/// Send `query` and read the answer until `done` says it is all there, or for [`QUERY_TIMEOUT`]
/// at most. Raw mode keeps the reply off the screen and hands it over without waiting for Enter.
fn ask(tty: &Tty, query: &[u8], done: impl Fn(&[u8]) -> bool) -> Result<Vec<u8>, String> {
    let _raw = tty.raw()?;
    tty.write_all(query)?;
    let deadline = Instant::now() + QUERY_TIMEOUT;
    let mut reply = Vec::new();
    while reply.len() < detect::MAX_REPLY {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let Some(bytes) = tty.read_timeout(left)? else { break };
        reply.extend_from_slice(&bytes);
        if done(&reply) {
            break;
        }
    }
    Ok(reply)
}

/// Inside tmux: ask tmux which terminal its client runs in (`TERM` names tmux itself), and
/// whether the pane passes escape codes through. When it doesn't, `view` stops, and leaves the
/// setting as it is ([`tmux::passthrough`]).
fn tmux_terminal(protocol: Protocol) -> Result<Terminal, CliError> {
    let to_terminal = std::io::stdout().is_terminal();
    if protocol == Protocol::Auto && !to_terminal {
        return Err(NOT_A_TERMINAL.into());
    }
    let client = tmux::client();
    let colour = tmux_colour(protocol, &client)?;
    match tmux::passthrough() {
        Ok(true) => {}
        // Nothing would show. Written elsewhere, the output may yet be shown where it is on.
        Ok(false) if to_terminal => return Err(tmux::passthrough_off().into()),
        Ok(false) => note(&format!("warning: {}", tmux::passthrough_off())),
        Err(e) => note(&format!("warning: couldn't check tmux's allow-passthrough ({e}); images may not show")),
    }
    let cell = client.ok().and_then(|c| c.cell).map(|(w, h)| Cell { w: f32::from(w), h: f32::from(h) });
    Ok(Terminal { output: Output::Tmux(colour), cell, effort: PngEffort::Fast, files: false, local: true })
}

/// The colour placeholder cells name images with, once the terminal outside tmux is known to
/// speak the protocol (or `--protocol kitty` says to trust it): 24-bit when tmux passes it
/// through unchanged, the 256-colour palette otherwise.
fn tmux_colour(protocol: Protocol, client: &Result<tmux::Client, String>) -> Result<IdColor, String> {
    match (protocol, client) {
        (Protocol::Auto, Err(e)) => Err(format!("inside tmux, but tmux didn't say which terminal it runs in ({e}); --protocol kitty tries anyway")),
        (Protocol::Auto, Ok(c)) if !detect::term_name_says_kitty(&c.termname) => {
            let name = if c.termname.is_empty() { "unknown" } else { &c.termname };
            Err(format!(
                "tmux runs in a terminal ({name}) not known to show images through the kitty graphics protocol \
                 (kitty and Ghostty are); --protocol kitty tries anyway"
            ))
        }
        (_, Ok(c)) if c.rgb => Ok(IdColor::Rgb),
        _ => Ok(IdColor::Indexed),
    }
}

/// `text` made safe to write to a terminal: no control characters, so none of the ESC, CSI or
/// OSC introducers that start escape sequences (a clipboard write, a title change, a graphics
/// command). Line breaks and tabs become spaces, the others U+FFFD. Everything a document or a
/// file name can shape goes through this on its way to the terminal.
fn printable(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' | '\r' | '\t' => ' ',
            c if c.is_control() => '\u{FFFD}',
            c => c,
        })
        .collect()
}

/// A line on stderr that doesn't stop `view`, made safe for the terminal.
fn note(line: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{}", printable(line));
}

/// Draw `pages` at the cursor, one row of them under another, like `cat`: as many to a row as the
/// pager shows at `zoom` from the first page, each row fitted the way the pager fits the reader's
/// row. A list's tiles come with `labels`, the names of `source`'s pages, each under its tile, as in
/// the pager. A page that fails is reported at the end; the others are still drawn. Inside tmux,
/// the ids of the images written to the terminal go into `sent`.
#[expect(clippy::too_many_arguments, reason = "each is a different part of what is printed, and where")]
fn print_pages(
    source: &dyn Source,
    pages: &[usize],
    labels: Option<&[String]>,
    zoom: Zoom,
    size: WindowSize,
    terminal: &Terminal,
    first_id: u64,
    sent: &mut Vec<u32>,
) -> Result<(), CliError> {
    let mut renderer = source.renderer()?;
    let cell = Cell::from_window(size.cols, size.rows, size.xpixel, size.ypixel).or(terminal.cell).unwrap_or(fit::FALLBACK_CELL);
    // One row stays free for the shell prompt after the last page.
    let rows = size.rows.saturating_sub(1).max(1);
    let (cols, max_rows) = terminal.output.limits(size.cols);
    let area = Area { cols, rows, cell, max_rows };
    let size_of = |i: usize| pages.get(i).and_then(|p| page_size(source, *p));
    let grid_at = |zoom: Zoom, start: usize| match labels {
        Some(_) => fit::tiles(zoom, pages.len(), area),
        None => fit::grid(zoom, pages.len(), start, size_of, area),
    };
    // Every row has as many pages as the first: `auto` decides once.
    let first = grid_at(zoom, 0);
    let zoom = if first.fit == Fit::Width { Zoom::Width } else { Zoom::PerRow(first.per_row) };
    let per_row = usize::from(first.per_row.max(1));
    let (mut failed, mut drawn_rows) = (Vec::new(), 0usize);
    let mut stdout = std::io::stdout().lock();
    for (row, chunk) in pages.chunks(per_row).enumerate() {
        let start = row.saturating_mul(per_row);
        let grid = grid_at(zoom, start);
        let mut printed = Vec::new();
        for (slot, &page) in (0u16..).zip(chunk) {
            let drawn = page_layout(source, page, grid.fit, grid.cols, grid.page_rows, cell, max_rows)
                .and_then(|layout| encode(&renderer.render(request(page, &layout, 0)), terminal.effort).map(|image| (layout, image)));
            match drawn {
                Ok((layout, image)) => {
                    if let Some(warning) = &image.warning {
                        note(&format!("warning: {warning}"));
                    }
                    let index = u64::try_from(start.saturating_add(usize::from(slot))).unwrap_or(u64::MAX);
                    printed.push(Printed { slot, index, layout, png: image.png });
                }
                Err(e) => failed.push(e),
            }
        }
        if printed.is_empty() {
            continue;
        }
        let mut out = Vec::new();
        // Rows of pages side by side, or of tiles, have a row between them, as in the pager.
        if drawn_rows > 0 && (grid.per_row > 1 || labels.is_some()) {
            out.extend(std::iter::repeat_n(b'\n', usize::from(fit::ROW_GAP)));
        }
        drawn_rows = drawn_rows.saturating_add(1);
        print_row(&mut out, &printed, grid, terminal.output, first_id);
        if let Output::Tmux(colour) = terminal.output {
            sent.extend(printed.iter().map(|p| colour.id(first_id.wrapping_add(p.index))));
        }
        if let Some(labels) = labels {
            let names = chunk.iter().map(|page| labels.get(*page).map_or("", String::as_str));
            let line: Vec<String> = names.map(|name| label(name, grid.cols)).collect();
            out.extend_from_slice(printable(line.join(&" ".repeat(usize::from(fit::GAP))).trim_end()).as_bytes());
            out.push(b'\n');
        }
        stdout.write_all(&out).and_then(|()| stdout.flush()).map_err(CliError::Stdout)?;
    }
    if failed.is_empty() { Ok(()) } else { Err(failed.join("; ").into()) }
}

/// A page of a printed row: its place in the row, its place in the list (which picks its image
/// id inside tmux), the cells it covers, and its image.
struct Printed {
    slot: u16,
    index: u64,
    layout: Layout,
    png: Vec<u8>,
}

/// Draw a row of printed pages, at least one, at the cursor where `grid` puts them, and leave the
/// cursor at the start of the line after it. A page alone in its row is drawn the way text is, the cursor and
/// the screen moving down with it, so a page taller than the window scrolls by. Pages side by side
/// go into a band of lines made first, each placed from the band's top with the cursor left
/// where it is; the band is never taller than the window, so going back up to it always works.
fn print_row(out: &mut Vec<u8>, row: &[Printed], grid: Grid, output: Output, first_id: u64) {
    // A tile is as tall as its place, which is never taller than the window.
    let alone = grid.per_row <= 1 && grid.label == 0;
    // Where a page goes in the band: across, and down to centre it in its place.
    let at = |p: &Printed| {
        let across = p.slot.saturating_mul(grid.cols.saturating_add(fit::GAP)).saturating_add(grid.cols.saturating_sub(p.layout.cols) / 2);
        if alone { (0, 0) } else { (across, grid.page_rows.saturating_sub(p.layout.rows) / 2) }
    };
    match output {
        Output::Direct if alone => {
            for p in row {
                kitty::show_png(out, &p.png, p.layout.cols, p.layout.rows);
                out.push(b'\n');
            }
        }
        Output::Direct => {
            let band = grid.page_rows;
            out.extend(std::iter::repeat_n(b'\n', usize::from(band)));
            cursor_by(out, band, 'A');
            for p in row {
                let (x, y) = at(p);
                out.push(b'\r');
                cursor_by(out, x, 'C');
                cursor_by(out, y, 'B');
                kitty::show_png_in_place(out, &p.png, p.layout.cols, p.layout.rows);
                cursor_by(out, y, 'A');
            }
            cursor_by(out, band, 'B');
            out.push(b'\r');
        }
        Output::Tmux(colour) => {
            let id = |p: &Printed| colour.id(first_id.wrapping_add(p.index));
            for p in row {
                kitty::transmit(out, id(p), &p.png, Wrap::Tmux);
                kitty::place_virtual(out, id(p), p.layout.cols, p.layout.rows, Wrap::Tmux);
            }
            // A line of text for each row of cells: every page's placeholder cells in its place.
            let lines = if alone { row.first().map_or(0, |p| p.layout.rows) } else { grid.page_rows };
            for line in 0..lines {
                let mut written = 0u16;
                for p in row {
                    let (x, y) = at(p);
                    let Some(cells) = line.checked_sub(y).filter(|r| *r < p.layout.rows) else { continue };
                    out.extend(std::iter::repeat_n(b' ', usize::from(x.saturating_sub(written))));
                    // The layout keeps rows and columns within the placeholders' table.
                    if kitty::placeholder_row(out, id(p), colour, cells, p.layout.cols) {
                        written = x.max(written).saturating_add(p.layout.cols);
                    }
                }
                out.push(b'\n');
            }
        }
    }
}

/// Characters, as the terminal shows them one to a cell.
fn chars(text: &str) -> usize {
    text.chars().count()
}

/// `text` in at most `room` characters: whole, or its start and its end around `…`, the end the
/// longer, so names that differ only at the end (`IMG_2041.jpg`, `IMG_2042.jpg`) still differ.
fn cut(text: &str, room: usize) -> String {
    let count = chars(text);
    if count <= room {
        return text.to_owned();
    }
    let Some(kept) = room.checked_sub(1) else { return String::new() };
    let start = kept / 2;
    let end = kept.saturating_sub(start);
    text.chars().take(start).chain(['…']).chain(text.chars().skip(count.saturating_sub(end))).collect()
}

/// A tile's name as it goes under the tile, `cols` wide: cut to fit, in the middle of them.
fn label(name: &str, cols: u16) -> String {
    let cols = usize::from(cols);
    let name = cut(name, cols);
    let left = cols.saturating_sub(chars(&name)) / 2;
    format!("{}{name}{}", " ".repeat(left), " ".repeat(cols.saturating_sub(left).saturating_sub(chars(&name))))
}

/// Move the cursor `n` cells (`code`: `A` up, `B` down, `C` right); nothing for 0, which the
/// terminal would take as 1.
fn cursor_by(out: &mut Vec<u8>, n: u16, code: char) {
    if n > 0 {
        out.extend_from_slice(format!("\x1b[{n}{code}").as_bytes());
    }
}

/// Where this run's image ids start ([`IdColor::id`]): mixed from the clock and the process id,
/// so runs in the same terminal are unlikely to draw over each other's images.
fn fresh_index() -> u64 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    now.as_secs().wrapping_mul(1_000_000_007) ^ u64::from(now.subsec_nanos()) ^ u64::from(std::process::id()).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// A page rendered for the terminal, as a PNG.
struct Image {
    png: Vec<u8>,
    /// Something the reader should know about the page, such as content left out.
    warning: Option<String>,
}

/// `source`'s `page`'s size, and whether it is a natural one; `None` past its last page.
fn page_size(source: &dyn Source, page: usize) -> Option<Size> {
    source.size(page).map(|(w, h)| Size { w, h, natural: source.natural(page) })
}

/// Where `source`'s `page` goes in `cols` × `rows` cells, never more than `max_rows` tall. Errors
/// name the page.
fn page_layout(source: &dyn Source, page: usize, fit: Fit, cols: u16, rows: u16, cell: Cell, max_rows: u16) -> Result<Layout, String> {
    let number = page.saturating_add(1);
    let size = page_size(source, page).ok_or_else(|| format!("no page {number}"))?;
    fit::layout(size, cols, rows, cell, fit, max_rows).map_err(|e| format!("page {number}: {e}"))
}

/// The render of `page` at `layout`'s scale, tagged so its result can be told from others.
fn request(page: usize, layout: &Layout, tag: u64) -> Request {
    Request { page, scale: layout.scale, tag }
}

/// A rendered page as a PNG, or why there is none. Errors name the page.
fn encode(p: &Frame, effort: PngEffort) -> Result<Image, String> {
    let warning = checked(p)?;
    Ok(Image { png: png(p, effort)?, warning })
}

/// Why a page failed to render, or what the reader should know about it, if anything, in the
/// backend's words. Both name the page.
fn checked(p: &Frame) -> Result<Option<String>, String> {
    let number = p.request.page.saturating_add(1);
    if let Some(e) = &p.error {
        return Err(format!("page {number}: {e}"));
    }
    Ok((!p.warnings.is_empty()).then(|| format!("page {number}: {}", p.warnings.join("; "))))
}

/// A rendered page's pixels as a PNG. Errors name the page.
fn png(p: &Frame, effort: PngEffort) -> Result<Vec<u8>, String> {
    let number = p.request.page.saturating_add(1);
    encode::encode_png(p.width, p.height, &p.rgba, effort).map_err(|e| format!("page {number}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    const PASSWORD: OptionSpec = OptionSpec { name: "--password", short: None, value: Some("PW"), help: "a password" };

    /// The viewer's options, and a backend's `--password`.
    fn parsed(given: &[String]) -> Parsed<'_> {
        Parsed::parse(given, &[OPTIONS, &[PASSWORD]].concat()).unwrap()
    }

    #[test]
    fn the_files_are_the_arguments_that_are_not_option_values() {
        let given = |list: &[&str]| file_args(&parsed(&args(list))).map(|files| files.iter().map(|f| f.to_string()).collect::<Vec<_>>());
        assert_eq!(given(&["a.pdf", "--pages", "2"]), Ok(vec!["a.pdf".into()]));
        assert_eq!(given(&["--pages", "2", "a.pdf"]), Ok(vec!["a.pdf".into()]));
        assert_eq!(given(&["--password", "a.pdf", "b.pdf"]), Ok(vec!["b.pdf".into()]));
        // `--pager` takes no value; the files are in the order given.
        assert_eq!(given(&["b.png", "--pager", "a.pdf"]), Ok(vec!["b.png".into(), "a.pdf".into()]));
        assert!(given(&["--zoom", "2"]).is_err_and(|e| e.contains("missing file")));
    }

    #[test]
    fn an_option_that_none_of_the_files_backends_takes_is_refused() {
        use crate::source::tests::{Fake, PDF};
        const LOCKED: Fake = Fake { name: "locked", shows: "PDFs", magic: b"%PDF-", options: &[PASSWORD], holds: false };
        const PNG: Fake = Fake { name: "png", shows: "images", magic: b"\x89PNG", options: &[], holds: false };
        let given = args(&["a.pdf", "--zoom", "2", "--password", "pw"]);
        assert_eq!(takes_all(&[&LOCKED], &parsed(&given), "a.pdf"), Ok(()));
        assert_eq!(takes_all(&[&PDF], &parsed(&given), "a.pdf"), Err("--password doesn't apply to a.pdf, which pdf-one opens".into()));
        assert_eq!(takes_all(&[&PDF], &parsed(&args(&["a.pdf", "--zoom", "2", "--no-pager"])), "a.pdf"), Ok(()), "the viewer's own apply to all");
        // Among several files, it is enough that one of them takes it.
        assert_eq!(takes_all(&[&PNG, &LOCKED], &parsed(&given), "any of these files"), Ok(()));
        assert_eq!(
            takes_all(&[&PNG, &PDF], &parsed(&given), "any of these files"),
            Err("--password doesn't apply to any of these files, which png and pdf-one open".into())
        );
    }

    #[test]
    fn the_reader_is_told_how_many_files_were_left_out_and_the_first_ones_name() {
        let file = |name: &str| (name.to_string(), format!("dir/{name}: no"));
        assert_eq!(left_out_note(&[]), None);
        assert_eq!(left_out_note(&[file("a.txt")]).as_deref(), Some("1 file left out: a.txt"));
        assert_eq!(left_out_note(&[file("a.txt"), file("b")]).as_deref(), Some("2 files left out: a.txt, …"));
    }

    /// A file that holds files (an archive) is a container: alone, a list of its files, as a
    /// directory alone; among others, a folder; recursing among others, its files, named from it.
    #[test]
    fn a_file_that_holds_files_is_listed_as_a_directory_is() {
        use crate::source::tests::{ARCHIVE, PDF};
        let dir = std::env::temp_dir().join(format!("clview-holds-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("book.arc.d/ch")).unwrap();
        for (name, bytes) in [("book.arc", &b"\0arc"[..]), ("book.arc.d/a.pdf", b"%PDF-"), ("book.arc.d/ch/b.pdf", b"%PDF-"), ("c.pdf", b"%PDF-")] {
            std::fs::write(dir.join(name), bytes).unwrap();
        }
        let (book, c) = (dir.join("book.arc"), dir.join("c.pdf"));
        let (book, c) = (book.to_str().unwrap(), c.to_str().unwrap());
        let given = args(&[]);
        let opener = Opener { backends: &[&PDF, &ARCHIVE], options: &parsed(&given) };
        let names = |paths: &[&str], recurse: bool| {
            let shown = opener.open(paths, recurse).map_err(|e| e.to_string()).unwrap().shown;
            shown.items.iter().map(|item| item.name().to_owned()).collect::<Vec<_>>()
        };
        assert_eq!(names(&[book], false), ["a.pdf", "ch/"]);
        assert_eq!(names(&[book], true), ["a.pdf", "ch/b.pdf"]);
        assert_eq!(names(&[book, c], false), ["book.arc/", "c.pdf"]);
        assert_eq!(names(&[book, c], true), ["book.arc/a.pdf", "book.arc/ch/b.pdf", "c.pdf"]);
        // A file alone is its pages, not a list.
        assert!(!opener.open(&[c], false).map_err(|e| e.to_string()).unwrap().shown.is_list());
        // In a directory, a folder, even recursing.
        std::fs::remove_dir_all(dir.join("book.arc.d/ch")).unwrap();
        let top = dir.to_str().unwrap();
        assert_eq!(names(&[top], true), ["book.arc/", "book.arc.d/a.pdf", "c.pdf"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A container's files are opened in the order it says is quickest (a compressed tar's, as they
    /// come in it), and shown in the order listed; an order that isn't each entry once is passed
    /// over.
    #[test]
    fn a_containers_files_are_opened_in_its_quickest_order() {
        use std::cell::RefCell;

        use crate::source::tests::PDF;
        use crate::source::{Listed, Listing};

        struct Ordered {
            dir: std::path::PathBuf,
            quickest: Vec<usize>,
            opened: RefCell<Vec<String>>,
        }
        impl Container for Ordered {
            fn path(&self) -> &str {
                "ordered"
            }
            fn list(&self, _recurse: bool) -> Result<Listing, String> {
                let entries = ["a.pdf", "b.pdf", "c.pdf"].map(|name| Listed { name: name.into(), container: None }).into();
                Ok(Listing { entries, quickest: self.quickest.clone(), ..Listing::default() })
            }
            fn open(&self, name: &str) -> Result<Input, String> {
                self.opened.borrow_mut().push(name.to_owned());
                crate::source::choose(self.dir.join(name).to_str().unwrap(), &[&PDF]).map(|(_, input)| input)
            }
        }
        let dir = std::env::temp_dir().join(format!("clview-quickest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["a.pdf", "b.pdf", "c.pdf"] {
            std::fs::write(dir.join(name), b"%PDF-").unwrap();
        }
        let given = args(&[]);
        let opener = Opener { backends: &[&PDF], options: &parsed(&given) };
        for (quickest, opened) in [(vec![2, 0, 1], ["c.pdf", "a.pdf", "b.pdf"]), (vec![0, 0, 1], ["a.pdf", "b.pdf", "c.pdf"])] {
            let container = Rc::new(Ordered { dir: dir.clone(), quickest, opened: RefCell::default() });
            let mut found = Found::default();
            opener.list(&mut found, container.clone(), "", false);
            assert_eq!(*container.opened.borrow(), opened);
            assert_eq!(found.items.iter().map(Item::name).collect::<Vec<_>>(), ["a.pdf", "b.pdf", "c.pdf"]);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn help_lists_the_options_each_backend_takes_then_the_formats_they_show_and_through_what() {
        use crate::source::Format;
        use crate::source::tests::{Fake, PDF};
        /// A backend that shows `.0` through the libraries `.1`, in the formats `.2`, and opens
        /// nothing.
        struct Lists(&'static str, &'static [&'static str], &'static [Format]);
        impl Backend for Lists {
            fn name(&self) -> &'static str {
                "lists"
            }
            fn shows(&self) -> &'static str {
                self.0
            }
            fn formats(&self) -> Vec<Format> {
                self.2.to_vec()
            }
            fn credits(&self) -> &[&'static str] {
                self.1
            }
            fn options(&self) -> &'static [OptionSpec] {
                &[]
            }
            fn opens(&self, _path: &str, _head: &[u8]) -> bool {
                false
            }
            fn open(&self, input: Input, _options: &Parsed<'_>) -> Result<Opened, String> {
                Err(input.path)
            }
        }
        const fn format(name: &'static str, extensions: &'static [&'static str]) -> Format {
            Format { name, extensions }
        }
        const LOCKED: Fake = Fake { name: "locked", shows: "locked PDFs", magic: b"%PDF-", options: &[PASSWORD], holds: false };
        const PICTURES: Lists = Lists(
            "images",
            &["lib-a"],
            &[
                format("PNG", &[]),
                format("TGA", &["tga", "icb", "vda", "vst"]),
                format("a format with a long name", &[]),
                format("another format with a long name", &["one", "two"]),
            ],
        );
        const MORE: Lists = Lists("images", &["lib-a", "lib-b"], &[format("PNG", &[]), format("WebP", &[])]);
        const NONE: Lists = Lists("nothing", &["lib-c"], &[]);
        const OWN: Lists = Lists("drawings", &[], &[format("SVG", &[])]);
        let help = usage(&[&LOCKED, &PDF, &NONE, &OWN, &PICTURES, &MORE]);
        let is = format!("clview {}\nCommand-line Viewer: view graphic files without leaving your terminal", env!("CARGO_PKG_VERSION"));
        assert!(help.starts_with(&format!("{is}\n\n{0} [options] <file>…\n{0} --help | --version\n", crate::NAME)), "{help}");
        // Aligned under the longest, `--compression auto|fast|small`.
        for option in OPTIONS {
            assert!(help.contains(&format!("\n  {:29}  {}\n", option.shown(), option.help)), "{help}");
        }
        // A backend's options under what it shows, and nothing for one that takes none.
        assert!(help.contains(&format!("\n\nLocked PDFs:\n  {:29}  a password\n\n", "--password PW")), "{help}");
        assert!(!help.contains("\nPDFs:\n") && !help.contains("\nImages:\n") && !help.contains("no options"), "{help}");
        // Last, the formats, under what their backend shows and the libraries it credits: those of
        // backends that show the same together, each once, with the names a file of it goes by,
        // wrapped at 100 columns; no credit for a backend that credits none, and no row for one
        // that lists no format.
        let shown = help.split_once(&format!("\n\n{SHOWN}\n")).map_or("", |(_, shown)| shown);
        let images = format!(
            "  Images, by lib-a and lib-b: PNG, TGA (.tga .icb .vda .vst), a format with a long name,\n{}another format with a long name (.one .two), WebP\n",
            " ".repeat(30)
        );
        assert_eq!(shown, format!("  Locked PDFs, by locked: locked\n  PDFs, by pdf-one: pdf-one\n  Drawings: SVG\n{images}\n{REPORT}\n"), "{help}");
        assert!(!help.contains("lib-c"), "{help}");
        assert_eq!([by(&[]), by(&["a"]), by(&["a", "b"]), by(&["a", "b", "c"])], ["", ", by a", ", by a and b", ", by a, b and c"]);
        assert!(shown.lines().all(|line| line.chars().count() <= HELP_WIDTH), "{help}");
        // An item longer than a line has one to itself.
        let mut out = String::new();
        wrapped(&mut out, "Long:", &["x".repeat(HELP_WIDTH), "y".into()]);
        assert_eq!(out, format!("  Long: {},\n        y\n", "x".repeat(HELP_WIDTH)));
        let nothing = usage(&[]);
        assert!(nothing.contains(" first\n\nThis build has no backends, so it shows nothing.\n\n") && !nothing.contains(SHOWN), "{nothing}");
        // Last, where to report issues, whatever the build has.
        for help in [&help, &nothing] {
            assert!(help.ends_with("\n\nReport issues and ideas to: https://github.com/dobladez/clview\n"), "{help}");
        }
    }

    #[test]
    fn nothing_written_out_can_start_an_escape_sequence() {
        // OSC 52 (a clipboard write), a title change, a kitty graphics command, 8-bit CSI and OSC.
        let hostile = "a\x1b]52;c;cHduZWQ=\x07b\x1b]0;title\x1b\\c\x1b_Ga=d\x1b\\d\u{9b}2J\u{9d}0;t\u{7}";
        let safe = printable(hostile);
        assert!(!safe.chars().any(char::is_control), "{safe:?}");
        assert!(safe.starts_with("a\u{FFFD}]52;c;cHduZWQ=\u{FFFD}b"), "{safe:?}");
        assert_eq!(printable("two\nlines\tand\r"), "two lines and ");
        assert_eq!(printable("rapport-été.pdf"), "rapport-été.pdf");
    }

    #[test]
    fn page_ids_follow_on_from_the_runs_start() {
        let first = fresh_index();
        let (a, b) = (IdColor::Indexed.id(first), IdColor::Indexed.id(first.wrapping_add(1)));
        assert_ne!(a, b);
        assert_ne!(a & 0xff, 0);
    }

    #[test]
    fn inside_tmux_the_client_terminal_decides() {
        let client = |termname: &str, rgb| Ok(tmux::Client { termname: termname.into(), rgb, cell: None });
        assert_eq!(tmux_colour(Protocol::Auto, &client("xterm-kitty", true)), Ok(IdColor::Rgb));
        assert_eq!(tmux_colour(Protocol::Auto, &client("xterm-ghostty", false)), Ok(IdColor::Indexed));
        let other = tmux_colour(Protocol::Auto, &client("xterm-256color", true));
        assert!(other.is_err_and(|e| e.contains("(xterm-256color)") && e.contains("--protocol kitty")));
        let unknown = tmux_colour(Protocol::Auto, &Err("tmux: not found".into()));
        assert!(unknown.is_err_and(|e| e.contains("tmux: not found")));
        // Trusted: whatever tmux says, or when it can't say.
        assert_eq!(tmux_colour(Protocol::Kitty, &client("xterm-256color", true)), Ok(IdColor::Rgb));
        assert_eq!(tmux_colour(Protocol::Kitty, &Err("tmux: not found".into())), Ok(IdColor::Indexed));
    }

    #[test]
    fn placeholders_limit_the_size_of_a_page() {
        assert_eq!(Output::Direct.limits(400), (400, u16::MAX));
        assert_eq!(Output::Tmux(IdColor::Rgb).limits(400), (297, 297));
        assert_eq!(Output::Tmux(IdColor::Rgb).limits(80), (80, 297));
    }

    #[test]
    fn more_than_a_page_on_a_terminal_opens_the_pager() {
        let auto = Paging::from_flags(false, false).unwrap();
        assert!(auto.pages(18, true));
        assert!(!auto.pages(1, true), "one page is printed, as less -F does");
        assert!(!auto.pages(18, false), "not on a terminal");
        assert!(Paging::from_flags(true, false).unwrap().pages(1, true));
        assert!(!Paging::from_flags(false, true).unwrap().pages(18, true));
        assert!(Paging::from_flags(true, true).is_err_and(|e| e.contains("not both")));
    }

    #[test]
    fn compression_follows_where_the_terminal_is() {
        assert_eq!(Compression::parse("auto"), Ok(Compression::Auto));
        assert!(Compression::parse("best").is_err_and(|e| e.contains("auto, fast or small")));
        // Over SSH, small PNGs; on this machine, fast ones, and temporary files may be tried.
        assert_eq!(Compression::Auto.choose(|| true), (PngEffort::Small, false));
        assert_eq!(Compression::Auto.choose(|| false), (PngEffort::Fast, true));
        // Chosen by hand: PNGs in the stream, and where the terminal is isn't even asked.
        assert_eq!(Compression::Fast.choose(|| panic!("asked")), (PngEffort::Fast, false));
        assert_eq!(Compression::Small.choose(|| panic!("asked")), (PngEffort::Small, false));
        // tmux's session knows better than a pane's environment, which may be from another attach.
        assert!(over_ssh(true, None));
        assert!(!over_ssh(false, None));
        assert!(!over_ssh(true, Some(false)));
        assert!(over_ssh(false, Some(true)));
    }

    #[test]
    fn protocol_names() {
        assert_eq!(Protocol::parse("auto"), Ok(Protocol::Auto));
        assert_eq!(Protocol::parse("kitty"), Ok(Protocol::Kitty));
        assert!(Protocol::parse("sixel").is_err_and(|e| e.contains("auto or kitty")));
    }

    /// Page 3 rendered with `warnings`, or failed with `error`.
    fn rendered(warnings: &[&str], error: Option<&str>) -> Frame {
        let request = Request { page: 2, scale: 1.0, tag: 0 };
        let rgba = crate::source::Pixels::new(Vec::new());
        Frame { request, width: 0, height: 0, rgba, error: error.map(String::from), warnings: warnings.iter().map(|w| w.to_string()).collect() }
    }

    #[test]
    fn a_page_says_everything_the_backend_warned_about() {
        assert_eq!(checked(&rendered(&[], None)), Ok(None));
        assert_eq!(checked(&rendered(&["a colour was guessed"], None)), Ok(Some("page 3: a colour was guessed".into())));
        assert_eq!(
            checked(&rendered(&["content was skipped", "a colour was guessed"], None)),
            Ok(Some("page 3: content was skipped; a colour was guessed".into()))
        );
        // A page that failed says only that.
        assert_eq!(checked(&rendered(&["a colour was guessed"], Some("no page box"))), Err("page 3: no page box".into()));
    }

    /// Where a page goes depends only on its size, and whether it is a natural one: a source past
    /// its last page has no layout.
    #[test]
    fn a_page_is_laid_out_from_its_sources_size() {
        struct Sizes(Vec<(f32, f32)>);
        impl Source for Sizes {
            fn pages(&self) -> usize {
                self.0.len()
            }
            fn size(&self, page: usize) -> Option<(f32, f32)> {
                self.0.get(page).copied()
            }
            fn natural(&self, page: usize) -> bool {
                page == 2
            }
            fn renderer(&self) -> Result<Box<dyn crate::source::Render + '_>, String> {
                Err("not here".into())
            }
            fn pool(&self, _threads: usize) -> Box<dyn crate::source::Pool> {
                Box::new(Idle)
            }
        }
        struct Idle;
        impl crate::source::Pool for Idle {
            fn set_queue(&self, _requests: Vec<Request>) {}
            fn try_recv(&self) -> Option<Frame> {
                None
            }
        }
        let source = Sizes(vec![(612.0, 792.0), (792.0, 612.0), (100.0, 100.0)]);
        let cell = Cell { w: 10.0, h: 20.0 };
        let portrait = page_layout(&source, 0, Fit::Page, 80, 24, cell, 24).unwrap();
        let landscape = page_layout(&source, 1, Fit::Page, 80, 24, cell, 24).unwrap();
        // An 800 × 480 pixel window: both fill its height, the landscape page wider, each scaled to
        // 480 pixels over its own height.
        assert_eq!((portrait.rows, landscape.rows), (24, 24));
        assert!(portrait.cols < landscape.cols, "{portrait:?} {landscape:?}");
        assert!((portrait.scale - 480.0 / 792.0).abs() < 1e-4 && (landscape.scale - 480.0 / 612.0).abs() < 1e-4, "{portrait:?} {landscape:?}");
        // A picture, the size it is.
        let picture = page_layout(&source, 2, Fit::Page, 80, 24, cell, 24).unwrap();
        assert_eq!((picture.scale, picture.cols, picture.rows), (1.0, 10, 5));
        assert_eq!(page_layout(&source, 3, Fit::Page, 80, 24, cell, 24).map(|_| ()), Err("no page 4".into()));
    }
}
