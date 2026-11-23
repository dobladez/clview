//! The reader's config: a TOML file that sets options, by the names of the options catalog
//! ([`crate::args::Catalog`]), and binds keys to commands, by the names of the catalog of commands
//! ([`super::commands`]).
//!
//! clv reads `$XDG_CONFIG_HOME/clv/config.toml` when that variable holds an absolute path, and
//! `$HOME/.config/clv/config.toml` otherwise, on macOS too. `--config FILE` reads another file, and
//! `--no-config` none ([`Which`]). The file is untrusted input, as every file is: clv reads at most
//! [`MAX_SIZE`] of it, and whatever is wrong with it is an error that names the file and the line.
//!
//! The config is also written here, as text ([`written`]): the config in effect, for
//! `--print-config`, and with nothing set the template that `--generate-config` puts where there
//! is no config yet ([`generate`]). clv writes a config at no other time.
//! Why TOML, and why the file is flat:
//! [ADR-0043](../../docs/adr/0043-a-config-in-toml-flat-keyed-by-the-names-of-the-catalogs.md).

use std::io::Read;
use std::path::{Path, PathBuf};

use toml::de::{DeTable, DeValue};

use super::commands::{self, Binding, Keymaps, Mode};
use super::keys::{self, Key};
use crate::args::{Arg, Catalog, Entry, Parsed, Settable};
use crate::source::Setting;

/// The most of a config that clv reads, in bytes: 1 MiB. A config of a few options and keys is a
/// few hundred bytes, and a larger file is refused before it is parsed.
const MAX_SIZE: u64 = 1 << 20;

/// Where the config is, under the directory of configs.
const FILE: &str = "config.toml";

/// The table of the keys, at the top level, and its two tables of the keys of one keymap.
const KEYS: &str = "keys";
const KEYMAPS: [(&str, Mode); 2] = [("pages", Mode::Pages), ("list", Mode::List)];

/// What unbinds a key, in place of the name of a command.
pub(super) const NONE: &str = "none";

/// Which config to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Which<'a> {
    /// The reader's own, where there is one: none is not an error.
    Own,
    /// `--config FILE`: this file, which must be there.
    File(&'a str),
    /// `--no-config`: none.
    None,
}

/// An option that a config sets: the option's name in the catalog (`--per-row`), its value as the
/// command line would give it (none for a flag), and the line that sets it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Set {
    pub(super) name: &'static str,
    pub(super) value: Option<String>,
    pub(super) line: usize,
}

/// What is wrong with a config, and on which line, where a line says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Wrong {
    pub(super) line: Option<usize>,
    pub(super) why: String,
}

impl Wrong {
    fn at(line: usize, why: impl Into<String>) -> Self {
        Self { line: Some(line), why: why.into() }
    }
}

/// A config as read: what it sets. One that was not read (none there, or `--no-config`) sets
/// nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Config {
    /// The file, as a message names it: `~/.config/clv/config.toml`. Empty where none was read.
    pub(super) file: String,
    /// The options it sets, in the order of the file.
    pub(super) options: Vec<Set>,
    /// The keys it binds, in the order of the file, as it writes them: the keymaps say which are
    /// keys and commands ([`super::commands::Keymaps::bound`]).
    pub(super) keys: Vec<Binding>,
}

impl Config {
    /// The config `which` says, read against `catalog`. Whatever is wrong is the error, which
    /// names the file, and the line where one says it: clv stops before it shows anything.
    pub(super) fn read(which: Which<'_>, catalog: &Catalog) -> Result<Self, String> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let (path, must) = match which {
            Which::None => return Ok(Self::default()),
            Which::File(path) => (PathBuf::from(path), true),
            Which::Own => match own(std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from), home.clone()) {
                Some(path) => (path, false),
                None => return Ok(Self::default()),
            },
        };
        let file = shown(&path, home.as_deref());
        let Some(text) = load(&path, must).map_err(|why| format!("{file}: {why}"))? else { return Ok(Self::default()) };
        match Self::parse(&text, catalog) {
            Ok(config) => Ok(Self { file, ..config }),
            Err(wrong) => Err(said(&file, &wrong)),
        }
    }

    /// What is wrong on `line` of this config, as the reader is told.
    pub(super) fn wrong(&self, line: usize, why: &str) -> String {
        said(&self.file, &Wrong::at(line, why))
    }

    /// `text` as a config, read against `catalog`: what its top level sets, and what `[keys]`,
    /// `[keys.pages]` and `[keys.list]` bind.
    pub(super) fn parse(text: &str, catalog: &Catalog) -> Result<Self, Wrong> {
        let table = DeTable::parse(text).map_err(|e| Wrong { line: e.span().map(|span| line(text, span.start)), why: e.message().to_owned() })?;
        // A table is kept in the order of its keys' names: the order of the file is by where each is.
        let mut entries: Vec<_> = table.get_ref().iter().collect();
        entries.sort_by_key(|(key, _)| key.span().start);
        let mut config = Self::default();
        for (key, value) in entries {
            let (name, at) = (key.get_ref().as_ref(), line(text, key.span().start));
            if name == KEYS {
                let DeValue::Table(keys) = value.get_ref() else { return Err(Wrong::at(at, format!("{KEYS}: expected a table of keys"))) };
                bindings(keys, None, text, catalog, &mut config.keys)?;
                continue;
            }
            // A line that binds a key, written above the tables: said, with where it goes.
            let binds = matches!(value.get_ref(), DeValue::String(_)) && keys::named(name).is_ok();
            let misplaced =
                |why: String| if binds && why == unknown(name) { format!("{why}: {name} is a key, and a key is bound under [{KEYS}]") } else { why };
            let entry = option(catalog, name).map_err(|why| Wrong::at(at, misplaced(why)))?;
            let wrong = |why: String| Wrong::at(at, format!("{name}: {why}"));
            if let Some(value) = given(entry, value.get_ref(), text.get(value.span()).unwrap_or_default()).map_err(wrong)? {
                config.options.push(Set { name: entry.spec.name, value, line: at });
            }
        }
        // In the order of the file, whichever table each is in.
        config.keys.sort_by_key(|binding| binding.at);
        Ok(config)
    }
}

/// Into `into`, what the table `keys` binds in `mode` (`None`, in every keymap where the command
/// applies): each key and the name of its command, or [`NONE`], as written. The tables of the
/// keymaps, in the table of them all, are read in turn.
fn bindings(keys: &DeTable<'_>, mode: Option<Mode>, text: &str, catalog: &Catalog, into: &mut Vec<Binding>) -> Result<(), Wrong> {
    for (key, value) in keys.iter() {
        let (name, at) = (key.get_ref().as_ref(), key.span().start);
        let keymap = KEYMAPS.iter().find(|(keymap, _)| mode.is_none() && *keymap == name);
        // An option written under a table of keys, as a line added at the file's end is: said, with
        // where it goes. No key is called as an option is.
        if keymap.is_none() && catalog.options().any(|entry| entry.spec.key() == name) {
            return Err(Wrong::at(line(text, at), format!("{name} is an option, not a key: write it above [{KEYS}], before the first table")));
        }
        match (value.get_ref(), keymap) {
            (DeValue::Table(keys), Some((_, mode))) => bindings(keys, Some(*mode), text, catalog, into)?,
            (DeValue::String(command), _) => {
                into.push(Binding { mode, key: name.to_owned(), command: command.to_string(), line: line(text, at), at })
            }
            (other, _) => {
                let got: String = match other {
                    DeValue::Array(_) => "an array".to_owned(),
                    DeValue::Table(_) => "a table".to_owned(),
                    _ => text.get(value.span()).unwrap_or_default().chars().take(40).collect(),
                };
                return Err(Wrong::at(line(text, at), format!("{name}: expected the name of a command or \"{NONE}\", got {got}")));
            }
        }
    }
    Ok(())
}

/// What a config's template starts with: how to read it. True of the config in effect as
/// `--print-config` writes it too, where a line that something set has no `#`.
const ABOUT: &str = "\
The config of clv. clv reads this file when it starts, and never changes it.

A line that starts with \"##\" is an explanation.
A line that starts with one \"#\" is an example: it shows the default.
To change a setting, remove the \"# \" from its line and edit the value.
A line that keeps its \"#\" changes nothing.";

/// How many `#` make a rule: the line above and below the title of each part of the template.
const RULE: usize = 78;

/// The titles of the template's parts: its options, its keys, and each of the three tables of
/// the keys.
const OPTIONS_TITLE: &str = "Options";
const KEYS_TITLE: &str = "Keys";
const BOTH_TITLE: &str = "Keys in the pages of a file and in a list";
const KEYMAP_TITLES: [&str; 2] = ["Keys in the pages of a file only", "Keys in a list only"];

/// What the template says under the title of the options.
const ABOUT_OPTIONS: &str = "An option on the command line wins over the same option here.";

/// What the template says under the title of the keys.
const ABOUT_KEYS: &str = "\
Each line binds a key to a command: key = \"command\".
Write \"none\" in place of the command to unbind the key.
A key that you bind comes first among the keys of its command. The other keys stay.
A punctuation key needs quotes: \"/\" = \"search\".

The three lines in brackets start the tables of the keys. Leave them as they are.
  [keys]        binds a key in the pages of a file and in a list
  [keys.pages]  binds a key in the pages of a file only
  [keys.list]   binds a key in a list only

A config cannot change two bindings: ctrl-c always quits, and a number
before enter goes to that page or tile.";

/// The config in effect, as a config: the template that `--generate-config` writes, with what
/// `options` give and what `keymaps` bind in place of the defaults. Made from the two catalogs of
/// this build: every option that a config may set, under its help lines, and every key that a
/// config can write, under what its command does.
///
/// A line that sets something is `# name = value` when it is the default, an example for the
/// reader to take the `# ` from and edit, and has no `# ` when the config or the command line set
/// it. A line of explanation starts with `##`. The three lines that
/// start the tables of the keys are as they are, since an empty table sets nothing. So with
/// nothing set, the file sets nothing, and one `# ` taken away sets one thing.
pub(super) fn written(catalog: &Catalog, options: &Parsed<'_>, settings: &[&'static Setting], keymaps: &Keymaps) -> String {
    let mut out = String::new();
    help(&mut out, ABOUT);
    out.push('\n');
    banner(&mut out, OPTIONS_TITLE);
    help(&mut out, ABOUT_OPTIONS);
    for entry in catalog.options().filter(|entry| entry.spec.config == Settable::Yes) {
        out.push('\n');
        // Each line of help it has: a setting that two backends declare has one from each.
        let mut said: Vec<&str> = Vec::new();
        for other in catalog.entries().iter().filter(|other| other.spec.name == entry.spec.name) {
            if !said.contains(&other.spec.help) && !other.spec.help.is_empty() {
                said.push(other.spec.help);
            }
        }
        said.iter().for_each(|said| help(&mut out, said));
        let takes = if entry.is_flag() { "true or false".to_owned() } else { entry.takes() };
        help(&mut out, &format!("Takes {takes}."));
        let (key, given) = (entry.spec.key(), options.has(entry.spec.name));
        let value = match (entry.is_flag(), options.value(entry.spec.name)) {
            (true, _) => given.to_string(),
            (false, Some(value)) => value_written(entry, value),
            (false, None) => value_written(entry, &entry.default().unwrap_or_default()),
        };
        out.push_str(&format!("{}{key} = {value}\n", if given { "" } else { "# " }));
    }
    out.push('\n');
    banner(&mut out, KEYS_TITLE);
    help(&mut out, ABOUT_KEYS);
    out.push('\n');
    out.push_str(&keys_written(settings, keymaps, &Keymaps::new(settings)));
    out
}

/// Into `out`, the banner that starts a part of the template: a rule of [`RULE`] `#`, the part's
/// `title`, and a rule, so that a reader sees where each part starts. A rule starts with `##`, so
/// it is a line of explanation as the file says, and no setting's line.
fn banner(out: &mut String, title: &str) {
    let rule = "#".repeat(RULE);
    out.push_str(&format!("{rule}\n"));
    help(out, title);
    out.push_str(&format!("{rule}\n"));
}

/// Into `out`, `text` as lines of explanation: each after `## `, and an empty one as `##` alone,
/// so that no line ends with a space.
fn help(out: &mut String, text: &str) {
    for line in text.lines() {
        out.push_str(if line.is_empty() { "##" } else { "## " });
        out.push_str(line);
        out.push('\n');
    }
}

/// What the tables of the keys say of a command in a keymap: a key bound to it (or to `none`,
/// under the command that had the key), and whether the config set that; or that no key is bound
/// to it, by its name.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Said {
    Bound { key: Key, to: String, set: bool },
    NoKey(String),
}

/// A [`Said`], with what `?` says of its command in its keymap.
struct Line {
    says: String,
    said: Said,
}

/// What the tables of the keys say of `mode`, in order: every key that a config can write, under
/// what its command does, in the groups that `?` lists the commands in. A key bound as it is by
/// default is not set. A key that the config binds is, and so is a key that it unbinds, written
/// `none` under the command that had it.
fn lines(mode: Mode, settings: &[&'static Setting], keymaps: &Keymaps, defaults: &Keymaps) -> Vec<Line> {
    let (now, default) = (keymaps.of(mode), defaults.of(mode));
    // `ctrl-c` always quits: no config writes it.
    let writable = |key: &Key| *key != Key::Interrupt;
    let mut lines = Vec::new();
    for (says, commands) in commands::groups(mode, settings, now) {
        for command in commands {
            let before = lines.len();
            for key in now.all(command).into_iter().filter(writable) {
                let said = Said::Bound { key, to: command.name(), set: default.alone(key) != Some(command) };
                lines.push(Line { says: says.clone(), said });
            }
            for key in default.all(command).into_iter().filter(writable) {
                if now.alone(key).is_none() {
                    lines.push(Line { says: says.clone(), said: Said::Bound { key, to: NONE.to_owned(), set: true } });
                }
            }
            if lines.len() == before {
                lines.push(Line { says: says.clone(), said: Said::NoKey(command.name()) });
            }
        }
    }
    lines
}

/// The three tables of the keys, as [`written`] writes them: `[keys]`, then one for each keymap.
///
/// A key that both keymaps bind alike is written once, under `[keys]`: the same key, the same
/// command or `none`, set or not in both. Every other key is under the table of its keymap. So a
/// key is in the file once where one line can say it (`q = "quit"`), and never both under `[keys]`
/// and under a keymap's table.
///
/// One thing keeps a key that both bind alike under the tables of the keymaps: the order of a
/// command's keys. clv reads the keys in the order of the file, and `[keys]` comes first. So of
/// the keys that the config binds to a command, only those that come first in both keymaps, in
/// the same order, go under `[keys]`. Then what is written reads back to the same keymaps.
///
/// The lines of the tables are never commented out: an empty table sets nothing, and a reader
/// who takes the `# ` from one key has bound that key.
fn keys_written(settings: &[&'static Setting], keymaps: &Keymaps, defaults: &Keymaps) -> String {
    let [pages, list] = [Mode::Pages, Mode::List].map(|mode| lines(mode, settings, keymaps, defaults));
    // The keys that the config binds to the command called `name`, in a keymap, in their order.
    let bound_again = |lines: &[Line], name: &str| -> Vec<Key> {
        let key = |line: &Line| match &line.said {
            Said::Bound { key, to, set: true } if to == name => Some(*key),
            _ => None,
        };
        lines.iter().filter_map(key).collect()
    };
    let alike = |said: &Said| match said {
        Said::Bound { key, to, set: true } if to != NONE => {
            let (ours, theirs) = (bound_again(&pages, to), bound_again(&list, to));
            let first = ours.iter().zip(&theirs).take_while(|(ours, theirs)| ours == theirs).count();
            ours.iter().take(first).any(|ours| ours == key)
        }
        said => pages.iter().any(|line| line.said == *said) && list.iter().any(|line| line.said == *said),
    };
    // Under `[keys]`, in the order of a file's pages, each under what `?` says in both keymaps.
    let both = pages.iter().filter(|line| alike(&line.said)).map(|line| {
        let other = list.iter().find(|other| other.said == line.said).map_or(line.says.as_str(), |other| other.says.as_str());
        let says = match other == line.says {
            true => vec![line.says.clone()],
            false => vec![format!("In the pages of a file: {}", line.says), format!("In a list: {other}")],
        };
        (says, &line.said)
    });
    // Under the table of a keymap, what is left, each under what `?` says there.
    fn only<'l>(lines: &'l [Line], alike: &dyn Fn(&Said) -> bool) -> Vec<(Vec<String>, &'l Said)> {
        lines.iter().filter(|line| !alike(&line.said)).map(|line| (vec![line.says.clone()], &line.said)).collect()
    }
    let mut out = table(BOTH_TITLE, KEYS.to_owned(), both.collect());
    for (((name, _), title), lines) in KEYMAPS.iter().zip(KEYMAP_TITLES).zip([&pages, &list]) {
        out.push('\n');
        out.push_str(&table(title, format!("{KEYS}.{name}"), only(lines, &alike)));
    }
    out
}

/// The table called `name` under the banner of its `title`, with `lines` in it, each under its
/// lines of help, said once for the lines that share them.
fn table(title: &str, name: String, lines: Vec<(Vec<String>, &Said)>) -> String {
    let mut out = String::new();
    banner(&mut out, title);
    out.push_str(&format!("[{name}]\n"));
    let mut last: Option<&Vec<String>> = None;
    for (says, said) in &lines {
        if last != Some(says) {
            out.push('\n');
            says.iter().for_each(|says| help(&mut out, says));
            last = Some(says);
        }
        match said {
            Said::Bound { key, to, set } => out.push_str(&format!("{}{} = \"{to}\"\n", if *set { "" } else { "# " }, key_written(&keys::name(*key)))),
            Said::NoKey(command) => help(&mut out, &format!("No key is bound to {command}.")),
        }
    }
    out
}

/// `value` of the option `entry` as a config writes it: a number as an integer, a named value as
/// a string. The number is the one that clv read ([`Entry::read`]), in base ten, never the text as
/// it was typed: `03` and `+3` are both 3 to clv, and TOML takes neither as that integer. A value
/// that the option doesn't take is written as a string, for clv to refuse when it reads it.
fn value_written(entry: &Entry, value: &str) -> String {
    match entry.read(value) {
        Ok(Arg::Number(number)) => number.to_string(),
        Ok(Arg::Named(named)) => quoted(named),
        _ => quoted(value),
    }
}

/// The name of a key as a config writes it: bare where it is letters, digits, `-` and `_` only,
/// and between quotes otherwise (`"/"`).
fn key_written(name: &str) -> String {
    let bare = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if bare { name.to_owned() } else { quoted(name) }
}

/// `text` as a TOML string, between quotes: `"` and `\` escaped, and a control character written
/// by its number.
fn quoted(text: &str) -> String {
    let mut out = String::from('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Where `--generate-config` writes: the file `--config` names, or else the reader's own. Nowhere,
/// with no home to put one in.
pub(super) fn place(which: Which<'_>) -> Option<PathBuf> {
    match which {
        Which::File(path) => Some(PathBuf::from(path)),
        Which::Own | Which::None => own(std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from), std::env::var_os("HOME").map(PathBuf::from)),
    }
}

/// `path` whole, from the directory clv runs in where it is not: what `--generate-config` prints of
/// the file it wrote, so that the reader sees where the file is. The path as it is, where there is
/// no such directory to say.
pub(super) fn whole(path: &Path) -> PathBuf {
    match std::env::current_dir() {
        // Joined part by part, which leaves out a `.` on the way.
        Ok(here) if path.is_relative() => here.join(path).components().collect(),
        _ => path.to_path_buf(),
    }
}

/// Writes `template` to `path`, a config that is not there yet, and makes the directory when there
/// is none. It never writes over a file: one that is there is the error, which names it. The only
/// time clv writes a config.
pub(super) fn generate(path: &Path, template: &str) -> Result<(), String> {
    use std::io::Write;
    let named = path.display();
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("{named}: can't make its directory ({})", e.kind()))?;
    }
    // Made only if nothing is there: a file, a directory or a link, even one that leads nowhere.
    let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(format!("{named} is there already: {} never writes over a config", crate::NAME));
        }
        Err(e) => return Err(format!("{named}: can't write it ({})", e.kind())),
    };
    file.write_all(template.as_bytes()).map_err(|e| format!("{named}: can't write it ({})", e.kind()))
}

/// `wrong` in `file`, as the reader is told: `~/.config/clv/config.toml:3: width: expected window
/// or 20–200, got 300`.
fn said(file: &str, wrong: &Wrong) -> String {
    match wrong.line {
        Some(line) => format!("{file}:{line}: {}", wrong.why),
        None => format!("{file}: {}", wrong.why),
    }
}

/// The reader's own config, from what `XDG_CONFIG_HOME` and `HOME` hold: under the first where it
/// is an absolute path, under `.config` in the second otherwise. Nowhere, with neither.
fn own(xdg: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    let configs = match xdg.filter(|dir| dir.is_absolute()) {
        Some(dir) => dir,
        None => home.filter(|home| !home.as_os_str().is_empty())?.join(".config"),
    };
    Some(configs.join(crate::NAME).join(FILE))
}

/// `path` as a message names it: with `~` for the reader's home, where it is under it.
fn shown(path: &Path, home: Option<&Path>) -> String {
    match home.filter(|home| home.is_absolute()).and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// The text of the config at `path`, or `None` where there is no file and none `must` be there.
/// A file that is not a regular file, one larger than [`MAX_SIZE`], and one that can't be read
/// (or is not UTF-8) are refused, with why.
fn load(path: &Path, must: bool) -> Result<Option<String>, String> {
    use std::io::ErrorKind::{NotADirectory, NotFound};
    let unread = |e: std::io::Error| format!("can't read it ({})", e.kind());
    // Asked before it is opened: to open a pipe that nothing writes to would wait for ever.
    match std::fs::metadata(path) {
        Ok(metadata) if !metadata.is_file() => return Err("not a regular file".into()),
        Ok(_) => {}
        Err(e) if !must && matches!(e.kind(), NotFound | NotADirectory) => return Ok(None),
        Err(e) if e.kind() == NotFound => return Err("no such file".into()),
        Err(e) => return Err(unread(e)),
    }
    let file = std::fs::File::open(path).map_err(unread)?;
    // And asked again of the file as opened, so what is checked is what is read.
    let metadata = file.metadata().map_err(unread)?;
    if !metadata.is_file() {
        return Err("not a regular file".into());
    }
    let too_large = || format!("larger than {} MiB, the most of a config that {} reads", MAX_SIZE >> 20, crate::NAME);
    if metadata.len() > MAX_SIZE {
        return Err(too_large());
    }
    // The size is checked again as it is read: a file can grow.
    let mut bytes = Vec::new();
    file.take(MAX_SIZE.saturating_add(1)).read_to_end(&mut bytes).map_err(unread)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SIZE {
        return Err(too_large());
    }
    String::from_utf8(bytes).map(Some).map_err(|_| "not UTF-8 text".into())
}

/// The line of `text` that the byte at `offset` is on, from 1.
fn line(text: &str, offset: usize) -> usize {
    let before = text.as_bytes().get(..offset).unwrap_or(text.as_bytes());
    before.iter().filter(|byte| **byte == b'\n').count().saturating_add(1)
}

/// The option that a config calls `name`: its name in the catalog without the dashes. An unknown
/// name, an option that a config may not set and an option's old name are refused, with why.
fn option<'c>(catalog: &'c Catalog, name: &str) -> Result<&'c Entry, String> {
    if let Some(entry) = catalog.options().find(|entry| entry.spec.key() == name) {
        return match entry.spec.config {
            Settable::Yes => Ok(entry),
            Settable::No(why) => Err(format!("{name}: a config can't set it: {why}")),
        };
    }
    match catalog.options().find(|entry| entry.spec.alias.is_some_and(|alias| alias.trim_start_matches('-') == name)) {
        Some(entry) => Err(format!("{name} is now {new}: write {new}", new = entry.spec.key())),
        None => Err(unknown(name)),
    }
}

/// What a name that is no option's is told.
fn unknown(name: &str) -> String {
    format!("unknown option {name}")
}

/// What `value`, written `written` in the file, gives the option `entry`: `None` for a flag set
/// to `false`, which is a flag not given; a flag's `None`, or the value as the command line would
/// give it, read by the parser of the command line's, so that the two say the same of a bad one.
/// A flag is a boolean, a number is an integer or a string, and a named value is a string.
fn given(entry: &Entry, value: &DeValue<'_>, written: &str) -> Result<Option<Option<String>>, String> {
    // What a value of the wrong kind is said as: a short one as it is written.
    let got = || match value {
        DeValue::Array(_) => "an array".to_owned(),
        DeValue::Table(_) => "a table".to_owned(),
        _ => written.chars().take(40).collect(),
    };
    if entry.is_flag() {
        return match value {
            DeValue::Boolean(true) => Ok(Some(None)),
            DeValue::Boolean(false) => Ok(None),
            _ => Err(format!("expected true or false, got {}", got())),
        };
    }
    let text = match value {
        DeValue::String(text) => text.to_string(),
        // In base ten, as the command line gives a number.
        DeValue::Integer(integer) => match i64::from_str_radix(integer.as_str(), integer.radix()) {
            Ok(number) => number.to_string(),
            Err(_) => got(),
        },
        _ => return Err(format!("expected {}, got {}", entry.takes(), got())),
    };
    entry.read(&text)?;
    Ok(Some(Some(text)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::{OptionSpec, Shape};
    use crate::source::tests::cycled;
    use crate::source::{Named, Numbers, Setting};
    use crate::view::commands::Command;

    const fn option(name: &'static str, value: Option<&'static str>, shape: Shape) -> OptionSpec {
        OptionSpec { name, short: None, value, help: "", alias: None, shape, config: Settable::Yes }
    }

    static WIDTH: Setting = Setting {
        named: &[Named { name: "window", says: "" }],
        numbers: Some(Numbers { least: 20, most: 200, step: 4, unit: "" }),
        default: crate::source::Value::Named("window"),
        auto: None,
        keys: crate::source::Keys::Step('[', ']'),
        ..cycled("width", '[', "--width")
    };
    static SETTINGS: [Setting; 2] = [cycled("theme", 'D', "--theme"), WIDTH];

    /// A catalog as a build's: a count with an old name, flags, a page list and a password that a
    /// config may not set, and two settings.
    fn catalog() -> Catalog {
        const COUNT: Shape = Shape::Value { named: &["auto", "width"], numbers: Some(Numbers { least: 1, most: 8, step: 1, unit: "" }) };
        let viewer = [
            OptionSpec { alias: Some("--zoom"), ..option("--per-row", Some("N"), COUNT) },
            option("--recurse", None, Shape::Flag),
            option("--pager", None, Shape::Flag),
            OptionSpec { config: Settable::No("it belongs to one run"), ..option("--pages", Some("1,3-5"), Shape::Pages) },
        ];
        let password = OptionSpec { config: Settable::No("a secret"), ..option("--password", Some("PW"), Shape::Text) };
        let mut catalog = Catalog::of(&viewer);
        catalog.declare("documents", &[password], &SETTINGS);
        catalog
    }

    /// What a config sets: each option's name, value and line.
    type Sets = Vec<(&'static str, Option<String>, usize)>;

    /// What `text` sets, as a config.
    fn sets(text: &str) -> Result<Sets, Wrong> {
        Config::parse(text, &catalog()).map(|config| config.options.into_iter().map(|set| (set.name, set.value, set.line)).collect())
    }

    /// The top level of a config sets options by their names without the dashes, in the order of
    /// the file: a number as an integer or a string, a named value as a string, a flag as a boolean.
    #[test]
    fn a_config_sets_options_by_their_names_in_the_order_of_the_file() {
        let text = "# Mine.\nwidth = 72\ntheme = \"dark\"\n\nrecurse = true\nper-row = \"3\"\npager = false\n";
        let value = |text: &str| Some(text.to_owned());
        assert_eq!(
            sets(text),
            Ok(vec![("--width", value("72"), 2), ("--theme", value("dark"), 3), ("--recurse", None, 5), ("--per-row", value("3"), 6)])
        );
        assert_eq!(sets(""), Ok(Vec::new()));
        assert_eq!(sets("\n# Only this.\n"), Ok(Vec::new()));
        // An integer in another base, or with its digits apart, is the number it is.
        assert_eq!(sets("width = 0x48"), Ok(vec![("--width", value("72"), 1)]));
        assert_eq!(sets("width = 1_0_0"), Ok(vec![("--width", value("100"), 1)]));
        assert_eq!(sets("width = \"window\"\nper-row = \"width\""), Ok(vec![("--width", value("window"), 1), ("--per-row", value("width"), 2)]));
        // A dotted key or a table of the name is no option.
        assert!(sets("width.most = 3").is_err() && sets("[width]\nmost = 3").is_err());
    }

    /// Whatever is wrong says its line, and what: a value the option doesn't take, as the command
    /// line says it; an unknown name; an option a config may not set; an old name; bad TOML.
    #[test]
    fn what_is_wrong_with_a_config_says_its_line_and_why() {
        for (text, line, why) in [
            ("theme = \"dark\"\n\nwidth = 300\n", Some(3), "width: expected window or 20–200, got 300"),
            ("width = \"300\"", Some(1), "width: expected window or 20–200, got 300"),
            ("width = -4", Some(1), "width: expected window or 20–200, got -4"),
            ("width = 72.5", Some(1), "width: expected window or 20–200, got 72.5"),
            ("width = true", Some(1), "width: expected window or 20–200, got true"),
            ("width = [72]", Some(1), "width: expected window or 20–200, got an array"),
            ("width = { most = 3 }", Some(1), "width: expected window or 20–200, got a table"),
            ("width = 99999999999999999999999", Some(1), "width: expected window or 20–200, got 99999999999999999999999"),
            ("theme = \"sepia\"", Some(1), "theme: expected auto, light or dark, got sepia"),
            ("theme = 3", Some(1), "theme: expected auto, light or dark, got 3"),
            ("per-row = 9", Some(1), "per-row: expected auto, width or 1–8, got 9"),
            ("recurse = \"yes\"", Some(1), "recurse: expected true or false, got \"yes\""),
            ("recurse = 1", Some(1), "recurse: expected true or false, got 1"),
            ("\n\nfrobnicate = 1", Some(3), "unknown option frobnicate"),
            ("--width = 72", None, ""),
            ("\"--width\" = 72", Some(1), "unknown option --width"),
            ("Width = 72", Some(1), "unknown option Width"),
            ("pages = \"1,3\"", Some(1), "pages: a config can't set it: it belongs to one run"),
            ("\npassword = \"x\"", Some(2), "password: a config can't set it: a secret"),
            ("zoom = 2", Some(1), "zoom is now per-row: write per-row"),
            ("width = 72\nwidth = 80", Some(2), ""),
            ("theme = ", Some(1), ""),
            ("theme = \"dark\nwidth = 72", Some(1), ""),
        ] {
            let wrong = sets(text).unwrap_err();
            assert!(line.is_none() || wrong.line == line, "{text:?}: {wrong:?}");
            assert!(why.is_empty() || wrong.why == why, "{text:?}: {wrong:?}");
            assert!(!wrong.why.is_empty(), "{text:?}");
        }
        // A long value of the wrong kind is said in part.
        let long = format!("recurse = {}", "9".repeat(5_000));
        assert!(sets(&long).unwrap_err().why.len() < 100);
        // The reader is told the file, and the line where there is one.
        assert_eq!(said("~/.config/clv/config.toml", &Wrong::at(3, "width: no")), "~/.config/clv/config.toml:3: width: no");
        assert_eq!(said("c.toml", &Wrong { line: None, why: "no".into() }), "c.toml: no");
    }

    /// `[keys]`, `[keys.pages]` and `[keys.list]` bind keys to commands by name, as written: in
    /// the order of the file, whichever table each is in. What is no name of a command is said.
    #[test]
    fn the_tables_of_keys_bind_keys_to_commands_as_written_in_the_order_of_the_file() {
        let text = "width = 72\n[keys.list]\nJ = \"list-tile-down\"\n[keys]\nctrl-d = \"screen-down\"\nx = \"none\"\n\"/\" = \"search\"\n[keys.pages]\nenter = \"next\"\n";
        let config = Config::parse(text, &catalog()).unwrap();
        let bound: Vec<(Option<Mode>, &str, &str, usize)> =
            config.keys.iter().map(|b| (b.mode, b.key.as_str(), b.command.as_str(), b.line)).collect();
        assert_eq!(
            bound,
            [
                (Some(Mode::List), "J", "list-tile-down", 3),
                (None, "ctrl-d", "screen-down", 5),
                (None, "x", "none", 6),
                (None, "/", "search", 7),
                (Some(Mode::Pages), "enter", "next", 9)
            ]
        );
        assert_eq!(config.options.len(), 1, "the options beside them");
        // An inline table is a table too, and two on a line keep their order.
        let config = Config::parse("keys = { b = \"quit\", a = \"none\", list = { z = \"list-open\" } }", &catalog()).unwrap();
        assert_eq!(config.keys.iter().map(|b| (b.key.as_str(), b.mode)).collect::<Vec<_>>(), [("b", None), ("a", None), ("z", Some(Mode::List))]);
        assert_eq!(Config::parse("[keys]\n[keys.pages]\n[keys.list]\n", &catalog()).map(|config| config.keys.len()), Ok(0));
        for (text, line, why) in [
            ("keys = 3", 1, "keys: expected a table of keys"),
            ("keys = \"x\"", 1, "keys: expected a table of keys"),
            ("[keys]\nx = 3", 2, "x: expected the name of a command or \"none\", got 3"),
            ("[keys]\nx = true", 2, "x: expected the name of a command or \"none\", got true"),
            ("[keys]\n\nx = [\"quit\"]", 3, "x: expected the name of a command or \"none\", got an array"),
            ("[keys.other]\nx = \"quit\"", 1, "other: expected the name of a command or \"none\", got a table"),
            ("[keys.pages.more]\nx = \"quit\"", 1, "more: expected the name of a command or \"none\", got a table"),
            ("[keys.list]\nJ = { x = \"quit\" }", 2, "J: expected the name of a command or \"none\", got a table"),
        ] {
            assert_eq!(Config::parse(text, &catalog()).unwrap_err(), Wrong::at(line, why), "{text:?}");
        }
    }

    /// The options that `config` sets, as given.
    fn options(config: &Config) -> Parsed<'_> {
        let mut options = Parsed::default();
        config.options.iter().for_each(|set| options.configure(set.name, set.value.as_deref()));
        options
    }

    /// The three tables of the keys in `text`, a config as written: `[keys]`, then the two keymaps'.
    fn tables(text: &str) -> [String; 3] {
        let after = |header: &str| text.split_once(&format!("\n{header}\n")).map_or("", |(_, after)| after);
        let until = |text: &str, header: &str| text.split_once(&format!("\n{header}\n")).map_or(text, |(before, _)| before).to_owned();
        [until(after("[keys]"), "[keys.pages]"), until(after("[keys.pages]"), "[keys.list]"), after("[keys.list]").to_owned()]
    }

    /// The keys of each command in each keymap, in their order, and those of them that `?` lists:
    /// what two keymaps must share to be the same to a reader. A file groups its keys by command, so
    /// only the order of the bindings among the commands may differ, which nothing shows.
    fn keys_of(keymaps: &Keymaps, settings: &[&'static Setting]) -> [Vec<(String, Vec<Key>, String)>; 2] {
        [Mode::Pages, Mode::List].map(|mode| {
            let commands = commands::groups(mode, settings, keymaps.of(mode)).into_iter().flat_map(|(_, commands)| commands);
            commands.map(|command| (command.name(), keymaps.of(mode).all(command), keymaps.of(mode).each(&[command]))).collect()
        })
    }

    /// The template sets nothing: every line that sets something is commented out, under its help,
    /// and the lines of the three tables of the keys are as they are. A key that both keymaps bind
    /// alike is written once, under `[keys]`, with what `?` says of it in each; any other key is
    /// under the table of its keymap.
    #[test]
    fn the_template_writes_a_key_that_both_keymaps_bind_alike_once_under_keys() {
        let (catalog, settings): (Catalog, [&'static Setting; 2]) = (catalog(), [&SETTINGS[0], &SETTINGS[1]]);
        let template = written(&catalog, &Parsed::default(), &settings, &Keymaps::new(&settings));
        let headers = ["[keys]", "[keys.pages]", "[keys.list]"];
        let sets_nothing = |line: &str| line.is_empty() || line.starts_with("##") || line.starts_with("# ") || headers.contains(&line);
        assert!(template.lines().all(sets_nothing), "{template}");
        // It starts by saying how to read it: an explanation after `##`, an example after one `#`.
        let top = "## The config of clv. clv reads this file when it starts, and never changes it.\n##\n\
                   ## A line that starts with \"##\" is an explanation.\n\
                   ## A line that starts with one \"#\" is an example: it shows the default.\n\
                   ## To change a setting, remove the \"# \" from its line and edit the value.\n\
                   ## A line that keeps its \"#\" changes nothing.\n\n";
        assert!(template.starts_with(top), "{template}");
        // An explanation's empty line is `##` alone, and no line ends with a space.
        assert!(template.lines().any(|line| line == "##") && !template.lines().any(|line| line.ends_with(' ')), "{template}");
        let intro = "## Each line binds a key to a command: key = \"command\".\n\
                    ## Write \"none\" in place of the command to unbind the key.\n";
        let brackets = "##\n## The three lines in brackets start the tables of the keys. Leave them as they are.\n\
                      ##   [keys]        binds a key in the pages of a file and in a list\n\
                      ##   [keys.pages]  binds a key in the pages of a file only\n\
                      ##   [keys.list]   binds a key in a list only\n##\n\
                      ## A config cannot change two bindings: ctrl-c always quits, and a number\n\
                      ## before enter goes to that page or tile.\n\n";
        assert!(template.contains(intro) && template.contains(brackets), "{template}");
        // Each of its five parts starts with a banner: a rule of 78 `#`, its title, a rule. A rule
        // starts with `##`, so it is an explanation, and stays when the `# ` are taken away.
        let rule = "#".repeat(78);
        for (title, after) in [
            ("Options", "## An option on the command line wins over the same option here.\n\n"),
            ("Keys", "## Each line binds a key to a command: key = \"command\".\n"),
            ("Keys in the pages of a file and in a list", "[keys]\n\n## "),
            ("Keys in the pages of a file only", "[keys.pages]\n\n## "),
            ("Keys in a list only", "[keys.list]\n\n## "),
        ] {
            let banner = format!("\n\n{rule}\n## {title}\n{rule}\n{after}");
            assert_eq!(template.matches(&banner).count(), 1, "{title}\n{template}");
        }
        assert_eq!(template.lines().filter(|line| *line == rule).count(), 10, "{template}");
        assert!(template.starts_with(&format!("{top}{rule}\n## Options\n")), "{template}");
        let uncommented: String = template.lines().map(|line| format!("{}\n", line.strip_prefix("# ").unwrap_or(line))).collect();
        assert_eq!(uncommented.lines().filter(|line| *line == rule).count(), 10, "a rule is no setting's line");
        let once = |line: &str| template.lines().filter(|other| *other == line).count() == 1;
        for line in ["# per-row = \"auto\"", "# recurse = false", "# theme = \"auto\"", "# width = \"window\"", "## Takes window or 20–200."] {
            assert!(once(line), "{line}\n{template}");
        }
        // The three tables, in that order, each once, and never commented out.
        assert!(headers.iter().all(|header| once(header)) && !template.lines().any(|line| line.starts_with("# [")), "{template}");
        // What a config may not set isn't in it.
        assert!(!template.contains("pages =") && !template.contains("password"), "{template}");
        let [both, pages, list] = tables(&template);
        // Alike in both keymaps: once, under `[keys]`, one that `?` doesn't list and a setting's too.
        for line in [
            "# q = \"quit\"",
            "# space = \"screen-down\"",
            "# \"=\" = \"zoom-in\"",
            "# D = \"cycle-theme\"",
            "# \"[\" = \"decrease-width\"",
            "# esc = \"leave\"",
        ] {
            assert!(once(line) && both.lines().any(|other| other == line), "{line}\n{template}");
        }
        // Bound otherwise in the two, or in one alone: under the table of each.
        for (line, other) in [
            ("# j = \"scroll-down\"", "# j = \"list-tile-down\""),
            ("# enter = \"scroll-down\"", "# enter = \"list-open\""),
            ("# \"/\" = \"search\"", "# \"/\" = \"list-filter\""),
            ("# n = \"next\"", "# n = \"screen-down\""),
        ] {
            assert!(once(line) && once(other) && pages.lines().any(|l| l == line) && list.lines().any(|l| l == other), "{line}\n{template}");
        }
        assert!(
            pages.contains("# f = \"follow\"") && list.contains("# h = \"list-tile-left\"") && !both.contains("list-") && !both.contains("search")
        );
        // Under `[keys]`, what `?` says in each keymap, where the two differ.
        assert!(
            both.contains("\n## In the pages of a file: the first or the last page\n## In a list: the first or the last\n# g = \"go-to\"\n"),
            "{both}"
        );
        assert!(both.contains("\n## quit\n# q = \"quit\"\n") && pages.contains("\n## a few rows down or up, or a row of pages\n# j = "), "{both}");
        // No key twice: under `[keys]`, or under the keymaps' tables.
        let keys = |table: &str| {
            table
                .lines()
                .filter(|line| line.starts_with("# "))
                .filter_map(|line| line.split_once(" = "))
                .map(|(key, _)| key.to_owned())
                .collect::<Vec<_>>()
        };
        assert!(keys(&both).iter().all(|key| !keys(&pages).contains(key) && !keys(&list).contains(key)), "{template}");
        assert!(!template.contains("ctrl-c =") && template.contains("ctrl-c always quits"), "{template}");
        assert!(!template.contains("enter = \"go-to\""), "a number and Enter is no config's");
    }

    /// The config in effect is written as a config: the template, with what the options give and
    /// what the keymaps bind in place of the defaults, not commented out. A key that the config
    /// binds or unbinds in both keymaps alike is under `[keys]`, once. Read back, the file gives
    /// the same options and the same keymaps.
    #[test]
    fn the_config_in_effect_is_written_as_a_config_that_reads_back_as_itself() {
        let (catalog, settings): (Catalog, [&'static Setting; 2]) = (catalog(), [&SETTINGS[0], &SETTINGS[1]]);
        let defaults = Keymaps::new(&settings);
        let text = "width = 72\ntheme = \"dark\"\nrecurse = true\n[keys]\nx = \"quit\"\nq = \"none\"\nD = \"quit\"\n[keys.list]\nJ = \"list-tile-down\"\n[keys.pages]\nenter = \"next\"\n";
        let config = Config::parse(text, &catalog).unwrap();
        let keymaps = Keymaps::bound(&settings, &config.keys).unwrap();
        let effective = written(&catalog, &options(&config), &settings, &keymaps);
        for line in ["width = 72", "theme = \"dark\"", "recurse = true", "# per-row = \"auto\"", "[keys]", "[keys.pages]", "[keys.list]"] {
            assert_eq!(effective.lines().filter(|other| *other == line).count(), 1, "{line}\n{effective}");
        }
        let [both, pages, list] = tables(&effective);
        // Bound alike in both: under `[keys]`, first under their command, an unbound key under the
        // command that had it, and a command left with no key said.
        assert!(both.contains("\n## quit\nx = \"quit\"\nD = \"quit\"\nq = \"none\"\n"), "{both}");
        assert!(both.contains("## No key is bound to cycle-theme.") && !pages.contains("quit") && !list.contains("quit"), "{effective}");
        // Bound in one keymap: under its table, and gone from under the command that had it there.
        assert!(pages.contains("\nenter = \"next\"\n# n = \"next\"\n") && !pages.contains("enter = \"scroll-down\""), "{pages}");
        assert!(list.contains("\nJ = \"list-tile-down\"\n# down = \"list-tile-down\"\n") && list.contains("# enter = \"list-open\""), "{list}");
        assert!(!pages.contains("J =") && !both.contains("J =") && !both.contains("enter ="), "{effective}");
        // Read back, it is the config it was written from.
        let again = Config::parse(&effective, &catalog).unwrap();
        let said = |config: &Config| config.options.iter().map(|set| (set.name, set.value.clone())).collect::<Vec<_>>();
        assert_eq!(said(&again), [("--recurse", None), ("--theme", Some("dark".into())), ("--width", Some("72".into()))]);
        let read = Keymaps::bound(&settings, &again.keys).unwrap();
        assert_eq!(keys_of(&read, &settings), keys_of(&keymaps, &settings));
        assert_ne!(keys_of(&read, &settings), keys_of(&defaults, &settings));
        assert_eq!(written(&catalog, &options(&again), &settings, &read), effective, "written again, it is the same file");
    }

    /// A command's keys keep their order through the file. `[keys]` comes first, so of the keys
    /// that the config binds to a command, only those that come first in both keymaps go there:
    /// one that is behind a key of one keymap alone stays under the two tables.
    #[test]
    fn a_key_goes_under_keys_only_where_that_keeps_the_order_of_its_commands_keys() {
        let (catalog, settings): (Catalog, [&'static Setting; 2]) = (catalog(), [&SETTINGS[0], &SETTINGS[1]]);
        // `a` quits in a file's pages and opens a tile in a list; `b` quits in both, after `a`.
        let text = "[keys]\na = \"quit\"\nb = \"quit\"\nz = \"copy\"\n[keys.list]\na = \"list-open\"\n";
        let keymaps = Keymaps::bound(&settings, &Config::parse(text, &catalog).unwrap().keys).unwrap();
        assert_eq!(
            (keymaps.of(Mode::Pages).each(&[Command::Quit]), keymaps.of(Mode::List).each(&[Command::Quit])),
            ("a  b  q  ctrl-c".to_owned(), "b  q  ctrl-c".to_owned())
        );
        let effective = written(&catalog, &Parsed::default(), &settings, &keymaps);
        let [both, pages, list] = tables(&effective);
        // `b` is behind `a` in a file's pages: under `[keys]` it would come before it.
        let has = |table: &str, line: &str| table.lines().any(|other| other == line);
        assert!(!has(&both, "b = \"quit\"") && !has(&both, "a = \"quit\""), "{both}");
        assert!(pages.contains("\n## quit\na = \"quit\"\nb = \"quit\"\n"), "{pages}");
        assert!(list.contains("\n## quit\nb = \"quit\"\n") && has(&list, "a = \"list-open\""), "{list}");
        // The default's key, which both still have, is under `[keys]`: its place tells no order.
        assert!(both.contains("\n## quit\n# q = \"quit\"\n"), "{both}");
        // A key that comes first in both goes under `[keys]`, before the default's.
        assert!(both.contains("\nz = \"copy\"\n# c = \"copy\"\n") && !has(&pages, "z = \"copy\""), "{both}");
        let read = Keymaps::bound(&settings, &Config::parse(&effective, &catalog).unwrap().keys).unwrap();
        assert_eq!(keys_of(&read, &settings), keys_of(&keymaps, &settings));
    }

    /// A line that is in the wrong part of the file says where it goes: a key bound above the
    /// tables, and an option under a table of keys.
    #[test]
    fn a_line_in_the_wrong_part_of_the_file_says_where_it_goes() {
        let wrong = |text: &str| Config::parse(text, &catalog()).unwrap_err();
        for (text, line, why) in [
            ("x = \"quit\"\n[keys]\n", 1, "unknown option x: x is a key, and a key is bound under [keys]"),
            ("width = 72\nctrl-d = \"screen-down\"\n", 2, "unknown option ctrl-d: ctrl-d is a key, and a key is bound under [keys]"),
            ("\"/\" = \"none\"\n", 1, "unknown option /: / is a key, and a key is bound under [keys]"),
            // No key, or no binding: an unknown option, as before.
            ("frobnicate = \"quit\"\n", 1, "unknown option frobnicate"),
            ("x = 3\n", 1, "unknown option x"),
            ("Enter = \"next\"\n", 1, "unknown option Enter"),
            // An option's line below a table of keys, whatever its value.
            ("[keys]\nwidth = 72\n", 2, "width is an option, not a key: write it above [keys], before the first table"),
            (
                "[keys.list]\nJ = \"list-tile-down\"\n\ntheme = \"dark\"\n",
                4,
                "theme is an option, not a key: write it above [keys], before the first table",
            ),
            ("[keys.pages]\nrecurse = true\n", 2, "recurse is an option, not a key: write it above [keys], before the first table"),
        ] {
            assert_eq!(wrong(text), Wrong::at(line, why), "{text:?}");
        }
        // The tables of the keymaps are tables still, though `pages` is an option's name too; and
        // `pages` bound as a key under `[keys]` is left for the keymaps to refuse, as no key's name.
        assert_eq!(Config::parse("[keys.pages]\nx = \"quit\"\n", &catalog()).map(|config| config.keys.len()), Ok(1));
        assert_eq!(Config::parse("[keys]\npages = \"quit\"\n", &catalog()).map(|config| config.keys[0].key.clone()), Ok("pages".to_owned()));
    }

    /// The path of a file is said whole, from the directory clv runs in where it is not, with no
    /// `.` on the way; a whole path is said as it is.
    #[test]
    fn a_path_is_said_whole_from_the_directory_clv_runs_in() {
        let here = std::env::current_dir().unwrap();
        assert_eq!(whole(Path::new("mine.toml")), here.join("mine.toml"));
        assert_eq!(whole(Path::new("./sub/./mine.toml")), here.join("sub/mine.toml"));
        assert_eq!(whole(Path::new("/x/./mine.toml")), PathBuf::from("/x/./mine.toml"));
        assert!(whole(Path::new("../mine.toml")).is_absolute());
    }

    /// A number is written as clv read it, in base ten, never as it was typed: a leading zero or a
    /// sign would be no TOML integer, or another one. A named value is written between quotes.
    #[test]
    fn a_value_is_written_as_it_was_read_not_as_it_was_typed() {
        let (catalog, settings): (Catalog, [&'static Setting; 2]) = (catalog(), [&SETTINGS[0], &SETTINGS[1]]);
        let keymaps = Keymaps::new(&settings);
        for (count, width) in [("3", "72"), ("03", "072"), ("+3", "+72"), ("0003", "00072")] {
            let mut options = Parsed::default();
            options.configure("--per-row", Some(count));
            options.configure("--width", Some(width));
            options.configure("--theme", Some("dark"));
            let text = written(&catalog, &options, &settings, &keymaps);
            for line in ["per-row = 3", "width = 72", "theme = \"dark\""] {
                assert_eq!(text.lines().filter(|other| *other == line).count(), 1, "{count} {width}: {line}\n{text}");
            }
            let read: Vec<(&str, Option<String>)> =
                Config::parse(&text, &catalog).unwrap().options.into_iter().map(|set| (set.name, set.value)).collect();
            assert_eq!(read, [("--per-row", Some("3".into())), ("--theme", Some("dark".into())), ("--width", Some("72".into()))]);
        }
    }

    /// A key and a value are written so that TOML reads them back: a key bare where it may be, and
    /// what is between quotes escaped.
    #[test]
    fn a_key_and_a_value_are_written_as_toml_reads_them() {
        assert_eq!(["ctrl-d", "J", "5", "alt-_"].map(key_written), ["ctrl-d", "J", "5", "alt-_"]);
        assert_eq!(
            ["/", "\"", "\\", "é", "alt-é", "", "a b"].map(key_written),
            ["\"/\"", "\"\\\"\"", "\"\\\\\"", "\"é\"", "\"alt-é\"", "\"\"", "\"a b\""]
        );
        let catalog = catalog();
        let width = catalog.entry("--width").unwrap();
        let written = ["72", "072", "+72", "window", "", "-4", "300", "a\u{1b}\"b"].map(|value| value_written(width, value));
        assert_eq!(written, ["72", "72", "72", "\"window\"", "\"\"", "\"-4\"", "\"300\"", "\"a\\u001B\\\"b\""]);
        for key in ["/", "\"", "\\", "é", "'", "#", "=", "[", "]", ".", " x"] {
            let text = format!("[keys]\n{} = \"quit\"\n", key_written(key));
            assert_eq!(Config::parse(&text, &catalog).map(|config| config.keys[0].key.clone()), Ok(key.to_owned()), "{text}");
        }
    }

    /// A config is written once, where none is, its directory made; and never over what is there:
    /// a file, a directory, or a link that leads nowhere.
    #[test]
    fn a_config_is_written_once_and_never_over_what_is_there() {
        let dir = std::env::temp_dir().join(format!("clview-config-generate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("deep/er/config.toml");
        assert_eq!(generate(&path, "# width = 72\n"), Ok(()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# width = 72\n");
        let there = format!("{} is there already: clv never writes over a config", path.display());
        assert_eq!(generate(&path, "other"), Err(there));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# width = 72\n", "left as it was");
        assert!(generate(&dir.join("deep"), "x").is_err_and(|why| why.contains("is there already")), "a directory");
        let link = dir.join("link.toml");
        std::os::unix::fs::symlink(dir.join("nowhere"), &link).unwrap();
        assert!(generate(&link, "x").is_err_and(|why| why.contains("is there already")), "a link that leads nowhere");
        assert!(!dir.join("nowhere").exists());
        // Under a file, where a directory is needed: said, and nothing written.
        assert!(generate(&path.join("under/config.toml"), "x").is_err_and(|why| why.contains("can't make its directory")));
        // Where it goes: the file named, or the reader's own.
        assert_eq!(place(Which::File("x/c.toml")), Some(PathBuf::from("x/c.toml")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The reader's own config is under `XDG_CONFIG_HOME` where that is an absolute path, and under
    /// `.config` in the home otherwise, on macOS too; a message names it from the home.
    #[test]
    fn the_readers_own_config_is_under_xdg_config_home_or_the_homes_config() {
        let path = |text: &str| Some(PathBuf::from(text));
        assert_eq!(own(path("/x/cfg"), path("/home/me")), path("/x/cfg/clv/config.toml"));
        assert_eq!(own(None, path("/home/me")), path("/home/me/.config/clv/config.toml"));
        assert_eq!(own(path("relative/cfg"), path("/home/me")), path("/home/me/.config/clv/config.toml"), "not an absolute path");
        assert_eq!(own(path(""), path("/home/me")), path("/home/me/.config/clv/config.toml"));
        assert_eq!((own(None, None), own(path("cfg"), path(""))), (None, None));
        assert_eq!(shown(Path::new("/home/me/.config/clv/config.toml"), Some(Path::new("/home/me"))), "~/.config/clv/config.toml");
        assert_eq!(shown(Path::new("/x/cfg/clv/config.toml"), Some(Path::new("/home/me"))), "/x/cfg/clv/config.toml");
        assert_eq!(shown(Path::new("c.toml"), None), "c.toml");
        assert_eq!(shown(Path::new("me/c.toml"), Some(Path::new("me"))), "me/c.toml", "a home that is no absolute path is no home");
    }

    /// A file that is not there is no config, unless it was asked for by name. A directory, a file
    /// past the size clv reads and text that is not UTF-8 are refused, with why.
    #[test]
    fn a_config_is_a_regular_file_within_its_size_or_it_is_refused() {
        let dir = std::env::temp_dir().join(format!("clview-config-load-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = |name: &str, bytes: &[u8]| {
            std::fs::write(dir.join(name), bytes).unwrap();
            dir.join(name)
        };
        assert_eq!(load(&dir.join("none.toml"), false), Ok(None));
        assert_eq!(load(&dir.join("none.toml"), true), Err("no such file".into()));
        // Under a file, where a directory was meant: no config either.
        let plain = file("plain", b"width = 72\n");
        assert_eq!(load(&plain.join("clv/config.toml"), false), Ok(None));
        assert_eq!(load(&plain, true), Ok(Some("width = 72\n".into())));
        assert_eq!(load(&dir, true), Err("not a regular file".into()));
        assert_eq!(load(&dir, false), Err("not a regular file".into()));
        assert_eq!(load(&file("latin", b"theme = \"\xe9\""), true), Err("not UTF-8 text".into()));
        let most = usize::try_from(MAX_SIZE).unwrap();
        assert!(load(&file("most", &vec![b'\n'; most]), true).is_ok_and(|text| text.is_some_and(|text| text.len() == most)));
        assert_eq!(load(&file("more", &vec![b'\n'; most + 1]), true), Err("larger than 1 MiB, the most of a config that clv reads".into()));
        // A pipe that nothing writes to is refused, not waited for.
        let pipe = dir.join("pipe");
        if std::process::Command::new("mkfifo").arg(&pipe).status().is_ok_and(|status| status.success()) {
            assert_eq!(load(&pipe, true), Err("not a regular file".into()));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A line's number counts from 1, by the breaks before a place; a place past the end is on the
    /// last line.
    #[test]
    fn a_place_is_on_the_line_its_breaks_say() {
        let text = "a = 1\n\nb = 2\n";
        assert_eq!([0, 5, 6, 7, 12, 13, 99].map(|at| line(text, at)), [1, 1, 2, 3, 3, 4, 4]);
        assert_eq!(line("", 0), 1);
    }

    proptest::proptest! {
        /// Whatever keys a config binds and unbinds, in one keymap and in both, what clv writes of
        /// the keymaps reads back to the same keymaps: the same keys for each command, in the same
        /// order, the same of them listed. No key is written both under `[keys]` and under the
        /// table of a keymap, and written again the file is the same file.
        #[test]
        fn the_keys_written_read_back_to_the_same_keymaps(
            bindings in proptest::collection::vec(
                (
                    proptest::option::of(proptest::bool::ANY),
                    "(a|b|c|x|q|j|J|n|g|/|=|D|\\[|\\]|enter|esc|space|up|ctrl-d|alt-v|backspace)",
                    "(quit|none|none|search|next|go-to|zoom-in|copy|back|leave|list-open|list-tile-down|list-filter|cycle-theme|increase-width|screen-down|rerender)",
                ),
                0..14,
            ),
        ) {
            let (catalog, settings): (Catalog, [&'static Setting; 2]) = (catalog(), [&SETTINGS[0], &SETTINGS[1]]);
            let mode = |list: Option<bool>| list.map(|list| if list { Mode::List } else { Mode::Pages });
            // A key once in each table, as TOML has it: the first of each.
            let mut given: Vec<Binding> = Vec::new();
            for (at, (list, key, command)) in bindings.iter().enumerate() {
                if !given.iter().any(|other| other.mode == mode(*list) && other.key == *key) {
                    given.push(Binding { mode: mode(*list), key: key.clone(), command: command.clone(), line: at, at });
                }
            }
            if let Ok(keymaps) = Keymaps::bound(&settings, &given) {
                let text = written(&catalog, &Parsed::default(), &settings, &keymaps);
                let config = Config::parse(&text, &catalog);
                proptest::prop_assert!(config.is_ok(), "{:?}\n{}", config, text);
                let read = Keymaps::bound(&settings, &config.map(|config| config.keys).unwrap_or_default());
                proptest::prop_assert!(read.is_ok(), "{:?}\n{}", read, text);
                let read = read.unwrap_or_default();
                proptest::prop_assert_eq!(keys_of(&read, &settings), keys_of(&keymaps, &settings), "{}", text);
                proptest::prop_assert_eq!(&written(&catalog, &Parsed::default(), &settings, &read), &text);
                let [both, pages, list] = tables(&text);
                let keys = |table: &str| {
                    let bound = table.lines().filter(|line| !line.starts_with("##") && !line.is_empty());
                    bound.filter_map(|line| line.trim_start_matches("# ").split_once(" = ")).map(|(key, _)| key.to_owned()).collect::<Vec<_>>()
                };
                for key in keys(&both) {
                    proptest::prop_assert!(!keys(&pages).contains(&key) && !keys(&list).contains(&key), "{} twice\n{}", key, text);
                }
            }
        }

        /// No text of a config makes clv panic: it sets options, or it says what is wrong. The
        /// text is nearly a config: names of the catalog and others, values of every kind, tables,
        /// any text at all somewhere in it, and all of that cut anywhere.
        #[test]
        fn no_text_of_a_config_panics(
            lines in proptest::collection::vec(
                (
                    "(width|theme|per-row|recurse|pager|pages|password|zoom|keys|keys.x|keys.pages.enter|keys.list.\"/\"|keys.ctrl-c|\\[keys\\]|\\[\\[a\\]\\]|[a-z.\"' -]{0,8})",
                    "( = |=| |\\.x = )",
                    "(72|300|-1|0x48|1e9|true|false|\"dark\"|\"auto\"|'window'|\"quit\"|\"none\"|\"list-open\"|\\[1, 2\\]|\\{ a = 1 \\}|1979-05-27|\"\"\"|[\\PC]{0,6})",
                ),
                0..12,
            ),
            junk in proptest::option::weighted(0.3, "\\PC{0,40}"),
            at in 0usize..13,
            cut in 0usize..400,
        ) {
            let mut lines: Vec<String> = lines.iter().map(|(name, equals, value)| format!("{name}{equals}{value}\n")).collect();
            // Any text at all in a few of them: most of those are no TOML, and say so.
            lines.insert(at.min(lines.len()), junk.unwrap_or_default());
            let text = lines.concat();
            // The keys it binds go on to the keymaps, which refuse them or take them.
            let keys = |text: &str| Config::parse(text, &catalog()).map(|config| super::super::commands::Keymaps::bound(&[], &config.keys));
            let _ = (sets(&text), keys(&text));
            let cut = (0..=cut.min(text.len())).rev().find(|at| text.is_char_boundary(*at)).unwrap_or(0);
            let _ = (sets(&text[..cut]), keys(&text[..cut]));
        }
    }
}
