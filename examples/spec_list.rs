//! The change specs in `docs/specs/`: which are in flight, ready, drafted, set aside or done, and
//! the Index in its README, made from the specs' own headers. The `spec-list` skill runs it; see
//! `docs/specs/README.md`. From the repository's root:
//!
//! ```sh
//! cargo run -q --no-default-features --example spec_list                # every bucket
//! cargo run -q --no-default-features --example spec_list -- ready       # one: implementing, ready, draft, shelved, done
//! cargo run -q --no-default-features --example spec_list -- --index     # rewrite the README's Index
//! cargo run -q --no-default-features --example spec_list -- --check     # what tests/docs.rs checks
//! ```
//!
//! An example rather than a binary, so `cargo install` leaves it out; `tests/docs.rs` builds this
//! file as a module of its own and runs the check, so a spec whose Index row disagrees with it fails
//! the tests. The standard library only.

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

/// The README whose `## Index` this keeps, in the specs' directory.
const README: &str = "README.md";
/// How many done specs the list shows, the most recently changed first.
const RECENT: usize = 10;
/// How long a next item may be, in chars, before it is cut.
const NEXT: usize = 100;

/// Where a spec stands, from its `**Status:**` line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bucket {
    Implementing,
    Ready,
    Draft,
    /// Shelved or superseded: out of the queue, but still listed, since a spec nobody works on is
    /// the one that goes missing.
    Shelved,
    Done,
}

impl Bucket {
    const ALL: [Bucket; 5] = [Bucket::Implementing, Bucket::Ready, Bucket::Draft, Bucket::Shelved, Bucket::Done];

    fn name(self) -> &'static str {
        match self {
            Bucket::Implementing => "implementing",
            Bucket::Ready => "ready",
            Bucket::Draft => "draft",
            Bucket::Shelved => "shelved",
            Bucket::Done => "done",
        }
    }

    /// The bucket a status names, if it is one of the lifecycle's.
    pub fn of(status: &str) -> Option<Bucket> {
        let status = status.trim();
        match status {
            "Implementing" => Some(Bucket::Implementing),
            "Ready" => Some(Bucket::Ready),
            "Draft" => Some(Bucket::Draft),
            "Done" => Some(Bucket::Done),
            "Shelved" => Some(Bucket::Shelved),
            _ => status.strip_prefix("Superseded by Spec-").filter(|n| is_number(n)).map(|_| Bucket::Shelved),
        }
    }
}

/// What a spec's file says of it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Spec {
    /// The four digits its file name starts with.
    pub number: String,
    /// Its file name, without `.md`.
    pub file: String,
    /// What follows `# Spec-NNNN: `; empty if the heading is missing.
    pub title: String,
    /// The number its heading gives, which should be its file's.
    pub heading_number: String,
    pub status: String,
    pub date: String,
    /// Ticked and all, under `## Tasks`.
    pub tasks: (usize, usize),
    /// Ticked and all, under `## Done`.
    pub done: (usize, usize),
    /// The first unticked item, tasks before done.
    pub next: Option<String>,
    /// 🔴 marks under `## Open questions`.
    pub blockers: usize,
    /// The latest date its changelog names, else its date.
    pub changed: String,
    /// When its latest implementation notes were opened, `YYYY-MM-DD HH:MM`.
    pub notes: Option<String>,
}

fn is_number(s: &str) -> bool {
    s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit())
}

/// The number of a spec's file: `NNNN-<slug>.md`, but not its implementation notes, nor the
/// README, the template or `project.md`.
pub fn spec_number(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".md")?;
    let (number, slug) = stem.split_once('-')?;
    (is_number(number) && !slug.is_empty() && !stem.contains("-impl-")).then_some(number)
}

/// The `YYYY-MM-DD`s in `line`.
fn dates(line: &str) -> Vec<&str> {
    let bytes = line.as_bytes();
    let shape = |at: usize| {
        let digit = |i: usize| bytes.get(at + i).is_some_and(u8::is_ascii_digit);
        let boundary = |i: Option<usize>| i.and_then(|i| bytes.get(i)).is_none_or(|b| !b.is_ascii_alphanumeric());
        (0..10).all(|i| if i == 4 || i == 7 { bytes.get(at + i) == Some(&b'-') } else { digit(i) })
            && boundary(at.checked_sub(1))
            && boundary(Some(at + 10))
    };
    (0..bytes.len()).filter(|&at| shape(at)).filter_map(|at| line.get(at..at + 10)).collect()
}

/// A checklist item's text, if `line` is one, and whether it is ticked.
fn checkbox(line: &str) -> Option<(bool, &str)> {
    let rest = line.trim_start().strip_prefix("- [")?;
    let ticked = match rest.get(..1)? {
        " " => false,
        "x" | "X" => true,
        _ => return None,
    };
    Some((ticked, rest.get(1..)?.strip_prefix(']')?.trim()))
}

/// A task's name for the list: its bold lead without the marks, up to its dash or its fields,
/// cut to `NEXT` chars.
fn summary(item: &str) -> String {
    let item = item.trim();
    let item = match item.strip_prefix("**").and_then(|rest| rest.split_once("**")) {
        Some((bold, rest)) => format!("{bold}{rest}"),
        None => item.to_owned(),
    };
    let end = [" — ", " -- ", " *Files:*", " *Verify:*"].iter().filter_map(|sep| item.find(sep)).filter(|&at| at > 0).min();
    let item = end.and_then(|at| item.get(..at)).unwrap_or(&item).trim();
    if item.chars().count() > NEXT { item.chars().take(NEXT - 1).chain(['…']).collect() } else { item.to_owned() }
}

/// What the spec in the file `file` (its name without `.md`) says, from its text.
pub fn parse(number: &str, file: &str, text: &str) -> Spec {
    let mut spec = Spec { number: number.to_owned(), file: file.to_owned(), ..Spec::default() };
    let mut section = "header";
    let mut changes = Vec::new();
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            let heading = heading.trim().to_lowercase();
            section = ["tasks", "done", "open questions", "changelog"].into_iter().find(|s| heading.starts_with(s)).unwrap_or("other");
            continue;
        }
        if section == "header" {
            if let Some((n, title)) = line.strip_prefix("# Spec-").and_then(|rest| rest.split_once(':')) {
                spec.heading_number = n.trim().to_owned();
                spec.title = title.trim().to_owned();
            } else if let Some(status) = line.trim().strip_prefix("- **Status:**") {
                spec.status = status.trim().to_owned();
            } else if let Some(date) = line.trim().strip_prefix("- **Date:**") {
                spec.date = date.trim().to_owned();
            }
            continue;
        }
        let tally = match section {
            "tasks" => &mut spec.tasks,
            "done" => &mut spec.done,
            "open questions" => {
                // A question is a bullet: the legend above them isn't one.
                spec.blockers += usize::from(line.trim_start().starts_with("- ") && line.contains('🔴'));
                continue;
            }
            "changelog" => {
                changes.extend(dates(line).into_iter().map(str::to_owned));
                continue;
            }
            _ => continue,
        };
        if let Some((ticked, item)) = checkbox(line) {
            tally.1 += 1;
            if ticked {
                tally.0 += 1;
            } else if spec.next.is_none() {
                spec.next = Some(summary(item));
            }
        }
    }
    spec.changed = changes.into_iter().max().unwrap_or_else(|| spec.date.clone());
    spec
}

/// When the latest of `file`'s implementation notes among `names` was opened.
fn latest_notes<'a>(file: &str, names: impl Iterator<Item = &'a str>) -> Option<String> {
    let prefix = format!("{file}-impl-");
    let stamp = names
        .filter_map(|name| name.strip_prefix(&prefix)?.strip_suffix(".md"))
        .filter(|s| s.len() == 13 && s.bytes().enumerate().all(|(i, b)| if i == 8 { b == b'_' } else { b.is_ascii_digit() }))
        .max()?;
    // All ASCII, checked above, so every position is a char boundary.
    let at = |from: usize, to: usize| stamp.get(from..to).unwrap_or_default();
    Some(format!("{}-{}-{} {}:{}", at(0, 4), at(4, 6), at(6, 8), at(9, 11), at(11, 13)))
}

/// The specs in `dir`, by number.
pub fn specs(dir: &Path) -> Result<Vec<Spec>, String> {
    let read = std::fs::read_dir(dir).map_err(|e| format!("can't read {}: {e}", dir.display()))?;
    let mut names = Vec::new();
    for entry in read {
        let entry = entry.map_err(|e| format!("can't read {}: {e}", dir.display()))?;
        if entry.path().is_file() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    let mut found = Vec::new();
    for name in &names {
        let Some(number) = spec_number(name) else { continue };
        let path = dir.join(name);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
        let file = name.strip_suffix(".md").unwrap_or(name);
        let mut spec = parse(number, file, &text);
        spec.notes = latest_notes(file, names.iter().map(String::as_str));
        found.push(spec);
    }
    Ok(found)
}

/// The Index's rows for `specs`: number, title linked to its file, status.
pub fn index_rows(specs: &[Spec]) -> Vec<String> {
    specs.iter().map(|s| format!("| {} | [{}]({}.md) | {} |", s.number, s.title, s.file, s.status)).collect()
}

/// Where the Index's rows are among `lines`: from the line after the separator under
/// `## Index`, to the first line that isn't a table's. Found by its separator, so an Index with
/// no rows yet is found too.
fn index_span(lines: &[&str]) -> Option<(usize, usize)> {
    let heading = lines.iter().position(|l| l.trim() == "## Index")?;
    let separator = |l: &str| l.starts_with('|') && l.contains('-') && l.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '));
    let start = heading + 1 + lines.get(heading + 1..)?.iter().take_while(|l| !l.starts_with("## ")).position(|l| separator(l.trim()))? + 1;
    let end = start + lines.get(start..)?.iter().take_while(|l| l.trim_start().starts_with('|')).count();
    Some((start, end))
}

/// `readme` with its Index's rows replaced by `rows`.
pub fn with_index(readme: &str, rows: &[String]) -> Result<String, String> {
    let lines: Vec<&str> = readme.lines().collect();
    let (start, end) = index_span(&lines).ok_or("no table under `## Index`")?;
    let mut out: Vec<&str> = lines.get(..start).unwrap_or_default().to_vec();
    out.extend(rows.iter().map(String::as_str));
    out.extend(lines.get(end..).unwrap_or_default().iter().copied());
    let mut text = out.join("\n");
    if readme.ends_with('\n') {
        text.push('\n');
    }
    Ok(text)
}

/// What is wrong with the specs in `dir` and their Index: a status the lifecycle doesn't have, a
/// heading missing or naming another number, a number used twice, a title that would break its
/// row, and an Index that isn't the one `--index` would write.
pub fn problems(dir: &Path) -> Vec<String> {
    let specs = match specs(dir) {
        Ok(specs) => specs,
        Err(e) => return vec![e],
    };
    let mut found = Vec::new();
    for (i, s) in specs.iter().enumerate() {
        let name = format!("{}.md", s.file);
        if s.title.is_empty() {
            found.push(format!("{name}: no `# Spec-{}: <title>` heading", s.number));
        } else if s.heading_number != s.number {
            found.push(format!("{name}: its heading says Spec-{}", s.heading_number));
        }
        if s.title.contains(['|', '[', ']']) {
            found.push(format!("{name}: a title with `|`, `[` or `]` breaks its Index row"));
        }
        if Bucket::of(&s.status).is_none() {
            found.push(format!("{name}: status `{}` is none of Draft, Ready, Implementing, Done, Shelved, Superseded by Spec-NNNN", s.status));
        }
        if specs.get(..i).is_some_and(|before| before.iter().any(|b| b.number == s.number)) {
            found.push(format!("{name}: Spec-{} is taken", s.number));
        }
    }
    let path = dir.join(README);
    match std::fs::read_to_string(&path) {
        Err(e) => found.push(format!("can't read {}: {e}", path.display())),
        Ok(readme) => match with_index(&readme, &index_rows(&specs)) {
            Err(e) => found.push(format!("{README}: {e}")),
            Ok(synced) if synced != readme => found.push(format!("{README}: its Index isn't the specs' own: run this with --index")),
            Ok(_) => {}
        },
    }
    found
}

/// The list, of every bucket or of `only`.
pub fn list(specs: &[Spec], only: Option<Bucket>) -> String {
    let mut out = String::new();
    for bucket in Bucket::ALL.into_iter().filter(|b| only.is_none_or(|o| o == *b)) {
        let mut these: Vec<&Spec> = specs.iter().filter(|s| Bucket::of(&s.status) == Some(bucket)).collect();
        if bucket == Bucket::Done {
            these.sort_by(|a, b| b.changed.cmp(&a.changed).then(b.number.cmp(&a.number)));
        }
        if !out.is_empty() {
            out.push('\n');
        }
        let count = these.len();
        let _ = match bucket {
            Bucket::Implementing => writeln!(out, "## Implementing ({count})\n"),
            Bucket::Ready => writeln!(out, "## Ready ({count})\n"),
            Bucket::Draft => writeln!(out, "## Draft, to lift to Ready ({count})\n"),
            Bucket::Shelved => writeln!(out, "## Shelved or superseded, out of the queue ({count})\n"),
            Bucket::Done if count > RECENT => writeln!(out, "## Recently done (the last {RECENT} of {count})\n"),
            Bucket::Done => writeln!(out, "## Recently done ({count})\n"),
        };
        if these.is_empty() {
            out.push_str("- (none)\n");
        }
        for s in these.into_iter().take(if bucket == Bucket::Done { RECENT } else { usize::MAX }) {
            let _ = write!(out, "- **{}** {}", s.number, s.title);
            let _ = match bucket {
                Bucket::Implementing => write!(out, ": tasks {}/{}, done {}/{}", s.tasks.0, s.tasks.1, s.done.0, s.done.1),
                Bucket::Ready => write!(out, ": {} tasks", s.tasks.1),
                Bucket::Draft => write!(out, ": {}", s.date),
                Bucket::Shelved => write!(out, ": {}, {}", s.status, s.date),
                Bucket::Done => write!(out, ": {}", s.changed),
            };
            if let (Bucket::Implementing, Some(notes)) = (bucket, &s.notes) {
                let _ = write!(out, ", notes from {notes}");
            }
            if s.blockers > 0 && bucket != Bucket::Done {
                let _ = write!(out, ", {} 🔴", s.blockers);
            }
            out.push('\n');
            if let (Bucket::Implementing, Some(next)) = (bucket, &s.next) {
                let _ = writeln!(out, "  Next: {next}");
            }
        }
    }
    out
}

fn main() -> ExitCode {
    let dir = Path::new("docs/specs");
    let arg = std::env::args().nth(1).unwrap_or_default();
    match arg.as_str() {
        "--check" => {
            let found = problems(dir);
            if found.is_empty() {
                println!("spec_list --check: the specs and their Index agree.");
                return ExitCode::SUCCESS;
            }
            eprintln!("spec_list --check:\n  {}", found.join("\n  "));
            ExitCode::FAILURE
        }
        "--index" => {
            let path = dir.join(README);
            let synced = specs(dir).and_then(|specs| {
                let readme = std::fs::read_to_string(&path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
                Ok((with_index(&readme, &index_rows(&specs))?, readme, specs.len()))
            });
            match synced {
                Err(e) => {
                    eprintln!("spec_list --index: {e}");
                    ExitCode::FAILURE
                }
                Ok((synced, readme, _)) if synced == readme => {
                    println!("spec_list --index: the Index already agrees.");
                    ExitCode::SUCCESS
                }
                Ok((synced, _, count)) => match std::fs::write(&path, synced) {
                    Ok(()) => {
                        println!("spec_list --index: wrote {}'s Index, {count} specs.", path.display());
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("spec_list --index: can't write {}: {e}", path.display());
                        ExitCode::FAILURE
                    }
                },
            }
        }
        _ => {
            let only = Bucket::ALL.into_iter().find(|b| b.name() == arg.to_lowercase());
            if !arg.is_empty() && only.is_none() {
                eprintln!("spec_list: no bucket `{arg}`: implementing, ready, draft, shelved or done");
                return ExitCode::from(2);
            }
            match specs(dir) {
                Ok(specs) => {
                    print!("{}", list(&specs, only));
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("spec_list: {e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = "# Spec-0007: Show AVIF pictures\n\n- **Status:** Implementing\n- **Date:** 2026-11-01\n\n\
        ## Open questions\n\n🔴 blocks it · 🟡 to keep in mind.\n\n- 🔴 **Which decoder** — …\n- 🟡 **Its speed**\n\n## Tasks\n\n- [x] **Decode** — it. *Files:* `a`.\n\
        - [ ] **Show its colours right** — then. *Verify:* `cargo test`.\n\n## Done\n\n- [ ] `cargo test`\n\n\
        ## Changelog\n\n- 2026-11-01 — Ready.\n- 2026-11-03 — Implementing started ([notes](x)).\n";

    #[test]
    fn a_spec_is_read_from_its_header_and_checklists() {
        let spec = parse("0007", "0007-show-avif-pictures", SPEC);
        assert_eq!((spec.title.as_str(), spec.heading_number.as_str(), spec.status.as_str()), ("Show AVIF pictures", "0007", "Implementing"));
        assert_eq!((spec.tasks, spec.done, spec.blockers), ((1, 2), (0, 1), 1));
        assert_eq!(spec.next.as_deref(), Some("Show its colours right"));
        assert_eq!(spec.changed, "2026-11-03");
    }

    #[test]
    fn only_numbered_specs_are_specs() {
        assert_eq!(spec_number("0007-show-avif.md"), Some("0007"));
        for not in ["README.md", "template.md", "project.md", "0007-show-avif-impl-20261103_0915.md", "007-x.md", "0007-.md", "0007-x.txt"] {
            assert_eq!(spec_number(not), None, "{not}");
        }
        let names = ["0007-a-impl-20261103_0915.md", "0007-a-impl-20261104_1200.md", "0007-ab-impl-20261231_2359.md", "0007-a-impl-junk.md"];
        assert_eq!(latest_notes("0007-a", names.into_iter()).as_deref(), Some("2026-11-04 12:00"));
    }

    #[test]
    fn the_lifecycle_has_these_statuses() {
        assert_eq!(Bucket::of("Superseded by Spec-0012"), Some(Bucket::Shelved));
        assert_eq!(Bucket::of(" Ready "), Some(Bucket::Ready));
        for not in ["Accepted", "ready", "Superseded", "Superseded by Spec-12", ""] {
            assert_eq!(Bucket::of(not), None, "{not}");
        }
    }

    #[test]
    fn the_index_is_found_by_its_separator_even_with_no_rows() {
        let rows = vec!["| 0001 | [A](0001-a.md) | Ready |".to_owned()];
        let empty = "# Specs\n\n## Index\n\nGenerated.\n\n| # | Title | Status |\n|---|---|---|\n\n## Lifecycle\n";
        let one = with_index(empty, &rows).unwrap();
        assert_eq!(
            one,
            "# Specs\n\n## Index\n\nGenerated.\n\n| # | Title | Status |\n|---|---|---|\n| 0001 | [A](0001-a.md) | Ready |\n\n## Lifecycle\n"
        );
        assert_eq!(with_index(&one, &[]).unwrap(), empty);
        assert!(with_index("## Index\n\nno table\n\n## Next\n\n|---|\n", &rows).is_err());
    }

    #[test]
    fn dates_are_found_whole() {
        assert_eq!(dates("- 2026-11-03 — Done, after 2026-11-01; not 12026-11-03 nor 2026-11-0x."), ["2026-11-03", "2026-11-01"]);
    }

    #[test]
    fn a_long_task_is_cut_at_a_char() {
        let long = "é".repeat(NEXT + 5);
        let cut = summary(&format!("**{long}** — rest"));
        assert_eq!((cut.chars().count(), cut.ends_with('…')), (NEXT, true));
    }
}
