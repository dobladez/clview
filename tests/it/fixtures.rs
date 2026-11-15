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

/// A PDF with links, for following them: three pages, each reading its number.
///
/// - **Page 1**, US Letter. Down its left side, a link every 40 pt, each 200 pt wide and 20 high,
///   the first with its top 72 pt down the page as shown (`/Rect [72 700 272 720]`), in this order:
///   to page 3 by `/XYZ`, its top at 200 of that page's 300 units; to the named destination
///   `chapter` (page 2, `/XYZ 300 400`); to `https://example.com/a?b=c`; to `other.pdf`'s
///   destination `intro` (`/GoToR`), and to its third page; then to page 2 by `/FitH 400`,
///   `/FitV 300`, `/FitR 200 300 400 500`, `/Fit` and `/XYZ` with nothing given; to page 1 itself
///   by `/FitR 72 500 300 700`; a script; a program to launch (`run.sh`); and last, at
///   `/Rect [72 160 300 200]`, a link to page 3 active only over two lines of that
///   (`/QuadPoints`), 228 pt wide and then 128 pt, 20 pt high each.
/// - **Page 2**, US Letter cropped to `[100 100 500 700]` and turned a quarter (`/Rotate 90`), so
///   600 × 400 pt as shown: a link to page 1, at `/Rect [150 600 250 650]`.
/// - **Page 3**, 300 × 300 units of 2 pt (`/UserUnit 2`), so 600 × 600 pt as shown: a link to
///   `mailto:someone@example.com`, at `/Rect [30 240 130 270]`.
///
/// The catalog names the destination `chapter` in its `/Names` tree.
pub(crate) fn linked() -> Vec<u8> {
    let link = |rect: &str, rest: &str| format!("<< /Type /Annot /Subtype /Link /Border [0 0 0] /Rect [{rect}] {rest} >>");
    let first = [
        "/Dest [8 0 R /XYZ 0 200 0]",
        "/A << /S /GoTo /D (chapter) >>",
        "/A << /S /URI /URI (https://example.com/a?b=c) >>",
        "/A << /S /GoToR /F (other.pdf) /D (intro) >>",
        "/A << /S /GoToR /F (other.pdf) /D [2 /Fit] >>",
        "/Dest [6 0 R /FitH 400]",
        "/Dest [6 0 R /FitV 300]",
        "/Dest [6 0 R /FitR 200 300 400 500]",
        "/Dest [6 0 R /Fit]",
        "/Dest [6 0 R /XYZ null null null]",
        "/Dest [4 0 R /FitR 72 500 300 700]",
        "/A << /S /JavaScript /JS (app.alert\\(1\\)) >>",
        "/A << /S /Launch /F (run.sh) >>",
    ];
    let mut annotations: Vec<String> =
        first.iter().enumerate().map(|(i, rest)| link(&format!("72 {} 272 {}", 700 - 40 * i, 720 - 40 * i), rest)).collect();
    let lines = "/QuadPoints [72 180 300 180 72 200 300 200 72 160 200 160 72 180 200 180] /Dest [8 0 R /Fit]";
    annotations.push(link("72 160 300 200", lines));
    let first = annotations.len();
    annotations.push(link("150 600 250 650", "/Dest [4 0 R /Fit]"));
    annotations.push(link("30 240 130 270", "/A << /S /URI /URI (mailto:someone@example.com) >>"));
    // The catalog, the page tree and the font; each page and its content; then the annotations.
    let numbers = |range: std::ops::Range<usize>| range.map(|n| format!("{} 0 R", 10 + n)).collect::<Vec<_>>().join(" ");
    let page = |boxes: &str, content: usize, annotations: String| {
        format!("<< /Type /Page /Parent 2 0 R {boxes} /Contents {content} 0 R /Resources << /Font << /F1 3 0 R >> >> /Annots [{annotations}] >>")
    };
    let content = |at: &str, number: usize| {
        let content = format!("BT /F1 24 Tf {at} Td (Links, page {number}) Tj ET");
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len())
    };
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /Names << /Dests << /Names [(chapter) [6 0 R /XYZ 300 400 null]] >> >> >>".to_string(),
        "<< /Type /Pages /Kids [4 0 R 6 0 R 8 0 R] /Count 3 >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        page("/MediaBox [0 0 612 792]", 5, numbers(0..first)),
        content("320 740", 1),
        page("/MediaBox [0 0 612 792] /CropBox [100 100 500 700] /Rotate 90", 7, numbers(first..first + 1)),
        content("120 400", 2),
        page("/MediaBox [0 0 300 300] /UserUnit 2", 9, numbers(first + 1..first + 2)),
        content("150 270", 3),
    ];
    objects.append(&mut annotations);
    pdf(&objects)
}

/// A US Letter page with three links, each to the page's own top: at its top left corner
/// (`/Rect [0 772 80 792]`), in its middle (`/Rect [280 400 360 420]`), and at its right edge, a
/// little lower (`/Rect [606 370 612 390]`).
pub(crate) fn linked_at_its_edges() -> Vec<u8> {
    let link = |rect: &str| format!("<< /Type /Annot /Subtype /Link /Border [0 0 0] /Rect [{rect}] /Dest [4 0 R /Fit] >>");
    let content = "BT /F1 24 Tf 200 600 Td (Links at the edges) Tj ET";
    pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 612 792] >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 3 0 R >> >> /Annots [6 0 R 7 0 R 8 0 R] >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        link("0 772 80 792"),
        link("280 400 360 420"),
        link("606 370 612 390"),
    ])
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
