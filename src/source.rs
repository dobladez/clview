//! What the viewer shows: a source of pages, and the backends that render them. The pager and
//! the printer in `view` see only the traits here; each backend brings its own renderer and its
//! own threads, so one can cancel or time out work the way it knows how.

#[cfg(feature = "pdfcraft")]
mod pdfcraft;

use std::ops::Deref;

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
    #[cfg_attr(all(not(feature = "pdfcraft"), not(test)), expect(dead_code, reason = "only a backend or a test makes pixels"))]
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

/// Something with pages to show: a PDF's pages.
pub(crate) trait Source {
    /// How many pages: at least one, since a source with none fails to open.
    fn pages(&self) -> usize;
    /// Page `page`'s size as shown, in the source's own unit (points for a PDF), or `None` past
    /// the last page. A render at scale `s` is about `s` times as many pixels on each side.
    fn size(&self, page: usize) -> Option<(f32, f32)>;
    /// A renderer on this thread, for pages printed one after another.
    fn renderer(&self) -> Result<Box<dyn Render + '_>, String>;
    /// Renders on threads of their own, for the pager: keys still work while pages render, and
    /// a page that takes too long is given up on.
    fn pool(&self, threads: usize) -> Box<dyn Pool + '_>;
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

/// `path` as a source, opened with `password` if it needs one.
#[cfg(feature = "pdfcraft")]
pub(crate) fn open(path: &str, password: Option<&str>) -> Result<Box<dyn Source>, String> {
    Ok(Box::new(pdfcraft::Pdf::open(path, password)?))
}

/// `path` as a source: a build without backends has none.
#[cfg(not(feature = "pdfcraft"))]
pub(crate) fn open(path: &str, _password: Option<&str>) -> Result<Box<dyn Source>, String> {
    Err(format!("{path}: this {} was built without a PDF backend (the `pdfcraft` feature)", crate::NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The viewer knows sources only through the traits here: nothing in `src/view/` names a
    /// backend. A build without default features checks it too, for the code it compiles.
    #[test]
    fn the_viewer_names_no_backend() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/view");
        let mut read = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                assert!(!text.contains("pdfcraft_"), "{} names a pdfcraft crate", path.display());
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

    #[cfg(not(feature = "pdfcraft"))]
    #[test]
    fn without_a_backend_nothing_opens() {
        assert!(open("a.pdf", None).is_err_and(|e| e.contains("without a PDF backend")));
    }
}
