//! Opening a PDF the way pdfcraft displays it.

use std::sync::Arc;

pub(crate) fn read(path: &str) -> Result<Arc<Vec<u8>>, String> {
    std::fs::read(path).map(Arc::new).map_err(|e| format!("{path}: {e}"))
}

/// What is drawn for a file, as the pdfcraft app draws it.
pub(crate) struct Displayed {
    /// The display copy the renderer reads: comments missing their /AP streams are drawn, and a
    /// dynamic XFA form is laid out. The file itself is left intact.
    pub(crate) bytes: Arc<Vec<u8>>,
    /// The password for the renderer, which takes only the user password of R2–R4 files: for one
    /// opened with its owner password, the user password that recovers.
    pub(crate) password: Option<String>,
    /// The displayed document's pages and the rest (a laid-out form's, not its placeholder's),
    /// or why the renderer can't open the file.
    pub(crate) info: Result<pdfcraft_render::DocInfo, String>,
}

/// [`Displayed`] for the file `bytes` read from `path`, opened with `password` (either one).
pub(crate) fn displayed(path: &str, bytes: Arc<Vec<u8>>, password: Option<&str>) -> Result<Displayed, String> {
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
