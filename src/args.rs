//! Command-line options, the catalog that lists them, and page lists.
//!
//! The catalog ([`Catalog`]) lists every option of a build once: the viewer's own, each backend's
//! own, and each setting's. An entry says who declares the option, the shape of its value and
//! whether a config may set it, and one parser reads a value against it ([`Entry::read`]),
//! wherever the value came from.

use crate::source::{Numbers, Setting, Value};

/// An option: its `name` (`--per-row`), the `short` one it has too (`-r`), what its value looks like
/// (`None` for one that takes none) and what it does, as `--help` shows them; the `alias` it is
/// still taken by, an old name `--help` doesn't show (`--zoom`); the shape of its value; and
/// whether a config may set it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OptionSpec {
    pub(crate) name: &'static str,
    pub(crate) short: Option<&'static str>,
    pub(crate) value: Option<&'static str>,
    pub(crate) help: &'static str,
    pub(crate) alias: Option<&'static str>,
    pub(crate) shape: Shape,
    pub(crate) config: Settable,
}

/// The shape of an option's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    /// No value: the option is given, or it is not.
    Flag,
    /// One of some named values, a number in a range, or either: `auto|fast|small`, `50-400`,
    /// `auto|width|1-8`. The first named value is the option's default.
    Value { named: &'static [&'static str], numbers: Option<Numbers> },
    /// What its setting takes: its named values, `auto` where it goes by the terminal's background,
    /// and its numbers. The shape of a setting's option, whatever its backend writes: the catalog
    /// reads the setting ([`Entry::named`], [`Entry::numbers`]), so that a backend declares what a
    /// setting takes once.
    Setting,
    /// A page list: `1,3-5`.
    Pages,
    /// Free text: a password, a path.
    Text,
}

/// Whether a config may set an option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Settable {
    /// It may.
    Yes,
    /// It may not, and why, as the reader is told: `it belongs to one run`.
    No(&'static str),
}

impl OptionSpec {
    /// The option as `--help` lists it: `--per-row auto|width|1-8`, `-r, --recurse`; never its alias.
    pub(crate) fn shown(&self) -> String {
        let name = match self.short {
            Some(short) => format!("{short}, {}", self.name),
            None => self.name.into(),
        };
        match self.value {
            Some(value) => format!("{name} {value}"),
            None => name,
        }
    }

    /// Its name in a config: its name without the dashes, `per-row`.
    pub(crate) fn key(&self) -> &'static str {
        self.name.trim_start_matches('-')
    }
}

/// Who declares an option.
#[derive(Clone, Copy, Debug)]
pub(crate) enum By {
    /// The viewer: the option holds whatever is shown.
    Viewer,
    /// A backend, by its name: the option is for its files.
    Backend(&'static str),
    /// A setting, and the backend that declares it so, by its name. Two backends may declare one
    /// setting, each with its own help line: the first is the setting's.
    Setting(&'static Setting, &'static str),
}

/// An option as the catalog lists it: what it is, and who declares it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Entry {
    pub(crate) spec: OptionSpec,
    pub(crate) by: By,
}

/// The value of an option, read ([`Entry::read`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Arg<'a> {
    /// One of the option's named values, as the option names it.
    Named(&'static str),
    Number(u32),
    /// 0-based pages, in the order given.
    Pages(Vec<usize>),
    Text(&'a str),
}

impl Entry {
    /// The named values it takes: for a setting's option, `auto` where the setting goes by the
    /// terminal's background, then the setting's named values.
    pub(crate) fn named(&self) -> Vec<&'static str> {
        match (self.spec.shape, self.by) {
            (Shape::Setting, By::Setting(setting, _)) => {
                let auto = setting.auto.map(|_| AUTO);
                auto.into_iter().chain(setting.named.iter().map(|named| named.name)).collect()
            }
            (Shape::Value { named, .. }, _) => named.to_vec(),
            _ => Vec::new(),
        }
    }

    /// The numbers it takes, if it takes any.
    pub(crate) fn numbers(&self) -> Option<Numbers> {
        match (self.spec.shape, self.by) {
            (Shape::Setting, By::Setting(setting, _)) => setting.numbers,
            (Shape::Value { numbers, .. }, _) => numbers,
            _ => None,
        }
    }

    /// Whether it takes no value.
    pub(crate) fn is_flag(&self) -> bool {
        self.spec.shape == Shape::Flag
    }

    /// The value it has when it is not given, as it would be given: a setting's `auto` where it goes
    /// by the terminal's background, or else its default; the first named value of another option,
    /// or else its least number. None for a flag, a page list and free text.
    pub(crate) fn default(&self) -> Option<String> {
        match (self.spec.shape, self.by) {
            (Shape::Setting, By::Setting(setting, _)) => Some(match (setting.auto, setting.default) {
                (Some(_), _) => AUTO.to_owned(),
                (None, Value::Named(named)) => named.to_owned(),
                (None, Value::Number(number)) => number.to_string(),
            }),
            (Shape::Value { named, numbers }, _) => {
                named.first().map(|named| (*named).to_owned()).or(numbers.map(|numbers| numbers.least.to_string()))
            }
            _ => None,
        }
    }

    /// What its value may be, in words: `auto, width or 1–8`, `50–400%`.
    pub(crate) fn takes(&self) -> String {
        let range = self.numbers().map(|numbers| format!("{}–{}{}", numbers.least, numbers.most, numbers.unit));
        let takes: Vec<String> = self.named().into_iter().map(str::to_owned).chain(range).collect();
        match (self.spec.shape, takes.split_last()) {
            (Shape::Pages, _) => "pages, as 1,3-5".into(),
            (Shape::Text, _) => "text".into(),
            (_, Some((last, []))) => last.clone(),
            (_, Some((last, rest))) => format!("{} or {last}", rest.join(", ")),
            (_, None) => "nothing".into(),
        }
    }

    /// `given` read as the value of this option: the one parser of a value, from the command line
    /// and from a config alike. A value the option doesn't take is the error, which says what it
    /// takes, and the same wherever the value came from: `expected window or 20–200, got 300`.
    /// The caller says which option, as the reader gave it.
    pub(crate) fn read<'a>(&self, given: &'a str) -> Result<Arg<'a>, String> {
        match self.spec.shape {
            Shape::Pages => return page_list(given).map(Arg::Pages),
            Shape::Text => return Ok(Arg::Text(given)),
            Shape::Flag | Shape::Value { .. } | Shape::Setting => {}
        }
        if let Some(named) = self.named().into_iter().find(|named| *named == given) {
            return Ok(Arg::Named(named));
        }
        if let (Some(numbers), Ok(number)) = (self.numbers(), given.parse::<u32>())
            && (numbers.least..=numbers.most).contains(&number)
        {
            return Ok(Arg::Number(number));
        }
        Err(format!("expected {}, got {given}", self.takes()))
    }
}

/// The value a setting's option takes where the setting goes by the terminal's background.
pub(crate) const AUTO: &str = "auto";

/// Every option of a build, each with who declares it: the viewer's, then each backend's own and
/// its settings', in the backends' order. `--help`, the command line and a config are all read
/// against it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Catalog(Vec<Entry>);

impl Catalog {
    /// The viewer's options alone.
    pub(crate) fn of(viewer: &[OptionSpec]) -> Self {
        Self(viewer.iter().map(|spec| Entry { spec: *spec, by: By::Viewer }).collect())
    }

    /// Adds the options of the backend called `backend`: its own, then its settings'.
    pub(crate) fn declare(&mut self, backend: &'static str, options: &[OptionSpec], settings: &'static [Setting]) {
        self.0.extend(options.iter().map(|spec| Entry { spec: *spec, by: By::Backend(backend) }));
        // A setting's option has the setting's shape, whatever its backend wrote.
        let option = |setting: &'static Setting| OptionSpec { shape: Shape::Setting, ..setting.option };
        self.0.extend(settings.iter().map(|setting| Entry { spec: option(setting), by: By::Setting(setting, backend) }));
    }

    /// Every entry, in order: an option two backends declare is there twice, as each declares it.
    pub(crate) fn entries(&self) -> &[Entry] {
        &self.0
    }

    /// Each option once, as the first to declare it does.
    pub(crate) fn options(&self) -> impl Iterator<Item = &Entry> {
        let first = |(at, entry): &(usize, &Entry)| !self.0.iter().take(*at).any(|before| before.spec.name == entry.spec.name);
        self.0.iter().enumerate().filter(first).map(|(_, entry)| entry)
    }

    /// The option called `name`, as the first to declare it does.
    pub(crate) fn entry(&self, name: &str) -> Option<&Entry> {
        self.0.iter().find(|entry| entry.spec.name == name)
    }

    /// The option `arg` asks for on a command line: by its name, its short name or its alias.
    fn asked(&self, arg: &str) -> Option<&Entry> {
        self.0.iter().find(|entry| entry.spec.name == arg || entry.spec.short == Some(arg) || entry.spec.alias == Some(arg))
    }
}

/// A command line read against the options it may hold, before any file is read, and then the
/// options a config sets ([`Parsed::configure`]): the command line's come first, so an option on
/// the command line wins over the same option in the config.
#[derive(Clone, Debug, Default)]
pub(crate) struct Parsed<'a> {
    /// The arguments that are neither an option nor an option's value, in order.
    pub(crate) operands: Vec<&'a str>,
    /// Each option given, in order, with its value if it takes one: the command line's, then a
    /// config's.
    given: Vec<(&'static str, Option<&'a str>)>,
    /// The options given by their alias, by their name: `--zoom` for `--per-row`.
    aliased: Vec<&'static str>,
    /// How many of `given` the command line gave: the first ones. The rest are a config's.
    typed: usize,
}

impl<'a> Parsed<'a> {
    /// Refuses an option not in `catalog`, and one left without its value. An option that takes a
    /// value takes the next argument, whatever it is: a password can be anything. An argument
    /// that starts with `-` is an option, but for `-` alone; `./-file` names a file that does. An
    /// option given by its alias is that option, and given by both its name and its alias, refused.
    pub(crate) fn parse(args: &'a [String], catalog: &Catalog) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut rest = args.iter();
        let mut by_name = Vec::new();
        while let Some(arg) = rest.next() {
            if !arg.starts_with('-') || arg == "-" {
                parsed.operands.push(arg);
                continue;
            }
            let entry = catalog.asked(arg).ok_or_else(|| format!("unknown option {arg}"))?;
            let spec = &entry.spec;
            let value = match entry.is_flag() {
                false => Some(rest.next().ok_or_else(|| format!("{arg} needs a value"))?.as_str()),
                true => None,
            };
            if let Some(alias) = spec.alias {
                let (by, other) = if alias == arg { (&mut parsed.aliased, &by_name) } else { (&mut by_name, &parsed.aliased) };
                if other.contains(&spec.name) {
                    return Err(format!("{} and {alias} are one option: give one of them", spec.name));
                }
                if !by.contains(&spec.name) {
                    by.push(spec.name);
                }
            }
            parsed.given.push((spec.name, value));
        }
        parsed.typed = parsed.given.len();
        Ok(parsed)
    }

    /// Option `name` as a config sets it, with its value if it takes one: after the command line's,
    /// so the first value given still wins.
    pub(crate) fn configure(&mut self, name: &'static str, value: Option<&'a str>) {
        self.given.push((name, value));
    }

    /// Whether option `name` goes by a config: it is given, and not on the command line.
    pub(crate) fn by_config(&self, name: &str) -> bool {
        self.given.iter().position(|(given, _)| *given == name).is_some_and(|at| at >= self.typed)
    }

    /// These options, without what a config says of option `name`: as the reader typed it, or not
    /// given.
    pub(crate) fn without_config(&self, name: &str) -> Self {
        let kept = |(at, (given, _)): &(usize, &(&'static str, Option<&'a str>))| *at < self.typed || *given != name;
        Self { given: self.given.iter().enumerate().filter(kept).map(|(_, given)| *given).collect(), ..self.clone() }
    }

    /// How option `name` was given, the first time: by its name, or by its alias (`--zoom`).
    pub(crate) fn typed(&self, name: &'static str, catalog: &Catalog) -> &'static str {
        let alias = catalog.entry(name).and_then(|entry| entry.spec.alias);
        match alias {
            Some(alias) if self.aliased.contains(&name) => alias,
            _ => name,
        }
    }

    /// The options given by an alias, by their name.
    pub(crate) fn aliased(&self) -> &[&'static str] {
        &self.aliased
    }

    /// The value option `name` was given, the first time, if it was.
    pub(crate) fn value(&self, name: &str) -> Option<&'a str> {
        self.given.iter().find(|(given, _)| *given == name).and_then(|(_, value)| *value)
    }

    pub(crate) fn has(&self, name: &str) -> bool {
        self.given.iter().any(|(given, _)| *given == name)
    }

    /// The options given, each as often as it was.
    pub(crate) fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.given.iter().map(|(name, _)| *name)
    }

    /// The options given, to outlast the command line: for a thread that opens files.
    pub(crate) fn owned(&self) -> OwnedOptions {
        OwnedOptions(self.given.iter().map(|(name, value)| (*name, value.map(str::to_owned))).collect())
    }
}

/// The options of a [`Parsed`], its own.
#[derive(Debug, Default)]
pub(crate) struct OwnedOptions(Vec<(&'static str, Option<String>)>);

impl OwnedOptions {
    /// As they were read, without the files.
    pub(crate) fn parsed(&self) -> Parsed<'_> {
        let given: Vec<_> = self.0.iter().map(|(name, value)| (*name, value.as_deref())).collect();
        Parsed { operands: Vec::new(), typed: given.len(), given, aliased: Vec::new() }
    }
}

/// The most pages one `A-B` range, or a whole list, may name: a bound on what a typed list can
/// allocate (repeating a range must not multiply it).
const MAX_RANGE: usize = 1_000_000;

/// 1-based pages ("1,3,5-8": single pages and inclusive ranges, in the order given) → 0-based
/// indices.
pub(crate) fn page_list(s: &str) -> Result<Vec<usize>, String> {
    let number = |t: &str, item: &str| t.trim().parse::<usize>().ok().filter(|n| *n > 0).ok_or_else(|| format!("bad page number {item:?}"));
    let mut out = Vec::new();
    for item in s.split(',').map(str::trim) {
        match item.split_once('-') {
            Some((from, to)) => {
                let (from, to) = (number(from, item)?, number(to, item)?);
                if to < from {
                    return Err(format!("bad page range {item:?}: it ends before it starts"));
                }
                if to - from >= MAX_RANGE {
                    return Err(format!("bad page range {item:?}: more than {MAX_RANGE} pages"));
                }
                if out.len().saturating_add(to - from + 1) > MAX_RANGE {
                    return Err(format!("bad page list: more than {MAX_RANGE} pages in all"));
                }
                out.extend(from - 1..to);
            }
            None if out.len() >= MAX_RANGE => return Err(format!("bad page list: more than {MAX_RANGE} pages in all")),
            None => out.push(number(item, item)? - 1),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_list_is_bounded_in_all_not_just_per_range() {
        assert_eq!(page_list("1,3-5"), Ok(vec![0, 2, 3, 4]));
        let full = format!("1-{MAX_RANGE}");
        assert_eq!(page_list(&full).map(|pages| pages.len()), Ok(MAX_RANGE));
        // Repeating a range that is allowed on its own would multiply what it allocates.
        assert!(page_list(&format!("{full},{full}")).is_err_and(|e| e.contains("in all")), "two full ranges");
        assert!(page_list(&format!("{full},7")).is_err_and(|e| e.contains("in all")), "a full range and a page");
    }

    #[test]
    fn a_bad_page_list_says_what_is_wrong() {
        assert!(page_list("0").is_err_and(|e| e.contains("bad page number")));
        assert!(page_list("5-3").is_err_and(|e| e.contains("ends before it starts")));
        assert!(page_list("1-x").is_err_and(|e| e.contains("bad page number")));
        assert!(page_list("").is_err());
    }

    const fn option(name: &'static str, value: Option<&'static str>, shape: Shape) -> OptionSpec {
        OptionSpec { name, short: None, value, help: "", alias: None, shape, config: Settable::Yes }
    }

    /// One parser reads a value, by the shape of its option: a named value, a number in the range,
    /// either, a page list, or free text. A value the option doesn't take is refused with what it
    /// takes, the same wherever the value came from.
    #[test]
    fn one_parser_reads_a_value_by_the_shape_of_its_option() {
        const NUMBERS: Numbers = Numbers { least: 1, most: 8, step: 1, unit: "" };
        const PERCENT: Numbers = Numbers { least: 50, most: 400, step: 10, unit: "%" };
        let entry = |shape| Entry { spec: option("--it", Some("X"), shape), by: By::Viewer };
        let either = entry(Shape::Value { named: &["auto", "width"], numbers: Some(NUMBERS) });
        assert_eq!((either.read("auto"), either.read("width")), (Ok(Arg::Named("auto")), Ok(Arg::Named("width"))));
        assert_eq!((either.read("1"), either.read("8")), (Ok(Arg::Number(1)), Ok(Arg::Number(8))));
        for wrong in ["0", "9", "65536", "-1", "page", "", "Auto", " 3", "4294967296"] {
            assert_eq!(either.read(wrong), Err(format!("expected auto, width or 1–8, got {wrong}")), "{wrong:?}");
        }
        let named = entry(Shape::Value { named: &["auto", "fast", "small"], numbers: None });
        assert_eq!((named.read("small"), named.read("3")), (Ok(Arg::Named("small")), Err("expected auto, fast or small, got 3".into())));
        let number = entry(Shape::Value { named: &[], numbers: Some(PERCENT) });
        assert_eq!((number.read("150"), number.read("500")), (Ok(Arg::Number(150)), Err("expected 50–400%, got 500".into())));
        assert_eq!((number.takes(), named.takes(), either.takes()), ("50–400%".into(), "auto, fast or small".into(), "auto, width or 1–8".into()));
        let pages = entry(Shape::Pages);
        assert_eq!(pages.read("1,3-5"), Ok(Arg::Pages(vec![0, 2, 3, 4])));
        assert!(pages.read("0").is_err_and(|e| e.contains("bad page number")));
        assert_eq!(entry(Shape::Text).read("--anything\x1b"), Ok(Arg::Text("--anything\x1b")), "a password can be anything");
        let flag = entry(Shape::Flag);
        assert!(flag.is_flag() && !either.is_flag() && flag.read("true").is_err());
        assert_eq!(
            [either.default(), named.default(), number.default(), flag.default()],
            [Some("auto".into()), Some("auto".into()), Some("50".into()), None]
        );
    }

    /// The catalog lists the viewer's options, then each backend's own and its settings', each
    /// with who declares it. A setting's option takes what the setting has, whatever shape its
    /// backend wrote: `auto` where it goes by the terminal's background, its named values and its
    /// numbers. An option two backends declare is listed under each, and is the first's.
    #[test]
    fn the_catalog_lists_each_option_with_who_declares_it_and_reads_a_settings_from_the_setting() {
        use crate::source::tests::cycled;
        static THEME: Setting = cycled("theme", 'D', "--theme");
        static WIDTH: Setting = Setting {
            named: &[crate::source::Named { name: "window", says: "" }],
            numbers: Some(Numbers { least: 20, most: 200, step: 4, unit: "" }),
            auto: None,
            option: option("--width", Some("window|N"), Shape::Text),
            ..cycled("width", '[', "--width")
        };
        static OURS: [Setting; 2] = [THEME, WIDTH];
        static THEIRS: [Setting; 1] = [Setting { option: OptionSpec { help: "theirs", ..THEME.option }, ..THEME }];
        let viewer = [option("--pages", Some("1,3-5"), Shape::Pages), option("--pager", None, Shape::Flag)];
        let password = OptionSpec { config: Settable::No("a secret"), ..option("--password", Some("PW"), Shape::Text) };
        let mut catalog = Catalog::of(&viewer);
        catalog.declare("ours", &[password], &OURS);
        catalog.declare("theirs", &[password], &THEIRS);
        let listed: Vec<(&str, String)> =
            catalog.entries().iter().map(|entry| (entry.spec.name, format!("{:?}", entry.by).chars().take(7).collect())).collect();
        assert_eq!(
            listed,
            [
                ("--pages", "Viewer".to_owned()),
                ("--pager", "Viewer".into()),
                ("--password", "Backend".into()),
                ("--theme", "Setting".into()),
                ("--width", "Setting".into()),
                ("--password", "Backend".into()),
                ("--theme", "Setting".into())
            ]
        );
        let theme = catalog.entry("--theme").unwrap();
        assert!(matches!(theme.by, By::Setting(setting, "ours") if setting.name == "theme") && theme.spec.help.is_empty(), "the first's");
        assert_eq!((theme.named(), theme.numbers(), theme.takes()), (vec!["auto", "light", "dark"], None, "auto, light or dark".to_owned()));
        assert_eq!((theme.read("auto"), theme.read("dark")), (Ok(Arg::Named("auto")), Ok(Arg::Named("dark"))));
        assert_eq!(theme.read("sepia"), Err("expected auto, light or dark, got sepia".into()));
        // Its backend wrote free text: the setting says what it takes all the same.
        let width = catalog.entry("--width").unwrap();
        assert_eq!((width.spec.shape, width.named()), (Shape::Setting, vec!["window"]));
        assert_eq!((width.read("window"), width.read("72")), (Ok(Arg::Named("window")), Ok(Arg::Number(72))));
        assert_eq!(width.read("300"), Err("expected window or 20–200, got 300".into()));
        assert!(width.read("auto").is_err(), "no `auto` for a setting that doesn't go by the background");
        // What each has when it is not given.
        let defaults: Vec<Option<String>> =
            ["--theme", "--width", "--pages", "--pager", "--password"].iter().map(|name| catalog.entry(name).unwrap().default()).collect();
        assert_eq!(defaults, [Some("auto".into()), Some("light".into()), None, None, None]);
        // Whether a config may set it is the option's to say.
        let may: Vec<Settable> = ["--pages", "--password", "--theme"].iter().map(|name| catalog.entry(name).unwrap().spec.config).collect();
        assert_eq!(may, [Settable::Yes, Settable::No("a secret"), Settable::Yes]);
        assert!(catalog.entry("--zoom").is_none() && Catalog::default().entries().is_empty());
    }

    #[test]
    fn unknown_options_are_refused_and_values_taken_whatever_they_are() {
        let specs = [
            OptionSpec {
                name: "--password",
                short: None,
                value: Some("PW"),
                help: "",
                alias: None,
                shape: Shape::Text,
                config: Settable::No("a password does not belong in a plain file"),
            },
            OptionSpec { name: "--pager", short: None, value: None, help: "", alias: None, shape: Shape::Flag, config: Settable::Yes },
            OptionSpec { name: "--recurse", short: Some("-r"), value: None, help: "", alias: None, shape: Shape::Flag, config: Settable::Yes },
        ];
        let catalog = Catalog::of(&specs);
        let args = |a: &[&str]| a.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let given = args(&["a.pdf", "--pager", "--password", "--anything", "b.pdf"]);
        let parsed = Parsed::parse(&given, &catalog).unwrap();
        assert_eq!(parsed.value("--password"), Some("--anything"), "a password can look like an option");
        assert_eq!(parsed.operands, ["a.pdf", "b.pdf"]);
        assert!(parsed.has("--pager") && !parsed.has("--zoom"));
        assert_eq!(parsed.value("--pager"), None);
        assert_eq!(parsed.names().collect::<Vec<_>>(), ["--pager", "--password"]);
        let given = args(&["--password", "first", "--password", "second"]);
        assert_eq!(Parsed::parse(&given, &catalog).unwrap().value("--password"), Some("first"));
        assert_eq!(Parsed::parse(&args(&["a.pdf", "--zoom", "2"]), &catalog).unwrap_err(), "unknown option --zoom");
        assert_eq!(Parsed::parse(&args(&["a.pdf", "--password"]), &catalog).unwrap_err(), "--password needs a value");
        // A short name is the option's, as the option's name.
        let given = args(&["-r", "dir", "-"]);
        let parsed = Parsed::parse(&given, &catalog).unwrap();
        assert!(parsed.has("--recurse") && parsed.names().eq(["--recurse"]));
        assert_eq!(parsed.operands, ["dir", "-"], "`-` alone is no option");
        assert_eq!(Parsed::parse(&args(&["-x", "a.pdf"]), &catalog).unwrap_err(), "unknown option -x");
        assert_eq!(Parsed::parse(&args(&["--password", "-r"]), &catalog).unwrap().value("--password"), Some("-r"));
    }

    /// A config's options come after the command line's: the command line wins, and what goes by
    /// the config is known, and can be left out.
    #[test]
    fn an_option_on_the_command_line_wins_over_the_same_option_in_a_config() {
        let specs = [option("--width", Some("N"), Shape::Text), option("--per-row", Some("N"), Shape::Text), option("--recurse", None, Shape::Flag)];
        let catalog = Catalog::of(&specs);
        let given: Vec<String> = ["a.md", "--per-row", "2"].iter().map(|s| (*s).to_string()).collect();
        let mut parsed = Parsed::parse(&given, &catalog).unwrap();
        assert!(!parsed.by_config("--per-row") && !parsed.by_config("--width"), "not given is not from a config");
        parsed.configure("--per-row", Some("3"));
        parsed.configure("--width", Some("72"));
        parsed.configure("--recurse", None);
        assert_eq!((parsed.value("--per-row"), parsed.value("--width"), parsed.has("--recurse")), (Some("2"), Some("72"), true));
        assert_eq!(["--per-row", "--width", "--recurse"].map(|name| parsed.by_config(name)), [false, true, true]);
        assert_eq!(parsed.names().collect::<Vec<_>>(), ["--per-row", "--per-row", "--width", "--recurse"]);
        // Without the config's width, and the command line's count kept whatever is asked.
        let without = parsed.without_config("--width").without_config("--per-row");
        assert_eq!((without.value("--width"), without.value("--per-row"), without.has("--recurse")), (None, Some("2"), true));
        assert_eq!(without.operands, ["a.md"]);
        // On a thread that opens files, they are all options alike.
        let owned = parsed.owned();
        assert_eq!((owned.parsed().value("--per-row"), owned.parsed().value("--width")), (Some("2"), Some("72")));
        assert_eq!(specs.map(|spec| spec.key()), ["width", "per-row", "recurse"]);
        assert_eq!(Catalog::of(&[specs[0], specs[1], specs[0]]).options().map(|entry| entry.spec.name).collect::<Vec<_>>(), ["--width", "--per-row"]);
    }

    /// An option is taken by its alias too, as itself, the alias remembered to warn of; given by both,
    /// it is refused. `--help` never shows the alias.
    #[test]
    fn an_option_is_taken_by_its_alias_and_refused_by_both() {
        let specs = [
            OptionSpec {
                name: "--per-row",
                short: None,
                value: Some("N"),
                help: "",
                alias: Some("--zoom"),
                shape: Shape::Text,
                config: Settable::Yes,
            },
            OptionSpec { name: "--pager", short: None, value: None, help: "", alias: None, shape: Shape::Flag, config: Settable::Yes },
        ];
        let catalog = Catalog::of(&specs);
        let args = |a: &[&str]| a.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let given = args(&["a.pdf", "--zoom", "3", "--pager"]);
        let parsed = Parsed::parse(&given, &catalog).unwrap();
        assert_eq!((parsed.value("--per-row"), parsed.value("--zoom")), (Some("3"), None), "the alias is the option");
        assert_eq!((parsed.aliased(), parsed.typed("--per-row", &catalog)), (&["--per-row"][..], "--zoom"));
        let given = args(&["--per-row", "3"]);
        let parsed = Parsed::parse(&given, &catalog).unwrap();
        assert_eq!((parsed.aliased().len(), parsed.typed("--per-row", &catalog), parsed.typed("--pager", &catalog)), (0, "--per-row", "--pager"));
        for both in [["--zoom", "2", "--per-row", "3"], ["--per-row", "2", "--zoom", "3"]] {
            assert_eq!(Parsed::parse(&args(&both), &catalog).unwrap_err(), "--per-row and --zoom are one option: give one of them", "{both:?}");
        }
        // Twice by its alias is as twice by its name: the first holds.
        let given = args(&["--zoom", "2", "--zoom", "3"]);
        let parsed = Parsed::parse(&given, &catalog).unwrap();
        assert_eq!((parsed.value("--per-row"), parsed.aliased()), (Some("2"), &["--per-row"][..]));
        assert_eq!(specs[0].shown(), "--per-row N");
    }
}
