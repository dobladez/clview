//! Spreadsheets (xlsx and its kin, xls, xlsb, ods, csv), as gridcraft prints them: each visible
//! sheet's print pages, laid out by its print area, page setup, title rows, headers and footers,
//! gridlines and charts, its formulas recalculated first, as gridcraft's app does when it opens
//! one. gridcraft writes those pages as a PDF, which the backend that opens PDFs then shows: this
//! one lays out the pages but draws none of them, and holds nothing of the workbook once printed.
//! A password opens a protected workbook.
//!
//! An xlsx and an ods are zips: this backend is asked before the zip backend, and knows one by the
//! files that start it, or by its name.

use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::Arc;

use gridcraft_engine::calc::Calc;
use gridcraft_engine::cmd::print::{PrintOptions, layouts, render_pdf};
use gridcraft_engine::core::{RangeRef, Value};
use gridcraft_engine::model::Workbook;
use gridcraft_engine::{DocState, Session, Trust};

use super::head::{COMPOUND, Names, ZIP, mimetype};
use super::{Again, BACKENDS, Backend, Deferred, Format, Frame, HEAD, Input, Opened, Pool, Render, Reopenable, Request, Source, Texts};
use crate::args::{OptionSpec, Parsed};

/// Spreadsheets, printed by gridcraft.
pub(super) struct Gridcraft;

const OPTIONS: &[OptionSpec] =
    &[OptionSpec { name: "--password", short: None, value: Some("PW"), help: "the password of a protected workbook", alias: None }];

impl Backend for Gridcraft {
    fn name(&self) -> &'static str {
        "gridcraft"
    }

    fn shows(&self) -> &'static str {
        "spreadsheets"
    }

    fn formats(&self) -> Vec<Format> {
        vec![EXCEL, ODS, TEXT]
    }

    fn credits(&self) -> &[&'static str] {
        &["gridcraft"]
    }

    fn options(&self) -> &'static [OptionSpec] {
        OPTIONS
    }

    fn opens(&self, path: &str, head: &[u8]) -> bool {
        Kind::of(path, head).is_some()
    }

    /// It prints the whole workbook to open it.
    fn deferred(&self, _path: &str, _head: &[u8]) -> Option<Deferred> {
        Some(Deferred { settings: &[] })
    }

    fn open(&self, input: Input, options: &Parsed<'_>) -> Result<Opened, String> {
        let path = input.path.clone();
        let error = |e: String| format!("{path}: {e}");
        let kind = Kind::of(&path, &input.head).ok_or_else(|| error("not a spreadsheet this reads".into()))?;
        let bytes = match input.reopenable()? {
            Reopenable::Disk(file) => read(file),
            Reopenable::Bytes(bytes) => within(bytes),
            Reopenable::Again(again) => again().and_then(within),
        };
        let bytes = bytes.map_err(error)?;
        if kind == Kind::Package {
            checked_sheets(&bytes, MAX_SHEETS).map_err(error)?;
        }
        let Printed { pdf, notes } = caught(|| printed(&path, &bytes, kind, options.value("--password"))).map_err(error)?;
        drop(bytes);
        let head = pdf.get(..HEAD as usize).unwrap_or(&pdf).to_vec();
        let pdf = Arc::new(pdf);
        let again: Again = Arc::new(move || Ok(pdf.to_vec()));
        let printed = Input::held(path, head, again);
        // Not this one: what it prints is never a spreadsheet.
        let Some(backend) = BACKENDS.iter().filter(|b| b.name() != self.name()).find(|b| b.opens(&printed.path, &printed.head)) else {
            return Err(format!("{}: a spreadsheet is shown as the PDF it prints to, and no backend opens that", printed.path));
        };
        match backend.open(printed, options)? {
            Opened::Pages(pages) if !notes.is_empty() => Ok(Opened::Pages(Box::new(Noted { pages, notes: notes.into() }))),
            opened => Ok(opened),
        }
    }
}

/// The largest file this reads: 64 MiB. A workbook takes far more once read (about 180 bytes a
/// cell), and a CSV is all cells.
const MAX_FILE: u64 = 64 << 20;

/// The most an xlsx's sheets may unpack to, together: 128 MiB of XML, which gridcraft would hold
/// in some 850 MB (97 MiB took 649 MB). Its own limits are each sheet's.
const MAX_SHEETS: u64 = 128 << 20;

/// What an OpenDocument spreadsheet says it is, in its first file: a spreadsheet or a template.
const ODS_TYPE: &[u8] = b"application/vnd.oasis.opendocument.spreadsheet";

/// An Excel workbook, by what it is named where its first bytes can't tell: a workbook, a
/// template, with macros or without, in the format of 2007 on, its binary kind, and that of
/// 97-2003.
const EXCEL: Format = Format::of("Excel", &["xlsx", "xlsm", "xltx", "xltm", "xlsb", "xls", "xlt"]);

/// An OpenDocument spreadsheet, by its name where its first file doesn't say: a spreadsheet or a
/// template.
const ODS: Format = Format::of("OpenDocument spreadsheet", &["ods", "ots"]);

/// Values separated by commas or tabs, by their name alone.
const TEXT: Format = Format::of("CSV", &["csv", "tsv", "tab"]);

/// What kind of spreadsheet a file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// An Excel package (xlsx, xlsb and the rest), a zip.
    Package,
    /// An OpenDocument spreadsheet, a zip whose first file says so.
    Ods,
    /// A compound file: an Excel 97-2003 workbook, or an Excel package protected with a password.
    Compound,
    /// Values separated by commas or tabs.
    Text,
}

impl Kind {
    /// The kind of the file at `path`, which starts with `head`, if this backend reads it. An ods
    /// says so in its first file; an Excel package is a zip with a file in `xl/` among the first,
    /// or named as a workbook; a compound file only when named so, since documents and slides are
    /// too; and values separated by commas or tabs only when named so, and text.
    fn of(path: &str, head: &[u8]) -> Option<Self> {
        if head.starts_with(ZIP) {
            if mimetype(head).is_some_and(|m| m.starts_with(ODS_TYPE)) || ODS.named(path) {
                return Some(Self::Ods);
            }
            let excel = Names(head).any(|name| name.starts_with(b"xl/"));
            return (excel || EXCEL.named(path)).then_some(Self::Package);
        }
        if head.starts_with(COMPOUND) {
            return EXCEL.named(path).then_some(Self::Compound);
        }
        // UTF-16 text has a NUL in every other byte, after its byte order mark.
        let text = head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) || !head.contains(&0);
        (text && TEXT.named(path)).then_some(Self::Text)
    }
}

/// A workbook printed: the PDF, and what gridcraft said on reading it that the reader should
/// know (a sheet cut short at one of its limits).
struct Printed {
    pdf: Vec<u8>,
    notes: Vec<String>,
}

/// The workbook in `bytes`, of `kind`, read from `path` (whose extension says how gridcraft reads
/// a CSV or a TSV) with `password` if it is protected, its formulas recalculated, and each visible
/// sheet's print pages written as a PDF.
fn printed(path: &str, bytes: &[u8], kind: Kind, password: Option<&str>) -> Result<Printed, String> {
    let (mut workbook, said) = gridcraft_engine::io::open_bytes_with(path, bytes, password, false).map_err(|e| reworded(path, &e.to_string()))?;
    let workbook = match kind {
        Kind::Text => fitted(workbook)?,
        _ => {
            Calc::new().recalc_all(&mut workbook);
            Arc::new(workbook)
        }
    };
    let laid = layouts(&workbook, &PrintOptions { all_sheets: true, range: None, fit_to_page: None });
    if laid.iter().all(|sheet| sheet.pages.is_empty()) {
        return Err("the workbook has nothing to print".into());
    }
    let name = Path::new(path).file_name().map_or_else(|| path.into(), |name| name.to_string_lossy());
    let (pdf, pages) = render_pdf(&workbook, &laid, &name, path);
    if pages == 0 {
        return Err("the workbook has nothing to print".into());
    }
    let mut notes: Vec<String> = said.iter().filter_map(|note| for_reader(note)).collect();
    if blanked(&workbook) {
        notes.push(BLANKED.into());
    }
    Ok(Printed { pdf, notes })
}

/// What a page says when some of the workbook's text is left blank ([`blanked`]).
const BLANKED: &str =
    "text in letters other than Western European ones (Greek, Cyrillic, Chinese…) is left blank: the PDF gridcraft prints to has no font for them";

/// Whether some of `workbook`'s text is left blank in its pages: gridcraft's PDF sets what the
/// standard PDF fonts' encoding (Windows-1252) lacks in a Chinese font it doesn't carry, which the
/// PDF backend doesn't substitute, so that text prints as nothing, with no warning of its own.
fn blanked(workbook: &Workbook) -> bool {
    let drawn = |c: char| matches!(c, ' '..='~' | '\u{A0}'..='\u{FF}' | '\t' | '\n' | '\r') || "€‚ƒ„…†‡ˆ‰Š‹ŒŽ‘’“”•–—˜™š›œžŸ".contains(c);
    let text = |value: &Value| matches!(value, Value::Text(text) if !text.chars().all(drawn));
    workbook.sheets.iter().any(|sheet| sheet.cells.iter().any(|(_, cell)| text(&cell.value)))
}

/// What gridcraft said on reading a workbook, `note`, as the reader of its pages should hear it:
/// what its app tells an editor about saving left out, and what a page leaves out said plainly.
fn for_reader(note: &str) -> Option<String> {
    // Saved again, it would be saved without its password.
    if note == gridcraft_engine::io::DECRYPTED_NOTICE {
        return None;
    }
    if note.starts_with("ODS data-only import") {
        return Some("an ods is shown by its values: its formatting, charts and pictures are left out".into());
    }
    if note.starts_with("XLSB data-only import") {
        return Some("an xlsb is shown by its values: its formatting, charts and pictures are left out, and dates, amounts and percentages may show as plain numbers".into());
    }
    Some(note.into())
}

/// `workbook`, read from values separated by commas or tabs, its formulas calculated and its
/// columns as wide as what they hold, as gridcraft's app makes them when it imports such a file:
/// at the width every column starts with, a date shows as `#######`, and text is cut off.
fn fitted(workbook: Workbook) -> Result<Arc<Workbook>, String> {
    let used = workbook.sheet(0).and_then(|sheet| sheet.used_range());
    let mut session = Session::new();
    // From a file, so nothing in it runs; made, it is calculated.
    session.add_document(DocState::new(workbook, None, String::new(), Trust::FromFile));
    if let Some(used) = used {
        let columns = RangeRef::cols(used.start.col, used.end.col).a1();
        session.execute("home.autofitColumnWidth", serde_json::json!({ "cols": columns })).map_err(|e| e.to_string())?;
    }
    session.doc().map(|document| document.wb.clone()).map_err(|e| e.to_string())
}

/// gridcraft's message `message` about the file at `path`, in clv's words where they differ: the
/// path said once, and the option that gives a password.
fn reworded(path: &str, message: &str) -> String {
    if message.contains(gridcraft_engine::io::PASSWORD_REQUIRED) {
        return "this workbook is protected with a password: give it with --password".into();
    }
    if message.contains(gridcraft_engine::io::WRONG_PASSWORD) {
        return "the password is incorrect".into();
    }
    let name = Path::new(path).file_name().map_or_else(|| path.into(), |name| name.to_string_lossy());
    for said in [format!("We can't open '{name}': "), format!("We can't import '{name}': "), format!("We can't open '{path}': ")] {
        if let Some(rest) = message.strip_prefix(&said) {
            return rest.into();
        }
    }
    message.into()
}

/// Refuses an Excel package whose sheets unpack to more than `max` bytes together ([`MAX_SHEETS`]),
/// by what its central directory says, before gridcraft reads them.
fn checked_sheets(bytes: &[u8], max: u64) -> Result<(), String> {
    let Ok(mut package) = ::zip::ZipArchive::new(Cursor::new(bytes)) else {
        // gridcraft says what is wrong with it.
        return Ok(());
    };
    let mut total = 0u64;
    for at in 0..package.len() {
        if let Ok(file) = package.by_index_raw(at)
            && file.name().is_ok_and(|name| name.starts_with("xl/worksheets/"))
        {
            total = total.saturating_add(file.size());
        }
    }
    match total > max {
        true => Err(format!("its sheets unpack to {} MiB, more than the {} MiB {} reads", total >> 20, max >> 20, crate::NAME)),
        false => Ok(()),
    }
}

/// A spreadsheet's printed pages, as the backend that opens PDFs opened them, with what gridcraft
/// said on reading it: said with the first page, as a page's own warnings are.
struct Noted {
    pages: Box<dyn Source + Send>,
    notes: Arc<[String]>,
}

impl Source for Noted {
    fn pages(&self) -> usize {
        self.pages.pages()
    }

    fn size(&self, page: usize) -> Option<(f32, f32)> {
        self.pages.size(page)
    }

    fn natural(&self, page: usize) -> bool {
        self.pages.natural(page)
    }

    fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
        Ok(Box::new(NotedRender { render: self.pages.renderer()?, notes: self.notes.clone() }))
    }

    fn pool(&self, threads: usize) -> Box<dyn Pool> {
        Box::new(NotedPool { pool: self.pages.pool(threads), notes: self.notes.clone() })
    }

    /// The printed pages' own: a workbook is searched as the PDF it prints to.
    fn texts(&self) -> Option<Box<dyn Texts>> {
        self.pages.texts()
    }
}

struct NotedRender<'a> {
    render: Box<dyn Render + 'a>,
    notes: Arc<[String]>,
}

impl Render for NotedRender<'_> {
    fn render(&mut self, request: Request) -> Frame {
        noted(self.render.render(request), &self.notes)
    }
}

struct NotedPool {
    pool: Box<dyn Pool>,
    notes: Arc<[String]>,
}

impl Pool for NotedPool {
    fn set_queue(&self, requests: Vec<Request>) {
        self.pool.set_queue(requests);
    }

    fn try_recv(&self) -> Option<Frame> {
        self.pool.try_recv().map(|frame| noted(frame, &self.notes))
    }
}

/// `frame`, with `notes` first among its warnings if it is the first page.
fn noted(mut frame: Frame, notes: &[String]) -> Frame {
    if frame.request.page == 0 {
        frame.warnings.splice(0..0, notes.iter().cloned());
    }
    frame
}

/// What `read` returns, or an error if gridcraft panics: its bug costs the workbook, not the
/// viewer.
fn caught<T>(read: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|_| Err("gridcraft failed on this workbook".into()))
}

/// All of `file`, as long as it is no larger than [`MAX_FILE`].
fn read(file: File) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    within(bytes)
}

/// `bytes`, as long as they are no more than [`MAX_FILE`].
fn within(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    match u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE {
        true => Err(format!("the workbook is more than {} MiB", MAX_FILE >> 20)),
        false => Ok(bytes),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use ::zip::write::SimpleFileOptions;
    use ::zip::{CompressionMethod, ZipWriter};
    use proptest::prelude::*;

    use super::super::head::zip_head;
    use super::*;

    const COMPOUND_HEAD: [u8; 16] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0, 0, 0, 0, 0, 0, 0];

    #[test]
    fn an_excel_package_is_a_zip_with_xl_files_first_or_named_so() {
        let package = zip_head(&[("[Content_Types].xml", b"<Types/>"), ("_rels/.rels", b"<Relationships/>"), ("xl/workbook.xml", b"<workbook/>")]);
        for path in ["a.xlsx", "a.zip", "a"] {
            assert_eq!(Kind::of(path, &package), Some(Kind::Package), "{path}");
        }
        let other = zip_head(&[("notes.txt", b"text")]);
        for path in ["a.xlsx", "a.XLSM", "a.xltx", "a.xltm", "a.xlsb", "a.xls"] {
            assert_eq!(Kind::of(path, &other), Some(Kind::Package), "{path}: named as one");
        }
        for path in ["a.zip", "a.docx", "a.epub", "xlsx"] {
            assert_eq!(Kind::of(path, &other), None, "{path}");
        }
    }

    #[test]
    fn an_ods_is_a_zip_whose_first_file_says_so() {
        let ods = zip_head(&[("mimetype", ODS_TYPE), ("content.xml", b"<office:document-content/>")]);
        assert_eq!((Kind::of("a.ods", &ods), Kind::of("a.zip", &ods)), (Some(Kind::Ods), Some(Kind::Ods)));
        let template = zip_head(&[("mimetype", b"application/vnd.oasis.opendocument.spreadsheet-template")]);
        assert_eq!(Kind::of("a.ots", &template), Some(Kind::Ods));
        let text = zip_head(&[("mimetype", b"application/vnd.oasis.opendocument.text")]);
        assert_eq!(Kind::of("a.odt", &text), None);
    }

    #[test]
    fn a_compound_file_is_a_workbook_only_when_named_so() {
        for path in ["a.xls", "a.XLT", "a.xlsx", "a.xlsb"] {
            assert_eq!(Kind::of(path, &COMPOUND_HEAD), Some(Kind::Compound), "{path}");
        }
        for path in ["a.doc", "a.ppt", "a.msg", "a"] {
            assert_eq!(Kind::of(path, &COMPOUND_HEAD), None, "{path}");
        }
    }

    #[test]
    fn values_separated_by_commas_or_tabs_are_text_named_so() {
        for path in ["a.csv", "a.TSV", "a.tab"] {
            assert_eq!(Kind::of(path, b"date,item\n2026-03-01,widget\n"), Some(Kind::Text), "{path}");
        }
        assert_eq!(Kind::of("a.csv", &[0xFF, 0xFE, b'a', 0, b',', 0]), Some(Kind::Text), "UTF-16, by its byte order mark");
        assert_eq!(Kind::of("a.csv", b"PNG\0\0\0"), None, "binary");
        for path in ["a.txt", "a.json", "a"] {
            assert_eq!(Kind::of(path, b"date,item\n"), None, "{path}");
        }
    }

    /// A zip of `files`, each `size` bytes of zeros, deflated.
    fn zip_of(files: &[(&str, usize)]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, size) in files {
            zip.start_file(*name, SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)).unwrap();
            zip.write_all(&vec![0; *size]).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn an_excel_packages_sheets_are_bounded_together_before_they_are_read() {
        let package = zip_of(&[("xl/workbook.xml", 5000), ("xl/worksheets/sheet1.xml", 3000), ("xl/worksheets/sheet2.xml", 3000)]);
        assert_eq!(checked_sheets(&package, 6000), Ok(()), "the sheets' 6000 bytes, not the workbook's");
        let refused = checked_sheets(&package, 5999).unwrap_err();
        assert!(refused.contains("its sheets unpack to"), "{refused}");
        assert_eq!(checked_sheets(b"not a zip", 0), Ok(()), "gridcraft says what is wrong with it");
    }

    #[test]
    fn a_csvs_columns_are_as_wide_as_what_they_hold() {
        let csv = b"date,note\n2026-03-01,a note much longer than a column starts\n";
        let (workbook, _) = gridcraft_engine::io::open_bytes_with("a.csv", csv, None, false).unwrap();
        let start = workbook.sheet(0).unwrap().col_width(1);
        let fitted = fitted(workbook).unwrap();
        let sheet = fitted.sheet(0).unwrap();
        assert!(sheet.col_width(1) > start * 2.0, "the note's column: {} from {start}", sheet.col_width(1));
        assert!(sheet.col_width(0) > 0.0);
    }

    #[test]
    fn text_its_pdf_leaves_blank_is_said() {
        let notes = |csv: &str| printed("a.csv", csv.as_bytes(), Kind::Text, None).unwrap().notes;
        assert_eq!(notes("word\naño ñandú\n“quoted” – €5\n"), Vec::<String>::new(), "Windows-1252 is drawn");
        for word in ["καλημέρα", "привет", "Łódź", "你好", "≤ ∞"] {
            assert_eq!(notes(&format!("word\nhello\n{word}\n")), [BLANKED], "{word}");
        }
    }

    #[test]
    fn what_gridcraft_tells_an_editor_is_told_a_reader_as_a_reader_needs_it() {
        assert!(for_reader("XLSB data-only import: …").is_some_and(|note| note.starts_with("an xlsb is shown by its values")));
        assert_eq!(for_reader("sheet \"Data\" is damaged; kept 37036 rows").as_deref(), Some("sheet \"Data\" is damaged; kept 37036 rows"));
    }

    #[test]
    fn gridcraft_saying_it_cant_open_a_file_says_only_why() {
        assert_eq!(reworded("dir/a.xlsx", "We can't open 'a.xlsx': no workbook part"), "no workbook part");
        assert_eq!(reworded("a.xlsx", "something else"), "something else");
    }

    proptest! {
        /// Whatever a file starts with, and whatever its name, choosing never panics.
        #[test]
        fn any_first_bytes_are_safe(head in proptest::collection::vec(any::<u8>(), 0..1024), zip in any::<bool>(), name in "[a-z.]{0,8}") {
            let head = if zip { [ZIP, &head].concat() } else { head };
            let _ = Kind::of(&name, &head);
        }
    }
}
