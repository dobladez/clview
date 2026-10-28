//! A directory's files, in the order a gallery shows them: by name, the way people number files
//! (`img2` before `img10`), hidden ones left out. Its subdirectories are left out too, unless
//! asked to recurse: then each one's files come at its place among the names, depth first. A
//! link to a file is followed; a link to a directory is not, so no loop of links goes on forever.

use std::cmp::Ordering;
use std::path::Path;

/// The most files listed from a directory: each becomes a source, and a source holds its header
/// (a PDF, its whole file).
const MAX_FILES: usize = 4096;

/// The most directories down a recursion goes.
const MAX_DEPTH: usize = 32;

/// The most entries read from a directory, files and directories, shown or not: a recursion into
/// a home directory stops in time.
const MAX_ENTRIES: usize = 100_000;

/// What a directory holds to show.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Listing {
    /// Each file: its path, and its name from the directory listed (`2024/beach.jpg`).
    pub(crate) files: Vec<(String, String)>,
    /// What couldn't be read, by its name from the directory listed, and why: a subdirectory,
    /// a name that isn't UTF-8.
    pub(crate) unread: Vec<(String, String)>,
    /// Whether subdirectories were left out, not recursed into.
    pub(crate) subdirectories: bool,
    /// Whether the listing stopped at one of the limits before the end.
    pub(crate) cut: bool,
}

/// [`Listing`] of the directory at `dir`, its subdirectories' too if `recurse`.
pub(crate) fn list(dir: &str, recurse: bool) -> Result<Listing, String> {
    list_within(dir, recurse, MAX_FILES, MAX_DEPTH)
}

/// [`list`], within `max_files` files and `max_depth` directories down.
fn list_within(dir: &str, recurse: bool, max_files: usize, max_depth: usize) -> Result<Listing, String> {
    let mut listing = Listing::default();
    let top = entries(Path::new(dir)).map_err(|e| format!("{dir}: {e}"))?;
    // Each directory being listed, from `dir` down: the prefix of its names, and the entries left.
    let mut open = vec![(String::new(), top.into_iter())];
    let mut read = 0usize;
    while let Some((prefix, mut left)) = open.pop() {
        let Some(entry) = left.next() else { continue };
        let depth = open.len();
        open.push((prefix.clone(), left));
        read = read.saturating_add(1);
        if listing.files.len() >= max_files || read > MAX_ENTRIES {
            listing.cut = true;
            break;
        }
        let name = format!("{prefix}{}", entry.name);
        let path = Path::new(dir).join(&name);
        let (Some(path), true) = (path.to_str().map(String::from), entry.utf8) else {
            listing.unread.push((name, format!("{}: the name isn't UTF-8", path.display())));
            continue;
        };
        match entry.kind {
            Kind::File => listing.files.push((path, name)),
            Kind::Directory if !recurse => listing.subdirectories = true,
            Kind::Directory if depth >= max_depth => listing.unread.push((name, format!("{path}: more than {max_depth} directories down"))),
            Kind::Directory => match entries(Path::new(&path)) {
                Ok(inside) => open.push((format!("{name}/"), inside.into_iter())),
                Err(e) => listing.unread.push((name, format!("{path}: {e}"))),
            },
        }
    }
    Ok(listing)
}

/// An entry of a directory worth listing.
struct Entry {
    /// As it reads: with U+FFFD in place of what isn't UTF-8, when `utf8` is false.
    name: String,
    utf8: bool,
    kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    File,
    Directory,
}

/// The files and directories in `dir`, by name: no hidden ones, nothing that is neither (a pipe
/// would never answer a read), and no link to a directory. A name that isn't UTF-8 is kept as
/// it reads, to be said.
fn entries(dir: &Path) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let Ok(entry) = entry else { continue };
        let raw = entry.file_name();
        let (name, utf8) = (raw.to_string_lossy().into_owned(), raw.to_str().is_some());
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else { continue };
        let kind = if kind.is_symlink() {
            // Followed to a file only.
            match std::fs::metadata(entry.path()) {
                Ok(target) if target.is_file() => Kind::File,
                _ => continue,
            }
        } else if kind.is_file() {
            Kind::File
        } else if kind.is_dir() {
            Kind::Directory
        } else {
            continue;
        };
        entries.push(Entry { name, utf8, kind });
    }
    entries.sort_by(|a, b| natural(&a.name, &b.name));
    Ok(entries)
}

/// `a` against `b` the way people number files: runs of digits by their value, so `img2` comes
/// before `img10`, the rest letter by letter, ignoring ASCII case until all else is equal.
fn natural(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        let order = match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let digits = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut run = String::new();
                    while let Some(c) = chars.next_if(char::is_ascii_digit) {
                        run.push(c);
                    }
                    run
                };
                let (m, n) = (digits(&mut x), digits(&mut y));
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                // No leading zeros: the longer run is the larger number.
                m.len().cmp(&n.len()).then_with(|| m.cmp(n))
            }
            (Some(p), Some(q)) => {
                x.next();
                y.next();
                p.to_ascii_lowercase().cmp(&q.to_ascii_lowercase())
            }
        };
        if order != Ordering::Equal {
            return order;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of the test's own, holding `files` (a name ending in `/` is a directory).
    fn tree(test: &str, files: &[&str]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("clview-dir-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for file in files {
            let path = dir.join(file);
            if file.ends_with('/') {
                std::fs::create_dir_all(&path).unwrap();
            } else {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, file).unwrap();
            }
        }
        dir
    }

    fn names(listing: &Listing) -> Vec<&str> {
        listing.files.iter().map(|(_, name)| name.as_str()).collect()
    }

    #[test]
    fn numbers_sort_by_their_value_and_letters_without_case() {
        let mut names = ["img10.png", "img2.png", "IMG1.png", "img02.png", "b", "a10b", "a9b", "a", ""];
        names.sort_by(|a, b| natural(a, b));
        assert_eq!(names, ["", "a", "a9b", "a10b", "b", "IMG1.png", "img02.png", "img2.png", "img10.png"]);
        assert_eq!(natural("a", "A"), "a".cmp("A"), "only then by case");
        assert_eq!(natural("x99999999999999999999999", "x100000000000000000000000"), Ordering::Less, "past u64");
    }

    #[test]
    fn a_directory_lists_its_files_by_name_and_leaves_out_hidden_ones_and_subdirectories() {
        let dir = tree("flat", &["b10.pdf", "b9.png", ".hidden.png", "sub/c.png", "empty/"]);
        let listing = list(dir.to_str().unwrap(), false).unwrap();
        assert_eq!(names(&listing), ["b9.png", "b10.pdf"]);
        assert_eq!(listing.files[0].0, dir.join("b9.png").to_str().unwrap());
        assert!(listing.subdirectories && !listing.cut && listing.unread.is_empty(), "{listing:?}");
        assert!(list(dir.join("nothing").to_str().unwrap(), false).is_err_and(|e| e.contains("nothing: ")));
    }

    #[test]
    fn recursing_lists_each_subdirectorys_files_at_its_place() {
        let dir = tree("deep", &["a.png", "m/b.png", "m/n/c.png", "m/.d/e.png", "z.png"]);
        let listing = list(dir.to_str().unwrap(), true).unwrap();
        assert_eq!(names(&listing), ["a.png", "m/b.png", "m/n/c.png", "z.png"]);
        assert_eq!(listing.files[2].0, dir.join("m/n/c.png").to_str().unwrap());
        assert!(!listing.subdirectories);
        // Within limits: two files, or one directory down.
        let cut = list_within(dir.to_str().unwrap(), true, 2, MAX_DEPTH).unwrap();
        assert_eq!((names(&cut), cut.cut), (vec!["a.png", "m/b.png"], true));
        let shallow = list_within(dir.to_str().unwrap(), true, MAX_FILES, 1).unwrap();
        assert_eq!(names(&shallow), ["a.png", "m/b.png", "z.png"]);
        assert!(shallow.unread.iter().any(|(name, why)| name == "m/n" && why.ends_with("m/n: more than 1 directories down")), "{shallow:?}");
    }

    #[cfg(unix)]
    #[test]
    fn links_are_followed_to_files_only_and_pipes_are_left_out() {
        let dir = tree("links", &["a.png", "sub/b.png"]);
        std::os::unix::fs::symlink(dir.join("a.png"), dir.join("to-a.png")).unwrap();
        // A loop, if it were followed.
        std::os::unix::fs::symlink(&dir, dir.join("sub/up")).unwrap();
        std::os::unix::fs::symlink(dir.join("gone"), dir.join("broken.png")).unwrap();
        let made = std::process::Command::new("mkfifo").arg(dir.join("pipe.png")).status().is_ok_and(|s| s.success());
        let listing = list(dir.to_str().unwrap(), true).unwrap();
        assert_eq!(names(&listing), ["a.png", "sub/b.png", "to-a.png"], "made a pipe: {made}");
    }
}
