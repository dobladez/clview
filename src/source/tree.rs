//! The directories an archive's names make, for the archives shown as directories (zip, tar):
//! built once from its entries, then listed and searched the way a directory on disk is.

use std::collections::BTreeMap;
use std::rc::Rc;

use super::{Container, Listed, Listing};

/// The most files and directories listed from a directory in an archive, as from one on disk.
pub(super) const MAX_FILES: usize = 4096;

/// The most directories down a name in an archive goes.
pub(super) const MAX_DEPTH: usize = 32;

/// A directory in an archive, by the names in it.
#[derive(Default)]
pub(super) struct Tree {
    /// By name: each file's index in the archive.
    files: BTreeMap<String, usize>,
    directories: BTreeMap<String, Tree>,
}

impl Tree {
    /// The entry at `index` in the archive, called `name` there: a file, or a directory when
    /// `directory` (or when its name ends with `/`). Hidden names are passed over, as on disk
    /// (`..` too, which is only a name here), and so are macOS's copies of what a file system of
    /// its own keeps beside a file; a name too deep is refused, with why.
    pub(super) fn add(&mut self, name: &str, directory: bool, index: usize) -> Result<(), String> {
        let parts: Vec<&str> = name.split('/').filter(|part| !part.is_empty() && *part != ".").collect();
        if parts.is_empty() || parts.iter().any(|part| part.starts_with('.') || *part == "__MACOSX") {
            return Ok(());
        }
        if parts.len() > MAX_DEPTH {
            return Err(format!("more than {MAX_DEPTH} directories down"));
        }
        let Some((last, above)) = parts.split_last() else { return Ok(()) };
        let mut tree = self;
        for part in above {
            tree = tree.directories.entry((*part).to_owned()).or_default();
        }
        if directory || name.ends_with('/') {
            tree.directories.entry((*last).to_owned()).or_default();
        } else {
            // The first of two entries of the same name.
            tree.files.entry((*last).to_owned()).or_insert(index);
        }
        Ok(())
    }

    /// What the directory `at` (its names from the top) holds, as a container lists it: its
    /// files, and its directories as the containers `inner` makes from their names from the top
    /// and in it, or with `recurse`, their files at their place instead; at most [`MAX_FILES`].
    /// `None` when there is no such directory.
    pub(super) fn list(&self, at: &[String], recurse: bool, inner: impl Fn(Vec<String>, &str) -> Rc<dyn Container>) -> Option<Listing> {
        let tree = self.find(at)?;
        let mut listing = Listing::default();
        // Each directory being listed, from this one down: the prefix of its names, and the
        // entries left. Directories in an archive go at most `MAX_DEPTH` down.
        let mut open = vec![(String::new(), tree.entries().into_iter())];
        while let Some((prefix, mut left)) = open.pop() {
            let Some((name, entry)) = left.next() else { continue };
            open.push((prefix.clone(), left));
            if listing.entries.len() >= MAX_FILES {
                listing.cut = true;
                break;
            }
            let name = format!("{prefix}{name}");
            match entry {
                Entry::File => listing.entries.push(Listed { name, container: None }),
                Entry::Directory(tree) if recurse => open.push((format!("{name}/"), tree.entries().into_iter())),
                Entry::Directory(_) => {
                    let names = at.iter().cloned().chain(name.split('/').map(String::from)).collect();
                    let container = inner(names, &name);
                    listing.entries.push(Listed { name, container: Some(container) });
                }
            }
        }
        Some(listing)
    }

    /// The index of the file called `name` in the directory `at` (its names from the top).
    pub(super) fn file(&self, at: &[String], name: &str) -> Option<usize> {
        let parts: Vec<&str> = name.split('/').collect();
        let (file, above) = parts.split_last()?;
        let at: Vec<String> = at.iter().cloned().chain(above.iter().map(|part| (*part).to_owned())).collect();
        self.find(&at)?.files.get(*file).copied()
    }

    /// The directory `at` down from this one.
    fn find(&self, at: &[String]) -> Option<&Tree> {
        at.iter().try_fold(self, |tree, part| tree.directories.get(part))
    }

    /// Its files and directories, in the order a directory on disk lists them.
    fn entries(&self) -> Vec<(&str, Entry<'_>)> {
        let files = self.files.keys().map(|name| (name.as_str(), Entry::File));
        let directories = self.directories.iter().map(|(name, tree)| (name.as_str(), Entry::Directory(tree)));
        let mut entries: Vec<_> = files.chain(directories).collect();
        entries.sort_by(|(a, _), (b, _)| super::dir::natural(a, b));
        entries
    }
}

#[derive(Clone, Copy)]
enum Entry<'a> {
    /// Opened by its name ([`Tree::file`]).
    File,
    Directory(&'a Tree),
}
