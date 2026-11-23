//! What the pager can do, by name, and the keys that do it.
//!
//! A [`Command`] is one thing the reader can ask of the pager: `screen-down`, `list-tile-left`,
//! `quit`. The catalog is the viewer's own ([`VIEWER`]) and, for each setting a backend declares, one
//! or two named from it: `cycle-theme`, or `decrease-width` and `increase-width`. Each has its name,
//! what it makes of a number typed before it ([`Count`]), where it applies ([`Mode`]) and its line in
//! `?` ([`Line`]). The names are a contract with the reader's config, and keep to three rules
//! ([`Command::name`]).
//!
//! A [`Keymap`] binds keys to commands: one for a file's pages and one for a list ([`Keymaps`]),
//! built once a run from the viewer's defaults ([`PAGES`], [`LIST`]) and the keys the settings
//! suggest ([`Keys`]). The pager looks a key up and does what its command says; `?`, the status
//! line's reminders ([`Hint`]) and whatever else names a key look the command up, so a key is
//! written nowhere but in the defaults here and in a setting's declaration
//! ([ADR-0036](../../docs/adr/0036-keys-are-bound-to-named-commands.md)).
//!
//! Outside the keymaps: the digits, which are the number typed before a key, and while one is
//! typed Esc and Backspace, which drop it and its last digit; the links' labels; and the text of
//! a search or a filter being typed.

use super::keys::{self, Key};
use crate::source::{Keys, Setting};

/// Where the reader is, each with a keymap of its own: among a file's pages, or among a list's
/// tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Pages,
    List,
}

impl Mode {
    /// Where it is, as the reader is told: `search does not apply in a list`.
    fn said(self) -> &'static str {
        match self {
            Self::Pages => "a file's pages",
            Self::List => "a list",
        }
    }
}

/// Something the pager does when asked, whatever key asks: [`Command::name`] is what it is called.
#[derive(Clone, Copy, Debug)]
pub(super) enum Command {
    /// A screen down: the rest of a page taller than the window, then the next pages; in a list,
    /// the next screen of tiles.
    ScreenDown,
    ScreenUp,
    /// A few rows down a page, then the next row of pages.
    ScrollDown,
    ScrollUp,
    /// The next screen of pages, or with a search kept the next match.
    Next,
    Previous,
    /// The next screen of pages, with a search kept too.
    NextPage,
    PreviousPage,
    /// With a search kept, the match before.
    PreviousMatch,
    /// The first page or tile, or the one counted.
    GoTo,
    Last,
    /// The zoom, or on a document with a setting on the zoom keys ([`Keys::Zoom`]), shown by
    /// itself, that setting a step up or down.
    ZoomIn,
    ZoomOut,
    MorePerRow,
    FewerPerRow,
    Search,
    /// Label the links on screen, to follow one.
    Follow,
    /// Label them, to copy one's address.
    CopyLink,
    /// To where what is shown lists its contents, where it says it has such a place.
    Contents,
    Copy,
    OpenInApp,
    Rerender,
    /// Clear the search or the filter kept; then back through the links followed; then out of the
    /// file or the folder, where there is somewhere to go back to.
    Back,
    /// Clear the search or the filter kept; then out of the file or the folder, and where there is
    /// nowhere to go back to, quit.
    Leave,
    Quit,
    Help,
    TileLeft,
    TileRight,
    TileUp,
    TileDown,
    /// Open the tile chosen.
    Open,
    Filter,
    /// A setting some backend declares, turned one way: the viewer names none of them.
    Setting(&'static Setting, Turn),
}

/// Which way a setting's command turns its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Turn {
    /// To its next named value, after the last the first ([`Keys::Cycle`]).
    Next,
    /// A step down among its numbers, or up ([`Keys::Step`]).
    Down,
    Up,
}

/// What a command makes of the number typed before it, Emacs's prefix argument: the first half of
/// its signature, its line in `?` saying what it does with one and without.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Count {
    /// Nothing: the number stays typed, for the key that takes it.
    Left,
    /// Nothing, and the number is dropped.
    Dropped,
    /// It takes it, where it has a use for one: `go-to` everywhere, `next` with a search kept, the
    /// zoom on a document whose font size it sets. Where it has none, the number stays typed.
    Taken,
}

/// The viewer's own commands: with those of the settings a build declares, the catalog.
pub(super) const VIEWER: &[Command] = &[
    Command::ScreenDown,
    Command::ScreenUp,
    Command::ScrollDown,
    Command::ScrollUp,
    Command::Next,
    Command::Previous,
    Command::NextPage,
    Command::PreviousPage,
    Command::PreviousMatch,
    Command::GoTo,
    Command::Last,
    Command::ZoomIn,
    Command::ZoomOut,
    Command::MorePerRow,
    Command::FewerPerRow,
    Command::Search,
    Command::Follow,
    Command::CopyLink,
    Command::Contents,
    Command::Copy,
    Command::OpenInApp,
    Command::Rerender,
    Command::Back,
    Command::Leave,
    Command::Quit,
    Command::Help,
    Command::TileLeft,
    Command::TileRight,
    Command::TileUp,
    Command::TileDown,
    Command::Open,
    Command::Filter,
];

impl PartialEq for Command {
    /// The same command: a setting's by the setting's name, which is all a setting is known by.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Setting(a, turn), Self::Setting(b, other)) => a.name == b.name && turn == other,
            _ => std::mem::discriminant(self) == std::mem::discriminant(other),
        }
    }
}

impl Eq for Command {}

impl Command {
    /// What it is called, in kebab-case. The names keep to three rules:
    ///
    /// - A command that steps an option is `increase-` or `decrease-` and the name of the option. A
    ///   command that goes to the next named value of an option is `cycle-` and that name. A setting's
    ///   commands are built so from the setting's name, which is its option's.
    /// - A command for the list alone is `list-` and what it does. The others have no prefix.
    /// - No name has the prefix of a format or a backend: a name two backends declare is one setting.
    pub(super) fn name(self) -> String {
        let name = match self {
            Self::ScreenDown => "screen-down",
            Self::ScreenUp => "screen-up",
            Self::ScrollDown => "scroll-down",
            Self::ScrollUp => "scroll-up",
            Self::Next => "next",
            Self::Previous => "previous",
            Self::NextPage => "next-page",
            Self::PreviousPage => "previous-page",
            Self::PreviousMatch => "previous-match",
            Self::GoTo => "go-to",
            Self::Last => "last",
            Self::ZoomIn => "zoom-in",
            Self::ZoomOut => "zoom-out",
            Self::MorePerRow => "increase-per-row",
            Self::FewerPerRow => "decrease-per-row",
            Self::Search => "search",
            Self::Follow => "follow",
            Self::CopyLink => "copy-link",
            Self::Contents => "contents",
            Self::Copy => "copy",
            Self::OpenInApp => "open-in-app",
            Self::Rerender => "rerender",
            Self::Back => "back",
            Self::Leave => "leave",
            Self::Quit => "quit",
            Self::Help => "help",
            Self::TileLeft => "list-tile-left",
            Self::TileRight => "list-tile-right",
            Self::TileUp => "list-tile-up",
            Self::TileDown => "list-tile-down",
            Self::Open => "list-open",
            Self::Filter => "list-filter",
            Self::Setting(setting, Turn::Next) => return format!("cycle-{}", setting.name),
            Self::Setting(setting, Turn::Down) => return format!("decrease-{}", setting.name),
            Self::Setting(setting, Turn::Up) => return format!("increase-{}", setting.name),
        };
        name.to_owned()
    }

    /// What it makes of the number typed before it.
    pub(super) fn count(self) -> Count {
        match self {
            Self::GoTo | Self::Next | Self::Previous | Self::PreviousMatch => Count::Taken,
            Self::ZoomIn | Self::ZoomOut | Self::MorePerRow | Self::FewerPerRow => Count::Taken,
            Self::Setting(setting, _) if setting.numbers.is_some() => Count::Taken,
            Self::Search | Self::Filter | Self::Follow | Self::CopyLink | Self::Contents => Count::Dropped,
            _ => Count::Left,
        }
    }

    /// Whether it means something in `mode`: a keymap binds only the commands of its own.
    pub(super) fn applies(self, mode: Mode) -> bool {
        match self {
            Self::ScrollDown | Self::ScrollUp | Self::Next | Self::Previous | Self::NextPage | Self::PreviousPage | Self::PreviousMatch => {
                mode == Mode::Pages
            }
            Self::Search | Self::Follow | Self::CopyLink | Self::Contents => mode == Mode::Pages,
            Self::TileLeft | Self::TileRight | Self::TileUp | Self::TileDown | Self::Open | Self::Filter => mode == Mode::List,
            _ => true,
        }
    }
}

/// A key as a keymap binds it: by itself, or only with a number typed before it (`12⏎`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Chord {
    key: Key,
    counted: bool,
    /// Whether `?` and the reminders list it: not a key that is another with Shift or without (`=`,
    /// which is `+`), which is bound all the same.
    listed: bool,
}

/// `c` by itself.
const fn ch(c: char) -> Chord {
    key(Key::Char(c))
}

/// `key` by itself.
const fn key(key: Key) -> Chord {
    Chord { key, counted: false, listed: true }
}

/// `key` with a number typed before it.
const fn counted(key: Key) -> Chord {
    Chord { key, counted: true, listed: true }
}

/// `c` too, the same key as the one before it with Shift or without: bound, and not listed.
const fn also(c: char) -> Chord {
    Chord { key: Key::Char(c), counted: false, listed: false }
}

/// The viewer's keys in a file's pages: each command's, in the order `?` lists them, `less`'s and
/// `vi`'s with Emacs's beside them. A setting's keys are the setting's to suggest ([`Keys`]), and
/// come after these: none may be one of them, which is why the theme's is `D`, not `d`.
const PAGES: &[(Command, &[Chord])] = &[
    (Command::ScreenDown, &[ch(' '), key(Key::PageDown), ch('d'), key(Key::Ctrl('v'))]),
    (Command::ScreenUp, &[ch('b'), key(Key::PageUp), ch('u'), key(Key::Alt('v'))]),
    (Command::ScrollDown, &[ch('j'), key(Key::Down), key(Key::Ctrl('n')), key(Key::Enter)]),
    (Command::ScrollUp, &[ch('k'), key(Key::Up), key(Key::Ctrl('p'))]),
    (Command::Next, &[ch('n')]),
    (Command::Previous, &[ch('p')]),
    (Command::NextPage, &[key(Key::Right)]),
    (Command::PreviousPage, &[key(Key::Left)]),
    (Command::PreviousMatch, &[ch('N')]),
    (Command::GoTo, &[ch('g'), key(Key::Home), counted(Key::Enter)]),
    (Command::Last, &[ch('G'), key(Key::End)]),
    (Command::ZoomIn, &[ch('+'), also('=')]),
    (Command::ZoomOut, &[ch('-'), also('_')]),
    (Command::MorePerRow, &[ch(',')]),
    (Command::FewerPerRow, &[ch('.')]),
    (Command::Search, &[ch('/')]),
    (Command::Follow, &[ch('f')]),
    (Command::CopyLink, &[ch('F')]),
    (Command::Contents, &[ch('T')]),
    (Command::Copy, &[ch('c')]),
    (Command::OpenInApp, &[ch('o')]),
    (Command::Rerender, &[ch('r'), key(Key::Ctrl('l'))]),
    (Command::Back, &[key(Key::Backspace)]),
    (Command::Leave, &[key(Key::Escape)]),
    (Command::Quit, &[ch('q'), key(Key::Interrupt)]),
    (Command::Help, &[ch('?')]),
];

/// And in a list.
const LIST: &[(Command, &[Chord])] = &[
    (Command::TileLeft, &[key(Key::Left), ch('h'), key(Key::Ctrl('b'))]),
    (Command::TileRight, &[key(Key::Right), ch('l'), key(Key::Ctrl('f'))]),
    (Command::TileUp, &[key(Key::Up), ch('k'), key(Key::Ctrl('p'))]),
    (Command::TileDown, &[key(Key::Down), ch('j'), key(Key::Ctrl('n'))]),
    (Command::ScreenDown, &[ch(' '), ch('n'), ch('d'), key(Key::Ctrl('v')), key(Key::PageDown)]),
    (Command::ScreenUp, &[ch('b'), ch('p'), ch('u'), key(Key::Alt('v')), key(Key::PageUp)]),
    (Command::GoTo, &[ch('g'), key(Key::Home), counted(Key::Enter)]),
    (Command::Last, &[ch('G'), key(Key::End)]),
    (Command::Open, &[key(Key::Enter)]),
    (Command::Filter, &[ch('/')]),
    (Command::ZoomIn, &[ch('+'), also('=')]),
    (Command::ZoomOut, &[ch('-'), also('_')]),
    (Command::MorePerRow, &[ch(',')]),
    (Command::FewerPerRow, &[ch('.')]),
    (Command::Copy, &[ch('c')]),
    (Command::OpenInApp, &[ch('o')]),
    (Command::Rerender, &[ch('r'), key(Key::Ctrl('l'))]),
    (Command::Back, &[key(Key::Backspace)]),
    (Command::Leave, &[key(Key::Escape)]),
    (Command::Quit, &[ch('q'), key(Key::Interrupt)]),
    (Command::Help, &[ch('?')]),
];

/// The keys of one [`Mode`]: each bound to one command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Keymap {
    bindings: Vec<(Chord, Command)>,
}

impl Keymap {
    /// The viewer's defaults for `mode`, from `table`. What can't be bound is left out and said in
    /// `wrong`: a command that means nothing in `mode`, or a key already bound.
    fn defaults(mode: Mode, table: &[(Command, &[Chord])], wrong: &mut Vec<String>) -> Self {
        let mut keymap = Self::default();
        for (command, chords) in table {
            for chord in *chords {
                if !command.applies(mode) {
                    wrong.push(format!("{} means nothing in {mode:?}", command.name()));
                } else if let Some(other) = keymap.bound(chord.key, chord.counted) {
                    wrong.push(format!("{:?} is {}'s and {}'s in {mode:?}", chord.key, other.name(), command.name()));
                } else {
                    keymap.bindings.push((*chord, *command));
                }
            }
        }
        keymap
    }

    /// The command `key` is bound to as it is: with a number (`counted`), or by itself.
    fn bound(&self, key: Key, counted: bool) -> Option<Command> {
        self.bindings.iter().find(|(chord, _)| chord.key == key && chord.counted == counted).map(|(_, command)| *command)
    }

    /// The command `key` asks for, if it is bound: with a number typed before it (`counted`), the
    /// one it is bound to with a number, where it is.
    pub(super) fn command(&self, key: Key, counted: bool) -> Option<Command> {
        let with_a_number = if counted { self.bound(key, true) } else { None };
        with_a_number.or_else(|| self.bound(key, false))
    }

    /// The keys bound to `command` that are listed, in their order: by themselves, or those bound
    /// with a number.
    fn keys(&self, command: Command, counted: bool) -> impl Iterator<Item = Key> + '_ {
        let listed = self.bindings.iter().filter(move |(chord, bound)| *bound == command && chord.counted == counted && chord.listed);
        listed.map(|(chord, _)| chord.key)
    }

    /// The first key bound to `command`, as `?` names it; nothing if none is.
    pub(super) fn first(&self, command: Command) -> String {
        self.keys(command, false).next().map(long).unwrap_or_default()
    }

    /// The first key bound to `command` with a number, as `?` names it; nothing if none is.
    pub(super) fn first_counted(&self, command: Command) -> String {
        self.keys(command, true).next().map(long).unwrap_or_default()
    }

    /// The keys of `commands`, each command's first before any's second: `q  Esc  Ctrl-C`.
    pub(super) fn each(&self, commands: &[Command]) -> String {
        self.rows(commands).into_iter().flatten().collect::<Vec<_>>().join("  ")
    }

    /// The keys of `commands`, which go together two by two, as down and up do: key beside key,
    /// `j k  ↓ ↑  Enter`.
    pub(super) fn beside(&self, commands: &[Command]) -> String {
        let pairs = commands.chunks(2).flat_map(|pair| self.rows(pair));
        pairs.map(|row| row.join(" ")).collect::<Vec<_>>().join("  ")
    }

    /// The keys of `commands` by rank: their first keys, then their second, as far as any has one.
    fn rows(&self, commands: &[Command]) -> Vec<Vec<String>> {
        let keys: Vec<Vec<String>> = commands.iter().map(|command| self.keys(*command, false).map(long).collect()).collect();
        let most = keys.iter().map(Vec::len).max().unwrap_or(0);
        (0..most).map(|rank| keys.iter().filter_map(|keys| keys.get(rank).cloned()).collect()).collect()
    }

    /// `number` before the first key of `command`, as it is typed: `12g`, `3.`. Nothing if no key is
    /// bound to it.
    pub(super) fn numbered(&self, number: u32, command: Command) -> String {
        self.keys(command, false).next().map(|key| after(number, &long(key))).unwrap_or_default()
    }

    /// The keys of `line`, as `?` lists them.
    pub(super) fn shown(&self, line: &Line) -> String {
        let mut keys = Vec::new();
        if line.counted {
            let number = line.number.map_or(0, |(number, _)| number);
            let counted = line.commands.iter().flat_map(|command| self.keys(*command, true));
            keys.extend(counted.map(|key| after(number, &long(key))));
        } else {
            let from = line.beside.unwrap_or(line.commands.len());
            let (alone, together) = line.commands.split_at_checked(from).unwrap_or((line.commands, &[]));
            keys.extend([self.each(alone), self.beside(together)]);
        }
        if let Some((number, at)) = line.number
            && let Some(command) = line.commands.get(at)
        {
            keys.push(self.numbered(number, *command));
        }
        keys.retain(|keys| !keys.is_empty());
        keys.join("  ")
    }

    /// `text`, each `{name}` in it turned into the first key bound here to the command so named
    /// (nothing, if none is): what `?` and the status line say of a key, they say through this.
    pub(super) fn fill(&self, text: &str) -> String {
        let mut filled = String::new();
        let mut rest = text;
        while let Some((before, after)) = rest.split_once('{') {
            filled.push_str(before);
            let Some((name, after)) = after.split_once('}') else {
                rest = after;
                continue;
            };
            let bound = self.bindings.iter().find(|(chord, command)| !chord.counted && chord.listed && command.name() == name);
            filled.push_str(&bound.map(|(chord, _)| long(chord.key)).unwrap_or_default());
            rest = after;
        }
        filled.push_str(rest);
        filled
    }

    /// The reminder `hints` make, as the status line starts with: for each, the first keys of its
    /// commands and its word, `space b j k · n p · + - zoom · q quit`. `zoom` is what the zoom
    /// keys are said to set ([`Says::Zoom`]). A hint none of whose commands has a key is left out.
    pub(super) fn reminder(&self, hints: &[Hint], zoom: &str) -> String {
        let said = hints.iter().filter_map(|hint| {
            let keys: Vec<Key> = hint.commands.iter().filter_map(|command| self.keys(*command, hint.counted).next()).collect();
            let keys = match keys.as_slice() {
                [] => return None,
                // The count, as it is typed before them.
                keys if hint.counted => keys.iter().map(|key| after(12, &short(*key))).collect::<Vec<_>>().join(" "),
                keys if ARROWS.iter().all(|arrow| keys.contains(arrow)) && keys.len() == ARROWS.len() => "arrows".to_owned(),
                keys => keys.iter().map(|key| short(*key)).collect::<Vec<_>>().join(" "),
            };
            Some(match hint.says {
                Says::Nothing => keys,
                Says::Word(word) => format!("{keys} {word}"),
                Says::Zoom => format!("{keys} {zoom}"),
            })
        });
        said.collect::<Vec<_>>().join(" · ")
    }
}

/// The four arrows, which a reminder calls `arrows` where they are the keys it lists.
const ARROWS: [Key; 4] = [Key::Left, Key::Right, Key::Up, Key::Down];

/// `number` typed before a key called `name`: `12g`, and `12 Enter` for a key with a longer name.
fn after(number: u32, name: &str) -> String {
    if name.chars().count() == 1 { format!("{number}{name}") } else { format!("{number} {name}") }
}

/// What `key` is called in `?`: as a config writes it ([`keys::name`]), but for the arrows, which
/// `?` draws. A key that is none typed (a terminal's answer) has no name.
fn long(key: Key) -> String {
    match key {
        Key::Up => "↑".to_owned(),
        Key::Down => "↓".to_owned(),
        Key::Left => "←".to_owned(),
        Key::Right => "→".to_owned(),
        key => keys::name(key),
    }
}

/// What `key` is called in the status line's reminders, where room is short.
fn short(key: Key) -> String {
    match key {
        Key::Enter => "⏎".to_owned(),
        key => long(key),
    }
}

/// A key that a config binds, as the config writes it: the keymap it is for (`None`, every keymap
/// where the command applies), the name of the key, the name of the command or `none`, and where
/// in the file it is (its line, and its place, which orders two on one line).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Binding {
    pub(super) mode: Option<Mode>,
    pub(super) key: String,
    pub(super) command: String,
    pub(super) line: usize,
    pub(super) at: usize,
}

/// The keymaps of a run: a file's pages' and a list's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Keymaps {
    pages: Keymap,
    list: Keymap,
}

impl Default for Keymaps {
    /// The viewer's defaults alone: a build that declares no setting.
    fn default() -> Self {
        Self::new(&[])
    }
}

impl Keymaps {
    /// The viewer's defaults, then the keys `settings` suggest for their commands, in both
    /// keymaps: a setting holds for a list's tiles as for a file's pages.
    pub(super) fn new(settings: &[&'static Setting]) -> Self {
        Self::build(settings).0
    }

    /// The keymap of `mode`.
    pub(super) fn of(&self, mode: Mode) -> &Keymap {
        match mode {
            Mode::Pages => &self.pages,
            Mode::List => &self.list,
        }
    }

    /// The keymaps of a run whose config binds `bindings`: the defaults ([`Keymaps::new`]), then
    /// each key of the config, in the order of the file.
    ///
    /// - A key bound in `[keys]` is bound in every keymap where its command applies, and a key
    ///   bound in the table of one keymap wins there. `none` unbinds a key.
    /// - A key that the config binds loses its default binding, a setting's suggestion too, and the
    ///   command keeps its other keys. The key comes first among the keys of its command, so `?`
    ///   and the reminders show it; two such keys keep the order of the file.
    /// - A binding that is the same as the default changes nothing: the key keeps its place, and a
    ///   key that `?` doesn't list (`=`) stays unlisted.
    /// - A key bound with a number typed before it (`12` and Enter) is never changed.
    ///
    /// What can't be bound is the error, with its line: a name that is no key's
    /// ([`keys::named`]), a digit, `ctrl-c`, a command that doesn't exist, and a command that
    /// doesn't apply in the keymap of its table.
    pub(super) fn bound(settings: &[&'static Setting], bindings: &[Binding]) -> Result<Self, (usize, String)> {
        let defaults = Self::new(settings);
        let mut keymaps = defaults.clone();
        let mut wanted: Vec<(&Binding, Key, Option<Command>)> = Vec::new();
        for binding in bindings {
            let wrong = |why: String| (binding.line, format!("{}: {why}", binding.key));
            let key = keys::named(&binding.key).map_err(wrong)?;
            match key {
                Key::Char('0'..='9') => return Err(wrong("a digit starts the number typed before a key, so it can't be bound".into())),
                Key::Interrupt => return Err(wrong("it always quits, so it can't be bound".into())),
                _ => {}
            }
            let command = match binding.command.as_str() {
                super::config::NONE => None,
                name => Some(named(name, settings).ok_or_else(|| wrong(format!("no command is called {name}")))?),
            };
            if let (Some(mode), Some(command)) = (binding.mode, command)
                && !command.applies(mode)
            {
                return Err(wrong(format!("{} does not apply in {}", command.name(), mode.said())));
            }
            wanted.push((binding, key, command));
        }
        for mode in [Mode::Pages, Mode::List] {
            // What the config binds here, a key once: in the table of this keymap, or else in the
            // table of them all, where its command applies here.
            let here = |(binding, _, command): &&(&Binding, Key, Option<Command>)| match binding.mode {
                Some(only) => only == mode,
                None => command.is_none_or(|command| command.applies(mode)),
            };
            let mut bound: Vec<&(&Binding, Key, Option<Command>)> = Vec::new();
            for binding in wanted.iter().filter(here) {
                match bound.iter_mut().find(|(_, key, _)| *key == binding.1) {
                    Some(other) if binding.0.mode.is_some() => *other = binding,
                    Some(_) => {}
                    None => bound.push(binding),
                }
            }
            bound.sort_by_key(|(binding, ..)| binding.at);
            let (keymap, default) = match mode {
                Mode::Pages => (&mut keymaps.pages, &defaults.pages),
                Mode::List => (&mut keymaps.list, &defaults.list),
            };
            let mut first = 0usize;
            for (_, key, command) in bound {
                if default.bound(*key, false) == *command {
                    continue;
                }
                keymap.bindings.retain(|(chord, _)| chord.key != *key || chord.counted);
                if let Some(command) = command {
                    keymap.bindings.insert(first.min(keymap.bindings.len()), (Chord { key: *key, counted: false, listed: true }, *command));
                    first = first.saturating_add(1);
                }
            }
        }
        Ok(keymaps)
    }

    /// [`Keymaps::new`], and what was wrong with what it was given, each left out: a key the
    /// defaults bind twice, a setting's key that is one of the viewer's, a digit among them, or
    /// another setting's, or a setting whose command would be called as one of the viewer's is. The
    /// viewer's defaults come first: a setting's key never takes one of them
    /// ([ADR-0036](../../docs/adr/0036-keys-are-bound-to-named-commands.md)).
    fn build(settings: &[&'static Setting]) -> (Self, Vec<String>) {
        let mut wrong = Vec::new();
        let mut keymaps = Self { pages: Keymap::defaults(Mode::Pages, PAGES, &mut wrong), list: Keymap::defaults(Mode::List, LIST, &mut wrong) };
        for setting in settings {
            for (char, turn) in suggested(setting) {
                let command = Command::Setting(setting, turn);
                if VIEWER.iter().any(|viewer| viewer.name() == command.name()) {
                    wrong.push(format!("{}: its command {} is called as the viewer's is", setting.name, command.name()));
                    continue;
                }
                let key = Key::Char(char);
                match keymaps.taken(key) {
                    _ if char.is_ascii_digit() => wrong.push(format!("{char:?}, {}'s key, is the viewer's: a digit of the count", setting.name)),
                    None => {
                        let chord = Chord { key, counted: false, listed: true };
                        keymaps.pages.bindings.push((chord, command));
                        keymaps.list.bindings.push((chord, command));
                    }
                    // The same setting, declared by another backend: one command, bound once.
                    Some(other) if other == command => {}
                    Some(Command::Setting(other, _)) => wrong.push(format!("{char:?} is {}'s and {}'s", other.name, setting.name)),
                    Some(other) => wrong.push(format!("{char:?}, {}'s key, is the viewer's ({})", setting.name, other.name())),
                }
            }
        }
        (keymaps, wrong)
    }

    /// The command `key` is bound to in either keymap, with a number or without.
    fn taken(&self, key: Key) -> Option<Command> {
        [&self.pages, &self.list].into_iter().find_map(|keymap| keymap.command(key, true))
    }
}

/// The command called `name`: one of the viewer's, or one of a setting that `settings` holds.
fn named(name: &str, settings: &[&'static Setting]) -> Option<Command> {
    let of_settings = settings.iter().flat_map(|setting| suggested(setting).into_iter().map(|(_, turn)| Command::Setting(setting, turn)));
    VIEWER.iter().copied().chain(of_settings).find(|command| command.name() == name)
}

/// The keys `setting` suggests, each with the way its command turns the setting: one command for a
/// key that cycles, two for keys that step. A setting on the zoom keys has no command of its own:
/// it is reached through `zoom-in` and `zoom-out`, where it takes them.
fn suggested(setting: &'static Setting) -> Vec<(char, Turn)> {
    match setting.keys {
        Keys::Cycle(key) => vec![(key, Turn::Next)],
        Keys::Step(down, up) => vec![(down, Turn::Down), (up, Turn::Up)],
        Keys::Zoom => Vec::new(),
    }
}

/// The names of the commands made for `setting`, for a build's own tests: a config binds keys to
/// them, so they keep to the rules of [`Command::name`].
#[cfg(test)]
pub(crate) fn setting_commands(setting: &'static Setting) -> Vec<String> {
    suggested(setting).into_iter().map(|(_, turn)| Command::Setting(setting, turn).name()).collect()
}

/// The commands that the phrases of `settings` name between braces ([`Setting::help`]) and that no
/// key is bound to by default, for a build's own tests: nothing, in a build whose phrases name the
/// commands of its settings or the viewer's.
#[cfg(test)]
pub(crate) fn unbound_in_phrases(settings: &[&'static Setting]) -> Vec<String> {
    let keymaps = Keymaps::new(settings);
    let names = |setting: &&'static Setting| {
        let named = setting.help.split('{').skip(1).filter_map(|rest| rest.split_once('}')).map(|(name, _)| name.to_owned());
        named.collect::<Vec<_>>()
    };
    let bound = |name: &String| [Mode::Pages, Mode::List].iter().all(|mode| keymaps.of(*mode).fill(&format!("{{{name}}}")).is_empty());
    settings.iter().flat_map(names).filter(bound).collect()
}

/// What is wrong with the keys `settings` suggest, for a build's own tests ([`Keymaps::build`]):
/// nothing, in a build whose settings each have keys of their own.
#[cfg(test)]
pub(crate) fn wrong_keys(settings: &[&'static Setting]) -> Vec<String> {
    Keymaps::build(settings).1
}

/// A line of `?`: the commands it is about, how their keys are set out, and what they do.
#[derive(Clone, Copy, Debug)]
pub(super) struct Line {
    pub(super) commands: &'static [Command],
    /// From which of the commands on they go together two by two, as down and up do, key beside key
    /// (`j k  ↓ ↑`), those before it each key by itself (`n  N p`); nowhere, and every key is by
    /// itself (`q  Esc  Ctrl-C`).
    beside: Option<usize>,
    /// Whether the line is about the commands with a number typed before them: then it lists the
    /// keys bound with one (`12 Enter`) in place of the others.
    counted: bool,
    /// A number, shown after the keys before the first key of the command at this place among
    /// `commands`, as it is typed: `3.`, `12g`.
    number: Option<(u32, usize)>,
    /// What the commands do. `{name}` stands for the first key of the command so named
    /// ([`Keymap::fill`]).
    pub(super) says: &'static str,
}

/// A line listing each key of `commands`.
const fn line(commands: &'static [Command], says: &'static str) -> Line {
    Line { commands, beside: None, counted: false, number: None, says }
}

/// A line of `commands` that go together two by two.
const fn pair(commands: &'static [Command], says: &'static str) -> Line {
    Line { beside: Some(0), ..line(commands, says) }
}

impl Line {
    /// The line, with `number` typed before the first key of its command at `at`.
    const fn with(self, number: u32, at: usize) -> Self {
        Self { number: Some((number, at)), ..self }
    }

    /// The line, its commands from the one at `from` on going together two by two.
    const fn beside(self, from: usize) -> Self {
        Self { beside: Some(from), ..self }
    }

    /// The line, about its commands with `number` typed before them.
    const fn counted(self, number: u32) -> Self {
        Self { counted: true, number: Some((number, 0)), ..self }
    }
}

/// `?` in a file's pages: the viewer's commands, before the settings' and the ways out.
pub(super) const PAGE_LINES: &[Line] = &[
    line(&[Command::ScreenDown], "the next screen, or the rest of a page taller than the window"),
    line(&[Command::ScreenUp], "back a screen"),
    pair(&[Command::ScrollDown, Command::ScrollUp], "a few rows down or up, or a row of pages"),
    pair(&[Command::Next, Command::Previous, Command::NextPage, Command::PreviousPage], "the next or the previous screen of pages"),
    pair(&[Command::GoTo, Command::Last], "the first or the last page"),
    line(&[Command::GoTo], "page 12").counted(12),
    ZOOM_LINE,
    PER_ROW_LINE,
    line(&[Command::Search], "search the text as typed: enter goes to the first match, esc clears the search"),
    line(
        &[Command::Next, Command::PreviousMatch, Command::Previous],
        "with a search: the next match, or with {previous-match} or {previous} the one before; 3{next} the third on",
    )
    .beside(1)
    .with(3, 0),
    line(&[Command::Follow], "label the links on screen: type a label to follow its link, esc drops the labels"),
    line(&[Command::CopyLink], "label them, and copy the address of the one whose label is typed"),
    CONTENTS_LINE,
    line(&[Command::Back], "back to where the last link followed was"),
    line(&[Command::Rerender], "render the pages again"),
    line(&[Command::Copy], "copy the page to the clipboard"),
    line(&[Command::OpenInApp], "open the file in the app the desktop opens it with"),
];

/// The line of the command that goes to the contents: listed only where what is shown says where
/// its contents are.
pub(super) const CONTENTS_LINE: Line = line(&[Command::Contents], "to the contents: {back} comes back");

/// The zoom's line in a file's pages: a setting on the zoom keys has its own in its place, where
/// the keys are its own.
const ZOOM_LINE: Line = pair(&ZOOM, "zoom in, as far as the window's width; out, to rows of pages");
/// The commands of the zoom, in and out.
pub(super) const ZOOM: [Command; 2] = [Command::ZoomIn, Command::ZoomOut];
/// The commands of the pages to a row, more and fewer.
pub(super) const PER_ROW: [Command; 2] = [Command::MorePerRow, Command::FewerPerRow];
/// Their line in a file's pages.
const PER_ROW_LINE: Line =
    pair(&PER_ROW, "one more page to a row, or one fewer, then the width; 3{decrease-per-row} sets 3, 0{decrease-per-row} auto").with(3, 1);
/// And on a document laid out for its room, where they are columns.
pub(super) const COLUMNS_LINE: Line =
    pair(&PER_ROW, "one more column, or one fewer, down to one; 3{decrease-per-row} sets 3, 0{decrease-per-row} auto").with(3, 1);

/// `?` in a list.
pub(super) const LIST_LINES: &[Line] = &[
    pair(&[Command::TileLeft, Command::TileRight], "the one before or after"),
    pair(&[Command::TileUp, Command::TileDown], "the one above or below"),
    pair(&[Command::ScreenDown, Command::ScreenUp], "a screen on or back"),
    pair(&[Command::GoTo, Command::Last], "the first or the last"),
    line(&[Command::GoTo], "the 12th").counted(12),
    line(&[Command::Open], "open it: a file's pages, or a directory's files"),
    line(&[Command::Filter], "filter by name as you type: enter keeps it, esc clears it"),
    pair(&ZOOM, "bigger, fewer to a row; smaller, more to a row"),
    pair(&PER_ROW, "one more tile to a row, or one fewer; 3{decrease-per-row} sets 3, 0{decrease-per-row} auto").with(3, 1),
    line(&[Command::Rerender], "render again"),
    line(&[Command::Copy], "copy its picture to the clipboard"),
    line(&[Command::OpenInApp], "open it in the desktop's app: a file in its app, a folder in the file manager"),
];

/// The ways out, after the settings: gone into from a list or through a link, where there is
/// somewhere to go back to; or not, where leaving quits.
pub(super) const BACK_LINES: &[Line] =
    &[line(&[Command::Leave, Command::Back], "back to the list, or to the file a link led from"), line(&[Command::Quit], "quit")];
pub(super) const QUIT_LINES: &[Line] = &[line(&[Command::Quit, Command::Leave], "quit")];
/// And `?` itself, last.
pub(super) const HELP_LINE: Line = line(&[Command::Help], "these keys: any key goes back");

/// A part of the reminder the status line starts with: the first keys of its commands, and a word
/// for what they do.
#[derive(Clone, Copy, Debug)]
pub(super) struct Hint {
    commands: &'static [Command],
    /// Whether the keys are those bound with a number, shown with one before them: `12⏎`.
    counted: bool,
    says: Says,
}

/// What a [`Hint`] says after its keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Says {
    Nothing,
    Word(&'static str),
    /// What the zoom keys set here: the zoom, or a document's setting on those keys, by its name.
    Zoom,
}

const fn hint(commands: &'static [Command], says: Says) -> Hint {
    Hint { commands, counted: false, says }
}

const TILES: [Command; 4] = [Command::TileLeft, Command::TileRight, Command::TileUp, Command::TileDown];
const OPENS: Hint = hint(&[Command::Open], Says::Word("open"));
const ZOOMS: Hint = hint(&ZOOM, Says::Zoom);
const PER_ROWS: Hint = hint(&PER_ROW, Says::Word("per row"));
const QUITS: Hint = hint(&[Command::Quit], Says::Word("quit"));
const LEAVES: Hint = hint(&[Command::Leave], Says::Word("back"));
const SCREENS: Hint = hint(&[Command::ScreenDown, Command::ScreenUp], Says::Nothing);
const MOVES: Hint = hint(&[Command::ScreenDown, Command::ScreenUp, Command::ScrollDown, Command::ScrollUp], Says::Nothing);
const TURNS: Hint = hint(&[Command::Next, Command::Previous], Says::Nothing);
const ENDS: Hint = hint(&[Command::GoTo, Command::Last], Says::Nothing);
const GOES: Hint = Hint { commands: &[Command::GoTo], counted: true, says: Says::Nothing };

/// The reminders the status line starts with, until the first key: shorter and shorter, for a
/// narrow window. In a list:
pub(super) const LIST_HINTS: [&[Hint]; 4] = [
    &[hint(&TILES, Says::Word("move")), OPENS, ZOOMS, PER_ROWS, QUITS],
    &[hint(&TILES, Says::Word("move")), OPENS, ZOOMS, QUITS],
    &[hint(&TILES, Says::Nothing), OPENS, QUITS],
    &[OPENS, QUITS],
];
/// In a file's pages, gone into from a list or through a link:
pub(super) const OPENED_HINTS: [&[Hint]; 4] =
    [&[LEAVES, SCREENS, TURNS, ZOOMS, PER_ROWS, QUITS], &[LEAVES, SCREENS, TURNS, ZOOMS, QUITS], &[LEAVES, TURNS, QUITS], &[LEAVES, QUITS]];
/// And in a file's pages otherwise.
pub(super) const PAGE_HINTS: [&[Hint]; 4] =
    [&[MOVES, TURNS, ENDS, GOES, ZOOMS, PER_ROWS, QUITS], &[MOVES, TURNS, ENDS, GOES, ZOOMS, QUITS], &[TURNS, ZOOMS, QUITS], &[TURNS, QUITS]];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Numbers;
    use crate::source::tests::cycled;

    static THEME: Setting = cycled("theme", 'D', "--theme");
    static WIDTH: Setting =
        Setting { keys: Keys::Step('[', ']'), numbers: Some(Numbers { least: 20, most: 200, step: 4, unit: "" }), ..cycled("width", '[', "--width") };
    static FONT: Setting = Setting { keys: Keys::Zoom, ..cycled("font", '+', "--font") };

    /// Every line `?` may show in `mode`.
    fn lines(mode: Mode) -> Vec<Line> {
        let own = match mode {
            Mode::Pages => [PAGE_LINES, &[COLUMNS_LINE]].concat(),
            Mode::List => LIST_LINES.to_vec(),
        };
        [own, BACK_LINES.to_vec(), QUIT_LINES.to_vec(), vec![HELP_LINE]].concat()
    }

    /// The catalog's names are what a config file will call the commands by: kebab-case, each its
    /// own, and a setting's built from the setting's.
    #[test]
    fn every_command_has_a_name_of_its_own_in_kebab_case() {
        let settings = [Command::Setting(&THEME, Turn::Next), Command::Setting(&WIDTH, Turn::Down), Command::Setting(&WIDTH, Turn::Up)];
        assert_eq!(settings.map(Command::name), ["cycle-theme", "decrease-width", "increase-width"]);
        let names: Vec<String> = VIEWER.iter().chain(&settings).map(|command| command.name()).collect();
        for name in &names {
            let words: Vec<&str> = name.split('-').collect();
            assert!(words.iter().all(|word| !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase())), "{name}");
            assert_eq!(names.iter().filter(|other| *other == name).count(), 1, "{name}");
        }
        // A setting's commands are the same command whichever backend's declaration they came from.
        static AGAIN: Setting = cycled("theme", 'D', "--theme");
        assert_eq!(Command::Setting(&THEME, Turn::Next), Command::Setting(&AGAIN, Turn::Next));
        assert_ne!(Command::Setting(&WIDTH, Turn::Down), Command::Setting(&WIDTH, Turn::Up));
        assert_ne!(Command::Setting(&THEME, Turn::Next), Command::Setting(&WIDTH, Turn::Next));
        assert_ne!(Command::Quit, Command::Leave);
    }

    /// In a keymap, a key goes to one command, and every command of the catalog is bound where it
    /// applies and nowhere else: so `?` has a key for each.
    #[test]
    fn the_default_keymaps_bind_each_key_once_and_every_command_where_it_applies() {
        let (keymaps, wrong) = Keymaps::build(&[]);
        assert_eq!(wrong, Vec::<String>::new());
        for (mode, table) in [(Mode::Pages, PAGES), (Mode::List, LIST)] {
            let keymap = keymaps.of(mode);
            let chords: Vec<(Key, bool)> = table.iter().flat_map(|(_, chords)| chords.iter().map(|chord| (chord.key, chord.counted))).collect();
            for chord in &chords {
                assert_eq!(chords.iter().filter(|other| *other == chord).count(), 1, "{chord:?} in {mode:?}");
                // The digits are the count's, and so are Esc and Backspace with one typed.
                assert!(!matches!(chord, (Key::Char('0'..='9'), _) | (Key::Escape | Key::Backspace, true)), "{chord:?} in {mode:?}");
            }
            assert_eq!(keymap.bindings.len(), chords.len(), "{mode:?}");
            for command in VIEWER {
                let bound = keymap.bindings.iter().any(|(_, bound)| bound == command);
                assert_eq!(bound, command.applies(mode), "{} in {mode:?}", command.name());
                let listed = lines(mode).iter().any(|line| line.commands.contains(command));
                assert_eq!(listed, command.applies(mode), "{} in {mode:?}'s `?`", command.name());
            }
            assert!(keymap.bindings.iter().all(|(_, command)| VIEWER.contains(command)), "{mode:?}");
        }
        assert!(VIEWER.iter().all(|command| command.applies(Mode::Pages) || command.applies(Mode::List)));
        // What it would say: a key bound twice, and a command that means nothing there.
        let table: &[(Command, &[Chord])] = &[(Command::Quit, &[ch('q')]), (Command::Help, &[ch('q')]), (Command::Open, &[ch('o')])];
        let mut wrong = Vec::new();
        let keymap = Keymap::defaults(Mode::Pages, table, &mut wrong);
        assert_eq!(wrong.len(), 2, "{wrong:?}");
        assert_eq!((keymap.command(Key::Char('q'), false), keymap.command(Key::Char('o'), false)), (Some(Command::Quit), None));
    }

    /// A key is looked up as it is pressed: with a number typed before it, a key bound with one is
    /// that command, and any other is what it is without. A key bound to nothing is no command, Ctrl
    /// and Alt keys among them.
    #[test]
    fn a_key_is_its_command_and_with_a_number_the_one_bound_with_one() {
        let keymaps = Keymaps::default();
        let (pages, list) = (keymaps.of(Mode::Pages), keymaps.of(Mode::List));
        assert_eq!((pages.command(Key::Enter, false), pages.command(Key::Enter, true)), (Some(Command::ScrollDown), Some(Command::GoTo)));
        assert_eq!((list.command(Key::Enter, false), list.command(Key::Enter, true)), (Some(Command::Open), Some(Command::GoTo)));
        assert_eq!((pages.command(Key::Char('g'), false), pages.command(Key::Char('g'), true)), (Some(Command::GoTo), Some(Command::GoTo)));
        assert_eq!((pages.command(Key::Char('j'), true), list.command(Key::Char('j'), true)), (Some(Command::ScrollDown), Some(Command::TileDown)));
        assert_eq!((pages.command(Key::Char('='), false), pages.command(Key::Char('_'), false)), (Some(Command::ZoomIn), Some(Command::ZoomOut)));
        // `d` and `u` a screen, and Emacs's keys beside the ones they are: no key goes back but
        // Backspace, and `u` is none of them.
        for (key, in_pages, in_list) in [
            (Key::Char('d'), Command::ScreenDown, Command::ScreenDown),
            (Key::Char('u'), Command::ScreenUp, Command::ScreenUp),
            (Key::Ctrl('v'), Command::ScreenDown, Command::ScreenDown),
            (Key::Alt('v'), Command::ScreenUp, Command::ScreenUp),
            (Key::Ctrl('n'), Command::ScrollDown, Command::TileDown),
            (Key::Ctrl('p'), Command::ScrollUp, Command::TileUp),
            (Key::Backspace, Command::Back, Command::Back),
        ] {
            assert_eq!((pages.command(key, false), list.command(key, false)), (Some(in_pages), Some(in_list)), "{key:?}");
        }
        assert_eq!((list.command(Key::Ctrl('f'), false), list.command(Key::Ctrl('b'), false)), (Some(Command::TileRight), Some(Command::TileLeft)));
        assert_eq!((pages.command(Key::Ctrl('f'), false), pages.command(Key::Ctrl('b'), false)), (None, None), "no tiles in a file's pages");
        for keymap in [pages, list] {
            assert_eq!(keymap.command(Key::Ctrl('l'), false), Some(Command::Rerender));
            assert_eq!(keymap.command(Key::Interrupt, true), Some(Command::Quit));
            for key in
                [Key::Char('x'), Key::Char('é'), Key::Ctrl('a'), Key::Alt('q'), Key::Alt('n'), Key::Ctrl('x'), Key::Missing(3), Key::Clipboard(true)]
            {
                assert_eq!(keymap.command(key, false), None, "{key:?}");
            }
        }
    }

    /// A setting's commands are bound to the keys it suggests, in both keymaps, after the viewer's:
    /// a key of the viewer's, in either keymap, stays the viewer's, a key two settings suggest stays
    /// the first's, and each is said. A setting on the zoom keys has no command of its own.
    #[test]
    fn a_settings_keys_are_suggestions_the_keymaps_take_after_the_viewers() {
        let (keymaps, wrong) = Keymaps::build(&[&THEME, &WIDTH, &FONT]);
        assert_eq!(wrong, Vec::<String>::new());
        for mode in [Mode::Pages, Mode::List] {
            let keymap = keymaps.of(mode);
            assert_eq!(keymap.command(Key::Char('D'), false), Some(Command::Setting(&THEME, Turn::Next)), "{mode:?}");
            assert_eq!(keymap.command(Key::Char('['), true), Some(Command::Setting(&WIDTH, Turn::Down)), "{mode:?}");
            assert_eq!(keymap.command(Key::Char(']'), false), Some(Command::Setting(&WIDTH, Turn::Up)), "{mode:?}");
            assert_eq!(keymap.command(Key::Char('+'), false), Some(Command::ZoomIn), "{mode:?}");
            assert!(!keymap.bindings.iter().any(|(_, command)| matches!(command, Command::Setting(setting, _) if setting.name == "font")));
            assert_eq!(
                (keymap.each(&[Command::Setting(&THEME, Turn::Next)]), keymap.numbered(12, Command::Setting(&WIDTH, Turn::Up))),
                ("D".into(), "12]".into())
            );
        }
        assert_eq!((Command::Setting(&THEME, Turn::Next).count(), Command::Setting(&WIDTH, Turn::Up).count()), (Count::Left, Count::Taken));
        // What it would say.
        static QUITS: Setting = cycled("tint", 'q', "--tint");
        static LEFT: Setting = cycled("tint", 'h', "--tint");
        static DIGIT: Setting = cycled("tint", '7', "--tint");
        static EQUALS: Setting = cycled("tint", '=', "--tint");
        static OTHER: Setting = cycled("tint", 'D', "--tint");
        static PLUS: Setting = Setting { keys: Keys::Step('-', '+'), ..cycled("font", '+', "--font") };
        // The one name whose commands are called as the viewer's are: `increase-per-row`.
        static ROW: Setting = Setting { keys: Keys::Step('<', '>'), ..cycled("per-row", '<', "--per-row") };
        assert!(wrong_keys(&[&THEME, &THEME]).is_empty(), "one setting in two backends");
        for (settings, said) in [
            (&[&QUITS][..], 1),
            // Free in a file's pages, and a tile's key in a list: the viewer's.
            (&[&LEFT], 1),
            (&[&DIGIT], 1),
            (&[&EQUALS], 1),
            (&[&THEME, &OTHER], 1),
            (&[&PLUS], 2),
            (&[&ROW], 2),
        ] {
            assert_eq!(wrong_keys(settings).len(), said, "{:?}", wrong_keys(settings));
        }
        let keymaps = Keymaps::new(&[&QUITS, &LEFT, &THEME, &OTHER]);
        for mode in [Mode::Pages, Mode::List] {
            let keymap = keymaps.of(mode);
            assert_eq!(keymap.command(Key::Char('q'), false), Some(Command::Quit));
            assert_eq!(keymap.command(Key::Char('h'), false), (mode == Mode::List).then_some(Command::TileLeft));
            assert_eq!(keymap.command(Key::Char('D'), false), Some(Command::Setting(&THEME, Turn::Next)));
        }
    }

    fn binding(mode: Option<Mode>, key: &str, command: &str, line: usize) -> Binding {
        Binding { mode, key: key.into(), command: command.into(), line, at: line }
    }

    /// A config's keys are bound after the defaults and the settings' suggestions. A key bound
    /// loses what it was bound to, and comes first among the keys of its command, in the order of
    /// the file; a key bound as it is by default keeps its place. `[keys]` binds wherever the
    /// command applies, the table of a keymap wins there, and `none` unbinds. A key bound with a
    /// number typed before it, and `ctrl-c`, stay as they are.
    #[test]
    fn a_configs_keys_come_first_among_their_commands_and_take_the_defaults_place() {
        let settings: [&'static Setting; 2] = [&THEME, &WIDTH];
        assert_eq!(Keymaps::bound(&settings, &[]), Ok(Keymaps::new(&settings)));
        let keymaps = Keymaps::bound(
            &settings,
            &[
                binding(None, "x", "quit", 1),
                binding(None, "ctrl-d", "screen-down", 2),
                binding(None, "q", "none", 3),
                binding(None, "/", "search", 4),
                binding(None, "X", "quit", 5),
                binding(None, "S", "search", 6),
                binding(None, "D", "quit", 7),
                binding(None, "w", "increase-width", 8),
                binding(Some(Mode::List), "J", "list-tile-down", 9),
                binding(Some(Mode::List), "x", "list-open", 10),
                binding(None, "enter", "next", 11),
                binding(None, "=", "zoom-in", 12),
                binding(None, "y", "none", 13),
            ],
        )
        .unwrap();
        let (pages, list) = (keymaps.of(Mode::Pages), keymaps.of(Mode::List));
        let key = |c| Key::Char(c);
        // Bound wherever the command applies; the table of a keymap wins there.
        assert_eq!((pages.command(key('x'), false), list.command(key('x'), false)), (Some(Command::Quit), Some(Command::Open)));
        assert_eq!((pages.command(key('S'), false), list.command(key('S'), false)), (Some(Command::Search), None));
        assert_eq!((pages.command(key('J'), false), list.command(key('J'), false)), (None, Some(Command::TileDown)));
        assert_eq!((pages.command(key('/'), false), list.command(key('/'), false)), (Some(Command::Search), Some(Command::Filter)));
        // Unbound, and a key bound to nothing unbound again.
        assert_eq!([pages.command(key('q'), false), list.command(key('q'), false), pages.command(key('y'), false)], [None; 3]);
        // First among the keys of its command, in the order of the file; the others keep theirs.
        assert_eq!((pages.each(&[Command::Quit]), list.each(&[Command::Quit])), ("x  X  D  ctrl-c".to_owned(), "X  D  ctrl-c".to_owned()));
        assert_eq!(pages.each(&[Command::ScreenDown]), "ctrl-d  space  pagedown  d  ctrl-v");
        assert_eq!(list.each(&[Command::ScreenDown]), "ctrl-d  space  n  d  ctrl-v  pagedown");
        assert_eq!((pages.each(&[Command::Search]), list.each(&[Command::TileDown])), ("S  /".to_owned(), "J  ↓  j  ctrl-n".to_owned()));
        assert_eq!(list.each(&[Command::Open]), "x  enter");
        // A key bound as it is by default keeps its place, and one that `?` doesn't list stays so.
        assert_eq!((pages.shown(&ZOOM_LINE), pages.command(key('='), false)), ("+ -".to_owned(), Some(Command::ZoomIn)));
        // Enter alone is bound again: with a number it still goes to the page, and a list's opens.
        assert_eq!((pages.command(Key::Enter, false), pages.command(Key::Enter, true)), (Some(Command::Next), Some(Command::GoTo)));
        assert_eq!((list.command(Key::Enter, false), list.command(Key::Enter, true)), (Some(Command::Open), Some(Command::GoTo)));
        assert_eq!((pages.each(&[Command::ScrollDown]), pages.each(&[Command::Next])), ("j  ↓  ctrl-n".to_owned(), "enter  n".to_owned()));
        assert_eq!(pages.first_counted(Command::GoTo), "enter");
        // A setting's suggestion is left out where the config binds its key, and its commands are
        // bound by their names.
        for keymap in [pages, list] {
            assert_eq!(keymap.command(key('D'), false), Some(Command::Quit));
            assert!(keymap.each(&[Command::Setting(&THEME, Turn::Next)]).is_empty());
            assert_eq!(keymap.each(&[Command::Setting(&WIDTH, Turn::Up)]), "w  ]");
            assert_eq!(keymap.fill("72{increase-width} sets 72"), "72w sets 72");
            assert_eq!(keymap.command(Key::Interrupt, false), Some(Command::Quit), "ctrl-c still quits");
        }
        // What names a key shows it as bound: `?`, the reminders, a line's text.
        assert_eq!(pages.shown(&QUIT_LINES[0]), "x  esc  X  D  ctrl-c");
        assert!(pages.reminder(PAGE_HINTS[0], "zoom").ends_with(" · x quit"), "{}", pages.reminder(PAGE_HINTS[0], "zoom"));
        assert!(pages.fill(PAGE_LINES[9].says).contains("3enter the third on"), "{}", pages.fill(PAGE_LINES[9].says));
    }

    /// What a config can't bind is refused, with the line that binds it and why: a command that
    /// doesn't apply in the keymap of its table, a command that doesn't exist, a digit, `ctrl-c`,
    /// and a name that is no key's.
    #[test]
    fn what_a_config_cant_bind_is_refused_with_its_line_and_why() {
        for (binding, why) in [
            (binding(Some(Mode::List), "s", "search", 3), "s: search does not apply in a list"),
            (binding(Some(Mode::Pages), "x", "list-open", 3), "x: list-open does not apply in a file's pages"),
            (binding(None, "x", "frobnicate", 3), "x: no command is called frobnicate"),
            (binding(None, "x", "paper", 3), "x: no command is called paper"),
            (binding(None, "x", "cycle-fences", 3), "x: no command is called cycle-fences"),
            (binding(None, "x", "None", 3), "x: no command is called None"),
            (binding(None, "x", "", 3), "x: no command is called "),
            (binding(None, "5", "quit", 3), "5: a digit starts the number typed before a key, so it can't be bound"),
            (binding(None, "0", "none", 3), "0: a digit starts the number typed before a key, so it can't be bound"),
            (binding(None, "ctrl-c", "none", 3), "ctrl-c: it always quits, so it can't be bound"),
            (binding(None, "ctrl-c", "quit", 3), "ctrl-c: it always quits, so it can't be bound"),
            (binding(None, "ctrl-D", "quit", 3), "ctrl-D: write ctrl-d: a terminal sends the same byte for both"),
            (binding(None, "ctrl-m", "quit", 3), "ctrl-m: write enter: a terminal sends the same byte for both"),
            (binding(None, "tab", "quit", 3), "tab: clv does not read Tab as a key"),
            (binding(None, "f5", "quit", 3), "f5: clv reads no key of that name"),
        ] {
            assert_eq!(Keymaps::bound(&[&THEME], std::slice::from_ref(&binding)), Err((3, why.to_owned())), "{binding:?}");
        }
        // The first that is wrong, in the order of the file.
        let both = [binding(None, "x", "quit", 1), binding(None, "tab", "quit", 2), binding(None, "5", "quit", 3)];
        assert!(Keymaps::bound(&[], &both).is_err_and(|(line, _)| line == 2));
    }

    proptest::proptest! {
        /// Whatever a config binds, the keymaps are refused or they hold: a key is bound once in a
        /// keymap, to a command that applies there; `ctrl-c` quits; a number and Enter go to the
        /// page; and no digit is bound.
        #[test]
        fn whatever_a_config_binds_the_keymaps_hold(
            bindings in proptest::collection::vec(
                (
                    proptest::option::of(proptest::bool::ANY),
                    "(x|q|j|J|/|=|D|\\]|enter|esc|space|up|ctrl-d|alt-v|ctrl-c|5|tab|ctrl-D|\\PC{0,3})",
                    "(quit|none|search|next|go-to|zoom-in|list-open|list-tile-down|cycle-theme|increase-width|screen-down|\\PC{0,6})",
                ),
                0..10,
            ),
        ) {
            let mode = |list: Option<bool>| list.map(|list| if list { Mode::List } else { Mode::Pages });
            let bindings: Vec<Binding> =
                bindings.iter().enumerate().map(|(at, (list, key, command))| binding(mode(*list), key, command, at)).collect();
            if let Ok(keymaps) = Keymaps::bound(&[&THEME, &WIDTH], &bindings) {
                for mode in [Mode::Pages, Mode::List] {
                    let keymap = keymaps.of(mode);
                    for (chord, command) in &keymap.bindings {
                        let same = keymap.bindings.iter().filter(|(other, _)| other.key == chord.key && other.counted == chord.counted).count();
                        proptest::prop_assert_eq!(same, 1, "{:?} in {:?}", chord, mode);
                        proptest::prop_assert!(command.applies(mode), "{} in {:?}", command.name(), mode);
                        proptest::prop_assert!(!matches!(chord.key, Key::Char('0'..='9')));
                    }
                    proptest::prop_assert_eq!(keymap.command(Key::Interrupt, false), Some(Command::Quit));
                    proptest::prop_assert_eq!(keymap.command(Key::Enter, true), Some(Command::GoTo));
                }
            }
        }
    }

    /// `?`'s keys and the status line's reminders are read from the keymap: with the defaults, as
    /// they were when they were written by hand, and with other keys bound, those.
    #[test]
    fn the_keys_listed_and_the_reminders_are_read_from_the_keymap() {
        let keymaps = Keymaps::default();
        let (pages, list) = (keymaps.of(Mode::Pages), keymaps.of(Mode::List));
        let shown = |keymap: &Keymap, lines: &[Line]| lines.iter().map(|line| keymap.shown(line)).collect::<Vec<_>>();
        assert_eq!(
            shown(pages, PAGE_LINES),
            [
                "space  pagedown  d  ctrl-v",
                "b  pageup  u  alt-v",
                "j k  ↓ ↑  ctrl-n ctrl-p  enter",
                "n p  → ←",
                "g G  home end",
                "12 enter  12g",
                "+ -",
                ", .  3.",
                "/",
                "n  N p  3n",
                "f",
                "F",
                "T",
                "backspace",
                "r  ctrl-l",
                "c",
                "o"
            ]
        );
        assert_eq!(
            shown(list, LIST_LINES),
            [
                "← →  h l  ctrl-b ctrl-f",
                "↑ ↓  k j  ctrl-p ctrl-n",
                "space b  n p  d u  ctrl-v alt-v  pagedown pageup",
                "g G  home end",
                "12 enter  12g",
                "enter",
                "/",
                "+ -",
                ", .  3.",
                "r  ctrl-l",
                "c",
                "o"
            ]
        );
        assert_eq!(shown(pages, &[COLUMNS_LINE, HELP_LINE]), [", .  3.", "?"]);
        assert_eq!(
            (shown(pages, BACK_LINES), shown(list, QUIT_LINES)),
            (vec!["esc  backspace".to_owned(), "q  ctrl-c".to_owned()], vec!["q  esc  ctrl-c".to_owned()])
        );
        // What a line says names keys through the keymap too.
        assert_eq!(pages.fill(PER_ROW_LINE.says), "one more page to a row, or one fewer, then the width; 3. sets 3, 0. auto");
        assert_eq!(pages.fill("{next} {previous-match}{go-to} {nothing}{"), "n Ng ");
        for mode in [Mode::Pages, Mode::List] {
            for line in lines(mode) {
                let filled = keymaps.of(mode).fill(line.says);
                assert!(!filled.contains(['{', '}']), "{filled}");
                // Every name between braces is a command's, bound there.
                for name in line.says.split('{').skip(1).filter_map(|rest| rest.split_once('}')).map(|(name, _)| name) {
                    let command = VIEWER.iter().find(|command| command.name() == name).unwrap_or_else(|| panic!("{name} in {:?}", line.says));
                    assert!(!keymaps.of(mode).first(*command).is_empty(), "{name} in {mode:?}");
                }
            }
        }
        let reminders = |keymap: &Keymap, hints: [&[Hint]; 4]| hints.map(|hints| keymap.reminder(hints, "zoom"));
        assert_eq!(
            reminders(list, LIST_HINTS),
            [
                "arrows move · ⏎ open · + - zoom · , . per row · q quit",
                "arrows move · ⏎ open · + - zoom · q quit",
                "arrows · ⏎ open · q quit",
                "⏎ open · q quit"
            ]
        );
        assert_eq!(
            reminders(pages, OPENED_HINTS),
            [
                "esc back · space b · n p · + - zoom · , . per row · q quit",
                "esc back · space b · n p · + - zoom · q quit",
                "esc back · n p · q quit",
                "esc back · q quit"
            ]
        );
        assert_eq!(
            reminders(pages, PAGE_HINTS),
            [
                "space b j k · n p · g G · 12⏎ · + - zoom · , . per row · q quit",
                "space b j k · n p · g G · 12⏎ · + - zoom · q quit",
                "n p · + - zoom · q quit",
                "n p · q quit"
            ]
        );
        assert!(pages.reminder(PAGE_HINTS[0], "font").contains(" · + - font · "));
        // Other keys bound: those are the ones said, and a command with none is left out.
        let table: &[(Command, &[Chord])] = &[
            (Command::Quit, &[key(Key::Ctrl('x')), ch('q')]),
            (Command::TileLeft, &[ch('h')]),
            (Command::TileRight, &[key(Key::Right)]),
            (Command::TileUp, &[key(Key::Up)]),
            (Command::TileDown, &[key(Key::Alt('n'))]),
            (Command::GoTo, &[counted(Key::Char(' ')), ch('t')]),
        ];
        let other = Keymap::defaults(Mode::List, table, &mut Vec::new());
        assert_eq!(other.reminder(LIST_HINTS[0], "zoom"), "h → ↑ alt-n move · ctrl-x quit");
        assert_eq!(shown(&other, &[LIST_LINES[4], QUIT_LINES[0], LIST_LINES[5]]), ["12 space  12t", "ctrl-x  q", ""]);
        assert_eq!(other.fill(QUIT_LINES[0].says), "quit");
    }
}
