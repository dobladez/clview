//! `clv` draws pages through the kitty graphics protocol. Test output is a pipe, not a terminal,
//! so these tests pass `--protocol kitty` and read the escape codes back from stdout.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use base64::Engine as _;

const BIN: &str = env!("CARGO_BIN_EXE_clv");

pub(crate) fn tmp(test: &str, name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("clview-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

/// A US Letter document (612 × 792 pt) with `pages` pages.
fn fixture(test: &str, pages: usize) -> PathBuf {
    let pdf = tmp(test, if pages == 1 { "one.pdf" } else { "many.pdf" });
    std::fs::write(&pdf, crate::fixtures::letter("View test", pages)).unwrap();
    pdf
}

/// Run `clv`, kept away from the terminal, the tmux and the SSH login the tests may run in.
pub(crate) fn view(pdf: &Path, args: &[&str]) -> Output {
    view_with(pdf, args, &[])
}

/// [`view`], with `env` set.
pub(crate) fn view_with(pdf: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(BIN);
    command.arg(pdf).args(args).env("TERM", "dumb");
    for name in ["TMUX", "TMUX_PANE", "KITTY_WINDOW_ID", "TERM_PROGRAM", "SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY", "COLORTERM", "NO_COLOR"] {
        command.env_remove(name);
    }
    command.envs(env.iter().copied()).output().unwrap()
}

pub(crate) fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// One image drawn by `clv`: its first control keys and its decoded payload.
pub(crate) struct Drawn {
    pub(crate) keys: String,
    pub(crate) png: Vec<u8>,
}

/// Split stdout into images: escape codes `ESC _G <control> ; <base64> ESC \`, with `m=1` on
/// every chunk but an image's last. Anything between images must be a newline, a carriage return,
/// a cursor move (`ESC [ n A`, `B` or `C`), or a line of a list's names ([`names`]).
pub(crate) fn images(stdout: &[u8]) -> Vec<Drawn> {
    printed(stdout).0
}

/// The lines of names under a list's rows of tiles, each with its runs of spaces made one.
pub(crate) fn names(stdout: &[u8]) -> Vec<String> {
    printed(stdout).1
}

/// [`images`], and the lines of names among them.
fn printed(stdout: &[u8]) -> (Vec<Drawn>, Vec<String>) {
    let mut rest = std::str::from_utf8(stdout).unwrap();
    let (mut out, mut keys, mut payload, mut names) = (Vec::new(), None::<String>, String::new(), Vec::new());
    while !rest.is_empty() {
        if keys.is_none() {
            if let Some(after) = rest.strip_prefix('\n').or_else(|| rest.strip_prefix('\r')) {
                rest = after;
                continue;
            }
            // Text to the end of its line, without an escape code.
            if !rest.starts_with('\x1b') {
                let (line, after) = rest.split_once('\n').unwrap_or((rest, ""));
                assert!(!line.contains('\x1b'), "an escape code in a line of names: {line:?}");
                names.push(line.split(' ').filter(|w| !w.is_empty()).fold(String::new(), |all, w| all + " " + w));
                rest = after;
                continue;
            }
            if let Some(after) = rest.strip_prefix("\x1b[") {
                let digits = after.find(|c: char| !c.is_ascii_digit()).unwrap();
                if digits > 0 && after[digits..].starts_with(['A', 'B', 'C']) {
                    rest = &after[digits + 1..];
                    continue;
                }
            }
        }
        let body = rest.strip_prefix("\x1b_G").unwrap_or_else(|| panic!("expected an escape code at {:?}", &rest[..rest.len().min(20)]));
        let end = body.find("\x1b\\").unwrap();
        let (control, data) = body[..end].split_once(';').unwrap();
        keys.get_or_insert_with(|| control.to_string());
        payload.push_str(data);
        if control.ends_with("m=0") {
            let png = base64::engine::general_purpose::STANDARD.decode(&payload).unwrap();
            out.push(Drawn { keys: keys.take().unwrap(), png });
            payload.clear();
        } else {
            assert!(control.ends_with("m=1"), "{control}");
        }
        rest = &body[end + 2..];
    }
    assert!(keys.is_none(), "an image was left unfinished");
    (out, names)
}

pub(crate) fn png_size(png: &[u8]) -> (u32, u32) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&png[12..16], b"IHDR");
    let be32 = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    (be32(&png[16..20]), be32(&png[20..24]))
}

/// The value of `key` in a control string such as `a=T,c=40,r=20`.
fn key(control: &str, key: &str) -> u32 {
    control.split(',').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('=')).unwrap().parse().unwrap()
}

#[test]
fn each_page_is_a_png_shown_at_the_cursor() {
    let pdf = fixture("pages", 2);
    let out = view(&pdf, &["--pages", "2,1", "--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let drawn = images(&out.stdout);
    assert_eq!(drawn.len(), 2);
    for image in &drawn {
        assert!(image.keys.starts_with("a=T,f=100,t=d,q=2,"), "{}", image.keys);
        let (w, h) = png_size(&image.png);
        // The page's shape, whatever the size of the terminal running the tests.
        let ratio = w as f32 / h as f32;
        assert!((ratio - 612.0 / 792.0).abs() < 0.01, "{w}×{h}");
        assert!(key(&image.keys, "c") >= 1 && key(&image.keys, "r") >= 1);
    }
}

#[test]
fn zoom_width_is_wider_than_a_page_to_a_row() {
    let pdf = fixture("fit", 1);
    let page = images(&view(&pdf, &["--protocol", "kitty", "--zoom", "1"]).stdout);
    let width = images(&view(&pdf, &["--protocol", "kitty", "--zoom", "width"]).stdout);
    let (cols_page, cols_width) = (key(&page[0].keys, "c"), key(&width[0].keys, "c"));
    // A portrait page fits the height of any window wider than it is tall in cells.
    assert!(cols_width >= cols_page, "{cols_width} < {cols_page}");
    assert!(png_size(&width[0].png).0 >= png_size(&page[0].png).0);
    // As the width, the page moves the cursor down and the screen with it, as text does: a page
    // taller than the window scrolls by.
    assert!(width[0].keys.starts_with("a=T,f=100,t=d,q=2,c="), "{}", width[0].keys);
    // Auto prints one page as one to a row does.
    let auto = images(&view(&pdf, &["--protocol", "kitty", "--zoom", "auto"]).stdout);
    assert_eq!((key(&auto[0].keys, "c"), png_size(&auto[0].png)), (cols_page, png_size(&page[0].png)));
    // Three to a row is a size, alone or not: a page in a third of the 80 columns, 25 of them.
    let third = images(&view(&pdf, &["--protocol", "kitty", "--zoom", "3"]).stdout);
    assert_eq!(key(&third[0].keys, "c"), 25, "{}", third[0].keys);
    assert!(png_size(&third[0].png).0 < png_size(&page[0].png).0);
}

#[test]
fn without_a_terminal_auto_points_to_the_protocol_option() {
    let pdf = fixture("auto", 1);
    let out = view(&pdf, &[]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    let stderr = text(&out.stderr);
    assert!(stderr.contains("stdout is not a terminal") && stderr.contains("--protocol kitty"), "{stderr}");
}

#[test]
fn page_numbers_are_checked_before_anything_is_drawn() {
    let pdf = fixture("past", 1);
    let out = view(&pdf, &["--pages", "1,3", "--protocol", "kitty"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(text(&out.stderr).contains("no page 3"), "{}", text(&out.stderr));
}

#[test]
fn bad_arguments_say_what_is_allowed() {
    let pdf = fixture("args", 1);
    for (args, expected) in [
        (&["--zoom", "sideways"][..], "use auto, width or a number of pages per row (1–8)"),
        (&["--zoom", "9"], "use auto, width or a number of pages per row (1–8)"),
        (&["--protocol", "sixel"], "use auto, kitty, blocks or ascii"),
        (&["--fit", "width"], "unknown option --fit"),
        (&["--pages", "0"], "bad page number"),
        (&["--protocol", "kitty", "--pages"], "--pages needs a value"),
    ] {
        let out = view(&pdf, args);
        assert!(!out.status.success(), "{args:?}");
        assert!(text(&out.stderr).contains(expected), "{args:?}: {}", text(&out.stderr));
    }
}

/// Nothing given at all is a call for help: what `--help` prints, and no error.
#[test]
fn no_arguments_print_the_help() {
    let help = Command::new(BIN).arg("--help").output().unwrap();
    let out = Command::new(BIN).output().unwrap();
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), text(&help.stdout));
    let end =
        "\n\nManual: https://github.com/dobladez/clview/blob/main/docs/usage.md\nReport issues and ideas to: https://github.com/dobladez/clview\n";
    assert!(text(&out.stdout).ends_with(end), "{}", text(&out.stdout));
}

/// Run `clv` as if in a tmux pane, with no `tmux` command on the PATH to ask.
fn view_in_tmux(test: &str, pdf: &Path, args: &[&str]) -> Output {
    let empty = tmp(test, "empty-bin");
    std::fs::create_dir_all(&empty).unwrap();
    Command::new(BIN)
        .arg(pdf)
        .args(args)
        .env("TMUX", "/nonexistent/tmux-socket,1,0")
        .env_remove("TMUX_PANE")
        .env_remove("KITTY_WINDOW_ID")
        .env_remove("TERM_PROGRAM")
        .env("TERM", "tmux-256color")
        .env("PATH", &empty)
        .output()
        .unwrap()
}

/// The bytes inside the tmux passthrough sequences at the start of `bytes` (`ESC P tmux; …
/// ESC \`, with every inner ESC doubled), and what follows them.
fn passthrough(bytes: &[u8]) -> (Vec<u8>, &[u8]) {
    let mut inner = Vec::new();
    let mut rest = bytes;
    while let Some(body) = rest.strip_prefix(b"\x1bPtmux;") {
        let mut i = 0;
        loop {
            match (body[i], body[i + 1]) {
                (0x1b, 0x1b) => {
                    inner.push(0x1b);
                    i += 2;
                }
                (0x1b, b'\\') => {
                    i += 2;
                    break;
                }
                (b, _) => {
                    assert_ne!(b, 0x1b, "a lone ESC inside a passthrough sequence");
                    inner.push(b);
                    i += 1;
                }
            }
        }
        rest = &body[i..];
    }
    (inner, rest)
}

#[test]
fn inside_tmux_pages_go_through_passthrough_and_placeholders_draw_them() {
    let pdf = fixture("tmux", 1);
    let out = view_in_tmux("tmux", &pdf, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    // With no tmux to ask, `clv` says it couldn't check passthrough, and goes on.
    assert!(text(&out.stderr).contains("allow-passthrough"), "{}", text(&out.stderr));

    // The image is transmitted, then placed virtually, all through passthrough.
    let (codes, after) = passthrough(&out.stdout);
    let mut transmitted: Vec<(&str, &str)> = std::str::from_utf8(&codes)
        .unwrap()
        .strip_suffix("\x1b\\")
        .unwrap()
        .split("\x1b\\")
        .map(|code| {
            let body = code.strip_prefix("\x1b_G").unwrap();
            body.split_once(';').unwrap_or((body, ""))
        })
        .collect();
    let (place, _) = transmitted.pop().unwrap();
    let id = key(transmitted[0].0, "i");
    assert!(transmitted[0].0.starts_with("a=t,f=100,t=d,q=2,i="), "{}", transmitted[0].0);
    // tmux couldn't say whether it passes 24-bit colour, so a palette colour carries the low 8
    // bits and a third diacritic the high byte; nothing is left for the bits in between.
    assert_eq!(id & 0x00FF_FF00, 0, "{id:#x}");
    assert_ne!(id & 0xff, 0, "{id:#x}");
    let png = base64::engine::general_purpose::STANDARD.decode(transmitted.iter().map(|(_, p)| *p).collect::<String>()).unwrap();
    let (w, h) = png_size(&png);
    assert!((w as f32 / h as f32 - 612.0 / 792.0).abs() < 0.01, "{w}×{h}");
    let (cols, rows) = (key(place, "c"), key(place, "r"));
    assert_eq!(place, format!("a=p,U=1,q=2,i={id},c={cols},r={rows}"));

    // Then a line of placeholder cells per row, in the colour that names the image.
    let lines: Vec<&str> = std::str::from_utf8(after).unwrap().split_terminator('\n').collect();
    assert_eq!(lines.len(), rows as usize);
    for line in lines {
        assert!(line.starts_with(&format!("\x1b[38;5;{}m", id & 0xff)) && line.ends_with("\x1b[39m"), "{line:?}");
        assert_eq!(line.chars().filter(|c| *c == '\u{10EEEE}').count(), cols as usize);
    }
}

/// Printed inside tmux, pages side by side share their lines of placeholder cells: the first
/// page's, the gap, then the second's, each in the colour that names its image.
#[test]
fn inside_tmux_pages_side_by_side_share_their_lines() {
    let pdf = fixture("tmux-zoom", 2);
    let out = view_in_tmux("tmux-zoom", &pdf, &["--protocol", "kitty", "--zoom", "2"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let (codes, after) = passthrough(&out.stdout);
    let codes = std::str::from_utf8(&codes).unwrap();
    let placed: Vec<&str> = codes.split("\x1b\\").filter_map(|code| code.strip_prefix("\x1b_Ga=p,U=1,")).collect();
    assert_eq!(placed.len(), 2, "{codes:?}");
    let (first, second) = (key(placed[0], "i"), key(placed[1], "i"));
    assert_ne!(first, second);
    let (cols, rows) = (key(placed[0], "c"), key(placed[0], "r"));
    let lines: Vec<&str> = std::str::from_utf8(after).unwrap().split_terminator('\n').collect();
    assert_eq!(lines.len(), rows as usize);
    for line in lines {
        let (left, right) = line.split_once("\x1b[39m  ").unwrap_or_else(|| panic!("no gap between the pages: {line:?}"));
        assert!(left.starts_with(&format!("\x1b[38;5;{}m", first & 0xff)), "{line:?}");
        assert!(right.starts_with(&format!("\x1b[38;5;{}m", second & 0xff)) && right.ends_with("\x1b[39m"), "{line:?}");
        assert_eq!(line.chars().filter(|c| *c == '\u{10EEEE}').count(), 2 * cols as usize);
    }
}

#[test]
fn inside_tmux_auto_still_needs_a_terminal() {
    let pdf = fixture("tmux-auto", 1);
    let out = view_in_tmux("tmux-auto", &pdf, &[]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(text(&out.stderr).contains("stdout is not a terminal"), "{}", text(&out.stderr));
}

#[test]
fn a_hostile_file_name_cant_send_escape_sequences_to_the_terminal() {
    // A name that would write to the clipboard (OSC 52) and retitle the window if printed raw.
    let dir = tmp("escape", "");
    let name = dir.join("x\x1b]52;c;cHduZWQ=\x07y\x1b]0;owned\x07.pdf");
    for args in [&[][..], &["--protocol", "kitty"], &["--pages", "x\x1b[2J"]] {
        let out = Command::new(BIN).arg(&name).args(args).env_remove("TMUX").output().unwrap();
        assert!(!out.status.success());
        let stderr = text(&out.stderr);
        // A bad page list is said, quoted with its escapes spelled out, before any file is read.
        let said = if args.len() == 2 && args[0] == "--pages" { r"x\u{1b}[2J" } else { "x\u{FFFD}]52;c;cHduZWQ=\u{FFFD}y" };
        assert!(stderr.contains(said), "{stderr:?}");
        assert!(!out.stderr.iter().any(|b| *b == 0x1b || *b == 0x07), "{stderr:?}");
    }
}

#[test]
fn compression_trades_bytes_for_speed_and_auto_follows_ssh() {
    let pdf = fixture("compression", 1);
    let png = |args: &[&str], env: &[(&str, &str)]| {
        let out = view_with(&pdf, &[&["--protocol", "kitty"], args].concat(), env);
        assert!(out.status.success(), "{}", text(&out.stderr));
        images(&out.stdout).remove(0).png
    };
    let (fast, small) = (png(&["--compression", "fast"], &[]), png(&["--compression", "small"], &[]));
    // The same page, in more bytes for less work.
    assert_eq!(png_size(&fast), png_size(&small));
    assert!(fast.len() > small.len(), "fast {} bytes, small {}", fast.len(), small.len());
    // auto: fast on this machine, small when the login came over SSH.
    assert_eq!(png(&[], &[]), fast);
    assert_eq!(png(&[], &[("SSH_CONNECTION", "192.0.2.7 50122 192.0.2.1 22")]), small);
    assert_eq!(png(&["--compression", "fast"], &[("SSH_TTY", "/dev/pts/3")]), fast);
    let bad = view(&pdf, &["--protocol", "kitty", "--compression", "best"]);
    assert!(text(&bad.stderr).contains("use auto, fast or small"));
}

#[test]
fn help_and_version() {
    let out = Command::new(BIN).arg("--help").output().unwrap();
    let help = text(&out.stdout);
    let is = "Command-line Viewer: view graphic files without leaving your terminal";
    let start = format!("clview {}\n{is}\n\nclv [options] <file>…\nclv --help | --version\n", env!("CARGO_PKG_VERSION"));
    assert!(out.status.success() && help.starts_with(&start), "{help}");
    assert!(help.contains("\n\nGeneral Options:\n  --pages ") && help.contains("\n  --zoom auto|width|N "), "{help}");
    // A backend that takes options of its own lists them under a heading of its own, what it
    // shows: the PDF one, and the documents' and spreadsheets' when they are built.
    let heading = |(line, next): &(&str, &str)| line.starts_with("   for ") && line.ends_with(':') && next.starts_with("     --");
    let headings = help.lines().zip(help.lines().skip(1)).filter(heading).count();
    assert_eq!(headings, 1 + usize::from(cfg!(feature = "word")) + usize::from(cfg!(feature = "sheet")), "{help}");
    assert!(help.contains("\n\n   for PDFs:\n     --password PW ") && !help.contains("no options"), "{help}");
    // Then the formats shown, by the libraries each backend credits: a backend's only when it is
    // built. Which libraries, each backend's own tests say.
    let shown = |what: &str, formats: &str| {
        let row =
            |line: &str| line.strip_prefix(&format!("  {what}, by ")).and_then(|rest| rest.split_once(": ")).map(|(_, listed)| listed.to_owned());
        help.lines().filter_map(row).any(|listed| listed.starts_with(formats))
    };
    assert!(shown("PDFs", "PDF"), "{help}");
    assert_eq!(shown("Images", "PNG, JPEG, "), cfg!(feature = "image"), "{help}");
    assert_eq!(shown("SVG drawings", "SVG (.svg), svgz (.svgz)"), cfg!(feature = "svg"), "{help}");
    assert_eq!(shown("Zip archives", "zip"), cfg!(feature = "zip"), "{help}");
    assert_eq!(shown("Tar archives and compressed files", "tar, gzip, "), cfg!(feature = "tar"), "{help}");
    assert_eq!(shown("Documents", "Word (.docx "), cfg!(feature = "word"), "{help}");
    assert_eq!(shown("Spreadsheets", "Excel (.xlsx "), cfg!(feature = "sheet"), "{help}");
    let out = Command::new(BIN).arg("--version").output().unwrap();
    assert_eq!(text(&out.stdout), format!("clv {}\n", env!("CARGO_PKG_VERSION")));
}

/// A file is shown by the first backend that opens it, going by its first bytes, not its name.
#[test]
fn a_file_no_backend_opens_is_refused_and_a_pdf_is_known_by_its_header() {
    let notes = tmp("backends", "notes.pdf");
    std::fs::write(&notes, "not a PDF, whatever its name says\n").unwrap();
    let out = view(&notes, &["--protocol", "kitty"]);
    assert_eq!(text(&out.stderr), format!("clv: {}: not a file clv shows (it shows {})\n", notes.display(), shows()));
    assert!(!out.status.success() && out.stdout.is_empty());
    let pdf = tmp("backends", "page");
    std::fs::write(&pdf, crate::fixtures::letter("No extension", 1)).unwrap();
    let out = view(&pdf, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(images(&out.stdout).len(), 1);
}

/// The file is read once, so a PDF can come through a pipe: the bytes read to choose its backend
/// are not lost to it.
#[test]
fn a_pdf_through_a_pipe_is_shown() {
    use std::io::Write as _;
    let mut child = Command::new(BIN)
        .args(["/dev/stdin", "--protocol", "kitty"])
        .env("TERM", "dumb")
        .env_remove("TMUX")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&crate::fixtures::letter("Piped", 2)).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(images(&out.stdout).len(), 2);
}

/// A picture through a pipe can't be read again when it renders, so it is kept as it came.
#[cfg(feature = "image")]
#[test]
fn a_picture_through_a_pipe_is_shown() {
    use std::io::Write as _;
    let png = std::fs::read(picture("piped", "photo.png", 30, 20)).unwrap();
    let mut child = Command::new(BIN)
        .args(["/dev/stdin", "--protocol", "kitty"])
        .env("TERM", "dumb")
        .env_remove("TMUX")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&png).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(images(&out.stdout).iter().map(|d| png_size(&d.png)).collect::<Vec<_>>(), [(30, 20)]);
}

/// A `width` × `height` PNG made by the test.
#[cfg(feature = "image")]
fn picture(test: &str, name: &str, width: u32, height: u32) -> PathBuf {
    let path = tmp(test, name);
    let rgba: Vec<u8> = (0..width * height).flat_map(|i| [(i % 251) as u8, 128, 64, 255]).collect();
    image::save_buffer(&path, &rgba, width, height, image::ExtendedColorType::Rgba8).unwrap();
    path
}

/// An image is one page, shown no larger than it is, and a PDF's options don't apply to it.
#[cfg(feature = "image")]
#[test]
fn an_image_is_one_page_its_own_size_and_takes_no_pdf_options() {
    let png = picture("image", "photo.png", 300, 200);
    let out = view(&png, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let drawn = images(&out.stdout);
    assert_eq!(drawn.len(), 1);
    // Its own size, 30 × 10 cells of 10 × 20 px, in an 800 × 460 px window it would fill the
    // height of.
    assert_eq!(png_size(&drawn[0].png), (300, 200));
    assert!(drawn[0].keys.contains(",c=30,r=10,"), "{}", drawn[0].keys);
    // As wide as the window when asked.
    let wide = images(&view(&png, &["--protocol", "kitty", "--zoom", "width"]).stdout);
    assert_eq!(png_size(&wide[0].png), (800, 533));
    let out = view(&png, &["--protocol", "kitty", "--password", "pw"]);
    assert_eq!(text(&out.stderr), format!("clv: --password doesn't apply to {}, which image opens\n", png.display()));
    assert!(!out.status.success() && out.stdout.is_empty());
    let out = view(&png, &["--protocol", "kitty", "--pages", "2"]);
    assert_eq!(text(&out.stderr), "clv: no page 2 (photo.png has 1 page)\n");
}

/// Several files are a list, each by its first page, its name under it. `--pages` picks the
/// list's tiles, in its order.
#[cfg(feature = "image")]
#[test]
fn several_files_are_a_list_each_by_its_first_page() {
    let pdf = fixture("several", 2);
    let png = picture("several", "photo.png", 300, 200);
    let both = |args: &[&str]| view(&pdf, &[&[png.to_str().unwrap(), "--protocol", "kitty"], args].concat());
    let out = both(&[]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let sizes: Vec<(u32, u32)> = images(&out.stdout).iter().map(|d| png_size(&d.png)).collect();
    assert_eq!(sizes.len(), 2, "{sizes:?}");
    assert_eq!(sizes[1], (300, 200));
    assert_eq!(names(&out.stdout), [" many.pdf photo.png"]);
    // The second tile, not each file's second page: the picture, which has no second page.
    let second = both(&["--pages", "2"]);
    assert!(second.status.success() && second.stderr.is_empty(), "{}", text(&second.stderr));
    assert_eq!(names(&second.stdout), [" photo.png"]);
    // In the order given, each still by its first page.
    let turned = both(&["--pages", "2,1"]);
    let turned_sizes: Vec<(u32, u32)> = images(&turned.stdout).iter().map(|d| png_size(&d.png)).collect();
    assert_eq!((turned_sizes, names(&turned.stdout)), (vec![sizes[1], sizes[0]], vec![" photo.png many.pdf".to_string()]));
    let past = both(&["--pages", "3"]);
    assert!(!past.status.success() && past.stdout.is_empty());
    assert_eq!(text(&past.stderr), "clv: no tile 3 (the list has 2 tiles)\n");
    // An option one of the files takes is no error; one none takes is.
    assert!(both(&["--password", "pw"]).status.success());
    let png2 = picture("several", "other.png", 30, 20);
    let out = view(&png, &[png2.to_str().unwrap(), "--protocol", "kitty", "--password", "pw"]);
    assert_eq!(text(&out.stderr), "clv: --password doesn't apply to any of these files, which image opens\n");
}

#[test]
fn a_file_that_cant_be_shown_among_others_is_left_out() {
    let pdf = fixture("left-out", 2);
    let notes = tmp("left-out", "notes.txt");
    std::fs::write(&notes, "plain text\n").unwrap();
    let out = view(&pdf, &[notes.to_str().unwrap(), "--protocol", "kitty"]);
    assert_eq!((images(&out.stdout).len(), names(&out.stdout)), (1, vec![" many.pdf".to_string()]));
    let why = format!("{}: not a file clv shows", notes.display());
    let stderr = text(&out.stderr);
    assert!(stderr.starts_with(&format!("warning: {why}")), "{stderr}");
    assert!(stderr.ends_with("clv: 1 file left out: notes.txt\n"), "{stderr}");
    assert!(!out.status.success());
    let out = view(&notes, &[notes.to_str().unwrap(), "--protocol", "kitty"]);
    assert!(!out.status.success() && out.stdout.is_empty());
    assert!(text(&out.stderr).ends_with("clv: none of the 2 files can be shown\n"), "{}", text(&out.stderr));
    // A tile past the end of a list made shorter by a file left out: why it is shorter is said.
    let out = view(&pdf, &[notes.to_str().unwrap(), "--protocol", "kitty", "--pages", "2"]);
    assert!(!out.status.success() && out.stdout.is_empty());
    let stderr = text(&out.stderr);
    assert!(stderr.starts_with(&format!("warning: {why}")) && stderr.lines().count() == 2, "{stderr}");
    assert!(stderr.ends_with("\nclv: no tile 2 (the list has 1 tile)\n"), "{stderr}");
}

/// A directory shows each file a backend opens by its first page, and each subdirectory as a
/// folder, in the order of their names (numbers by value); hidden files and files nothing shows
/// are passed over. With `-r`, the subdirectories' files instead of folders.
#[cfg(feature = "image")]
#[test]
fn a_directory_shows_each_of_its_files_by_its_first_page_and_its_subdirectories_as_folders() {
    let dir = tmp("gallery", "photos");
    std::fs::create_dir_all(dir.join("2024")).unwrap();
    std::fs::write(dir.join("a10.pdf"), crate::fixtures::letter("Gallery", 3)).unwrap();
    picture("gallery", "photos/a9.png", 300, 200);
    picture("gallery", "photos/.hidden.png", 50, 50);
    picture("gallery", "photos/2024/b.png", 30, 20);
    std::fs::write(dir.join("notes.txt"), "plain text\n").unwrap();
    let shown = |args: &[&str]| {
        let out = view(&dir, &[&["--protocol", "kitty"], args].concat());
        assert!(out.status.success() && out.stderr.is_empty(), "{args:?}: {}", text(&out.stderr));
        (images(&out.stdout).iter().map(|d| png_size(&d.png)).collect::<Vec<_>>(), names(&out.stdout))
    };
    // 2024 before a9: a folder, wider than tall, then the picture, then the PDF's first page.
    // Three to a row in 80 columns: square places 250 × 240 px, which the picture fits into.
    let (sizes, labels) = shown(&[]);
    assert_eq!((sizes.len(), sizes[1], labels), (3, (250, 167), vec![" 2024/ a9.png a10.pdf".to_string()]), "{sizes:?}");
    assert!(sizes[0].0 > sizes[0].1, "a folder: {:?}", sizes[0]);
    let (w, h) = sizes[2];
    assert!((w as f32 / h as f32 - 612.0 / 792.0).abs() < 0.01, "the PDF's first page only: {w}×{h}");
    // Recursing, a subdirectory's files come at its place among the names.
    for recurse in ["-r", "--recurse"] {
        let (sizes, labels) = shown(&[recurse]);
        assert_eq!((sizes.len(), sizes[0], sizes[1]), (3, (30, 20), (250, 167)), "{recurse}");
        assert_eq!(labels, [" 2024/b.png a9.png a10.pdf"]);
    }
    // A row of tiles at a time, names under each row.
    let out = view(&dir, &["--protocol", "kitty", "-r", "--zoom", "2"]);
    assert_eq!(names(&out.stdout), [" 2024/b.png a9.png", " a10.pdf"]);
    // Among others, a directory is a folder, and an option that a file inside it may take is no
    // error.
    let out = view(&dir, &[dir.join("2024").to_str().unwrap(), "--protocol", "kitty", "--password", "pw"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(names(&out.stdout), [" photos/ 2024/"]);
}

/// A directory with nothing to show says so; a file in it that should show and doesn't is left
/// out, as one given by name is.
#[cfg(feature = "image")]
#[test]
fn a_directory_with_nothing_to_show_says_so() {
    let dir = tmp("empty-gallery", "photos");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.txt"), "plain text\n").unwrap();
    let out = view(&dir, &["--protocol", "kitty"]);
    assert!(!out.status.success() && out.stdout.is_empty());
    assert_eq!(text(&out.stderr), format!("clv: {}: nothing in it that clv shows\n", dir.display()));
    // A subdirectory is a folder to show, and recursing, its files.
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub/one.pdf"), crate::fixtures::letter("Below", 1)).unwrap();
    assert_eq!(names(&view(&dir, &["--protocol", "kitty"]).stdout), [" sub/"]);
    assert_eq!(images(&view(&dir, &["--protocol", "kitty", "-r"]).stdout).len(), 1);
    // A picture whose header is broken: opened by the image backend, which then fails.
    std::fs::write(dir.join("broken.png"), b"\x89PNG\r\n\x1a\nno chunks").unwrap();
    let out = view(&dir, &["--protocol", "kitty", "-r"]);
    assert_eq!(images(&out.stdout).len(), 1);
    let stderr = text(&out.stderr);
    assert!(stderr.starts_with(&format!("warning: {}: ", dir.join("broken.png").display())), "{stderr}");
    assert!(stderr.ends_with("clv: 1 file left out: broken.png\n") && !out.status.success(), "{stderr}");
}

fn shows() -> String {
    let backends = [
        ("SVG drawings", cfg!(feature = "svg")),
        ("tar archives and compressed files", cfg!(feature = "tar")),
        ("documents", cfg!(feature = "word")),
        ("spreadsheets", cfg!(feature = "sheet")),
        ("PDFs", true),
        ("images", cfg!(feature = "image")),
        ("zip archives", cfg!(feature = "zip")),
    ];
    let shown: Vec<&str> = backends.into_iter().filter_map(|(what, built)| built.then_some(what)).collect();
    match shown.split_last() {
        Some((last, [])) => (*last).to_string(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// Errors are said once, under the command's name.
#[test]
fn errors_name_the_command() {
    let out = Command::new(BIN).arg("--zoom").arg("2").output().unwrap();
    assert_eq!(text(&out.stderr), "clv: missing file\n");
    let out = Command::new(BIN).arg("--frobnicate").output().unwrap();
    assert_eq!(text(&out.stderr), "clv: unknown option --frobnicate\n");
}

/// ASCII through a pipe is plain text, for whatever reads it: no escape codes and nothing past
/// ASCII, whatever the environment says of colours, the same width all the way down, and a white
/// page all ink, as nothing says the background is light.
#[test]
fn ascii_through_a_pipe_is_plain_text() {
    let pdf = fixture("ascii", 1);
    let out = view_with(&pdf, &["--protocol", "ascii"], &[("COLORTERM", "truecolor"), ("TERM", "xterm-256color")]);
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    assert!(out.stdout.is_ascii() && !out.stdout.contains(&0x1b), "{}", text(&out.stdout));
    let page = text(&out.stdout);
    let lines: Vec<&str> = page.lines().collect();
    assert!(lines.len() > 1 && lines.iter().all(|line| line.len() == lines[0].len()), "{page}");
    assert!(page.matches('@').count() > page.len() / 2, "{page}");
    // The text on the page is there, as less ink.
    assert!(lines.iter().any(|line| line.contains(|c| !matches!(c, '@' | ' '))), "{page}");
}

/// Blocks keep the colours the environment says, for a terminal that shows them later, and shade
/// where it says there are none. No graphics either way, and the colours go back at each line's end.
#[test]
fn blocks_take_their_colours_from_the_environment() {
    let pdf = fixture("blocks", 1);
    const WHITE_RGB: &str = "\x1b[48;2;255;255;255m";
    const WHITE_256: &str = "\x1b[48;5;231m";
    let page = |env: &[(&str, &str)]| {
        let out = view_with(&pdf, &["--protocol", "blocks"], env);
        assert!(out.status.success() && out.stderr.is_empty(), "{env:?}: {}", text(&out.stderr));
        let page = text(&out.stdout);
        assert!(!page.contains("\x1b_G"), "{env:?}: {page}");
        page
    };
    let rgb = page(&[("COLORTERM", "truecolor")]);
    assert!(rgb.contains(WHITE_RGB) && !rgb.contains(WHITE_256), "{rgb}");
    assert!(rgb.lines().all(|line| line.ends_with("\x1b[0m")), "{rgb}");
    let palette = page(&[("TERM", "xterm-256color")]);
    assert!(palette.contains(WHITE_256) && !palette.contains(WHITE_RGB), "{palette}");
    for none in [&[("NO_COLOR", "1"), ("COLORTERM", "truecolor")][..], &[]] {
        let shaded = page(none);
        assert!(!shaded.contains('\x1b') && shaded.contains('█'), "{none:?}: {shaded}");
    }
    // An empty NO_COLOR isn't one, as its convention has it.
    assert!(page(&[("NO_COLOR", ""), ("COLORTERM", "truecolor")]).contains(WHITE_RGB));
}

/// Pages side by side are text side by side: each line both pages' cells, with the gap between.
#[test]
fn text_cells_print_in_rows_as_zoomed() {
    let pdf = fixture("ascii-rows", 3);
    let out = view(&pdf, &["--protocol", "ascii", "--zoom", "2"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let page = text(&out.stdout);
    let lines: Vec<&str> = page.lines().collect();
    // A page's top is blank, so all ink: two runs of it on a row of two.
    assert_eq!(lines[0].split_whitespace().count(), 2, "{page}");
    // The second row's one page, under the first of the row above.
    let rows: Vec<&[&str]> = lines.split(|line| line.is_empty()).collect();
    assert_eq!(rows.len(), 2, "{page}");
    assert_eq!(rows[1][0].split_whitespace().count(), 1, "{page}");
    assert_eq!(rows[1][0].len(), rows[0][0].find(char::is_whitespace).unwrap(), "{page}");
}

/// A list's tiles are text too, each row with its names under it.
#[test]
fn a_lists_tiles_print_as_text_with_their_names() {
    let (a, b) = (tmp("ascii-list", "first.pdf"), tmp("ascii-list", "second.pdf"));
    std::fs::write(&a, crate::fixtures::letter("A", 1)).unwrap();
    std::fs::write(&b, crate::fixtures::letter("B", 1)).unwrap();
    let out = view(&a, &[b.to_str().unwrap(), "--protocol", "ascii"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let page = text(&out.stdout);
    let names = page.lines().last().unwrap();
    assert!(names.contains("first.pdf") && names.contains("second.pdf"), "{page}");
    assert!(page.lines().next().unwrap().split_whitespace().count() == 2, "{page}");
}
