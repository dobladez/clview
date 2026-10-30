//! Zip archives, read through the `zip` crate, shown as directories: the archive is a container,
//! and so is each directory in it. A file in it is read from the archive when the list opens it,
//! its first bytes only to choose a backend, and read again, whole, when it renders. Nothing is
//! written out: a name in the archive is only ever a name.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::rc::Rc;
use std::sync::Arc;

use ::zip::ZipArchive;

use super::{Again, Backend, Container, HEAD, Input, Listed, Listing, Opened, Reopenable};
use crate::args::{OptionSpec, Parsed};

/// The most bytes a file in an archive is read to: a picture larger than that is more than the
/// image backend opens, and it bounds what an entry that unpacks without end (a zip bomb) costs.
const MAX_BYTES: u64 = 512 << 20;

/// The most files and directories listed from a directory in an archive, as from one on disk.
const MAX_FILES: usize = 4096;

/// The most entries read from an archive's central directory, shown or not.
const MAX_ENTRIES: usize = 100_000;

/// The most directories down a name in an archive goes.
const MAX_DEPTH: usize = 32;

/// The zip backend.
pub(crate) struct Zip;

impl Backend for Zip {
    fn name(&self) -> &'static str {
        "zip"
    }

    fn shows(&self) -> &'static str {
        "zip archives"
    }

    fn options(&self) -> &'static [OptionSpec] {
        &[]
    }

    /// By its first local header, or by the end of the central directory for an archive with
    /// nothing in it.
    fn opens(&self, _path: &str, head: &[u8]) -> bool {
        head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06")
    }

    fn open(&self, input: Input, _options: &Parsed<'_>) -> Result<Opened, String> {
        let path = input.path.clone();
        let reader = match input.reopenable()? {
            Reopenable::Disk(file) => Reader::Disk { path: Arc::from(path.as_str()), file: Some(file), at: 0 },
            Reopenable::Bytes(bytes) => Reader::Memory(Cursor::new(bytes.into())),
            // An archive in an archive: read out of it whole, for the zip crate to seek through.
            Reopenable::Again(again) => Reader::Memory(Cursor::new(again().map_err(|e| format!("{path}: {e}"))?.into())),
        };
        let archive = Archive::read(path, reader)?;
        Ok(Opened::Files(Rc::new(Directory { path: archive.path.clone(), at: Vec::new(), archive: Arc::new(archive) })))
    }
}

/// An archive opened: its central directory, parsed once, and the directories its names make.
struct Archive {
    path: String,
    /// Holding no file while idle: each read works on a clone, which opens the file for itself.
    zip: ZipArchive<Reader>,
    tree: Tree,
    /// The names left out, and why, said when the archive's top is listed.
    unread: Vec<(String, String)>,
    /// Whether the central directory had more than [`MAX_ENTRIES`] entries.
    cut: bool,
}

impl Archive {
    /// The archive at `path`, which `reader` reads.
    fn read(path: String, reader: Reader) -> Result<Self, String> {
        let parsed = caught(|| ZipArchive::new(reader).map_err(|e| e.to_string())).map_err(|e| format!("{path}: {e}"))?;
        // A clone shares what was parsed, without the file the parse opened, which goes with
        // `parsed`.
        let zip = parsed.clone();
        drop(parsed);
        let (mut tree, mut unread) = (Tree::default(), Vec::new());
        let cut = zip.len() > MAX_ENTRIES;
        for index in 0..zip.len().min(MAX_ENTRIES) {
            let Some(Ok(name)) = zip.name_for_index(index) else { continue };
            let parts: Vec<&str> = name.split('/').filter(|part| !part.is_empty() && *part != ".").collect();
            // Hidden, as on disk (`..` too, which is only a name here), and macOS's copies of
            // what a file system of its own keeps beside a file.
            if parts.is_empty() || parts.iter().any(|part| part.starts_with('.') || *part == "__MACOSX") {
                continue;
            }
            if parts.len() > MAX_DEPTH {
                unread.push((name.to_string(), format!("{path}/{name}: more than {MAX_DEPTH} directories down")));
                continue;
            }
            tree.insert(&parts, name.ends_with('/'), index);
        }
        Ok(Self { path, zip, tree, unread, cut })
    }

    /// The first [`HEAD`] bytes of the file at `index`, or all of a shorter one.
    fn head(&self, index: usize) -> Result<Vec<u8>, String> {
        caught(|| {
            let mut zip = self.zip.clone();
            let entry = zip.by_index(index).map_err(|e| e.to_string())?;
            let mut head = Vec::new();
            entry.take(HEAD).read_to_end(&mut head).map_err(|e| e.to_string())?;
            Ok(head)
        })
    }

    /// The file at `index`, whole, as long as it unpacks to at most `max` bytes.
    fn whole(&self, index: usize, max: u64) -> Result<Vec<u8>, String> {
        caught(|| {
            let mut zip = self.zip.clone();
            let entry = zip.by_index(index).map_err(|e| e.to_string())?;
            let too_large = || format!("it unpacks to more than {} MiB, more than {} reads from an archive", max >> 20, crate::NAME);
            // What the archive says first, then what it unpacks to, which may be more.
            if entry.size() > max {
                return Err(too_large());
            }
            let mut bytes = Vec::new();
            entry.take(max.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            match u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max {
                true => Err(too_large()),
                false => Ok(bytes),
            }
        })
    }
}

/// What `read` returns, or an error if the zip crate panics on the archive: its bug costs the
/// archive or the file, not the viewer.
fn caught<T>(read: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|_| Err("the archive reader failed on it".into()))
}

/// A directory in an archive, by the names in it.
#[derive(Default)]
struct Tree {
    /// By name: each file's index in the archive.
    files: BTreeMap<String, usize>,
    directories: BTreeMap<String, Tree>,
}

impl Tree {
    /// The entry at `index`, named by `parts`: a file, or a directory when `directory`.
    fn insert(&mut self, parts: &[&str], directory: bool, index: usize) {
        let Some((last, above)) = parts.split_last() else { return };
        let mut tree = self;
        for part in above {
            tree = tree.directories.entry((*part).to_owned()).or_default();
        }
        if directory {
            tree.directories.entry((*last).to_owned()).or_default();
        } else {
            // The first of two entries of the same name.
            tree.files.entry((*last).to_owned()).or_insert(index);
        }
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
    /// Opened by its name ([`Directory::open`]).
    File,
    Directory(&'a Tree),
}

/// A directory in an archive, the archive's top or one in it, as a container.
struct Directory {
    archive: Arc<Archive>,
    /// Its names from the archive's top: none for the top.
    at: Vec<String>,
    /// The archive's path, and its names: `photos.zip/2024`.
    path: String,
}

impl Container for Directory {
    fn path(&self) -> &str {
        &self.path
    }

    fn list(&self, recurse: bool) -> Result<Listing, String> {
        let tree = self.archive.tree.find(&self.at).ok_or_else(|| format!("{}: not in the archive", self.path))?;
        let mut listing = Listing::default();
        if self.at.is_empty() {
            listing.unread.clone_from(&self.archive.unread);
            listing.cut = self.archive.cut;
        }
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
                    let at = self.at.iter().cloned().chain(name.split('/').map(String::from)).collect();
                    let inner = Directory { archive: self.archive.clone(), at, path: format!("{}/{name}", self.path) };
                    listing.entries.push(Listed { name, container: Some(Rc::new(inner)) });
                }
            }
        }
        Ok(listing)
    }

    fn open(&self, name: &str) -> Result<Input, String> {
        let path = format!("{}/{name}", self.path);
        let parts: Vec<&str> = name.split('/').collect();
        let index = parts.split_last().and_then(|(file, above)| {
            let at: Vec<String> = self.at.iter().cloned().chain(above.iter().map(|part| (*part).to_owned())).collect();
            self.archive.tree.find(&at)?.files.get(*file).copied()
        });
        let index = index.ok_or_else(|| format!("{path}: not in the archive"))?;
        let head = self.archive.head(index).map_err(|e| format!("{path}: {e}"))?;
        let archive = self.archive.clone();
        let again: Again = Arc::new(move || archive.whole(index, MAX_BYTES));
        Ok(Input::held(path, head, again))
    }
}

/// An archive's bytes, for the zip crate to seek through: in memory, or on disk, where the file
/// is open only while it is read. A clone opens it for itself, so the archive kept between reads
/// holds no file, and reads on several threads don't share one.
enum Reader {
    Disk { path: Arc<str>, file: Option<File>, at: u64 },
    Memory(Cursor<Arc<[u8]>>),
}

impl Reader {
    /// The file, opened at the position read up to, if it wasn't open.
    fn file<'a>(file: &'a mut Option<File>, path: &str, at: u64) -> std::io::Result<&'a mut File> {
        if file.is_none() {
            let mut opened = File::open(path)?;
            opened.seek(SeekFrom::Start(at))?;
            *file = Some(opened);
        }
        file.as_mut().ok_or_else(|| std::io::Error::other("the archive isn't open"))
    }
}

impl Clone for Reader {
    fn clone(&self) -> Self {
        match self {
            Self::Disk { path, at, .. } => Self::Disk { path: path.clone(), file: None, at: *at },
            Self::Memory(bytes) => Self::Memory(bytes.clone()),
        }
    }
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Disk { path, file, at } => {
                let read = Self::file(file, path, *at)?.read(buf)?;
                *at = at.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
                Ok(read)
            }
            Self::Memory(bytes) => bytes.read(buf),
        }
    }
}

impl Seek for Reader {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::Disk { path, file, at } => {
                *at = Self::file(file, path, *at)?.seek(to)?;
                Ok(*at)
            }
            Self::Memory(bytes) => bytes.seek(to),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use ::zip::write::SimpleFileOptions;
    use ::zip::{CompressionMethod, ZipWriter};
    use proptest::prelude::*;

    use super::*;

    /// An archive of `files`, by name and contents: a name ending in `/` is a directory, and the
    /// contents are deflated.
    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, contents) in files {
            match name.strip_suffix('/') {
                Some(directory) => zip.add_directory(directory, options).unwrap(),
                None => {
                    zip.start_file(*name, options).unwrap();
                    zip.write_all(contents).unwrap();
                }
            }
        }
        zip.finish().unwrap().into_inner()
    }

    /// `bytes` in a file of the test's own, called `name`.
    fn on_disk(test: &str, name: &str, bytes: &[u8]) -> String {
        let dir = std::env::temp_dir().join(format!("clview-zip-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_owned()
    }

    /// The archive at `path`, opened as the viewer opens it.
    fn opened(path: &str) -> Result<Rc<dyn Container>, String> {
        let input = Input::open(path)?;
        assert!(Zip.opens(path, &input.head));
        match Zip.open(input, &Parsed::default())? {
            Opened::Files(container) => Ok(container),
            Opened::Pages(_) => panic!("an archive is a container"),
        }
    }

    /// The names listed, a directory's with a `/` after it.
    fn names(listing: &Listing) -> Vec<String> {
        listing.entries.iter().map(|e| if e.container.is_some() { format!("{}/", e.name) } else { e.name.clone() }).collect()
    }

    /// The container listed as `name` in `listing`.
    fn inner(listing: &Listing, name: &str) -> Rc<dyn Container> {
        listing.entries.iter().find(|e| e.name == name).and_then(|e| e.container.clone()).unwrap()
    }

    #[test]
    fn an_archive_lists_as_a_directory_does_and_its_directories_are_containers() {
        let bytes = archive(&[
            ("b10.png", b"ten"),
            ("b9.png", b"nine"),
            (".hidden.png", b"no"),
            ("__MACOSX/._b9.png", b"no"),
            ("sub/../c.png", b"a name, not a way out"),
            ("sub/c.png", b"c"),
            ("empty/", b""),
            ("m/n/d.png", b"d"),
        ]);
        let path = on_disk("list", "t.zip", &bytes);
        let top = opened(&path).unwrap();
        assert_eq!(top.path(), path);
        let listing = top.list(false).unwrap();
        assert_eq!(names(&listing), ["b9.png", "b10.png", "empty/", "m/", "sub/"]);
        assert!(listing.unread.is_empty() && !listing.cut, "{listing:?}");
        // Each directory a container of its own, named from the archive.
        let sub = inner(&listing, "sub");
        assert_eq!(sub.path(), format!("{path}/sub"));
        assert_eq!(names(&sub.list(false).unwrap()), ["c.png"]);
        assert!(inner(&listing, "empty").list(false).unwrap().entries.is_empty());
        let m = inner(&listing, "m");
        assert_eq!(names(&m.list(false).unwrap()), ["n/"]);
        assert_eq!(inner(&m.list(false).unwrap(), "n").path(), format!("{path}/m/n"));
        // Recursing, each directory's files at its place.
        assert_eq!(names(&top.list(true).unwrap()), ["b9.png", "b10.png", "m/n/d.png", "sub/c.png"]);
        assert_eq!(names(&m.list(true).unwrap()), ["n/d.png"]);
    }

    #[test]
    fn a_file_in_an_archive_opens_by_its_head_and_is_read_whole_again() {
        let big: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let bytes = archive(&[("big.bin", &big), ("d/small.txt", b"small")]);
        let path = on_disk("open", "t.zip", &bytes);
        let top = opened(&path).unwrap();
        let input = top.open("big.bin").unwrap();
        assert_eq!((input.path.as_str(), input.head.as_slice()), (format!("{path}/big.bin").as_str(), &big[..1024]));
        let Ok(Reopenable::Again(again)) = input.reopenable() else { panic!("held in the archive") };
        assert_eq!(again().unwrap(), big);
        assert_eq!(again().unwrap(), big, "every time");
        // Recursing names it from where it is listed; listed in its directory, from there.
        assert_eq!(top.open("d/small.txt").unwrap().bytes().unwrap(), b"small");
        let d = inner(&top.list(false).unwrap(), "d");
        assert_eq!(d.open("small.txt").unwrap().bytes().unwrap(), b"small");
        for missing in ["nothing", "d", "d/", "big.bin/x", "", "/big.bin"] {
            assert!(top.open(missing).is_err_and(|e| e.ends_with(": not in the archive")), "{missing}");
        }
    }

    #[test]
    fn an_archive_holds_no_file_while_idle_so_its_file_can_go() {
        let bytes = archive(&[("a.txt", b"first")]);
        let path = on_disk("idle", "t.zip", &bytes);
        let top = opened(&path).unwrap();
        let input = top.open("a.txt").unwrap();
        assert_eq!(input.bytes().unwrap(), b"first");
        // Gone: the archive opens the file only to read, and says so.
        std::fs::remove_file(&path).unwrap();
        assert!(top.open("a.txt").is_err());
        let reader = Reader::Disk { path: Arc::from(path.as_str()), file: None, at: 7 };
        let Reader::Disk { file, at, .. } = reader.clone() else { panic!("on disk") };
        assert!(file.is_none() && at == 7);
    }

    #[test]
    fn an_archive_in_an_archive_opens_from_it() {
        let nested = archive(&[("in/deep.txt", b"deep")]);
        let bytes = archive(&[("nested.zip", &nested)]);
        let path = on_disk("nested", "t.zip", &bytes);
        let input = opened(&path).unwrap().open("nested.zip").unwrap();
        assert!(Zip.opens(&input.path, &input.head));
        let Opened::Files(nested) = Zip.open(input, &Parsed::default()).unwrap() else { panic!("a container") };
        assert_eq!(nested.path(), format!("{path}/nested.zip"));
        assert_eq!(names(&nested.list(true).unwrap()), ["in/deep.txt"]);
        assert_eq!(nested.open("in/deep.txt").unwrap().bytes().unwrap(), b"deep");
    }

    #[test]
    fn what_an_archive_cant_give_is_an_error_for_that_file() {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, contents) in [("secret.txt", &b"secret"[..]), ("packed.txt", b"packed"), ("open.txt", &[7; 100])] {
            zip.start_file(name, SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
            zip.write_all(contents).unwrap();
        }
        let mut bytes = zip.finish().unwrap().into_inner();
        // The first entry encrypted, and the second packed by zstd, which this build leaves
        // out: as their local and central headers say.
        let at = |signature: &[u8]| -> Vec<usize> { bytes.windows(4).enumerate().filter(|(_, w)| *w == signature).map(|(i, _)| i).collect() };
        let (local, central) = (at(b"PK\x03\x04"), at(b"PK\x01\x02"));
        bytes[local[0] + 6] |= 1;
        bytes[central[0] + 8] |= 1;
        bytes[local[1] + 8] = 93;
        bytes[central[1] + 10] = 93;
        let path = on_disk("refused", "t.zip", &bytes);
        let top = opened(&path).unwrap();
        for (name, why) in [("secret.txt", "Password required"), ("packed.txt", "compression method not supported")] {
            let error = top.open(name).err().unwrap();
            assert!(error.starts_with(&format!("{path}/{name}: ")) && error.contains(why), "{error}");
        }
        // Larger unpacked than read: by what the archive says, before reading it.
        let archive = Archive::read(path.clone(), Reader::Disk { path: Arc::from(path.as_str()), file: None, at: 0 }).unwrap();
        let index = archive.tree.files["open.txt"];
        assert!(archive.whole(index, 99).is_err_and(|e| e.contains("unpacks to more than")));
        assert_eq!(archive.whole(index, 100).unwrap(), [7; 100]);
        // Not an archive after all.
        let path = on_disk("refused", "fake.zip", b"PK\x03\x04 and then nothing");
        assert!(opened(&path).is_err_and(|e| e.starts_with(&path)));
    }

    #[test]
    fn names_too_deep_are_said_and_a_listing_stops_at_its_limit() {
        let deep = format!("{}x.png", "d/".repeat(MAX_DEPTH));
        let mut files: Vec<(String, Vec<u8>)> = vec![(deep.clone(), b"x".to_vec())];
        files.extend((0..=MAX_FILES).map(|i| (format!("f{i}.png"), Vec::new())));
        let files: Vec<(&str, &[u8])> = files.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect();
        let path = on_disk("limits", "t.zip", &archive(&files));
        let listing = opened(&path).unwrap().list(false).unwrap();
        assert_eq!((listing.entries.len(), listing.cut), (MAX_FILES, true));
        assert_eq!(listing.unread, [(deep.clone(), format!("{path}/{deep}: more than {MAX_DEPTH} directories down"))]);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Whatever an archive's bytes are, it opens or is an error, and so does each file it
        /// lists: nothing panics.
        #[test]
        fn any_bytes_open_or_are_an_error(flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 0..16), tail in prop::collection::vec(any::<u8>(), 0..64)) {
            let mut bytes = archive(&[("a/b.png", b"bytes of a picture"), ("c.pdf", &[1; 300]), ("e/", b"")]);
            for (at, byte) in flips {
                let at = at.index(bytes.len());
                bytes[at] ^= byte;
            }
            bytes.extend(tail);
            let path = on_disk("any", "t.zip", &bytes);
            let input = Input::open(&path).unwrap();
            if let Ok(Opened::Files(top)) = Zip.open(input, &Parsed::default()) {
                for recurse in [false, true] {
                    for entry in top.list(recurse).map(|l| l.entries).unwrap_or_default() {
                        if let Ok(input) = top.open(&entry.name) {
                            let _ = input.bytes();
                        }
                    }
                }
            }
        }
    }
}
