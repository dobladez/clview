//! The zip backend, through the viewer: an archive is a list as a directory is, its directories
//! folders, and among files, or in a directory, a folder to go into. Its own tests
//! (`src/source/zip.rs`) read archives directly.
#![cfg(all(unix, feature = "zip", feature = "image"))]

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use ::zip::write::SimpleFileOptions;
use ::zip::{CompressionMethod, ZipWriter};

use crate::view::{images, names, png_size, text, tmp, view};
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

/// What `view` drew for `path` with `args`: each image's size, and the names under the tiles.
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
    assert_eq!(shown(&path, &[other, "-r", "--zoom", "2"]).1, [" photos.zip/2024/b.png photos.zip/a9.png", " photos.zip/a10.pdf c.png"]);
}

/// In a directory an archive is a folder, even recursing: `-r` goes through directories, not
/// into every archive (a jar, a document that is a zip inside). Its folder is tied up as a
/// parcel; a directory's is plain.
#[test]
fn in_a_directory_an_archive_is_a_folder_even_recursing() {
    let dir = tmp("zip-dir", "shelf");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    archive(&dir.join("sub/trip.zip"), &[("b.png", png(30, 20))]);
    std::fs::write(dir.join("a.png"), png(40, 30)).unwrap();
    assert_eq!(shown(&dir, &[]).1, [" a.png sub/"]);
    assert_eq!(shown(&dir, &["-r"]).1, [" a.png sub/trip.zip/"]);
    // The second tile: the directory's folder, then the archive's.
    let tied = |args: &[&str]| {
        let out = view(&dir, &[&["--protocol", "kitty"], args].concat());
        let folder = image::load_from_memory(&images(&out.stdout)[1].png).unwrap().to_rgba8();
        // Where the tape crosses: brown on a parcel, the folder's blue on a plain one.
        let [r, _, b, _] = folder.get_pixel(folder.width() / 2, folder.height() * 6 / 10).0;
        r > b
    };
    assert_eq!((tied(&[]), tied(&["-r"])), (false, true));
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
