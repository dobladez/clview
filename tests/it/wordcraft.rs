//! The Word backend, `src/source/wordcraft.rs`: Word documents, OpenDocument text and RTF, as
//! pages; a protected Word document opened with its password; the rule that nothing else names
//! wordcraft; and its pin. Its own tests read the first bytes it goes by.
#![cfg(all(unix, feature = "word"))]

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use ::zip::write::SimpleFileOptions;
use ::zip::{CompressionMethod, ZipWriter};
use serde_json::Value;

use crate::view::{images, names, png_size, text, tmp, view};

/// A zip of `files`, by name and contents, in order: each deflated, but a file named `mimetype`
/// stored, as OpenDocument has it.
fn zip(files: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in files {
        let method = if *name == "mimetype" { CompressionMethod::Stored } else { CompressionMethod::Deflated };
        zip.start_file(*name, SimpleFileOptions::default().compression_method(method)).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// A docx of `pages` pages, 200 × 300 pt each, the n-th reading "Page n".
fn docx(pages: usize) -> Vec<u8> {
    let body: String = (1..=pages)
        .map(|n| {
            let page = format!("<w:p><w:r><w:t>Page {n}</w:t></w:r></w:p>");
            if n < pages { page + r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"# } else { page }
        })
        .collect();
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}<w:sectPr><w:pgSz w:w="4000" w:h="6000"/><w:pgMar w:top="720" w:right="720" w:bottom="720" w:left="720"/></w:sectPr></w:body></w:document>"#
    );
    zip(&[
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        ("word/document.xml", &document),
    ])
}

/// An OpenDocument text of one A5 page (420 × 595 pt), reading "Open".
fn odt() -> Vec<u8> {
    const OFFICE: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.3""#;
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-styles {OFFICE}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="148mm" fo:page-height="210mm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {OFFICE}><office:body><office:text><text:p>Open</text:p></office:text></office:body></office:document-content>"#
    );
    zip(&[
        ("mimetype", "application/vnd.oasis.opendocument.text"),
        (
            "META-INF/manifest.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/></manifest:manifest>"#,
        ),
        ("styles.xml", &styles),
        ("content.xml", &content),
    ])
}

/// An RTF of three US Letter pages (612 × 792 pt).
const RTF: &str = r"{\rtf1\ansi\deff0{\fonttbl{\f0 Times New Roman;}}\paperw12240\paperh15840 \f0\fs24 One\page Two\page Three\par}";

fn write(path: PathBuf, bytes: &[u8]) -> PathBuf {
    std::fs::write(&path, bytes).unwrap();
    path
}

/// The size of each page `clv` drew of `path`, given `args`.
fn drawn(path: &Path, args: &[&str]) -> Vec<(u32, u32)> {
    let out = view(path, &[&["--protocol", "kitty"], args].concat());
    assert!(out.status.success() && out.stderr.is_empty(), "{}: {}", path.display(), text(&out.stderr));
    images(&out.stdout).iter().map(|d| png_size(&d.png)).collect()
}

/// Whether every one of `sizes` is `w` × `h` in shape.
fn shaped(sizes: &[(u32, u32)], w: f32, h: f32) -> bool {
    sizes.iter().all(|(x, y)| (*x as f32 / *y as f32 - w / h).abs() < 0.01)
}

/// A docx, an odt and an RTF are pages, each its own size, as a PDF's are.
#[test]
fn a_word_document_odt_and_rtf_are_their_pages() {
    let pages = drawn(&write(tmp("word-pages", "three.docx"), &docx(3)), &[]);
    assert!(pages.len() == 3 && shaped(&pages, 200.0, 300.0), "{pages:?}");
    let pages = drawn(&write(tmp("word-pages", "a5.odt"), &odt()), &[]);
    assert!(pages.len() == 1 && shaped(&pages, 420.0, 595.0), "{pages:?}");
    let pages = drawn(&write(tmp("word-pages", "letter.rtf"), RTF.as_bytes()), &[]);
    assert!(pages.len() == 3 && shaped(&pages, 612.0, 792.0), "{pages:?}");
}

/// A Word package is known by what it holds, whatever its name: `word/` among its first files
/// (an RTF by its first bytes, as the backend's own tests check); named as a Word document, one
/// that isn't is the error, rather than a folder.
#[test]
fn a_document_is_known_by_what_it_holds() {
    assert_eq!(drawn(&write(tmp("word-known", "renamed.zip"), &docx(2)), &[]).len(), 2);
    let broken = write(tmp("word-known", "broken.docx"), &zip(&[("notes.txt", "not a document")]));
    let out = view(&broken, &["--protocol", "kitty"]);
    assert!(!out.status.success() && text(&out.stderr).starts_with(&format!("clv: {}: ", broken.display())), "{}", text(&out.stderr));
}

/// In a directory, or an archive, a document is a tile: its first page.
#[test]
fn in_a_list_a_document_is_its_first_page() {
    let dir = tmp("word-list", "shelf");
    std::fs::create_dir_all(&dir).unwrap();
    write(dir.join("a.docx"), &docx(3));
    write(dir.join("b.rtf"), RTF.as_bytes());
    let out = view(&dir, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(names(&out.stdout), [" a.docx b.rtf"]);
    let tiles: Vec<_> = images(&out.stdout).iter().map(|d| png_size(&d.png)).collect();
    assert!(shaped(&tiles[..1], 200.0, 300.0) && shaped(&tiles[1..], 612.0, 792.0), "{tiles:?}");
}

/// A protected Word document opens with its password, says it needs one without, and refuses a
/// wrong one.
#[test]
fn a_protected_document_opens_with_its_password() {
    let locked = wordcraft_docx::encrypt_with_spin_count(&docx(2), "secret", 1000).unwrap();
    let path = write(tmp("word-password", "locked.docx"), &locked);
    let pages = drawn(&path, &["--password", "secret"]);
    assert!(pages.len() == 2 && shaped(&pages, 200.0, 300.0), "{pages:?}");
    let out = view(&path, &["--protocol", "kitty"]);
    assert_eq!(text(&out.stderr), format!("clv: {}: this document is protected with a password: give it with --password\n", path.display()));
    let out = view(&path, &["--protocol", "kitty", "--password", "wrong"]);
    assert_eq!(text(&out.stderr), format!("clv: {}: the password is incorrect\n", path.display()));
}

/// The crates wordcraft is built from, as `cargo metadata` resolves them.
fn packages() -> Vec<Value> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let out = Command::new(cargo).args(["metadata", "--format-version", "1", "--manifest-path"]).arg(&manifest).output().unwrap();
    assert!(out.status.success(), "cargo metadata: {}", String::from_utf8_lossy(&out.stderr));
    let metadata: Value = serde_json::from_slice(&out.stdout).unwrap();
    metadata["packages"].as_array().unwrap().clone()
}

/// Every wordcraft crate comes from the one commit `Cargo.toml` pins. wordcraft patches crates
/// (`[patch.crates-io]`, which Cargo honours only in the root manifest) for what this backend
/// doesn't build, its PDF export and its app: none of them may be built here, stock, without the
/// patch.
#[test]
fn every_wordcraft_crate_is_from_the_pinned_commit_and_none_it_patches_is_built() {
    let packages = packages();
    let render = packages.iter().find(|p| p["name"] == "wordcraft-render").unwrap();
    let pinned = render["source"].as_str().unwrap();
    assert!(pinned.starts_with("git+https://github.com/storytold/wordcraft?rev="), "wordcraft-render from {pinned}");
    let ours: Vec<&Value> = packages.iter().filter(|p| p["name"].as_str().is_some_and(|n| n.starts_with("wordcraft-"))).collect();
    assert!(ours.len() >= 6, "{} wordcraft crates", ours.len());
    for p in ours {
        assert_eq!(p["source"].as_str(), Some(pinned), "{} is not at the pinned commit", p["name"]);
    }
    // wordcraft's root manifest at that commit: <root>/crates/render/Cargo.toml → <root>/Cargo.toml.
    let root = Path::new(render["manifest_path"].as_str().unwrap()).ancestors().nth(3).unwrap().join("Cargo.toml");
    let manifest: toml::Table = std::fs::read_to_string(&root).unwrap().parse().unwrap();
    let patched: Vec<&String> = manifest["patch"]["crates-io"].as_table().unwrap().keys().collect();
    assert!(patched.iter().any(|name| *name == "krilla"), "read from {}: {patched:?}", root.display());
    let built: Vec<&String> = patched.into_iter().filter(|name| packages.iter().any(|p| p["name"] == name.as_str())).collect();
    assert!(built.is_empty(), "wordcraft patches {built:?}, built here without the patch");
}

/// What the Word backend renders with is named only by it and its tests (`CLAUDE.md`), so it can
/// be swapped for another.
#[test]
fn nothing_but_this_backend_names_wordcraft() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let allowed_line = |line: &str| {
        let line = line.trim();
        line == "&wordcraft::Wordcraft," || line == "mod wordcraft;"
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
            let own = path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("wordcraft"));
            if own || path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            read += 1;
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                if line.to_lowercase().contains("wordcraft") && !allowed_line(line) {
                    named.push(format!("{}:{}: {}", path.strip_prefix(root).unwrap().display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(read >= 20, "read {read} files");
    assert!(named.is_empty(), "named outside the Word backend:\n{}", named.join("\n"));
}
