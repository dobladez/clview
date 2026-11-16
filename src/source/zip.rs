//! Zip archives, read through the `zip` crate ([`Zipped`]), shown as directories: the archive is a
//! container, and so is each directory in it. A file in it is read from the archive when the list
//! opens it, its first bytes only to choose a backend, and read again, whole, when it renders.
//! Nothing is written out: a name in the archive is only ever a name.

use std::rc::Rc;
use std::sync::Arc;

use super::tree::Tree;
use super::zipped::{MAX_BYTES, MAX_ENTRIES, Zipped};
use super::{Again, Backend, Badge, Bounded, Container, Format, HEAD, Input, Listing, Near, Opened, Reopenable, Start};
use crate::args::{OptionSpec, Parsed};

/// The zip backend.
pub(crate) struct Zip;

impl Backend for Zip {
    fn name(&self) -> &'static str {
        "zip"
    }

    fn shows(&self) -> &'static str {
        "zip archives"
    }

    fn formats(&self) -> Vec<Format> {
        vec![Format::of("zip", &[]).named_by(&["zip"])]
    }

    fn credits(&self) -> &[&'static str] {
        &["zip"]
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
        let error = |e: String| format!("{path}: {e}");
        let zipped = match input.reopenable()? {
            Reopenable::Disk(file) => Zipped::on_disk(&path, file),
            Reopenable::Bytes(bytes) => Zipped::in_memory(bytes),
            // An archive in an archive: read out of it whole, for the zip crate to seek through.
            Reopenable::Again(again) => again().and_then(Zipped::in_memory),
        };
        let archive = Archive::read(path.clone(), zipped.map_err(error)?);
        Ok(Opened::Files(Rc::new(Directory { path: archive.path.clone(), at: Vec::new(), archive: Arc::new(archive) })))
    }
}

/// An archive opened: its central directory, parsed once, and the directories its names make.
struct Archive {
    path: String,
    zipped: Zipped,
    tree: Tree,
    /// The names left out, and why, said when the archive's top is listed.
    unread: Vec<(String, String)>,
    /// Whether the central directory had more than [`MAX_ENTRIES`] entries.
    cut: bool,
}

impl Archive {
    /// The archive at `path`, which `zipped` reads.
    fn read(path: String, zipped: Zipped) -> Self {
        let (mut tree, mut unread) = (Tree::default(), Vec::new());
        let cut = zipped.len() > MAX_ENTRIES;
        for index in 0..zipped.len().min(MAX_ENTRIES) {
            let Some(name) = zipped.name(index) else { continue };
            if let Err(why) = tree.add(&name, false, index) {
                unread.push((name.clone(), format!("{path}/{name}: {why}")));
            }
        }
        Self { path, zipped, tree, unread, cut }
    }

    /// The file at `index`, called `path` for the reader, as a file held here: its first [`HEAD`]
    /// bytes read now, and the rest when it is asked for, whole, to a bound or its first bytes alone.
    fn held(self: &Arc<Self>, index: usize, path: String) -> Result<Input, String> {
        let head = self.zipped.first(index, HEAD).map_err(|e| format!("{path}: {e}"))?;
        let (whole, bound, first) = (self.clone(), self.clone(), self.clone());
        let again: Again = Arc::new(move || whole.zipped.whole(index, MAX_BYTES));
        let bounded: Bounded = Arc::new(move |most| bound.zipped.within(index, most));
        let start: Start = Arc::new(move |most| first.zipped.first(index, most));
        Ok(Input::held_bounded(path, head, again, bounded).starting(start))
    }

    /// What is beside a file held here ([`Near`]): the file at a path from the archive's top, by the
    /// names its directories list, so never a hidden one, nor anything a `..` would name.
    fn near(self: &Arc<Self>) -> Near {
        let archive = self.clone();
        Arc::new(move |path| {
            let index = archive.tree.file(&[], path).ok_or_else(|| format!("{}/{path}: not in the archive", archive.path))?;
            archive.held(index, format!("{}/{path}", archive.path))
        })
    }
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
        let inner =
            |at, name: &str| -> Rc<dyn Container> { Rc::new(Directory { archive: self.archive.clone(), at, path: format!("{}/{name}", self.path) }) };
        let mut listing = self.archive.tree.list(&self.at, recurse, inner).ok_or_else(|| format!("{}: not in the archive", self.path))?;
        if self.at.is_empty() {
            listing.unread.clone_from(&self.archive.unread);
            listing.cut |= self.archive.cut;
        }
        Ok(listing)
    }

    fn open(&self, name: &str) -> Result<Input, String> {
        let path = format!("{}/{name}", self.path);
        let index = self.archive.tree.file(&self.at, name).ok_or_else(|| format!("{path}: not in the archive"))?;
        // Where it is from the archive's top, for what it refers to beside it.
        let from_the_top = self.at.iter().map(String::as_str).chain([name]).collect::<Vec<_>>().join("/");
        Ok(self.archive.held(index, path)?.held_near(self.archive.near(), from_the_top))
    }

    /// The archive itself is zipped, its plate saying `ZIP`; a directory in it, a folder as any
    /// directory is.
    fn badge(&self) -> Badge {
        if self.at.is_empty() { Badge::Archive("ZIP") } else { Badge::None }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use ::zip::write::SimpleFileOptions;
    use ::zip::{CompressionMethod, ZipWriter};
    use proptest::prelude::*;

    use super::*;
    use crate::source::tree::{MAX_DEPTH, MAX_FILES};

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
        // The archive is zipped and says its kind, a directory in it a plain folder.
        assert_eq!((top.badge(), inner(&listing, "sub").badge()), (Badge::Archive("ZIP"), Badge::None));
        assert!(top.badge().spelled());
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

    /// A file in an archive gives its first bytes alone, and what is beside it in the archive, by a
    /// path from the archive's top: only what its directories list, under that very name.
    #[test]
    fn a_file_in_an_archive_gives_its_first_bytes_and_what_is_beside_it() {
        let big: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let bytes = archive(&[("docs/README.md", b"read me"), ("docs/img/a.png", &big), ("top.png", b"top"), (".hidden/b.png", b"hidden")]);
        let path = on_disk("near", "t.zip", &bytes);
        let top = opened(&path).unwrap();
        let docs = inner(&top.list(false).unwrap(), "docs");
        for input in [top.open("docs/README.md").unwrap(), docs.open("README.md").unwrap()] {
            let beside = input.near().expect("what is beside it");
            let (near, at) = &*beside;
            assert_eq!(at, "docs/README.md");
            let picture = near("docs/img/a.png").unwrap();
            assert_eq!((picture.path.as_str(), picture.head.as_slice()), (format!("{path}/docs/img/a.png").as_str(), &big[..1024]));
            let start = picture.start().expect("its first bytes");
            assert_eq!(start(2000).unwrap(), &big[..2000]);
            assert_eq!(start(1 << 20).unwrap(), big, "all of a shorter file");
            assert_eq!(picture.bytes().unwrap(), big);
            assert_eq!(near("top.png").unwrap().bytes().unwrap(), b"top");
            for missing in ["docs/img/../img/a.png", "../t.zip", "docs/../top.png", ".hidden/b.png", "docs", "", "/top.png", "docs/img/a.png/"] {
                assert!(near(missing).is_err_and(|e| e.ends_with(": not in the archive")), "{missing}");
            }
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
        let archive = Archive::read(path.clone(), Zipped::on_disk(&path, std::fs::File::open(&path).unwrap()).unwrap());
        let index = archive.tree.file(&[], "open.txt").unwrap();
        assert!(archive.zipped.whole(index, 99).is_err_and(|e| e.contains("unpacks to more than")));
        assert_eq!(archive.zipped.whole(index, 100).unwrap(), [7; 100]);
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
