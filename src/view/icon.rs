//! The icon `--help` shows to the right of its first lines, where the terminal is known to show
//! it: a prompt and the picture it brings up (`assets/icon.svg`, made into the PNG by
//! `assets/README.md`'s command).
//!
//! It is a decoration, so it never costs the help anything. Nothing is asked of the terminal: no
//! wait, and no query for a terminal that may print it. Where anything is unknown, or there is no
//! room, the help is the same text without it.

use std::io::IsTerminal as _;

use super::fit::{Cell, FALLBACK_CELL};
use super::kitty::{self, Wrap};
use super::tty::{Tty, WindowSize};
use super::{HELP_WIDTH, Output, Protocol, chars, cursor_by, detect, tmux, tmux_colour};

/// The icon, square, with straight alpha. A PNG goes to the terminal as it is, so this needs no
/// encoder and no backend, and every build has it.
const PNG: &[u8] = include_bytes!("../../assets/icon.png");

/// The rows it covers where the window has room: the help's first lines, down to its usage.
const MOST_ROWS: u16 = 5;

/// The fewest rows it is worth showing at: below this, it's a smudge.
const FEWEST_ROWS: u16 = 3;

/// The columns kept free between the longest line beside it and the icon.
const GAP: usize = 2;

/// Picks the image id inside tmux ([`kitty::IdColor::id`]): "clv" in ASCII. Always the same, so
/// each `--help` replaces the last one's image rather than adding to what the terminal keeps,
/// and nothing needs recording for `sweep.rs` to delete: at most one icon ever stays.
const ID_INDEX: u64 = 0x63_6C76;

/// Where the icon goes: `across` columns from the left, `cols` × `rows` cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Place {
    across: u16,
    cols: u16,
    rows: u16,
}

/// `text`, the whole help, as it goes to stdout: with the icon where this terminal shows it,
/// otherwise as it is.
pub(super) fn with_icon(text: &str) -> Vec<u8> {
    known_output().and_then(|(output, cell)| drawn(text, output, cell)).unwrap_or_else(|| text.as_bytes().to_vec())
}

/// How the icon reaches this terminal, and the cell size if tmux says it, when stdout is a
/// terminal known to show it without asking: from the environment, or inside tmux from tmux.
fn known_output() -> Option<(Output, Option<Cell>)> {
    if !std::io::stdout().is_terminal() {
        return None;
    }
    if tmux::inside() {
        let client = tmux::client();
        let colour = tmux_colour(Protocol::Auto, &client).ok()?;
        if !tmux::passthrough().ok()? {
            return None;
        }
        let cell = client.ok()?.cell.map(|(w, h)| Cell { w: f32::from(w), h: f32::from(h) });
        return Some((Output::Tmux(colour), cell));
    }
    let var = |name| std::env::var(name).ok();
    let known = detect::env_says_kitty(var("TERM").as_deref(), var("TERM_PROGRAM").as_deref(), std::env::var_os("KITTY_WINDOW_ID").is_some());
    known.then_some((Output::Direct, None))
}

/// `text` with the icon, in this window; `None` when the window is unknown or has no room.
fn drawn(text: &str, output: Output, cell: Option<Cell>) -> Option<Vec<u8>> {
    let window = Tty::open().ok()?.size()?;
    let cell = Cell::from_window(window.cols, window.rows, window.xpixel, window.ypixel).or(cell).unwrap_or(FALLBACK_CELL);
    let place = place(text, window, cell)?;
    written(text, place, output)
}

/// Where the icon goes beside `text`'s first lines: its right edge at the help's width, or the
/// window's if that is less, as many rows as leave [`GAP`] from the longest line beside it. The
/// lines beside it don't wrap, so they are as many rows as lines. `None` when even
/// [`FEWEST_ROWS`] don't fit, or `text` is too short.
fn place(text: &str, window: WindowSize, cell: Cell) -> Option<Place> {
    let right = usize::from(window.cols).min(HELP_WIDTH);
    (FEWEST_ROWS..=MOST_ROWS).rev().find_map(|rows| {
        // The line the cursor comes back to needs a row too, below the lines beside the icon.
        if window.rows <= rows || newline_after(text, rows).is_none() {
            return None;
        }
        // Square, whatever shape the cells are; `as` saturates, and a NaN gives 0.
        let cols = (f32::from(rows) * cell.h / cell.w).round() as u16;
        let across = right.checked_sub(usize::from(cols)).filter(|_| cols > 0)?;
        let longest = text.lines().take(usize::from(rows)).map(chars).max()?;
        (longest.saturating_add(GAP) <= across).then_some(Place { across: u16::try_from(across).ok()?, cols, rows })
    })
}

/// Where the line `rows` lines into `text` ends, its newline included; `None` when it has fewer.
fn newline_after(text: &str, rows: u16) -> Option<usize> {
    text.match_indices('\n').nth(usize::from(rows).checked_sub(1)?).map(|(at, _)| at.saturating_add(1))
}

/// `text` with the icon at `place`. The lines beside it go first, so that any scrolling is done
/// before the cursor goes back up to them; then the cursor comes back for the rest.
fn written(text: &str, place: Place, output: Output) -> Option<Vec<u8>> {
    let (top, rest) = text.split_at_checked(newline_after(text, place.rows)?)?;
    let mut out = Vec::with_capacity(text.len().saturating_add(PNG.len().saturating_mul(2)));
    out.extend_from_slice(top.as_bytes());
    cursor_by(&mut out, place.rows, 'A');
    match output {
        Output::Direct => {
            out.push(b'\r');
            cursor_by(&mut out, place.across, 'C');
            kitty::show_png_in_place(&mut out, PNG, place.cols, place.rows);
            cursor_by(&mut out, place.rows, 'B');
        }
        Output::Tmux(colour) => {
            let id = colour.id(ID_INDEX);
            kitty::transmit(&mut out, id, PNG, Wrap::Tmux);
            kitty::place_virtual(&mut out, id, place.cols, place.rows, Wrap::Tmux);
            for row in 0..place.rows {
                out.push(b'\r');
                cursor_by(&mut out, place.across, 'C');
                // `place` keeps it within the help's width, far inside the placeholders' table.
                if !kitty::placeholder_row(&mut out, id, colour, row, place.cols) {
                    return None;
                }
                cursor_by(&mut out, 1, 'B');
            }
        }
        // Text cells are for terminals without images, which get no icon.
        Output::Text(_) => return None,
    }
    out.push(b'\r');
    out.extend_from_slice(rest.as_bytes());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::super::kitty::IdColor;
    use super::*;

    const CELL: Cell = Cell { w: 10.0, h: 20.0 };

    fn window(cols: u16, rows: u16) -> WindowSize {
        WindowSize { cols, rows, xpixel: 0, ypixel: 0 }
    }

    /// A help whose first five lines are at most 69 characters long, as the real one's are.
    fn help() -> String {
        format!("clview\n{}\n\nclv [options]\nclv --help\n\nthe rest\nand more\n", "x".repeat(69))
    }

    #[test]
    fn the_icon_is_a_square_png() {
        assert_eq!(PNG.get(..8), Some(&b"\x89PNG\r\n\x1a\n"[..]));
        let side = |at: usize| u32::from_be_bytes(PNG[at..at + 4].try_into().unwrap());
        assert_eq!(side(16), side(20), "cols are worked out from a square");
        assert!(side(16) >= 128, "{} pixels would blur at five rows", side(16));
    }

    #[test]
    fn its_right_edge_is_the_helps_width_or_the_windows() {
        assert_eq!(place(&help(), window(170, 40), CELL), Some(Place { across: 90, cols: 10, rows: 5 }));
        assert_eq!(place(&help(), window(100, 40), CELL), Some(Place { across: 90, cols: 10, rows: 5 }));
        assert_eq!(place(&help(), window(85, 40), CELL), Some(Place { across: 75, cols: 10, rows: 5 }));
    }

    #[test]
    fn its_columns_keep_it_square_in_cells_of_any_shape() {
        assert_eq!(place(&help(), window(170, 40), Cell { w: 15.0, h: 40.0 }), Some(Place { across: 87, cols: 13, rows: 5 }));
        assert_eq!(place(&help(), window(170, 40), Cell { w: 9.0, h: 20.0 }), Some(Place { across: 89, cols: 11, rows: 5 }));
    }

    #[test]
    fn it_shrinks_to_leave_the_lines_room_then_goes() {
        assert_eq!(place(&help(), window(81, 40), CELL), Some(Place { across: 71, cols: 10, rows: 5 }));
        assert_eq!(place(&help(), window(80, 40), CELL), Some(Place { across: 72, cols: 8, rows: 4 }), "the classic width");
        assert_eq!(place(&help(), window(78, 40), CELL), Some(Place { across: 72, cols: 6, rows: 3 }));
        assert_eq!(place(&help(), window(76, 40), CELL), None);
        assert_eq!(place(&help(), window(20, 40), CELL), None);
    }

    #[test]
    fn it_leaves_a_row_for_the_cursor_to_come_back_to() {
        assert_eq!(place(&help(), window(170, 6), CELL).map(|p| p.rows), Some(5));
        assert_eq!(place(&help(), window(170, 5), CELL).map(|p| p.rows), Some(4));
        assert_eq!(place(&help(), window(170, 4), CELL).map(|p| p.rows), Some(3));
        assert_eq!(place(&help(), window(170, 3), CELL), None);
    }

    #[test]
    fn it_only_goes_beside_lines_that_are_there() {
        assert_eq!(place("one\ntwo\nthree\nfour\n", window(170, 40), CELL).map(|p| p.rows), Some(4));
        assert_eq!(place("one\ntwo\n", window(170, 40), CELL), None);
        assert_eq!(place("", window(170, 40), CELL), None);
    }

    #[test]
    fn cells_of_no_sensible_shape_show_no_icon() {
        for (w, h) in [(1.0, 65535.0), (65535.0, 1.0), (0.0, 20.0), (0.0, 0.0), (f32::NAN, 20.0)] {
            assert_eq!(place(&help(), window(170, 40), Cell { w, h }), None, "{w} × {h}");
        }
    }

    /// The help as `--help` prints it now: if a line beside the icon grew, the icon would shrink
    /// or go in every window, without anything saying so.
    #[test]
    fn the_real_help_has_room_for_all_of_it_in_its_own_width() {
        let help = crate::view::usage(&[]);
        assert_eq!(place(&help, window(100, 40), CELL).map(|p| p.rows), Some(MOST_ROWS));
    }

    #[test]
    fn directly_the_icon_goes_up_beside_the_lines_and_the_cursor_comes_back() {
        let help = help();
        let place = Place { across: 90, cols: 10, rows: 5 };
        let out = written(&help, place, Output::Direct).unwrap();
        let top = format!("clview\n{}\n\nclv [options]\nclv --help\n", "x".repeat(69));
        let before = format!("{top}\x1b[5A\r\x1b[90C\x1b_Ga=T,f=100,t=d,q=2,C=1,c=10,r=5,m=1;");
        assert!(out.starts_with(before.as_bytes()), "{:?}", String::from_utf8_lossy(&out[..before.len()]));
        assert!(out.ends_with(b"\x1b\\\x1b[5B\r\nthe rest\nand more\n"), "{:?}", String::from_utf8_lossy(&out[out.len() - 40..]));
    }

    #[test]
    fn inside_tmux_it_is_placeholder_cells_beside_the_lines_with_the_image_passed_through() {
        let help = help();
        let place = Place { across: 90, cols: 10, rows: 5 };
        for colour in [IdColor::Rgb, IdColor::Indexed] {
            let out = String::from_utf8(written(&help, place, Output::Tmux(colour)).unwrap()).unwrap();
            let id = colour.id(ID_INDEX);
            let top = format!("clview\n{}\n\nclv [options]\nclv --help\n", "x".repeat(69));
            assert!(out.starts_with(&format!("{top}\x1b[5A\x1bPtmux;\x1b\x1b_Ga=t,f=100,t=d,q=2,i={id},m=1;")), "{colour:?}");
            assert!(out.contains(&format!("\x1b\x1b_Ga=p,U=1,q=2,i={id},c=10,r=5\x1b\x1b\\\x1b\\")), "{colour:?}");
            for row in 0..5 {
                let mut cells = Vec::new();
                assert!(kitty::placeholder_row(&mut cells, id, colour, row, 10));
                let line = format!("\r\x1b[90C{}\x1b[1B", String::from_utf8(cells).unwrap());
                assert!(out.contains(&line), "{colour:?}: row {row}");
            }
            assert!(out.ends_with("\x1b[1B\r\nthe rest\nand more\n"), "{colour:?}");
        }
    }
}
