//! Keys from a terminal in raw mode: bytes in, keys out. Pure, and total over any bytes.
//!
//! The terminal's answers arrive among the keys: to graphics commands as `ESC _ G … ESC \` (the
//! kitty graphics protocol), to `clv`'s questions as CSI sequences (the cell size, device
//! attributes, whether it has a clipboard for pictures), late ones included, and to a clipboard
//! write as an OSC sequence, `ESC ] 5522 ; … ESC \` (kitty's clipboard protocol). They are never
//! keys, even cut in two: one that says an image is missing is [`Key::Missing`], the clipboard's
//! are [`Key::Clipboard`] and [`Key::Copied`], and the rest are dropped.

/// A key the pager knows. Anything else is dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Char(char),
    Enter,
    Escape,
    Backspace,
    /// Ctrl-C, which raw mode delivers as a byte instead of a signal.
    Interrupt,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    /// Not typed: the terminal's answer that image `id`, which it was asked to place, isn't there
    /// (it drops images to stay under its storage quota).
    Missing(u32),
    /// Not typed: whether the terminal speaks the clipboard protocol (OSC 5522), as it answers
    /// DECRQM for mode 5522: `CSI ? 5522 ; Ps $ y`, where 0 (unknown) and 4 say it doesn't.
    Clipboard(bool),
    /// Not typed: the terminal's answer to clipboard write `id` (`OSC 5522 ;
    /// type=write:status=…:id=clv-<id> ST`): written, or why not.
    Copied(u32, Result<(), ClipError>),
}

/// Why the terminal didn't write a clipboard, as the clipboard protocol names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClipError {
    /// `EPERM`: the user or the system said no.
    Denied,
    /// `EBUSY`: for now (several clients of a multiplexer).
    Busy,
    /// `EFBIG`: more than the terminal takes.
    TooLarge,
    /// `EINVAL`, `ENOSYS`, `EIO` or a code the protocol doesn't name.
    Failed,
}

/// The id clipboard writes carry, before their number: answers to anyone else's are dropped.
pub(crate) const CLIP_ID: &str = "clv-";

/// The longest escape sequence worth waiting for; longer ones are dropped.
const MAX_SEQUENCE: usize = 16;
/// The longest graphics answer worth waiting for. kitty's are under 200 bytes.
pub(crate) const MAX_ANSWER: usize = 512;
/// How a graphics answer starts.
const ANSWER: &[u8] = b"\x1b_G";
/// How an OSC sequence starts; a terminal's answers go on with their number.
const OSC: &[u8] = b"\x1b]";

/// Decode `bytes` into keys, and say how many bytes were used. An escape sequence cut off at the
/// end is left for the next read, whole: past its ESC it is no key, however long the rest takes
/// (the caller drops what never completes). Only a lone ESC at the end can be a key, the Escape
/// key, and is when `flush` (the user pressed Esc and nothing followed).
pub(crate) fn decode(bytes: &[u8], flush: bool) -> (Vec<Key>, usize) {
    let mut keys = Vec::new();
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        let rest = bytes.get(i..).unwrap_or_default();
        let (key, used) = match b {
            0x1b if rest.starts_with(ANSWER) => match answer(rest) {
                Some(found) => found,
                None => break,
            },
            // The start of what may be an answer waits too.
            0x1b if rest == b"\x1b_" => break,
            0x1b if rest.starts_with(OSC) && rest.get(OSC.len()).is_some_and(u8::is_ascii_digit) => match osc(rest) {
                Some(found) => found,
                None => break,
            },
            0x1b if rest == OSC => break,
            0x1b => match escape(rest) {
                Some(found) => found,
                None if flush && rest.len() == 1 => (Some(Key::Escape), 1),
                None => break,
            },
            b'\r' | b'\n' => (Some(Key::Enter), 1),
            0x7f | 0x08 => (Some(Key::Backspace), 1),
            0x03 => (Some(Key::Interrupt), 1),
            // Printable ASCII.
            0x20..=0x7e => (Some(Key::Char(char::from(b))), 1),
            0x0c => (Some(Key::Char('\u{c}')), 1),
            // A character past ASCII, typed into a list's filter (`é`); one cut off waits for the
            // rest, as an escape sequence does.
            0x80..=0xff => match utf8(rest) {
                Some(found) => found,
                None => break,
            },
            _ => (None, 1),
        };
        keys.extend(key);
        i = i.saturating_add(used);
    }
    (keys, i)
}

/// The character UTF-8 encodes at the start of `bytes` (which starts past ASCII) and its length:
/// [`Key::Char`], or nothing for a control character (C1). Bytes that aren't UTF-8 are dropped, as
/// many as can't start a character. `None` while a character is cut off.
fn utf8(bytes: &[u8]) -> Option<(Option<Key>, usize)> {
    // How long the character its first byte starts is; a byte that starts none is one alone.
    let len = match bytes.first() {
        Some(0xc2..=0xdf) => 2,
        Some(0xe0..=0xef) => 3,
        Some(0xf0..=0xf4) => 4,
        _ => 1,
    };
    let start = bytes.get(..len).unwrap_or(bytes);
    match std::str::from_utf8(start) {
        Ok(text) => Some((text.chars().next().filter(|c| !c.is_control()).map(Key::Char), start.len())),
        // Right so far, and the rest still to come.
        Err(e) if e.error_len().is_none() => None,
        Err(e) => Some((None, e.error_len().unwrap_or(1).max(1))),
    }
}

/// The key an escape sequence at the start of `bytes` (which starts with ESC) stands for, and
/// its length. `None` when it is cut off.
fn escape(bytes: &[u8]) -> Option<(Option<Key>, usize)> {
    match bytes.get(1) {
        None => None,
        // CSI: parameters and intermediates, then a final byte in 0x40..=0x7e.
        Some(b'[') => {
            let body = bytes.get(2..).unwrap_or_default();
            let Some(end) = body.iter().position(|b| (0x40..=0x7e).contains(b)) else {
                return if body.len() >= MAX_SEQUENCE { Some((None, bytes.len())) } else { None };
            };
            let params = body.get(..end).unwrap_or_default();
            let key = match (params, body.get(end)) {
                (b"" | b"1", Some(b'A')) => Some(Key::Up),
                (b"" | b"1", Some(b'B')) => Some(Key::Down),
                (b"" | b"1", Some(b'C')) => Some(Key::Right),
                (b"" | b"1", Some(b'D')) => Some(Key::Left),
                (b"" | b"1", Some(b'H')) | (b"1" | b"7", Some(b'~')) => Some(Key::Home),
                (b"" | b"1", Some(b'F')) | (b"4" | b"8", Some(b'~')) => Some(Key::End),
                (b"5", Some(b'~')) => Some(Key::PageUp),
                (b"6", Some(b'~')) => Some(Key::PageDown),
                (params, Some(b'y')) => clipboard_mode(params),
                _ => None,
            };
            Some((key, end.saturating_add(3)))
        }
        // SS3, as some terminals send arrows, Home and End.
        Some(b'O') => {
            let key = match bytes.get(2)? {
                b'A' => Some(Key::Up),
                b'B' => Some(Key::Down),
                b'C' => Some(Key::Right),
                b'D' => Some(Key::Left),
                b'H' => Some(Key::Home),
                b'F' => Some(Key::End),
                _ => None,
            };
            Some((key, 3))
        }
        // A string terminator on its own: the end of something cut off, never a key.
        Some(b'\\') => Some((None, 2)),
        // Esc, then another key typed quickly enough to arrive in the same read.
        Some(_) => Some((Some(Key::Escape), 1)),
    }
}

/// The graphics answer at the start of `bytes` (which starts with [`ANSWER`]) and its length,
/// through its `ESC \`: [`Key::Missing`] when it says an image isn't there, nothing otherwise.
/// `None` while it is cut off; one longer than [`MAX_ANSWER`] is dropped whole.
fn answer(bytes: &[u8]) -> Option<(Option<Key>, usize)> {
    let body = bytes.get(ANSWER.len()..).unwrap_or_default();
    let Some(end) = body.windows(2).position(|w| w == b"\x1b\\") else {
        return (bytes.len() > MAX_ANSWER).then_some((None, bytes.len()));
    };
    let used = ANSWER.len().saturating_add(end).saturating_add(2);
    // `i=<id>,…;ENOENT:<message>`, as kitty answers a placement of an image it doesn't have.
    let text = std::str::from_utf8(body.get(..end).unwrap_or_default()).unwrap_or_default();
    let (control, message) = text.split_once(';').unwrap_or((text, ""));
    let id = control.split(',').find_map(|kv| kv.strip_prefix("i=")?.parse::<u32>().ok());
    Some((id.filter(|_| message.starts_with("ENOENT")).map(Key::Missing), used))
}

/// [`Key::Clipboard`] from the parameters of a DECRQM answer (`CSI ? 5522 ; Ps $ y`): the mode is
/// known when Ps is 1, 2 or 3 (set, reset, always set). `None` for another mode's.
fn clipboard_mode(params: &[u8]) -> Option<Key> {
    let ps = params.strip_prefix(b"?5522;")?.strip_suffix(b"$")?;
    Some(Key::Clipboard(matches!(ps, b"1" | b"2" | b"3")))
}

/// The OSC answer at the start of `bytes` (which starts with [`OSC`] and a digit) and its length,
/// through its terminator, `ESC \` or BEL: [`Key::Copied`] when it answers a clipboard write of
/// ours, nothing otherwise. An ESC that starts something else ends it, and is left for what it
/// starts. `None` while it is cut off; one longer than [`MAX_ANSWER`] is dropped whole.
fn osc(bytes: &[u8]) -> Option<(Option<Key>, usize)> {
    let body = bytes.get(OSC.len()..).unwrap_or_default();
    let stop = body.iter().position(|b| *b == 0x1b || *b == 0x07);
    let (end, used) = match stop.map(|at| (at, body.get(at), body.get(at.saturating_add(1)))) {
        Some((at, Some(0x07), _)) => (at, at.saturating_add(1)),
        Some((at, _, Some(b'\\'))) => (at, at.saturating_add(2)),
        Some((at, _, Some(_))) => (at, at),
        // Nothing yet to end it, or an ESC last, which may be the terminator's start.
        _ => return (bytes.len() > MAX_ANSWER).then_some((None, bytes.len())),
    };
    let text = std::str::from_utf8(body.get(..end).unwrap_or_default()).unwrap_or_default();
    Some((text.strip_prefix("5522;").and_then(copied), OSC.len().saturating_add(used)))
}

/// [`Key::Copied`] from a clipboard protocol answer's metadata and payload: `type=write`, its
/// `status`, and an `id` of ours, keys and values separated by colons.
fn copied(answer: &str) -> Option<Key> {
    let metadata = answer.split(';').next().unwrap_or_default();
    let value = |key: &str| metadata.split(':').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='));
    if value("type")? != "write" {
        return None;
    }
    let id = value("id")?.strip_prefix(CLIP_ID)?.parse::<u32>().ok()?;
    let status = match value("status")? {
        "DONE" => Ok(()),
        "EPERM" => Err(ClipError::Denied),
        "EBUSY" => Err(ClipError::Busy),
        "EFBIG" => Err(ClipError::TooLarge),
        _ => Err(ClipError::Failed),
    };
    Some(Key::Copied(id, status))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(bytes: &[u8]) -> Vec<Key> {
        let (keys, used) = decode(bytes, true);
        assert_eq!(used, bytes.len());
        keys
    }

    #[test]
    fn plain_keys() {
        assert_eq!(
            keys(b" bq7G\r\n\x7f\x03"),
            [Key::Char(' '), Key::Char('b'), Key::Char('q'), Key::Char('7'), Key::Char('G'), Key::Enter, Key::Enter, Key::Backspace, Key::Interrupt]
        );
    }

    #[test]
    fn arrows_pages_home_and_end() {
        assert_eq!(keys(b"\x1b[A\x1b[B\x1b[C\x1b[D\x1bOA\x1bOD"), [Key::Up, Key::Down, Key::Right, Key::Left, Key::Up, Key::Left]);
        assert_eq!(keys(b"\x1b[5~\x1b[6~"), [Key::PageUp, Key::PageDown]);
        assert_eq!(keys(b"\x1b[H\x1b[1~\x1b[7~\x1bOH"), [Key::Home; 4]);
        assert_eq!(keys(b"\x1b[F\x1b[4~\x1b[8~\x1bOF"), [Key::End; 4]);
    }

    #[test]
    fn unknown_sequences_and_bytes_are_dropped() {
        // F5, Shift+Up, a mouse report, a C1 control in UTF-8 (CSI), a NUL.
        assert_eq!(keys(b"\x1b[15~\x1b[1;2A\x1b[<0;3;4M\xc2\x9b\0j"), [Key::Char('j')]);
    }

    #[test]
    fn a_character_past_ascii_is_one_key() {
        // Two, three and four bytes, among ASCII keys.
        assert_eq!(keys("jé€😀k".as_bytes()), [Key::Char('j'), Key::Char('é'), Key::Char('€'), Key::Char('😀'), Key::Char('k')]);
        assert_eq!(keys("CAFÉ".as_bytes()), "CAFÉ".chars().map(Key::Char).collect::<Vec<_>>());
        // Control characters are no keys, C1 ones included (NEL, CSI, OSC), as UTF-8 encodes them.
        assert_eq!(keys(b"\xc2\x85\xc2\x9b\xc2\x9dq"), [Key::Char('q')]);
    }

    #[test]
    fn a_character_cut_across_reads_waits_for_the_rest() {
        for c in ['é', '€', '😀'] {
            let bytes = c.to_string().into_bytes();
            for cut in 1..bytes.len() {
                // Even when nothing more came yet: it isn't the start of anything else.
                for flush in [false, true] {
                    let first = [b"j".as_slice(), &bytes[..cut]].concat();
                    assert_eq!(decode(&first, flush), (vec![Key::Char('j')], 1), "{c} cut at {cut}");
                }
                let (first, used) = decode(&bytes[..cut], true);
                let pending = [&bytes[used..cut], &bytes[cut..]].concat();
                assert_eq!((first, keys(&pending)), (vec![], vec![Key::Char(c)]), "{c} cut at {cut}");
            }
        }
        // The rest, then an escape sequence, in the next read.
        let (first, used) = decode(b"\xe2\x82", true);
        let pending = [&b"\xe2\x82"[used..], b"\xac\x1b[A"].concat();
        assert_eq!((first, keys(&pending)), (vec![], vec![Key::Char('€'), Key::Up]));
    }

    #[test]
    fn bytes_that_arent_utf8_are_dropped_and_what_follows_them_is_kept() {
        for (bytes, expected) in [
            // A byte that continues a character, alone; bytes that start none.
            (b"\x80j".as_slice(), vec![Key::Char('j')]),
            (b"\xbf\xc0\xc1\xf5\xffj", vec![Key::Char('j')]),
            // A character cut off by another key: the key stays.
            (b"\xc3j", vec![Key::Char('j')]),
            (b"\xe2\x82j", vec![Key::Char('j')]),
            (b"\xf0\x9f\x98q", vec![Key::Char('q')]),
            (b"\xc3\x1b[B", vec![Key::Down]),
            (b"\xe2\x82\r", vec![Key::Enter]),
            // Too long for what it encodes, a surrogate, past U+10FFFF.
            (b"\xc1\xbfj", vec![Key::Char('j')]),
            (b"\xe0\x80\xafj", vec![Key::Char('j')]),
            (b"\xed\xa0\x80j", vec![Key::Char('j')]),
            (b"\xf4\x90\x80\x80j", vec![Key::Char('j')]),
            // A character whole after them.
            (b"\x80\xc3\xa9", vec![Key::Char('é')]),
        ] {
            assert_eq!(keys(bytes), expected, "{bytes:?}");
        }
    }

    #[test]
    fn a_sequence_cut_off_waits_for_the_next_read() {
        assert_eq!(decode(b"j\x1b", false), (vec![Key::Char('j')], 1));
        assert_eq!(decode(b"j\x1b[", false), (vec![Key::Char('j')], 1));
        assert_eq!(decode(b"\x1b[6", false), (vec![], 0));
        assert_eq!(decode(b"\x1bO", false), (vec![], 0));
        // Nothing followed a lone ESC: it was the Esc key.
        assert_eq!(decode(b"\x1b", true), (vec![Key::Escape], 1));
        assert_eq!(decode(b"j\x1b", true), (vec![Key::Char('j'), Key::Escape], 2));
        // Past the ESC, a sequence waits for the rest even when nothing more came yet: it is no
        // Esc (which quits), nor its bytes keys.
        assert_eq!(decode(b"j\x1b[", true), (vec![Key::Char('j')], 1));
        assert_eq!(decode(b"\x1b[6;", true), (vec![], 0));
        assert_eq!(decode(b"\x1bO", true), (vec![], 0));
        // Esc typed before another key.
        assert_eq!(keys(b"\x1bq"), [Key::Escape, Key::Char('q')]);
    }

    #[test]
    fn a_reply_cut_anywhere_past_its_esc_is_never_keys() {
        // A late cell size and device attributes, as the terminal answers `clv`'s question, and
        // both together; each cut in two, the first part read alone with nothing more to come.
        // (Not cut just past an ESC: an ESC alone with nothing after it is the Esc key, which is
        // what the pager waits a while to tell.)
        let cell = b"\x1b[6;25;14t".as_slice();
        let attributes = b"\x1b[?62;22c".as_slice();
        for reply in [cell, attributes, &[cell, attributes].concat()] {
            for cut in (2..reply.len()).filter(|&cut| reply[cut - 1] != 0x1b) {
                let (first, used) = decode(&reply[..cut], true);
                let pending = [&reply[used..cut], &reply[cut..]].concat();
                let (rest, all) = decode(&pending, true);
                assert_eq!((first, rest, all), (vec![], vec![], pending.len()), "{:?} cut at {cut}", String::from_utf8_lossy(reply));
            }
        }
        // A key cut in two, as over SSH, is the key.
        let page_down = b"\x1b[6~".as_slice();
        for cut in 2..page_down.len() {
            let (first, used) = decode(&page_down[..cut], true);
            let pending = [&page_down[used..cut], &page_down[cut..]].concat();
            assert_eq!((first, decode(&pending, true)), (vec![], (vec![Key::PageDown], pending.len())), "cut at {cut}");
        }
    }

    #[test]
    fn graphics_answers_are_never_keys() {
        // kitty's answer to placing an image it dropped, between two keys.
        let missing = b"j\x1b_Gi=3000,p=1;ENOENT:Put command refers to non-existent image with id: 3000 and number: 0\x1b\\k";
        assert_eq!(keys(missing), [Key::Char('j'), Key::Missing(3000), Key::Char('k')]);
        // Any other answer is dropped, an OK or another error alike: its ESC isn't Esc, nor its
        // letters keys (`q` would quit).
        assert_eq!(keys(b"\x1b_Gi=7;OK\x1b\\\x1b_Gi=7;EINVAL:quit\x1b\\\x1b_G;ENOENT:no id\x1b\\"), []);
        assert_eq!(keys(b"\x1b_Gi=x;ENOENT:\x1b\\\x1b\\"), []);
        // Cut off anywhere, it waits for the rest even when nothing more came yet.
        let answer = &missing[1..missing.len() - 1];
        for end in ANSWER.len()..answer.len() {
            assert_eq!(decode(&answer[..end], true), (vec![], 0), "cut at {end}");
        }
        // Too long to be an answer: dropped whole.
        let long = [b"\x1b_Gi=1;".as_slice(), &[b'q'; MAX_ANSWER]].concat();
        assert_eq!(decode(&long, false), (vec![], long.len()));
        // Its first bytes wait for the rest, even when nothing more came yet.
        assert_eq!(decode(b"j\x1b_", false), (vec![Key::Char('j')], 1));
        assert_eq!(decode(b"j\x1b_", true), (vec![Key::Char('j')], 1));
        // Esc and _ typed together, then another key: Esc, and the keys.
        assert_eq!(keys(b"\x1b_j"), [Key::Escape, Key::Char('_'), Key::Char('j')]);
    }

    #[test]
    fn whether_the_terminal_speaks_the_clipboard_protocol_is_its_decrqm_answer() {
        // kitty 0.49.2 answers 2 (known, reset), Ghostty 1.3.1 answers 0 (unknown).
        assert_eq!(keys(b"\x1b[?5522;2$y\x1b[?62;52;c"), [Key::Clipboard(true)]);
        assert_eq!(keys(b"\x1b[?5522;1$y\x1b[?5522;3$y"), [Key::Clipboard(true); 2]);
        assert_eq!(keys(b"\x1b[?5522;0$y\x1b[?5522;4$y"), [Key::Clipboard(false); 2]);
        // Another mode's answer, or a malformed one, is no key.
        assert_eq!(keys(b"\x1b[?2026;2$y\x1b[?5522;2y\x1b[?5522$y\x1b[5522;2$y"), []);
    }

    #[test]
    fn clipboard_answers_are_never_keys() {
        // kitty's answer to a write, between two keys, as kitty 0.49.2 sent it, directly and
        // through tmux 3.5a.
        let done = b"j\x1b]5522;type=write:status=DONE:id=clv-3\x1b\\k";
        assert_eq!(keys(done), [Key::Char('j'), Key::Copied(3, Ok(())), Key::Char('k')]);
        // Ended by BEL too; each error the protocol names.
        for (status, error) in [
            ("EPERM", ClipError::Denied),
            ("EBUSY", ClipError::Busy),
            ("EFBIG", ClipError::TooLarge),
            ("EIO", ClipError::Failed),
            ("EWHAT", ClipError::Failed),
        ] {
            let answer = format!("\x1b]5522;type=write:status={status}:id=clv-12\x07");
            assert_eq!(keys(answer.as_bytes()), [Key::Copied(12, Err(error))], "{status}");
        }
        // The keys in any order, and a payload, change nothing.
        assert_eq!(keys(b"\x1b]5522;id=clv-1:status=DONE:type=write;\x1b\\"), [Key::Copied(1, Ok(()))]);
        // Not an answer to a write of ours: dropped, its letters no keys (`q` would quit).
        for other in [
            b"\x1b]5522;type=write:status=DONE:id=vim-3\x1b\\".as_slice(),
            b"\x1b]5522;type=write:status=DONE\x1b\\",
            b"\x1b]5522;type=read:status=DONE:id=clv-3\x1b\\",
            b"\x1b]5522;type=write:id=clv-3\x1b\\",
            b"\x1b]5522;type=write:status=DONE:id=clv-x\x1b\\",
            b"\x1b]11;rgb:0000/0000/0000\x1b\\",
            b"\x1b]52;c;cXVpdA==\x07",
            b"\x1b]5522;\xff\xfeq\x1b\\",
        ] {
            assert_eq!(keys(other), [], "{:?}", String::from_utf8_lossy(other));
        }
        // Cut off anywhere past its ESC, it waits for the rest even when nothing more came yet,
        // and then is the answer: never Esc, which quits, nor its letters keys. (Not cut just
        // past its first ESC: an ESC alone with nothing after it is the Esc key, which is what
        // the pager waits a while to tell. Its terminator's ESC alone waits.)
        for cut in 3..done.len() - 1 {
            let (first, used) = decode(&done[..cut], true);
            let pending = [&done[used..cut], &done[cut..]].concat();
            let (rest, all) = decode(&pending, true);
            assert_eq!([first, rest].concat(), [Key::Char('j'), Key::Copied(3, Ok(())), Key::Char('k')], "cut at {cut}");
            assert_eq!(all, pending.len(), "cut at {cut}");
        }
        assert_eq!(decode(b"j\x1b]", true), (vec![Key::Char('j')], 1));
        // Too long to be an answer: dropped whole.
        let long = [b"\x1b]5522;".as_slice(), &[b'q'; MAX_ANSWER]].concat();
        assert_eq!(decode(&long, false), (vec![], long.len()));
        // An ESC that starts something else cuts it short: it is dropped, and the rest decoded.
        assert_eq!(keys(b"\x1b]5522;type=wri\x1b[Aq"), [Key::Up, Key::Char('q')]);
        // Esc and ] typed together, then another key: Esc, and the keys.
        assert_eq!(keys(b"\x1b]j"), [Key::Escape, Key::Char(']'), Key::Char('j')]);
    }

    #[test]
    fn an_endless_sequence_is_dropped() {
        let long = [b"\x1b[".as_slice(), &[b'1'; 40]].concat();
        assert_eq!(decode(&long, false), (vec![], long.len()));
    }

    proptest::proptest! {
        #[test]
        fn any_bytes_decode(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..200), flush in proptest::prelude::any::<bool>()) {
            let (_, used) = decode(&bytes, flush);
            proptest::prop_assert!(used <= bytes.len());
            if flush {
                // All of it, but for an escape sequence still arriving past its ESC: never a lone
                // ESC, and short unless it is a graphics or an OSC answer, which is no longer than
                // an answer may be. Or a character cut off, right so far.
                let rest = &bytes[used..];
                let answer = rest.starts_with(ANSWER) || rest.starts_with(OSC);
                let cut_off = rest.len() < 4 && std::str::from_utf8(rest).is_err_and(|e| e.valid_up_to() == 0 && e.error_len().is_none());
                proptest::prop_assert!(
                    rest.is_empty() || cut_off || (rest.len() > 1 && rest[0] == 0x1b && rest.len() <= MAX_ANSWER && (answer || rest.len() < MAX_SEQUENCE + 2))
                );
            }
        }

        /// Text typed, however the reads cut it, is its characters, one key each, as the pager
        /// decodes what it reads: what is left of one read goes before the next.
        #[test]
        fn text_cut_anywhere_is_its_characters(text in "[^\\p{C}]{0,40}", cuts in proptest::collection::vec(0usize..200, 0..6)) {
            let bytes = text.as_bytes();
            let mut cuts: Vec<usize> = cuts.into_iter().map(|c| c % (bytes.len() + 1)).chain([bytes.len()]).collect();
            cuts.sort_unstable();
            let (mut pending, mut got, mut from) = (Vec::new(), Vec::new(), 0);
            for cut in cuts {
                pending.extend_from_slice(&bytes[from..cut]);
                from = cut;
                let (keys, used) = decode(&pending, true);
                got.extend(keys);
                pending.drain(..used);
            }
            proptest::prop_assert!(pending.is_empty());
            proptest::prop_assert_eq!(got, text.chars().map(Key::Char).collect::<Vec<_>>());
        }

        /// Keys and the clipboard's answers, in any order and cut into reads anywhere: the keys
        /// come out, and the answers as answers, never as keys.
        #[test]
        fn answers_among_keys_cut_anywhere_are_never_keys(
            parts in proptest::collection::vec(0usize..4, 0..12),
            cuts in proptest::collection::vec(1usize..40, 0..12),
        ) {
            let mut bytes = Vec::new();
            let mut expected = Vec::new();
            for part in parts {
                let (code, key): (&[u8], Option<Key>) = match part {
                    0 => (b"j", Some(Key::Char('j'))),
                    1 => (b"\x1b]5522;type=write:status=DONE:id=clv-7\x1b\\", Some(Key::Copied(7, Ok(())))),
                    2 => (b"\x1b[?5522;2$y", Some(Key::Clipboard(true))),
                    _ => (b"\x1b]5522;type=write:status=EPERM:id=clv-8\x07", Some(Key::Copied(8, Err(ClipError::Denied)))),
                };
                bytes.extend_from_slice(code);
                expected.extend(key);
            }
            // Read in pieces, each decoded with what was left of the one before, as the pager does,
            // then once more when nothing more comes.
            let (mut got, mut pending, mut at) = (Vec::new(), Vec::new(), 0usize);
            for (cut, flush) in cuts.into_iter().map(|cut| (cut, false)).chain([(usize::MAX, false), (0, true)]) {
                let end = at.saturating_add(cut).min(bytes.len());
                pending.extend_from_slice(&bytes[at..end]);
                at = end;
                let (keys, used) = decode(&pending, flush);
                got.extend(keys);
                pending.drain(..used);
            }
            proptest::prop_assert_eq!(got, expected);
            proptest::prop_assert!(pending.is_empty());
        }
    }
}
