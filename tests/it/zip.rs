//! The zip backend, through the viewer: an archive is a list as a directory is, its directories
//! folders, and among files, or in a directory, a folder to go into. Its own tests
//! (`src/source/zip.rs`) read archives directly.
#![cfg(all(unix, feature = "zip", feature = "image"))]

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use ::zip::write::SimpleFileOptions;
use ::zip::{CompressionMethod, ZipWriter};

use crate::view::{images, names, png_size, text, tmp, view};
#[cfg(feature = "word")]
use crate::view_pager::{PLACED, ids_after, next_image};
use crate::view_pager::{Session, bar, chosen, pty_works};

/// A `width` × `height` PNG.
fn png(width: u32, height: u32) -> Vec<u8> {
    let picture = image::RgbaImage::from_fn(width, height, |x, y| image::Rgba([(x % 251) as u8, (y % 251) as u8, 64, 255]));
    let mut bytes = Cursor::new(Vec::new());
    picture.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// An archive at `path` of `files`, by name and contents, deflated: a name ending in `/` is a
/// directory.
fn archive(path: &Path, files: &[(&str, Vec<u8>)]) -> PathBuf {
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
    std::fs::write(path, zip.finish().unwrap().into_inner()).unwrap();
    path.to_owned()
}

/// What `clv` drew for `path` with `args`: each image's size, and the names under the tiles.
fn shown(path: &Path, args: &[&str]) -> (Vec<(u32, u32)>, Vec<String>) {
    let out = view(path, &[&["--protocol", "kitty"], args].concat());
    assert!(out.status.success() && out.stderr.is_empty(), "{args:?}: {}", text(&out.stderr));
    (images(&out.stdout).iter().map(|d| png_size(&d.png)).collect(), names(&out.stdout))
}

/// An archive given is a list as a directory is: each file a backend opens by its first page, read
/// from the archive, each directory a folder, hidden files and files nothing shows passed over;
/// with `-r`, the directories' files instead of folders. Among other files it is a folder, and
/// recursing, its files named from it.
#[test]
fn an_archive_is_a_list_of_its_files_and_its_directories_are_folders() {
    let path = archive(
        &tmp("zip-list", "photos.zip"),
        &[
            ("a10.pdf", crate::fixtures::letter("Zipped", 3)),
            ("a9.png", png(300, 200)),
            ("2024/b.png", png(30, 20)),
            (".hidden.png", png(50, 50)),
            ("notes.txt", b"plain text\n".to_vec()),
        ],
    );
    // As the directory of the same files shows: a folder, the picture, the PDF's first page.
    let (sizes, labels) = shown(&path, &[]);
    assert_eq!((sizes.len(), sizes[1], labels), (3, (250, 167), vec![" 2024/ a9.png a10.pdf".to_string()]), "{sizes:?}");
    assert!(sizes[0].0 > sizes[0].1, "a folder: {:?}", sizes[0]);
    let (w, h) = sizes[2];
    assert!((w as f32 / h as f32 - 612.0 / 792.0).abs() < 0.01, "the PDF's first page: {w}×{h}");
    let (sizes, labels) = shown(&path, &["-r"]);
    assert_eq!((sizes.len(), sizes[0], sizes[1], labels), (3, (30, 20), (250, 167), vec![" 2024/b.png a9.png a10.pdf".to_string()]));
    // Among others, a folder; recursing, its files, named from it.
    let other = tmp("zip-list", "c.png");
    std::fs::write(&other, png(40, 30)).unwrap();
    let other = other.to_str().unwrap();
    assert_eq!(shown(&path, &[other]).1, [" photos.zip/ c.png"]);
    assert_eq!(shown(&path, &[other, "-r", "--per-row", "2"]).1, [" photos.zip/2024/b.png photos.zip/a9.png", " photos.zip/a10.pdf c.png"]);
}

/// In a directory an archive is a folder, even recursing: `-r` goes through directories, not
/// into every archive (a jar, a document that is a zip inside). Its folder carries a plate
/// saying its kind; a directory's is plain.
#[test]
fn in_a_directory_an_archive_is_a_folder_even_recursing() {
    let dir = tmp("zip-dir", "shelf");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    archive(&dir.join("sub/trip.zip"), &[("b.png", png(30, 20))]);
    std::fs::write(dir.join("a.png"), png(40, 30)).unwrap();
    assert_eq!(shown(&dir, &[]).1, [" a.png sub/"]);
    assert_eq!(shown(&dir, &["-r"]).1, [" a.png sub/trip.zip/"]);
    // The second tile: the directory's folder, then the archive's.
    let plated = |args: &[&str]| {
        let out = view(&dir, &[&["--protocol", "kitty"], args].concat());
        let folder = image::load_from_memory(&images(&out.stdout)[1].png).unwrap().to_rgba8();
        // On the plate, under its letters: orange on an archive, the folder's blue on a plain one.
        let [r, _, b, _] = folder.get_pixel(folder.width() * 56 / 100, folder.height() * 9 / 10).0;
        r > b
    };
    assert_eq!((plated(&[]), plated(&["-r"])), (false, true));
}

/// An archive with nothing to show says so; one that isn't one after all is the error.
#[test]
fn an_archive_with_nothing_to_show_says_so() {
    let path = archive(&tmp("zip-empty", "empty.zip"), &[]);
    let out = view(&path, &["--protocol", "kitty"]);
    assert!(!out.status.success() && out.stdout.is_empty());
    assert_eq!(text(&out.stderr), format!("clv: {}: nothing in it that clv shows\n", path.display()));
    let path = archive(&tmp("zip-empty", "notes.zip"), &[("notes.txt", b"plain text\n".to_vec())]);
    assert_eq!(text(&view(&path, &["--protocol", "kitty"]).stderr), format!("clv: {}: nothing in it that clv shows\n", path.display()));
    let path = tmp("zip-empty", "broken.zip");
    std::fs::write(&path, b"PK\x03\x04 and then nothing").unwrap();
    let out = view(&path, &["--protocol", "kitty"]);
    assert!(!out.status.success() && text(&out.stderr).starts_with(&format!("clv: {}: ", path.display())), "{}", text(&out.stderr));
}

/// A zip whose first file is a PDF, stored, holds the PDF's header in its first 1024 bytes: it is
/// still a zip, a list of its files, never taken for a PDF that can't be read.
#[test]
fn an_archive_starting_with_a_stored_pdf_is_an_archive() {
    let path = tmp("zip-stored-pdf", "papers.zip");
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for name in ["a.pdf", "b.pdf"] {
        zip.start_file(name, options).unwrap();
        zip.write_all(&crate::fixtures::letter(name, 1)).unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    assert!(bytes[..1024].windows(5).any(|w| w == b"%PDF-"), "the PDF's header among the first bytes");
    std::fs::write(&path, bytes).unwrap();
    let (sizes, labels) = shown(&path, &[]);
    assert_eq!((sizes.len(), labels), (2, vec![" a.pdf b.pdf".to_string()]), "{sizes:?}");
}

/// In the pager an archive is a folder to go into, as a directory is: Enter lists it, its files
/// named from it on the status line, and Backspace goes back.
#[test]
fn an_archive_is_a_folder_to_go_into() {
    if !pty_works() {
        return;
    }
    let dir = tmp("zip-pager", "shelf");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.png"), png(800, 600)).unwrap();
    archive(&dir.join("trip.zip"), &[("b.png", png(800, 600)), ("c.png", png(400, 300))]);
    let mut view = Session::start(&dir, &["--protocol", "kitty"], 80, 48);
    view.wait_until("the list", |t| bar(t).contains(" a.png ") && bar(t).contains(" 1/2 "));
    view.press("l\r");
    view.wait_until("in trip.zip", |t| bar(t).contains(" trip.zip/b.png 1/2 ") && chosen(t).as_deref() == Some("b.png"));
    view.press("l");
    view.wait_until("c chosen", |t| bar(t).contains(" trip.zip/c.png 2/2 "));
    view.press("\x7f");
    view.wait_until("back", |t| bar(t).contains(" trip.zip/ 2/2 ") && chosen(t).as_deref() == Some("trip.zip/"));
    view.press("q");
    assert!(view.finish().success());
}

/// A `side` × `side` PNG of `colour`.
#[cfg(feature = "word")]
fn plain(side: u32, colour: [u8; 3]) -> Vec<u8> {
    let [r, g, b] = colour;
    let mut png = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(side, side, image::Rgba([r, g, b, 255])).write_to(&mut png, image::ImageFormat::Png).unwrap();
    png.into_inner()
}

#[cfg(feature = "word")]
const RED: [u8; 3] = [220, 20, 20];
#[cfg(feature = "word")]
const BLUE: [u8; 3] = [20, 20, 220];
#[cfg(feature = "word")]
const GREEN: [u8; 3] = [20, 180, 20];
/// The grey of a picture's stand-in: what a document shows of one it measured and couldn't draw.
#[cfg(feature = "word")]
const STAND_IN: [u8; 3] = [0xD0, 0xD0, 0xD0];

/// How many of `page`'s pixels are about `colour`.
#[cfg(feature = "word")]
fn of(page: &image::RgbImage, colour: [u8; 3]) -> usize {
    page.pixels().filter(|pixel| pixel.0.iter().zip(colour).all(|(a, b)| a.abs_diff(b) < 12)).count()
}

/// A Markdown document in an archive (`packed` makes one called `name` of files, by name and
/// contents) shows the pictures beside it there, by their paths from it as on disk: beside it and,
/// by `..`, above it, but nothing above the archive's top, even where a picture of that name is on
/// disk there. Seen as its tile in the list an archive prints as; and in the pager, where the
/// document is opened only once its tile is wanted, opened by itself, and still after `+` lays it
/// out again. Shared with the tar backend's test.
#[cfg(feature = "word")]
pub(crate) fn a_documents_pictures_are_drawn_from_its_archive(test: &str, name: &str, packed: impl Fn(&[(&str, Vec<u8>)]) -> Vec<u8>) {
    let dir = tmp(test, "shelf");
    std::fs::create_dir_all(dir.join("in")).unwrap();
    // Where a path above the archive's top would lead on disk, from the archive and from its files.
    for outside in ["green.png", "in/green.png"] {
        std::fs::write(dir.join(outside), plain(120, GREEN)).unwrap();
    }
    let readme =
        "One\n\n![red](red.png)\n\ntwo\n\n![blue](../assets/my%20blue.png)\n\nthree\n\n![green](../../green.png)\n\n![green](../green.png)\n";
    let files = [("assets/my blue.png", plain(120, BLUE)), ("docs/README.md", readme.as_bytes().to_vec()), ("docs/red.png", plain(120, RED))];
    let archive = dir.join("in").join(name);
    std::fs::write(&archive, packed(&files)).unwrap();

    // Printed, the archive is a list: the document's tile is its first page.
    let out = view(&archive, &["--protocol", "kitty", "-r", "--per-row", "1"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let labels = names(&out.stdout);
    let at = labels.iter().position(|label| label.contains("README.md")).unwrap_or_else(|| panic!("{labels:?}"));
    let tile = image::load_from_memory(&images(&out.stdout)[at].png).unwrap().to_rgb8();
    assert!(of(&tile, RED) > 0 && of(&tile, BLUE) > 0, "beside it and above it: {} red, {} blue", of(&tile, RED), of(&tile, BLUE));
    assert_eq!(of(&tile, GREEN), 0, "nothing from above the archive's top");

    if !pty_works() {
        return;
    }
    let mut view = Session::start(&archive, &["--protocol", "kitty", "-r"], 100, 50);
    view.wait_until("the list", |t| bar(t).contains(" 1/3 "));
    view.press("l");
    view.wait_until("the document chosen", |t| chosen(t).as_deref() == Some("docs/README.md"));
    let shown = ids_after(&view.text(), PLACED).last().copied().unwrap();
    view.press("\r");
    view.wait_until("the document opened", |t| bar(t).contains(" page 1/1 "));
    let (opened, page) = next_image(&view, "its page", shown);
    assert!(of(&page, RED) > 0 && of(&page, BLUE) > 0 && of(&page, GREEN) == 0, "{} red, {} blue", of(&page, RED), of(&page, BLUE));
    // Laid out again at a larger font, from the archive again: its pictures with it.
    view.press("+");
    let (_, larger) = next_image(&view, "laid out again", opened);
    assert!(of(&larger, RED) > 0 && of(&larger, BLUE) > 0 && of(&larger, GREEN) == 0, "{} red, {} blue", of(&larger, RED), of(&larger, BLUE));
    view.press("q");
    assert!(view.finish().success());
}

/// A Markdown README in a zip shows the screenshots beside it.
#[cfg(feature = "word")]
#[test]
fn a_documents_pictures_are_drawn_from_its_zip() {
    a_documents_pictures_are_drawn_from_its_archive("zip-pictures", "repo.zip", |files| {
        let path = tmp("zip-pictures", "packed.zip");
        std::fs::read(archive(&path, files)).unwrap()
    });
}

/// A picture is measured, for the document to be laid out around it, from its first bytes: one of
/// 10 MB whose bytes past them can't be read from the archive (they fail its checksum) is still
/// given its place on the page, and only then, drawn, found unreadable.
#[cfg(feature = "word")]
#[test]
fn a_documents_pictures_are_measured_without_being_read_whole() {
    // A PNG's header, saying 800 × 600, and 10 MB that no decoder is ever shown.
    let mut big = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\x03\x20\0\0\x02\x58\x08\x06\0\0\0".to_vec();
    big.extend(crc32(&big[12..]).to_be_bytes());
    big.extend(10u32.pow(7).to_be_bytes());
    big.extend(b"IDAT");
    big.extend((0..10u32.pow(7)).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8));
    let path = tmp("zip-measured", "big.zip");
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, contents) in [("README.md", &b"![big](big.png)\n"[..]), ("big.png", &big)] {
        zip.start_file(name, stored).unwrap();
        zip.write_all(contents).unwrap();
    }
    let mut bytes = zip.finish().unwrap().into_inner();
    // One byte of the picture, a megabyte in, is no longer what its checksum was taken of.
    let at = bytes.windows(8).position(|window| window == b"\x89PNG\r\n\x1a\n").unwrap() + (1 << 20);
    bytes[at] ^= 0xFF;
    std::fs::write(&path, bytes).unwrap();
    let out = view(&path, &["--protocol", "kitty", "--per-row", "1"]);
    // The picture's own tile fails, read whole; the document's is drawn around it, and says that
    // drawing it then failed.
    let said = text(&out.stderr);
    assert!(said.contains("big.png: Invalid checksum") && said.contains("a picture it refers to could not be drawn"), "{said}");
    let tiles: Vec<image::RgbImage> = images(&out.stdout).iter().map(|tile| image::load_from_memory(&tile.png).unwrap().to_rgb8()).collect();
    let placed = |tile: &image::RgbImage| of(tile, STAND_IN) > (tile.width() * tile.height()) as usize / 10;
    assert!(tiles.iter().any(placed), "its place on the page, measured from its first bytes: {} tiles", tiles.len());
}

/// The CRC-32 of `bytes`, as a PNG's chunk ends with.
#[cfg(feature = "word")]
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}
