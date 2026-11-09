//! PDFs rendered by pdfcraft, the way its app draws them: comments without an appearance of their
//! own are drawn, a dynamic XFA form shows the pages its template lays out into, and either
//! password opens a protected file.

use std::sync::Arc;

use pdfcraft_render::{PageRenderer, RenderConfig, RenderPool, RenderRequest, RenderWarning, RenderedPage, RequestKind};

use super::{Backend, Deferred, Format, Frame, Input, Opened, Pixels, Request, Source};
use crate::args::{OptionSpec, Parsed};

/// PDFs, rendered by pdfcraft.
pub(super) struct Pdfcraft;

const OPTIONS: &[OptionSpec] =
    &[OptionSpec { name: "--password", short: None, value: Some("PW"), help: "the user or the owner password of an encrypted PDF", alias: None }];

impl Backend for Pdfcraft {
    fn name(&self) -> &'static str {
        "pdfcraft"
    }

    fn shows(&self) -> &'static str {
        "PDFs"
    }

    fn formats(&self) -> Vec<Format> {
        vec![Format { name: "PDF", extensions: &[] }]
    }

    fn credits(&self) -> &[&'static str] {
        &["pdfcraft"]
    }

    fn options(&self) -> &'static [OptionSpec] {
        OPTIONS
    }

    fn opens(&self, _path: &str, head: &[u8]) -> bool {
        is_pdf(head)
    }

    fn draws_pictures(&self) -> bool {
        true
    }

    /// It reads the whole file to open it.
    fn deferred(&self, _path: &str, _head: &[u8]) -> Option<Deferred> {
        Some(Deferred { settings: &[] })
    }

    fn open(&self, input: Input, options: &Parsed<'_>) -> Result<Opened, String> {
        let path = input.path.clone();
        Ok(Opened::Pages(Box::new(Pdf::open(&path, Arc::new(input.bytes()?), options.value("--password"))?)))
    }
}

/// Whether a file starting with `head` is a PDF: its header (`%PDF-` and the version) within the
/// first 1024 bytes. ISO 32000-2 §7.5.2 puts the header first; readers have long allowed bytes
/// before it, up to that bound.
fn is_pdf(head: &[u8]) -> bool {
    head.get(..1024).unwrap_or(head).windows(5).any(|w| w == b"%PDF-")
}

/// An open PDF: the display copy the renderers read, and each page's size as shown.
pub(super) struct Pdf {
    bytes: Arc<Vec<u8>>,
    config: RenderConfig,
    /// In points, after `/Rotate` and `/UserUnit`.
    sizes: Vec<(f32, f32)>,
}

impl Pdf {
    fn open(path: &str, bytes: Arc<Vec<u8>>, password: Option<&str>) -> Result<Self, String> {
        let display = displayed(path, bytes, password)?;
        let info = display.info.map_err(|e| format!("{path}: {e}"))?;
        if info.pages.is_empty() {
            return Err(format!("{path}: no pages to show"));
        }
        Ok(Self {
            bytes: display.bytes,
            config: RenderConfig { password: display.password.as_deref().map(Arc::from), reject_oversize: true, ..Default::default() },
            sizes: info.pages.iter().map(|p| (p.width, p.height)).collect(),
        })
    }
}

impl Source for Pdf {
    fn pages(&self) -> usize {
        self.sizes.len()
    }

    fn size(&self, page: usize) -> Option<(f32, f32)> {
        self.sizes.get(page).copied()
    }

    fn natural(&self, _page: usize) -> bool {
        false
    }

    fn renderer(&self) -> Result<Box<dyn super::Render + '_>, String> {
        let renderer = PageRenderer::new(self.bytes.clone(), self.config.clone());
        // `inspect` read the file, but the renderer's own parser may not.
        if renderer.page_count() == 0 {
            return Err("the renderer can't read this document".into());
        }
        Ok(Box::new(Renderer(renderer)))
    }

    fn pool(&self, threads: usize) -> Box<dyn super::Pool> {
        Box::new(Pool(RenderPool::new(self.bytes.clone(), threads, self.config.clone())))
    }
}

struct Renderer(PageRenderer);

impl super::Render for Renderer {
    fn render(&mut self, request: Request) -> Frame {
        frame(self.0.render(pdfcraft_request(request)))
    }
}

/// pdfcraft's render threads and their watchdog: a page that takes too long is told to stop at
/// its next drawing operator, and comes back failed.
struct Pool(RenderPool);

impl super::Pool for Pool {
    fn set_queue(&self, requests: Vec<Request>) {
        self.0.set_queue(requests.into_iter().map(pdfcraft_request).collect());
    }

    fn try_recv(&self) -> Option<Frame> {
        self.0.try_recv().map(frame)
    }
}

fn pdfcraft_request(r: Request) -> RenderRequest {
    RenderRequest { page: r.page, kind: RequestKind::Pixels, tile: None, scale: r.scale, tag: r.tag, preview: false }
}

/// A page pdfcraft rendered, its warnings put into words.
fn frame(p: RenderedPage) -> Frame {
    let request = Request { page: p.request.page, scale: p.request.scale, tag: p.request.tag, ..Default::default() };
    let warnings = p
        .warnings
        .iter()
        .map(|w| match w {
            RenderWarning::ContentTruncated => "content past the page's safety budget was skipped",
            RenderWarning::ColourFallback => "a colour that could not be computed was painted in a fallback colour",
        })
        .map(String::from)
        .collect();
    Frame { request, width: p.width, height: p.height, rgba: Pixels::new(p.rgba), error: p.error, warnings, again: false }
}

/// What is drawn for a file, as the pdfcraft app draws it.
struct Displayed {
    /// The display copy the renderer reads: comments missing their /AP streams are drawn, and a
    /// dynamic XFA form is laid out. The file itself is left intact.
    bytes: Arc<Vec<u8>>,
    /// The password for the renderer, which takes only the user password of R2–R4 files: for one
    /// opened with its owner password, the user password that recovers.
    password: Option<String>,
    /// The displayed document's pages and the rest (a laid-out form's, not its placeholder's),
    /// or why the renderer can't open the file.
    info: Result<pdfcraft_render::DocInfo, String>,
}

/// [`Displayed`] for the file `bytes` read from `path`, opened with `password` (either one).
fn displayed(path: &str, bytes: Arc<Vec<u8>>, password: Option<&str>) -> Result<Displayed, String> {
    // `Session::open` and the display copy take either password; the renderer doesn't.
    let render_password = pdfcraft_engine::render_password_for(bytes.clone(), password);
    let info = pdfcraft_render::inspect(bytes.clone(), render_password.as_deref()).map_err(|e| e.to_string());
    if info.as_ref().is_ok_and(|info| info.xfa == Some(pdfcraft_render::Xfa::Dynamic)) {
        let mut session = pdfcraft_engine::Session::new();
        let id = session.open(path, None, bytes, password).map_err(|e| e.to_string())?;
        let doc = session.get(id).ok_or("the document could not be opened")?;
        return Ok(Displayed { bytes: doc.display.clone(), password: render_password, info: Ok(doc.info.clone()) });
    }
    Ok(Displayed { bytes: pdfcraft_engine::display_bytes_for(bytes, password), password: render_password, info })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No test file makes a page fail to render, so this is what says a render's error gets through.
    #[test]
    fn a_frame_keeps_why_its_render_failed() {
        let request = RenderRequest { page: 2, kind: RequestKind::Pixels, tile: None, scale: 1.5, tag: 9, preview: false };
        let rgba = pdfcraft_render::Pixels::from(vec![0xFF80_8080; 2]);
        let failed =
            RenderedPage { request, width: 2, height: 1, rgba, error: Some("no page box".into()), text: None, warnings: Vec::new(), millis: 0 };
        assert_eq!(frame(failed).error.as_deref(), Some("no page box"));
    }

    #[test]
    fn a_pdf_is_a_file_with_the_header_in_its_first_kilobyte() {
        assert!(is_pdf(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n"));
        assert!(is_pdf(b"%PDF-2.0"));
        let prefixed = [vec![b' '; 1019], b"%PDF-1.4".to_vec()].concat();
        assert!(is_pdf(&prefixed), "the header may start as late as byte 1019");
        assert!(!is_pdf(&[vec![b' '; 1020], b"%PDF-1.4".to_vec()].concat()), "it must end within the first 1024 bytes");
        assert!(!is_pdf(b"%PDF"), "a truncated header");
        assert!(!is_pdf(b"\x89PNG\r\n\x1a\n"));
        assert!(!is_pdf(b""));
    }
}
