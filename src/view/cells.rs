//! Rendered pages as text, for a terminal without the graphics protocol: each cell a character and
//! its colours. Pure: pixels in, a line of bytes for each row of cells out.
//!
//! Half-blocks (`▀`) are the default: the foreground colours the cell's top half and the background
//! its bottom half, so a cell holds two samples of the page. Without colours, or in ASCII, a cell
//! holds one sample, and how much of the cell its character inks says how bright it is, which
//! depends on the terminal's background ([`Ground`]). What goes out is characters and digits this
//! file writes, so nothing a document shapes reaches the terminal raw.
//!
//! `docs/architecture.md` says how the pager and the printer use it, and ADR-0011 why.

use std::io::Write as _;

use super::fit::Cell;
use crate::encode::check_raster;

/// The most cells a page is drawn in: far more than a window has, and a page fitted to a window's
/// width is a few thousand rows at most. A cell may take 39 bytes, so this bounds a page's text to
/// about 160 MB, however the terminal reports its size.
const MAX_CELLS: u64 = 1 << 22;

/// How many pixels a side a page with no natural size renders at for each sample, before they are
/// averaged into it. One leaves a PDF's pictures speckled, as they are point-sampled at so small a
/// scale; four is clean, more shows nothing new, and none of it costs a noticeable time (ADR-0011).
pub(crate) const OVERSAMPLE: f32 = 4.0;

/// Characters from the least ink to the most, one sample to a cell.
const ASCII: [&str; 10] = [" ", ".", ":", "-", "=", "+", "*", "#", "%", "@"];
const SHADES: [&str; 5] = [" ", "░", "▒", "▓", "█"];

/// What a cell is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Glyphs {
    /// Half-blocks with their two colours; [`SHADES`] where there are no colours.
    Blocks,
    /// [`ASCII`], coloured where there are colours.
    Ascii,
}

/// The colours a terminal shows, or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Colours {
    Rgb,
    /// xterm's 256: every terminal that has any colours worth drawing with takes these escape
    /// codes, and tmux and the Linux console turn them into what they have.
    Palette,
    None,
}

impl Colours {
    /// What the environment says: `NO_COLOR` (<https://no-color.org>: set and not empty) first, then
    /// `COLORTERM`, or tmux that the terminal outside it takes 24-bit colours, then `TERM`.
    pub(crate) fn from_env(no_color: bool, colorterm: Option<&str>, term: Option<&str>, tmux_rgb: bool) -> Self {
        let term = term.unwrap_or_default();
        let says_24_bit = colorterm.is_some_and(|c| c.eq_ignore_ascii_case("truecolor") || c.eq_ignore_ascii_case("24bit"));
        if no_color {
            Self::None
        } else if says_24_bit || tmux_rgb || term.ends_with("-direct") || super::detect::term_name_says_kitty(term) {
            Self::Rgb
        } else if super::detect::term_name_says_plain(Some(term)) {
            Self::None
        } else {
            Self::Palette
        }
    }

    /// `rgb` as this terminal can colour it.
    fn ink(self, [r, g, b]: [u8; 3]) -> Option<Ink> {
        match self {
            Self::Rgb => Some(Ink::Rgb([r, g, b])),
            Self::Palette => Some(Ink::Palette(palette(r, g, b))),
            Self::None => None,
        }
    }
}

/// The terminal's background, which a character's ink shows against. Half-blocks colour the whole
/// cell, so they don't depend on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ground {
    Dark,
    Light,
}

/// How pages are drawn as text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Style {
    pub(crate) glyphs: Glyphs,
    pub(crate) colours: Colours,
    pub(crate) ground: Ground,
}

impl Style {
    /// Whether cells are half-blocks, two samples each.
    fn halves(self) -> bool {
        self.glyphs == Glyphs::Blocks && self.colours != Colours::None
    }

    /// Whether what is drawn depends on [`Ground`], so it is worth asking the terminal.
    pub(crate) fn inks(self) -> bool {
        !self.halves()
    }

    /// The character for a sample of `rgb`, and its colour: as much ink as the sample is bright,
    /// on a dark background, or dark, on a light one.
    fn glyph(self, rgb: [u8; 3]) -> (&'static str, Option<Ink>) {
        let ramp: &[&'static str] = if self.glyphs == Glyphs::Ascii { &ASCII } else { &SHADES };
        let (ink, colour) = match self.colours {
            Colours::None => (plain_ink(rgb, self.ground), rgb),
            _ => coloured_ink(rgb, self.ground),
        };
        let steps = ramp.len().saturating_sub(1);
        let at = (usize::from(ink).saturating_mul(steps).saturating_add(127)) / 255;
        (ramp.get(at).copied().unwrap_or(" "), self.colours.ink(colour))
    }
}

/// How much ink, of 255, a sample of `rgb` takes in one colour.
fn plain_ink([r, g, b]: [u8; 3], ground: Ground) -> u8 {
    // Rec. 709's weights, which sum to 10000, so the result is at most 255 and the cast is exact.
    let luma = ((2126 * u32::from(r) + 7152 * u32::from(g) + 722 * u32::from(b)) / 10000) as u8;
    match ground {
        Ground::Dark => luma,
        Ground::Light => 255 - luma,
    }
}

/// How much ink, of 255, a sample of `rgb` takes, and in what colour, so that the ink mixed with
/// the background by how much of the cell it covers comes to `rgb`: over black, the sample at its
/// brightest (a dim red is a little bright red); over white, what is left of it once its white is
/// taken out (a pale blue is a little pure blue).
fn coloured_ink(rgb: [u8; 3], ground: Ground) -> (u8, [u8; 3]) {
    let [r, g, b] = rgb;
    let scaled = |c: u8, by: u8| (u16::from(c).saturating_mul(255) / u16::from(by.max(1))).min(255) as u8;
    match ground {
        Ground::Dark => {
            let most = r.max(g).max(b);
            (most, rgb.map(|c| scaled(c, most)))
        }
        Ground::Light => {
            let least = r.min(g).min(b);
            let ink = 255 - least;
            (ink, rgb.map(|c| scaled(c - least, ink)))
        }
    }
}

/// The nearest of xterm's 256 colours: its 6 × 6 × 6 cube (16 to 231) or its 24 greys (232 to 255).
fn palette(r: u8, g: u8, b: u8) -> u8 {
    const LEVELS: [i32; 6] = [0, 95, 135, 175, 215, 255];
    // Which of the levels is nearest: the thresholds are halfway between them.
    let step = |c: u8| match c {
        0..48 => 0u8,
        48..115 => 1,
        115..155 => 2,
        155..195 => 3,
        195..235 => 4,
        235.. => 5,
    };
    let (cr, cg, cb) = (step(r), step(g), step(b));
    // A third of three bytes fits one, so the cast is exact.
    let mean = ((u16::from(r) + u16::from(g) + u16::from(b)) / 3) as u8;
    let grey_step = mean.saturating_sub(3) / 10;
    let grey_step = grey_step.min(23);
    let grey = 8 + 10 * i32::from(grey_step);
    let distance = |to: [i32; 3]| {
        let d = [i32::from(r) - to[0], i32::from(g) - to[1], i32::from(b) - to[2]];
        d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
    };
    let level = |s: u8| LEVELS.get(usize::from(s)).copied().unwrap_or(0);
    let cube = [level(cr), level(cg), level(cb)];
    if distance([grey, grey, grey]) < distance(cube) { 232 + grey_step } else { 16 + 36 * cr + 6 * cg + cb }
}

/// A colour as the terminal is told it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ink {
    Rgb([u8; 3]),
    Palette(u8),
}

/// The colours a line has set so far, so that each is written only when it changes: a page has
/// long runs of one colour, and a white page is mostly nothing but spaces.
#[derive(Clone, Copy, Default)]
struct Pen {
    fg: Option<Ink>,
    bg: Option<Ink>,
}

impl Pen {
    fn cell(&mut self, line: &mut Vec<u8>, glyph: &str, fg: Option<Ink>, bg: Option<Ink>) {
        // A space shows no foreground: it stays as it is, for the next cell that shows one.
        let fg = if glyph == " " { self.fg } else { fg };
        if fg != self.fg || bg != self.bg {
            line.extend_from_slice(b"\x1b[");
            if fg != self.fg {
                colour_code(line, 38, fg);
            }
            if fg != self.fg && bg != self.bg {
                line.push(b';');
            }
            if bg != self.bg {
                colour_code(line, 48, bg);
            }
            line.push(b'm');
            (self.fg, self.bg) = (fg, bg);
        }
        line.extend_from_slice(glyph.as_bytes());
    }

    /// The two halves of a cell: `▀` for what is in the top half, on what is in the bottom half.
    /// A half with no colour shows the terminal's background.
    fn halves(&mut self, line: &mut Vec<u8>, top: Option<Ink>, bottom: Option<Ink>) {
        match (top, bottom) {
            (top, bottom) if top == bottom => self.cell(line, " ", None, top),
            (Some(top), bottom) => self.cell(line, "▀", Some(top), bottom),
            (None, bottom) => self.cell(line, "▄", bottom, None),
        }
    }

    /// The line's end: the colours go back, so nothing after it is drawn with them.
    fn finish(self, line: &mut Vec<u8>) {
        if self.fg.is_some() || self.bg.is_some() {
            line.extend_from_slice(b"\x1b[0m");
        }
    }
}

/// The parameters that set a colour: `base` 38 for the foreground, 48 for the background, and one
/// more than that for the terminal's own. Writing to a `Vec` doesn't fail.
fn colour_code(line: &mut Vec<u8>, base: u8, ink: Option<Ink>) {
    let _ = match ink {
        None => write!(line, "{}", base.saturating_add(1)),
        Some(Ink::Rgb([r, g, b])) => write!(line, "{base};2;{r};{g};{b}"),
        Some(Ink::Palette(n)) => write!(line, "{base};5;{n}"),
    };
}

/// The cell pages are fitted with when they are drawn as text: the terminal's cell's shape (taken
/// within reason, since the terminal may report anything) in the fewest pixels that give each
/// sample one, so a page renders no larger than what is drawn, and a picture that keeps its natural
/// size gets a column for each of its pixels across.
pub(crate) fn sampling_cell(terminal: Cell) -> Cell {
    let shape = terminal.h / terminal.w;
    let shape = if shape.is_finite() { shape.clamp(1.0, 4.0) } else { 2.0 };
    let across = (2.0 / shape).max(1.0);
    Cell { w: across, h: across * shape }
}

/// The pixels that sample `index` of `count` covers, of `whole` in a row or a column: a box of
/// them when they outnumber the samples, and at least one when they don't.
fn span(index: u32, count: u32, whole: u32) -> (usize, usize) {
    let at = |i: u32| u64::from(i).saturating_mul(u64::from(whole)) / u64::from(count.max(1));
    let start = at(index);
    let end = at(index.saturating_add(1)).max(start.saturating_add(1)).min(u64::from(whole));
    let size = |n: u64| usize::try_from(n).unwrap_or(usize::MAX);
    (size(start), size(end))
}

/// The colour of a box of `pixels`, premultiplied RGBA rows of `width`, or `None` when it is mostly
/// see-through: the terminal's background is unknown, so nothing can be mixed with it.
fn sample(pixels: &[[u8; 4]], width: usize, across: (usize, usize), down: (usize, usize)) -> Option<[u8; 3]> {
    let (mut sum, mut count) = ([0u64; 4], 0u64);
    for y in down.0..down.1 {
        let row = y.saturating_mul(width);
        let Some(pixels) = pixels.get(row.saturating_add(across.0)..row.saturating_add(across.1)) else { continue };
        for px in pixels {
            // At most 255 for each pixel of a buffer in memory, so these can't overflow.
            for (total, channel) in sum.iter_mut().zip(px) {
                *total += u64::from(*channel);
            }
            count += 1;
        }
    }
    let [r, g, b, a] = sum;
    if count == 0 || a / count < 128 {
        return None;
    }
    // Premultiplied, so a channel is at most the alpha, give or take rounding: `min` takes care of
    // that, and the cast is exact.
    let straight = |c: u64| (c.saturating_mul(255).saturating_add(a / 2) / a).min(255) as u8;
    Some([straight(r), straight(g), straight(b)])
}

/// A rendered page, premultiplied RGBA, as `rows` lines of `cols` cells each, every line leaving
/// the colours as it found them. Whatever size the page came back at, it covers all the cells.
pub(crate) fn lines(width: u32, height: u32, premultiplied: &[u8], cols: u16, rows: u16, style: Style) -> Result<Vec<Vec<u8>>, String> {
    check_raster(width, height, premultiplied)?;
    let cells = u64::from(cols).saturating_mul(u64::from(rows));
    if cells == 0 {
        return Err("no cells to draw the page in".into());
    }
    if cells > MAX_CELLS {
        return Err(format!("{cols} × {rows} cells are too many to draw the page in"));
    }
    let pixels = premultiplied.as_chunks::<4>().0;
    let row_width = usize::try_from(width).map_err(|e| e.to_string())?;
    let (cols, rows) = (u32::from(cols), u32::from(rows));
    let across: Vec<(usize, usize)> = (0..cols).map(|col| span(col, cols, width)).collect();
    let samples_down = if style.halves() { rows.saturating_mul(2) } else { rows };
    let mut out = Vec::with_capacity(rows as usize);
    for row in 0..rows {
        let mut line = Vec::with_capacity(cols as usize);
        let mut pen = Pen::default();
        if style.halves() {
            let top = span(row.saturating_mul(2), samples_down, height);
            let bottom = span(row.saturating_mul(2).saturating_add(1), samples_down, height);
            for x in &across {
                let ink = |down| sample(pixels, row_width, *x, down).and_then(|rgb| style.colours.ink(rgb));
                pen.halves(&mut line, ink(top), ink(bottom));
            }
        } else {
            let down = span(row, samples_down, height);
            for x in &across {
                let (glyph, ink) = sample(pixels, row_width, *x, down).map_or((" ", None), |rgb| style.glyph(rgb));
                pen.cell(&mut line, glyph, ink, None);
            }
        }
        pen.finish(&mut line);
        out.push(line);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const CLEAR: [u8; 4] = [0, 0, 0, 0];
    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    const BLOCKS: Style = Style { glyphs: Glyphs::Blocks, colours: Colours::Rgb, ground: Ground::Dark };
    const ASCII_PLAIN: Style = Style { glyphs: Glyphs::Ascii, colours: Colours::None, ground: Ground::Dark };

    /// `pixels`, rows of `width`, as text, every line joined by a newline.
    fn drawn(pixels: &[[u8; 4]], width: u32, cols: u16, rows: u16, style: Style) -> String {
        let bytes = pixels.concat();
        let height = pixels.len() as u32 / width;
        let lines = lines(width, height, &bytes, cols, rows, style).unwrap();
        lines.iter().map(|line| String::from_utf8(line.clone()).unwrap()).collect::<Vec<_>>().join("\n")
    }

    /// The cells of a line, its escape codes taken out.
    fn visible(line: &str) -> String {
        let (mut out, mut chars) = (String::new(), line.chars());
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                chars.by_ref().find(|c| *c == 'm');
            } else {
                out.push(c);
            }
        }
        out
    }

    /// Whether a line's escape codes leave no colour set at its end.
    fn leaves_no_colour(line: &str) -> bool {
        let (mut fg, mut bg) = (false, false);
        for code in line.split('\x1b').skip(1) {
            let params = code.strip_prefix('[').and_then(|code| code.split_once('m')).map_or("", |(params, _)| params);
            let mut params = params.split(';');
            while let Some(param) = params.next() {
                match param {
                    "0" => (fg, bg) = (false, false),
                    "39" => fg = false,
                    "49" => bg = false,
                    "38" | "48" => {
                        *(if param == "38" { &mut fg } else { &mut bg }) = true;
                        // `5;n` or `2;r;g;b`.
                        let skip = if params.next() == Some("5") { 1 } else { 3 };
                        params.by_ref().take(skip).for_each(drop);
                    }
                    other => panic!("{other} in {line:?}"),
                }
            }
        }
        !fg && !bg
    }

    #[test]
    fn a_half_block_is_its_top_on_its_bottom() {
        assert_eq!(drawn(&[RED, BLUE], 1, 1, 1, BLOCKS), "\x1b[38;2;255;0;0;48;2;0;0;255m▀\x1b[0m");
    }

    #[test]
    fn a_colour_is_written_only_when_it_changes() {
        // Two cells alike, and a third whose bottom half alone differs.
        let page = [RED, RED, RED, BLUE, BLUE, RED];
        let line = drawn(&page, 3, 3, 1, BLOCKS);
        assert_eq!(line, "\x1b[38;2;255;0;0;48;2;0;0;255m▀▀\x1b[48;2;255;0;0m \x1b[0m");
    }

    #[test]
    fn a_cell_of_one_colour_is_a_space_on_it() {
        // Which leaves the foreground out altogether: a white page is little more than spaces.
        assert_eq!(drawn(&[WHITE, WHITE, WHITE, WHITE], 2, 2, 1, BLOCKS), "\x1b[48;2;255;255;255m  \x1b[0m");
    }

    #[test]
    fn what_is_see_through_shows_the_terminals_background() {
        assert_eq!(drawn(&[RED, CLEAR], 1, 1, 1, BLOCKS), "\x1b[38;2;255;0;0m▀\x1b[0m");
        assert_eq!(drawn(&[CLEAR, RED], 1, 1, 1, BLOCKS), "\x1b[38;2;255;0;0m▄\x1b[0m");
        // No colours set, so none to put back.
        assert_eq!(drawn(&[CLEAR, CLEAR], 1, 1, 1, BLOCKS), " ");
    }

    #[test]
    fn colours_go_back_between_a_full_cell_and_a_see_through_one() {
        // Its last cell has no colours of its own, so the line ends with none set.
        assert_eq!(drawn(&[RED, CLEAR, RED, CLEAR], 2, 2, 1, BLOCKS), "\x1b[48;2;255;0;0m \x1b[49m ");
    }

    #[test]
    fn premultiplied_pixels_come_back_to_their_colour() {
        // Half opaque, half red: premultiplied, 128 of 255 for the red it keeps.
        let half_red = [128, 0, 0, 128];
        assert_eq!(drawn(&[half_red, half_red], 1, 1, 1, BLOCKS), "\x1b[48;2;255;0;0m \x1b[0m");
        // Less than half opaque is the terminal's background.
        let faint = [100, 0, 0, 100];
        assert_eq!(drawn(&[faint, faint], 1, 1, 1, BLOCKS), " ");
    }

    #[test]
    fn a_box_of_pixels_is_their_average() {
        let page = [WHITE, BLACK, BLACK, WHITE, WHITE, BLACK, BLACK, WHITE];
        // Four pixels to each half: a grey as even as it comes.
        assert_eq!(drawn(&page, 2, 1, 1, BLOCKS), "\x1b[48;2;128;128;128m \x1b[0m");
    }

    #[test]
    fn fewer_pixels_than_samples_still_cover_every_cell() {
        let line = drawn(&[RED], 1, 3, 2, BLOCKS);
        assert_eq!(visible(&line), "   \n   ");
    }

    #[test]
    fn the_palette_picks_the_nearest_colour() {
        // The cube's corners and the greys, as xterm numbers them.
        assert_eq!(palette(0, 0, 0), 16);
        assert_eq!(palette(255, 255, 255), 231);
        assert_eq!(palette(255, 0, 0), 196);
        assert_eq!(palette(0, 255, 0), 46);
        assert_eq!(palette(0, 0, 255), 21);
        assert_eq!(palette(128, 128, 128), 244);
        assert_eq!(palette(8, 8, 8), 232);
        assert_eq!(palette(238, 238, 238), 255);
        // A cube's exact colour stays itself.
        assert_eq!(palette(95, 135, 175), 16 + 36 + 2 * 6 + 3);
        let style = Style { colours: Colours::Palette, ..BLOCKS };
        assert_eq!(drawn(&[RED, BLUE], 1, 1, 1, style), "\x1b[38;5;196;48;5;21m▀\x1b[0m");
    }

    #[test]
    fn colours_alike_in_the_palette_are_one_colour() {
        let (near_a, near_b) = ([250, 0, 0, 255], [255, 2, 2, 255]);
        let style = Style { colours: Colours::Palette, ..BLOCKS };
        assert_eq!(drawn(&[near_a, near_b], 1, 1, 1, style), "\x1b[48;5;196m \x1b[0m");
    }

    #[test]
    fn plain_ascii_is_nothing_but_ascii() {
        let page = [BLACK, [128, 128, 128, 255], WHITE, CLEAR];
        assert_eq!(drawn(&page, 4, 4, 1, ASCII_PLAIN), " +@ ");
    }

    #[test]
    fn on_a_light_background_more_ink_is_darker() {
        let page = [BLACK, [128, 128, 128, 255], WHITE, CLEAR];
        let style = Style { ground: Ground::Light, ..ASCII_PLAIN };
        assert_eq!(drawn(&page, 4, 4, 1, style), "@=  ");
    }

    #[test]
    fn coloured_ascii_inks_with_the_colour_at_its_brightest_or_its_purest() {
        let dim = [64, 0, 0, 255];
        let style = Style { colours: Colours::Rgb, ..ASCII_PLAIN };
        assert_eq!(drawn(&[dim], 1, 1, 1, style), "\x1b[38;2;255;0;0m:\x1b[0m");
        let pale = [191, 191, 255, 255];
        let light = Style { ground: Ground::Light, ..style };
        assert_eq!(drawn(&[pale], 1, 1, 1, light), "\x1b[38;2;0;0;255m:\x1b[0m");
        // Nothing to ink: no colour is set either.
        assert_eq!(drawn(&[BLACK], 1, 1, 1, style), " ");
        assert_eq!(drawn(&[WHITE], 1, 1, 1, light), " ");
    }

    #[test]
    fn blocks_without_colours_shade() {
        let page = [BLACK, [64, 64, 64, 255], [128, 128, 128, 255], [191, 191, 191, 255], WHITE];
        let style = Style { colours: Colours::None, ..BLOCKS };
        assert_eq!(drawn(&page, 5, 5, 1, style), " ░▒▓█");
    }

    #[test]
    fn the_cell_keeps_the_terminals_shape_with_a_pixel_for_each_sample() {
        assert_eq!(sampling_cell(Cell { w: 10.0, h: 20.0 }), Cell { w: 1.0, h: 2.0 });
        assert_eq!(sampling_cell(Cell { w: 18.0, h: 46.0 }), Cell { w: 1.0, h: 46.0 / 18.0 });
        // Squatter than two to one: more than a pixel across, for two down.
        assert_eq!(sampling_cell(Cell { w: 10.0, h: 16.0 }), Cell { w: 1.25, h: 2.0 });
        // What a terminal shouldn't say.
        assert_eq!(sampling_cell(Cell { w: 1.0, h: 1000.0 }), Cell { w: 1.0, h: 4.0 });
        assert_eq!(sampling_cell(Cell { w: 1000.0, h: 1.0 }), Cell { w: 2.0, h: 2.0 });
        assert_eq!(sampling_cell(Cell { w: 0.0, h: 0.0 }), Cell { w: 1.0, h: 2.0 });
    }

    #[test]
    fn the_environment_says_which_colours() {
        let from = Colours::from_env;
        assert_eq!(from(false, Some("truecolor"), Some("xterm-256color"), false), Colours::Rgb);
        assert_eq!(from(false, Some("24bit"), None, false), Colours::Rgb);
        assert_eq!(from(false, None, Some("xterm-256color"), false), Colours::Palette);
        assert_eq!(from(false, None, Some("screen"), false), Colours::Palette);
        // tmux knows for the terminal outside it.
        assert_eq!(from(false, None, Some("tmux-256color"), true), Colours::Rgb);
        assert_eq!(from(false, None, Some("xterm-direct"), false), Colours::Rgb);
        assert_eq!(from(false, None, Some("xterm-kitty"), false), Colours::Rgb);
        for term in [Some("dumb"), Some("vt100"), Some(""), None] {
            assert_eq!(from(false, None, term, false), Colours::None, "{term:?}");
        }
        // NO_COLOR over all of them.
        assert_eq!(from(true, Some("truecolor"), Some("xterm-kitty"), true), Colours::None);
    }

    #[test]
    fn too_many_cells_are_an_error_not_an_allocation() {
        let e = lines(1, 1, &WHITE, u16::MAX, u16::MAX, BLOCKS).unwrap_err();
        assert!(e.contains("too many"), "{e}");
        assert!(lines(1, 1, &WHITE, 0, 1, BLOCKS).is_err());
        assert!(lines(1, 1, &WHITE, 1, 0, BLOCKS).is_err());
    }

    #[test]
    fn pixels_that_arent_the_size_they_say_are_an_error() {
        assert!(lines(2, 2, &WHITE, 1, 1, BLOCKS).is_err());
        assert!(lines(0, 0, &[], 1, 1, BLOCKS).is_err());
        assert!(lines(u32::MAX, u32::MAX, &WHITE, 1, 1, BLOCKS).is_err());
    }

    fn any_style() -> impl Strategy<Value = Style> {
        (
            prop::sample::select(vec![Glyphs::Blocks, Glyphs::Ascii]),
            prop::sample::select(vec![Colours::Rgb, Colours::Palette, Colours::None]),
            any::<bool>(),
        )
            .prop_map(|(glyphs, colours, light)| Style { glyphs, colours, ground: if light { Ground::Light } else { Ground::Dark } })
    }

    proptest! {
        /// Whatever the pixels and however they come out of the cells asked for: as many lines as
        /// rows, as many cells as columns, the colours put back, nothing but what is drawn with.
        #[test]
        fn any_page_fills_its_cells(
            (width, height, bytes) in (1u32..24, 1u32..24).prop_flat_map(|(w, h)| {
                (Just(w), Just(h), prop::collection::vec(any::<u8>(), (w * h * 4) as usize))
            }),
            cols in 1u16..40,
            rows in 1u16..40,
            style in any_style(),
        ) {
            let lines = lines(width, height, &bytes, cols, rows, style).unwrap();
            prop_assert_eq!(lines.len(), usize::from(rows));
            for line in lines {
                let line = String::from_utf8(line).unwrap();
                let cells = visible(&line);
                prop_assert_eq!(cells.chars().count(), usize::from(cols), "{:?}", line);
                prop_assert!(cells.chars().all(|c| " ▀▄░▒▓█.:-=+*#%@".contains(c)), "{:?}", line);
                prop_assert!(leaves_no_colour(&line), "{:?}", line);
                if style.colours == Colours::None {
                    prop_assert!(!line.contains('\x1b'), "{:?}", line);
                }
                if style.glyphs == Glyphs::Ascii {
                    prop_assert!(cells.is_ascii(), "{:?}", line);
                }
            }
        }

        #[test]
        fn the_palette_is_never_further_than_a_step_of_the_cube(r in any::<u8>(), g in any::<u8>(), b in any::<u8>()) {
            let code = palette(r, g, b);
            prop_assert!(code >= 16);
            let colour = if code >= 232 {
                let grey = 8 + 10 * i32::from(code - 232);
                [grey, grey, grey]
            } else {
                let at = code - 16;
                let level = |s: u8| [0, 95, 135, 175, 215, 255][usize::from(s)];
                [level(at / 36), level(at / 6 % 6), level(at % 6)]
            };
            for (c, near) in [r, g, b].iter().zip(colour) {
                prop_assert!((i32::from(*c) - near).abs() <= 48, "{r},{g},{b} → {code}");
            }
        }

        /// The spans of a row or column of pixels cover it all, in order, when samples are fewer.
        #[test]
        fn spans_cover_what_they_divide(whole in 1u32..5000, count in 1u32..5000) {
            let spans: Vec<_> = (0..count).map(|i| span(i, count, whole)).collect();
            prop_assert_eq!(spans.first().map(|s| s.0), Some(0));
            prop_assert_eq!(spans.last().map(|s| s.1), Some(whole as usize));
            for s in &spans {
                prop_assert!(s.0 < s.1 && s.1 <= whole as usize, "{:?}", s);
            }
            if count <= whole {
                for pair in spans.windows(2) {
                    prop_assert_eq!(pair[0].1, pair[1].0);
                }
            }
        }
    }
}
