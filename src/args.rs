//! Command-line options and page lists.

/// An option: its `name` (`--per-row`), the `short` one it has too (`-r`), what its value looks like
/// (`None` for one that takes none) and what it does, as `--help` shows them; and the `alias` it is
/// still taken by, an old name `--help` doesn't show (`--zoom`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct OptionSpec {
    pub(crate) name: &'static str,
    pub(crate) short: Option<&'static str>,
    pub(crate) value: Option<&'static str>,
    pub(crate) help: &'static str,
    pub(crate) alias: Option<&'static str>,
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
}

/// A command line read against the options it may hold, before any file is read.
#[derive(Debug, Default)]
pub(crate) struct Parsed<'a> {
    /// The arguments that are neither an option nor an option's value, in order.
    pub(crate) operands: Vec<&'a str>,
    /// Each option given, in order, with its value if it takes one.
    given: Vec<(&'static str, Option<&'a str>)>,
    /// The options given by their alias, by their name: `--zoom` for `--per-row`.
    aliased: Vec<&'static str>,
}

impl<'a> Parsed<'a> {
    /// Refuses an option not in `specs`, and one left without its value. An option that takes a
    /// value takes the next argument, whatever it is: a password can be anything. An argument
    /// that starts with `-` is an option, but for `-` alone; `./-file` names a file that does. An
    /// option given by its alias is that option, and given by both its name and its alias, refused.
    pub(crate) fn parse(args: &'a [String], specs: &[OptionSpec]) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut rest = args.iter();
        let mut by_name = Vec::new();
        while let Some(arg) = rest.next() {
            if !arg.starts_with('-') || arg == "-" {
                parsed.operands.push(arg);
                continue;
            }
            let spec = specs
                .iter()
                .find(|spec| spec.name == arg || spec.short == Some(arg) || spec.alias == Some(arg))
                .ok_or_else(|| format!("unknown option {arg}"))?;
            let value = match spec.value {
                Some(_) => Some(rest.next().ok_or_else(|| format!("{arg} needs a value"))?.as_str()),
                None => None,
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
        Ok(parsed)
    }

    /// How option `name` was given, the first time: by its name, or by its alias (`--zoom`).
    pub(crate) fn typed(&self, name: &'static str, specs: &[OptionSpec]) -> &'static str {
        let alias = specs.iter().find(|spec| spec.name == name).and_then(|spec| spec.alias);
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
        Parsed { operands: Vec::new(), given: self.0.iter().map(|(name, value)| (*name, value.as_deref())).collect(), aliased: Vec::new() }
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

    #[test]
    fn unknown_options_are_refused_and_values_taken_whatever_they_are() {
        let specs = [
            OptionSpec { name: "--password", short: None, value: Some("PW"), help: "", alias: None },
            OptionSpec { name: "--pager", short: None, value: None, help: "", alias: None },
            OptionSpec { name: "--recurse", short: Some("-r"), value: None, help: "", alias: None },
        ];
        let args = |a: &[&str]| a.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let given = args(&["a.pdf", "--pager", "--password", "--anything", "b.pdf"]);
        let parsed = Parsed::parse(&given, &specs).unwrap();
        assert_eq!(parsed.value("--password"), Some("--anything"), "a password can look like an option");
        assert_eq!(parsed.operands, ["a.pdf", "b.pdf"]);
        assert!(parsed.has("--pager") && !parsed.has("--zoom"));
        assert_eq!(parsed.value("--pager"), None);
        assert_eq!(parsed.names().collect::<Vec<_>>(), ["--pager", "--password"]);
        let given = args(&["--password", "first", "--password", "second"]);
        assert_eq!(Parsed::parse(&given, &specs).unwrap().value("--password"), Some("first"));
        assert_eq!(Parsed::parse(&args(&["a.pdf", "--zoom", "2"]), &specs).unwrap_err(), "unknown option --zoom");
        assert_eq!(Parsed::parse(&args(&["a.pdf", "--password"]), &specs).unwrap_err(), "--password needs a value");
        // A short name is the option's, as the option's name.
        let given = args(&["-r", "dir", "-"]);
        let parsed = Parsed::parse(&given, &specs).unwrap();
        assert!(parsed.has("--recurse") && parsed.names().eq(["--recurse"]));
        assert_eq!(parsed.operands, ["dir", "-"], "`-` alone is no option");
        assert_eq!(Parsed::parse(&args(&["-x", "a.pdf"]), &specs).unwrap_err(), "unknown option -x");
        assert_eq!(Parsed::parse(&args(&["--password", "-r"]), &specs).unwrap().value("--password"), Some("-r"));
    }

    /// An option is taken by its alias too, as itself, the alias remembered to warn of; given by both,
    /// it is refused. `--help` never shows the alias.
    #[test]
    fn an_option_is_taken_by_its_alias_and_refused_by_both() {
        let specs = [
            OptionSpec { name: "--per-row", short: None, value: Some("N"), help: "", alias: Some("--zoom") },
            OptionSpec { name: "--pager", short: None, value: None, help: "", alias: None },
        ];
        let args = |a: &[&str]| a.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let given = args(&["a.pdf", "--zoom", "3", "--pager"]);
        let parsed = Parsed::parse(&given, &specs).unwrap();
        assert_eq!((parsed.value("--per-row"), parsed.value("--zoom")), (Some("3"), None), "the alias is the option");
        assert_eq!((parsed.aliased(), parsed.typed("--per-row", &specs)), (&["--per-row"][..], "--zoom"));
        let given = args(&["--per-row", "3"]);
        let parsed = Parsed::parse(&given, &specs).unwrap();
        assert_eq!((parsed.aliased().len(), parsed.typed("--per-row", &specs), parsed.typed("--pager", &specs)), (0, "--per-row", "--pager"));
        for both in [["--zoom", "2", "--per-row", "3"], ["--per-row", "2", "--zoom", "3"]] {
            assert_eq!(Parsed::parse(&args(&both), &specs).unwrap_err(), "--per-row and --zoom are one option: give one of them", "{both:?}");
        }
        // Twice by its alias is as twice by its name: the first holds.
        let given = args(&["--zoom", "2", "--zoom", "3"]);
        let parsed = Parsed::parse(&given, &specs).unwrap();
        assert_eq!((parsed.value("--per-row"), parsed.aliased()), (Some("2"), &["--per-row"][..]));
        assert_eq!(specs[0].shown(), "--per-row N");
    }
}
