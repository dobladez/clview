//! A container as a list shows it, to be opened: one page, a folder drawn here, in code, with
//! the badge its container gives it.

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

/// A parcel's tape and bow, and the line around each of their pieces.
const TAPE: [f32; 3] = [150.0, 106.0, 62.0];
const LINE: [f32; 3] = [28.0, 24.0, 22.0];

/// What a folder carries, saying what kind of container it is: the container's to say
/// ([`super::Container::badge`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Badge {
    /// Nothing: a directory.
    #[default]
    None,
    /// Tied up as a parcel, tape down and across it and a bow where they cross: an archive.
    #[cfg_attr(not(any(feature = "zip", feature = "tar", test)), expect(dead_code, reason = "only archives are parcels"))]
    Parcel,
}

/// The page a container shows: a folder, with its badge.
pub(crate) struct Folder(pub(crate) Badge);

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
        Ok(Box::new(Drawer(self.0)))
    }

    fn pool(&self, _threads: usize) -> Box<dyn Pool> {
        let badge = self.0;
        Box::new(Threads::new(1, Arc::new(move |request| draw(request, badge))))
    }
}

struct Drawer(Badge);

impl Render for Drawer {
    fn render(&mut self, request: Request) -> Frame {
        draw(request, self.0)
    }
}

/// The folder at `request`'s scale: a back with a tab at its top left, and a front over it,
/// rounded, their edges smoothed over a pixel; then its badge.
fn draw(request: Request, badge: Badge) -> Frame {
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
    let pieces: &[Shape] = match badge {
        Badge::None => &[],
        Badge::Parcel => &parcel(fw, fh),
    };
    let line = (fw.min(fh) * 0.02).max(1.0);
    let mut rgba = Vec::with_capacity(usize::try_from(u64::from(w).saturating_mul(u64::from(h)).saturating_mul(4)).unwrap_or(0));
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let b = covered(back, radius, px, py).max(covered(tab, radius, px, py));
            // The front over the back, then each piece of the badge over them, its line first.
            let mut pixel = over(([0.0; 3], 0.0), BACK, b);
            pixel = over(pixel, FRONT, covered(front, radius, px, py));
            for piece in pieces {
                pixel = over(pixel, LINE, piece.covered(line, px, py));
                pixel = over(pixel, TAPE, piece.covered(0.0, px, py));
            }
            let (colour, alpha) = pixel;
            rgba.extend(colour.map(byte));
            rgba.push(byte(alpha * 255.0));
        }
    }
    Frame { request, width: w, height: h, rgba: Pixels::new(rgba), error: None, warnings: Vec::new() }
}

/// `colour` over a premultiplied pixel (0 to 255, and its alpha), covering `cover` of it.
fn over((below, alpha): ([f32; 3], f32), colour: [f32; 3], cover: f32) -> ([f32; 3], f32) {
    let mut out = below;
    for (value, top) in out.iter_mut().zip(colour) {
        *value = top * cover + *value * (1.0 - cover);
    }
    (out, cover + alpha * (1.0 - cover))
}

/// A parcel's pieces in pixels, in the order they are drawn, each over a line of its own: the tape
/// down the whole folder and across it, then the bow, its two loops and its knot. The lines go a
/// little past the folder's edges, as if the tape went round it.
fn parcel(fw: f32, fh: f32) -> [Shape; 5] {
    let at = |l: f32, t: f32, r: f32, b: f32| [l * fw, t * fh, r * fw, b * fh];
    let point = |x: f32, y: f32| (x * fw, y * fh);
    [
        Shape::Rect(at(0.45, 0.14, 0.55, 0.92), 0.0),
        Shape::Rect(at(0.06, 0.5375, 0.94, 0.6625), 0.0),
        Shape::Triangle([point(0.5, 0.6), point(0.34, 0.45), point(0.32, 0.675)]),
        Shape::Triangle([point(0.5, 0.6), point(0.66, 0.45), point(0.68, 0.675)]),
        Shape::Rect(at(0.44, 0.525, 0.56, 0.675), fw.min(fh) * 0.03),
    ]
}

/// A piece of a badge, in pixels: a rectangle `[left, top, right, bottom]` with its corners
/// rounded by a radius, or a triangle.
enum Shape {
    Rect([f32; 4], f32),
    Triangle([(f32, f32); 3]),
}

impl Shape {
    /// How much of the pixel centred at `x`, `y` it covers, grown by `grow` pixels all round.
    fn covered(&self, grow: f32, x: f32, y: f32) -> f32 {
        match *self {
            Shape::Rect([l, t, r, b], radius) => covered([l - grow, t - grow, r + grow, b + grow], radius + grow, x, y),
            Shape::Triangle([a, b, c]) => {
                // Which way round its points go: a point is in where it is on the third point's
                // side of each edge. A triangle with no area covers nothing.
                let side = |(x0, y0): (f32, f32), (x1, y1): (f32, f32), (px, py): (f32, f32)| (x1 - x0) * (py - y0) - (y1 - y0) * (px - x0);
                let turn = side(a, b, c);
                if turn == 0.0 {
                    return 0.0;
                }
                // The distance out past its farthest edge: negative inside.
                let distance = [(a, b), (b, c), (c, a)]
                    .map(|(p, q)| -turn.signum() * side(p, q, (x, y)) / (q.0 - p.0).hypot(q.1 - p.1))
                    .into_iter()
                    .fold(f32::MIN, f32::max);
                (0.5 - (distance - grow)).clamp(0.0, 1.0)
            }
        }
    }
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

    /// What makes it a folder, so that it can be repainted or reshaped without this failing.
    #[test]
    fn a_folder_is_a_lighter_front_over_a_back_with_a_tab_and_nothing_around() {
        let frame = Folder(Badge::None).renderer().unwrap().render(Request { page: 0, scale: 1.0, tag: 3 });
        assert_eq!((frame.width, frame.height, frame.error.as_deref(), frame.request.tag), (100, 80, None, 3));
        let (w, h) = (frame.width, frame.height);
        // Around it, nothing.
        for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
            assert_eq!(pixel(&frame, x, y), [0; 4], "({x}, {y})");
        }
        // Its top row is the tab's, on the left.
        let top = (0..h).find(|&y| (0..w).any(|x| pixel(&frame, x, y)[3] == 255)).unwrap();
        let tab: Vec<u32> = (0..w).filter(|&x| pixel(&frame, x, top)[3] == 255).collect();
        assert!(tab.iter().all(|x| *x < w / 2), "{tab:?}");
        // The tab is the back's: the front is lighter.
        let light = |p: [u8; 4]| p[..3].iter().map(|c| u32::from(*c)).sum::<u32>();
        assert!(light(pixel(&frame, w / 2, h / 2)) > light(pixel(&frame, tab[tab.len() / 2], top)));
        // Premultiplied: no colour brighter than its alpha.
        assert!(frame.rgba.chunks(4).all(|p| p[..3].iter().all(|c| *c <= p[3])));
    }

    /// A parcel is a folder with something tied round it, across its middle and past its edge, and
    /// most of the folder shows as it is.
    #[test]
    fn a_parcel_is_a_folder_with_something_tied_round_it() {
        let drawn = |badge| Folder(badge).renderer().unwrap().render(Request { page: 0, scale: 1.0, tag: 0 });
        let (frame, plain) = (drawn(Badge::Parcel), drawn(Badge::None));
        let (w, h) = (frame.width, frame.height);
        assert_eq!((w, h), (plain.width, plain.height));
        assert_ne!(pixel(&frame, w / 2, h * 6 / 10), pixel(&plain, w / 2, h * 6 / 10), "where it crosses");
        let both = || frame.rgba.chunks(4).zip(plain.rgba.chunks(4));
        assert!(both().any(|(tied, bare)| tied[3] > 0 && bare[3] == 0), "past the folder's edge");
        assert!(both().filter(|(tied, bare)| tied == bare).count() * 2 > both().count(), "the rest as it is");
        assert!(frame.rgba.chunks(4).all(|p| p[..3].iter().all(|c| *c <= p[3])));
        // At any scale, small or large, it draws.
        for scale in [0.001, 0.3, 7.5] {
            assert!(draw(Request { page: 0, scale, tag: 0 }, Badge::Parcel).error.is_none(), "{scale}");
        }
    }

    #[test]
    fn a_folder_is_drawn_at_any_scale_it_can_be_and_refuses_the_rest() {
        let draw = |scale: f32, page: usize| draw(Request { page, scale, tag: 0 }, Badge::None);
        assert_eq!((draw(0.25, 0).width, draw(0.25, 0).height), (25, 20));
        assert_eq!((draw(0.001, 0).width, draw(0.001, 0).height), (1, 1));
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 1000.0] {
            assert!(draw(scale, 0).error.is_some(), "{scale}");
        }
        assert!(draw(1.0, 1).error.is_some_and(|e| e.contains("one page")));
        let folder = Folder(Badge::Parcel);
        assert_eq!((folder.pages(), folder.size(0), folder.size(1)), (1, Some(SIZE), None));
    }
}
