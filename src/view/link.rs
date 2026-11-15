//! A link checked, before anything is done with it. What a source says of its links is input
//! like any other, and where a link leads is the document's to say, so each is checked here,
//! whatever backend gave it: a page's links held to the document's pages and to their limits
//! ([`checked`]); a web address to the three kinds that are opened, with nothing hidden in it, and a
//! `mailto` that names no file for the mail app to read ([`web`]); a path to a regular file or a
//! directory of its own, found beside the document ([`local`]). Nothing here opens or launches
//! anything.

use std::path::{Path, PathBuf};

use crate::source::{Link, MAX_ADDRESS, MAX_LINK_RECTS, MAX_LINKS, MAX_NAME, Target};

/// The longest a source's words for a link that isn't followed are kept ([`Target::Other`]), in
/// bytes: the status line shows less.
const MAX_SAID: usize = 256;

/// What a source gives as page `page`'s links, checked: at most [`MAX_LINKS`], each on that page,
/// with at most [`MAX_LINK_RECTS`] rectangles, finite and clipped to the page, and leading to a
/// page of the document's `pages`, to a path or an address within [`MAX_ADDRESS`], or to a place
/// whose name is within [`MAX_NAME`]. A link on another page or with no rectangle left is left
/// out; one that leads past a limit stays, to say so when it is followed. With them, how many
/// were past [`MAX_LINKS`].
pub(super) fn checked(links: Vec<Link>, page: usize, pages: usize, size_of: &dyn Fn(usize) -> Option<(f32, f32)>) -> (Vec<Link>, usize) {
    let over = links.len().saturating_sub(MAX_LINKS);
    let Some((width, height)) = size_of(page).filter(|(width, height)| page < pages && width.is_finite() && height.is_finite()) else {
        return (Vec::new(), over);
    };
    let clipped = |[x0, y0, x1, y1]: [f32; 4]| {
        let finite = [x0, y0, x1, y1].iter().all(|v| v.is_finite());
        let rect = [x0.max(0.0), y0.max(0.0), x1.min(width), y1.min(height)];
        (finite && rect[0] < rect[2] && rect[1] < rect[3]).then_some(rect)
    };
    let other = |says: &str| Target::Other(says.into());
    let target = |target: Target| match target {
        Target::Place { page, y } => match size_of(page).filter(|(_, height)| page < pages && height.is_finite() && *height >= 0.0) {
            // `clamp` takes the bounds checked above.
            Some((_, height)) => Target::Place { page, y: if y.is_finite() { y.clamp(0.0, height) } else { 0.0 } },
            None => other("a link to a page this document doesn't have"),
        },
        Target::File { path, .. } if path.is_empty() || path.len() > MAX_ADDRESS => other("a link to a file whose path is too long to follow"),
        Target::File { fragment: Some(name), .. } if name.len() > MAX_NAME => other("a link to a place whose name is too long to follow"),
        Target::Web(address) if address.len() > MAX_ADDRESS => other(TOO_LONG),
        Target::Other(says) => Target::Other(cut(says, MAX_SAID)),
        target => target,
    };
    let link = |link: Link| {
        let rects: Vec<[f32; 4]> = link.rects.into_iter().take(MAX_LINK_RECTS).filter_map(clipped).collect();
        let address = link.address.filter(|address| address.len() <= MAX_ADDRESS);
        (link.page == page && !rects.is_empty()).then(|| Link { page, rects, address, target: target(link.target) })
    };
    (links.into_iter().take(MAX_LINKS).filter_map(link).collect(), over)
}

/// `text`, no longer than `most` bytes: cut at the last character that fits.
fn cut(mut text: String, most: usize) -> String {
    if text.len() > most {
        let mut end = most;
        while !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        text.truncate(end);
    }
    text
}

/// Why a web address longer than [`MAX_ADDRESS`] isn't followed.
const TOO_LONG: &str = "a web address longer than 4 KiB, which isn't followed";

/// The parameters of a `mailto` that name a file for the mail app to read, as prefixes: some mail
/// apps attach the file `attach` or `attachment` names, or put the file `insert` names in the
/// message.
const READS_A_FILE: &[&str] = &["attach", "insert"];

/// How many layers of percent-encoding a `mailto`'s parameters are looked for through: one
/// encoded deeper than that is refused unread.
const LAYERS: usize = 8;

/// Whether `address` is a web address to hand to the desktop's opener: `http`, `https` or `mailto`
/// (in any case), at most [`MAX_ADDRESS`] bytes, with no space, no control character, no
/// bidirectional mark and no other character that doesn't show ([`hidden`]), and for a `mailto`,
/// no parameter that names a file to read ([`READS_A_FILE`]), through [`LAYERS`] layers of
/// percent-encoding. Otherwise what kind of link it is, in words for the reader. It is never
/// changed: what is launched is `address` as it is, a single argument.
pub(super) fn web(address: &str) -> Result<(), String> {
    if address.len() > MAX_ADDRESS {
        return Err(TOO_LONG.into());
    }
    if address.chars().any(hidden) {
        return Err("a web address with a space or a hidden character in it, which isn't followed".into());
    }
    let Some((scheme, rest)) = address.split_once(':').filter(|(scheme, _)| is_scheme(scheme)) else {
        return Err("a link that is no web address, nor a file's path: it isn't followed".into());
    };
    match scheme.to_ascii_lowercase().as_str() {
        // `//` and a host: `http:x` is a path to some browsers, a host to others. A host starts
        // with a letter or a digit of any script (`例え.jp`), a percent-encoded one, or the bracket of
        // an IPv6 address.
        "http" | "https" => {
            match rest.strip_prefix("//").is_some_and(|host| host.starts_with(|c: char| c.is_alphanumeric() || matches!(c, '%' | '['))) {
                true => Ok(()),
                false => Err(format!("a {}: link without a host, which isn't followed", scheme.to_ascii_lowercase())),
            }
        }
        "mailto" => match reads_a_file(rest) {
            true => Err("a mailto: link that names a file for the mail app to read, which isn't followed".into()),
            false => Ok(()),
        },
        scheme => Err(format!("a {scheme}: link, which isn't followed")),
    }
}

/// Whether `scheme` is a URI's scheme (RFC 3986 §3.1): a letter, then letters, digits, `+`, `-`
/// and `.`; and no longer than any registered one, so that what is said of it stays short.
fn is_scheme(scheme: &str) -> bool {
    scheme.len() <= 32
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Whether `c` is a character a web address is refused for: a control character, a space of any
/// kind, a bidirectional mark, or another character that doesn't show (a soft hyphen, a
/// zero-width space or joiner, a filler, a variation selector, a tag), any of which may make an
/// address read as another.
fn hidden(c: char) -> bool {
    c.is_control()
        || c.is_whitespace()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{115F}'
                | '\u{1160}'
                | '\u{17B4}'
                | '\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0000}'..='\u{E0FFF}'
        )
}

/// Whether what follows `mailto:` has a parameter that names a file to read, as it is written or
/// under up to [`LAYERS`] layers of percent-encoding; one still encoded past them counts as
/// having one.
fn reads_a_file(rest: &str) -> bool {
    let mut layer = rest.as_bytes().to_vec();
    for _ in 0..=LAYERS {
        if names_a_file(&layer) {
            return true;
        }
        let decoded = percent_decoded(&layer);
        if decoded == layer {
            return false;
        }
        layer = decoded;
    }
    true
}

/// Whether a parameter of `rest`, what follows `mailto:`, is one of [`READS_A_FILE`]: the
/// parameters are what follows the first `?`, apart at `&`, `;` and any further `?`, each named by
/// what comes before its `=`, in any case, the blanks before it left out.
fn names_a_file(rest: &[u8]) -> bool {
    let Some(at) = rest.iter().position(|b| *b == b'?') else { return false };
    let parameters = rest.get(at.saturating_add(1)..).unwrap_or_default();
    parameters.split(|b| matches!(b, b'&' | b';' | b'?')).any(|parameter| {
        let name = parameter.split(|b| *b == b'=').next().unwrap_or_default().trim_ascii().to_ascii_lowercase();
        READS_A_FILE.iter().any(|reads| name.starts_with(reads.as_bytes()))
    })
}

/// `bytes` with each `%` and two hexadecimal digits made the byte they say; any other `%` stays.
fn percent_decoded(bytes: &[u8]) -> Vec<u8> {
    let digit = |b: Option<&u8>| b.and_then(|b| char::from(*b).to_digit(16)).and_then(|d| u8::try_from(d).ok());
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0usize;
    while let Some(byte) = bytes.get(at) {
        match (*byte, digit(bytes.get(at.saturating_add(1))), digit(bytes.get(at.saturating_add(2)))) {
            (b'%', Some(high), Some(low)) => {
                out.push(high.saturating_mul(16).saturating_add(low));
                at = at.saturating_add(3);
            }
            _ => {
                out.push(*byte);
                at = at.saturating_add(1);
            }
        }
    }
    out
}

/// A local file a link leads to, by its real path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Local {
    /// A regular file, to open as any file is.
    File(PathBuf),
    /// A directory, to open as a list.
    Directory(PathBuf),
}

/// The file or the directory the path `path` of a link leads to, found beside `document` (the
/// document's own real path) and made real ([`crate::source::real`]: absolute, every symbolic link
/// followed, and nothing under `/dev` or `/proc`). Only a regular file or a directory: a FIFO, a
/// socket or a device is refused by what the file system says of it, before anything opens it, as
/// is a path that leads nowhere; why, in words for the reader. The path is taken as written: no
/// `~`, and no percent-decoding.
pub(super) fn local(document: &Path, path: &str) -> Result<Local, String> {
    if path.is_empty() || path.len() > MAX_ADDRESS || path.contains('\0') {
        return Err("its path is no path to follow".into());
    }
    let beside = document.parent().ok_or("the document it is in has no folder to look beside")?;
    let joined = beside.join(path);
    if let Err(e) = std::fs::canonicalize(&joined) {
        return Err(format!("it can't be found ({e})"));
    }
    let real = crate::source::real(joined.to_str().ok_or("its path isn't UTF-8")?)?;
    let kind = std::fs::metadata(&real).map_err(|e| format!("it can't be read ({e})"))?;
    match (kind.is_file(), kind.is_dir()) {
        (true, _) => Ok(Local::File(real)),
        (_, true) => Ok(Local::Directory(real)),
        _ => Err("it is no file or folder (a pipe, a socket or a device), so it isn't opened".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A link on `page`, over one rectangle, leading to `target`.
    fn link(page: usize, rect: [f32; 4], target: Target) -> Link {
        Link { page, rects: vec![rect], address: None, target }
    }

    /// Every page 100 by 200.
    fn tall(_page: usize) -> Option<(f32, f32)> {
        Some((100.0, 200.0))
    }

    const TOP: Target = Target::Place { page: 0, y: 0.0 };

    /// What a source gives as a page's links is checked: a link on another page, on a page the
    /// document hasn't, or whose rectangles are malformed or off its page is left out; a rectangle
    /// is clipped to its page, and one that isn't finite, is empty or lies outside is dropped.
    #[test]
    fn a_pages_links_are_checked_and_a_malformed_rectangle_dropped() {
        let given = vec![
            link(1, [10.0, 10.0, 20.0, 20.0], TOP),
            link(0, [10.0, 10.0, 20.0, 20.0], TOP),
            link(1, [f32::NAN, 10.0, 20.0, 20.0], TOP),
            link(1, [10.0, 10.0, f32::INFINITY, 20.0], TOP),
            link(1, [-5.0, 190.0, 300.0, 400.0], TOP),
            link(1, [200.0, 10.0, 220.0, 20.0], TOP),
            link(1, [20.0, 10.0, 10.0, 20.0], TOP),
            Link { page: 1, rects: vec![[f32::NAN; 4], [1.0, 2.0, 3.0, 4.0], [0.0; 4]], address: None, target: TOP },
        ];
        let (kept, over) = checked(given, 1, 3, &tall);
        let rects: Vec<&[[f32; 4]]> = kept.iter().map(|link| &link.rects[..]).collect();
        assert_eq!(rects, [&[[10.0, 10.0, 20.0, 20.0]][..], &[[0.0, 190.0, 100.0, 200.0]], &[[1.0, 2.0, 3.0, 4.0]]]);
        assert!(over == 0 && kept.iter().all(|link| link.page == 1));
        // A page the document hasn't, or whose size isn't one, has no links.
        let one = || vec![link(5, [10.0, 10.0, 20.0, 20.0], TOP)];
        assert!(checked(one(), 5, 3, &tall).0.is_empty());
        assert!(checked(one(), 5, 9, &|_| None).0.is_empty());
        assert!(checked(one(), 5, 9, &|_| Some((f32::NAN, 10.0))).0.is_empty());
        assert_eq!(checked(one(), 5, 9, &tall).0.len(), 1);
    }

    /// A page's links past the most a page gives are left out, and counted; a link's rectangles
    /// past the most a link has are left out.
    #[test]
    fn a_pages_links_and_a_links_rectangles_are_held_to_their_caps() {
        let many: Vec<Link> = (0..MAX_LINKS + 7).map(|i| link(0, [0.0, 0.0, 1.0 + i as f32 / 100.0, 1.0], TOP)).collect();
        let (kept, over) = checked(many, 0, 1, &tall);
        assert_eq!((kept.len(), over), (MAX_LINKS, 7));
        assert_eq!(kept.last().map(|link| link.rects[0][2]), Some(1.0 + (MAX_LINKS - 1) as f32 / 100.0), "the first of them, in order");
        let lines = Link { page: 0, rects: vec![[0.0, 0.0, 5.0, 5.0]; MAX_LINK_RECTS + 3], address: None, target: TOP };
        let (kept, _) = checked(vec![lines], 0, 1, &tall);
        assert_eq!(kept[0].rects.len(), MAX_LINK_RECTS);
    }

    /// Where a link leads is held to the document and to the limits: a place on a page it hasn't
    /// is said, never gone to, and one off its page is brought onto it; a path, an address or a
    /// name past its limit is said too; an address too long to copy isn't kept; what a source says
    /// of a link it doesn't follow is cut.
    #[test]
    fn where_a_pages_links_lead_is_checked() {
        let leads = |target: Target| checked(vec![link(0, [0.0, 0.0, 5.0, 5.0], target)], 0, 2, &tall).0.remove(0).target;
        let said = |target: Target| matches!(leads(target), Target::Other(_));
        assert_eq!(leads(Target::Place { page: 1, y: 50.0 }), Target::Place { page: 1, y: 50.0 });
        assert_eq!(leads(Target::Place { page: 1, y: 900.0 }), Target::Place { page: 1, y: 200.0 });
        assert_eq!(leads(Target::Place { page: 1, y: -3.0 }), Target::Place { page: 1, y: 0.0 });
        assert_eq!(leads(Target::Place { page: 1, y: f32::NAN }), Target::Place { page: 1, y: 0.0 });
        assert_eq!(leads(Target::Place { page: 2, y: 0.0 }), Target::Other("a link to a page this document doesn't have".into()));
        assert!(said(Target::Place { page: usize::MAX, y: 0.0 }));
        let file = |path: String, fragment: Option<String>| Target::File { path, fragment, page: None };
        assert_eq!(leads(file("a.md".into(), Some("x".repeat(MAX_NAME)))), file("a.md".into(), Some("x".repeat(MAX_NAME))));
        assert!(said(file("a.md".into(), Some("x".repeat(MAX_NAME + 1)))));
        assert!(said(file("x".repeat(MAX_ADDRESS + 1), None)) && said(file(String::new(), None)));
        assert_eq!(leads(Target::Web("x".repeat(MAX_ADDRESS))), Target::Web("x".repeat(MAX_ADDRESS)));
        assert!(said(Target::Web("x".repeat(MAX_ADDRESS + 1))));
        let Target::Other(says) = leads(Target::Other("é".repeat(MAX_SAID))) else { panic!("not said") };
        assert_eq!(says, "é".repeat(MAX_SAID / 2), "cut at a character");
        let addressed = |address: String| {
            let given = Link { address: Some(address), ..link(0, [0.0, 0.0, 5.0, 5.0], TOP) };
            checked(vec![given], 0, 2, &tall).0.remove(0).address
        };
        assert_eq!(addressed("#usage".into()).as_deref(), Some("#usage"));
        assert_eq!(addressed("x".repeat(MAX_ADDRESS + 1)), None);
    }

    /// A web address is `http`, `https` or `mailto`, in any case; anything else is said by its
    /// kind and never opened.
    #[test]
    fn a_web_address_is_http_https_or_mailto() {
        for address in ["http://example.com", "https://example.com/a?b=c#d", "HTTPS://Example.com/", "https://[::1]:8080/x", "mailto:a@example.com"] {
            assert_eq!(web(address), Ok(()), "{address}");
        }
        // A host in any script, as written or percent-encoded.
        for address in ["https://例え.jp/", "http://bücher.example/a", "https://%E4%BE%8B.jp/", "https://١.example/"] {
            assert_eq!(web(address), Ok(()), "{address}");
        }
        for address in ["https://", "https:///x", "http://?q", "https://-x.example/", "https://.example/", "http://@example.com"] {
            assert!(web(address).is_err_and(|why| why.contains("without a host")), "{address}");
        }
        assert_eq!(web("MailTo:a@example.com?subject=Hello%20there&body=insert%20coin"), Ok(()), "a value isn't a name");
        assert_eq!(web("mailto:insert@example.com"), Ok(()), "nor is the address");
        let said = |address: &str| web(address).unwrap_err();
        assert_eq!(said("javascript:alert(1)"), "a javascript: link, which isn't followed");
        assert_eq!(said("FILE:///etc/passwd"), "a file: link, which isn't followed");
        assert_eq!(said("ftp://example.com/"), "a ftp: link, which isn't followed");
        assert_eq!(said("x-apple.systempreferences:com.apple"), "a x-apple.systempreferences: link, which isn't followed");
        assert_eq!(said("http:example.com"), "a http: link without a host, which isn't followed");
        assert_eq!(said("https:///etc/passwd"), "a https: link without a host, which isn't followed");
        for none in ["", "example.com", "docs/guide.md", "//example.com", ":x", "1http://x", "a_b:c", &format!("{}://x", "s".repeat(33))] {
            assert_eq!(said(none), "a link that is no web address, nor a file's path: it isn't followed", "{none:?}");
        }
    }

    /// An address longer than 4 KiB, or with a control character, a space, a bidirectional mark or
    /// another character that doesn't show anywhere in it, is refused.
    #[test]
    fn a_web_address_is_refused_for_its_length_or_for_hidden_characters() {
        let long = |length: usize| format!("https://example.com/{}", "a".repeat(length - 20));
        assert_eq!((long(MAX_ADDRESS).len(), web(&long(MAX_ADDRESS))), (MAX_ADDRESS, Ok(())));
        assert_eq!(web(&long(MAX_ADDRESS + 1)), Err(TOO_LONG.into()));
        let hides = [
            '\0',
            '\x1b',
            '\n',
            '\t',
            '\x7f',
            '\u{85}',
            '\u{9b}',
            ' ',
            '\u{a0}',
            '\u{2003}',
            '\u{3000}',
            '\u{2028}',
            '\u{ad}',
            '\u{34f}',
            '\u{61c}',
            '\u{115f}',
            '\u{180e}',
            '\u{200b}',
            '\u{200d}',
            '\u{200e}',
            '\u{200f}',
            '\u{202a}',
            '\u{202e}',
            '\u{2060}',
            '\u{2066}',
            '\u{2069}',
            '\u{206f}',
            '\u{3164}',
            '\u{fe0f}',
            '\u{feff}',
            '\u{ffa0}',
            '\u{fffb}',
            '\u{1d173}',
            '\u{e0001}',
            '\u{e0041}',
            '\u{e01ef}',
        ];
        for hide in hides {
            for address in [format!("https://example.com/a{hide}b"), format!("https://exa{hide}mple.com/"), format!("mailto:a@example.com{hide}")] {
                assert!(web(&address).is_err_and(|why| why.contains("hidden character")), "U+{:04X}", u32::from(hide));
            }
        }
        // What shows is let through, whatever its script.
        assert_eq!(web("https://example.com/naïve/日本語/✓?q=é"), Ok(()));
    }

    /// A `mailto` whose parameters name a file for the mail app to read is refused: `attach` and
    /// `insert` as prefixes, in any case, among the parameters, as written or percent-encoded,
    /// once, twice, up to eight times over; one encoded deeper than that is refused unread.
    #[test]
    fn a_mailto_that_names_a_file_to_read_is_refused() {
        let refused = |address: &str| web(address).is_err_and(|why| why.contains("names a file"));
        for address in [
            "mailto:a@example.com?attach=/etc/passwd",
            "mailto:a@example.com?subject=hi&attach=/etc/passwd",
            "mailto:a@example.com?subject=hi&Attachment=~/.ssh/id_rsa",
            "mailto:a@example.com?subject=hi;INSERT=/etc/passwd",
            "mailto:?attachments=x",
            "mailto:a@example.com?subject=hi?attach=x",
            "mailto:a@example.com?%61ttach=/etc/passwd",
            "mailto:a@example.com?subject=hi%26attach=/etc/passwd",
            "mailto:a@example.com%3Fattach=/etc/passwd",
            "mailto:a@example.com?%2561ttach=/etc/passwd",
            "mailto:a@example.com?subject=hi%2526attach%253D/etc/passwd",
            "mailto:a@example.com?%20attach=x",
        ] {
            assert!(refused(address), "{address}");
        }
        // `%61` is `a`, under one more layer each time: `%2561`, `%252561`, …
        let layered = |layers: usize| format!("mailto:a@example.com?%{}61ttach=x", "25".repeat(layers - 1));
        assert_eq!(layered(2), "mailto:a@example.com?%2561ttach=x");
        assert!((1..=LAYERS).all(|layers| refused(&layered(layers))), "found through {LAYERS} layers");
        assert!(refused(&layered(LAYERS + 1)) && refused(&layered(40)), "deeper, refused unread");
        // What names none is let through, encoded or not.
        for address in
            ["mailto:a@example.com?subject=attach", "mailto:a@example.com?subject=%2561", "mailto:a@example.com?body=100%25%20sure&cc=b@example.com"]
        {
            assert_eq!(web(address), Ok(()), "{address}");
        }
        assert_eq!(percent_decoded(b"a%20b%2G%2%%41"), b"a b%2G%2%A");
    }

    /// A link's path is found beside its document and made real; it leads to a regular file or a
    /// directory, through a symbolic link too, and to nothing else: a FIFO, a device, a symbolic
    /// link to either, or a path that leads nowhere is refused, without being opened.
    #[test]
    fn a_links_path_leads_to_a_regular_file_or_a_directory_only() {
        let dir = std::env::temp_dir().join(format!("clview-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs/more")).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        std::fs::write(dir.join("docs/guide.md"), "# Guide").unwrap();
        std::fs::write(dir.join("README.md"), "# Read me").unwrap();
        std::os::unix::fs::symlink(dir.join("README.md"), dir.join("docs/readme-link.md")).unwrap();
        std::os::unix::fs::symlink("/dev/null", dir.join("docs/null.md")).unwrap();
        assert!(std::process::Command::new("mkfifo").arg(dir.join("docs/pipe.md")).status().unwrap().success());
        std::os::unix::fs::symlink(dir.join("docs/pipe.md"), dir.join("docs/pipe-link.md")).unwrap();
        std::os::unix::fs::symlink(dir.join("docs/gone.md"), dir.join("docs/dangling.md")).unwrap();
        let document = dir.join("docs/index.md");
        let leads = |path: &str| local(&document, path);
        assert_eq!(leads("guide.md"), Ok(Local::File(dir.join("docs/guide.md"))));
        assert_eq!(leads("./more/../guide.md"), Ok(Local::File(dir.join("docs/guide.md"))));
        assert_eq!(leads("../README.md"), Ok(Local::File(dir.join("README.md"))));
        assert_eq!(leads("readme-link.md"), Ok(Local::File(dir.join("README.md"))), "made real");
        assert_eq!(leads("more"), Ok(Local::Directory(dir.join("docs/more"))));
        assert_eq!(leads(".."), Ok(Local::Directory(dir.clone())));
        assert_eq!(leads(dir.join("README.md").to_str().unwrap()), Ok(Local::File(dir.join("README.md"))), "an absolute path is itself");
        // Nothing reads the pipe: a reader would wait here for a writer.
        for refused in ["pipe.md", "pipe-link.md"] {
            assert!(leads(refused).is_err_and(|why| why.starts_with("it is no file or folder")), "{refused}");
        }
        for device in ["null.md", "/dev/null", "/dev/stdin", "../../../../../../../../../../dev/null"] {
            assert!(leads(device).is_err(), "{device}");
        }
        for nowhere in ["gone.md", "dangling.md", "guide.md/x", "guide.md#top", "guide%2Emd"] {
            assert!(leads(nowhere).is_err_and(|why| why.starts_with("it can't be found")), "{nowhere}");
        }
        for no_path in ["", "a\0b", &"x".repeat(MAX_ADDRESS + 1)] {
            assert_eq!(leads(no_path), Err("its path is no path to follow".into()));
        }
        assert!(local(Path::new("/"), "etc").is_err(), "a document with no folder");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
