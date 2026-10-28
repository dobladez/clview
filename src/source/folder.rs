//! A directory as a list shows it, to be opened: one page, a folder drawn here, in code.

use std::sync::Arc;

use super::threads::Threads;
use super::{Frame, Pixels, Pool, Render, Request, Source};

/// A folder's size, in units of its own: wider than tall.
const SIZE: (f32, f32) = (100.0, 80.0);

/// The most pixels a folder is drawn with: what a page may take ([`crate::view`]'s layout keeps a
/// page within it), so a request no layout made can't ask for more.
const MAX_PIXELS: u64 = 16_000_000;

/// Its back, with the tab, and its front, lighter, as opaque RGB.
const BACK: [f32; 3] = [107.0, 143.0, 191.0];
const FRONT: [f32; 3] = [140.0, 174.0, 216.0];

/// The page a directory shows: a folder.
pub(crate) struct Folder;

impl Source for Folder {
    fn pages(&self) -> usize {
        1
    }

    fn size(&self, page: usize) -> Option<(f32, f32)> {
        (page == 0).then_some(SIZE)
    }

    fn natural(&self, _page: usize) -> bool {
        false
    }

    fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
        Ok(Box::new(Drawer))
    }

    fn pool(&self, _threads: usize) -> Box<dyn Pool> {
        Box::new(Threads::new(1, Arc::new(draw)))
    }
}

struct Drawer;

impl Render for Drawer {
    fn render(&mut self, request: Request) -> Frame {
        draw(request)
    }
}

/// The folder at `request`'s scale: a back with a tab at its top left, and a front over it,
/// rounded, their edges smoothed over a pixel.
fn draw(request: Request) -> Frame {
    let side = |units: f32| {
        let pixels = (f64::from(units) * f64::from(request.scale)).ceil();
        // Finite and at most 8192, so the cast is exact.
        (1.0..=8192.0).contains(&pixels).then_some(pixels as u32)
    };
    if request.page != 0 {
        return Frame::failed(request, format!("a folder has one page, not {}", request.page.saturating_add(1)));
    }
    let (Some(w), Some(h)) = (side(SIZE.0), side(SIZE.1)) else {
        return Frame::failed(request, format!("can't draw a folder at scale {}", request.scale));
    };
    if u64::from(w).saturating_mul(u64::from(h)) > MAX_PIXELS {
        return Frame::failed(request, format!("{w} × {h} pixels is too large to draw"));
    }
    let (fw, fh) = (w as f32, h as f32);
    let radius = (fw.min(fh) * 0.05).max(1.0);
    // In pixels: left, top, right, bottom.
    let at = |l: f32, t: f32, r: f32, b: f32| [l * fw, t * fh, r * fw, b * fh];
    let (back, tab, front) = (at(0.06, 0.14, 0.94, 0.92), at(0.06, 0.06, 0.42, 0.24), at(0.06, 0.28, 0.94, 0.92));
    let mut rgba = Vec::with_capacity(usize::try_from(u64::from(w).saturating_mul(u64::from(h)).saturating_mul(4)).unwrap_or(0));
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let b = covered(back, radius, px, py).max(covered(tab, radius, px, py));
            let f = covered(front, radius, px, py);
            // The front over the back, premultiplied.
            let alpha = f + b * (1.0 - f);
            for channel in 0..3 {
                let value = FRONT.get(channel).copied().unwrap_or(0.0) * f + BACK.get(channel).copied().unwrap_or(0.0) * b * (1.0 - f);
                rgba.push(byte(value));
            }
            rgba.push(byte(alpha * 255.0));
        }
    }
    Frame { request, width: w, height: h, rgba: Pixels::new(rgba), error: None, warnings: Vec::new() }
}

/// How much of the pixel centred at `x`, `y` the rectangle `[left, top, right, bottom]` with
/// corners rounded by `radius` covers: from its signed distance, over a pixel.
fn covered([left, top, right, bottom]: [f32; 4], radius: f32, x: f32, y: f32) -> f32 {
    let (cx, cy) = ((left + right) / 2.0, (top + bottom) / 2.0);
    let (qx, qy) = ((x - cx).abs() - ((right - left) / 2.0 - radius), (y - cy).abs() - ((bottom - top) / 2.0 - radius));
    let outside = qx.max(0.0).hypot(qy.max(0.0));
    let distance = outside + qx.max(qy).min(0.0) - radius;
    (0.5 - distance).clamp(0.0, 1.0)
}

/// `value`, 0 to 255, as a byte: rounded, and clamped, so the cast is exact.
fn byte(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 4] {
        let at = usize::try_from((y * frame.width + x) * 4).unwrap();
        frame.rgba[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn a_folder_is_its_front_over_its_back_with_a_tab_and_nothing_around() {
        let frame = Folder.renderer().unwrap().render(Request { page: 0, scale: 1.0, tag: 3 });
        assert_eq!((frame.width, frame.height, frame.error.as_deref(), frame.request.tag), (100, 80, None, 3));
        // Around it, nothing; the tab is the back's colour; the front is lighter.
        assert_eq!(pixel(&frame, 1, 1), [0; 4]);
        assert_eq!(pixel(&frame, 60, 7), [0; 4], "right of the tab");
        assert_eq!(pixel(&frame, 20, 10), [107, 143, 191, 255]);
        assert_eq!(pixel(&frame, 50, 50), [140, 174, 216, 255]);
        // Premultiplied: no colour brighter than its alpha.
        assert!(frame.rgba.chunks(4).all(|p| p[..3].iter().all(|c| *c <= p[3])));
    }

    #[test]
    fn a_folder_is_drawn_at_any_scale_it_can_be_and_refuses_the_rest() {
        let draw = |scale: f32, page: usize| draw(Request { page, scale, tag: 0 });
        assert_eq!((draw(0.25, 0).width, draw(0.25, 0).height), (25, 20));
        assert_eq!((draw(0.001, 0).width, draw(0.001, 0).height), (1, 1));
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 1000.0] {
            assert!(draw(scale, 0).error.is_some(), "{scale}");
        }
        assert!(draw(1.0, 1).error.is_some_and(|e| e.contains("one page")));
        assert_eq!((Folder.pages(), Folder.size(0), Folder.size(1)), (1, Some(SIZE), None));
    }
}
