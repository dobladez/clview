//! What the tests share: synthetic PDFs, written by hand so that any PDF backend reads them, and
//! the check that what a backend renders with is named only by it.

use std::path::{Path, PathBuf};

/// A US Letter document (612 × 792 pt) of `pages` pages, each reading `title` and its number.
/// `title` is plain text, without parentheses or backslashes.
pub(crate) fn letter(title: &str, pages: usize) -> Vec<u8> {
    // The catalog, the page tree and the font, then each page and its content.
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {pages} /MediaBox [0 0 612 792] >>", kids.join(" ")),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ];
    for i in 0..pages {
        let content = format!("BT /F1 36 Tf 72 680 Td ({title}, page {}) Tj ET", i + 1);
        objects.push(format!("<< /Type /Page /Parent 2 0 R /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>", 5 + 2 * i));
        objects.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
    }
    pdf(&objects)
}

/// A PDF of `objects`, numbered from 1, the first the catalog: a header, the objects, a
/// cross-reference table that finds each one, and the trailer.
pub(crate) fn pdf(objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

/// Panics if the code in `src` or `tests` names any of `libraries` (in lower case, in any case in
/// the code) outside the files `own` (from the crate's root) and the lines `allowed`. Each backend's
/// tests call this with their own names, which live nowhere else: this file names none of them.
pub(crate) fn named_only_in(own: &[&str], allowed: &[&str], libraries: &[&str]) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let own: Vec<PathBuf> = own.iter().map(|file| root.join(file)).collect();
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
                if !allowed.contains(&line.trim()) && libraries.iter().any(|library| lower.contains(library)) {
                    named.push(format!("{}:{}: {}", path.strip_prefix(root).unwrap().display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(read >= 20, "read {read} files");
    assert!(named.is_empty(), "{libraries:?} named outside their backend:\n{}", named.join("\n"));
}
