//! A zip, read through the `zip` crate, for the backends whose files are zips they read themselves
//! (the zip backend's archives, an epub's book): its central directory parsed once, and a file read
//! from it when asked, whole, to a bound or its first bytes alone, each within the limits here.
//! Nothing is written out, and it holds no open file while idle.

use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::sync::Arc;

use ::zip::ZipArchive;

/// The most bytes a file in a zip is read to: a picture larger than that is more than the image
/// backend opens, and it bounds what an entry that unpacks without end (a zip bomb) costs.
pub(super) const MAX_BYTES: u64 = 512 << 20;

/// The most entries read from a zip's central directory, shown or not.
pub(super) const MAX_ENTRIES: usize = 100_000;

/// A zip opened: its central directory, parsed once.
pub(super) struct Zipped {
    /// Holding no file while idle: each read works on a clone, which opens the file for itself.
    zip: ZipArchive<Reader>,
}

impl Zipped {
    /// The zip in the file at `path`, read from `file`, which is opened again by that path for each
    /// read after.
    pub(super) fn on_disk(path: &str, file: File) -> Result<Self, String> {
        Self::read(Reader::Disk { path: Arc::from(path), file: Some(file), at: 0 })
    }

    /// The zip in `bytes`.
    pub(super) fn in_memory(bytes: Vec<u8>) -> Result<Self, String> {
        Self::read(Reader::Memory(Cursor::new(bytes.into())))
    }

    fn read(reader: Reader) -> Result<Self, String> {
        let parsed = caught(|| ZipArchive::new(reader).map_err(|e| e.to_string()))?;
        // A clone shares what was parsed, without the file the parse opened, which goes with
        // `parsed`.
        let zip = parsed.clone();
        drop(parsed);
        Ok(Self { zip })
    }

    /// How many entries its central directory has, those past [`MAX_ENTRIES`] too.
    pub(super) fn len(&self) -> usize {
        self.zip.len()
    }

    /// The name of the entry at `index`, one of the first [`MAX_ENTRIES`], if it is text.
    pub(super) fn name(&self, index: usize) -> Option<String> {
        match self.zip.name_for_index(index).filter(|_| index < MAX_ENTRIES) {
            Some(Ok(name)) => Some(name.to_string()),
            _ => None,
        }
    }

    /// Which entry is called exactly `name`, among the first [`MAX_ENTRIES`].
    #[cfg_attr(not(feature = "epub"), expect(dead_code, reason = "only a book finds its files by their very names"))]
    pub(super) fn index_of(&self, name: &str) -> Option<usize> {
        self.zip.index_for_name(name).filter(|index| *index < MAX_ENTRIES)
    }

    /// The first `most` bytes of the file at `index`, or all of a shorter one, unpacking no more.
    pub(super) fn first(&self, index: usize, most: u64) -> Result<Vec<u8>, String> {
        caught(|| {
            let mut zip = self.zip.clone();
            let entry = zip.by_index(index).map_err(|e| e.to_string())?;
            let mut first = Vec::new();
            entry.take(most.min(MAX_BYTES)).read_to_end(&mut first).map_err(|e| e.to_string())?;
            Ok(first)
        })
    }

    /// The file at `index`, whole, as long as it unpacks to at most `max` bytes.
    pub(super) fn whole(&self, index: usize, max: u64) -> Result<Vec<u8>, String> {
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

    /// File `index`, unpacked whole if it is no more than `most` bytes, else `None`, unpacking no more
    /// than a byte past them: past [`MAX_BYTES`], as [`Zipped::whole`] says.
    pub(super) fn within(&self, index: usize, most: u64) -> Result<Option<Vec<u8>>, String> {
        if most >= MAX_BYTES {
            return self.whole(index, MAX_BYTES).map(Some);
        }
        caught(|| {
            let mut zip = self.zip.clone();
            let entry = zip.by_index(index).map_err(|e| e.to_string())?;
            // What the archive says first, then what it unpacks to, which may be more.
            if entry.size() > most {
                return Ok(None);
            }
            let mut bytes = Vec::new();
            entry.take(most.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            Ok((u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= most).then_some(bytes))
        })
    }
}

/// What `read` returns, or an error if the zip crate panics on the archive: its bug costs the
/// archive or the file, not the viewer.
fn caught<T>(read: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|_| Err("the archive reader failed on it".into()))
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
