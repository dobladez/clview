//! Word documents (docx, doc), OpenDocument text and RTF, read, laid out into pages and drawn by
//! wordcraft, as its app prints them: headers and footers at full strength, no comment balloons,
//! no proofing marks. A password opens a protected Word document.
//!
//! Opening reads and lays out the document once, to know it opens and its pages' sizes, and keeps
//! only where its bytes are: a renderer, or a pool, reads and lays it out again on its first
//! render and keeps that for the renders after, until it goes, as a drawing keeps its tree.
//!
//! A docx and an odt are zips: this backend is asked before the zip backend, and knows one by the
//! names of the files that start it, or a Word document by its own name.

use std::borrow::Cow;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use wordcraft_layout::{DocLayout, LayoutCache, LayoutOptions, ViewMode};
use wordcraft_render::RenderOptions;

use super::head::{COMPOUND, Names, ZIP, mimetype};
use super::threads::Threads;
use super::{Again, Backend, Format, Frame, Input, Opened, Pixels, Pool, Render, Reopenable, Request, Source, guarded};
use crate::args::{OptionSpec, Parsed};

/// Word documents, OpenDocument text and RTF, laid out and drawn by wordcraft.
pub(super) struct Wordcraft;

const OPTIONS: &[OptionSpec] =
    &[OptionSpec { name: "--password", short: None, value: Some("PW"), help: "the password of a protected Word document" }];

impl Backend for Wordcraft {
    fn name(&self) -> &'static str {
        "wordcraft"
    }

    fn shows(&self) -> &'static str {
        "documents"
    }

    fn formats(&self) -> Vec<Format> {
        vec![WORD, ODT, RTF]
    }

    fn options(&self) -> &'static [OptionSpec] {
        OPTIONS
    }

    fn opens(&self, path: &str, head: &[u8]) -> bool {
        Kind::of(path, head).is_some()
    }

    fn open(&self, input: Input, options: &Parsed<'_>) -> Result<Opened, String> {
        let path = input.path.clone();
        let error = |e: String| format!("{path}: {e}");
        let kind = Kind::of(&path, &input.head).ok_or_else(|| error("not a document this reads".into()))?;
        let password = options.value("--password").map(String::from);
        let read_as = |bytes: &[u8]| caught(|| laid_out(bytes, kind, password.as_deref(), &path));
        let (stored, laid) = match input.reopenable()? {
            Reopenable::Disk(file) => {
                let bytes = read(file).map_err(error)?;
                (Stored::Disk(PathBuf::from(&path)), read_as(&bytes))
            }
            Reopenable::Bytes(bytes) => {
                let bytes = within(bytes).map_err(error)?;
                let laid = read_as(&bytes);
                (Stored::Memory(bytes), laid)
            }
            Reopenable::Again(again) => {
                let bytes = again().and_then(within).map_err(error)?;
                (Stored::Held(again), read_as(&bytes))
            }
        };
        // Only the pages' sizes are kept: the document goes, and a render reads it again.
        let sizes = laid.map_err(error)?.layout.pages.iter().map(|page| (page.w, page.h)).collect();
        Ok(Opened::Pages(Box::new(Paged(Arc::new(Document { stored, kind, password, path, sizes })))))
    }
}

/// The largest file this reads: 256 MiB.
const MAX_FILE: u64 = 256 << 20;

/// The most pixels a page is drawn with: 64 million, 256 MiB.
const MAX_PIXELS: u64 = 64_000_000;

/// A Word document, by what it is named where its first bytes can't tell: a document, a
/// template, with macros or without, in either format, since Word opens each by what it holds.
const WORD: Format = Format { name: "Word", extensions: &["docx", "docm", "dotx", "dotm", "doc", "dot"] };

/// OpenDocument text, by its name where its first file doesn't say: a document or a template.
const ODT: Format = Format { name: "OpenDocument text", extensions: &["odt", "ott"] };

/// RTF, by its first bytes alone.
const RTF: Format = Format { name: "RTF", extensions: &[] };

/// What an OpenDocument text says it is, in its first file: a document or a template.
const ODT_TYPE: &[u8] = b"application/vnd.oasis.opendocument.text";

/// What kind of document a file is, which says how it is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A Word package (docx and the rest), a zip.
    Package,
    /// A compound file: a Word 97-2003 document, or a Word package protected with a password.
    Compound,
    /// OpenDocument text, a zip whose first file says so.
    Odt,
    Rtf,
}

impl Kind {
    /// The kind of the file at `path`, which starts with `head`, if this backend reads it. An RTF
    /// says so in its first bytes whatever its name (one is often saved as `.doc`), an odt in its
    /// first file. A Word package is a zip with a file in `word/` among the first, or named as a
    /// Word document; a compound file only when named so, since spreadsheets and slides are too.
    fn of(path: &str, head: &[u8]) -> Option<Self> {
        if head.starts_with(b"{\\rtf") {
            return Some(Self::Rtf);
        }
        if head.starts_with(ZIP) {
            if mimetype(head).is_some_and(|m| m.starts_with(ODT_TYPE)) || ODT.named(path) {
                return Some(Self::Odt);
            }
            let word = Names(head).any(|name| name.starts_with(b"word/"));
            return (word || WORD.named(path)).then_some(Self::Package);
        }
        (head.starts_with(COMPOUND) && WORD.named(path)).then_some(Self::Compound)
    }
}

/// An open document.
struct Paged(Arc<Document>);

struct Document {
    stored: Stored,
    kind: Kind,
    password: Option<String>,
    /// As given, for the fields that show the file's name.
    path: String,
    /// Each page's, in points.
    sizes: Vec<(f32, f32)>,
}

/// Where a document's bytes are.
enum Stored {
    /// In a file on disk, read again to render.
    Disk(PathBuf),
    /// As they came through a pipe, which can't be read again.
    Memory(Vec<u8>),
    /// In another file (an archive), read from it again to render.
    Held(Again),
}

/// A document read and laid out.
struct Laid {
    document: wordcraft_doc::Document,
    layout: DocLayout,
}

/// A document laid out, by the first render of a renderer or a pool, each of which has its own:
/// an error, or a panic's, is kept too, so a document that fails isn't read again for every
/// render.
type Cache = OnceLock<Result<Arc<Laid>, String>>;

impl Source for Paged {
    fn pages(&self) -> usize {
        self.0.sizes.len()
    }

    fn size(&self, page: usize) -> Option<(f32, f32)> {
        self.0.sizes.get(page).copied()
    }

    /// A page scales: fitted to the window, it fills it, as a PDF's does.
    fn natural(&self, _page: usize) -> bool {
        false
    }

    fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
        Ok(Box::new(Renderer { document: self.0.clone(), laid: Cache::new() }))
    }

    fn pool(&self, threads: usize) -> Box<dyn Pool> {
        let (document, laid) = (self.0.clone(), Arc::new(Cache::new()));
        Box::new(Threads::new(threads, Arc::new(move |request| document.render(request, &laid))))
    }
}

struct Renderer {
    document: Arc<Document>,
    laid: Cache,
}

impl Render for Renderer {
    fn render(&mut self, request: Request) -> Frame {
        guarded(request, || self.document.render(request, &self.laid))
    }
}

impl Document {
    fn render(&self, request: Request, laid: &Cache) -> Frame {
        self.rendered(request, laid).unwrap_or_else(|e| Frame::failed(request, e))
    }

    fn rendered(&self, request: Request, laid: &Cache) -> Result<Frame, String> {
        let laid = laid
            .get_or_init(|| {
                let bytes = self.bytes()?;
                caught(|| laid_out(&bytes, self.kind, self.password.as_deref(), &self.path)).map(Arc::new)
            })
            .clone()?;
        let page = laid.layout.pages.get(request.page).ok_or_else(|| format!("the document has no page {}", request.page.saturating_add(1)))?;
        let (w, h) = (pixels(page.w, request.scale)?, pixels(page.h, request.scale)?);
        let side = u64::from(wordcraft_render::MAX_SIDE);
        if w > side || h > side || w.saturating_mul(h) > MAX_PIXELS {
            return Err(format!("{w} × {h} pixels is too large to render"));
        }
        let mut options = RenderOptions::default();
        // A printed page: headers and footers dimmed only while the body is being edited.
        options.display.dim_header = false;
        let drawn = wordcraft_render::render_page(&laid.document, page, request.scale, &options);
        // Premultiplied RGBA, row by row, as a frame's are.
        Ok(Frame { request, width: drawn.width, height: drawn.height, rgba: Pixels::new(drawn.pixels), error: None, warnings: Vec::new() })
    }

    /// Its bytes: the ones kept, or the file's again.
    fn bytes(&self) -> Result<Cow<'_, [u8]>, String> {
        let again = match &self.stored {
            Stored::Memory(bytes) => return Ok(Cow::Borrowed(&bytes[..])),
            Stored::Disk(path) => File::open(path).map_err(|e| e.to_string()).and_then(read),
            Stored::Held(again) => again().and_then(within),
        };
        again.map(Cow::Owned).map_err(|e| format!("reading the file again: {e}"))
    }
}

/// `size` points at `scale`, in whole pixels, as wordcraft rounds them.
fn pixels(size: f32, scale: f32) -> Result<u64, String> {
    let scaled = (f64::from(size) * f64::from(scale)).ceil();
    if !(scale > 0.0 && scaled.is_finite() && scaled <= 1e15) {
        return Err(format!("can't render at scale {scale}"));
    }
    // In range: checked just above.
    Ok((scaled as u64).max(1))
}

/// The document in `bytes`, of `kind`, read with `password` if it is protected, and laid out into
/// pages. `path` is the file's, for the fields that show it.
fn laid_out(bytes: &[u8], kind: Kind, password: Option<&str>, path: &str) -> Result<Laid, String> {
    let read = match kind {
        Kind::Package => wordcraft_docx::read_with_password(bytes, password).map_err(|e| e.to_string()),
        Kind::Compound if wordcraft_docx::is_encrypted(bytes) => wordcraft_docx::read_with_password(bytes, password).map_err(|e| e.to_string()),
        Kind::Compound => wordcraft_docbin::read_with_password(bytes, password).map_err(|e| e.to_string()),
        Kind::Odt => imported("odt", bytes),
        Kind::Rtf => imported("rtf", bytes),
    };
    let mut document = read.map_err(|e| match e == wordcraft_docx::PASSWORD_REQUIRED {
        true => format!("{e}: give it with --password"),
        false => e,
    })?;
    document.ensure_nonempty();
    let name = Path::new(path).file_name().map_or_else(|| path.into(), |name| name.to_string_lossy());
    let options = LayoutOptions { view: ViewMode::Print, filename: Some(Arc::from(&*name)), filepath: Some(Arc::from(path)), ..Default::default() };
    let layout = wordcraft_layout::layout(&document, &mut LayoutCache::new(), &options);
    if layout.pages.is_empty() {
        return Err("the document has no pages to show".into());
    }
    Ok(Laid { document, layout })
}

/// The document in `bytes`, in the format `extension` names, through wordcraft's importers.
fn imported(extension: &str, bytes: &[u8]) -> Result<wordcraft_doc::Document, String> {
    let page = wordcraft_doc::SectionProps::default();
    wordcraft_formats::import_on(extension, bytes, &page).unwrap_or_else(|| Err(format!("wordcraft reads no .{extension}")))
}

/// What `read` returns, or an error if wordcraft panics: its bug costs the document, not the
/// viewer.
fn caught<T>(read: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|_| Err("wordcraft failed on this document".into()))
}

/// All of `file`, as long as it is no larger than [`MAX_FILE`].
fn read(file: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    within(bytes)
}

/// `bytes`, as long as they are no more than [`MAX_FILE`].
fn within(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    match u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE {
        true => Err(format!("the document is more than {} MiB", MAX_FILE >> 20)),
        false => Ok(bytes),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::super::head::zip_head;
    use super::*;

    const COMPOUND_HEAD: [u8; 16] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0, 0, 0, 0, 0, 0, 0];

    #[test]
    fn an_rtf_is_known_by_its_first_bytes_whatever_its_name() {
        for path in ["notes.rtf", "report.doc", "untitled"] {
            assert_eq!(Kind::of(path, b"{\\rtf1\\ansi Hello}"), Some(Kind::Rtf), "{path}");
        }
        assert_eq!(Kind::of("notes.rtf", b"plain text"), None, "named so, it isn't one");
    }

    #[test]
    fn a_word_package_is_a_zip_with_word_files_first_or_named_so() {
        let package =
            zip_head(&[("[Content_Types].xml", b"<Types/>"), ("_rels/.rels", b"<Relationships/>"), ("word/document.xml", b"<w:document/>")]);
        for path in ["a.docx", "a.zip", "a"] {
            assert_eq!(Kind::of(path, &package), Some(Kind::Package), "{path}");
        }
        let other = zip_head(&[("notes.txt", b"text")]);
        for path in ["a.docx", "a.DOCM", "a.dotx", "a.dotm", "a.doc", "a.dot"] {
            assert_eq!(Kind::of(path, &other), Some(Kind::Package), "{path}: named as one");
        }
        for path in ["a.zip", "a.xlsx", "a.epub", "docx"] {
            assert_eq!(Kind::of(path, &other), None, "{path}");
        }
    }

    #[test]
    fn an_odt_is_a_zip_whose_first_file_says_so() {
        let odt = zip_head(&[("mimetype", ODT_TYPE), ("content.xml", b"<office:document-content/>")]);
        assert_eq!((Kind::of("a.odt", &odt), Kind::of("a.zip", &odt)), (Some(Kind::Odt), Some(Kind::Odt)));
        let template = zip_head(&[("mimetype", b"application/vnd.oasis.opendocument.text-template")]);
        assert_eq!(Kind::of("a.ott", &template), Some(Kind::Odt));
        let sheet = zip_head(&[("mimetype", b"application/vnd.oasis.opendocument.spreadsheet")]);
        assert_eq!(Kind::of("a.ods", &sheet), None);
        let epub = zip_head(&[("mimetype", b"application/epub+zip")]);
        assert_eq!(Kind::of("a.epub", &epub), None);
    }

    #[test]
    fn a_compound_file_is_a_word_document_only_when_named_so() {
        for path in ["a.doc", "a.DOT", "a.docx"] {
            assert_eq!(Kind::of(path, &COMPOUND_HEAD), Some(Kind::Compound), "{path}");
        }
        for path in ["a.xls", "a.ppt", "a.msg", "a"] {
            assert_eq!(Kind::of(path, &COMPOUND_HEAD), None, "{path}");
        }
    }

    #[test]
    fn a_page_is_drawn_with_its_size_at_the_scale_rounded_up() {
        assert_eq!(pixels(612.0, 1.0), Ok(612));
        assert_eq!(pixels(200.0, 1.005), Ok(201));
        assert_eq!(pixels(0.0, 2.0), Ok(1), "at least one pixel");
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(pixels(612.0, scale).is_err(), "{scale}");
        }
        assert!(pixels(f32::MAX, f32::MAX).is_err());
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
