//! What the viewer shows: a source of pages, and the backends that open and render them. The
//! pager and the printer in `view` see only the traits here; each backend brings its own renderer
//! and its own threads, so one can cancel or time out work the way it knows how.

mod dir;
mod folder;
#[cfg(feature = "image")]
mod image;
mod joined;
#[cfg(feature = "pdf")]
mod pdfcraft;
mod threads;

use std::fs::File;
use std::io::{Read, Seek};
use std::ops::Deref;
use std::rc::Rc;

use crate::args::{OptionSpec, Parsed};
pub(crate) use dir::Directory;
pub(crate) use folder::Folder;
pub(crate) use joined::{Joined, Part};

/// The backends this build has, in the order they are asked whether they open a file.
pub(crate) const BACKENDS: &[&dyn Backend] = &[
    #[cfg(feature = "pdf")]
    &pdfcraft::Pdfcraft,
    #[cfg(feature = "image")]
    &image::Images,
];

/// How much of a file a backend sees to decide whether it opens it: a PDF's header may come
/// anywhere in its first 1024 bytes.
const HEAD: u64 = 1024;

/// A kind of file the viewer shows, and how it opens one.
pub(crate) trait Backend {
    /// Its name, as messages and `--help` give it.
    fn name(&self) -> &'static str;
    /// What it shows, for `--help` and for a file no backend opens: "PDFs".
    fn shows(&self) -> &'static str;
    /// The options it takes, beyond the viewer's own.
    fn options(&self) -> &'static [OptionSpec];
    /// Whether it opens the file at `path`, going by `head`: its first [`HEAD`] bytes, or all of
    /// a shorter file.
    fn opens(&self, path: &str, head: &[u8]) -> bool;
    /// `input` opened, with the values `options` gives its own options.
    fn open(&self, input: Input, options: &Parsed<'_>) -> Result<Box<dyn Source>, String>;
}

/// A file being opened: its first bytes, read to choose a backend, and the rest still to read. A
/// file is read once, so a pipe (`/dev/stdin`) opens too.
pub(crate) struct Input {
    pub(crate) path: String,
    head: Vec<u8>,
    rest: File,
}

impl Input {
    fn open(path: &str) -> Result<Self, String> {
        let error = |e: std::io::Error| format!("{path}: {e}");
        let mut rest = File::open(path).map_err(error)?;
        let mut head = Vec::new();
        Read::by_ref(&mut rest).take(HEAD).read_to_end(&mut head).map_err(error)?;
        Ok(Self { path: path.to_owned(), head, rest })
    }

    /// The whole file.
    pub(crate) fn bytes(self) -> Result<Vec<u8>, String> {
        let Self { path, mut head, mut rest } = self;
        rest.read_to_end(&mut head).map_err(|e| format!("{path}: {e}"))?;
        Ok(head)
    }

    /// The file for a backend that reads it again when it renders, and so keeps only what it
    /// needs meanwhile: on disk, from its start; otherwise the whole of it, read now.
    #[cfg_attr(all(not(feature = "image"), not(test)), expect(dead_code, reason = "only images read their file again"))]
    pub(crate) fn reopenable(self) -> Result<Reopenable, String> {
        if !self.rest.metadata().is_ok_and(|m| m.is_file()) {
            return self.bytes().map(Reopenable::Bytes);
        }
        let Self { path, mut rest, .. } = self;
        rest.rewind().map_err(|e| format!("{path}: {e}"))?;
        Ok(Reopenable::Disk(rest))
    }
}

/// A file as [`Input::reopenable`] gives it.
#[cfg_attr(not(feature = "image"), expect(dead_code, reason = "only images read their file again"))]
pub(crate) enum Reopenable {
    /// On disk, from its start: its path opens it again.
    Disk(File),
    /// A pipe or a device, which can't be read again: all of it.
    Bytes(Vec<u8>),
}

/// `path` opened, with the first of `backends` that opens it.
pub(crate) fn choose<'b>(path: &str, backends: &[&'b dyn Backend]) -> Result<(&'b dyn Backend, Input), String> {
    if backends.is_empty() {
        return Err(format!("{path}: this {} was built without backends (cargo features such as `pdf`), so it shows nothing", crate::NAME));
    }
    let input = Input::open(path)?;
    Ok((pick(backends, path, &input.head)?, input))
}

/// `input`, with the first of `backends` that opens it, or `None` when none does: for a
/// container's files, where what nothing shows is no error.
pub(crate) fn shown_by<'b>(input: Input, backends: &[&'b dyn Backend]) -> Option<(&'b dyn Backend, Input)> {
    let backend = backends.iter().find(|backend| backend.opens(&input.path, &input.head)).copied();
    backend.map(|backend| (backend, input))
}

/// Files held together, shown as a list: a directory's. Each file is opened from it, and each
/// container it holds is listed from that one, when the reader goes into it.
pub(crate) trait Container {
    /// Where it is, as messages give it: a directory's path.
    fn path(&self) -> &str;
    /// What it holds to show, in order: its files and the containers in it, or with `recurse`,
    /// those containers' files at their place instead.
    fn list(&self, recurse: bool) -> Result<Listing, String>;
    /// The file it listed as `name`, opened.
    fn open(&self, name: &str) -> Result<Input, String>;
}

impl std::fmt::Debug for dyn Container + '_ {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Container({:?})", self.path())
    }
}

/// What a container holds to show.
#[derive(Debug, Default)]
pub(crate) struct Listing {
    /// Each file, and each container in it unless recursing, in order.
    pub(crate) entries: Vec<Listed>,
    /// What couldn't be read, by its name in the container, and why: a subdirectory, a name that
    /// isn't UTF-8.
    pub(crate) unread: Vec<(String, String)>,
    /// Whether the listing stopped at one of the container's limits before the end.
    pub(crate) cut: bool,
}

/// A file or a container listed.
#[derive(Debug)]
pub(crate) struct Listed {
    /// In the container listed: `2024/beach.jpg`, recursing. A file opens by it.
    pub(crate) name: String,
    /// A container's own, to list; `None` for a file.
    pub(crate) container: Option<Rc<dyn Container>>,
}

/// The first of `backends` that opens the file at `path`, which starts with `head`.
fn pick<'b>(backends: &[&'b dyn Backend], path: &str, head: &[u8]) -> Result<&'b dyn Backend, String> {
    if let Some(backend) = backends.iter().find(|backend| backend.opens(path, head)) {
        return Ok(*backend);
    }
    let mut shown: Vec<&str> = Vec::new();
    for backend in backends {
        if !shown.contains(&backend.shows()) {
            shown.push(backend.shows());
        }
    }
    let list = match shown.split_last() {
        Some((last, [])) => (*last).to_string(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => "nothing".into(),
    };
    Err(format!("{path}: not a file {} shows (it shows {list})", crate::NAME))
}

/// A render asked of a source: `page` (0-based) at `scale` output pixels per unit of its
/// [`Source::size`]. `tag` tells the result apart from others of the same page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Request {
    pub(crate) page: usize,
    pub(crate) scale: f32,
    pub(crate) tag: u64,
}

/// Premultiplied RGBA, row by row, top to bottom: a backend's own buffer, read as bytes, so its
/// pixels reach the terminal without a copy.
pub(crate) struct Pixels(Box<dyn AsRef<[u8]> + Send>);

impl Pixels {
    pub(crate) fn new(pixels: impl AsRef<[u8]> + Send + 'static) -> Self {
        Self(Box::new(pixels))
    }
}

impl Deref for Pixels {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        (*self.0).as_ref()
    }
}

/// A rendered page, or why there is none.
pub(crate) struct Frame {
    pub(crate) request: Request,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Pixels,
    pub(crate) error: Option<String>,
    /// What the reader should know about the page, each once: content left out, a colour that
    /// had to be guessed. In words for the reader, without the page number.
    pub(crate) warnings: Vec<String>,
}

impl Frame {
    /// No page for `request`, and why.
    pub(crate) fn failed(request: Request, error: String) -> Self {
        Self { request, width: 0, height: 0, rgba: Pixels::new(Vec::new()), error: Some(error), warnings: Vec::new() }
    }
}

/// What `render` returns, or a failed page if it panics: a backend's bug costs the page, not the
/// viewer.
pub(crate) fn guarded(request: Request, render: impl FnOnce() -> Frame) -> Frame {
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(render));
    caught.unwrap_or_else(|_| Frame::failed(request, "the renderer failed on this page".into()))
}

/// Something with pages to show: a PDF's pages, or an image as one page.
pub(crate) trait Source {
    /// How many pages: at least one, since a source with none fails to open.
    fn pages(&self) -> usize;
    /// Page `page`'s size as shown, in the source's own unit (points for a PDF), or `None` past
    /// the last page. A render at scale `s` is about `s` times as many pixels on each side.
    fn size(&self, page: usize) -> Option<(f32, f32)>;
    /// Whether page `page`'s size is its natural size, in pixels of its own (an image's): fitted
    /// to the window, it is never drawn larger, unless the reader zooms to the width. A PDF's
    /// points are a unit for any scale.
    fn natural(&self, page: usize) -> bool;
    /// A renderer on this thread, for pages printed one after another.
    fn renderer(&self) -> Result<Box<dyn Render + '_>, String>;
    /// Renders on threads of their own, for the pager: keys still work while pages render, and
    /// a page that takes too long is given up on. The pool holds what it needs of the source,
    /// so the pager can keep both.
    fn pool(&self, threads: usize) -> Box<dyn Pool>;
}

/// Renders on the caller's thread.
pub(crate) trait Render {
    fn render(&mut self, request: Request) -> Frame;
}

/// Renders on threads of the pool's own, the most urgent first.
pub(crate) trait Pool {
    /// Replace the renders wanted, the most urgent first. A result is delivered only while its
    /// request is in the latest queue: one still waited for must be listed again every time.
    fn set_queue(&self, requests: Vec<Request>);
    /// A finished render, if one is ready. Never blocks.
    fn try_recv(&self) -> Option<Frame>;
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The viewer knows sources only through the items here: nothing in `src/view/` reaches into
    /// a module of `src/source/`, a backend or the threads lent to one. (What a backend renders
    /// with is named only by the backend and its own tests, which check it.)
    #[test]
    fn the_viewer_names_no_backend() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let modules: Vec<String> = std::fs::read_dir(root.join("src/source"))
            .unwrap()
            .filter_map(|e| e.unwrap().path().file_stem().map(|s| s.to_string_lossy().into_owned()))
            .collect();
        assert!(modules.len() >= 3, "{modules:?}");
        let dir = root.join("src/view");
        let mut read = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                for module in &modules {
                    assert!(!text.contains(&format!("source::{module}")), "{} names source::{module}", path.display());
                }
                read += 1;
            }
        }
        assert!(read >= 9, "read {read} files in {}", dir.display());
    }

    #[test]
    fn pixels_read_a_backends_buffer_as_bytes() {
        assert_eq!(&*Pixels::new(vec![1u8, 2, 3, 4]), &[1, 2, 3, 4]);
        assert_eq!(&*Pixels::new([9u8; 3]), &[9, 9, 9]);
        assert!(Pixels::new(Vec::new()).is_empty());
    }

    /// A backend that opens files starting with `magic`.
    pub(crate) struct Fake {
        pub(crate) name: &'static str,
        pub(crate) shows: &'static str,
        pub(crate) magic: &'static [u8],
        pub(crate) options: &'static [OptionSpec],
    }

    impl Backend for Fake {
        fn name(&self) -> &'static str {
            self.name
        }
        fn shows(&self) -> &'static str {
            self.shows
        }
        fn options(&self) -> &'static [OptionSpec] {
            self.options
        }
        fn opens(&self, _path: &str, head: &[u8]) -> bool {
            head.starts_with(self.magic)
        }
        fn open(&self, input: Input, _options: &Parsed<'_>) -> Result<Box<dyn Source>, String> {
            Err(format!("{} opened by {}", input.path, self.name))
        }
    }

    pub(crate) const PDF: Fake = Fake { name: "pdf-one", shows: "PDFs", magic: b"%PDF-", options: &[] };
    const PDF_TOO: Fake = Fake { name: "pdf-two", shows: "PDFs", magic: b"%PDF-", options: &[] };
    const PNG: Fake = Fake { name: "png", shows: "images", magic: b"\x89PNG", options: &[] };
    const DIR: Fake = Fake { name: "dir", shows: "directories", magic: b"\0dir", options: &[] };

    #[test]
    fn the_first_backend_that_opens_a_file_opens_it() {
        let backends: [&dyn Backend; 3] = [&PNG, &PDF, &PDF_TOO];
        assert_eq!(pick(&backends, "a.pdf", b"%PDF-1.7\n").map(|b| b.name()), Ok("pdf-one"));
        assert_eq!(pick(&backends, "a.png", b"\x89PNG\r\n").map(|b| b.name()), Ok("png"));
        assert_eq!(pick(&[&PDF_TOO, &PDF], "a.pdf", b"%PDF-2.0").map(|b| b.name()), Ok("pdf-two"));
    }

    #[test]
    fn a_directorys_file_that_no_backend_opens_is_no_error() {
        let path = std::env::temp_dir().join(format!("clview-shown-by-{}", std::process::id()));
        std::fs::write(&path, b"hello").unwrap();
        let path = path.to_str().unwrap();
        assert!(shown_by(Input::open(path).unwrap(), &[&PDF, &PNG]).is_none());
        std::fs::write(path, b"\x89PNG\r\n").unwrap();
        assert_eq!(shown_by(Input::open(path).unwrap(), &[&PDF, &PNG]).map(|(b, _)| b.name()), Some("png"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_file_no_backend_opens_is_refused_with_what_would_be_shown() {
        let refused = |backends: &[&dyn Backend]| pick(backends, "notes.txt", b"hello").err();
        let said = |shows: &str| Some(format!("notes.txt: not a file {} shows (it shows {shows})", crate::NAME));
        assert_eq!(refused(&[&PDF, &PDF_TOO]), said("PDFs"));
        assert_eq!(refused(&[&PDF, &PNG]), said("PDFs and images"));
        assert_eq!(refused(&[&PDF, &PNG, &PDF_TOO, &DIR]), said("PDFs, images and directories"));
        assert!(pick(&[&PDF], "empty", b"").is_err());
    }

    #[test]
    fn a_file_is_read_once_its_head_and_then_the_rest() {
        let path = std::env::temp_dir().join(format!("clview-input-{}", std::process::id()));
        let bytes: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        let path = path.to_str().unwrap();
        let input = Input::open(path).unwrap();
        assert_eq!(input.head, bytes[..1024]);
        assert_eq!(input.bytes().unwrap(), bytes);
        // On disk, a backend gets the file back from its start.
        let Ok(Reopenable::Disk(mut file)) = Input::open(path).unwrap().reopenable() else { panic!("not on disk") };
        let mut again = Vec::new();
        file.read_to_end(&mut again).unwrap();
        assert_eq!(again, bytes);
        std::fs::write(path, b"%PDF").unwrap();
        assert_eq!(Input::open(path).unwrap().head, b"%PDF");
        std::fs::remove_file(path).unwrap();
        assert!(Input::open(path).is_err_and(|e| e.starts_with(path)));
    }

    #[test]
    fn without_backends_nothing_opens() {
        assert!(choose("a.pdf", &[]).is_err_and(|e| e.contains("built without backends")));
        #[cfg(not(any(feature = "pdf", feature = "image")))]
        assert!(BACKENDS.is_empty());
    }
}
