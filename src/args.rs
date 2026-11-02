//! Command-line options and page lists.

/// An option: its `name` (`--zoom`), the `short` one it has too (`-r`), what its value looks like
/// (`None` for one that takes none) and what it does, as `--help` shows them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OptionSpec {
    pub(crate) name: &'static str,
    pub(crate) short: Option<&'static str>,
    pub(crate) value: Option<&'static str>,
    pub(crate) help: &'static str,
}

impl OptionSpec {
    /// The option as `--help` lists it: `--zoom auto|width|N`, `-r, --recurse`.
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
}

impl<'a> Parsed<'a> {
    /// Refuses an option not in `specs`, and one left without its value. An option that takes a
    /// value takes the next argument, whatever it is: a password can be anything. An argument
    /// that starts with `-` is an option, but for `-` alone; `./-file` names a file that does.
    pub(crate) fn parse(args: &'a [String], specs: &[OptionSpec]) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut rest = args.iter();
        while let Some(arg) = rest.next() {
            if !arg.starts_with('-') || arg == "-" {
                parsed.operands.push(arg);
                continue;
            }
            let spec = specs.iter().find(|spec| spec.name == arg || spec.short == Some(arg)).ok_or_else(|| format!("unknown option {arg}"))?;
            let value = match spec.value {
                Some(_) => Some(rest.next().ok_or_else(|| format!("{arg} needs a value"))?.as_str()),
                None => None,
            };
            parsed.given.push((spec.name, value));
        }
        Ok(parsed)
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
            OptionSpec { name: "--password", short: None, value: Some("PW"), help: "" },
            OptionSpec { name: "--pager", short: None, value: None, help: "" },
            OptionSpec { name: "--recurse", short: Some("-r"), value: None, help: "" },
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
}
