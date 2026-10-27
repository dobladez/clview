//! Command-line options and page lists.

use crate::CliError;

pub(crate) fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

#[derive(Clone, Copy)]
pub(crate) struct OptionSpec {
    pub(crate) name: &'static str,
    pub(crate) takes_value: bool,
}

/// Refuse unknown options before any file is read. The value after an option that takes one is
/// skipped, whatever it is: a password can be anything.
pub(crate) fn validate_options(args: &[String], specs: &[OptionSpec]) -> Result<(), CliError> {
    let mut i = 0usize;
    while let Some(arg) = args.get(i) {
        if let Some(name) = arg.strip_prefix("--") {
            let Some(spec) = specs.iter().find(|spec| spec.name == arg) else {
                return Err(format!("unknown option --{name}").into());
            };
            i = i.saturating_add(if spec.takes_value { 2 } else { 1 });
        } else {
            i = i.saturating_add(1);
        }
    }
    Ok(())
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
    fn unknown_options_are_refused_and_values_skipped() {
        let specs = [OptionSpec { name: "--password", takes_value: true }, OptionSpec { name: "--pager", takes_value: false }];
        let args = |a: &[&str]| a.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        assert!(validate_options(&args(&["a.pdf", "--pager", "--password", "--anything"]), &specs).is_ok(), "a password can look like an option");
        assert!(validate_options(&args(&["a.pdf", "--zoom", "2"]), &specs).is_err_and(|e| e.to_string() == "unknown option --zoom"));
        assert!(validate_options(&args(&["--password"]), &specs).is_ok(), "a missing value is the command's to report");
    }
}
