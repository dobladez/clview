//! The tar backend, through the viewer: a tar, plain or compressed with gzip, bzip2 or xz, is a
//! list as a directory is, and a folder, tied up as a parcel, to go into, read on a thread of its
//! own while the pager says how far it has got; a file compressed alone is what it unpacks to. Its
//! own tests (`src/source/tar.rs`) read archives directly.
#![cfg(all(unix, feature = "tar", feature = "image"))]

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use crate::view::{images, names, png_size, text, tmp, view};
use crate::view_pager::{Session, bar, chosen, pty_works};

/// A `width` × `height` PNG.
fn png(width: u32, height: u32) -> Vec<u8> {
    let picture = image::RgbaImage::from_fn(width, height, |x, y| image::Rgba([(x % 251) as u8, (y % 251) as u8, 64, 255]));
    let mut bytes = Cursor::new(Vec::new());
    picture.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// A POSIX tar header for `name`, of `size` bytes: a directory when the name ends with `/`.
fn header(name: &str, size: u64) -> [u8; 512] {
    let mut block = [0u8; 512];
    block[..name.len()].copy_from_slice(name.as_bytes());
    block[100..108].copy_from_slice(b"0000644\0");
    block[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    block[156] = if name.ends_with('/') { b'5' } else { b'0' };
    block[257..265].copy_from_slice(b"ustar\x0000");
    block[148..156].fill(b' ');
    let sum: u32 = block.iter().map(|b| u32::from(*b)).sum();
    block[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    block
}

/// A tar of `files`, by name and contents.
fn tar(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data) in files {
        out.extend(header(name, data.len() as u64));
        out.extend(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.extend([0; 1024]);
    out
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn bzip2(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn xz(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(1)).unwrap();
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn write(path: PathBuf, bytes: &[u8]) -> PathBuf {
    std::fs::write(&path, bytes).unwrap();
    path
}

/// What `view` drew for `path` with `args`: each image's size, and the names under the tiles.
fn shown(path: &Path, args: &[&str]) -> (Vec<(u32, u32)>, Vec<String>) {
    let out = view(path, &[&["--protocol", "kitty"], args].concat());
    assert!(out.status.success() && out.stderr.is_empty(), "{args:?}: {}", text(&out.stderr));
    (images(&out.stdout).iter().map(|d| png_size(&d.png)).collect(), names(&out.stdout))
}

/// Plain, or compressed with gzip, bzip2 or xz, a tar given is a list as a directory is: each file
/// a backend opens by its first page, each directory a folder; with `-r`, the directories' files.
#[test]
fn a_tar_is_a_list_of_its_files_however_it_is_compressed() {
    let plain = tar(&[("b.png", png(300, 200)), ("notes.txt", b"plain text\n".to_vec()), ("2024/", Vec::new()), ("2024/c.png", png(30, 20))]);
    for (name, bytes) in [("t.tar", plain.clone()), ("t.tar.gz", gzip(&plain)), ("t.tar.bz2", bzip2(&plain)), ("t.txz", xz(&plain))] {
        let path = write(tmp("tar-list", name), &bytes);
        let (sizes, labels) = shown(&path, &[]);
        assert_eq!((sizes.len(), sizes[1], labels), (2, (300, 200), vec![" 2024/ b.png".to_string()]), "{name}: {sizes:?}");
        assert!(sizes[0].0 > sizes[0].1, "{name}: a folder: {:?}", sizes[0]);
        assert_eq!(shown(&path, &["-r"]).1, [" 2024/c.png b.png"], "{name}");
    }
}

/// A file compressed alone shows as the file it unpacks to: a picture, a PDF's pages. One that
/// unpacks to what nothing shows is the error.
#[test]
fn a_file_compressed_alone_is_shown_as_what_it_unpacks_to() {
    let picture = write(tmp("tar-alone", "picture.png.gz"), &gzip(&png(300, 200)));
    assert_eq!(shown(&picture, &[]).0, [(300, 200)]);
    let letter = write(tmp("tar-alone", "letter.pdf.xz"), &xz(&crate::fixtures::letter("Packed", 2)));
    assert_eq!(shown(&letter, &[]).0.len(), 2, "both pages");
    let notes = write(tmp("tar-alone", "notes.txt.bz2"), &bzip2(b"plain text\n"));
    let out = view(&notes, &["--protocol", "kitty"]);
    assert_eq!(text(&out.stderr), format!("clv: {}: it unpacks to a file that clv doesn't show\n", notes.display()));
}

/// In a directory a tar is a folder tied up as a parcel, as a zip is.
#[test]
fn in_a_directory_a_tar_is_a_parcel() {
    let dir = tmp("tar-dir", "shelf");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    write(dir.join("trip.tar.gz"), &gzip(&tar(&[("b.png", png(30, 20))])));
    let out = view(&dir, &["--protocol", "kitty"]);
    assert_eq!(names(&out.stdout), [" sub/ trip.tar.gz/"]);
    let tied = |at: usize| {
        let folder = image::load_from_memory(&images(&out.stdout)[at].png).unwrap().to_rgba8();
        let [r, _, b, _] = folder.get_pixel(folder.width() / 2, folder.height() * 6 / 10).0;
        r > b
    };
    assert_eq!((tied(0), tied(1)), (false, true));
}

/// A tar.gz of `zeros` GiB of zeros (fewer than 8, as an octal size goes), then a picture: small
/// on disk, a while to read through. Gzip streams one after another read as one, so each GiB is
/// the same stream again.
fn slow(zeros: u64) -> Vec<u8> {
    let gib = gzip(&vec![0; 1 << 30]);
    let mut bytes = gzip(&header("zeros.bin", zeros << 30));
    for _ in 0..zeros {
        bytes.extend(&gib);
    }
    bytes.extend(gzip(&tar(&[("last.png", png(80, 60))])));
    bytes
}

/// In the pager, a tar gone into is read on a thread of its own, the status line saying how far
/// it has got, and Esc stops it; one that reads quickly opens at once, and `u` comes back up.
#[test]
fn a_tar_being_read_says_how_far_it_has_got_and_esc_stops_it() {
    if !pty_works() {
        return;
    }
    let dir = tmp("tar-pager", "shelf");
    std::fs::create_dir_all(&dir).unwrap();
    write(dir.join("big.tar.gz"), &slow(7));
    write(dir.join("small.tar.gz"), &gzip(&tar(&[("b.png", png(800, 600)), ("c.png", png(400, 300))])));
    let mut view = Session::start(&dir, &["--protocol", "kitty"], 80, 48);
    view.wait_until("the list", |t| bar(t).contains(" big.tar.gz/ ") && bar(t).contains(" 1/2 "));
    view.press("\r");
    view.wait_until("reading", |t| bar(t).contains(" reading big.tar.gz/… "));
    view.press("\x1b");
    view.wait_until("stopped", |t| bar(t).contains(" stopped opening big.tar.gz/ "));
    view.press("l\r");
    view.wait_until("in small", |t| bar(t).contains(" small.tar.gz/b.png 1/2 ") && chosen(t).as_deref() == Some("b.png"));
    view.press("u");
    view.wait_until("back", |t| bar(t).contains(" small.tar.gz/ 2/2 ") && chosen(t).as_deref() == Some("small.tar.gz/"));
    view.press("q");
    assert!(view.finish().success());
}

/// What the tar backend decompresses bzip2 and xz with is named only by it and its tests
/// (`CLAUDE.md`). (Gzip's flate2 is the SVG backend's too, for svgz.)
#[test]
fn nothing_but_this_backend_names_its_decompressors() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let own = [root.join("src/source/tar.rs"), root.join("tests/it/tar.rs")];
    let (mut read, mut named) = (0, Vec::new());
    let mut dirs = vec![root.join("src"), root.join("tests")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
            if own.contains(&path) || path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            read += 1;
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                let lower = line.to_lowercase();
                if line.trim() != "mod tar;" && ["bzip2", "lzma"].iter().any(|library| lower.contains(library)) {
                    named.push(format!("{}:{}: {}", path.strip_prefix(root).unwrap().display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(read >= 20, "read {read} files");
    assert!(named.is_empty(), "named outside the tar backend:\n{}", named.join("\n"));
}
