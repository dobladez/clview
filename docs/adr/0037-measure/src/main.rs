//! How far into a picture's file the `image` crate reads to tell its size and its orientation,
//! as clv's image backend asks them of a header (`src/source/image.rs`'s `header`): what a
//! document's picture held in an archive costs to measure. For ADR-0037.
//!
//! `measure <files and directories>` reads every picture under them, by its signature, and
//! prints, for each format, how many there are, how many are measured within 64 KiB, and the
//! furthest any was read to; then each picture read past 64 KiB, with how far. With `--all`, every
//! picture. A read from the file's end counts as the whole file.
//!
//! A JPEG is another matter: the `image` crate reads one whole before it looks at it, then parses
//! its segments in order up to its first scan. So a JPEG's line says how many are read whole (all
//! of them, but for the small ones), and a second line, `Jpeg headers`, where their first scan
//! starts, found here by walking the segments: what the decoder needs of the file.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use image::{ImageDecoder, ImageFormat, ImageReader};

/// What clv reads of a held picture before reading it whole.
const WITHIN: u64 = 64 << 10;

/// A file, noting the furthest byte read from it.
struct Watched<'a> {
    file: BufReader<File>,
    at: u64,
    furthest: &'a Cell<u64>,
}

impl Watched<'_> {
    fn reached(&self, to: u64) {
        self.furthest.set(self.furthest.get().max(to));
    }
}

impl Read for Watched<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.file.read(buf)?;
        self.at += read as u64;
        self.reached(self.at);
        Ok(read)
    }
}

impl BufRead for Watched<'_> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.file.fill_buf()
    }

    fn consume(&mut self, amount: usize) {
        self.file.consume(amount);
        self.at += amount as u64;
        self.reached(self.at);
    }
}

impl Seek for Watched<'_> {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        if matches!(to, SeekFrom::End(_)) {
            self.reached(u64::MAX);
        }
        self.at = self.file.seek(to)?;
        Ok(self.at)
    }
}

/// The format of the picture at `path`, by its first bytes, and how far it is read to for its
/// size and orientation; `None` for what is no picture the `image` crate knows.
fn measured(path: &Path) -> Option<(ImageFormat, u64, u64)> {
    let mut head = [0u8; 1024];
    let length = std::fs::metadata(path).ok()?.len();
    let read = File::open(path).ok()?.read(&mut head).ok()?;
    let format = image::guess_format(&head[..read]).ok()?;
    let furthest = Cell::new(0);
    let watched = Watched { file: BufReader::new(File::open(path).ok()?), at: 0, furthest: &furthest };
    let mut decoder = ImageReader::with_format(watched, format).into_decoder().ok()?;
    let _ = decoder.dimensions();
    let _ = decoder.orientation();
    drop(decoder);
    Some((format, furthest.get().min(length), length))
}

/// Where a JPEG's first scan starts, past its header: the end of what tells its size and holds its
/// EXIF. `None` for a file whose segments don't lead to one.
fn first_scan(path: &Path) -> Option<u64> {
    let bytes = std::fs::read(path).ok()?;
    let mut at = 2usize;
    loop {
        let [0xFF, marker] = *bytes.get(at..at + 2)? else { return None };
        at += 2;
        match marker {
            // Padding, and the markers that stand alone.
            0xFF => at -= 1,
            0x01 | 0xD0..=0xD8 => {}
            _ => {
                let length = usize::from(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]));
                at += length;
                if marker == 0xDA {
                    return Some(at as u64);
                }
            }
        }
    }
}

fn main() {
    let (mut all, mut paths) = (false, Vec::new());
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--all" => all = true,
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    let (mut by_format, mut past) = (BTreeMap::<String, (usize, usize, u64)>::new(), Vec::new());
    while let Some(path) = paths.pop() {
        if path.is_dir() {
            paths.extend(std::fs::read_dir(&path).into_iter().flatten().flatten().map(|entry| entry.path()));
            continue;
        }
        let Some((format, read, length)) = measured(&path) else { continue };
        let entry = by_format.entry(format!("{format:?}")).or_default();
        entry.0 += 1;
        entry.1 += usize::from(read <= WITHIN || read == length && length <= WITHIN);
        entry.2 = entry.2.max(read);
        if format == ImageFormat::Jpeg
            && let Some(scan) = first_scan(&path)
        {
            let entry = by_format.entry("Jpeg headers".to_owned()).or_default();
            entry.0 += 1;
            entry.1 += usize::from(scan <= WITHIN);
            entry.2 = entry.2.max(scan);
            if all || scan > WITHIN {
                past.push((scan, length, path));
            }
        } else if all || read > WITHIN {
            past.push((read, length, path));
        }
    }
    println!("{:<12} {:>8} {:>14} {:>16}", "format", "pictures", "within 64 KiB", "furthest read");
    for (format, (count, within, furthest)) in &by_format {
        println!("{format:<12} {count:>8} {within:>14} {furthest:>16}");
    }
    past.sort();
    for (read, length, path) in past {
        println!("{read:>10} of {length:>10}  {}", path.display());
    }
}
