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

/// An archive's zipper and the line around its plate; the hole in the zipper's pull; the plate,
/// orange against the folder's blue; and the letters on it.
const INK: [f32; 3] = [28.0, 24.0, 22.0];
const METAL: [f32; 3] = [196.0, 200.0, 206.0];
const PLATE: [f32; 3] = [222.0, 104.0, 28.0];
const LETTER: [f32; 3] = [250.0, 250.0, 246.0];

/// What a folder carries, saying what kind of container it is: the container's to say
/// ([`super::Container::badge`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Badge {
    /// Nothing: a directory.
    #[default]
    None,
    /// A zipper down its front, and a plate over its bottom right corner saying which kind it is,
    /// in the capitals [`glyph`] has strokes for (`ZIP`, `TGZ`): an archive.
    #[cfg_attr(not(any(feature = "zip", feature = "tar", test)), expect(dead_code, reason = "only archives are zipped"))]
    Archive(&'static str),
}

impl Badge {
    /// Whether every letter of its label has strokes: one without is left out of the plate.
    #[cfg(test)]
    pub(crate) fn spelled(self) -> bool {
        match self {
            Badge::None => true,
            Badge::Archive(label) => label.chars().all(|letter| glyph(letter).is_some()),
        }
    }
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
    let pieces = match badge {
        Badge::None => Vec::new(),
        Badge::Archive(label) => archive(label, fw, fh),
    };
    // Each with the pixels it reaches, so that the rest don't ask it: a large folder's letters
    // took ten times the folder otherwise.
    let pieces: Vec<_> = pieces.into_iter().map(|(piece, colour)| (piece.reach(), piece, colour)).collect();
    let mut rgba = Vec::with_capacity(usize::try_from(u64::from(w).saturating_mul(u64::from(h)).saturating_mul(4)).unwrap_or(0));
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let b = covered(back, radius, px, py).max(covered(tab, radius, px, py));
            // The front over the back, then each piece of the badge over them.
            let mut pixel = over(([0.0; 3], 0.0), BACK, b);
            pixel = over(pixel, FRONT, covered(front, radius, px, py));
            for ([l, t, r, b], piece, colour) in &pieces {
                if px > *l && px < *r && py > *t && py < *b {
                    pixel = over(pixel, *colour, piece.covered(px, py));
                }
            }
            let (colour, alpha) = pixel;
            rgba.extend(colour.map(byte));
            rgba.push(byte(alpha * 255.0));
        }
    }
    Frame { request, width: w, height: h, rgba: Pixels::new(rgba), error: None, warnings: Vec::new(), again: false }
}

/// `colour` over a premultiplied pixel (0 to 255, and its alpha), covering `cover` of it.
fn over((below, alpha): ([f32; 3], f32), colour: [f32; 3], cover: f32) -> ([f32; 3], f32) {
    let mut out = below;
    for (value, top) in out.iter_mut().zip(colour) {
        *value = top * cover + *value * (1.0 - cover);
    }
    (out, cover + alpha * (1.0 - cover))
}

/// An archive's pieces in pixels, each with its colour, in the order they are drawn: a zipper down
/// the front from its top (its track, its teeth, its pull and the hole in it), then the plate, over
/// the folder's bottom right corner and a little past it, on a line of its own, and the label's
/// letters on it. Placed in the folder's units ([`SIZE`]).
fn archive(label: &str, fw: f32, fh: f32) -> Vec<(Shape, [f32; 3])> {
    let (ux, uy) = (fw / SIZE.0, fh / SIZE.1);
    let unit = ux.min(uy);
    let at = |[l, t, r, b]: [f32; 4], radius: f32| Shape::Rect([l * ux, t * uy, r * ux, b * uy], radius * unit);
    let mut pieces = vec![(at([28.9, 22.4, 31.1, 46.0], 0.0), INK)];
    pieces.extend([26.0, 31.2, 36.4, 41.6].map(|y| (at([25.4, y, 34.6, y + 2.2], 1.1), INK)));
    pieces.push((at([25.4, 45.0, 34.6, 57.5], 4.6), INK));
    pieces.push((at([28.0, 49.6, 32.0, 55.0], 2.0), METAL));
    // The line around the plate is the plate grown, by a pixel at least.
    let line = 1.6_f32.max(1.0 / unit);
    let [l, t, r, b] = PLATE_AT;
    pieces.push((at([l - line, t - line, r + line, b + line], 3.4 + line), INK));
    pieces.push((at(PLATE_AT, 3.4), PLATE));
    pieces.extend(letters(label, ux, uy).into_iter().map(|stroke| (stroke, LETTER)));
    pieces
}

/// Where an archive's plate is, in the folder's units: left, top, right, bottom.
const PLATE_AT: [f32; 4] = [51.0, 47.6, 97.4, 77.4];

/// The most letters of a label that are drawn: more than any archive's has.
const LETTERS: usize = 8;

/// A label's letters as strokes in pixels, in the middle of the plate: as tall as the plate has
/// room for, or smaller, for a label too long to fit across it at that size. A letter [`glyph`]
/// has no strokes for is left out, its room kept.
fn letters(label: &str, ux: f32, uy: f32) -> Vec<Shape> {
    // In the folder's units: how tall the letters are, and the room they have across the plate.
    const HEIGHT: f32 = 13.5;
    const ROOM: f32 = 40.0;
    // In a letter's grid: how wide a stroke is, and the room between two letters.
    const WIDTH: f32 = 0.95;
    const GAP: f32 = 1.5 + WIDTH / 2.0;
    let glyphs: Vec<_> = label.chars().take(LETTERS).map(|letter| glyph(letter).unwrap_or((4.0, &[]))).collect();
    // At most LETTERS of them, so the cast is exact.
    let across = glyphs.iter().map(|(width, _)| width).sum::<f32>() + GAP * glyphs.len().saturating_sub(1) as f32;
    // A step of the grid in units: the strokes' round ends reach half a stroke past the letters.
    let step = (HEIGHT / 6.0).min(ROOM / (across + WIDTH));
    let [l, t, r, b] = PLATE_AT;
    let (mut x, y) = ((l + r) / 2.0 - across * step / 2.0, (t + b) / 2.0 - 3.0 * step);
    let half = WIDTH * step * ux.min(uy) / 2.0;
    let mut strokes = Vec::new();
    for (width, lines) in glyphs {
        let point = |(gx, gy): (f32, f32)| ((x + gx * step) * ux, (y + gy * step) * uy);
        for line in lines {
            strokes.extend(line.windows(2).filter_map(|ends| Some(Shape::Stroke(point(*ends.first()?), point(*ends.get(1)?), half))));
        }
        x += (width + GAP) * step;
    }
    strokes
}

/// A letter's lines, each through its points on the letter's grid.
type Lines = &'static [&'static [(f32, f32)]];

/// A capital's strokes, each a line through its points on a grid 4 wide and 6 tall, from its top
/// left, and how wide it is on that grid; `None` for a letter no archive's label has had. Drawn
/// here, in straight strokes, because a folder is drawn without a font.
fn glyph(letter: char) -> Option<(f32, Lines)> {
    /// The stem and bowl `P` and `R` share.
    const BOWL: &[(f32, f32)] = &[(0.0, 6.0), (0.0, 0.0), (3.0, 0.0), (4.0, 1.0), (4.0, 2.3), (3.0, 3.3), (0.0, 3.3)];
    let glyph: (f32, Lines) = match letter {
        'A' => (4.0, &[&[(0.0, 6.0), (2.0, 0.0), (4.0, 6.0)], &[(0.75, 4.0), (3.25, 4.0)]]),
        'B' => (
            4.0,
            &[
                &[(0.0, 0.0), (3.0, 0.0), (4.0, 1.0), (4.0, 2.0), (3.0, 3.0), (0.0, 3.0)],
                &[(3.0, 3.0), (4.0, 4.0), (4.0, 5.0), (3.0, 6.0), (0.0, 6.0), (0.0, 0.0)],
            ],
        ),
        'G' => (4.0, &[&[(4.0, 1.0), (3.0, 0.0), (1.0, 0.0), (0.0, 1.0), (0.0, 5.0), (1.0, 6.0), (3.0, 6.0), (4.0, 5.0), (4.0, 3.3), (2.2, 3.3)]]),
        'I' => (0.0, &[&[(0.0, 0.0), (0.0, 6.0)]]),
        'P' => (4.0, &[BOWL]),
        'R' => (4.0, &[BOWL, &[(2.0, 3.3), (4.0, 6.0)]]),
        'T' => (4.0, &[&[(0.0, 0.0), (4.0, 0.0)], &[(2.0, 0.0), (2.0, 6.0)]]),
        'X' => (4.0, &[&[(0.0, 0.0), (4.0, 6.0)], &[(4.0, 0.0), (0.0, 6.0)]]),
        'Z' => (4.0, &[&[(0.0, 0.0), (4.0, 0.0), (0.0, 6.0), (4.0, 6.0)]]),
        _ => return None,
    };
    Some(glyph)
}

/// A piece of a badge, in pixels: a rectangle `[left, top, right, bottom]` with its corners
/// rounded by a radius, or a stroke from one point to another, half as wide each side as its last
/// number, its ends round.
enum Shape {
    Rect([f32; 4], f32),
    Stroke((f32, f32), (f32, f32), f32),
}

impl Shape {
    /// The rectangle `[left, top, right, bottom]` it covers no pixel centred outside of: itself and
    /// a pixel around it, more than the half pixel its edge is smoothed over.
    fn reach(&self) -> [f32; 4] {
        match *self {
            Shape::Rect([l, t, r, b], _) => [l - 1.0, t - 1.0, r + 1.0, b + 1.0],
            Shape::Stroke((ax, ay), (bx, by), half) => {
                [ax.min(bx) - half - 1.0, ay.min(by) - half - 1.0, ax.max(bx) + half + 1.0, ay.max(by) + half + 1.0]
            }
        }
    }

    /// How much of the pixel centred at `x`, `y` it covers.
    fn covered(&self, x: f32, y: f32) -> f32 {
        match *self {
            Shape::Rect(sides, radius) => covered(sides, radius, x, y),
            Shape::Stroke((ax, ay), (bx, by), half) => {
                let (dx, dy) = (bx - ax, by - ay);
                let length = dx * dx + dy * dy;
                // How far along it the nearest point to the pixel is: its start, for a stroke with
                // no length.
                let along = if length > 0.0 { (((x - ax) * dx + (y - ay) * dy) / length).clamp(0.0, 1.0) } else { 0.0 };
                let distance = (x - ax - along * dx).hypot(y - ay - along * dy) - half;
                (0.5 - distance).clamp(0.0, 1.0)
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
        let frame = Folder(Badge::None).renderer().unwrap().render(Request { page: 0, scale: 1.0, tag: 3, ..Default::default() });
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

    /// An archive is a folder with a zipper down its front and a plate over its corner and past
    /// it, and most of the folder shows as it is.
    #[test]
    fn an_archive_is_a_folder_zipped_with_a_plate_over_its_corner() {
        let drawn = |badge| Folder(badge).renderer().unwrap().render(Request { page: 0, scale: 1.0, tag: 0, ..Default::default() });
        let (frame, plain) = (drawn(Badge::Archive("ZIP")), drawn(Badge::None));
        let (w, h) = (frame.width, frame.height);
        assert_eq!((w, h), (plain.width, plain.height));
        // The zipper, on the front's left, and the plate, under its letters, in a colour of its own.
        assert_ne!(pixel(&frame, 30, 30), pixel(&plain, 30, 30), "the zipper");
        assert_eq!(pixel(&frame, 56, 72), [222, 104, 28, 255], "the plate");
        let both = || frame.rgba.chunks(4).zip(plain.rgba.chunks(4));
        assert!(both().any(|(zipped, bare)| zipped[3] > 0 && bare[3] == 0), "past the folder's edge");
        assert!(both().filter(|(zipped, bare)| zipped == bare).count() * 2 > both().count(), "the rest as it is");
        assert!(frame.rgba.chunks(4).all(|p| p[..3].iter().all(|c| *c <= p[3])));
        // At any scale, small or large, it draws.
        for scale in [0.001, 0.3, 7.5] {
            assert!(draw(Request { page: 0, scale, tag: 0, ..Default::default() }, Badge::Archive("ZIP")).error.is_none(), "{scale}");
        }
    }

    /// Past its reach a piece covers nothing, so leaving it unasked there changes no pixel.
    #[test]
    fn a_piece_covers_nothing_past_its_reach() {
        for (w, h) in [(1_u16, 1_u16), (30, 24), (250, 200)] {
            for (piece, _) in archive("TBZ", f32::from(w), f32::from(h)) {
                let [l, t, r, b] = piece.reach();
                let mut within = 0.0;
                for (x, y) in (0..w).flat_map(|x| (0..h).map(move |y| (f32::from(x) + 0.5, f32::from(y) + 0.5))) {
                    match x > l && x < r && y > t && y < b {
                        true => within += piece.covered(x, y),
                        false => assert_eq!(piece.covered(x, y), 0.0, "({x}, {y}) of {w} × {h}, past {l} {t} {r} {b}"),
                    }
                }
                // And within it, at a size worth drawing, it is there.
                assert!(within > 0.0 || w < 250, "nothing within {l} {t} {r} {b}");
            }
        }
    }

    /// The label is written on the plate and nowhere else, whatever it is: each kind its own, a
    /// long one smaller, and a letter there are no strokes for left out.
    #[test]
    fn an_archives_label_is_written_within_its_plate() {
        let drawn = |label| Folder(Badge::Archive(label)).renderer().unwrap().render(Request { page: 0, scale: 2.0, tag: 0, ..Default::default() });
        let blank = drawn("");
        // The pixels a label changes: how many, and whether all are within the plate, at twice
        // the folder's units.
        let written = |label| {
            let frame = drawn(label);
            let changed: Vec<usize> =
                (0..frame.rgba.len() / 4).filter(|at| frame.rgba[at * 4..at * 4 + 4] != blank.rgba[at * 4..at * 4 + 4]).collect();
            let within = changed.iter().all(|at| {
                let (x, y) = ((at % 200) as f32 / 2.0, (at / 200) as f32 / 2.0);
                x > PLATE_AT[0] && y > PLATE_AT[1] && x < PLATE_AT[2] - 0.5 && y < PLATE_AT[3] - 0.5
            });
            (changed.len(), within)
        };
        for label in ["ZIP", "TAR", "TGZ", "TBZ", "TXZ", "ZIPZIPZIPZIPZIP", "BBBBBBBB"] {
            let (letters, within) = written(label);
            assert!(letters > 100 && within, "{label}: {letters} pixels, within the plate: {within}");
        }
        assert!(drawn("ZIP").rgba[..] != drawn("TAR").rgba[..], "each kind its own");
        // Letters without strokes: nothing written, and the ones beside them still are.
        assert_eq!(written("é7 z").0, 0);
        assert!(written("éZ").0 > 100 && written("éZ").1);
        assert!(Badge::Archive("TGZ").spelled() && Badge::None.spelled() && !Badge::Archive("7Z").spelled());
    }

    #[test]
    fn a_folder_is_drawn_at_any_scale_it_can_be_and_refuses_the_rest() {
        let draw = |scale: f32, page: usize| draw(Request { page, scale, tag: 0, ..Default::default() }, Badge::None);
        assert_eq!((draw(0.25, 0).width, draw(0.25, 0).height), (25, 20));
        assert_eq!((draw(0.001, 0).width, draw(0.001, 0).height), (1, 1));
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 1000.0] {
            assert!(draw(scale, 0).error.is_some(), "{scale}");
        }
        assert!(draw(1.0, 1).error.is_some_and(|e| e.contains("one page")));
        let folder = Folder(Badge::Archive("ZIP"));
        assert_eq!((folder.pages(), folder.size(0), folder.size(1)), (1, Some(SIZE), None));
    }
}
