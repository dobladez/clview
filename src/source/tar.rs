//! Tar archives, plain or compressed (gzip, bzip2, xz), shown as directories, and a file
//! compressed alone (`paper.pdf.gz`), shown as the file it unpacks to. Decompressed in Rust:
//! gzip through flate2, bzip2 through the bzip2 crate (libbz2-rs-sys), xz through lzma-rust2.
//! The tar format is read here, every length it gives capped.
//!
//! A compressed tar has no index and can't be read from the middle: its files are found by
//! reading it through, once, on a thread of its own, which the viewer watches
//! ([`Container::ready`]) and can stop. That pass keeps where each file starts and the first
//! bytes of the first ones, and writes the archive, unpacked, to a file of its own ([`Spill`]),
//! removed as soon as it is made, so that nothing can leave it behind; a file is read from there,
//! in any order. Past [`MAX_SPILL`], or with no such file to be had, a file is read by one
//! decompressor kept going forward through the archive, which starts over only for a file before
//! where it is. All of it goes when the reader leaves the archive ([`Container::release`]), or
//! with the last of its files that could still be read. ADR-0004 has the measurements.

use std::fs::{File, OpenOptions};
use std::io::{Cursor, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use super::tree::Tree;
use super::{Again, BACKENDS, Backend, Badge, Container, Format, HEAD, Input, Listing, Opened, Ready, Reopenable};
use crate::args::{OptionSpec, Parsed};

/// The most bytes a file in an archive, or a file compressed alone, is read to, as for zip: it
/// bounds what a file that unpacks without end (a bomb) costs.
const MAX_BYTES: u64 = 512 << 20;

/// The most headers read from an archive, shown or not.
const MAX_ENTRIES: usize = 100_000;

/// The most bytes an archive is read through, unpacked, to find its files: past it, the listing
/// stops there.
const MAX_SCANNED: u64 = 32 << 30;

/// The most files whose first bytes the pass through keeps, to choose their backends without
/// reading them again.
const MAX_HEADS: usize = 8192;

/// The most bytes of a long name or of a pax header's records.
const MAX_NAME: u64 = 64 << 10;

/// The most bytes of an archive the pass through writes out, unpacked, to read its files from:
/// what it takes of the disk, for as long as the reader is in the archive.
const MAX_SPILL: u64 = 4 << 30;

/// The most memory the xz decoder may take, in KiB: `xz -9` needs 65 MiB; an archive asking more
/// is refused.
const XZ_MEMORY: u32 = 128 << 10;

/// A tar block: headers, and the files' data padded to it.
const BLOCK: u64 = 512;

/// The tar backend.
pub(crate) struct Tar;

impl Backend for Tar {
    fn name(&self) -> &'static str {
        "tar"
    }

    fn shows(&self) -> &'static str {
        "tar archives and compressed files"
    }

    /// A plain tar, by its header, and each compression it opens, by its magic number.
    fn formats(&self) -> Vec<Format> {
        [Codec::None].into_iter().chain(Codec::COMPRESSED).map(Codec::format).collect()
    }

    fn options(&self) -> &'static [OptionSpec] {
        &[]
    }

    /// By its compression's magic number (gzip, bzip2, xz), or by a tar header first.
    fn opens(&self, _path: &str, head: &[u8]) -> bool {
        Codec::of(head).is_some() || is_tar(head)
    }

    fn open(&self, input: Input, options: &Parsed<'_>) -> Result<Opened, String> {
        let path = input.path.clone();
        let codec = Codec::of(&input.head).unwrap_or(Codec::None);
        let raw = match input.reopenable()? {
            // Opened again by its path, on whichever thread reads it.
            Reopenable::Disk(file) => Raw::Disk(Arc::from(path.as_str()), file.metadata().ok().map(|m| m.len())),
            Reopenable::Bytes(bytes) => Raw::Memory(bytes.into()),
            Reopenable::Again(again) => Raw::Held(again),
        };
        if codec != Codec::None {
            let head = caught(|| {
                let mut head = Vec::new();
                stream(&raw, codec, None)?.take(HEAD).read_to_end(&mut head).map_err(|e| e.to_string())?;
                Ok(head)
            })
            .map_err(|e| format!("{path}: {e}"))?;
            if !is_tar(&head) {
                return alone(path, raw, codec, head, options);
            }
        }
        let shared = Arc::new(Shared::new(path.clone(), raw, codec));
        let handle = Rc::new(Handle(shared));
        Ok(Opened::Files(Rc::new(Directory { handle, at: Vec::new(), path })))
    }
}

/// A file compressed alone, opened as what it unpacks to (`paper.pdf.gz` as a PDF) by the first
/// other backend that opens it. It unpacks again, whole, every time it is read.
fn alone(path: String, raw: Raw, codec: Codec, head: Vec<u8>, options: &Parsed<'_>) -> Result<Opened, String> {
    let again: Again = Arc::new(move || caught(|| unpack(&raw, codec, MAX_BYTES)));
    let input = Input::held(path, head, again);
    // Not this one: a file compressed twice is left as it is.
    let backend = BACKENDS.iter().filter(|b| b.name() != Tar.name()).find(|b| b.opens(&input.path, &input.head));
    match backend {
        Some(backend) => backend.open(input, options),
        None => Err(format!("{}: it unpacks to a file that {} doesn't show", input.path, crate::NAME)),
    }
}

/// A file compressed alone, unpacked: at most `max` bytes ([`MAX_BYTES`]).
fn unpack(raw: &Raw, codec: Codec, max: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    stream(raw, codec, None)?.take(max.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    match u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max {
        true => Err(format!("it unpacks to more than {} MiB, more than {} unpacks", max >> 20, crate::NAME)),
        false => Ok(bytes),
    }
}

/// How an archive is compressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Codec {
    None,
    Gzip,
    Bzip2,
    Xz,
}

impl Codec {
    /// The compressions a file is known to have, each by its magic number ([`Codec::starts`]).
    const COMPRESSED: [Self; 3] = [Self::Gzip, Self::Bzip2, Self::Xz];

    /// The compression a file starting with `head` has, by its magic number.
    fn of(head: &[u8]) -> Option<Self> {
        Self::COMPRESSED.into_iter().find(|codec| codec.starts(head))
    }

    /// Whether `head` starts with this compression's magic number.
    fn starts(self, head: &[u8]) -> bool {
        match self {
            Self::None => false,
            Self::Gzip => head.starts_with(&[0x1f, 0x8b, 0x08]),
            Self::Bzip2 => head.starts_with(b"BZh") && head.get(3).is_some_and(|level| (b'1'..=b'9').contains(level)),
            Self::Xz => head.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]),
        }
    }

    /// The format it is, as `--help` lists it: uncompressed, a plain tar.
    fn format(self) -> Format {
        let name = match self {
            Self::None => "tar",
            Self::Gzip => "gzip",
            Self::Bzip2 => "bzip2",
            Self::Xz => "xz",
        };
        Format { name, extensions: &[] }
    }

    /// `raw`, unpacked as it reads. Several compressed streams one after another (`pigz`, `pbzip2`,
    /// `xz -T`) read as one.
    fn decode(self, raw: Box<dyn Read + Send>) -> Box<dyn Read + Send> {
        match self {
            Self::None => raw,
            Self::Gzip => Box::new(flate2::read::MultiGzDecoder::new(raw)),
            Self::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(raw)),
            Self::Xz => Box::new(lzma_rust2::XzReader::new_mem_limit(raw, true, XZ_MEMORY)),
        }
    }
}

/// Where an archive's bytes are, read from the start each time.
#[derive(Clone)]
enum Raw {
    /// A file, opened by its path for each read, and its size, when known.
    Disk(Arc<str>, Option<u64>),
    /// A pipe's, read once.
    Memory(Arc<[u8]>),
    /// In another archive: read whole from there, each time.
    Held(Again),
}

impl Raw {
    fn open(&self) -> Result<Box<dyn Read + Send>, String> {
        Ok(match self {
            Self::Disk(path, _) => Box::new(File::open(&**path).map_err(|e| e.to_string())?),
            Self::Memory(bytes) => Box::new(Cursor::new(bytes.clone())),
            Self::Held(again) => Box::new(Cursor::new(again()?)),
        })
    }

    /// From `at` on, for a plain tar, whose files are where its headers say.
    fn open_at(&self, at: u64) -> Result<Box<dyn Read + Send>, String> {
        let error = |e: std::io::Error| e.to_string();
        Ok(match self {
            Self::Disk(path, _) => {
                let mut file = File::open(&**path).map_err(error)?;
                file.seek(SeekFrom::Start(at)).map_err(error)?;
                Box::new(file)
            }
            Self::Memory(bytes) => {
                let mut cursor = Cursor::new(bytes.clone());
                cursor.set_position(at);
                Box::new(cursor)
            }
            Self::Held(again) => {
                let mut cursor = Cursor::new(again()?);
                cursor.set_position(at);
                Box::new(cursor)
            }
        })
    }

    /// How many bytes it has, when that is known without reading them.
    fn len(&self) -> Option<u64> {
        match self {
            Self::Disk(_, len) => *len,
            Self::Memory(bytes) => u64::try_from(bytes.len()).ok(),
            Self::Held(_) => None,
        }
    }
}

/// `raw` unpacked by `codec`, counting the bytes read of `raw` into `read`.
fn stream(raw: &Raw, codec: Codec, read: Option<Arc<AtomicU64>>) -> Result<Box<dyn Read + Send>, String> {
    let opened = raw.open()?;
    let counted: Box<dyn Read + Send> = match read {
        Some(read) => Box::new(Counted { inner: opened, read }),
        None => opened,
    };
    Ok(codec.decode(counted))
}

/// A reader counting what it reads, for the pass through to say how far it has got.
struct Counted {
    inner: Box<dyn Read + Send>,
    read: Arc<AtomicU64>,
}

impl Read for Counted {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.read.fetch_add(u64::try_from(read).unwrap_or(0), Ordering::Relaxed);
        Ok(read)
    }
}

/// What `read` returns, or an error if a decompressor panics: its bug costs the archive or the
/// file, not the viewer.
fn caught<T>(read: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|_| Err("the decompressor failed on it".into()))
}

/// Whether `head` starts with a tar header: `ustar` where POSIX and GNU tar write it, and a
/// checksum that adds up. (A tar from before POSIX has neither the word nor this backend.)
fn is_tar(head: &[u8]) -> bool {
    let Some(block) = head.get(..512).and_then(|b| <&[u8; 512]>::try_from(b).ok()) else { return false };
    block.get(257..262) == Some(b"ustar".as_slice()) && matches!(Header::parse(block), Ok(Some(_)))
}

/// A header's fields that matter here.
#[derive(Debug, PartialEq, Eq)]
struct Header {
    name: Vec<u8>,
    size: u64,
    kind: u8,
}

impl Header {
    /// The header in `block`: `None` for a block of zeros, which ends an archive.
    fn parse(block: &[u8; 512]) -> Result<Option<Self>, String> {
        if block.iter().all(|b| *b == 0) {
            return Ok(None);
        }
        let damaged = || "not a tar header, or a damaged one".to_string();
        // The sum of the header's bytes, its own field counted as spaces; old tars summed them
        // as signed bytes.
        let stored = number(field(block, 148, 8)).ok_or_else(damaged)?;
        let (mut unsigned, mut signed) = (0u64, 0i64);
        for (i, byte) in block.iter().enumerate() {
            let byte = if (148..156).contains(&i) { b' ' } else { *byte };
            unsigned = unsigned.saturating_add(u64::from(byte));
            signed = signed.saturating_add(i64::from(byte as i8));
        }
        if stored != unsigned && i64::try_from(stored).ok() != Some(signed) {
            return Err(damaged());
        }
        let size = number(field(block, 124, 12)).ok_or_else(damaged)?;
        let mut name = until_nul(field(block, 0, 100)).to_vec();
        // POSIX ustar: a prefix of the name, for a long one (GNU tar keeps other fields there).
        let prefix = until_nul(field(block, 345, 155));
        if field(block, 257, 6) == b"ustar\0" && !prefix.is_empty() {
            name = [prefix, b"/", &name].concat();
        }
        Ok(Some(Self { name, size, kind: block.get(156).copied().unwrap_or(0) }))
    }
}

/// The `len` bytes of `block` from `at`; fewer past its end, which no caller asks for.
fn field(block: &[u8; 512], at: usize, len: usize) -> &[u8] {
    block.get(at..at.saturating_add(len)).unwrap_or_default()
}

/// `bytes` up to the first NUL.
fn until_nul(bytes: &[u8]) -> &[u8] {
    bytes.split(|b| *b == 0).next().unwrap_or_default()
}

/// A number field: octal digits, after spaces and before a space or a NUL; or for a large one,
/// GNU's base 256, its first byte's top bit set. `None` for anything else, or a number past
/// `u64`.
fn number(field: &[u8]) -> Option<u64> {
    match field.split_first() {
        Some((first, rest)) if first & 0x80 != 0 => {
            // Negative numbers (the next bit set) are no size.
            if first & 0x40 != 0 {
                return None;
            }
            rest.iter().try_fold(u64::from(first & 0x3f), |n, byte| n.checked_mul(256)?.checked_add(u64::from(*byte)))
        }
        _ => {
            let digits = field.iter().skip_while(|b| **b == b' ').take_while(|b| **b != b' ' && **b != 0);
            digits.into_iter().try_fold(0u64, |n, byte| match byte {
                b'0'..=b'7' => n.checked_mul(8)?.checked_add(u64::from(byte - b'0')),
                _ => None,
            })
        }
    }
}

/// From a pax extended header's records (`30 path=a/rather/long/name.png\n`): the path and the
/// size they give the next entry. A record that isn't one ends them.
fn pax(mut records: &[u8]) -> (Option<Vec<u8>>, Option<u64>) {
    let (mut path, mut size) = (None, None);
    while let Some(space) = records.iter().position(|b| *b == b' ') {
        let length = records.get(..space).and_then(|digits| std::str::from_utf8(digits).ok()).and_then(|d| d.parse::<usize>().ok());
        let Some(record) = length.filter(|n| *n > space).and_then(|n| records.get(space.saturating_add(1)..n)) else { break };
        let Some((key, value)) = record.strip_suffix(b"\n").and_then(|r| r.iter().position(|b| *b == b'=').map(|at| r.split_at(at))) else { break };
        let value = value.get(1..).unwrap_or_default();
        match key {
            b"path" => path = Some(value.to_vec()),
            b"size" => size = std::str::from_utf8(value).ok().and_then(|v| v.parse::<u64>().ok()),
            _ => {}
        }
        records = length.and_then(|n| records.get(n..)).unwrap_or_default();
    }
    (path, size)
}

/// What the pass through an archive found.
#[derive(Default)]
struct Index {
    tree: Tree,
    /// By their index in `tree`.
    files: Vec<Member>,
    /// The names left out, and why, said when the archive's top is listed.
    unread: Vec<(String, String)>,
    /// Whether the pass stopped at a limit, or at damage, before the end.
    cut: bool,
}

/// A file in an archive: where its bytes start, unpacked, how many there are, and the first of
/// them, kept for the first [`MAX_HEADS`] files.
struct Member {
    at: u64,
    size: u64,
    head: Option<Vec<u8>>,
}

/// An archive's state, shared by its directories, the files read from it on render threads, and
/// the pass through it.
struct Shared {
    path: String,
    raw: Raw,
    codec: Codec,
    /// How many bytes `raw` has, for the pass to say how far it has got.
    of: Option<u64>,
    state: Mutex<State>,
    /// Told when `state` changes.
    changed: Condvar,
    /// Passes started, to tell a stopped one's end from the current one's.
    runs: AtomicU64,
    /// The archive unpacked, as far as the last pass wrote it.
    spill: Mutex<Option<Arc<Spill>>>,
    /// The most bytes a pass writes there: [`MAX_SPILL`].
    spill_limit: u64,
    /// The decompressor going forward through the archive, for files past the spill, and where
    /// it is.
    cursor: Mutex<Option<(Box<dyn Read + Send>, u64)>>,
    /// The reader left it: nothing is kept after a read.
    released: AtomicBool,
}

enum State {
    /// Not read through yet, or stopped.
    Idle,
    /// Being read through, by the pass numbered `run`, which has read `read` bytes of the archive
    /// and stops when `stop` is set.
    Reading {
        run: u64,
        read: Arc<AtomicU64>,
        stop: Arc<AtomicBool>,
    },
    Read(Arc<Index>),
    Failed(String),
}

/// An archive unpacked, its first `len` bytes, in a file removed as soon as it was made: nothing
/// names it, so it goes with its last handle, however the viewer ends.
struct Spill {
    file: File,
    len: u64,
}

impl Spill {
    /// A file to unpack an archive into, in the temporary directory, readable by this user only and
    /// already removed; `None` if there is none to be had there.
    fn file() -> Option<File> {
        static MADE: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir();
        for _ in 0..4 {
            let name = format!(".{}-tar-{}-{}", crate::NAME, std::process::id(), MADE.fetch_add(1, Ordering::Relaxed));
            let path = dir.join(name);
            match OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&path) {
                Ok(file) => return std::fs::remove_file(&path).is_ok().then_some(file),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                Err(_) => return None,
            }
        }
        None
    }

    /// The `size` bytes from `at`, if it has them.
    fn read(&self, at: u64, size: u64) -> Option<Result<Vec<u8>, String>> {
        if at.checked_add(size)? > self.len {
            return None;
        }
        let mut bytes = vec![0; usize::try_from(size).ok()?];
        Some(self.file.read_exact_at(&mut bytes, at).map(|()| bytes).map_err(|e| e.to_string()))
    }
}

/// The pass's stream, writing what it reads to a spill as it goes, up to `limit` bytes: past them,
/// or once a write fails (a full disk), it writes no more, and the spill holds what it wrote.
struct Tee {
    input: Box<dyn Read + Send>,
    spill: Option<File>,
    written: u64,
    limit: u64,
}

impl Tee {
    fn finish(self) -> Option<Spill> {
        self.spill.map(|file| Spill { file, len: self.written })
    }
}

impl Read for Tee {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.input.read(buf)?;
        let room = usize::try_from(self.limit.saturating_sub(self.written)).unwrap_or(usize::MAX);
        if let (Some(file), Some(bytes)) = (self.spill.as_mut(), buf.get(..read.min(room))) {
            match file.write_all(bytes) {
                Ok(()) => self.written = self.written.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX)),
                // What it wrote stands; it writes no more.
                Err(_) => self.limit = self.written,
            }
        }
        Ok(read)
    }
}

/// Why a pass through ended early.
enum Halt {
    /// It was stopped.
    Stopped,
    Failed(String),
}

impl From<String> for Halt {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

/// The lock on `mutex`: its value is consistent at every unlock, so a thread that panicked holding
/// it leaves nothing half done.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    fn new(path: String, raw: Raw, codec: Codec) -> Self {
        let of = raw.len();
        Self {
            path,
            raw,
            codec,
            of,
            state: Mutex::new(State::Idle),
            changed: Condvar::new(),
            runs: AtomicU64::new(0),
            spill: Mutex::new(None),
            spill_limit: MAX_SPILL,
            cursor: Mutex::new(None),
            released: AtomicBool::new(false),
        }
    }

    /// Whether it has been read through, starting the pass on a thread of its own if it hasn't.
    fn ready(self: &Arc<Self>) -> Ready {
        self.released.store(false, Ordering::Relaxed);
        let mut state = lock(&self.state);
        let (run, read, stop) = match &*state {
            State::Read(_) | State::Failed(_) => return Ready::Now,
            State::Reading { read, .. } => return Ready::Reading { read: read.load(Ordering::Relaxed), of: self.of },
            State::Idle => (self.runs.fetch_add(1, Ordering::Relaxed), Arc::new(AtomicU64::new(0)), Arc::new(AtomicBool::new(false))),
        };
        *state = State::Reading { run, read: read.clone(), stop: stop.clone() };
        drop(state);
        let shared = self.clone();
        let (thread_read, thread_stop) = (read.clone(), stop.clone());
        let spawned = std::thread::Builder::new().name(format!("{}-tar", crate::NAME)).spawn(move || {
            let result = shared.scan(&thread_read, &thread_stop);
            shared.finish(run, result);
        });
        match spawned {
            Ok(_) => Ready::Reading { read: read.load(Ordering::Relaxed), of: self.of },
            // No thread: it's read through when listed, on the caller's.
            Err(_) => {
                self.finish(run, Err(Halt::Stopped));
                Ready::Now
            }
        }
    }

    /// What the pass through found, waiting for one under way, or making one on this thread.
    fn index(self: &Arc<Self>) -> Result<Arc<Index>, String> {
        self.released.store(false, Ordering::Relaxed);
        let mut state = lock(&self.state);
        loop {
            match &*state {
                State::Read(index) => return Ok(index.clone()),
                State::Failed(why) => return Err(why.clone()),
                State::Reading { .. } => state = self.changed.wait(state).unwrap_or_else(PoisonError::into_inner),
                State::Idle => {
                    let (run, read, stop) =
                        (self.runs.fetch_add(1, Ordering::Relaxed), Arc::new(AtomicU64::new(0)), Arc::new(AtomicBool::new(false)));
                    *state = State::Reading { run, read: read.clone(), stop: stop.clone() };
                    drop(state);
                    let result = self.scan(&read, &stop);
                    self.finish(run, result);
                    state = lock(&self.state);
                }
            }
        }
    }

    /// The pass numbered `run` ended with `result`: kept, with the spill it wrote, unless it was
    /// stopped meanwhile.
    fn finish(&self, run: u64, result: Result<(Index, Option<Spill>), Halt>) {
        let mut state = lock(&self.state);
        if !matches!(&*state, State::Reading { run: r, .. } if *r == run) {
            return;
        }
        *state = match result {
            Ok((index, spill)) => {
                *lock(&self.spill) = spill.map(Arc::new);
                State::Read(Arc::new(index))
            }
            Err(Halt::Stopped) => State::Idle,
            Err(Halt::Failed(why)) => State::Failed(why),
        };
        self.changed.notify_all();
    }

    /// Stop a pass under way.
    fn stop(&self) {
        let mut state = lock(&self.state);
        if let State::Reading { stop, .. } = &*state {
            stop.store(true, Ordering::Relaxed);
            *state = State::Idle;
            self.changed.notify_all();
        }
    }

    /// Let go of all that reading it keeps, the reader having left it: the spill, the
    /// decompressor, and what the pass found, which a reader back in it reads again.
    fn release(&self) {
        self.released.store(true, Ordering::Relaxed);
        let mut state = lock(&self.state);
        match &*state {
            State::Reading { stop, .. } => stop.store(true, Ordering::Relaxed),
            State::Read(_) => {}
            State::Idle | State::Failed(_) => return self.forget(),
        }
        *state = State::Idle;
        self.changed.notify_all();
        drop(state);
        self.forget();
    }

    /// Let go of the spill and the decompressor.
    fn forget(&self) {
        *lock(&self.spill) = None;
        *lock(&self.cursor) = None;
    }

    /// Whether its files are read where they are, rather than unpacked: a plain tar's, on disk
    /// or in memory.
    fn direct(&self) -> bool {
        self.codec == Codec::None && !matches!(self.raw, Raw::Held(_))
    }

    /// Read the archive through, unpacked, for its files: each one's name, where it starts and
    /// its size, and the first bytes of the first ones; and unless they are read where they are,
    /// the archive unpacked, in a spill. `read` counts the archive's bytes read; `stop` ends the
    /// pass early.
    fn scan(&self, read: &Arc<AtomicU64>, stop: &AtomicBool) -> Result<(Index, Option<Spill>), Halt> {
        let path = &self.path;
        let stream = caught(|| stream(&self.raw, self.codec, Some(read.clone()))).map_err(|e| format!("{path}: {e}"))?;
        let spill = if self.direct() { None } else { Spill::file() };
        let mut input = Tee { input: stream, spill, written: 0, limit: self.spill_limit };
        let mut index = Index::default();
        // Where the stream is, unpacked; what the last long name or pax header gave the next
        // entry; entries read so far.
        let (mut at, mut long_name, mut pax_size, mut entries) = (0u64, None::<Vec<u8>>, None::<u64>, 0usize);
        loop {
            if stop.load(Ordering::Relaxed) {
                return Err(Halt::Stopped);
            }
            if entries >= MAX_ENTRIES || at >= MAX_SCANNED {
                index.cut = true;
                break;
            }
            let mut block = [0u8; 512];
            match caught(|| fill(&mut input, &mut block)) {
                Ok(true) => {}
                // The end, without the blocks of zeros that should mark it.
                Ok(false) if entries > 0 => break,
                Ok(false) => return Err(format!("{path}: not a tar archive").into()),
                Err(e) => return Err(format!("{path}: {e}").into()),
            }
            at = at.saturating_add(BLOCK);
            let header = match Header::parse(&block) {
                Ok(Some(header)) => header,
                Ok(None) => break,
                Err(e) if entries == 0 => return Err(format!("{path}: {e}").into()),
                // Damage past the first file: what came before is listed.
                Err(_) => {
                    index.cut = true;
                    break;
                }
            };
            entries = entries.saturating_add(1);
            let size = pax_size.take().unwrap_or(header.size);
            let padded = size.div_ceil(BLOCK).saturating_mul(BLOCK);
            let mut used = 0u64;
            match header.kind {
                // A long name (GNU), or extended headers (POSIX): for the next entry.
                b'L' | b'x' => {
                    let bytes = caught(|| capped(&mut input, size, MAX_NAME)).map_err(|e| format!("{path}: {e}"))?;
                    used = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                    if header.kind == b'L' {
                        long_name = Some(until_nul(&bytes).to_vec());
                    } else {
                        let (name, size) = pax(&bytes);
                        long_name = name.or(long_name);
                        pax_size = size;
                    }
                }
                b'0' | b'\0' | b'7' | b'5' => {
                    let name = long_name.take().unwrap_or(header.name);
                    let directory = header.kind == b'5';
                    match String::from_utf8(name) {
                        Ok(name) if directory => {
                            if let Err(why) = index.tree.add(&name, true, 0) {
                                index.unread.push((name.clone(), format!("{path}/{name}: {why}")));
                            }
                        }
                        Ok(name) => {
                            let file = index.files.len();
                            let mut member = Member { at, size, head: None };
                            if let Err(why) = index.tree.add(&name, false, file) {
                                index.unread.push((name.clone(), format!("{path}/{name}: {why}")));
                            } else if file < MAX_HEADS {
                                let head = caught(|| capped(&mut input, size.min(HEAD), HEAD)).map_err(|e| format!("{path}/{name}: {e}"))?;
                                used = u64::try_from(head.len()).unwrap_or(u64::MAX);
                                member.head = Some(head);
                            }
                            index.files.push(member);
                        }
                        Err(bad) => {
                            let shown = String::from_utf8_lossy(bad.as_bytes()).into_owned();
                            index.unread.push((shown.clone(), format!("{path}/{shown}: its name isn't UTF-8")));
                        }
                    }
                }
                // Links, devices, pax's global headers, GNU's sparse files and volume labels:
                // nothing to show, and nothing the next entry takes.
                _ => {
                    long_name = None;
                    pax_size = None;
                }
            }
            caught(|| skip(&mut input, padded.saturating_sub(used), stop)).map_err(|e| format!("{path}: {e}"))?;
            if stop.load(Ordering::Relaxed) {
                return Err(Halt::Stopped);
            }
            at = at.saturating_add(padded);
        }
        Ok((index, input.finish()))
    }

    /// The file at `index` (by its place in `files`), whole: where it is in a plain tar; from the
    /// spill; or unpacked from the archive, by the decompressor going forward through it, started
    /// over when it is past the file.
    fn read(&self, files: &[Member], index: usize) -> Result<Vec<u8>, String> {
        let member = files.get(index).ok_or("not in the archive")?;
        if member.size > MAX_BYTES {
            return Err(format!("it unpacks to more than {} MiB, more than {} reads from an archive", MAX_BYTES >> 20, crate::NAME));
        }
        let spill = lock(&self.spill).clone();
        if let Some(read) = spill.as_ref().and_then(|spill| spill.read(member.at, member.size)) {
            return read;
        }
        let mut cursor = lock(&self.cursor);
        let bytes = caught(|| {
            if self.direct() {
                return capped(&mut self.raw.open_at(member.at)?, member.size, member.size);
            }
            let (mut input, mut at) = match cursor.take() {
                Some((input, at)) if at <= member.at => (input, at),
                _ => (stream(&self.raw, self.codec, None)?, 0),
            };
            skip(&mut input, member.at.saturating_sub(at), &AtomicBool::new(false))?;
            let bytes = capped(&mut input, member.size, member.size)?;
            at = member.at.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
            if !self.released.load(Ordering::Relaxed) {
                *cursor = Some((input, at));
            }
            Ok(bytes)
        })?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) < member.size {
            return Err("the archive ends inside it".into());
        }
        Ok(bytes)
    }
}

/// Fill `block` from `input`: `false` at the end, before any of it.
fn fill(input: &mut dyn Read, block: &mut [u8; 512]) -> Result<bool, String> {
    let mut filled = 0;
    while let Some(rest) = block.get_mut(filled..).filter(|rest| !rest.is_empty()) {
        match input.read(rest) {
            Ok(0) if filled == 0 => return Ok(false),
            Ok(0) => return Err("the archive ends inside a header".into()),
            Ok(n) => filled = filled.saturating_add(n),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(true)
}

/// The next `size` bytes of `input`, or as many as there are, refused past `max`.
fn capped(input: &mut dyn Read, size: u64, max: u64) -> Result<Vec<u8>, String> {
    if size > max {
        return Err(format!("{size} bytes, more than the {max} {} reads", crate::NAME));
    }
    let mut bytes = Vec::new();
    input.take(size).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// Read past the next `count` bytes of `input`, stopping early when `stop` is set.
fn skip(input: &mut dyn Read, count: u64, stop: &AtomicBool) -> Result<(), String> {
    let mut left = count;
    let mut buffer = vec![0u8; 64 << 10];
    while left > 0 && !stop.load(Ordering::Relaxed) {
        let want = usize::try_from(left.min(64 << 10)).unwrap_or(64 << 10);
        let Some(chunk) = buffer.get_mut(..want) else { break };
        match input.read(chunk) {
            Ok(0) => break,
            Ok(n) => left = left.saturating_sub(u64::try_from(n).unwrap_or(u64::MAX)),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

/// An archive as its directories hold it: when the last of them goes, a pass through it under way
/// stops, since nothing can list what it finds. What reading its files keeps (the spill, the
/// decompressor) goes with the last of them to be read, or when the reader leaves the archive.
struct Handle(Arc<Shared>);

impl Drop for Handle {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// A directory in an archive, its top or one in it, as a container.
struct Directory {
    handle: Rc<Handle>,
    /// Its names from the archive's top: none for the top.
    at: Vec<String>,
    /// The archive's path, and its names: `photos.tar.gz/2024`.
    path: String,
}

impl Container for Directory {
    fn path(&self) -> &str {
        &self.path
    }

    fn list(&self, recurse: bool) -> Result<Listing, String> {
        let index = self.handle.0.index()?;
        let inner =
            |at, name: &str| -> Rc<dyn Container> { Rc::new(Directory { handle: self.handle.clone(), at, path: format!("{}/{name}", self.path) }) };
        let mut listing = index.tree.list(&self.at, recurse, inner).ok_or_else(|| format!("{}: not in the archive", self.path))?;
        if self.at.is_empty() {
            listing.unread.clone_from(&index.unread);
            listing.cut |= index.cut;
        }
        // Its files are quickest opened as they come in the archive: the decompressor only goes
        // forward.
        let start = |name: &str| index.tree.file(&self.at, name).and_then(|file| index.files.get(file)).map_or(0, |m| m.at);
        let mut quickest: Vec<(u64, usize)> = listing.entries.iter().enumerate().map(|(i, e)| (start(&e.name), i)).collect();
        quickest.sort_unstable();
        listing.quickest = quickest.into_iter().map(|(_, i)| i).collect();
        Ok(listing)
    }

    fn open(&self, name: &str) -> Result<Input, String> {
        let path = format!("{}/{name}", self.path);
        let index = self.handle.0.index()?;
        let file = index.tree.file(&self.at, name).ok_or_else(|| format!("{path}: not in the archive"))?;
        let head = match index.files.get(file).and_then(|m| m.head.clone()) {
            Some(head) => head,
            None => {
                let mut head = self.handle.0.read(&index.files, file).map_err(|e| format!("{path}: {e}"))?;
                head.truncate(usize::try_from(HEAD).unwrap_or(usize::MAX));
                head
            }
        };
        let shared = self.handle.0.clone();
        let again: Again = Arc::new(move || shared.read(&index.files, file));
        Ok(Input::held(path, head, again))
    }

    /// The archive itself is a parcel; a directory in it, a folder as any directory is.
    fn badge(&self) -> Badge {
        if self.at.is_empty() { Badge::Parcel } else { Badge::None }
    }

    fn ready(&self) -> Ready {
        self.handle.0.ready()
    }

    /// Left, the archive's top lets go of all that reading it keeps; one of its directories, back
    /// to the top or further in, is still in the archive.
    fn release(&self) {
        if self.at.is_empty() {
            self.handle.0.release();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::time::{Duration, Instant};

    use proptest::prelude::*;

    use super::*;
    use crate::source::tree::MAX_DEPTH;

    /// A header for `name`, of `size` bytes, of `kind`, as POSIX tar writes one.
    fn header(name: &[u8], size: u64, kind: u8) -> [u8; 512] {
        let mut block = [0u8; 512];
        let len = name.len().min(100);
        block[..len].copy_from_slice(&name[..len]);
        block[100..108].copy_from_slice(b"0000644\0");
        block[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
        block[156] = kind;
        block[257..263].copy_from_slice(b"ustar\0");
        block[263..265].copy_from_slice(b"00");
        sum(&mut block);
        block
    }

    /// Set `block`'s checksum.
    fn sum(block: &mut [u8; 512]) {
        block[148..156].fill(b' ');
        let total: u32 = block.iter().map(|b| u32::from(*b)).sum();
        block[148..156].copy_from_slice(format!("{total:06o}\0 ").as_bytes());
    }

    /// `header`, then `data`, padded to a block.
    fn push(out: &mut Vec<u8>, header: [u8; 512], data: &[u8]) {
        out.extend(header);
        out.extend(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }

    /// A tar of `entries`: a name ending in `/` is a directory, and one longer than 100 bytes comes
    /// after a long name entry, as GNU tar writes it.
    fn tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, data) in entries {
            if name.len() > 100 {
                let long = [name.as_bytes(), b"\0"].concat();
                push(&mut out, header(b"././@LongLink", long.len() as u64, b'L'), &long);
            }
            let kind = if name.ends_with('/') { b'5' } else { b'0' };
            push(&mut out, header(name.as_bytes(), data.len() as u64, kind), data);
        }
        out.extend([0u8; 1024]);
        out
    }

    fn compress(bytes: &[u8], codec: Codec) -> Vec<u8> {
        match codec {
            Codec::None => bytes.to_vec(),
            Codec::Gzip => {
                let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                encoder.write_all(bytes).unwrap();
                encoder.finish().unwrap()
            }
            Codec::Bzip2 => {
                let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
                encoder.write_all(bytes).unwrap();
                encoder.finish().unwrap()
            }
            Codec::Xz => {
                let mut encoder = lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(1)).unwrap();
                encoder.write_all(bytes).unwrap();
                encoder.finish().unwrap()
            }
        }
    }

    /// `bytes` in a file of the test's own, called `name`.
    fn on_disk(test: &str, name: &str, bytes: &[u8]) -> String {
        let dir = std::env::temp_dir().join(format!("clview-tar-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_owned()
    }

    /// The archive at `path`, opened as the viewer opens it.
    fn opened(path: &str) -> Result<Rc<dyn Container>, String> {
        let input = Input::open(path)?;
        assert!(Tar.opens(path, &input.head));
        match Tar.open(input, &Parsed::default())? {
            Opened::Files(container) => Ok(container),
            Opened::Pages(_) => panic!("an archive is a container"),
        }
    }

    /// The names listed, a directory's with a `/` after it.
    fn names(listing: &Listing) -> Vec<String> {
        listing.entries.iter().map(|e| if e.container.is_some() { format!("{}/", e.name) } else { e.name.clone() }).collect()
    }

    fn inner(listing: &Listing, name: &str) -> Rc<dyn Container> {
        listing.entries.iter().find(|e| e.name == name).and_then(|e| e.container.clone()).unwrap()
    }

    fn read(container: &dyn Container, name: &str) -> Vec<u8> {
        container.open(name).unwrap().bytes().unwrap()
    }

    /// The formats `--help` lists are those a file opens in: a plain tar, and each compression,
    /// which a file in it is known to have.
    #[test]
    fn the_formats_listed_are_those_a_file_opens_in() {
        let plain = tar(&[("a.png", b"a")]);
        let listed = Tar.formats();
        assert_eq!(listed.iter().map(|f| f.name).collect::<Vec<_>>(), ["tar", "gzip", "bzip2", "xz"]);
        for codec in [Codec::None, Codec::Gzip, Codec::Bzip2, Codec::Xz] {
            let file = compress(&plain, codec);
            let head = file.get(..HEAD as usize).unwrap_or(&file);
            assert!(Tar.opens("x", head) && Codec::of(head).unwrap_or(Codec::None) == codec, "{codec:?}");
            assert!(listed.contains(&codec.format()), "{codec:?} opens, but isn't listed");
        }
    }

    /// Plain, gzip, bzip2 or xz, an archive lists as a directory on disk does, its directories
    /// containers, its hidden files passed over, and its files are read from it; quickest opened
    /// as they come in it.
    #[test]
    fn an_archive_lists_as_a_directory_does_however_it_is_compressed() {
        let deep = "d".repeat(120);
        let long = format!("{deep}/deep.png");
        let plain = tar(&[
            ("b10.png", b"ten"),
            ("b9.png", b"nine"),
            (".hidden.png", b"no"),
            ("sub/", b""),
            ("sub/c.png", b"c"),
            (&long, b"long"),
            ("z.txt", &[7; 3000]),
        ]);
        for codec in [Codec::None, Codec::Gzip, Codec::Bzip2, Codec::Xz] {
            let path = on_disk("list", &format!("t-{codec:?}.tar"), &compress(&plain, codec));
            let top = opened(&path).unwrap();
            assert_eq!((top.path(), top.badge()), (path.as_str(), Badge::Parcel));
            let listing = top.list(false).unwrap();
            assert_eq!(names(&listing), ["b9.png", "b10.png", &format!("{deep}/"), "sub/", "z.txt"], "{codec:?}");
            assert!(listing.unread.is_empty() && !listing.cut, "{listing:?}");
            // Directories first, then the files as they come: b10, b9, z.
            assert_eq!(listing.quickest, [2, 3, 1, 0, 4]);
            assert_eq!((read(&*top, "b10.png"), read(&*top, "z.txt")), (b"ten".to_vec(), vec![7; 3000]));
            let sub = inner(&listing, "sub");
            assert_eq!((sub.path(), sub.badge()), (format!("{path}/sub").as_str(), Badge::None));
            assert_eq!((names(&sub.list(false).unwrap()), read(&*sub, "c.png")), (vec!["c.png".to_string()], b"c".to_vec()));
            assert_eq!(read(&*inner(&listing, &deep), "deep.png"), b"long");
            let all = names(&top.list(true).unwrap());
            assert_eq!(all, ["b9.png", "b10.png", &format!("{deep}/deep.png"), "sub/c.png", "z.txt"]);
            assert!(top.open("nothing.png").is_err_and(|e| e.ends_with("not in the archive")));
        }
    }

    #[test]
    fn headers_are_read_as_posix_and_gnu_tar_write_them() {
        let block = header(b"a.png", 1234, b'0');
        assert_eq!(Header::parse(&block), Ok(Some(Header { name: b"a.png".to_vec(), size: 1234, kind: b'0' })));
        // A long name's start in POSIX's prefix.
        let mut prefixed = header(b"b.png", 0, b'0');
        prefixed[345..349].copy_from_slice(b"some");
        sum(&mut prefixed);
        assert_eq!(Header::parse(&prefixed).unwrap().unwrap().name, b"some/b.png");
        // A size too large for its octal digits, in GNU's base 256.
        let mut large = header(b"c", 0, b'0');
        large[124..136].copy_from_slice(&[0x80, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]);
        sum(&mut large);
        assert_eq!(Header::parse(&large).unwrap().unwrap().size, 1 << 32);
        // A checksum summed as signed bytes, as old tars did.
        let mut signed = header(b"caf\xe9", 0, b'0');
        signed[148..156].fill(b' ');
        let total: i64 = signed.iter().map(|b| i64::from(*b as i8)).sum();
        signed[148..156].copy_from_slice(format!("{total:06o}\0 ").as_bytes());
        assert!(Header::parse(&signed).is_ok_and(|h| h.is_some()));
        // One that doesn't add up; a block of zeros, the end.
        let mut damaged = header(b"d", 0, b'0');
        damaged[0] = b'e';
        assert!(Header::parse(&damaged).is_err());
        assert_eq!(Header::parse(&[0; 512]), Ok(None));
        assert_eq!((number(b"  777 \0"), number(b"\0\0\0"), number(b"8"), number(b"777777777777777777777777")), (Some(511), Some(0), None, None));
        assert_eq!((number(&[0xc0, 0, 1]), number(&[0x80, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff])), (None, None));
        // Pax records: a path and a size; one that isn't a record ends them.
        assert_eq!(pax(b"18 path=x/y/z.png\n13 size=4096\n"), (Some(b"x/y/z.png".to_vec()), Some(4096)));
        assert_eq!(pax(b"99 path=x\n"), (None, None));
        assert_eq!(pax(b"13 size=4096\n2 x"), (None, Some(4096)));
        assert_eq!(pax(b""), (None, None));
    }

    /// A pax record for `key` and `value`, its length counting itself.
    fn record(key: &str, value: &str) -> Vec<u8> {
        let body = format!(" {key}={value}\n");
        let mut length = body.len() + 1;
        while format!("{length}{body}").len() != length {
            length += 1;
        }
        format!("{length}{body}").into_bytes()
    }

    /// Names come from pax headers and long name entries; links, and names that aren't UTF-8,
    /// show nothing; damage past the first file ends the listing, keeping what came before.
    #[test]
    fn names_come_from_pax_headers_and_what_shows_nothing_is_passed_over() {
        let mut bytes = Vec::new();
        let records = record("path", "pax/a-rather-long-name.png");
        push(&mut bytes, header(b"PaxHeader", records.len() as u64, b'x'), &records);
        push(&mut bytes, header(b"short", 3, b'0'), b"pax");
        push(&mut bytes, header(b"link.png", 0, b'2'), b"");
        push(&mut bytes, header(b"caf\xe9.png", 1, b'0'), b"x");
        push(&mut bytes, header(b"after.png", 5, b'0'), b"after");
        bytes.extend([b'?'; 512]);
        push(&mut bytes, header(b"lost.png", 4, b'0'), b"lost");
        let path = on_disk("names", "t.tar", &bytes);
        let top = opened(&path).unwrap();
        let listing = top.list(false).unwrap();
        assert_eq!(names(&listing), ["after.png", "pax/"]);
        assert_eq!(read(&*inner(&listing, "pax"), "a-rather-long-name.png"), b"pax");
        assert_eq!(listing.unread, [("caf\u{fffd}.png".to_string(), format!("{path}/caf\u{fffd}.png: its name isn't UTF-8"))]);
        assert!(listing.cut);
        // Not a tar at all.
        let path = on_disk("names", "not.tar", &[b'?'; 2048]);
        let input = Input::open(&path).unwrap();
        assert!(!Tar.opens(&path, &input.head));
    }

    #[test]
    fn a_name_too_long_is_refused_and_one_too_deep_left_out() {
        let deep = format!("{}x.png", "d/".repeat(MAX_DEPTH));
        let path = on_disk("limits", "deep.tar", &tar(&[(&deep, b"x"), ("ok.png", b"ok")]));
        let listing = opened(&path).unwrap().list(false).unwrap();
        assert_eq!(names(&listing), ["ok.png"]);
        assert_eq!(listing.unread, [(deep.clone(), format!("{path}/{deep}: more than {MAX_DEPTH} directories down"))]);
        let mut bytes = Vec::new();
        let name = vec![b'n'; usize::try_from(MAX_NAME).unwrap() + 1];
        push(&mut bytes, header(b"././@LongLink", name.len() as u64, b'L'), &name);
        push(&mut bytes, header(b"n", 1, b'0'), b"n");
        let path = on_disk("limits", "long.tar", &bytes);
        assert!(opened(&path).unwrap().list(false).is_err_and(|e| e.contains("more than the 65536")), "a long name past the limit");
    }

    /// A file is read from the spill the pass wrote, in any order; past what it holds, by the
    /// decompressor going forward, which starts over for one before where it is. Released,
    /// nothing is kept: neither the spill nor the decompressor, nor what the pass found.
    #[test]
    fn files_are_read_from_the_spill_or_going_forward_and_let_go_of() {
        let files: Vec<(String, Vec<u8>)> = (0..6u8).map(|i| (format!("f{i}.bin"), vec![i; 1000])).collect();
        let entries: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        let packed = compress(&tar(&entries), Codec::Gzip);
        let kept = |shared: &Shared| (lock(&shared.spill).as_ref().map(|s| s.len), lock(&shared.cursor).as_ref().map(|(_, at)| *at));
        // All of it spilled: read in any order, no decompressor.
        let shared = Arc::new(Shared::new("t.tar.gz".into(), Raw::Memory(packed.clone().into()), Codec::Gzip));
        let index = shared.index().unwrap();
        for i in [5, 0, 3] {
            assert_eq!(shared.read(&index.files, i).unwrap(), [i as u8; 1000]);
        }
        // Up to the first block of zeros, where the pass stopped reading.
        assert_eq!(kept(&shared), (Some(6 * 1536 + 512), None));
        // The first three spilled; the others by the decompressor.
        let mut shared = Shared::new("t.tar.gz".into(), Raw::Memory(packed.into()), Codec::Gzip);
        shared.spill_limit = 3 * 1536;
        let shared = Arc::new(shared);
        let index = shared.index().unwrap();
        let end = |i: usize| index.files[i].at + 1000;
        assert_eq!(shared.read(&index.files, 1).unwrap(), [1; 1000]);
        assert_eq!(kept(&shared), (Some(3 * 1536), None));
        assert_eq!(shared.read(&index.files, 5).unwrap(), [5; 1000]);
        assert_eq!(kept(&shared).1, Some(end(5)));
        // Before the decompressor: it starts over.
        assert_eq!(shared.read(&index.files, 3).unwrap(), [3; 1000]);
        assert_eq!(kept(&shared).1, Some(end(3)));
        shared.release();
        assert_eq!(kept(&shared), (None, None));
        assert!(matches!(&*lock(&shared.state), State::Idle));
        // Read meanwhile, by a render under way: nothing kept.
        assert_eq!(shared.read(&index.files, 4).unwrap(), [4; 1000]);
        assert_eq!(kept(&shared), (None, None));
        // Back in it, it is read through again.
        let index = shared.index().unwrap();
        assert_eq!((index.files.len(), kept(&shared).0), (6, Some(3 * 1536)));
    }

    /// The spill is a file nothing names: there is no file of it in the temporary directory.
    #[test]
    fn the_spill_has_no_name() {
        let file = Spill::file().unwrap();
        let prefix = format!(".{}-tar-{}-", crate::NAME, std::process::id());
        let named =
            std::fs::read_dir(std::env::temp_dir()).unwrap().filter_map(Result::ok).any(|e| e.file_name().to_string_lossy().starts_with(&prefix));
        assert!(!named);
        file.write_all_at(b"still there", 0).unwrap();
        let mut back = [0; 11];
        file.read_exact_at(&mut back, 0).unwrap();
        assert_eq!(&back, b"still there");
    }

    /// The pass through runs on a thread of its own, saying how far it has got, and stops when
    /// asked; asked again, it starts over. Listed meanwhile, it is waited for.
    #[test]
    fn a_pass_runs_on_its_own_thread_says_how_far_it_has_got_and_stops() {
        let zeros = vec![0; 64 << 20];
        let big = compress(&tar(&[("zeros.bin", &zeros), ("last.png", b"last")]), Codec::Gzip);
        let path = on_disk("pass", "big.tar.gz", &big);
        let top = opened(&path).unwrap();
        assert!(matches!(top.ready(), Ready::Reading { of: Some(of), .. } if of == big.len() as u64));
        top.release();
        let deadline = Instant::now() + Duration::from_secs(60);
        while top.ready() != Ready::Now {
            assert!(Instant::now() < deadline, "never read through");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(names(&top.list(false).unwrap()), ["last.png", "zeros.bin"]);
        // Listed while it reads: waited for.
        let top = opened(&path).unwrap();
        assert!(matches!(top.ready(), Ready::Reading { .. }));
        assert_eq!(names(&top.list(false).unwrap()), ["last.png", "zeros.bin"]);
    }

    /// An archive of `gib` GiB of zeros, then a file: small, and a while to read through. Gzip
    /// streams one after another read as one, so each GiB is the same stream again.
    fn slow(gib: u64) -> Vec<u8> {
        let zeros = compress(&vec![0; 1 << 30], Codec::Gzip);
        let mut bytes = compress(&header(b"zeros.bin", gib << 30, b'0'), Codec::Gzip);
        for _ in 0..gib {
            bytes.extend(&zeros);
        }
        bytes.extend(compress(&tar(&[("last.png", b"last")]), Codec::Gzip));
        bytes
    }

    /// Stopped, a pass reads no further: its thread doesn't carry on unseen.
    #[test]
    fn a_pass_stopped_reads_no_further() {
        let bytes = slow(4);
        let len = bytes.len() as u64;
        let shared = Arc::new(Shared::new("slow.tar.gz".into(), Raw::Memory(bytes.into()), Codec::Gzip));
        assert!(matches!(shared.ready(), Ready::Reading { .. }));
        let read = match &*lock(&shared.state) {
            State::Reading { read, .. } => read.clone(),
            _ => panic!("not reading"),
        };
        std::thread::sleep(Duration::from_millis(100));
        shared.stop();
        std::thread::sleep(Duration::from_millis(100));
        let stopped = read.load(Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(read.load(Ordering::Relaxed), stopped, "still reading");
        assert!(stopped < len, "read through before it was stopped");
    }

    /// What reading an archive keeps goes with the last thing that could read it: when its
    /// directories are gone, a file of it still to be read keeps it, and once that goes too,
    /// nothing is left. One of its directories left, the reader is still in the archive: nothing
    /// goes. A pass under way stops when the directories go.
    #[test]
    fn an_archive_gone_keeps_nothing() {
        let files: Vec<(String, Vec<u8>)> = (0..3u8).map(|i| (format!("f{i}.bin"), vec![i; 1000])).collect();
        let entries: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        let shared = Arc::new(Shared::new("t.tar.gz".into(), Raw::Memory(compress(&tar(&entries), Codec::Gzip).into()), Codec::Gzip));
        let weak = Arc::downgrade(&shared);
        let top = Directory { handle: Rc::new(Handle(shared)), at: Vec::new(), path: "t.tar.gz".into() };
        let listing = top.list(false).unwrap();
        let spilled = || weak.upgrade().map(|shared| lock(&shared.spill).is_some());
        let inner = Directory { handle: top.handle.clone(), at: vec!["sub".into()], path: "t.tar.gz/sub".into() };
        inner.release();
        assert_eq!(spilled(), Some(true), "left a directory in it");
        let input = top.open(&listing.entries[2].name).unwrap();
        drop((listing, inner, top));
        assert_eq!(spilled(), Some(true), "a file still to be read");
        assert_eq!(input.bytes().unwrap(), [2; 1000]);
        assert!(weak.upgrade().is_none(), "kept after all that could read it");
        let shared = Arc::new(Shared::new("slow.tar.gz".into(), Raw::Memory(slow(4).into()), Codec::Gzip));
        let top = Directory { handle: Rc::new(Handle(shared.clone())), at: Vec::new(), path: "slow.tar.gz".into() };
        assert!(matches!(top.ready(), Ready::Reading { .. }));
        drop(top);
        assert!(matches!(&*lock(&shared.state), State::Idle));
    }

    /// A file compressed alone is the file it unpacks to, as the backend that opens that shows it;
    /// one that unpacks to what nothing shows is the error, as is one past the limit.
    #[cfg(feature = "image")]
    #[test]
    fn a_file_compressed_alone_is_what_it_unpacks_to() {
        let mut png = Cursor::new(Vec::new());
        ::image::RgbaImage::new(30, 20).write_to(&mut png, ::image::ImageFormat::Png).unwrap();
        for codec in [Codec::Gzip, Codec::Bzip2, Codec::Xz] {
            let path = on_disk("alone", &format!("a-{codec:?}.png.gz"), &compress(png.get_ref(), codec));
            let input = Input::open(&path).unwrap();
            let Ok(Opened::Pages(source)) = Tar.open(input, &Parsed::default()) else { panic!("{codec:?}: not pages") };
            assert_eq!(source.size(0), Some((30.0, 20.0)), "{codec:?}");
        }
        let path = on_disk("alone", "notes.txt.gz", &compress(b"plain text", Codec::Gzip));
        let input = Input::open(&path).unwrap();
        assert!(Tar.open(input, &Parsed::default()).is_err_and(|e| e == format!("{path}: it unpacks to a file that clv doesn't show")));
        let raw = Raw::Memory(compress(&[1; 2000], Codec::Xz).into());
        assert!(unpack(&raw, Codec::Xz, 1999).is_err_and(|e| e.contains("unpacks to more than")));
        assert_eq!(unpack(&raw, Codec::Xz, 2000).unwrap(), [1; 2000]);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Whatever an archive's bytes are, plain or compressed, it opens or is an error, and so
        /// does each file it lists: nothing panics.
        #[test]
        fn any_bytes_open_or_are_an_error(flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 0..16), tail in prop::collection::vec(any::<u8>(), 0..64), gzip in any::<bool>()) {
            let plain = tar(&[("a/b.png", b"bytes of a picture"), ("c.pdf", &[1; 700]), ("e/", b"")]);
            let mut bytes = if gzip { compress(&plain, Codec::Gzip) } else { plain };
            for (at, byte) in flips {
                let at = at.index(bytes.len());
                bytes[at] ^= byte;
            }
            bytes.extend(tail);
            let path = on_disk("any", "t.tar", &bytes);
            let input = Input::open(&path).unwrap();
            if let Ok(Opened::Files(top)) = Tar.open(input, &Parsed::default()) {
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
