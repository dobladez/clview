//! The vectorcraft backend, `src/source/vectorcraft.rs`: a drawing vectorcraft saved, each artboard
//! a page; the rule that nothing else names vectorcraft; and its pin. Its own tests read the first
//! bytes it goes by, and stop a render that takes too long.
#![cfg(all(unix, feature = "vector"))]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::setup::Background;
use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_geom::{Rect, shapes};

use crate::view::{images, names, png_size, text, tmp, view};

/// A drawing on a white background with an artboard for each of `boards`, each filled with a
/// rectangle of its colour inset 10 pt.
fn drawing(boards: &[(Rect, (f32, f32, f32))]) -> Document {
    let mut document = Document::new(100.0, 100.0);
    document.setup.background = Background::White;
    let layer = document.layers[0].id;
    let first = document.artboards[0].clone();
    document.artboards.clear();
    for (n, (rect, (r, g, b))) in boards.iter().enumerate() {
        let mut artboard = first.clone();
        artboard.id = n as u32 + 1;
        artboard.rect = *rect;
        document.artboards.push(artboard);
        let id = document.alloc_id();
        let inset = Rect::new(rect.x0 + 10.0, rect.y0 + 10.0, rect.x1 - 10.0, rect.y1 - 10.0);
        let shape = Node::path(id, shapes::rectangle(inset), Appearance::basic(Paint::solid(Color::rgb(*r, *g, *b)), Paint::None, 0.0));
        document.insert(Some(layer), 0, shape).unwrap();
    }
    document
}

/// Two artboards: 200 × 300 pt in red, and beside it 400 × 100 pt in blue.
fn two_boards() -> Document {
    drawing(&[(Rect::new(0.0, 0.0, 200.0, 300.0), (1.0, 0.0, 0.0)), (Rect::new(300.0, 0.0, 700.0, 100.0), (0.0, 0.0, 1.0))])
}

fn write(path: PathBuf, bytes: &[u8]) -> PathBuf {
    std::fs::write(&path, bytes).unwrap();
    path
}

/// Each page `view` drew of `path`: its size, and the colour at its middle.
fn drawn(path: &Path) -> Vec<((u32, u32), [u8; 4])> {
    let out = view(path, &["--protocol", "kitty"]);
    assert!(out.status.success() && out.stderr.is_empty(), "{}: {}", path.display(), text(&out.stderr));
    let middle = |png: &[u8]| {
        let page = image::load_from_memory(png).unwrap().to_rgba8();
        page.get_pixel(page.width() / 2, page.height() / 2).0
    };
    images(&out.stdout).iter().map(|d| (png_size(&d.png), middle(&d.png))).collect()
}

/// Whether `size` is `w` × `h` in shape, within 2%.
fn shaped((x, y): (u32, u32), w: f32, h: f32) -> bool {
    ((x as f32 / y as f32) / (w / h) - 1.0).abs() < 0.02
}

/// Each artboard is a page, its size its own, drawn with what is on it.
#[test]
fn a_drawing_is_its_artboards() {
    let path = write(tmp("vector-boards", "two.vectorcraft"), &vectorcraft_format::save(&two_boards(), false));
    let pages = drawn(&path);
    assert_eq!(pages.len(), 2, "{pages:?}");
    assert!(shaped(pages[0].0, 200.0, 300.0) && shaped(pages[1].0, 400.0, 100.0), "{pages:?}");
    assert_eq!((pages[0].1, pages[1].1), ([255, 0, 0, 255], [0, 0, 255, 255]));
    // Pretty-printed, as vectorcraft writes it when asked to, it is the same drawing.
    let pretty = write(tmp("vector-boards", "pretty.vectorcraft"), &vectorcraft_format::save(&two_boards(), true));
    assert_eq!(drawn(&pretty).len(), 2);
}

/// A drawing whose first bytes hold `%PDF-` (its text, a note) is still a drawing: this backend
/// is asked before the PDF one, which goes by `%PDF-` anywhere in them.
#[test]
fn a_drawing_that_mentions_a_pdf_is_still_a_drawing() {
    let saved = vectorcraft_format::save(&two_boards(), false);
    let start = br#"{"format":"vectorcraft","#;
    assert!(saved.starts_with(start), "{}", String::from_utf8_lossy(&saved[..40]));
    let noted = [&start[..], br#""note":"%PDF-1.7 brochure","#, &saved[start.len()..]].concat();
    assert_eq!(drawn(&write(tmp("vector-pdf", "brochure.vectorcraft"), &noted)).len(), 2);
}

/// Saved compressed (vectorcraft's Use Compression), a drawing is gzipped: the tar backend unpacks
/// it, as any file compressed alone, and this one shows it.
#[cfg(feature = "tar")]
#[test]
fn a_compressed_drawing_is_shown_too() {
    let options = vectorcraft_format::SaveOptions { compress: true, ..Default::default() };
    let packed = vectorcraft_format::save_with(&two_boards(), &options).unwrap();
    assert_eq!(&packed[..2], [0x1f, 0x8b], "gzipped");
    assert_eq!(drawn(&write(tmp("vector-gz", "packed.vectorcraft"), &packed)).len(), 2);
}

/// In a directory a drawing is a tile, its first artboard; JSON that only mentions vectorcraft is
/// not one.
#[test]
fn in_a_list_a_drawing_is_its_first_artboard_and_other_json_is_not_one() {
    let dir = tmp("vector-list", "shelf");
    std::fs::create_dir_all(&dir).unwrap();
    write(dir.join("a.vectorcraft"), &vectorcraft_format::save(&two_boards(), false));
    write(dir.join("package.json"), br#"{"name": "vectorcraft", "format": "vectorcraft"}"#);
    let out = view(&dir, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(names(&out.stdout), [" a.vectorcraft"]);
    assert!(shaped(png_size(&images(&out.stdout)[0].png), 200.0, 300.0));
}

/// A drawing without artboards has nothing to show, and says so.
#[test]
fn a_drawing_without_artboards_says_so() {
    let path = write(tmp("vector-none", "empty.vectorcraft"), &vectorcraft_format::save(&drawing(&[]), false));
    let out = view(&path, &["--protocol", "kitty"]);
    assert_eq!(text(&out.stderr), format!("clv: {}: the drawing has no artboards to show\n", path.display()));
}

/// The crates vectorcraft is built from, as `cargo metadata` resolves them, and its graph.
fn metadata() -> Value {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let out = Command::new(cargo).args(["metadata", "--format-version", "1", "--manifest-path"]).arg(&manifest).output().unwrap();
    assert!(out.status.success(), "cargo metadata: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

/// Every vectorcraft crate comes from the one commit `Cargo.toml` pins. vectorcraft patches crates
/// (`[patch.crates-io]`, which Cargo honours only in the root manifest) for what this backend
/// doesn't build, its PDF export and its app: none of them may be among what the vectorcraft
/// crates are built with, stock, without the patch. (The PDF backend builds some of the same
/// crates, patched its own way.)
#[test]
fn every_vectorcraft_crate_is_from_the_pinned_commit_and_none_it_patches_is_built_for_it() {
    let metadata = metadata();
    let packages = metadata["packages"].as_array().unwrap();
    let render = packages.iter().find(|p| p["name"] == "vectorcraft-render").unwrap();
    let pinned = render["source"].as_str().unwrap();
    assert!(pinned.starts_with("git+https://github.com/storytold/vectorcraft?rev="), "vectorcraft-render from {pinned}");
    let ours: Vec<&Value> = packages.iter().filter(|p| p["name"].as_str().is_some_and(|n| n.starts_with("vectorcraft-"))).collect();
    assert!(ours.len() >= 3, "{} vectorcraft crates", ours.len());
    for p in &ours {
        assert_eq!(p["source"].as_str(), Some(pinned), "{} is not at the pinned commit", p["name"]);
    }
    // vectorcraft's root manifest at that commit: <root>/crates/render/Cargo.toml → <root>/Cargo.toml.
    let root = Path::new(render["manifest_path"].as_str().unwrap()).ancestors().nth(3).unwrap().join("Cargo.toml");
    let manifest: toml::Table = std::fs::read_to_string(&root).unwrap().parse().unwrap();
    let patched: Vec<&String> = manifest["patch"]["crates-io"].as_table().unwrap().keys().collect();
    assert!(patched.iter().any(|name| *name == "krilla"), "read from {}: {patched:?}", root.display());
    // Everything the vectorcraft crates are built with, from the resolved graph.
    let nodes = metadata["resolve"]["nodes"].as_array().unwrap();
    let deps = |id: &str| -> Vec<String> {
        let node = nodes.iter().find(|n| n["id"] == id).unwrap();
        node["deps"].as_array().unwrap().iter().map(|d| d["pkg"].as_str().unwrap().to_string()).collect()
    };
    let mut seen: Vec<String> = ours.iter().map(|p| p["id"].as_str().unwrap().to_string()).collect();
    let mut todo = seen.clone();
    while let Some(id) = todo.pop() {
        for dep in deps(&id) {
            if !seen.contains(&dep) {
                seen.push(dep.clone());
                todo.push(dep);
            }
        }
    }
    let name = |id: &str| packages.iter().find(|p| p["id"] == id).and_then(|p| p["name"].as_str()).unwrap().to_string();
    let built: Vec<String> = seen.iter().map(|id| name(id)).filter(|n| patched.contains(&n)).collect();
    assert!(built.is_empty(), "vectorcraft patches {built:?}, built for it here without the patch");
}

/// What the vectorcraft backend draws with is named only by it and its tests (`CLAUDE.md`), so it
/// can be swapped for another. "vectorcraft drawings" is the files' own name, which any backend
/// showing them would give.
#[test]
fn nothing_but_this_backend_names_vectorcraft() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let allowed_line = |line: &str| {
        let line = line.trim();
        line == "&vectorcraft::Vectorcraft," || line == "mod vectorcraft;" || line == r#"("vectorcraft drawings", cfg!(feature = "vector")),"#
    };
    let (mut read, mut named) = (0, Vec::new());
    let mut dirs = vec![root.join("src"), root.join("tests")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
            let own = path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("vectorcraft"));
            if own || path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            read += 1;
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                if line.to_lowercase().contains("vectorcraft") && !allowed_line(line) {
                    named.push(format!("{}:{}: {}", path.strip_prefix(root).unwrap().display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(read >= 20, "read {read} files");
    assert!(named.is_empty(), "named outside the vectorcraft backend:\n{}", named.join("\n"));
}
