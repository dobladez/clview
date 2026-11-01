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
/// answers), and whether `clv` runs over SSH.
#[derive(Clone, Copy)]
struct Terminal<'a> {
    takes_files: bool,
    tmpdir: Option<&'a Path>,
    cell: Option<(u16, u16)>,
    clipboard: Option<bool>,
    ssh: bool,
}

/// One that never reads files, as script(1) doesn't, and doesn't know its cell size.
const PLAIN: Terminal = Terminal { takes_files: false, tmpdir: None, cell: None, clipboard: None, ssh: false };

/// `clv`'s question about the cell size, which a pseudo-terminal without pixels gets.
const CELL_QUERY: &[u8] = b"\x1b[16t\x1b[c";

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
        let mut script = script(&format!("stty rows {rows} cols {cols} && exec {command}"));
        if let Some(dir) = terminal.tmpdir {
            script.env("TMPDIR", dir);
        }
        // On this machine, whatever machine the tests run from: an SSH login would turn the files off.
        for name in ["TMUX", "TMUX_PANE", "SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"] {
            script.env_remove(name);
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
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let (mut scanned, mut asked, mut clip_asked, mut written) = (0, 0, 0, 0);
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

#[test]
fn the_next_screen_of_a_grid_is_in_the_terminal_before_the_key() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("grid", 24);
    // Three to a row in two rows: six pages a screen. The screen and the two after it are 18
    // pages, more than the pager keeps for one page to a screen, and all of them stay.
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--zoom", "3"], 80, 48);
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
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--zoom", "3"], 80, 48);
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
    let mut view = Session::start(&png, &["--pager", "--protocol", "kitty", "--zoom", "4"], 80, 48);
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
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--no-pager", "--zoom", "3"], 80, 24);
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
        png.extend(base64::engine::general_purpose::STANDARD.decode(&data[..data.find("\x1b\\").unwrap()]).unwrap());
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
