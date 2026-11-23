//! The reader's config: a TOML file that sets options, by the names of the options catalog
//! ([`crate::args::Catalog`]), and binds keys to commands, by the names of the catalog of commands
//! ([`super::commands`]).
//!
//! clv reads `$XDG_CONFIG_HOME/clv/config.toml` when that variable holds an absolute path, and
//! `$HOME/.config/clv/config.toml` otherwise, on macOS too. `--config FILE` reads another file, and
//! `--no-config` none ([`Which`]). The file is untrusted input, as every file is: clv reads at most
//! [`MAX_SIZE`] of it, and whatever is wrong with it is an error that names the file and the line.
//! Why TOML, and why the file is flat:
//! [ADR-0043](../../docs/adr/0043-a-config-in-toml-flat-keyed-by-the-names-of-the-catalogs.md).

use std::io::Read;
use std::path::{Path, PathBuf};

use toml::de::{DeTable, DeValue};

use super::commands::{Binding, Mode};
use crate::args::{Catalog, Entry, Settable};

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
    fn parse(text: &str, catalog: &Catalog) -> Result<Self, Wrong> {
        let table = DeTable::parse(text).map_err(|e| Wrong { line: e.span().map(|span| line(text, span.start)), why: e.message().to_owned() })?;
        // A table is kept in the order of its keys' names: the order of the file is by where each is.
        let mut entries: Vec<_> = table.get_ref().iter().collect();
        entries.sort_by_key(|(key, _)| key.span().start);
        let mut config = Self::default();
        for (key, value) in entries {
            let (name, at) = (key.get_ref().as_ref(), line(text, key.span().start));
            if name == KEYS {
                let DeValue::Table(keys) = value.get_ref() else { return Err(Wrong::at(at, format!("{KEYS}: expected a table of keys"))) };
                bindings(keys, None, text, &mut config.keys)?;
                continue;
            }
            let entry = option(catalog, name).map_err(|why| Wrong::at(at, why))?;
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
fn bindings(keys: &DeTable<'_>, mode: Option<Mode>, text: &str, into: &mut Vec<Binding>) -> Result<(), Wrong> {
    for (key, value) in keys.iter() {
        let (name, at) = (key.get_ref().as_ref(), key.span().start);
        let keymap = KEYMAPS.iter().find(|(keymap, _)| mode.is_none() && *keymap == name);
        match (value.get_ref(), keymap) {
            (DeValue::Table(keys), Some((_, mode))) => bindings(keys, Some(*mode), text, into)?,
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
        None => Err(format!("unknown option {name}")),
    }
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

    const fn option(name: &'static str, value: Option<&'static str>, shape: Shape) -> OptionSpec {
        OptionSpec { name, short: None, value, help: "", alias: None, shape, config: Settable::Yes }
    }

    static WIDTH: Setting = Setting {
        named: &[Named { name: "window", says: "" }],
        numbers: Some(Numbers { least: 20, most: 200, step: 4, unit: "" }),
        auto: None,
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
            ("[keys.list]\npages = { x = \"quit\" }", 2, "pages: expected the name of a command or \"none\", got a table"),
        ] {
            assert_eq!(Config::parse(text, &catalog()).unwrap_err(), Wrong::at(line, why), "{text:?}");
        }
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
