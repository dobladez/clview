//! `clv` on a terminal, end to end: on a pseudo-terminal of its own that script(1) provides, so
//! the terminal running the tests is never touched. Keys go in through script's stdin; what
//! `clv` draws comes out of its stdout. The window's size is set with stty; no pixel size is
//! reported, and the terminal these tests play doesn't know its cell size unless a test says so,
//! so cells are the 10 × 20 px `clv` assumes.
#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_clv");

/// How long anything may take: generous, for a slow CI machine.
const PATIENCE: Duration = Duration::from_secs(60);

/// A US Letter document with `pages` pages.
fn fixture(test: &str, pages: usize) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("clview-pager-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pdf = dir.join(if pages == 1 { "one.pdf" } else { "many.pdf" });
    std::fs::write(&pdf, crate::fixtures::letter("Pager test", pages)).unwrap();
    pdf
}

/// `line` run by a shell on a new pseudo-terminal: BSD script (macOS, FreeBSD) takes the command
/// after the output file; util-linux script (Linux) takes it as one shell line.
fn script(line: &str) -> Command {
    let mut script = Command::new("script");
    if cfg!(target_os = "linux") {
        script.args(["-q", "-e", "-c", line, "/dev/null"]);
    } else {
        script.args(["-q", "/dev/null", "sh", "-c", line]);
    }
    script
}

/// Whether script(1) gives a command a pseudo-terminal of the size asked for, which every test
/// here stands on. Where it can't (no script, or one that can't open a terminal), the tests skip
/// with a warning instead of failing over something that isn't `clv`'s.
pub(crate) fn pty_works() -> bool {
    static WORKS: OnceLock<bool> = OnceLock::new();
    *WORKS.get_or_init(|| {
        let works = (|| {
            let mut child =
                script("stty rows 5 cols 7 && stty size").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
            let deadline = Instant::now() + Duration::from_secs(20);
            while child.try_wait().ok()?.is_none() {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let mut out = String::new();
            child.stdout.take()?.read_to_string(&mut out).ok()?;
            Some(out.contains("5 7"))
        })()
        .unwrap_or(false);
        if !works {
            eprintln!("warning: skipped: script(1) can't give a command a pseudo-terminal here");
        }
        works
    })
}

/// `view pdf args…` running on a `cols` × `rows` pseudo-terminal.
pub(crate) struct Session {
    child: Child,
    /// script's stdin: keys, and the terminal's answers to `clv`'s questions.
    keys: Arc<Mutex<Option<ChildStdin>>>,
    output: Arc<Mutex<Vec<u8>>>,
    /// The files taken, as a terminal on this machine takes them, or what was wrong with one.
    taken: Arc<Mutex<Vec<Result<PathBuf, String>>>>,
}

/// The terminal at the other end of the pseudo-terminal: whether it reads the temporary files
/// `clv` names, as kitty does (and deletes them), the temporary directory `clv` gets, the
/// cell size it answers when asked (the device attributes alone when `None`), whether it says it
/// speaks the clipboard protocol when asked and answers a write as kitty does (`None`: it never
/// answers), whether `clv` runs over SSH, the `TERM` it names (`None`: whatever the tests got), and
/// the background colour it says when asked (the device attributes alone when `None`).
#[derive(Clone, Copy)]
struct Terminal<'a> {
    takes_files: bool,
    tmpdir: Option<&'a Path>,
    cell: Option<(u16, u16)>,
    clipboard: Option<bool>,
    ssh: bool,
    term: Option<&'a str>,
    background: Option<&'a str>,
}

/// One that never reads files, as script(1) doesn't, and doesn't know its cell size.
const PLAIN: Terminal = Terminal { takes_files: false, tmpdir: None, cell: None, clipboard: None, ssh: false, term: None, background: None };

/// One that doesn't speak the graphics protocol: whatever terminal the tests run in, and with xterm's
/// 256 colours, which is what the environment says with no `COLORTERM`.
const TEXT_ONLY: Terminal = Terminal { term: Some("xterm-256color"), ..PLAIN };

/// `clv`'s question about the cell size, which a pseudo-terminal without pixels gets.
const CELL_QUERY: &[u8] = b"\x1b[16t\x1b[c";

/// `clv`'s question about the background, which ink on it depends on.
const BACKGROUND_QUERY: &[u8] = b"\x1b]11;?\x1b\\\x1b[c";

/// `clv`'s question whether the terminal speaks the clipboard protocol (DECRQM for mode 5522).
const CLIPBOARD_QUERY: &[u8] = b"\x1b[?5522$p";

/// How the last packet of a clipboard write starts: its id, then the end of the packet.
const WRITE_END: &[u8] = b"\x1b]5522;type=wdata:id=";

impl Session {
    pub(crate) fn start(pdf: &Path, args: &[&str], cols: u16, rows: u16) -> Self {
        Self::start_on(PLAIN, pdf, args, cols, rows)
    }

    fn start_on(terminal: Terminal, pdf: &Path, args: &[&str], cols: u16, rows: u16) -> Self {
        let quote = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
        let command = [BIN, pdf.to_str().unwrap()].iter().chain(args).map(|a| quote(a)).collect::<Vec<_>>().join(" ");
        Self::start_command(terminal, &format!("exec {command}"), cols, rows)
    }

    /// `clv` as `command` runs it, a shell line, on a `cols` × `rows` pseudo-terminal of `terminal`.
    fn start_command(terminal: Terminal, command: &str, cols: u16, rows: u16) -> Self {
        let mut script = script(&format!("stty rows {rows} cols {cols} && {command}"));
        if let Some(dir) = terminal.tmpdir {
            script.env("TMPDIR", dir);
        }
        // On this machine, whatever machine the tests run from: an SSH login would turn the files off.
        for name in ["TMUX", "TMUX_PANE", "SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY", "KITTY_WINDOW_ID", "TERM_PROGRAM", "COLORTERM", "NO_COLOR"] {
            script.env_remove(name);
        }
        if let Some(term) = terminal.term {
            script.env("TERM", term);
        }
        if terminal.ssh {
            script.env("SSH_CONNECTION", "192.0.2.1 50000 192.0.2.2 22");
        }
        let mut child = script.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        let output = Arc::new(Mutex::new(Vec::new()));
        let taken = Arc::new(Mutex::new(Vec::new()));
        let keys = Arc::new(Mutex::new(child.stdin.take()));
        let mut stdout = child.stdout.take().unwrap();
        let (collected, took, answers) = (output.clone(), taken.clone(), keys.clone());
        let background = terminal.background.map(str::to_owned);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let (mut scanned, mut asked, mut clip_asked, mut written, mut background_asked) = (0, 0, 0, 0, 0);
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut output = collected.lock().unwrap();
                output.extend_from_slice(&buf[..n]);
                if terminal.takes_files {
                    scanned = take_files(&output, scanned, &mut took.lock().unwrap());
                }
                // Answer the cell size question as a terminal does, once per asking.
                while let Some(at) = output[asked..].windows(CELL_QUERY.len()).position(|w| w == CELL_QUERY) {
                    asked += at + CELL_QUERY.len();
                    let cell = terminal.cell.map(|(w, h)| format!("\x1b[6;{h};{w}t")).unwrap_or_default();
                    if let Some(stdin) = answers.lock().unwrap().as_mut() {
                        let _ = stdin.write_all(format!("{cell}\x1b[?62;c").as_bytes()).and_then(|()| stdin.flush());
                    }
                }
                // Answer the background question, as kitty and xterm do.
                while let Some(at) = output[background_asked..].windows(BACKGROUND_QUERY.len()).position(|w| w == BACKGROUND_QUERY) {
                    background_asked += at + BACKGROUND_QUERY.len();
                    let colour = background.as_ref().map(|c| format!("\x1b]11;{c}\x1b\\")).unwrap_or_default();
                    if let Some(stdin) = answers.lock().unwrap().as_mut() {
                        let _ = stdin.write_all(format!("{colour}\x1b[?62;c").as_bytes()).and_then(|()| stdin.flush());
                    }
                }
                // Answer the clipboard question, and the end of each write, as kitty 0.49.2 does.
                let Some(speaks) = terminal.clipboard else { continue };
                let mut said = Vec::new();
                while let Some(at) = output[clip_asked..].windows(CLIPBOARD_QUERY.len()).position(|w| w == CLIPBOARD_QUERY) {
                    clip_asked += at + CLIPBOARD_QUERY.len();
                    said.extend_from_slice(format!("\x1b[?5522;{}$y", if speaks { 2 } else { 0 }).as_bytes());
                }
                while let Some(at) = output[written..].windows(WRITE_END.len()).position(|w| w == WRITE_END) {
                    let id_at = written + at + WRITE_END.len();
                    let Some(end) = output[id_at..].windows(2).position(|w| w == b"\x1b\\") else { break };
                    written = id_at + end;
                    let id = String::from_utf8_lossy(&output[id_at..written]).into_owned();
                    if speaks {
                        said.extend_from_slice(format!("\x1b]5522;type=write:status=DONE:id={id}\x1b\\").as_bytes());
                    }
                }
                if let (false, Some(stdin)) = (said.is_empty(), answers.lock().unwrap().as_mut()) {
                    let _ = stdin.write_all(&said).and_then(|()| stdin.flush());
                }
            }
        });
        Session { child, keys, output, taken }
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.output.lock().unwrap()).into_owned()
    }

    /// Wait until `done` holds for what was drawn so far; panics with the output if it never does.
    pub(crate) fn wait_until(&self, what: &str, done: impl Fn(&str) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(&self.text()) {
            assert!(Instant::now() < deadline, "{what} never happened: {:?}", self.text());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub(crate) fn press(&mut self, keys: &str) {
        let mut stdin = self.keys.lock().unwrap();
        let stdin = stdin.as_mut().unwrap();
        stdin.write_all(keys.as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    /// `bytes` typed, whole characters or not.
    fn press_bytes(&mut self, bytes: &[u8]) {
        let mut stdin = self.keys.lock().unwrap();
        let stdin = stdin.as_mut().unwrap();
        stdin.write_all(bytes).unwrap();
        stdin.flush().unwrap();
    }

    /// Wait for `clv` to exit, all of its output collected.
    pub(crate) fn finish(&mut self) -> ExitStatus {
        let deadline = Instant::now() + PATIENCE;
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "view didn't exit: {:?}", self.text());
            std::thread::sleep(Duration::from_millis(20));
        };
        *self.keys.lock().unwrap() = None;
        // The reader thread has the last of the output once script's stdout closes.
        std::thread::sleep(Duration::from_millis(100));
        status
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Take the files that the file transmissions in `output` from `from` on name, as kitty does:
/// read each, check it holds the pixels its escape code says, and delete it. Returns where the
/// next look starts (at a transmission not yet whole).
fn take_files(output: &[u8], mut from: usize, taken: &mut Vec<Result<PathBuf, String>>) -> usize {
    use base64::Engine as _;
    use std::os::unix::ffi::OsStrExt as _;
    const START: &[u8] = b"\x1b_Ga=t,f=32,t=t,";
    while let Some(at) = output[from..].windows(START.len()).position(|w| w == START).map(|p| from + p) {
        let Some(end) = output[at..].windows(2).position(|w| w == b"\x1b\\").map(|p| at + p) else { return at };
        from = end + 2;
        let code = String::from_utf8_lossy(&output[at + 3..end]).into_owned();
        let (control, payload) = code.split_once(';').unwrap_or((&code, ""));
        let key = |k: &str| control.split(',').find_map(|kv| kv.strip_prefix(k)?.strip_prefix('=')?.parse::<u64>().ok());
        let path = base64::engine::general_purpose::STANDARD.decode(payload).map(|p| PathBuf::from(std::ffi::OsStr::from_bytes(&p)));
        taken.push(match (path, key("s"), key("v")) {
            (Ok(path), Some(w), Some(h)) => match std::fs::metadata(&path) {
                Ok(meta) if meta.len() == w * h * 4 => std::fs::remove_file(&path).map(|()| path).map_err(|e| e.to_string()),
                Ok(meta) => Err(format!("{}: {} bytes for {w} × {h} pixels", path.display(), meta.len())),
                Err(e) => Err(format!("{}: {e}", path.display())),
            },
            _ => Err(format!("unreadable: {code:?}")),
        });
    }
    output.len().saturating_sub(START.len()).max(from)
}

/// Whether the status line has said `place` with nothing left to render: without the `…` it
/// carries while the pages on screen render.
fn drawn(text: &str, place: &str) -> bool {
    text.contains(&format!("  {place} "))
}

/// `text` with each run of spaces made one: what the status line says, whatever its padding.
pub(crate) fn bar(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if !(c == ' ' && out.ends_with(' ')) {
            out.push(c);
        }
    }
    out
}

/// How many images were transmitted (for the pager) or shown at the cursor (when printing).
fn transmitted(text: &str) -> usize {
    text.matches("a=t,f=100,t=d,q=2,i=").count()
}

fn printed(text: &str) -> usize {
    text.matches("a=T,f=100,t=d,q=2,").count()
}

#[test]
fn more_than_a_page_opens_the_pager_and_pages_arrive_without_a_key() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("tall", 3);
    // A tall window: one page to the screen. No --pager: three pages on a terminal page.
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 80, 60);
    // No key yet: the first page arrives from the render thread on its own, its status line
    // replacing "rendering". Before, the pager waited on a read until a key came.
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/3"));
    let text = view.text();
    assert!(text.contains("\x1b[?1049h"), "the alternate screen");
    assert!(text.contains(",p=1,C=1,"), "placed without moving the cursor");

    // The next pages are rendered and sent ahead, still without a key.
    view.wait_until("pages 2 and 3 sent ahead", |t| transmitted(t) == 3);
    // So turning the page sends nothing: page 2 is only placed, and page 1 hidden, at once.
    view.press("n");
    view.wait_until("page 2 shown", |t| t.contains(" page 2/3 ") && t.contains("a=d,d=i,q=2,i="));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(transmitted(&view.text()), 3, "a page was sent again");

    // q quits and puts the screen back.
    view.press("q");
    assert!(view.finish().success());
    assert!(view.text().contains("\x1b[?1049l"), "the alternate screen was left");
}

/// `?` shows the keys in place of the pages, which come back, unsent, with any key.
#[test]
fn question_mark_shows_the_keys_in_place_of_the_pages() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("help", 3);
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 80, 60);
    view.wait_until("pages 1-3 sent", |t| drawn(t, "page 1/3") && transmitted(t) == 3);
    let before = view.text().len();
    view.press("?");
    let after = |t: &str| t.get(before..).unwrap_or_default().to_string();
    view.wait_until("the keys", |t| after(t).contains("these keys: any key goes back"));
    let text = view.text();
    assert!(after(&text).contains(",p=1\x1b\\") && after(&text).contains("a=d,d=i,q=2,i="), "page 1 not hidden: {:?}", after(&text));
    // Any key goes back to the pages, n too, without turning one: page 1 is placed again.
    let before = text.len();
    view.press("n");
    view.wait_until("page 1 again", |t| t.get(before..).is_some_and(|t| t.contains("a=p,q=1,") && drawn(t, "page 1/3")));
    assert_eq!(transmitted(&view.text()), 3, "a page was sent again");
    view.press("q");
    assert!(view.finish().success());
}

#[test]
fn a_wide_window_shows_two_pages_and_turns_two_at_a_time() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("wide", 3);
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 120, 24);
    view.wait_until("pages 1-2 shown", |t| drawn(t, "pages 1–2/3"));
    // Page 3, on the next spread, is sent ahead: three images, three ids.
    view.wait_until("page 3 sent ahead", |t| transmitted(t) == 3);
    let text = view.text();
    let ids: std::collections::HashSet<String> = text.match_indices("a=t,f=100,t=d,q=2,i=").map(|(at, _)| view_id(&text[at..])).collect();
    assert_eq!(ids.len(), 3, "{ids:?}");
    // Space turns the spread, page 3 alone on the left, with nothing sent.
    view.press(" ");
    view.wait_until("page 3 shown", |t| t.contains(" page 3/3 "));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(transmitted(&view.text()), 3, "a page was sent again");
    // +: one page to the screen, from where the reader is; p goes back one page. A page fits the
    // height of this window as well beside another as alone, so it is only placed again.
    view.press("+p");
    view.wait_until("page 2 alone", |t| bar(t).contains(" page 2/3 · 1 per row "));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(transmitted(&view.text()), 3, "a page was sent again");
    view.press("q");
    assert!(view.finish().success());
}

#[test]
fn minus_zooms_out_to_rows_of_pages_and_plus_in_to_the_width() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("zoom", 6);
    // A tall window: one page to the screen.
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 80, 48);
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/6") && bar(t).contains(" page 1/6 · 1 per row "));
    // Out twice: three to a row, in two rows, all six pages, each in a place of its own.
    view.press("--");
    view.wait_until("six pages shown", |t| bar(t).contains(" pages 1–6/6 · 3 per row ") && (1..=6).all(|p| t.contains(&format!(",p={p},C=1,"))));
    // In three times: two to a row, one, then as wide as the window, which runs past its bottom.
    view.press("+++");
    view.wait_until("the page as wide as the window", |t| bar(t).contains(" page 1/6 · width ") && t.contains(",C=1,c=80,r=47,"));
    view.press("+");
    view.wait_until("no further in", |t| t.contains("fully zoomed in"));
    view.press("q");
    assert!(view.finish().success());
}

/// `,` and `.` set pages to a row as `-` and `+` zoom: one more, one fewer, then the width; a count
/// sets that many, as far as the window takes, and `0.` goes back to `auto`. In a list they set its
/// tiles to a row, `.` stopping at one.
#[test]
fn comma_and_dot_set_pages_and_tiles_to_a_row() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("per-row", 6);
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 80, 48);
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/6") && bar(t).contains(" page 1/6 · 1 per row "));
    let since = |view: &mut Session, keys: &str, what: &str, done: &dyn Fn(&str) -> bool| {
        let before = view.text().len();
        view.press(keys);
        view.wait_until(what, |t| done(&t[before..]));
    };
    since(&mut view, ",,", "three to a row", &|t| {
        bar(t).contains(" pages 1–6/6 · 3 per row ") && (1..=6).all(|p| t.contains(&format!(",p={p},C=1,")))
    });
    since(&mut view, "...", "the width", &|t| bar(t).contains(" page 1/6 · width ") && t.contains(",C=1,c=80,r=47,"));
    since(&mut view, ".", "no further in", &|t| t.contains("fully zoomed in"));
    since(&mut view, "2.", "two to a row", &|t| bar(t).contains(" pages 1–2/6 · 2 per row "));
    since(&mut view, "0.", "auto: one, in a tall window", &|t| bar(t).contains(" page 1/6 · 1 per row "));
    view.press("q");
    assert!(view.finish().success());
    // A window 28 columns wide takes three pages of 8 columns to a row, however many are asked for:
    // two rows of three, never four of 4 columns.
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 28, 30);
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/6"));
    since(&mut view, "8,", "three to a row of 8 columns", &|t| (1..=6).all(|p| t.contains(&format!(",p={p},C=1,c=8,"))) && !t.contains(",C=1,c=5,"));
    view.press("q");
    assert!(view.finish().success());
    // A list of three: three tiles to a row, then fewer, down to one and no further; then more.
    let dir = std::env::temp_dir().join(format!("clview-pager-per-row-list-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let files: Vec<PathBuf> = ["a", "b", "c"].iter().map(|name| dir.join(format!("{name}.pdf"))).collect();
    for file in &files {
        std::fs::write(file, crate::fixtures::letter("Tile", 1)).unwrap();
    }
    let mut view = Session::start(&files[0], &[files[1].to_str().unwrap(), files[2].to_str().unwrap(), "--protocol", "kitty"], 100, 40);
    view.wait_until("the list", |t| bar(t).contains(" 1/3 · 3 per row ") && drawn(t, "1/3"));
    since(&mut view, "..", "one tile to a row", &|t| bar(t).contains(" 1/3 · 1 per row "));
    since(&mut view, ".", "no further in", &|t| t.contains("fully zoomed in") && !t.contains("width"));
    since(&mut view, ",", "two to a row", &|t| bar(t).contains(" 1/3 · 2 per row "));
    since(&mut view, "0.", "auto", &|t| bar(t).contains(" 1/3 · 3 per row "));
    view.press("q");
    assert!(view.finish().success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_next_screen_of_a_grid_is_in_the_terminal_before_the_key() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("grid", 24);
    // Three to a row in two rows: six pages a screen. The screen and the two after it are 18
    // pages, more than the pager keeps for one page to a screen, and all of them stay.
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--per-row", "3"], 80, 48);
    view.wait_until("the screen and two ahead sent", |t| bar(t).contains(" pages 1–6/24 · 3 per row ") && transmitted(t) == 18);
    std::thread::sleep(Duration::from_millis(200));
    let before = view.text().len();
    view.press(" ");
    view.wait_until("pages 7–12 placed", |t| t.contains(" pages 7–12/24 ") && t[before..].contains("a=p,q=1,"));
    // Turning to them only placed them: none was sent again first.
    let after = view.text().split_off(before);
    let placed = after.find("a=p,q=1,").unwrap_or(0);
    assert!(after.find("a=t,").is_none_or(|sent| sent > placed), "a page was sent again before the screen showed: {after:?}");
    view.press("q");
    assert!(view.finish().success());
}

#[test]
fn a_document_shorter_than_the_screen_sits_in_its_middle() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("short", 3);
    // Three to a row: one row of pages 17 rows tall, where two would fit. It is in the middle of
    // the 47 rows above the status line, from row 16, not where the first of two rows would be.
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--per-row", "3"], 80, 48);
    view.wait_until("three pages placed", |t| bar(t).contains(" pages 1–3/3 · 3 per row ") && t.contains(",p=3,C=1,"));
    assert!(view.text().contains("\x1b[16;1H\x1b_Ga=p,q=1,"), "{:?}", view.text());
    view.press("q");
    assert!(view.finish().success());
}

#[test]
fn a_page_given_twice_shows_twice_side_by_side() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("twice", 3);
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--pages", "1,1"], 120, 24);
    view.wait_until("page 1 twice", |t| drawn(t, "pages 1–1/3"));
    // One image, placed twice: each place on screen a placement of its own.
    let text = view.text();
    assert_eq!(transmitted(&text), 1, "{text:?}");
    let id = view_id(&text[text.find("a=t,f=100,t=d,q=2,i=").unwrap()..]);
    for placement in [1, 2] {
        assert!(text.contains(&format!("a=p,q=1,i={id},p={placement},")), "placement {placement}: {text:?}");
    }
    view.press("q");
    assert!(view.finish().success());
}

#[test]
fn an_image_the_terminal_dropped_is_sent_again_and_its_answers_are_never_keys() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("dropped", 3);
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 80, 60);
    view.wait_until("pages 1-3 sent", |t| drawn(t, "page 1/3") && transmitted(t) == 3);
    let text = view.text();
    let at = text.find("a=p,q=1,i=").unwrap();
    let id: String = text[at + "a=p,q=1,i=".len()..].chars().take_while(char::is_ascii_digit).collect();
    // Answers that say nothing: an OK, another error, one cut in two. Not one of them is a key:
    // their ESC isn't Esc (which quits), nor their letters keys.
    view.press(&format!("\x1b_Gi={id};OK\x1b\\\x1b_Gi={id};EINVAL:quit\x1b\\\x1b_Gi={id};O"));
    std::thread::sleep(Duration::from_millis(100));
    view.press("K\x1b\\");
    std::thread::sleep(Duration::from_millis(300));
    assert!(view.child.try_wait().unwrap().is_none(), "an answer quit the pager");
    assert_eq!(transmitted(&view.text()), 3, "an answer that says nothing sent a page");
    // kitty dropped page 1 to stay under its quota, and answers its placement so: it is sent
    // again, and placed.
    view.press(&format!("\x1b_Gi={id},p=1;ENOENT:Put command refers to non-existent image with id: {id} and number: 0\x1b\\"));
    view.wait_until("page 1 sent again", |t| transmitted(t) == 4);
    let again = view.text();
    let new = view_id(&again[again.rfind("a=t,f=100,t=d,q=2,i=").unwrap()..]);
    assert_ne!(new, id);
    view.wait_until("page 1 placed again", |t| t.contains(&format!("a=p,q=1,i={new},p=1,")));
    view.press("q");
    assert!(view.finish().success());
}

#[test]
fn a_reply_cut_in_two_is_never_keys_and_esc_still_quits() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("cut", 3);
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 80, 60);
    // Settled: page 1 shown, and every page sent.
    view.wait_until("pages 1-3 sent", |t| drawn(t, "page 1/3") && transmitted(t) == 3);
    // A late cell size and device attributes, as over a slow connection: each cut past its CSI
    // prefix, the rest coming later than an Esc waits for more. Neither part is a key: the ESC
    // isn't Esc (which quits), nor are the digits a page number.
    for (first, rest) in [("\x1b[6;", "25;14t"), ("\x1b[", "?62;c")] {
        view.press(first);
        std::thread::sleep(Duration::from_millis(250));
        assert!(view.child.try_wait().unwrap().is_none(), "{first:?} quit the pager");
        view.press(rest);
        std::thread::sleep(Duration::from_millis(250));
        assert!(view.child.try_wait().unwrap().is_none(), "{first:?} then {rest:?} quit the pager");
    }
    assert!(!view.text().contains("go to page"), "a reply typed a page number");
    // A key cut in two the same way is the key: Page Down.
    view.press("\x1b[6");
    std::thread::sleep(Duration::from_millis(250));
    view.press("~");
    view.wait_until("page 2 shown", |t| t.contains(" page 2/3 "));
    // Esc on its own still quits.
    view.press("\x1b");
    assert!(view.finish().success());
    assert!(view.text().contains("\x1b[?1049l"), "the alternate screen was left");
}

/// An image pages on threads the viewer lends it, as a PDF does on its backend's: shown without a
/// key at its own size, larger only as wide as the window, and rendered again at each new scale.
#[cfg(feature = "image")]
#[test]
fn an_image_opens_in_the_pager_and_zooms() {
    if !pty_works() {
        return;
    }
    let png = picture("pager-image", 400, 100);
    // 600 × 580 px above the status line, which would take the picture 1.5 times as wide.
    let mut view = Session::start(&png, &["--pager", "--protocol", "kitty"], 60, 30);
    view.wait_until("the image shown", |t| t.contains(" page 1/1 ") && transmitted(t) == 1 && placed_after_sent(t));
    assert_eq!(first_page_size(&view.text()), ((400, 100), (40, 5)));
    view.press("+");
    view.wait_until("the image as wide as the window", |t| bar(t).contains(" page 1/1 · width ") && transmitted(t) == 2 && placed_after_sent(t));
    // Back to its own size, which is still in the terminal, then two to a row: a smaller one.
    view.press("--");
    view.wait_until("the image zoomed out", |t| bar(t).contains(" page 1/1 · 2 per row ") && transmitted(t) == 3 && placed_after_sent(t));
    assert_eq!(sent_sizes(&view.text()), [(400, 100), (600, 150), (290, 73)]);
    view.press("q");
    assert!(view.finish().success());
}

/// Fewer pages than a row takes are a row of their own, in the middle of the screen.
#[cfg(feature = "image")]
#[test]
fn one_image_zoomed_out_sits_in_the_middle() {
    if !pty_works() {
        return;
    }
    let png = picture("pager-middle", 800, 600);
    // Four to a row in 80 columns: an 18-column place, 7 rows tall for this picture, in the middle
    // of the 47 rows above the status line and of the 80 columns, from row 21 and column 32.
    let mut view = Session::start(&png, &["--pager", "--protocol", "kitty", "--per-row", "4"], 80, 48);
    view.wait_until("the image placed", |t| bar(t).contains(" page 1/1 · 4 per row ") && t.contains(",p=1,C=1,"));
    assert_eq!(first_page_size(&view.text()), ((180, 135), (18, 7)));
    assert!(view.text().contains("\x1b[21;32H\x1b_Ga=p,q=1,"), "{:?}", view.text());
    view.press("q");
    assert!(view.finish().success());
}

/// Whether an image was placed after the last one sent: the draw after it is written out whole.
#[cfg(feature = "image")]
fn placed_after_sent(text: &str) -> bool {
    text.rfind("a=t,f=100,t=d,q=2,i=").is_some_and(|at| text[at..].contains("a=p,q=1,"))
}

/// A `width` × `height` PNG made by the test.
#[cfg(feature = "image")]
fn picture(test: &str, width: u32, height: u32) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("clview-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("picture.png");
    save(&png, width, height);
    png
}

/// A `width` × `height` PNG at `path`.
#[cfg(feature = "image")]
fn save(path: &Path, width: u32, height: u32) {
    let rgba: Vec<u8> = (0..width * height).flat_map(|i| [(i % 251) as u8, 128, 64, 255]).collect();
    image::save_buffer(path, &rgba, width, height, image::ExtendedColorType::Rgba8).unwrap();
}

/// The size in pixels of every image sent, in order, from its PNG header.
#[cfg(feature = "image")]
fn sent_sizes(text: &str) -> Vec<(u32, u32)> {
    use base64::Engine as _;
    let sent = text.match_indices("a=t,f=100,t=d,q=2,i=").map(|(at, _)| &text[at..]);
    sent.map(|t| {
        let payload = &t[t.find(';').unwrap() + 1..];
        let header = base64::engine::general_purpose::STANDARD.decode(&payload[..44]).unwrap();
        let be = |at: usize| u32::from_be_bytes(header[at..at + 4].try_into().unwrap());
        (be(16), be(20))
    })
    .collect()
}

/// The name of the tile chosen last, highlighted under it: the last text in reverse video that
/// isn't the status line.
pub(crate) fn chosen(text: &str) -> Option<String> {
    let labels = text.split("\x1b[7m").skip(1).filter_map(|t| t.split_once("\x1b[0m").map(|(label, _)| label));
    labels.filter(|label| !label.contains("? help")).last().map(|label| label.trim().to_string())
}

/// Several files are a list, a tile each: the arrows choose one, its name highlighted under it and
/// on the status line, Enter opens its pages, and Esc goes back to the list as it was left, its
/// tiles still in the terminal. A file left out is said first. `--pages` picks tiles, and a tile
/// opens to all its file's pages.
#[test]
fn several_files_are_a_list_and_enter_opens_one() {
    if !pty_works() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("clview-pager-several-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b, notes) = (dir.join("a.pdf"), dir.join("b.pdf"), dir.join("notes.txt"));
    std::fs::write(&a, crate::fixtures::letter("First", 1)).unwrap();
    std::fs::write(&b, crate::fixtures::letter("Second", 2)).unwrap();
    std::fs::write(&notes, "plain text\n").unwrap();
    // A tall window: a page to the screen, once opened.
    let mut view = Session::start(&a, &[b.to_str().unwrap(), notes.to_str().unwrap(), "--protocol", "kitty"], 100, 60);
    view.wait_until("the list", |t| bar(t).contains(" a.pdf 1 file left out: notes.txt 1/2 · 2 per row ") && drawn(t, "1/2"));
    assert_eq!(chosen(&view.text()).as_deref(), Some("a.pdf"));
    assert_eq!(transmitted(&view.text()), 2, "a tile each");
    view.press("\x1b[C");
    view.wait_until("b chosen", |t| bar(t).contains(" b.pdf 2/2 · 2 per row ") && chosen(t).as_deref() == Some("b.pdf"));
    view.press("\r");
    view.wait_until("b opened", |t| bar(t).contains(" b.pdf esc back · space b · n p · + - zoom · q quit") && drawn(t, "page 1/2"));
    view.press("n");
    view.wait_until("b's second page", |t| drawn(t, "page 2/2"));
    std::thread::sleep(Duration::from_millis(200));
    let sent = transmitted(&view.text());
    let before = view.text().len();
    view.press("\x1b");
    view.wait_until("back to the list", |t| t.get(before..).is_some_and(|t| bar(t).contains(" b.pdf 2/2 · 2 per row ")));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(transmitted(&view.text()), sent, "a tile was sent again");
    // Backspace has nothing to go back to here; q quits.
    view.press("\x7f");
    std::thread::sleep(Duration::from_millis(200));
    assert!(view.child.try_wait().unwrap().is_none(), "Backspace quit");
    view.press("q");
    assert!(view.finish().success());
    // The second tile alone, still numbered as the list's, paged since it can be opened.
    let mut view = Session::start(&a, &[b.to_str().unwrap(), "--protocol", "kitty", "--pages", "2", "--pager"], 100, 60);
    view.wait_until("b's tile alone", |t| drawn(t, "2/2 · 1 of 1"));
    // A key, so that the status line names the tile chosen in place of the keys.
    view.press("\x1b[C");
    view.wait_until("b named", |t| bar(t).contains(" b.pdf 2/2 · 1 of 1 · 1 per row "));
    view.press("\r");
    view.wait_until("all of b", |t| drawn(t, "page 1/2"));
    view.press("n");
    view.wait_until("b's second page", |t| drawn(t, "page 2/2"));
    view.press("q");
    assert!(view.finish().success());
}

/// A directory's subdirectories are folders to go into: Enter lists one, its files named from
/// the first directory on the status line, and Backspace or `u` goes back; `u` at the top, where
/// there is nothing to go back to, doesn't quit. Recursing, their files are tiles instead.
#[cfg(feature = "image")]
#[test]
fn a_directorys_subdirectories_are_folders_to_go_into() {
    if !pty_works() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("clview-pager-gallery-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("trip")).unwrap();
    std::fs::create_dir_all(dir.join("empty")).unwrap();
    std::fs::write(dir.join("empty/notes.txt"), "plain text\n").unwrap();
    save(&dir.join("a.png"), 800, 600);
    save(&dir.join("trip/b.png"), 800, 600);
    save(&dir.join("trip/c.png"), 400, 300);
    let mut view = Session::start(&dir, &["--protocol", "kitty"], 80, 48);
    view.wait_until("the list", |t| bar(t).contains(" a.png arrows move · ⏎ open · + - zoom · q quit 1/3 "));
    // A folder with nothing to show says so, and stays.
    view.press("l\r");
    view.wait_until("empty", |t| bar(t).contains(" empty/ nothing in empty/ that clv shows 2/3 "));
    view.press("l\r");
    view.wait_until("in trip", |t| bar(t).contains(" trip/b.png 1/2 ") && chosen(t).as_deref() == Some("b.png"));
    assert!(bar(&view.text()).contains(" trip/ opening trip/… 3/3 "), "said before opening");
    view.press("l");
    view.wait_until("c chosen", |t| bar(t).contains(" trip/c.png 2/2 "));
    view.press("\x7f");
    view.wait_until("back", |t| bar(t).contains(" trip/ 3/3 ") && chosen(t).as_deref() == Some("trip/"));
    view.press("\r");
    view.wait_until("in trip again", |t| bar(t).contains(" trip/b.png 1/2 "));
    view.press("u");
    view.wait_until("up", |t| bar(t).contains(" trip/ 3/3 ") && chosen(t).as_deref() == Some("trip/"));
    view.press("ug");
    view.wait_until("still there, at the first", |t| bar(t).contains(" a.png ") && bar(t).contains(" 1/3 "));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(dir.join("empty")).unwrap();
    let mut flat = Session::start(&dir, &["-r", "--protocol", "kitty"], 80, 48);
    flat.wait_until("the list", |t| bar(t).contains(" a.png ") && bar(t).contains(" 1/3 "));
    flat.press("G");
    flat.wait_until("the last", |t| bar(t).contains(" trip/c.png 3/3 "));
    flat.press("q");
    assert!(flat.finish().success());
}

/// What the last draw wrote: from its synchronized output on.
fn last_draw(text: &str) -> &str {
    text.rfind("\x1b[?2026h").map_or(text, |at| &text[at..])
}

/// `/` filters a list by name as you type, in upper or lower case: the tiles that match go on
/// screen, keeping their size, so none is sent again. Every key that types is text meanwhile,
/// `q` too, and a character past ASCII is one key, even cut across reads. Enter keeps the filter,
/// and the keys move among the matches; Esc clears it, the tile chosen still chosen, or while
/// typing puts the list back as it was; then Esc quits.
#[test]
fn a_list_filters_by_name_as_you_type() {
    if !pty_works() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("clview-pager-filter-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let names = ["alpha.pdf", "beta.pdf", "Alphabet.pdf", "café.pdf"];
    let paths: Vec<PathBuf> = names.iter().map(|name| dir.join(name)).collect();
    for (path, name) in paths.iter().zip(names) {
        std::fs::write(path, crate::fixtures::letter(name, 1)).unwrap();
    }
    let rest: Vec<&str> = paths[1..].iter().map(|p| p.to_str().unwrap()).chain(["--protocol", "kitty"]).collect();
    let mut view = Session::start(&paths[0], &rest, 100, 60);
    view.wait_until("the list", |t| drawn(t, "1/4") && transmitted(t) == 4);
    view.press("/ALP");
    view.wait_until("the filter", |t| bar(t).contains(" alpha.pdf /ALP ⏎ keep · esc cancel 1/4 · 1 of 2 · 4 per row "));
    // Only the two that match are placed, and their names under them.
    let last = last_draw(&view.text()).to_owned();
    assert_eq!(last.matches("a=p,q=1,").count(), 2, "{last:?}");
    assert!(last.contains("Alphabet.pdf") && !last.contains("beta.pdf"), "{last:?}");
    view.press("q");
    view.wait_until("no match", |t| bar(t).contains(" /ALPq ⏎ keep · esc cancel no match · 4 per row "));
    assert!(view.child.try_wait().unwrap().is_none(), "q quit while typing");
    view.press("\x7f\r");
    view.wait_until("kept", |t| bar(t).contains(" alpha.pdf /ALP 1/4 · 1 of 2 · "));
    view.press("\x1b[C");
    view.wait_until("the second match", |t| bar(t).contains(" Alphabet.pdf /ALP 3/4 · 2 of 2 ") && chosen(t).as_deref() == Some("Alphabet.pdf"));
    view.press("\x1b");
    view.wait_until("all of them again", |t| bar(t).contains(" Alphabet.pdf 3/4 · 4 per row ") && chosen(t).as_deref() == Some("Alphabet.pdf"));
    // É cut in two, its second byte later than an Esc waits for.
    view.press("/CAF");
    view.wait_until("typing again", |t| bar(t).contains(" /CAF ⏎ keep"));
    view.press_bytes(b"\xc3");
    std::thread::sleep(Duration::from_millis(200));
    view.press_bytes(b"\x89");
    view.wait_until("café", |t| bar(t).contains(" café.pdf /CAFÉ ⏎ keep · esc cancel 4/4 · 1 of 1 "));
    let before = view.text().len();
    view.press("\x1b");
    view.wait_until("as it was", |t| t.get(before..).is_some_and(|t| bar(t).contains(" Alphabet.pdf 3/4 · 4 per row ")));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(transmitted(&view.text()), 4, "a tile was sent again");
    view.press("\x1b");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The `i=` value at the start of a transmission.
fn view_id(transmission: &str) -> String {
    transmission.trim_start_matches("a=t,f=100,t=d,q=2,i=").chars().take_while(char::is_ascii_digit).collect()
}

/// The first page placed: the cells it covers, and its size in pixels, from its PNG header. (The
/// first page sent may be one rendered ahead, never placed yet.)
fn first_page_size(text: &str) -> ((u32, u32), (u32, u32)) {
    use base64::Engine as _;
    let placed = &text[text.find("a=p,q=1,i=").unwrap()..];
    let control = placed.split(['\x1b', ';']).next().unwrap();
    let key = |k: &str| control.split(',').find_map(|kv| kv.strip_prefix(k)).unwrap().parse::<u32>().unwrap();
    let sent = &text[text.find(&format!("a=t,f=100,t=d,q=2,i={},", key("i="))).unwrap()..];
    let payload = &sent[sent.find(';').unwrap() + 1..];
    let header = base64::engine::general_purpose::STANDARD.decode(&payload[..44]).unwrap();
    let be = |at: usize| u32::from_be_bytes(header[at..at + 4].try_into().unwrap());
    ((be(16), be(20)), (key("c="), key("r=")))
}

#[test]
fn a_terminal_without_pixels_is_asked_its_cell_size() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("cells", 3);
    // As under `docker exec`: the pseudo-terminal has no pixel size, and the terminal knows its
    // cells are 16 × 24 px. One that doesn't know gets the 10 × 20 px assumed.
    for (terminal, (cell_w, cell_h)) in [(Terminal { cell: Some((16, 24)), ..PLAIN }, (16, 24)), (PLAIN, (10, 20))] {
        let mut view = Session::start_on(terminal, &pdf, &["--protocol", "kitty"], 80, 60);
        view.wait_until("page 1 shown", |t| drawn(t, "page 1/3"));
        let text = view.text();
        assert_eq!(text.matches("\x1b[16t").count(), 1, "asked once");
        // Rendered for those cells: the page fills the last of the cells it covers.
        let ((w, h), (cols, rows)) = first_page_size(&text);
        assert!(w <= cols * cell_w + 1 && w + 1 > (cols - 1) * cell_w, "{w} px wide over {cols} cells {cell_w} px wide");
        assert!(h <= rows * cell_h + 1 && h + 1 > (rows - 1) * cell_h, "{h} px tall over {rows} cells {cell_h} px tall");
        view.press("q");
        assert!(view.finish().success());
    }
}

#[test]
fn the_first_key_after_the_cell_size_question_reaches_the_pager() {
    if !pty_works() {
        return;
    }
    // On FreeBSD a read begun in canonical mode stays a line read after raw mode returns. Were
    // the terminal back in canonical mode between the question and the pager, a reader whose next
    // read began in that gap would hold every key until Enter: `q` would never quit. Whether a
    // read begins in the gap is a race, which many sessions at once, each asked its cell size
    // (no pixel size here), make likely to show.
    let pdf = fixture("first-key", 2);
    let sessions: Vec<Session> = (0..16).map(|_| Session::start(&pdf, &["--protocol", "kitty"], 80, 60)).collect();
    for mut view in sessions {
        view.wait_until("page 1 shown", |t| drawn(t, "page 1/2"));
        assert_eq!(view.text().matches("\x1b[16t").count(), 1, "asked its cell size");
        view.press("q");
        assert!(view.finish().success());
    }
}

#[test]
fn one_page_is_printed_without_the_pager() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("one", 1);
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 80, 24);
    assert!(view.finish().success());
    let text = view.text();
    assert_eq!(printed(&text), 1, "{text:?}");
    assert!(!text.contains("\x1b[?1049h"), "no alternate screen");
}

#[test]
fn no_pager_prints_every_page() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("print", 3);
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--no-pager"], 80, 24);
    assert!(view.finish().success());
    let text = view.text();
    assert_eq!(printed(&text), 3, "{text:?}");
    assert!(!text.contains("\x1b[?1049h"), "no alternate screen");
}

#[test]
fn printing_follows_the_zoom_in_rows_of_pages() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("print-zoom", 4);
    // Three to a row at 80 × 24: pages 25 columns wide with 2 between, and 17 rows tall.
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--no-pager", "--per-row", "3"], 80, 24);
    assert!(view.finish().success());
    let text = view.text();
    assert_eq!(printed(&text), 4, "{text:?}");
    // Each row: lines made for it, back up to its top, each page at its place with the cursor
    // left there, then down past the row. Pages 1–3, then page 4 a row below.
    let page = "\x1b_Ga=T,f=100,t=d,q=2,C=1,c=25,r=17,";
    for (before, times) in [("\x1b[17A\r", 2), ("\r\x1b[27C", 1), ("\r\x1b[54C", 1)] {
        assert_eq!(text.matches(&format!("{before}{page}")).count(), times, "{before:?}: {text:?}");
    }
    assert_eq!(text.matches("\x1b[17B\r").count(), 2, "{text:?}");
    // Auto at 80 × 24 prints two to a row, as the pager shows them.
    let mut auto = Session::start(&pdf, &["--protocol", "kitty", "--no-pager"], 80, 24);
    assert!(auto.finish().success());
    let text = auto.text();
    assert_eq!(text.matches("\r\x1b[38C\x1b_Ga=T,f=100,t=d,q=2,C=1,c=36,r=23,").count(), 2, "{text:?}");
}

#[test]
fn pager_shows_even_one_page() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("forced", 1);
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--pager"], 80, 60);
    view.wait_until("page 1 shown", |t| t.contains(" page 1/1 ") && transmitted(t) == 1);
    view.press("q");
    assert!(view.finish().success());
}

/// A directory of the test's own, empty, for `clv`'s temporary files. Owner-only whatever the
/// umask: `clv` rightly refuses a parent that others can write to without the sticky bit, which
/// a umask of 0002 (common on Linux) would make it.
fn tmpdir(test: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = std::env::temp_dir().join(format!("clview-tmp-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

/// The files `clv` left in the temporary directory `dir`: in its own directory there.
fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(files_in(&path));
        } else {
            files.push(path);
        }
    }
    files
}

#[test]
fn a_terminal_that_takes_files_gets_its_pages_through_them() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("files", 3);
    let dir = tmpdir("files");
    let terminal = Terminal { takes_files: true, tmpdir: Some(&dir), ..PLAIN };
    let mut view = Session::start_on(terminal, &pdf, &["--protocol", "kitty"], 80, 60);
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/3"));
    // The probe first: one pixel, its image deleted straight after.
    view.wait_until("the probe taken", |t| t.contains("a=t,f=32,t=t,s=1,v=1,q=2,i="));
    assert!(view.taken.lock().unwrap().first().is_some_and(Result::is_ok), "{:?}", view.taken.lock().unwrap());
    // r sends every page again, long after the probe: each through a file, none as a PNG.
    std::thread::sleep(Duration::from_millis(300));
    let before = view.text().len();
    view.press("r");
    let after = |t: &str| t.get(before..).unwrap_or_default().to_string();
    view.wait_until("pages 1-3 sent again", |t| after(t).matches("a=t,f=32,t=t,").count() == 3);
    std::thread::sleep(Duration::from_millis(200));
    let text = view.text();
    assert_eq!(transmitted(&after(&text)), 0, "a PNG after the probe was taken");
    let taken = view.taken.lock().unwrap().clone();
    assert!(taken.iter().all(Result::is_ok), "{taken:?}");
    assert!(taken.len() >= 4, "{taken:?}");
    for path in taken.iter().flatten() {
        assert!(path.starts_with(&dir) && path.to_string_lossy().contains("tty-graphics-protocol"), "{path:?}");
    }
    view.press("q");
    assert!(view.finish().success());
    assert!(files_in(&dir).is_empty(), "left behind: {:?}", files_in(&dir));
}

#[test]
fn a_terminal_that_leaves_files_gets_pngs_and_no_file_stays() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("nofiles", 3);
    let dir = tmpdir("nofiles");
    let terminal = Terminal { tmpdir: Some(&dir), ..PLAIN };
    let mut view = Session::start_on(terminal, &pdf, &["--protocol", "kitty"], 80, 60);
    view.wait_until("pages 1-3 sent as PNGs", |t| t.contains(" page 1/3 ") && transmitted(t) == 3);
    let text = view.text();
    assert_eq!(text.matches("a=t,f=32,t=t,").count(), 1, "only the probe went through a file");
    // The probe waits for the terminal a while, and goes after that or when the pager ends.
    view.press("q");
    assert!(view.finish().success());
    assert!(files_in(&dir).is_empty(), "left behind: {:?}", files_in(&dir));
}

#[test]
fn compression_by_name_sends_pngs_without_trying_files() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("named", 3);
    let dir = tmpdir("named");
    let terminal = Terminal { takes_files: true, tmpdir: Some(&dir), ..PLAIN };
    let mut view = Session::start_on(terminal, &pdf, &["--protocol", "kitty", "--compression", "fast"], 80, 60);
    view.wait_until("pages 1-3 sent as PNGs", |t| t.contains(" page 1/3 ") && transmitted(t) == 3);
    view.press("q");
    assert!(view.finish().success());
    assert!(!view.text().contains("t=t"), "no file, not even the probe");
}

/// The PNG clipboard write `id` carried: its `image/png` packets' data, put together.
fn copied_png(text: &str, id: u32) -> Vec<u8> {
    use base64::Engine as _;
    let start = format!("\x1b]5522;type=wdata:mime={}:id=clv-{id};", base64::engine::general_purpose::STANDARD.encode("image/png"));
    let mut png = Vec::new();
    for (at, _) in text.match_indices(&start) {
        let data = &text[at + start.len()..];
        // `wait_until` asks of what has come so far, which may end inside a packet.
        let Some(end) = data.find("\x1b\\") else { break };
        png.extend(base64::engine::general_purpose::STANDARD.decode(&data[..end]).unwrap());
    }
    png
}

/// A PNG's width and height, from its header.
fn png_size(png: &[u8]) -> (u32, u32) {
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "not a PNG: {} bytes", png.len());
    let be = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
    (be(16), be(20))
}

/// `c` copies the page the reader is on to a terminal that speaks the clipboard protocol: it is
/// asked once whether it does, and the page goes as a PNG of US Letter at 144 per inch. Its
/// answers are never keys: as keys, their ESC would quit.
#[test]
fn c_copies_the_page_to_the_terminals_clipboard() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("copy", 3);
    let terminal = Terminal { clipboard: Some(true), ..PLAIN };
    let mut view = Session::start_on(terminal, &pdf, &["--protocol", "kitty"], 100, 60);
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/3"));
    view.press("nc");
    view.wait_until("page 2 copied", |t| t.contains("copied page 2, 1224 × 1584 pixels"));
    let text = view.text();
    assert_eq!(text.matches(std::str::from_utf8(CLIPBOARD_QUERY).unwrap()).count(), 1, "asked once");
    assert_eq!(png_size(&copied_png(&text, 1)), (1224, 1584));
    // Again, page 3: not asked again.
    view.press("nc");
    view.wait_until("page 3 copied", |t| t.contains("copied page 3, 1224 × 1584 pixels"));
    assert_eq!(view.text().matches(std::str::from_utf8(CLIPBOARD_QUERY).unwrap()).count(), 1, "asked again");
    assert_eq!(png_size(&copied_png(&view.text(), 2)), (1224, 1584));
    assert!(view.child.try_wait().unwrap().is_none(), "an answer quit the pager");
    view.press("q");
    assert!(view.finish().success());
}

/// A terminal that doesn't speak the clipboard protocol, over SSH, has no clipboard for a
/// picture: the reader is told, and nothing is sent. (On this machine, the system's clipboard
/// would take it, which a test mustn't touch.)
#[test]
fn over_ssh_a_terminal_without_the_clipboard_protocol_gets_no_copy() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("nocopy", 3);
    let terminal = Terminal { clipboard: Some(false), ssh: true, ..PLAIN };
    // Wide, for the whole of what it says: two pages side by side.
    let mut view = Session::start_on(terminal, &pdf, &["--protocol", "kitty"], 200, 60);
    view.wait_until("pages 1-2 shown", |t| bar(t).contains(" pages 1–2/3 "));
    view.press("c");
    view.wait_until("told", |t| t.contains("can't copy page 1: this terminal has no clipboard for pictures, and the system's isn't yours over SSH"));
    assert!(!view.text().contains("\x1b]5522;"), "a write went out");
    view.press("q");
    assert!(view.finish().success());
}

/// In a list, `c` copies the tile chosen, a file's first page; a folder has no picture.
#[test]
fn in_a_list_c_copies_the_tile_chosen_and_a_folder_has_none() {
    if !pty_works() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("clview-pager-copy-list-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("more")).unwrap();
    std::fs::write(dir.join("a.pdf"), crate::fixtures::letter("First", 2)).unwrap();
    std::fs::write(dir.join("more/b.pdf"), crate::fixtures::letter("Second", 1)).unwrap();
    let terminal = Terminal { clipboard: Some(true), ..PLAIN };
    let mut view = Session::start_on(terminal, &dir, &["--protocol", "kitty"], 100, 48);
    view.wait_until("the list", |t| bar(t).contains(" a.pdf ") && drawn(t, "1/2"));
    view.press("lc");
    view.wait_until("told", |t| t.contains("more/ is a folder: no picture to copy"));
    view.press("hc");
    view.wait_until("a copied", |t| t.contains("copied a.pdf, 1224 × 1584 pixels"));
    assert_eq!(png_size(&copied_png(&view.text(), 1)), (1224, 1584));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `clv --help` on a `cols` × `rows` pseudo-terminal of type `term`, as it arrives there with the
/// newlines put back: the pseudo-terminal writes each as `\r\n`.
fn help_on(term: &str, cols: u16, rows: u16) -> String {
    let terminal = Terminal { term: Some(term), ..PLAIN };
    let mut help = Session::start_on(terminal, Path::new("--help"), &[], cols, rows);
    assert!(help.finish().success());
    help.text().replace("\r\n", "\n")
}

/// `clv --help` through a pipe: the text alone.
fn help_text() -> String {
    String::from_utf8(Command::new(BIN).arg("--help").output().unwrap().stdout).unwrap()
}

/// Where the icon can show, it goes up beside the first lines, nothing is asked of the terminal,
/// and the cursor comes back for the rest of the text, which is the same.
#[test]
fn help_shows_the_icon_beside_its_first_lines_where_the_terminal_is_known_to_show_it() {
    if !pty_works() {
        return;
    }
    let plain = help_text();
    let (end, _) = plain.match_indices('\n').nth(4).unwrap();
    let (top, rest) = plain.split_at(end + 1);
    for term in ["xterm-kitty", "xterm-ghostty"] {
        let shown = help_on(term, 120, 40);
        // 10 × 20 cells, as no pixel size is reported: 5 rows are 10 columns, ending at column 100.
        let up = format!("{top}\x1b[5A\r\x1b[90C\x1b_Ga=T,f=100,t=d,q=2,C=1,c=10,r=5,");
        assert!(shown.starts_with(&up), "{term}: {:?}", &shown[..up.len().min(shown.len())]);
        assert!(shown.ends_with(&format!("\x1b\\\x1b[5B\r{rest}")), "{term}: {:?}", &shown[shown.len().saturating_sub(rest.len() + 20)..]);
        assert_eq!(printed(&shown), 1, "{term}");
        assert!(!shown.contains("\x1b[c") && !shown.contains("a=q"), "{term}: the terminal was asked something");
    }
}

/// Where the icon can't show, the help is the same text, and the terminal is asked nothing.
#[test]
fn help_is_the_same_text_where_the_icon_cant_show() {
    if !pty_works() {
        return;
    }
    let plain = help_text();
    assert_eq!(help_on("xterm-256color", 120, 40), plain, "a terminal not known to show it");
    assert_eq!(help_on("xterm-kitty", 70, 40), plain, "no room beside the lines");
    assert_eq!(help_on("xterm-kitty", 120, 3), plain, "too few rows");
}

/// Through a pipe, on a terminal that shows images, `--help` is the text alone: what reads it isn't
/// the terminal. Run on a pseudo-terminal, since without one there is no window to fit an icon to,
/// and nothing would be left for this to catch.
#[test]
fn help_through_a_pipe_has_no_escape_codes_on_a_terminal_that_shows_images() {
    if !pty_works() {
        return;
    }
    let mut child = script(&format!("stty rows 40 cols 120 && '{BIN}' --help | cat"))
        .env("TERM", "xterm-kitty")
        .env("KITTY_WINDOW_ID", "1")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut out = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut out).unwrap();
    child.wait().unwrap();
    assert_eq!(String::from_utf8(out).unwrap().replace("\r\n", "\n"), help_text());
}

/// What a stream shows, once its escape sequences are taken out: control sequences, and the string
/// ones (the operating system's and the graphics protocol's), which end with ST.
fn visible(text: &str) -> String {
    let (mut out, mut chars) = (String::new(), text.chars());
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                chars.by_ref().find(|c| ('@'..='~').contains(c));
            }
            Some(']' | '_') => {
                chars.by_ref().find(|c| *c == '\x1b');
                chars.next();
            }
            _ => {}
        }
    }
    out
}

/// Whether `text` sets a colour for a cell: only page rows do, not the status line.
fn has_cells(text: &str) -> bool {
    text.contains("\x1b[48;") || text.contains("\x1b[38;")
}

/// A terminal that doesn't answer the graphics query gets the pages all the same, in text cells and
/// with nothing sent for images, and the status line says why. `q` puts the screen back.
#[test]
fn a_terminal_without_the_protocol_gets_text_cells_and_is_told() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("no-protocol", 3);
    let mut view = Session::start_on(TEXT_ONLY, &pdf, &[], 80, 40);
    view.wait_until("told", |t| t.contains("no graphics protocol: drawing text cells"));
    view.wait_until("page 1 drawn", |t| drawn(t, "page 1/3") && has_cells(t));
    let text = view.text();
    assert_eq!(text.matches("\x1b_G").count(), 1, "only the question: {text:?}");
    view.press("q");
    assert!(view.finish().success());
    assert!(view.text().ends_with("\x1b[?1049l") || view.text().contains("\x1b[?1049l"));
}

/// Asked for by name: no question about the protocol, no temporary files, no warning; and pages
/// turn, each of its own rows.
#[test]
fn blocks_by_name_ask_nothing_about_images_and_page() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("blocks-pager", 3);
    let dir = pdf.with_file_name("tmp");
    std::fs::create_dir_all(&dir).unwrap();
    let terminal = Terminal { takes_files: true, tmpdir: Some(&dir), ..TEXT_ONLY };
    let mut view = Session::start_on(terminal, &pdf, &["--protocol", "blocks"], 80, 40);
    view.wait_until("page 1 drawn", |t| drawn(t, "page 1/3") && has_cells(t));
    let before = view.text().len();
    view.press("n");
    view.wait_until("page 2 drawn", |t| drawn(&t[before..], "page 2/3") && has_cells(&t[before..]));
    let text = view.text();
    assert!(!text.contains("\x1b_G"), "{text:?}");
    assert!(!text.contains("no graphics protocol"), "{text:?}");
    assert!(view.taken.lock().unwrap().is_empty());
    view.press("q");
    assert!(view.finish().success());
    assert!(std::fs::read_dir(&dir).unwrap().next().is_none(), "a file in {}", dir.display());
}

/// A redraw that changes nothing on the page writes none of its rows: over SSH, or on a slow
/// terminal, a screenful of colours for a key that moves nothing would be felt.
#[test]
fn a_redraw_writes_only_the_rows_that_change() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("rows", 3);
    let mut view = Session::start_on(TEXT_ONLY, &pdf, &["--protocol", "blocks"], 80, 40);
    view.wait_until("page 1 drawn", |t| drawn(t, "page 1/3") && has_cells(t));
    let before = view.text().len();
    // A number being typed: the status line says so, and the page is as it was.
    view.press("2");
    view.wait_until("the number said", |t| t[before..].contains("go to page 2"));
    assert!(!has_cells(&view.text()[before..]), "{:?}", &view.text()[before..]);
    view.press("q");
    assert!(view.finish().success());
}

/// What else wrote to the rows, or asks for them all again: the keys' help, and `r`, which is how a
/// screen something else garbled comes right. So `r` writes even a row that was blank before and
/// stays blank: three pages to a row leave the window's top row empty.
#[test]
fn help_and_r_draw_every_row_again() {
    if !pty_works() {
        return;
    }
    const TOP_ROW: &str = "\x1b[1;1H\x1b[2K";
    let pdf = fixture("rows-again", 3);
    let mut view = Session::start_on(TEXT_ONLY, &pdf, &["--protocol", "blocks", "--per-row", "3"], 80, 40);
    view.wait_until("the pages drawn", |t| drawn(t, "pages 1–3/3") && has_cells(t));
    view.press("?");
    view.wait_until("the help", |t| t.contains("Keys"));
    let before = view.text().len();
    view.press("?");
    view.wait_until("the pages back", |t| has_cells(&t[before..]) && drawn(&t[before..], "pages 1–3/3"));
    // Written again, and nothing changed since.
    view.press("2");
    let settled = view.text().len();
    view.wait_until("the number said", |t| t[settled..].contains("go to page 2"));
    assert!(!view.text()[settled..].contains(TOP_ROW), "{:?}", &view.text()[settled..]);
    let before = view.text().len();
    view.press("r");
    view.wait_until("drawn again", |t| has_cells(&t[before..]) && t[before..].contains(TOP_ROW));
    view.press("q");
    assert!(view.finish().success());
}

/// Moving through a list only moves the highlight: the tiles' rows stay, the names' row is written.
#[test]
fn moving_through_a_list_of_text_tiles_writes_only_their_names() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("text-list", 1);
    let other = pdf.with_file_name("other.pdf");
    std::fs::write(&other, crate::fixtures::letter("Other", 1)).unwrap();
    let mut view = Session::start_on(TEXT_ONLY, &pdf, &[other.to_str().unwrap(), "--protocol", "blocks"], 100, 40);
    // The name chosen, centred under its tile, alone in its highlight as the status line's isn't.
    let chosen = |text: &str, name: &str| text.split("\x1b[7m").skip(1).any(|on| on.split("\x1b[0m").next().is_some_and(|on| on.trim() == name));
    view.wait_until("both tiles drawn", |t| chosen(t, "one.pdf") && t.contains("other.pdf") && has_cells(t) && drawn(t, "1/2"));
    let before = view.text().len();
    view.press("l");
    view.wait_until("the next chosen", |t| chosen(&t[before..], "other.pdf"));
    assert!(!has_cells(&view.text()[before..]), "{:?}", &view.text()[before..]);
    view.press("q");
    assert!(view.finish().success());
}

/// A picture whose top half is red and whose bottom half is blue.
#[cfg(feature = "image")]
fn two_colours(test: &str, width: u32, height: u32) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("clview-pager-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("two.png");
    let rgba: Vec<u8> =
        (0..height).flat_map(|y| (0..width).flat_map(move |_| if y < height / 2 { [255, 0, 0, 255] } else { [0, 0, 255, 255] })).collect();
    image::save_buffer(&png, &rgba, width, height, image::ExtendedColorType::Rgba8).unwrap();
    png
}

/// As wide as the window, a tall picture runs past the bottom: its rows of text scroll as an image's
/// pixels do, its bottom half only showing further down.
#[cfg(feature = "image")]
#[test]
fn a_page_taller_than_the_window_scrolls_its_rows() {
    if !pty_works() {
        return;
    }
    const RED: &str = "\x1b[48;5;196m";
    const BLUE: &str = "\x1b[48;5;21m";
    let png = two_colours("text-scroll", 40, 400);
    let mut view = Session::start_on(TEXT_ONLY, &png, &["--pager", "--protocol", "blocks", "--per-row", "width"], 40, 12);
    view.wait_until("the top drawn", |t| drawn(t, "page 1/1") && t.contains(RED));
    assert!(!view.text().contains(BLUE), "the bottom half already: {:?}", view.text());
    let before = view.text().len();
    view.press("G");
    view.press(" ".repeat(20).as_str());
    view.wait_until("the bottom drawn", |t| t[before..].contains(BLUE));
    view.press("q");
    assert!(view.finish().success());
}

/// A small picture, printed, is a column for each pixel across and as many rows as keep its shape in
/// the 10 × 20 px cells assumed, whatever it would cover as an image; not one that keeps its size on
/// screen, of which text cells would show next to nothing.
#[cfg(feature = "image")]
#[test]
fn a_small_picture_gets_a_column_for_each_pixel() {
    if !pty_works() {
        return;
    }
    let png = picture("text-small", 20, 10);
    let mut view = Session::start_on(TEXT_ONLY, &png, &["--protocol", "blocks"], 80, 24);
    assert!(view.finish().success());
    let shown = visible(&view.text());
    let lines: Vec<&str> = shown.split("\r\n").filter(|line| !line.is_empty()).collect();
    assert_eq!(lines.len(), 5, "{shown:?}");
    assert!(lines.iter().all(|line| line.chars().count() == 20), "{shown:?}");
}

/// One that says it is kitty, so isn't asked if it speaks the protocol, and shows images.
#[cfg(feature = "word")]
const KITTY: Terminal = Terminal { term: Some("xterm-kitty"), ..PLAIN };

/// A dark terminal, not grey, as many are: what a page takes from it can't be mistaken.
#[cfg(feature = "word")]
const DARK: &str = "rgb:1e1e/2e2e/3e3e";
#[cfg(feature = "word")]
const DARK_COLOUR: [u8; 3] = [0x1E, 0x2E, 0x3E];

/// What `clv` falls back on for dark paper.
#[cfg(feature = "word")]
const DARK_PAPER: [u8; 3] = [0x1E, 0x1E, 0x1E];

/// The colour at the corner of each page sent so far, in order.
#[cfg(feature = "word")]
fn corners(text: &str) -> Vec<[u8; 3]> {
    use base64::Engine as _;
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("\x1b_Ga=t,f=100,t=d,q=2,i=") {
        rest = &rest[at..];
        let mut payload = String::new();
        loop {
            let Some(body) = rest.strip_prefix("\x1b_G") else { return found };
            let Some(end) = body.find("\x1b\\") else { return found };
            let Some((control, data)) = body[..end].split_once(';') else { return found };
            payload.push_str(data);
            rest = &body[end + 2..];
            if control.ends_with("m=0") {
                break;
            }
        }
        let png = base64::engine::general_purpose::STANDARD.decode(&payload).unwrap();
        found.push(::image::load_from_memory(&png).unwrap().to_rgb8().get_pixel(0, 0).0);
    }
    found
}

/// A Markdown file for a test.
#[cfg(feature = "word")]
fn markdown(test: &str) -> PathBuf {
    let path = crate::view::tmp(test, "notes.md");
    std::fs::write(&path, "# One\n\nSome text.\n").unwrap();
    path
}

/// Wait until `count` pages have been sent, then the colour at the corner of the last.
#[cfg(feature = "word")]
fn last_corner(view: &Session, count: usize) -> [u8; 3] {
    view.wait_until("the page", |t| transmitted(t) >= count && corners(t).len() >= count);
    *corners(&view.text()).last().unwrap()
}

/// On a dark terminal, Markdown takes the terminal's own colour, so the page is what is behind it, and
/// `d` turns it to white and back: sent again each time, and saying so.
#[cfg(feature = "word")]
#[test]
fn markdown_is_on_a_dark_terminals_own_colour_and_d_turns_it() {
    if !pty_works() {
        return;
    }
    let terminal = Terminal { background: Some(DARK), ..KITTY };
    let mut view = Session::start_on(terminal, &markdown("dark"), &["--pager"], 80, 40);
    assert_eq!(last_corner(&view, 1), DARK_COLOUR);
    view.press("d");
    assert_eq!(last_corner(&view, 2), [255; 3]);
    view.wait_until("said", |t| bar(t).contains("dark on light"));
    view.press("d");
    assert_eq!(last_corner(&view, 3), DARK_COLOUR);
    view.wait_until("said", |t| bar(t).contains("light on dark"));
    view.press("q");
    assert!(view.finish().success());
}

/// A light terminal, or one that doesn't say, gets a page as printed; `d` still takes it to dark paper,
/// of a colour of `clv`'s own, since the terminal's isn't one or isn't known.
#[cfg(feature = "word")]
#[test]
fn on_a_light_terminal_or_one_that_doesnt_say_d_goes_to_a_dark_of_its_own() {
    if !pty_works() {
        return;
    }
    for background in [Some("rgb:fdfd/f6f6/e3e3"), None] {
        let mut view = Session::start_on(Terminal { background, ..KITTY }, &markdown("light"), &["--pager"], 80, 40);
        assert_eq!(last_corner(&view, 1), [255; 3], "{background:?}");
        view.press("d");
        assert_eq!(last_corner(&view, 2), DARK_PAPER, "{background:?}");
        view.press("q");
        assert!(view.finish().success());
    }
}

/// `--theme` decides how it starts, whatever the terminal is; turned, it still goes to that terminal's
/// colour.
#[cfg(feature = "word")]
#[test]
fn the_theme_decides_how_it_starts() {
    if !pty_works() {
        return;
    }
    let dark = Terminal { background: Some(DARK), ..KITTY };
    let mut view = Session::start_on(dark, &markdown("theme"), &["--pager", "--theme", "light"], 80, 40);
    assert_eq!(last_corner(&view, 1), [255; 3]);
    view.press("d");
    assert_eq!(last_corner(&view, 2), DARK_COLOUR);
    view.press("q");
    assert!(view.finish().success());

    let light = Terminal { background: Some("rgb:ffff/ffff/ffff"), ..KITTY };
    let mut view = Session::start_on(light, &markdown("theme"), &["--pager", "--theme", "dark"], 80, 40);
    assert_eq!(last_corner(&view, 1), DARK_PAPER);
    view.press("q");
    assert!(view.finish().success());
}

/// What has no dark form doesn't ask about the background, and `d` says so and sends nothing: `d`
/// is the documents' backend's.
#[cfg(feature = "word")]
#[test]
fn what_has_no_dark_form_isnt_asked_and_d_says_so() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("no-dark", 1);
    let mut view = Session::start_on(Terminal { background: Some(DARK), ..KITTY }, &pdf, &["--pager"], 80, 40);
    assert_eq!(last_corner(&view, 1), [255; 3]);
    view.press("d");
    view.wait_until("said", |t| bar(t).contains("only Markdown and HTML go dark"));
    assert_eq!(transmitted(&view.text()), 1, "sent again");
    assert!(!view.text().contains("\x1b]11;"), "asked about the background");
    view.press("q");
    assert!(view.finish().success());
}

/// In a list, `d` turns what has a dark form, and what hasn't stays white.
#[cfg(feature = "word")]
#[test]
fn in_a_list_only_what_has_a_dark_form_turns() {
    if !pty_works() {
        return;
    }
    let dir = crate::view::tmp("dark-list", "shelf");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.md"), "# One").unwrap();
    std::fs::copy(fixture("dark-list", 1), dir.join("b.pdf")).unwrap();
    let mut view = Session::start_on(Terminal { background: Some(DARK), ..KITTY }, &dir, &[], 80, 40);
    view.wait_until("both tiles", |t| corners(t).len() >= 2);
    let tiles = corners(&view.text());
    assert!(tiles.contains(&DARK_COLOUR) && tiles.contains(&[255; 3]), "{tiles:?}");
    view.press("d");
    view.wait_until("both again", |t| corners(t).len() >= 4);
    assert_eq!(corners(&view.text())[2..], [[255; 3], [255; 3]]);
    view.press("q");
    assert!(view.finish().success());
}

/// A long Markdown file for a test: several pages at any width.
#[cfg(feature = "word")]
fn long_markdown(test: &str, name: &str) -> PathBuf {
    let path = crate::view::tmp(test, name);
    let long: String = (1..=60).map(|n| format!("Paragraph {n}, with words enough in it to fill a line or two of a page.\n\n")).collect();
    std::fs::write(&path, format!("# Long\n\n{long}")).unwrap();
    path
}

/// Whether the last page sent is shaped like one `w` × `h` points large.
#[cfg(feature = "word")]
fn last_sent_shaped(text: &str, w: f32, h: f32) -> bool {
    sizes_sent(text).last().is_some_and(|(x, y)| (*x as f32 / *y as f32 - w / h).abs() < 0.01)
}

/// How many pages the document has, by the last status line drawn: `page 1/12`, `pages 1–2/12`.
#[cfg(feature = "word")]
fn page_count(text: &str) -> Option<usize> {
    let line = bar(last_draw(text));
    let count = line.get(line.rfind(" page")?..)?.split_once('/')?.1;
    count.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

/// A document's page in these tests' pager, in points: the window's, 100 × 59 cells of the 10 × 20 px
/// assumed (a column 5.64 pt, a row 11.28), and one 40 characters wide, as tall.
#[cfg(feature = "word")]
const WINDOW: (f32, f32) = (100.0 * 5.64, 59.0 * 11.28);
#[cfg(feature = "word")]
const NARROW: f32 = 46.0 * 5.64;

/// A document's page is the window's: as tall as the pages area, as wide. `40]` lays the document out
/// again at 40 characters a line, the new pages narrower and as tall, the status line saying `width 40`
/// on its right; `]` steps on to 44; `0]` is `window` again, the window's page, and nothing on the
/// right. Each time, the pages before stay until the new ones are ready.
#[cfg(feature = "word")]
#[test]
fn the_width_keys_lay_the_document_out_again_and_the_status_line_says_what_it_is_at() {
    if !pty_works() {
        return;
    }
    let mut view = Session::start_on(KITTY, &long_markdown("reflow", "long.md"), &["--pager"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/"));
    assert!(last_sent_shaped(&view.text(), WINDOW.0, WINDOW.1), "{:?}", sizes_sent(&view.text()));
    // Fitted to the pages area: as tall as its 59 rows, in all its 100 columns.
    let (w, h) = sizes_sent(&view.text())[0];
    assert!((999..=1001).contains(&w) && (1179..=1181).contains(&h), "the page fills the window: {w} × {h}");
    view.press("40]");
    view.wait_until("narrow pages, said", |t| bar(t).contains("· width 40 ·") && last_sent_shaped(t, NARROW, WINDOW.1));
    let narrow_pages = page_count(&view.text()).unwrap();
    view.press("]");
    view.wait_until("stepped", |t| bar(t).contains("· width 44 ·") && last_sent_shaped(t, 50.0 * 5.64, WINDOW.1));
    view.press("0]");
    view.wait_until("the window's again", |t| bar(t).contains("width window") && last_sent_shaped(t, WINDOW.0, WINDOW.1));
    view.wait_until("fewer pages", |t| page_count(t).is_some_and(|count| count < narrow_pages));
    assert!(!bar(&view.text()).rsplit_once("width window").unwrap().1.contains("width"), "{:?}", last_draw(&view.text()));
    view.press("q");
    assert!(view.finish().success());
}

/// On a PDF the width keys say so, and nothing is sent again.
#[cfg(feature = "word")]
#[test]
fn on_a_pdf_the_width_keys_say_only_markdown_and_html_reflow() {
    if !pty_works() {
        return;
    }
    let mut view = Session::start_on(KITTY, &fixture("no-reflow", 1), &["--pager"], 80, 40);
    view.wait_until("page 1", |t| drawn(t, "page 1/1"));
    view.press("]");
    view.wait_until("said", |t| bar(t).contains("only Markdown and HTML reflow"));
    view.press("72[");
    view.wait_until("said again", |t| bar(t).matches("only Markdown and HTML reflow").count() >= 2);
    assert_eq!(transmitted(&view.text()), 1);
    view.press("q");
    assert!(view.finish().success());
}

/// A copy still rendering when the document is laid out again stops, and says so: it was of the old
/// layout's pool.
#[cfg(feature = "word")]
#[test]
fn a_copy_still_rendering_stops_when_the_document_is_laid_out_again() {
    if !pty_works() {
        return;
    }
    let terminal = Terminal { clipboard: Some(true), ..KITTY };
    let mut view = Session::start_on(terminal, &long_markdown("reflow-copy", "notes.md"), &["--pager"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/"));
    view.press("c40]");
    view.wait_until("stopped", |t| bar(t).contains("stopped copying page 1"));
    view.wait_until("laid out again", |t| last_sent_shaped(t, NARROW, WINDOW.1));
    assert!(!view.text().contains("copied page 1"));
    view.press("q");
    assert!(view.finish().success());
}

/// In a list the width keys say to open a document; a document opened from it, and changed, is laid
/// out again, and its tile isn't; back in the list, the next document gone into is laid out at the
/// new width as it opens, the keys untouched.
#[cfg(feature = "word")]
#[test]
fn a_document_gone_into_after_a_change_is_laid_out_at_the_new_width_and_its_tile_is_not() {
    if !pty_works() {
        return;
    }
    let dir = crate::view::tmp("reflow-list", "shelf");
    std::fs::create_dir_all(&dir).unwrap();
    long_markdown("reflow-list", "shelf/a.md");
    long_markdown("reflow-list", "shelf/b.md");
    let mut view = Session::start_on(KITTY, &dir, &[], 100, 60);
    view.wait_until("both tiles", |t| drawn(t, "1/2 · 2 per row"));
    assert_eq!(sizes_sent(&view.text()).len(), 2);
    view.press("]");
    view.wait_until("said", |t| bar(t).contains("open a document to reflow it"));
    view.press("\r");
    view.wait_until("a opened", |t| t.contains("  page 1/") && bar(t).contains(" a.md "));
    view.press("40]");
    view.wait_until("a laid out again", |t| bar(t).contains("· width 40 ·") && last_sent_shaped(t, NARROW, WINDOW.1));
    view.press("u");
    view.wait_until("back in the list", |t| bar(last_draw(t)).contains(" 1/2 ·"));
    let sent = sizes_sent(&view.text()).len();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(sizes_sent(&view.text()).len(), sent, "the tiles are as they were");
    assert!(!bar(last_draw(&view.text())).contains("width"), "{:?}", last_draw(&view.text()));
    view.press("\x1b[C");
    view.wait_until("b chosen", |t| chosen(t).as_deref() == Some("b.md"));
    view.press("\r");
    view.wait_until("b opened at 40", |t| bar(t).contains(" b.md ") && bar(t).contains("· width 40 ·") && last_sent_shaped(t, NARROW, WINDOW.1));
    view.press("q");
    assert!(view.finish().success());
}

/// The layout runs on a thread: keys work meanwhile (`?` shows the keys before the new layout is
/// ready), the status line says the width wanted with `…` after it, and the new pages show with no key
/// once it is done.
#[cfg(feature = "word")]
#[test]
fn keys_work_while_the_document_is_laid_out_again_and_the_new_pages_show_with_no_key() {
    if !pty_works() {
        return;
    }
    // Enough text to take a while to lay out: about a second and a half here.
    let path = crate::view::tmp("reflow-threads", "big.md");
    let paragraph = "Words enough in a paragraph to fill a few lines of a page, over and over again. ".repeat(12);
    let big: String = (1..=2800).map(|n| format!("Paragraph {n}. {paragraph}\n\n")).collect();
    std::fs::write(&path, big).unwrap();
    let mut view = Session::start_on(KITTY, &path, &["--pager"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/") || t.contains("  pages 1"));
    view.press("40]");
    view.wait_until("under way", |t| t.contains("width 40…"));
    view.press("?");
    const KEYS: &str = "these keys: any key goes back";
    view.wait_until("the keys", |t| t.contains(KEYS));
    view.wait_until("laid out", |t| bar(t).contains("· width 40 ·"));
    let text = view.text();
    // The slot on the right, padded, once the layout is done: the key's own message says `width 40` too.
    let slot = text.match_indices("width 40").map(|(at, _)| at).find(|at| text[at + "width 40".len()..].trim_start_matches(' ').starts_with('·'));
    let (keys, done) = (text.find(KEYS).unwrap(), slot.expect("the slot without …"));
    assert!(keys < done, "the keys were shown only after the layout");
    // The keys stay up, over the new layout, until a key; then the narrow pages are there.
    view.press("n");
    view.wait_until("the pages", |t| t.get(done..).is_some_and(|t| t.contains("a=p,q=1,")) && last_sent_shaped(t, NARROW, WINDOW.1));
    view.press("q");
    assert!(view.finish().success());
}

/// A layout that fails keeps the pages as they are, says why, and puts the value back to what is on
/// screen: the next `]` steps from there, not from the value that failed.
#[cfg(feature = "word")]
#[test]
fn a_layout_that_fails_keeps_the_old_pages_and_puts_the_value_back() {
    if !pty_works() {
        return;
    }
    let path = long_markdown("reflow-fails", "gone.md");
    let mut view = Session::start_on(KITTY, &path, &["--pager"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/"));
    std::fs::remove_file(&path).unwrap();
    view.press("40]");
    view.wait_until("why", |t| bar(t).contains("reading the file again"));
    std::thread::sleep(Duration::from_millis(300));
    let text = view.text();
    let window = |(w, h): &(u32, u32)| (*w as f32 / *h as f32 - WINDOW.0 / WINDOW.1).abs() < 0.01;
    assert!(!bar(&text).contains("· width 40 ·") && sizes_sent(&text).iter().all(window), "{:?}", last_draw(&text));
    // From `window` again, set at the 94 characters this window holds (the number first), not from 40.
    view.press("[");
    view.wait_until("from window again", |t| bar(t).contains("width 94"));
    assert!(!view.text().contains("width 36"));
    view.press("q");
    assert!(view.finish().success());
}

/// A page a third of these tests' window wide, in points: (100 − 2 × 2) / 3 = 32 columns.
#[cfg(feature = "word")]
const THIRD: f32 = 32.0 * 5.64;

/// On Markdown `3.` lays the document out again in three columns, each as tall as the window, three to
/// a row; `,` four. Between a count and a typed width the last key wins: `72]` sets the count back to
/// `auto`, `3.` the width back to `window`, and `[` steps from `window` as it sits at `auto`, not in a
/// column. `0.` is `auto` again. With `--pages` they are the zoom alone: the pages as laid out.
#[cfg(feature = "word")]
#[test]
fn a_count_to_a_row_lays_markdown_out_in_columns_and_the_last_key_wins() {
    if !pty_works() {
        return;
    }
    let mut view = Session::start_on(KITTY, &long_markdown("columns", "long.md"), &["--pager"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/"));
    let since = |view: &mut Session, keys: &str, what: &str, done: &dyn Fn(&str) -> bool| {
        let before = view.text().len();
        view.press(keys);
        view.wait_until(what, |t| done(&t[before..]));
    };
    since(&mut view, "3.", "three columns", &|t| {
        bar(last_draw(t)).contains("· 3 per row ·") && t.contains("  pages 1–3/") && last_sent_shaped(t, THIRD, WINDOW.1)
    });
    since(&mut view, ",", "four", &|t| t.contains("  pages 1–4/") && last_sent_shaped(t, 23.0 * 5.64, WINDOW.1));
    // A width: the count back to auto, the pages 78 columns wide, one to a row.
    since(&mut view, "72]", "width 72", &|t| bar(last_draw(t)).contains("· width 72 · 1 per row ·") && last_sent_shaped(t, 78.0 * 5.64, WINDOW.1));
    // A count: the width back to window.
    since(&mut view, "3.", "three again", &|t| {
        last_sent_shaped(t, THIRD, WINDOW.1) && t.contains("  pages 1–3/") && !bar(last_draw(t)).contains("width")
    });
    // `[` from window at auto, the window's 94 characters: not from a column's 26.
    since(&mut view, "[", "from the window's width", &|t| bar(last_draw(t)).contains("· width 94 ·") && last_sent_shaped(t, WINDOW.0, WINDOW.1));
    assert!(!view.text().contains("width 26"));
    since(&mut view, "3.0.", "auto again", &|t| bar(last_draw(t)).contains("· 1 per row ·") && last_sent_shaped(t, WINDOW.0, WINDOW.1));
    view.press("q");
    assert!(view.finish().success());
    // With `--pages`, the zoom: three of the window's pages to a row, none laid out again.
    let mut view = Session::start_on(KITTY, &long_markdown("columns-pages", "long.md"), &["--pager", "--pages", "1-3"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/"));
    since(&mut view, "3.", "three to a row", &|t| t.contains("  pages 1–3/") && bar(last_draw(t)).contains("· 3 per row ·"));
    let window = |(w, h): &(u32, u32)| (*w as f32 / *h as f32 - WINDOW.0 / WINDOW.1).abs() < 0.01;
    assert!(sizes_sent(&view.text()).iter().all(window), "{:?}", sizes_sent(&view.text()));
    view.press("q");
    assert!(view.finish().success());
}

/// Printed on a terminal, `--per-row 3` lays Markdown out for the room the tty says, in three columns
/// as tall as it: in 100 × 60, pages 32 columns by 59 rows, three to a row; not the window's pages
/// shrunk into a third of it (32 × 19).
#[cfg(feature = "word")]
#[test]
fn printing_lays_markdown_out_in_columns_for_the_ttys_room() {
    if !pty_works() {
        return;
    }
    let path = long_markdown("print-columns", "long.md");
    let mut view = Session::start(&path, &["--protocol", "kitty", "--no-pager", "--per-row", "3"], 100, 60);
    assert!(view.finish().success());
    let text = view.text();
    let column = text.matches("\x1b_Ga=T,f=100,t=d,q=2,C=1,c=32,r=59,").count();
    assert!(column >= 3 && !text.contains(",c=32,r=19,"), "{column} columns: {:?}", &text[..text.len().min(400)]);
}

/// `}` and `{` lay a Markdown file out again with side margins a character wider or narrower, the
/// status line saying the margin while it isn't 3, `0}` back to 3; `}` stops at the most the window
/// holds, where a line keeps 20 characters (6 in a column of 32), and says so. In a list they
/// say to open a document.
#[cfg(feature = "word")]
#[test]
fn the_margin_keys_lay_the_document_out_again_and_stop_at_the_most_here() {
    if !pty_works() {
        return;
    }
    let mut view = Session::start_on(KITTY, &long_markdown("margins", "long.md"), &["--pager", "--width", "40"], 100, 60);
    // Narrow pages, two to a row.
    view.wait_until("page 1", |t| t.contains("  pages 1–2/") && last_sent_shaped(t, NARROW, WINDOW.1));
    view.press("}");
    view.wait_until("margin 4", |t| bar(last_draw(t)).contains("· width 40 · margin 4 ·") && last_sent_shaped(t, 48.0 * 5.64, WINDOW.1));
    view.press("0}");
    // Said as the key's message, and gone from the slot on the right.
    view.wait_until("3 again", |t| {
        bar(last_draw(t)).contains(" margin 3 ") && !bar(last_draw(t)).contains("· margin") && last_sent_shaped(t, NARROW, WINDOW.1)
    });
    view.press("q");
    assert!(view.finish().success());
    // In three columns of 32, a line keeps 20 characters within 6 at most.
    let mut view = Session::start_on(KITTY, &long_markdown("margins-most", "long.md"), &["--pager"], 100, 60);
    view.wait_until("page 1, again", |t| t.contains("  page 1/"));
    view.press("3.");
    view.wait_until("three columns", |t| last_sent_shaped(t, THIRD, WINDOW.1));
    view.press("6}}");
    view.wait_until("the most here", |t| bar(t).contains("margin 6, the most here"));
    // The margin yields to the font: 12 typed is kept and laid out at 6, the text at its size.
    let before = view.text().len();
    view.press("12}");
    view.wait_until("12, laid out at 6", |t| bar(last_draw(&t[before..])).contains("· margin 12 (6) ·"));
    assert!(!bar(last_draw(&view.text())).contains("font"), "the font as asked: {:?}", bar(last_draw(&view.text())));
    view.press("q");
    assert!(view.finish().success());
    let dir = crate::view::tmp("margins-list", "shelf");
    std::fs::create_dir_all(&dir).unwrap();
    long_markdown("margins-list", "shelf/a.md");
    long_markdown("margins-list", "shelf/b.md");
    let mut view = Session::start_on(KITTY, &dir, &[], 100, 60);
    view.wait_until("both tiles", |t| drawn(t, "1/2 · 2 per row"));
    view.press("}");
    view.wait_until("said", |t| bar(t).contains("open a document to reflow it"));
    view.press("q");
    assert!(view.finish().success());
}

/// A key that changes both the count and the width is one change: when its layout fails, both go back
/// to what is on screen, and the next step starts from there.
#[cfg(feature = "word")]
#[test]
fn a_count_whose_layout_fails_puts_the_count_and_the_width_back() {
    if !pty_works() {
        return;
    }
    let path = long_markdown("columns-fail", "gone.md");
    let mut view = Session::start_on(KITTY, &path, &["--pager"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/"));
    view.press("72]");
    view.wait_until("width 72", |t| bar(last_draw(t)).contains("· width 72 ·") && last_sent_shaped(t, 78.0 * 5.64, WINDOW.1));
    std::fs::remove_file(&path).unwrap();
    let before = view.text().len();
    view.press("3.");
    view.wait_until("why", |t| bar(&t[before..]).contains("os error"));
    std::thread::sleep(Duration::from_millis(300));
    let text = view.text();
    assert!(bar(last_draw(&text)).contains("· width 72 · 1 per row ·"), "both back: {:?}", last_draw(&text));
    // The old pages, drawn three to a row until the layout came back, and nothing laid out in columns.
    let as_laid = |(w, h): &(u32, u32)| (*w as f32 / *h as f32 - 78.0 * 5.64 / WINDOW.1).abs() < 0.01;
    assert!(sizes_sent(&text[before..]).iter().all(as_laid), "nothing laid out: {:?}", sizes_sent(&text[before..]));
    // From one to a row again: `,` is two, not four.
    let before = view.text().len();
    view.press(",");
    view.wait_until("two asked", |t| bar(&t[before..]).contains("os error") || bar(&t[before..]).contains("2 per row"));
    assert!(!view.text()[before..].contains("4 per row"));
    view.press("q");
    assert!(view.finish().success());
}

/// In text cells the count reaches the layout as it does in images: three columns, more pages.
#[cfg(feature = "word")]
#[test]
fn text_cells_lay_markdown_out_in_columns_too() {
    if !pty_works() {
        return;
    }
    let mut view = Session::start_on(TEXT_ONLY, &long_markdown("columns-cells", "long.md"), &["--pager", "--protocol", "blocks"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/"));
    let alone = page_count(&view.text()).unwrap();
    view.press("3.");
    view.wait_until("three columns", |t| bar(last_draw(t)).contains("· 3 per row ·") && page_count(t).is_some_and(|count| count > alone));
    view.press("q");
    assert!(view.finish().success());
}

/// Where the reader is, by the last status line drawn: the first page on screen, of how many.
#[cfg(feature = "word")]
fn place_now(text: &str) -> Option<(usize, usize)> {
    let line = bar(last_draw(text));
    let at = line.get(line.rfind(" page")?..)?.trim_start_matches(|c: char| !c.is_ascii_digit());
    let (first, count) = at.split_once('/')?;
    let first = first.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    Some((first, count.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?))
}

/// The paragraph at the top of the screen stays on the reader's page across a relayout: narrower pages
/// hold fewer of them, so from page 9 the reader lands further on (two narrow pages to a row, the pair
/// shown starts at an even page), and back on the window's page again, near 9.
#[cfg(feature = "word")]
#[test]
fn the_paragraph_at_the_top_of_the_screen_stays_on_the_readers_page_across_a_relayout() {
    if !pty_works() {
        return;
    }
    let path = crate::view::tmp("reflow-place", "place.md");
    let text: String = (1..=400)
        .map(|n| format!("Paragraph {n}, with words enough in it to take a line or two of a page as printed, and more of a narrow one.\n\n"))
        .collect();
    std::fs::write(&path, text).unwrap();
    let mut view = Session::start_on(KITTY, &path, &["--pager"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/"));
    let letter_pages = place_now(&view.text()).unwrap().1;
    assert!(letter_pages >= 8, "{letter_pages} pages");
    view.press("9g");
    view.wait_until("page 9", |t| t.contains("  page 9/"));
    view.press("40]");
    view.wait_until("laid out at 40", |t| bar(t).contains("· width 40 ·"));
    let (page, pages) = place_now(&view.text()).unwrap();
    assert!(pages > letter_pages && page >= 11 && page < pages, "page {page} of {pages}, from page 9 of {letter_pages}");
    view.press("0]");
    view.wait_until("the window's page again", |t| {
        let line = bar(last_draw(t));
        line.contains("width window") && !line.contains("· width")
    });
    let (back, _) = place_now(&view.text()).unwrap();
    assert!((7..=10).contains(&back), "page {back}");
    view.press("q");
    assert!(view.finish().success());
}

/// HTML through a pipe lays out again from the bytes kept, since it can't be read again.
#[cfg(feature = "word")]
#[test]
fn html_through_a_pipe_lays_out_again_too() {
    if !pty_works() {
        return;
    }
    let path = crate::view::tmp("reflow-pipe", "page.html");
    let long: String = (1..=60).map(|n| format!("<p>Paragraph {n}, with words enough in it to fill a line or two of a page.</p>\n")).collect();
    std::fs::write(&path, format!("<!doctype html><title>Long</title>{long}")).unwrap();
    let command = format!("cat '{}' | {BIN} /dev/stdin --pager", path.display());
    let mut view = Session::start_command(KITTY, &command, 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/"));
    view.press("40]");
    view.wait_until("laid out again", |t| bar(t).contains("· width 40 ·") && last_sent_shaped(t, NARROW, WINDOW.1));
    view.press("q");
    assert!(view.finish().success());
}

/// With `--pages`, which picks pages as laid out, the width keys say so and change nothing.
#[cfg(feature = "word")]
#[test]
fn with_pages_the_width_keys_are_refused() {
    if !pty_works() {
        return;
    }
    let mut view = Session::start_on(KITTY, &long_markdown("reflow-pages", "long.md"), &["--pager", "--pages", "2-3"], 100, 60);
    view.wait_until("page 2, and page 3 ahead", |t| t.contains("  page 2/") && transmitted(t) == 2);
    let sent = transmitted(&view.text());
    view.press("]");
    view.wait_until("said", |t| bar(t).contains("--pages picks pages as laid out: no reflow"));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(transmitted(&view.text()), sent);
    view.press("q");
    assert!(view.finish().success());
}

/// A change of the window's size lays the document out again, once the window has held still for half a
/// second: the status line says `laying out…` meanwhile (nothing but the default to say), the new pages
/// are the new window's shape, and the document has another number of pages. One layout for one
/// change.
#[cfg(feature = "word")]
#[test]
fn a_change_of_size_lays_the_document_out_again_once_the_window_has_held() {
    if !pty_works() {
        return;
    }
    let path = long_markdown("resize", "long.md");
    // The resize is asked for through a fifo, from a subshell on the same terminal, once the first
    // pages show.
    let fifo = crate::view::tmp("resize", "go");
    let _ = std::fs::remove_file(&fifo);
    assert!(Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    let resize = format!("{{ (cat '{}' > /dev/null; stty rows 40 cols 80 < /dev/tty) & }}", fifo.display());
    let command = format!("{resize}; exec '{BIN}' '{}' --pager", path.display());
    let mut view = Session::start_command(KITTY, &command, 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/") && last_sent_shaped(t, WINDOW.0, WINDOW.1));
    let before = page_count(&view.text()).unwrap();
    std::fs::write(&fifo, "go").unwrap();
    // 80 × 39 cells of 10 × 20 px: a page 451.2 × 439.92 pt.
    let (w, h) = (80.0 * 5.64, 39.0 * 11.28);
    view.wait_until("laying out", |t| t.contains("laying out…"));
    view.wait_until("the new window's pages", |t| last_sent_shaped(t, w, h) && page_count(t).is_some_and(|count| count != before));
    let after = page_count(&view.text()).unwrap();
    std::thread::sleep(Duration::from_millis(1200));
    let text = view.text();
    assert_eq!(page_count(&text), Some(after), "laid out once");
    assert_eq!(text.matches("laying out…").count(), text.matches("laying out…").count().min(4), "{}", text.matches("laying out…").count());
    assert!(!bar(last_draw(&text)).contains("laying out"), "{:?}", last_draw(&text));
    view.press("q");
    assert!(view.finish().success());
}

/// A terminal that reports no pixels is asked its cell size after the first layout, which took the cell
/// assumed (10 × 20): where the cell it says makes another room (8 × 20, a taller page), the document
/// is laid out once more, through a change of size.
#[cfg(feature = "word")]
#[test]
fn a_terminal_without_pixels_gets_a_second_layout_at_the_cell_it_says() {
    if !pty_works() {
        return;
    }
    let terminal = Terminal { cell: Some((8, 20)), ..KITTY };
    let mut view = Session::start_on(terminal, &long_markdown("resize-cell", "long.md"), &["--pager"], 100, 60);
    // 100 × 59 cells of 8 × 20 px: a page 564 × 831.9 pt, taller than the one assumed.
    view.wait_until("laid out at the cell it says", |t| t.contains("laying out…") && last_sent_shaped(t, WINDOW.0, 59.0 * 2.5 * 5.64));
    let shapes = sizes_sent(&view.text());
    assert!(
        shapes.iter().any(|(x, y)| (*x as f32 / *y as f32 - WINDOW.0 / WINDOW.1).abs() < 0.01),
        "the first layout, at the cell assumed: {shapes:?}"
    );
    view.press("q");
    assert!(view.finish().success());
}

/// On a Markdown file `+` and `-` are the font size: the document is laid out again at it, with fewer
/// pages at a larger size, the status line saying `font 110%` on its right; `0+` goes back to 100%, and
/// nothing on the right. The page still fills the window: the same pixels, larger text. On a PDF they
/// are the zoom, as ever.
#[cfg(feature = "word")]
#[test]
fn the_zoom_keys_are_the_font_size_on_a_markdown_file_and_the_zoom_on_a_pdf() {
    if !pty_works() {
        return;
    }
    let mut view = Session::start_on(KITTY, &long_markdown("font", "long.md"), &["--pager"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/") && last_sent_shaped(t, WINDOW.0, WINDOW.1));
    let at_100 = page_count(&view.text()).unwrap();
    view.press("+");
    view.wait_until("larger, said", |t| bar(t).contains("· font 110% ·"));
    assert!(view.text().contains("font 110%…"), "laid out again, the status line saying so meanwhile");
    assert!(last_sent_shaped(&view.text(), WINDOW.0, WINDOW.1), "the page is the window still");
    assert!(bar(&view.text()).contains("1 per row"), "the zoom didn't move");
    view.press("150+");
    view.wait_until("150%, more pages", |t| bar(t).contains("· font 150% ·") && page_count(t).is_some_and(|count| count > at_100));
    view.press("-");
    view.wait_until("140%", |t| bar(t).contains("· font 140% ·"));
    view.press("0+");
    view.wait_until("100% again", |t| bar(t).contains("font 100%") && page_count(t) == Some(at_100));
    assert!(!bar(last_draw(&view.text())).contains("· font"), "{:?}", last_draw(&view.text()));
    view.press("q");
    assert!(view.finish().success());
    // A PDF: the zoom.
    let mut view = Session::start_on(KITTY, &fixture("font-pdf", 3), &["--pager"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/3") || t.contains("  pages 1–2/3"));
    view.press("+");
    view.wait_until("zoomed", |t| bar(t).contains("· 1 per row ·") || bar(t).contains("· width ·"));
    assert!(!view.text().contains("font"), "no font on a PDF");
    view.press("q");
    assert!(view.finish().success());
}

/// Where `window` sits among the widths depends on the font size: `150+` then `[`, typed together before
/// the layout at 150% lands, steps down from `window` to the 60 characters a 100-column window holds at
/// 150% (the most there, a step of its own, `window` sitting after it), not to the 94 it held at 100%.
#[cfg(feature = "word")]
#[test]
fn a_width_key_after_a_font_key_steps_by_the_bounds_at_the_new_size() {
    if !pty_works() {
        return;
    }
    let mut view = Session::start_on(KITTY, &long_markdown("font-bounds", "long.md"), &["--pager"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/") && last_sent_shaped(t, WINDOW.0, WINDOW.1));
    view.press("150+[");
    view.wait_until("width 60 at 150%", |t| bar(t).contains("· width 60 · font 150% ·"));
    assert!(!view.text().contains("width 94"), "stepped from the bounds at 100%");
    view.press("q");
    assert!(view.finish().success());
}

/// Each image sent, its size: what kitty is told in the stream.
pub(crate) fn sizes_sent(text: &str) -> Vec<(u32, u32)> {
    use base64::Engine as _;
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("\x1b_Ga=t,f=100,t=d,q=2,i=") {
        rest = &rest[at..];
        let mut payload = String::new();
        loop {
            let Some(body) = rest.strip_prefix("\x1b_G") else { return found };
            let Some(end) = body.find("\x1b\\") else { return found };
            let Some((control, data)) = body[..end].split_once(';') else { return found };
            payload.push_str(data);
            rest = &body[end + 2..];
            if control.ends_with("m=0") {
                break;
            }
        }
        found.push(crate::view::png_size(&base64::engine::general_purpose::STANDARD.decode(&payload).unwrap()));
    }
    found
}

/// A list shows before its PDFs are opened, and each comes to its tile at its true size: the size it
/// was first taken to have is never rendered at.
#[test]
fn a_tile_opened_on_demand_is_sent_at_its_true_size() {
    if !pty_works() {
        return;
    }
    let dir = crate::view::tmp("later-size", "pager");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.pdf"), crate::view::wide("Wide")).unwrap();
    std::fs::write(dir.join("b.pdf"), crate::fixtures::letter("Tall", 1)).unwrap();
    let mut view = Session::start(&dir, &["--protocol", "kitty"], 100, 60);
    view.wait_until("both tiles", |t| drawn(t, "1/2 · 2 per row"));
    let sizes = sizes_sent(&view.text());
    assert_eq!(sizes.len(), 2, "each rendered once: {sizes:?}");
    let (wide, tall) = (sizes.iter().find(|(w, h)| w > h).unwrap(), sizes.iter().find(|(w, h)| h > w).unwrap());
    // Fitted to its place by its own width: at a US Letter page's scale it would come out wider.
    assert!(wide.0 * 792 < tall.1 * 999 && wide.0 * 10 >= tall.1 * 9, "{sizes:?}");
    view.press("q");
    assert!(view.finish().success());
}

/// A file that turns out broken keeps its place in the list and its name there, marked: the others
/// don't move, the status line says why when it is chosen, and Enter says so and stays in the list.
#[test]
fn a_file_that_turns_out_broken_keeps_its_tile_and_says_why() {
    if !pty_works() {
        return;
    }
    let dir = crate::view::tmp("later-broken", "pager");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.pdf"), b"%PDF-1.7 and nothing a PDF has").unwrap();
    std::fs::write(dir.join("b.pdf"), crate::fixtures::letter("Fine", 1)).unwrap();
    let mut view = Session::start(&dir, &["--protocol", "kitty"], 100, 60);
    view.wait_until("its mark", |t| t.contains("! a.pdf"));
    view.wait_until("the other tile, and why", |t| transmitted(t) == 1 && bar(t).contains("a.pdf: "));
    view.press("\r");
    view.wait_until("why, going in", |t| t.rmatch_indices("a.pdf: ").count() > 1);
    std::thread::sleep(Duration::from_millis(200));
    assert!(!view.text().contains("page 1/"), "it went into a file that can't be shown");
    // The other tile still opens to its pages.
    view.press("\x1b[C");
    view.wait_until("b chosen", |t| chosen(t).as_deref() == Some("b.pdf"));
    view.press("\r");
    view.wait_until("b opened", |t| bar(t).contains(" b.pdf esc back"));
    view.press("q");
    assert!(view.finish().success());
}

/// `c` copies the page as it is shown.
#[cfg(feature = "word")]
#[test]
fn a_page_is_copied_on_the_paper_it_is_shown_on() {
    if !pty_works() {
        return;
    }
    let terminal = Terminal { background: Some(DARK), clipboard: Some(true), ..KITTY };
    let mut view = Session::start_on(terminal, &markdown("copy-dark"), &["--pager"], 100, 60);
    assert_eq!(last_corner(&view, 1), DARK_COLOUR);
    view.press("c");
    view.wait_until("copied", |t| t.contains("copied notes.md"));
    let colour = |png: &[u8]| ::image::load_from_memory(png).unwrap().to_rgb8().get_pixel(0, 0).0;
    assert_eq!(colour(&copied_png(&view.text(), 1)), DARK_COLOUR);
    view.press("d");
    assert_eq!(last_corner(&view, 2), [255; 3]);
    view.press("c");
    view.wait_until("copied again", |t| copied_png(t, 2).ends_with(b"\x60\x82"));
    assert_eq!(colour(&copied_png(&view.text(), 2)), [255; 3]);
    view.press("q");
    assert!(view.finish().success());
}

/// In text cells too: on a dark terminal, a page of text is its letters, not a block of ink with holes
/// in it, as it is on white paper.
#[cfg(feature = "word")]
#[test]
fn text_cells_go_dark_too() {
    if !pty_works() {
        return;
    }
    let page = |args: &[&str]| {
        let terminal = Terminal { background: Some("rgb:0000/0000/0000"), ..TEXT_ONLY };
        let mut view = Session::start_on(terminal, &markdown("dark-cells"), &[&["--protocol", "ascii"], args].concat(), 80, 40);
        assert!(view.finish().success());
        visible(&view.text()).matches('@').count()
    };
    assert!(page(&["--theme", "light"]) > 500, "white paper is all ink");
    assert!(page(&[]) < 50, "dark paper is none");
}

/// A cell's ink shows against the background, so a terminal is asked for it, and white is all ink on
/// a dark one and none on a light one. Half-blocks don't depend on it, so aren't asked.
#[test]
fn ascii_inks_for_the_terminals_background() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("ground", 1);
    let page = |background| {
        let terminal = Terminal { background, ..TEXT_ONLY };
        let mut view = Session::start_on(terminal, &pdf, &["--protocol", "ascii"], 80, 40);
        assert!(view.finish().success());
        view.text()
    };
    let ink = |text: &str| visible(text).matches('@').count();
    assert!(ink(&page(Some("rgb:0000/0000/0000"))) > 500);
    assert!(ink(&page(None)) > 500, "dark, when it doesn't say");
    let light = page(Some("rgb:ffff/ffff/ffff"));
    assert_eq!(ink(&light), 0, "{light:?}");
    assert!(visible(&light).contains(':'), "the page's text in ink: {light:?}");
    let mut view = Session::start_on(TEXT_ONLY, &pdf, &["--protocol", "blocks"], 80, 40);
    assert!(view.finish().success());
    assert!(!view.text().contains("\x1b]11;"), "{:?}", view.text());
}

/// `c` copies the page as it is, not as text cells show it.
#[test]
fn c_copies_the_page_itself_from_text_cells() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("text-copy", 3);
    let terminal = Terminal { clipboard: Some(true), ..TEXT_ONLY };
    let mut view = Session::start_on(terminal, &pdf, &["--protocol", "blocks"], 100, 60);
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/3"));
    view.press("c");
    view.wait_until("page 1 copied", |t| t.contains("copied page 1, 1224 × 1584 pixels"));
    assert_eq!(png_size(&copied_png(&view.text(), 1)), (1224, 1584));
    view.press("q");
    assert!(view.finish().success());
}

/// A terminal that takes no escape codes is sent none: no question, no pager, no colours; every page
/// printed in characters alone, and it is told why.
#[test]
fn a_dumb_terminal_is_sent_no_escape_codes() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("dumb", 2);
    let terminal = Terminal { term: Some("dumb"), ..PLAIN };
    for args in [&[][..], &["--protocol", "ascii"], &["--protocol", "blocks"]] {
        let mut view = Session::start_on(terminal, &pdf, args, 80, 24);
        assert!(view.finish().success(), "{args:?}");
        let text = view.text();
        assert!(!text.contains('\x1b'), "{args:?}: {text:?}");
        // The two pages' rows, and the warning when it wasn't asked for by name.
        assert!(text.lines().count() > 20, "{args:?}: {text:?}");
        assert_eq!(text.contains("drawing text cells instead"), args.is_empty(), "{args:?}: {text:?}");
    }
}
