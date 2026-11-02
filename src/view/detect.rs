//! Does this terminal speak the kitty graphics protocol, and how big are its cells? Pure
//! decisions from what the environment says and from what the terminal answers; asking it is the
//! caller's job.

/// What the environment says, without asking the terminal: kitty sets `TERM=xterm-kitty` and
/// `KITTY_WINDOW_ID`, Ghostty `TERM=xterm-ghostty` and `TERM_PROGRAM=ghostty`.
pub(crate) fn env_says_kitty(term: Option<&str>, term_program: Option<&str>, kitty_window_id: bool) -> bool {
    kitty_window_id || term.is_some_and(term_name_says_kitty) || term_program.is_some_and(|p| p.eq_ignore_ascii_case("ghostty"))
}

/// A terminal name (`TERM`, or tmux's `#{client_termname}`) of a terminal known to speak it.
pub(crate) fn term_name_says_kitty(name: &str) -> bool {
    matches!(name.trim(), "xterm-kitty" | "xterm-ghostty")
}

/// A terminal name of one that takes no escape codes to speak of (`dumb`, such as Emacs' shell; a
/// real VT; or none at all), which would show a question instead of answering it, and has no use
/// for a pager's cursor moves. Inside tmux, `TERM` is tmux's, which takes them.
pub(crate) fn term_name_says_plain(name: Option<&str>) -> bool {
    let name = name.unwrap_or_default().trim();
    name.is_empty() || name == "dumb" || name.starts_with("vt")
}

/// The query the protocol recommends: a 1×1 RGB image the terminal checks but doesn't keep
/// (`a=q`, id 31), then a Primary Device Attributes request, which every terminal answers. A
/// graphics reply before the attributes reply means the protocol is supported.
pub(crate) const QUERY: &[u8] = b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c";

/// The most reply bytes worth reading: both replies together are a few dozen.
pub(crate) const MAX_REPLY: usize = 1024;

/// What the terminal has answered to [`QUERY`] so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reply {
    /// A graphics reply for id 31 came before the device attributes reply.
    pub(crate) graphics: bool,
    /// The device attributes reply came: nothing more is coming.
    pub(crate) done: bool,
}

/// Read the replies in `bytes`. Anything else in there (keys the user typed, garbage) is ignored.
pub(crate) fn parse_reply(bytes: &[u8]) -> Reply {
    let attributes = find_attributes(bytes);
    let graphics = find_graphics(bytes).is_some_and(|g| attributes.is_none_or(|a| g < a));
    Reply { graphics, done: attributes.is_some() }
}

/// Asks for the cell size in pixels (XTWINOPS 16, from xterm's control sequences), for a
/// terminal whose tty reports no pixel size, then for the device attributes again: a terminal
/// that doesn't know the first still answers the second, so the wait ends at once.
pub(crate) const CELL_QUERY: &[u8] = b"\x1b[16t\x1b[c";

/// What the terminal has answered to [`CELL_QUERY`] so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CellReply {
    /// The cell's width and height in pixels, from an `ESC [ 6 ; height ; width t` before the
    /// device attributes reply.
    pub(crate) cell: Option<(u16, u16)>,
    /// The device attributes reply came: nothing more is coming.
    pub(crate) done: bool,
}

/// Read the replies to [`CELL_QUERY`] in `bytes`. Anything else in there is ignored.
pub(crate) fn parse_cell_reply(bytes: &[u8]) -> CellReply {
    let attributes = find_attributes(bytes);
    let before = bytes.get(..attributes.unwrap_or(bytes.len())).unwrap_or_default();
    CellReply { cell: find_cell(before), done: attributes.is_some() }
}

/// The first complete `ESC [ 6 ; height ; width t` in `bytes`, as (width, height). A size of 0,
/// or past `u16`, is no answer.
fn find_cell(bytes: &[u8]) -> Option<(u16, u16)> {
    let mut from = 0;
    while let Some(start) = find(bytes, b"\x1b[6;", from) {
        let params = start.saturating_add(4);
        let rest = bytes.get(params..).unwrap_or_default();
        let end = rest.iter().position(|b| !(b.is_ascii_digit() || *b == b';'))?;
        if rest.get(end) == Some(&b't') {
            let number = |n: &[u8]| std::str::from_utf8(n).ok()?.parse::<u16>().ok().filter(|v| *v > 0);
            let mut numbers = rest.get(..end).unwrap_or_default().split(|b| *b == b';');
            if let (Some(h), Some(w), None) = (numbers.next(), numbers.next(), numbers.next())
                && let (Some(h), Some(w)) = (number(h), number(w))
            {
                return Some((w, h));
            }
        }
        from = params;
    }
    None
}

/// Asks for the background colour (OSC 11, from xterm's control sequences), for what is drawn in
/// ink ([`super::cells::Ground`]), then for the device attributes, which end the wait.
pub(crate) const BACKGROUND_QUERY: &[u8] = b"\x1b]11;?\x1b\\\x1b[c";

/// What the terminal has answered to [`BACKGROUND_QUERY`] so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BackgroundReply {
    /// Whether its background is light, from an `ESC ] 11 ; rgb:RRRR/GGGG/BBBB` before the device
    /// attributes reply.
    pub(crate) light: Option<bool>,
    /// That colour, to a byte a channel.
    pub(crate) colour: Option<[u8; 3]>,
    /// The device attributes reply came: nothing more is coming.
    pub(crate) done: bool,
}

/// Read the replies to [`BACKGROUND_QUERY`] in `bytes`. Anything else in there is ignored.
pub(crate) fn parse_background_reply(bytes: &[u8]) -> BackgroundReply {
    let attributes = find_attributes(bytes);
    let before = bytes.get(..attributes.unwrap_or(bytes.len())).unwrap_or_default();
    let colour = find_background(before);
    BackgroundReply { light: colour.map(is_light), colour, done: attributes.is_some() }
}

/// Whether `colour` is a light one.
pub(crate) fn is_light([r, g, b]: [u8; 3]) -> bool {
    0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b) > 127.5
}

/// The colour of the first complete `ESC ] 11 ; rgb:R/G/B`, ended by ST or BEL. Each channel is one to
/// four hex digits, as X11 writes colours, a fraction of all of them set.
fn find_background(bytes: &[u8]) -> Option<[u8; 3]> {
    const START: &[u8] = b"\x1b]11;rgb:";
    let mut from = 0;
    while let Some(start) = find(bytes, START, from) {
        let channels = start.saturating_add(START.len());
        let rest = bytes.get(channels..).unwrap_or_default();
        let end = rest.iter().position(|b| !(b.is_ascii_hexdigit() || *b == b'/'))?;
        if matches!(rest.get(end..), Some([0x07, ..] | [0x1b, b'\\', ..])) {
            let fraction = |channel: &[u8]| {
                let digits = u32::try_from(channel.len()).ok().filter(|n| (1..=4).contains(n))?;
                let value = u32::from_str_radix(std::str::from_utf8(channel).ok()?, 16).ok()?;
                // At most all of them set, so at most a byte.
                Some((f64::from(value) * 255.0 / f64::from((1u32 << (digits * 4)) - 1)).round() as u8)
            };
            let mut channels = rest.get(..end).unwrap_or_default().split(|b| *b == b'/');
            if let (Some(r), Some(g), Some(b), None) = (channels.next(), channels.next(), channels.next(), channels.next())
                && let (Some(r), Some(g), Some(b)) = (fraction(r), fraction(g), fraction(b))
            {
                return Some([r, g, b]);
            }
        }
        from = channels;
    }
    None
}

/// Where a complete graphics reply for id 31 (`ESC _G … i=31 … ESC \`) starts.
fn find_graphics(bytes: &[u8]) -> Option<usize> {
    let mut from = 0;
    while let Some(start) = find(bytes, b"\x1b_G", from) {
        let body = start.saturating_add(3);
        let end = find(bytes, b"\x1b\\", body)?;
        let control = bytes.get(body..end)?.split(|b| *b == b';').next().unwrap_or_default();
        if control.split(|b| *b == b',').any(|key| key == b"i=31") {
            return Some(start);
        }
        from = end;
    }
    None
}

/// Where a complete Primary Device Attributes reply (`ESC [ ? <digits and ;> c`) starts.
fn find_attributes(bytes: &[u8]) -> Option<usize> {
    let mut from = 0;
    while let Some(start) = find(bytes, b"\x1b[?", from) {
        let params = start.saturating_add(3);
        let rest = bytes.get(params..).unwrap_or_default();
        match rest.iter().position(|b| !(b.is_ascii_digit() || *b == b';')) {
            Some(i) if rest.get(i) == Some(&b'c') => return Some(start),
            Some(_) => from = params,
            None => return None,
        }
    }
    None
}

/// The first `needle` in `haystack` at or after `from`.
fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    let rest = haystack.get(from..)?;
    rest.windows(needle.len()).position(|w| w == needle).map(|i| i.saturating_add(from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_background_says_its_colour_however_many_digits_it_has() {
        for (said, colour) in [
            ("rgb:1e1e/2e2e/3e3e", [0x1E, 0x2E, 0x3E]),
            ("rgb:1e/2e/3e", [0x1E, 0x2E, 0x3E]),
            ("rgb:1/2/3", [0x11, 0x22, 0x33]),
            ("rgb:0000/0000/0000", [0, 0, 0]),
            ("rgb:ffff/ffff/ffff", [255, 255, 255]),
        ] {
            for end in ["\x1b\\", "\x07"] {
                let reply = parse_background_reply(format!("\x1b]11;{said}{end}\x1b[?62;c").as_bytes());
                assert_eq!((reply.colour, reply.done), (Some(colour), true), "{said}");
                assert_eq!(reply.light, Some(is_light(colour)), "{said}");
            }
        }
        assert_eq!(parse_background_reply(b"\x1b[?62;c").colour, None, "it doesn't say");
    }

    const OK: &[u8] = b"\x1b_Gi=31;OK\x1b\\";
    const ATTRIBUTES: &[u8] = b"\x1b[?62;c";

    #[test]
    fn the_environment_names_kitty_and_ghostty() {
        assert!(env_says_kitty(Some("xterm-kitty"), None, false));
        assert!(env_says_kitty(Some("xterm-ghostty"), None, false));
        assert!(env_says_kitty(None, Some("ghostty"), false));
        assert!(env_says_kitty(Some("xterm-256color"), None, true));
        assert!(!env_says_kitty(Some("xterm-256color"), Some("Apple_Terminal"), false));
        assert!(!env_says_kitty(Some("tmux-256color"), Some("tmux"), false));
        assert!(!env_says_kitty(None, None, false));
    }

    #[test]
    fn some_terminals_take_no_escape_codes() {
        for plain in [None, Some(""), Some("dumb"), Some("vt100"), Some("vt220")] {
            assert!(term_name_says_plain(plain), "{plain:?}");
        }
        for not in [Some("xterm"), Some("xterm-256color"), Some("screen"), Some("tmux-256color"), Some("linux"), Some("xterm-kitty")] {
            assert!(!term_name_says_plain(not), "{not:?}");
        }
    }

    #[test]
    fn a_graphics_reply_before_the_attributes_means_supported() {
        assert_eq!(parse_reply(&[OK, ATTRIBUTES].concat()), Reply { graphics: true, done: true });
        // An error is still an answer from a terminal that knows the protocol.
        let error = b"\x1b_Gi=31;ENOTSUPPORTED:no\x1b\\";
        assert_eq!(parse_reply(&[error, ATTRIBUTES].concat()), Reply { graphics: true, done: true });
    }

    #[test]
    fn the_attributes_alone_mean_unsupported() {
        assert_eq!(parse_reply(ATTRIBUTES), Reply { graphics: false, done: true });
        assert_eq!(parse_reply(b"\x1b[?1;2c"), Reply { graphics: false, done: true });
        // A graphics reply after the attributes is not this terminal answering the query.
        assert_eq!(parse_reply(&[ATTRIBUTES, OK].concat()), Reply { graphics: false, done: true });
    }

    #[test]
    fn partial_replies_wait_for_more() {
        assert_eq!(parse_reply(b""), Reply::default());
        assert_eq!(parse_reply(OK), Reply { graphics: true, done: false });
        assert_eq!(parse_reply(b"\x1b_Gi=31;O"), Reply::default());
        assert_eq!(parse_reply(&[OK, b"\x1b[?62;"].concat()), Reply { graphics: true, done: false });
        assert_eq!(parse_reply(b"\x1b[?"), Reply::default());
    }

    #[test]
    fn other_bytes_are_ignored() {
        // Keys typed while waiting, another image's reply, a cursor report, a stray ESC.
        let noisy = [b"jk\x1b".as_slice(), b"\x1b_Gi=7;OK\x1b\\", b"\x1b[12;1R", b"\x1b[?x", OK, b"q", ATTRIBUTES].concat();
        assert_eq!(parse_reply(&noisy), Reply { graphics: true, done: true });
        let other_id = [b"\x1b_Gi=310;OK\x1b\\".as_slice(), ATTRIBUTES].concat();
        assert_eq!(parse_reply(&other_id), Reply { graphics: false, done: true });
    }

    #[test]
    fn garbage_never_panics() {
        for bytes in [&b"\x1b"[..], b"\x1b_", b"\x1b_G", b"\x1b_G\x1b\\", b"\x1b_G;\x1b\\", b"\x1b[", b"\x1b[?c", &[0xff; 64]] {
            let _ = parse_reply(bytes);
        }
        assert_eq!(parse_reply(b"\x1b[?c"), Reply { graphics: false, done: true });
    }

    #[test]
    fn the_cell_size_comes_before_the_attributes() {
        // kitty's answer: cells 14 px wide and 25 tall.
        let kitty = [b"\x1b[6;25;14t".as_slice(), b"\x1b[?62;52;c"].concat();
        assert_eq!(parse_cell_reply(&kitty), CellReply { cell: Some((14, 25)), done: true });
        // A terminal that doesn't know the question answers the attributes alone.
        assert_eq!(parse_cell_reply(ATTRIBUTES), CellReply { cell: None, done: true });
        // An answer after the attributes is not this terminal answering the query.
        assert_eq!(parse_cell_reply(&[ATTRIBUTES, b"\x1b[6;25;14t"].concat()), CellReply { cell: None, done: true });
    }

    #[test]
    fn a_cell_size_cut_off_waits_for_more() {
        assert_eq!(parse_cell_reply(b""), CellReply::default());
        assert_eq!(parse_cell_reply(b"\x1b[6;25;1"), CellReply::default());
        assert_eq!(parse_cell_reply(b"\x1b[6;25;14t"), CellReply { cell: Some((14, 25)), done: false });
    }

    #[test]
    fn a_cell_size_that_makes_no_sense_is_no_answer() {
        for odd in [&b"\x1b[6;0;14t"[..], b"\x1b[6;25;0t", b"\x1b[6;25t", b"\x1b[6;25;14;3t", b"\x1b[6;;14t", b"\x1b[6;65536;14t", b"\x1b[6;25;14x"] {
            assert_eq!(parse_cell_reply(&[odd, ATTRIBUTES].concat()).cell, None, "{odd:?}");
        }
        // Keys typed while waiting, other reports, a stray ESC: ignored, the answer still found.
        let noisy = [b"j\x1b".as_slice(), b"\x1b[4;2100;3444t", b"\x1b[6;0;0t", b"\x1b[6;25;14t", b"q", ATTRIBUTES].concat();
        assert_eq!(parse_cell_reply(&noisy), CellReply { cell: Some((14, 25)), done: true });
    }

    #[test]
    fn the_background_comes_before_the_attributes() {
        let reply = |answer: &[u8]| parse_background_reply(&[answer, ATTRIBUTES].concat());
        // As kitty and xterm write it, ended by ST or by BEL.
        assert_eq!(reply(b"\x1b]11;rgb:0000/0000/0000\x1b\\"), BackgroundReply { light: Some(false), colour: Some([0; 3]), done: true });
        assert_eq!(reply(b"\x1b]11;rgb:ffff/ffff/ffff\x07"), BackgroundReply { light: Some(true), colour: Some([255; 3]), done: true });
        // Any number of digits, up to four; green counts most.
        assert_eq!(reply(b"\x1b]11;rgb:f/f/f\x07").light, Some(true));
        assert_eq!(reply(b"\x1b]11;rgb:ff/00/ff\x07").light, Some(false));
        assert_eq!(reply(b"\x1b]11;rgb:00/ff/00\x07").light, Some(true));
        // A terminal that doesn't know the question answers the attributes alone.
        assert_eq!(parse_background_reply(ATTRIBUTES), BackgroundReply { light: None, colour: None, done: true });
        // An answer after the attributes is not this terminal answering the query.
        let late = [ATTRIBUTES, b"\x1b]11;rgb:ffff/ffff/ffff\x07"].concat();
        assert_eq!(parse_background_reply(&late), BackgroundReply { light: None, colour: None, done: true });
    }

    #[test]
    fn a_background_cut_off_waits_for_more() {
        assert_eq!(parse_background_reply(b""), BackgroundReply::default());
        assert_eq!(parse_background_reply(b"\x1b]11;rgb:ffff/ff"), BackgroundReply::default());
        assert_eq!(parse_background_reply(b"\x1b]11;rgb:ffff/ffff/ffff\x1b"), BackgroundReply::default());
    }

    #[test]
    fn a_background_that_makes_no_sense_is_no_answer() {
        for odd in [
            &b"\x1b]11;rgb:ff/ff\x07"[..],
            b"\x1b]11;rgb:ff/ff/ff/ff\x07",
            b"\x1b]11;rgb:fffff/ffff/ffff\x07",
            b"\x1b]11;rgb:/ff/ff\x07",
            b"\x1b]11;rgb:ff/ff/ffx\x07",
            b"\x1b]11;?\x07",
            b"\x1b]10;rgb:ffff/ffff/ffff\x07",
        ] {
            assert_eq!(parse_background_reply(&[odd, ATTRIBUTES].concat()).light, None, "{odd:?}");
        }
        // Keys typed while waiting, a stray ESC: ignored, the answer still found.
        let noisy = [b"j\x1b".as_slice(), b"\x1b]11;rgb:zz/ff/ff\x07", b"\x1b]11;rgb:ffff/ffff/ffff\x07", b"q", ATTRIBUTES].concat();
        assert_eq!(parse_background_reply(&noisy), BackgroundReply { light: Some(true), colour: Some([255; 3]), done: true });
    }

    proptest::proptest! {
        #[test]
        fn any_bytes_parse(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..300)) {
            let _ = parse_reply(&bytes);
            let _ = parse_cell_reply(&bytes);
            let _ = parse_background_reply(&bytes);
        }

        #[test]
        fn any_cell_size_is_read_back(w in 1u16.., h in 1u16..) {
            let reply = format!("\x1b[6;{h};{w}t\x1b[?62;c");
            proptest::prop_assert_eq!(parse_cell_reply(reply.as_bytes()), CellReply { cell: Some((w, h)), done: true });
        }
    }
}
