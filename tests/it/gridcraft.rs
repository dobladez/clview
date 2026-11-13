//! The spreadsheet backend, `src/source/gridcraft.rs`: workbooks (xlsx, ods) and values separated
//! by commas, shown as the pages gridcraft prints them to; a protected workbook opened with its
//! password; the rule that nothing else names gridcraft; and its pin. Its own tests read the first
//! bytes it goes by.
#![cfg(all(unix, feature = "sheet"))]

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

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RELATIONSHIPS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// An xlsx of three sheets, the third hidden, each holding a number and a formula with no value
/// saved for it.
fn xlsx() -> Vec<u8> {
    package(true)
}

/// [`xlsx`], with its styles or without them, which gridcraft warns of.
fn package(styled: bool) -> Vec<u8> {
    let sheets = ["One", "Two", "Hidden"];
    let listed: String = sheets
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let state = if *name == "Hidden" { r#" state="hidden""# } else { "" };
            format!(r#"<sheet name="{name}" sheetId="{n}"{state} r:id="rId{n}"/>"#, n = i + 1)
        })
        .collect();
    let workbook =
        format!(r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="{MAIN}" xmlns:r="{RELATIONSHIPS}"><sheets>{listed}</sheets></workbook>"#);
    let mut related: String = (1..=sheets.len())
        .map(|n| format!(r#"<Relationship Id="rId{n}" Type="{RELATIONSHIPS}/worksheet" Target="worksheets/sheet{n}.xml"/>"#))
        .collect();
    if styled {
        related.push_str(&format!(r#"<Relationship Id="rIdStyles" Type="{RELATIONSHIPS}/styles" Target="styles.xml"/>"#));
    }
    let rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{related}</Relationships>"#
    );
    let overrides: String = (1..=sheets.len())
        .map(|n| format!(r#"<Override PartName="/xl/worksheets/sheet{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#))
        .collect();
    let types = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>{overrides}</Types>"#
    );
    let sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="{MAIN}"><sheetData><row r="1"><c r="A1"><v>21</v></c><c r="B1"><f>A1*2</f></c></row></sheetData></worksheet>"#
    );
    let package_rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RELATIONSHIPS}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
    );
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="{MAIN}"><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="1"><fill><patternFill patternType="none"/></fill></fills><borders count="1"><border/></borders><cellXfs count="1"><xf/></cellXfs></styleSheet>"#
    );
    let mut files = vec![
        ("[Content_Types].xml", types.as_str()),
        ("_rels/.rels", &package_rels),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", &rels),
        ("xl/worksheets/sheet1.xml", &sheet),
        ("xl/worksheets/sheet2.xml", &sheet),
        ("xl/worksheets/sheet3.xml", &sheet),
    ];
    if styled {
        files.push(("xl/styles.xml", &styles));
    }
    zip(&files)
}

/// An OpenDocument spreadsheet of one sheet, one cell.
fn ods() -> Vec<u8> {
    let content = r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.3"><office:body><office:spreadsheet><table:table table:name="Sheet1"><table:table-row><table:table-cell office:value-type="float" office:value="42"><text:p>42</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#;
    zip(&[
        ("mimetype", "application/vnd.oasis.opendocument.spreadsheet"),
        (
            "META-INF/manifest.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#,
        ),
        ("content.xml", content),
    ])
}

const CSV: &str = "date,item,qty,note\n2026-03-01,widget,3,\"said \"\"hello\"\", then left\"\n2026-03-02,gadget,5,a longer note than its column\n";

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

/// A workbook is the pages its visible sheets print to, each a sheet of paper; a hidden sheet
/// prints none. A CSV too, and an ods, which says what it leaves out.
#[test]
fn a_workbook_is_its_visible_sheets_printed() {
    let portrait = |pages: &[(u32, u32)]| pages.iter().all(|(w, h)| h > w);
    let pages = drawn(&write(tmp("sheet-pages", "book.xlsx"), &xlsx()), &[]);
    assert!(pages.len() == 2 && portrait(&pages), "{pages:?}");
    assert_eq!(drawn(&write(tmp("sheet-pages", "rows.csv"), CSV.as_bytes()), &[]).len(), 1);
    let out = view(&write(tmp("sheet-pages", "one.ods"), &ods()), &["--protocol", "kitty"]);
    assert_eq!(images(&out.stdout).len(), 1);
    assert_eq!(text(&out.stderr), "warning: page 1: an ods is shown by its values: its formatting, charts and pictures are left out\n");
}

/// What gridcraft says on reading a workbook, that the reader should know, comes with its first
/// page, as a page's own warnings do.
#[test]
fn what_gridcraft_says_on_reading_comes_with_the_first_page() {
    let path = write(tmp("sheet-said", "plain.xlsx"), &package(false));
    let out = view(&path, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stderr), "warning: page 1: no styles part\n");
}

/// A workbook is searched as the PDF it prints to, one that opened with something to say too,
/// whose pages are wrapped to carry it: `/` finds a cell's value on each sheet's page, and `n`
/// goes from one to the next.
#[test]
fn a_workbook_that_opened_with_warnings_is_searched_through_its_pdf() {
    use crate::view_pager::{Session, bar, pty_works};
    if !pty_works() {
        return;
    }
    // Without its styles, which gridcraft says: A1 is 21 and B1, `A1*2`, prints as 42, on two sheets.
    let path = write(tmp("sheet-search", "plain.xlsx"), &package(false));
    let mut view = Session::start(&path, &["--protocol", "kitty"], 100, 60);
    let last = |text: &str| bar(text.rfind("\x1b[?2026h").map_or(text, |at| &text[at..]));
    view.wait_until("the first sheet, and what gridcraft said", |t| last(t).contains(" page 1: no styles part ") && last(t).contains(" page 1/2 "));
    view.press("/42\r");
    view.wait_until("the first match", |t| last(t).contains(" plain.xlsx /42 1/2 ") && last(t).contains(" page 1/2 "));
    view.press("n");
    view.wait_until("the second sheet's", |t| last(t).contains(" plain.xlsx /42 2/2 ") && last(t).contains(" page 2/2 "));
    // A formula isn't text on the page: its value is.
    view.press("\x1b/A1*2\r");
    view.wait_until("no match", |t| last(t).contains(" plain.xlsx /A1*2 no match "));
    view.press("\x1bq");
    assert!(view.finish().success());
}

/// A workbook is known by what it holds, whatever its name; values separated by commas only by
/// theirs, so the same text named `.txt` is not one.
#[test]
fn a_workbook_is_known_by_what_it_holds_and_a_csv_by_its_name() {
    assert_eq!(drawn(&write(tmp("sheet-known", "renamed.zip"), &xlsx()), &[]).len(), 2);
    let notes = write(tmp("sheet-known", "rows.txt"), CSV.as_bytes());
    let out = view(&notes, &["--protocol", "kitty"]);
    assert!(!out.status.success() && text(&out.stderr).contains("not a file clv shows"), "{}", text(&out.stderr));
}

/// In a directory a workbook is a tile, as a PDF is: its first page.
#[test]
fn in_a_list_a_workbook_is_its_first_page() {
    let dir = tmp("sheet-list", "shelf");
    std::fs::create_dir_all(&dir).unwrap();
    write(dir.join("a.xlsx"), &xlsx());
    write(dir.join("b.csv"), CSV.as_bytes());
    let out = view(&dir, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!((names(&out.stdout), images(&out.stdout).len()), (vec![" a.xlsx b.csv".to_string()], 2));
}

/// A protected workbook opens with its password, says it needs one without, and refuses a wrong
/// one.
#[test]
fn a_protected_workbook_opens_with_its_password() {
    let path = write(tmp("sheet-password", "locked.xlsx"), &gridcraft_xlsx::testing::encrypt_workbook(&xlsx(), "secret"));
    assert_eq!(drawn(&path, &["--password", "secret"]).len(), 2);
    let out = view(&path, &["--protocol", "kitty"]);
    assert_eq!(text(&out.stderr), format!("clv: {}: this workbook is protected with a password: give it with --password\n", path.display()));
    let out = view(&path, &["--protocol", "kitty", "--password", "wrong"]);
    assert_eq!(text(&out.stderr), format!("clv: {}: the password is incorrect\n", path.display()));
}

/// The crates gridcraft is built from, as `cargo metadata` resolves them.
fn packages() -> Vec<Value> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let out = Command::new(cargo).args(["metadata", "--format-version", "1", "--manifest-path"]).arg(&manifest).output().unwrap();
    assert!(out.status.success(), "cargo metadata: {}", String::from_utf8_lossy(&out.stderr));
    let metadata: Value = serde_json::from_slice(&out.stdout).unwrap();
    metadata["packages"].as_array().unwrap().clone()
}

/// Every gridcraft crate comes from the one commit `Cargo.toml` pins. gridcraft patches crates
/// (`[patch.crates-io]`, which Cargo honours only in the root manifest) for what this backend
/// doesn't build, its app: none of them may be built here, stock, without the patch.
#[test]
fn every_gridcraft_crate_is_from_the_pinned_commit_and_none_it_patches_is_built() {
    let packages = packages();
    let engine = packages.iter().find(|p| p["name"] == "gridcraft-engine").unwrap();
    let pinned = engine["source"].as_str().unwrap();
    assert!(pinned.starts_with("git+https://github.com/storytold/gridcraft?rev="), "gridcraft-engine from {pinned}");
    let ours: Vec<&Value> = packages.iter().filter(|p| p["name"].as_str().is_some_and(|n| n.starts_with("gridcraft-"))).collect();
    assert!(ours.len() >= 6, "{} gridcraft crates", ours.len());
    for p in ours {
        assert_eq!(p["source"].as_str(), Some(pinned), "{} is not at the pinned commit", p["name"]);
    }
    // gridcraft's root manifest at that commit: <root>/crates/engine/Cargo.toml → <root>/Cargo.toml.
    let root = Path::new(engine["manifest_path"].as_str().unwrap()).ancestors().nth(3).unwrap().join("Cargo.toml");
    let manifest: toml::Table = std::fs::read_to_string(&root).unwrap().parse().unwrap();
    let patched: Vec<&String> = manifest["patch"]["crates-io"].as_table().unwrap().keys().collect();
    assert!(patched.iter().any(|name| *name == "egui"), "read from {}: {patched:?}", root.display());
    let built: Vec<&String> = patched.into_iter().filter(|name| packages.iter().any(|p| p["name"] == name.as_str())).collect();
    assert!(built.is_empty(), "gridcraft patches {built:?}, built here without the patch");
}

/// What the spreadsheet backend reads and prints with is named only by it and its tests
/// (`CLAUDE.md`), so it can be swapped for another.
#[test]
fn nothing_but_this_backend_names_gridcraft() {
    crate::fixtures::named_only_in(
        &["src/source/gridcraft.rs", "tests/it/gridcraft.rs"],
        &["&gridcraft::Gridcraft,", "mod gridcraft;"],
        &["gridcraft"],
    );
}
