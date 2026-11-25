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

use crate::bin::script;

/// `clv`, for a shell line that [`script`] runs: `script` keeps it away from the developer's config.
const BIN: &str = crate::bin::CLV;

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
/// answers), whether `clv` runs over SSH, the `TERM` it names (`None`: whatever the tests got),
/// the background colour it says when asked (the device attributes alone when `None`), the `PATH`
/// `clv` runs with (`None`: fake openers that do nothing, then `/bin` and `/usr/bin`, so that no test
/// ever opens a real desktop app), and whether a desktop session is there (`DISPLAY`).
#[derive(Clone, Copy)]
struct Terminal<'a> {
    takes_files: bool,
    tmpdir: Option<&'a Path>,
    cell: Option<(u16, u16)>,
    clipboard: Option<bool>,
    ssh: bool,
    term: Option<&'a str>,
    background: Option<&'a str>,
    path: Option<&'a str>,
    display: bool,
}

/// One that never reads files, as script(1) doesn't, and doesn't know its cell size.
const PLAIN: Terminal =
    Terminal { takes_files: false, tmpdir: None, cell: None, clipboard: None, ssh: false, term: None, background: None, path: None, display: false };

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
        // Set in the line, not on script: script itself is found on the tests' own.
        let path = terminal.path.map_or_else(|| format!("{}:/bin:/usr/bin", openers("guard", 0).display()), str::to_owned);
        let quoted = path.replace('\'', r"'\''");
        let mut script = script(&format!("stty rows {rows} cols {cols} && export PATH='{quoted}' && {command}"));
        if let Some(dir) = terminal.tmpdir {
            script.env("TMPDIR", dir);
        }
        // On this machine, whatever machine the tests run from: an SSH login would turn the files off.
        for name in [
            "TMUX",
            "TMUX_PANE",
            "SSH_CONNECTION",
            "SSH_CLIENT",
            "SSH_TTY",
            "KITTY_WINDOW_ID",
            "TERM_PROGRAM",
            "COLORTERM",
            "NO_COLOR",
            "DISPLAY",
            "WAYLAND_DISPLAY",
        ] {
            script.env_remove(name);
        }
        if terminal.display {
            script.env("DISPLAY", ":0");
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
pub(crate) fn drawn(text: &str, place: &str) -> bool {
    text.contains(&format!("  {place} "))
}

/// Wait until `said` is all the last status line drawn says, in a window of any width, with nothing
/// of where the reader is beside it; and then until the status line is back without it, which
/// takes about a second from when it was said.
#[cfg(feature = "word")]
fn whole_line_for_a_moment(view: &Session, said: &str) {
    view.wait_until("said, on the whole status line", |t| last_bar(t).trim() == said);
    let since = Instant::now();
    view.wait_until("the status line back", |t| !last_bar(t).contains(said) && last_bar(t).contains(" page "));
    let took = since.elapsed();
    assert!(took > Duration::from_millis(600) && took < Duration::from_secs(4), "the status line came back after {took:?}");
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

/// A file written again while it is shown is said on the status line, black on yellow, and `r`
/// opens it again, at the page the reader was on, the warning gone. A file deleted is said too, and
/// `r` leaves the pages as they are.
#[test]
fn r_reloads_a_file_changed_on_disk_which_the_status_line_says() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("reload", 3);
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 80, 60);
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/3"));
    view.press("n");
    view.wait_until("page 2 shown", |t| drawn(t, "page 2/3"));
    std::fs::write(&pdf, crate::fixtures::letter("Pager test", 5)).unwrap();
    view.wait_until("the change said", |t| t.contains("\x1b[0;1;30;43mchanged on disk, r reloads\x1b[0;7m"));
    let since = |view: &Session, at: usize| view.text().get(at..).unwrap_or_default().to_owned();
    let before = view.text().len();
    view.press("r");
    view.wait_until("the file reloaded, at the same page", |t| drawn(t.get(before..).unwrap_or_default(), "page 2/5"));
    let reloaded = view.text().len();
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!since(&view, reloaded).contains("changed on disk"), "the warning stayed: {:?}", since(&view, reloaded));

    std::fs::remove_file(&pdf).unwrap();
    view.wait_until("the file gone said", |t| t.get(reloaded..).unwrap_or_default().contains("\x1b[0;1;30;43mgone from disk\x1b[0;7m"));
    let gone = view.text().len();
    view.press("r");
    view.wait_until("the pages sent again", |t| transmitted(t.get(gone..).unwrap_or_default()) > 0);
    std::thread::sleep(Duration::from_millis(500));
    let after = since(&view, gone);
    assert!(after.contains("gone from disk\x1b[0;7m") && drawn(&after, "page 2/5") && !after.contains("reloading"), "{after:?}");
    view.press("q");
    assert!(view.finish().success());
}

/// A directory of the test's own, made afresh, holding a one-page PDF for each of `names` (a name
/// ending in `/` is a directory).
fn pdfs(test: &str, names: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("clview-pager-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for name in names {
        let path = dir.join(name);
        match name.strip_suffix('/') {
            Some(_) => std::fs::create_dir_all(&path).unwrap(),
            None => {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, crate::fixtures::letter(name, 1)).unwrap();
            }
        }
    }
    dir
}

/// The warning that what is on screen changed on disk, as the terminal gets it: black on yellow.
const CHANGED: &str = "\x1b[0;1;30;43mchanged on disk, r reloads\x1b[0;7m";

/// A file added to a directory listed is said on the status line, as a file's change is.
#[test]
fn a_list_warns_of_a_file_added_to_its_directory() {
    if !pty_works() {
        return;
    }
    let dir = pdfs("list-added", &["a.pdf", "b.pdf"]);
    let mut view = Session::start(&dir, &["--protocol", "kitty"], 100, 60);
    view.wait_until("the list", |t| drawn(t, "1/2"));
    std::thread::sleep(Duration::from_millis(1200));
    assert!(!view.text().contains("changed on disk"), "said before any change");
    std::fs::write(dir.join("c.pdf"), crate::fixtures::letter("C", 1)).unwrap();
    view.wait_until("the change said", |t| t.contains(CHANGED));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A tile's file written again while the tile is on screen is said, for files named on the command
/// line too, whose directory isn't listed. A file gone is said as gone.
#[test]
fn a_list_warns_of_a_tile_rewritten_on_screen() {
    if !pty_works() {
        return;
    }
    let dir = pdfs("list-rewritten", &["a.pdf", "b.pdf"]);
    let (a, b) = (dir.join("a.pdf"), dir.join("b.pdf"));
    let mut view = Session::start(&a, &[b.to_str().unwrap(), "--protocol", "kitty"], 100, 60);
    view.wait_until("the list", |t| drawn(t, "1/2"));
    std::fs::write(&b, crate::fixtures::letter("B again", 3)).unwrap();
    view.wait_until("the change said", |t| t.contains(CHANGED));
    view.press("q");
    assert!(view.finish().success());
    let mut view = Session::start(&a, &[b.to_str().unwrap(), "--protocol", "kitty"], 100, 60);
    view.wait_until("the list", |t| drawn(t, "1/2"));
    std::fs::remove_file(&b).unwrap();
    view.wait_until("the file gone said", |t| t.contains("\x1b[0;1;30;43mgone from disk\x1b[0;7m"));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// With `-r`, a file added to a subdirectory is said: each directory read is looked at.
#[test]
fn a_list_given_with_r_warns_of_a_file_added_to_a_subdirectory() {
    if !pty_works() {
        return;
    }
    let dir = pdfs("list-recursed", &["a.pdf", "trip/deep/b.pdf"]);
    let mut view = Session::start(&dir, &["-r", "--protocol", "kitty"], 100, 60);
    view.wait_until("the list", |t| drawn(t, "1/2"));
    std::fs::write(dir.join("trip/deep/c.pdf"), crate::fixtures::letter("C", 1)).unwrap();
    view.wait_until("the change said", |t| t.contains(CHANGED));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `r` in a list whose directory changed lists it again: a file added shows at its place, the tile
/// the reader was on still chosen by its name, and the warning gone. A file removed is dropped,
/// and the reader on it goes to the tile now at its place.
#[test]
fn r_lists_a_directory_again_and_keeps_the_tile_by_name() {
    if !pty_works() {
        return;
    }
    let dir = pdfs("relist", &["b.pdf", "d.pdf", "f.pdf"]);
    let mut view = Session::start(&dir, &["--protocol", "kitty"], 100, 60);
    view.wait_until("the list", |t| drawn(t, "1/3"));
    view.press("l");
    view.wait_until("d chosen", |t| last_bar(t).contains(" d.pdf ") && last_bar(t).contains(" 2/3 "));
    std::fs::write(dir.join("a.pdf"), crate::fixtures::letter("A", 1)).unwrap();
    view.wait_until("the change said", |t| t.contains(CHANGED));
    let before = view.text().len();
    view.press("r");
    view.wait_until("listed again, d still chosen", |t| {
        let t = t.get(before..).unwrap_or_default();
        last_bar(t).contains(" d.pdf ") && drawn(t, "3/4") && chosen(t).as_deref() == Some("d.pdf")
    });
    let listed = view.text().len();
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!view.text().get(listed..).unwrap_or_default().contains("changed on disk"), "the warning stayed");
    // d removed: the reader goes to the tile now at its place, f.
    std::fs::remove_file(dir.join("d.pdf")).unwrap();
    view.wait_until("the change said", |t| t.get(listed..).unwrap_or_default().contains(CHANGED));
    let before = view.text().len();
    view.press("r");
    view.wait_until("d dropped, f chosen", |t| {
        let t = t.get(before..).unwrap_or_default();
        last_bar(t).contains(" f.pdf ") && drawn(t, "3/3") && chosen(t).as_deref() == Some("f.pdf")
    });
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `r` keeps a list's filter by its text, and the tile chosen by its name: a file added that
/// matches is among the matches, and Esc puts the whole list back, the same tile chosen.
#[test]
fn r_keeps_a_lists_filter_by_name() {
    if !pty_works() {
        return;
    }
    let dir = pdfs("relist-filter", &["alpha.pdf", "beta.pdf", "alpine.pdf"]);
    let mut view = Session::start(&dir, &["--protocol", "kitty"], 100, 60);
    view.wait_until("the list", |t| drawn(t, "1/3"));
    view.press("/alp\r");
    view.wait_until("filtered", |t| last_bar(t).contains(" /alp ") && last_bar(t).contains(" 1 of 2 "));
    view.press("l");
    view.wait_until("alpine chosen", |t| last_bar(t).contains(" alpine.pdf /alp 2/3 · 2 of 2 "));
    std::fs::write(dir.join("alps.pdf"), crate::fixtures::letter("Alps", 1)).unwrap();
    view.wait_until("the change said", |t| t.contains(CHANGED));
    let before = view.text().len();
    view.press("r");
    view.wait_until("listed again, filtered", |t| last_bar(t.get(before..).unwrap_or_default()).contains(" alpine.pdf /alp 2/4 · 2 of 3 "));
    view.press("\x1b");
    view.wait_until("the whole list", |t| last_bar(t).contains(" alpine.pdf 2/4 ") && !last_bar(t).contains(" of "));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A list made again with nothing left in it keeps its tiles, and says so.
#[test]
fn r_on_a_list_emptied_keeps_its_tiles_and_says_so() {
    if !pty_works() {
        return;
    }
    let dir = pdfs("relist-emptied", &["a.pdf", "b.pdf"]);
    let mut view = Session::start(&dir, &["--protocol", "kitty"], 100, 60);
    view.wait_until("the list", |t| drawn(t, "1/2"));
    std::fs::remove_file(dir.join("a.pdf")).unwrap();
    std::fs::remove_file(dir.join("b.pdf")).unwrap();
    std::fs::write(dir.join("notes.txt"), "plain text\n").unwrap();
    view.wait_until("the change said", |t| t.contains(CHANGED));
    let before = view.text().len();
    view.press("r");
    // The directory's name, long, is cut in its middle.
    view.wait_until("said", |t| {
        let bar = last_bar(t.get(before..).unwrap_or_default());
        bar.contains(" nothing left in clv") && bar.contains(" that clv shows ")
    });
    assert!(last_bar(&view.text()).contains(" 1/2 "), "{}", last_bar(&view.text()));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A file named on the command line is never dropped from its list: gone, its tile stays, as the
/// other files are opened again.
#[test]
fn r_keeps_a_file_named_on_the_command_line_that_is_gone() {
    if !pty_works() {
        return;
    }
    let dir = pdfs("relist-named", &["a.pdf", "b.pdf"]);
    let (a, b) = (dir.join("a.pdf"), dir.join("b.pdf"));
    let mut view = Session::start(&a, &[b.to_str().unwrap(), "--protocol", "kitty"], 100, 60);
    view.wait_until("the list", |t| drawn(t, "1/2"));
    std::fs::remove_file(&b).unwrap();
    std::fs::write(&a, crate::fixtures::letter("A again", 3)).unwrap();
    view.wait_until("the change said", |t| t.contains(CHANGED));
    let before = view.text().len();
    view.press("r");
    view.wait_until("listed again", |t| t.get(before..).is_some_and(|t| drawn(t, "1/2") && transmitted(t) > 0));
    view.press("l");
    view.wait_until("b still there", |t| last_bar(t).contains(" b.pdf 2/2 "));
    view.press("h\r");
    view.wait_until("a opened, as it is now", |t| drawn(t, "page 1/3"));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Going into a tile whose file changed since the list opened it opens the file again first: its
/// page is the file as it is now, with nothing to warn of. A picture is laid out once, so nothing
/// else would read it again.
#[cfg(feature = "image")]
#[test]
fn going_into_a_tile_changed_on_disk_opens_it_again() {
    if !pty_works() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("clview-pager-relist-into-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    save(&dir.join("a.png"), 400, 300);
    save(&dir.join("b.png"), 400, 300);
    let mut view = Session::start(&dir, &["--protocol", "kitty"], 100, 60);
    view.wait_until("the list", |t| drawn(t, "1/2"));
    save(&dir.join("b.png"), 300, 600);
    view.wait_until("the change said", |t| t.contains(CHANGED));
    view.press("l");
    view.wait_until("b chosen", |t| last_bar(t).contains(" b.png "));
    let before = view.text().len();
    view.press("\r");
    view.wait_until("b opened", |t| drawn(t.get(before..).unwrap_or_default(), "page 1/1"));
    let opened = view.text().len();
    std::thread::sleep(Duration::from_millis(1500));
    let since = view.text().get(before..).unwrap_or_default().to_owned();
    let (w, h) = *sent_sizes(&since).last().expect("b's page sent");
    assert!(h > w, "b as it was: {w} × {h}");
    let after = view.text().get(opened..).unwrap_or_default().to_owned();
    assert!(!after.contains("changed on disk") && !last_bar(&view.text()).contains("changed on disk"), "{after:?}");
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `R` turns auto-reload on and off, which the status line says, then shows as `auto` on its right
/// side while it is on, dropped first in a narrow window.
#[test]
fn capital_r_turns_auto_reload_on_and_off() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("auto-toggle", 3);
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 100, 60);
    view.wait_until("page 1", |t| drawn(t, "page 1/3"));
    assert!(!last_bar(&view.text()).contains(" auto "), "{}", last_bar(&view.text()));
    view.press("R");
    view.wait_until("on", |t| last_bar(t).contains(" auto-reload on ") && last_bar(t).contains(" · auto · ? help "));
    view.press("R");
    view.wait_until("off", |t| last_bar(t).contains(" auto-reload off ") && !last_bar(t).contains(" · auto · "));
    view.press("q");
    assert!(view.finish().success());
    // Narrow, `auto` goes before the zoom does.
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--auto-reload"], 40, 60);
    view.wait_until("the first pages", |t| last_bar(t).contains("1–2/3 · 1 per row · ? help "));
    assert!(!last_bar(&view.text()).contains(" auto "), "{}", last_bar(&view.text()));
    view.press("q");
    assert!(view.finish().success());
}

/// With auto-reload on, a file written again over a while is reloaded once its writes pause, with
/// no warning: the last version, at the page the reader was on, the status line saying `reloaded`
/// for a moment.
#[test]
fn auto_reload_reloads_a_file_once_its_writes_pause() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("auto-file", 3);
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--auto-reload"], 100, 60);
    view.wait_until("page 1", |t| drawn(t, "page 1/3"));
    view.press("n");
    view.wait_until("page 2", |t| drawn(t, "page 2/3"));
    let before = view.text().len();
    // Written every 300 ms for 2 s: never still for the 500 ms auto-reload waits.
    for pages in 4..=10 {
        std::fs::write(&pdf, crate::fixtures::letter("Pager test", pages)).unwrap();
        std::thread::sleep(Duration::from_millis(300));
    }
    let written = view.text().len();
    assert!(!view.text().get(before..).unwrap_or_default().contains("page 2/"), "reloaded while it was written: {}", last_bar(&view.text()));
    view.wait_until("reloaded, the last version", |t| {
        let t = t.get(written..).unwrap_or_default();
        drawn(t, "page 2/10") && t.contains(" reloaded ")
    });
    let since = Instant::now();
    view.wait_until("reloaded gone", |t| !last_bar(t).contains(" reloaded ") && last_bar(t).contains(" page 2/10 "));
    let took = since.elapsed();
    assert!(took > Duration::from_millis(800) && took < Duration::from_secs(4), "reloaded went after {took:?}");
    assert!(!view.text().get(before..).unwrap_or_default().contains("changed on disk"), "warned");
    view.press("q");
    assert!(view.finish().success());
}

/// With auto-reload on, a list whose directory changes is made again by itself.
#[test]
fn auto_reload_reloads_a_list() {
    if !pty_works() {
        return;
    }
    let dir = pdfs("auto-list", &["b.pdf", "c.pdf"]);
    let mut view = Session::start(&dir, &["--protocol", "kitty", "--auto-reload"], 100, 60);
    view.wait_until("the list", |t| drawn(t, "1/2"));
    view.press("l");
    view.wait_until("c chosen", |t| last_bar(t).contains(" c.pdf ") && last_bar(t).contains(" 2/2 "));
    let before = view.text().len();
    std::fs::write(dir.join("a.pdf"), crate::fixtures::letter("A", 1)).unwrap();
    view.wait_until("made again, c still chosen", |t| {
        let bar = last_bar(t.get(before..).unwrap_or_default());
        bar.contains(" c.pdf ") && bar.contains(" reloaded ") && bar.contains(" 3/3 ")
    });
    assert!(!view.text().get(before..).unwrap_or_default().contains("changed on disk"), "warned");
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Auto-reload waits while a search is typed, and reloads once it is put away.
#[test]
fn auto_reload_waits_while_a_search_is_typed() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("auto-search", 3);
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--auto-reload"], 100, 60);
    view.wait_until("page 1", |t| drawn(t, "page 1/3"));
    view.press("/Pag");
    view.wait_until("typing", |t| last_bar(t).contains(" /Pag"));
    std::fs::write(&pdf, crate::fixtures::letter("Pager test", 5)).unwrap();
    std::thread::sleep(Duration::from_millis(2500));
    assert!(last_bar(&view.text()).contains(" /Pag") && last_bar(&view.text()).contains("/3 "), "{}", last_bar(&view.text()));
    view.press("\x1b");
    view.wait_until("reloaded", |t| last_bar(t).contains(" reloaded ") && last_bar(t).contains(" page 1/5 "));
    view.press("q");
    assert!(view.finish().success());
}

/// An auto-reload that fails says why, and the change, and isn't tried again by itself until the
/// file changes again.
#[test]
fn auto_reload_tries_a_reload_that_failed_again_only_once_the_file_changes() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("auto-failed", 3);
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--auto-reload"], 100, 60);
    view.wait_until("page 1", |t| drawn(t, "page 1/3"));
    std::fs::write(&pdf, "not a PDF any more\n").unwrap();
    view.wait_until("the failure said", |t| last_bar(t).contains(" not reloaded: "));
    view.wait_until("the change said", |t| t.contains(CHANGED));
    let failed = view.text().len();
    std::thread::sleep(Duration::from_millis(2500));
    let since = view.text().get(failed..).unwrap_or_default().to_owned();
    assert!(!since.contains("reloading") && !since.contains("not reloaded"), "tried again: {since:?}");
    std::fs::write(&pdf, crate::fixtures::letter("Pager test", 5)).unwrap();
    view.wait_until("reloaded", |t| last_bar(t.get(failed..).unwrap_or_default()).contains(" reloaded ") && last_bar(t).contains(" page 1/5 "));
    view.press("q");
    assert!(view.finish().success());
}

/// A config's `auto-reload = true` starts the pager with auto-reload on.
#[test]
fn config_auto_reload_starts_it_on() {
    if !pty_works() {
        return;
    }
    let config = keys_config("config-auto-reload", "auto-reload = true\n");
    let mut view = Session::start(&fixture("config-auto-reload", 3), &["--protocol", "kitty", "--config", &config], 100, 60);
    view.wait_until("page 1, auto on", |t| drawn(t, "page 1/3") && last_bar(t).contains(" · auto · ? help "));
    view.press("q");
    assert!(view.finish().success());
}

/// With `--pages`, a file reloaded keeps the pages picked that it still has, in their order.
#[test]
fn r_reloads_the_pages_picked_that_the_file_still_has() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("reload-picked", 3);
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--pages", "3,1"], 80, 60);
    view.wait_until("page 3 shown", |t| drawn(t, "page 3/3 · 1 of 2"));
    view.press("n");
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/3 · 2 of 2"));
    std::fs::write(&pdf, crate::fixtures::letter("Pager test", 2)).unwrap();
    view.wait_until("the change said", |t| t.contains("changed on disk, r reloads"));
    let before = view.text().len();
    view.press("r");
    view.wait_until("page 1 alone left", |t| drawn(t.get(before..).unwrap_or_default(), "page 1/2 · 1 of 1"));
    view.press("q");
    assert!(view.finish().success());
}

/// A relayout of a file changed on disk reloads it first: the new text shows, and the warning goes.
#[cfg(feature = "word")]
#[test]
fn a_relayout_of_a_file_changed_on_disk_reloads_it() {
    if !pty_works() {
        return;
    }
    let path = long_markdown("relayout-reload", "long.md");
    let mut view = Session::start_on(KITTY, &path, &["--pager"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/"));
    view.press("/with\r");
    view.wait_until("every paragraph found", |t| last_bar(t).contains(" /with 1/60 "));
    let half: String = (1..=60).map(|n| format!("Paragraph {n}, {} other words.\n\n", if n % 2 == 0 { "with" } else { "sans" })).collect();
    std::fs::write(&path, format!("# Long\n\n{half}")).unwrap();
    view.wait_until("the change said", |t| t.contains("changed on disk, r reloads"));
    view.press("40]");
    view.wait_until("laid out again from the new text", |t| bar(t).contains("· width 40 ·") && last_bar(t).contains(" /with 30 "));
    let laid = view.text().len();
    std::thread::sleep(Duration::from_millis(1500));
    let since = view.text().get(laid..).unwrap_or_default().to_owned();
    assert!(!last_bar(&view.text()).contains("changed on disk") && !since.contains("changed on disk"), "the warning stayed: {since:?}");
    view.press("q");
    assert!(view.finish().success());
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
    assert!(!view.text().contains(": g page"), "a reply typed a page number");
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

/// `d` and Ctrl-V go a screen down and `u` and Alt-V a screen up, as Space and `b` do, and Ctrl-N and
/// Ctrl-P are `j` and `k`; in a list Ctrl-F, Ctrl-B, Ctrl-N and Ctrl-P go from tile to tile as `l`,
/// `h`, `j` and `k` do, and `d` and `u` a screen of tiles. `u` goes back nowhere, in a file or in a
/// list: Backspace does. Alt-V is an ESC and `v` written together, as a terminal that sends Alt as
/// ESC writes them; an Alt key bound to nothing does nothing, where an Esc would leave the file.
#[test]
fn d_u_and_emacss_keys_page_and_move_and_u_goes_back_nowhere() {
    if !pty_works() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("clview-pager-emacs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, pages) in [("a.pdf", 4), ("b.pdf", 1), ("c.pdf", 1), ("e.pdf", 1)] {
        std::fs::write(dir.join(name), crate::fixtures::letter(name, pages)).unwrap();
    }
    // A tall window: a page to the screen once a file is opened. Two tiles to a row: a b, c e.
    let mut view = Session::start(&dir, &["--protocol", "kitty", "--per-row", "2"], 100, 60);
    let tile = |view: &Session, what: &str, name: &str, at: usize| {
        view.wait_until(what, |t| last_bar(t).contains(&format!(" {name} ")) && last_bar(t).contains(&format!(" {at}/4 ")));
    };
    view.wait_until("the list", |t| bar(t).contains(" a.pdf ") && drawn(t, "1/4"));
    for (key, name, at) in [("\x06", "b.pdf", 2), ("\x0e", "e.pdf", 4), ("\x02", "c.pdf", 3), ("\x10", "a.pdf", 1)] {
        view.press(key);
        tile(&view, &format!("{key:?} in the list"), name, at);
    }
    // A screen of tiles down and up: off the first tile, and back to it.
    for (down, up) in [("d", "u"), ("\x16", "\x1bv")] {
        view.press(down);
        view.wait_until("a screen down", |t| !last_bar(t).contains(" a.pdf ") && last_bar(t).contains("/4 "));
        view.press(up);
        tile(&view, &format!("{up:?} in the list"), "a.pdf", 1);
    }
    // Into a.pdf: a screen down and up is a page on and back.
    view.press("\r");
    let page = |view: &Session, what: &str, page: usize| view.wait_until(what, |t| last_bar(t).contains(&format!(" page {page}/4 ")));
    page(&view, "a.pdf opened", 1);
    for (key, at) in [("d", 2), ("\x16", 3), ("u", 2), ("\x1bv", 1), ("\x0e", 2), ("\x10", 1)] {
        view.press(key);
        page(&view, &format!("{key:?} in the file"), at);
    }
    // `u` at the first page stays in the file, and so does an Alt key bound to nothing (Alt-Q):
    // the next page is still this file's.
    view.press("u");
    std::thread::sleep(Duration::from_millis(200));
    view.press("\x1bq");
    std::thread::sleep(Duration::from_millis(200));
    assert!(view.child.try_wait().unwrap().is_none(), "`u` or Alt-Q quit");
    assert!(last_bar(&view.text()).contains(" page 1/4 "), "`u` or Alt-Q left the file: {}", last_bar(&view.text()));
    view.press("n");
    page(&view, "still in the file", 2);
    // Backspace is what goes back; in the first list, `u` goes nowhere and quits nothing.
    view.press("\x7f");
    tile(&view, "back to the list", "a.pdf", 1);
    view.press("u");
    std::thread::sleep(Duration::from_millis(200));
    assert!(view.child.try_wait().unwrap().is_none(), "`u` quit");
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A directory's subdirectories are folders to go into: Enter lists one, its files named from
/// the first directory on the status line, and Backspace goes back; at the top, where
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
    view.press("\x7f");
    view.wait_until("up", |t| bar(t).contains(" trip/ 3/3 ") && chosen(t).as_deref() == Some("trip/"));
    view.press("\x7fg");
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

/// The processes descended from `pid` whose command line holds `what`, by `ps`.
pub(crate) fn descendants_running(pid: u32, what: &str) -> Vec<u32> {
    let ps = Command::new("ps").args(["-axo", "pid=,ppid=,command="]).output().unwrap();
    let rows: Vec<(u32, u32, String)> = String::from_utf8_lossy(&ps.stdout)
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let (pid, ppid) = (words.next()?.parse().ok()?, words.next()?.parse().ok()?);
            Some((pid, ppid, words.collect::<Vec<_>>().join(" ")))
        })
        .collect();
    let mut family = vec![pid];
    let mut found = Vec::new();
    while let Some(parent) = family.pop() {
        for (pid, _, command) in rows.iter().filter(|(_, ppid, _)| *ppid == parent) {
            family.push(*pid);
            if command.contains(what) {
                found.push(*pid);
            }
        }
    }
    found
}

/// A list's tiles start no worker for a drawing's HTML labels, and warn of nothing about them;
/// opening a drawing from the list draws its labels in a worker of its own, which goes when the
/// reader comes back to the list.
#[cfg(feature = "svg-html")]
#[test]
fn a_lists_tiles_start_no_worker_for_a_drawings_labels_and_opening_one_does() {
    if !pty_works() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("clview-pager-labels-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["a.svg", "b.svg"] {
        let drawing = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><foreignObject width="40" height="20">
            <div xmlns="http://www.w3.org/1999/xhtml" style="width:100%;height:100%;background:blue"/></foreignObject></svg>"#;
        std::fs::write(dir.join(name), drawing).unwrap();
    }
    let mut view = Session::start(&dir, &["--protocol", "kitty"], 100, 48);
    view.wait_until("the list", |t| bar(t).contains(" a.svg ") && drawn(t, "1/2"));
    view.wait_until("both tiles", |t| transmitted(t) == 2);
    let pid = view.child.id();
    let workers = || descendants_running(pid, "--worker svg-html");
    assert!(workers().is_empty(), "a worker for a tile: {:?}", workers());
    assert!(!view.text().contains("HTML label"), "{}", view.text());
    view.press("\r");
    view.wait_until("a opened", |t| bar(t).contains(" a.svg ") && drawn(t, "page 1/1"));
    assert_eq!(workers().len(), 1, "the drawing's worker");
    let before = view.text().len();
    view.press("\x1b");
    view.wait_until("back to the list", |t| t.get(before..).is_some_and(|t| drawn(t, "1/2")));
    let deadline = Instant::now() + PATIENCE;
    while !workers().is_empty() {
        assert!(Instant::now() < deadline, "the worker outlived the drawing's level");
        std::thread::sleep(Duration::from_millis(20));
    }
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The Mermaid workers among `view`'s processes, while they live: a worker draws a diagram, and lives
/// 2 s past its last answer, so one that drew a diagram just now is still among them.
#[cfg(feature = "mermaid")]
fn mermaid_workers(view: &Session) -> Vec<u32> {
    descendants_running(view.child.id(), "--worker mermaid")
}

/// Wait until no Mermaid worker is left among `view`'s processes: gone once idle.
#[cfg(feature = "mermaid")]
fn no_mermaid_worker(view: &Session, why: &str) {
    let deadline = Instant::now() + PATIENCE;
    while !mermaid_workers(view).is_empty() {
        assert!(Instant::now() < deadline, "{why}: the worker outlived its idle time");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A list's diagram is turned into SVG only once its tile is wanted: no worker while the tiles
/// wanted are PDFs, then one for the diagram's, gone once idle. Opened from the list and copied, it is
/// what was drawn for its tile: no worker again.
#[cfg(feature = "mermaid")]
#[test]
fn a_lists_diagram_starts_a_worker_only_when_its_tile_is_wanted() {
    if !pty_works() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("clview-pager-mermaid-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for n in 0..12 {
        std::fs::write(dir.join(format!("a{n:02}.pdf")), crate::fixtures::letter("Before", 1)).unwrap();
    }
    let flow = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/flow.mmd");
    std::fs::copy(flow, dir.join("z.mmd")).unwrap();
    let terminal = Terminal { clipboard: Some(true), ..PLAIN };
    let mut view = Session::start_on(terminal, &dir, &["--protocol", "kitty", "--per-row", "1"], 100, 48);
    view.wait_until("the list", |t| bar(t).contains(" a00.pdf ") && drawn(t, "1/13"));
    std::thread::sleep(Duration::from_millis(500));
    assert!(mermaid_workers(&view).is_empty(), "a worker before the diagram's tile is wanted");
    view.press("G");
    let mut seen = std::collections::BTreeSet::new();
    let deadline = Instant::now() + PATIENCE;
    while !drawn(&view.text(), "13/13") {
        assert!(Instant::now() < deadline, "the diagram's tile isn't drawn");
        seen.extend(mermaid_workers(&view));
        std::thread::sleep(Duration::from_millis(20));
    }
    seen.extend(mermaid_workers(&view));
    assert_eq!(seen.len(), 1, "one worker for the diagram's tile: {seen:?}");
    no_mermaid_worker(&view, "the tile drawn");
    let before = view.text().len();
    view.press("\r");
    view.wait_until("z opened", |t| t.get(before..).is_some_and(|t| drawn(t, "page 1/1")));
    view.press("c");
    view.wait_until("copied", |t| t.contains("copied z.mmd, "));
    assert!(mermaid_workers(&view).is_empty(), "the diagram drawn again for its page or its copy");
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Whether `text` has a status line saying the pages on screen with nothing left to render:
/// `  page 2/3 ` or `  pages 1–2/5 `, not `… page 2/3`.
#[cfg(all(feature = "mermaid", feature = "word"))]
fn rendered(text: &str) -> bool {
    ["    page ", "    pages "].iter().any(|start| {
        text.match_indices(start).any(|(at, found)| {
            let rest = &text[at + found.len()..];
            let place: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '/' || *c == '–').collect();
            place.contains('/') && rest[place.len()..].starts_with(' ')
        })
    })
}

/// A Markdown document's diagram is turned into SVG once: rendered page after page, and laid out
/// again at another width, the document finds it drawn, and starts no worker.
#[cfg(all(feature = "mermaid", feature = "word"))]
#[test]
fn a_documents_diagram_is_turned_into_svg_once_whatever_it_is_laid_out_at() {
    drawn_once("flow.md", "![a flow](flow.mmd)", "![the flow again](flow.mmd)");
}

/// So is a ` ```mermaid ` block's, which the document hands to Mermaid's backend again every time it
/// is read: to open, for its first render, and for each layout.
#[cfg(all(feature = "mermaid", feature = "word"))]
#[test]
fn a_documents_mermaid_block_is_turned_into_svg_once_whatever_it_is_laid_out_at() {
    let flow = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/flow.mmd")).unwrap();
    let block = format!("```mermaid\n{flow}```");
    drawn_once("blocks.md", &block, &block);
}

/// A Markdown document named `name`, showing `flow.mmd`'s diagram as `first` says and, pages later,
/// as `again` does, read in the pager: once page 1 is drawn and Mermaid's worker gone, no step
/// through its pages or to another width starts one.
#[cfg(all(feature = "mermaid", feature = "word"))]
fn drawn_once(name: &str, first: &str, again: &str) {
    if !pty_works() {
        return;
    }
    let path = crate::view::tmp(&format!("pager-mermaid-{name}"), name);
    let flow = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/flow.mmd");
    std::fs::copy(flow, path.with_file_name("flow.mmd")).unwrap();
    let words: String = (1..=40).map(|n| format!("Paragraph {n}, with words enough in it to fill a line or two of a page.\n\n")).collect();
    std::fs::write(&path, format!("# A flow\n\n{first}\n\n{words}{again}\n")).unwrap();
    let mut view = Session::start_on(KITTY, &path, &["--pager"], 100, 40);
    view.wait_until("page 1", |t| t.contains("  page 1/"));
    no_mermaid_worker(&view, "page 1 drawn");
    // Each step's own words on the status line, then a page with nothing left to render.
    for (keys, said) in [("n", "page 2/"), ("n", "page 3/"), ("40]", "width 40"), ("g", "pages 1–2/"), ("]", "width 44")] {
        let before = view.text().len();
        view.press(keys);
        view.wait_until(keys, |t| t.get(before..).is_some_and(|t| t.contains(said) && rendered(t)));
        assert!(mermaid_workers(&view).is_empty(), "{keys}: the diagram turned into SVG again");
    }
    view.press("q");
    assert!(view.finish().success());
}

/// A Markdown file named `name` whose ` ```mermaid ` block is `flow.mmd`'s diagram, a red node and a
/// blue one, and no word of the diagram's anywhere else.
#[cfg(all(feature = "mermaid", feature = "word"))]
fn flow_document(test: &str, name: &str) -> PathBuf {
    let flow = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/flow.mmd")).unwrap();
    let path = crate::view::tmp(test, name);
    std::fs::write(&path, format!("# A diagram\n\nBefore it.\n\n```mermaid\n{flow}```\n\nAfter it.\n")).unwrap();
    path
}

/// Whether `page` shows `flow.mmd`'s diagram drawn: its red node and its blue one.
#[cfg(all(feature = "mermaid", feature = "word"))]
fn shows_flow(page: &::image::RgbImage) -> bool {
    let has = |colour: [u8; 3]| page.pixels().any(|pixel| pixel.0.iter().zip(colour).all(|(a, b)| a.abs_diff(b) < 40));
    has([255, 0, 0]) && has([0, 0, 255])
}

/// `s` shows a document's fenced blocks as the code they are, the document laid out again and the
/// status line saying so, and again draws them; `?` lists it; and `--fences code` starts so.
#[cfg(all(feature = "mermaid", feature = "word"))]
#[test]
fn s_shows_a_documents_blocks_as_code_and_draws_them_again() {
    if !pty_works() {
        return;
    }
    let path = flow_document("pager-fences", "flow.md");
    let mut view = Session::start_on(KITTY, &path, &["--pager"], 100, 40);
    let (drawn, page) = next_image(&view, "the block drawn", 0);
    assert!(shows_flow(&page), "drawn to start with");
    assert!(!bar(&view.text()).contains("blocks"), "nothing said of what is as it starts");
    view.press("s");
    let (coded, page) = next_image(&view, "the block as code", drawn);
    assert!(!shows_flow(&page));
    view.wait_until("said", |t| last_bar(t).contains("· blocks as code ·"));
    view.press("s");
    let (_, page) = next_image(&view, "the block drawn again", coded);
    assert!(shows_flow(&page));
    view.wait_until("said", |t| last_bar(t).contains(" blocks drawn "));
    view.press("?");
    view.wait_until("listed", |t| bar(t).contains(" s fenced blocks drawn, or as code (Markdown, HTML and others)"));
    // Any key leaves the keys; the next one quits.
    view.press("qq");
    assert!(view.finish().success());

    let mut view = Session::start_on(KITTY, &path, &["--pager", "--fences", "code"], 100, 40);
    let (coded, page) = next_image(&view, "as code to start with", 0);
    assert!(!shows_flow(&page));
    view.wait_until("said", |t| last_bar(t).contains("· blocks as code ·"));
    view.press("s");
    assert!(shows_flow(&next_image(&view, "drawn", coded).1));
    view.press("q");
    assert!(view.finish().success());
}

/// On what has no fenced blocks, `s` says so and nothing is sent again; in a list it says to open a
/// document, the tiles as they were; and a document gone into from the list takes it.
#[cfg(all(feature = "mermaid", feature = "word"))]
#[test]
fn s_on_a_pdf_says_only_markdown_and_html_draw_their_blocks_and_a_list_says_to_open_one() {
    if !pty_works() {
        return;
    }
    let mut view = Session::start_on(KITTY, &fixture("no-fences", 1), &["--pager"], 80, 40);
    view.wait_until("page 1", |t| drawn(t, "page 1/1"));
    view.press("s");
    whole_line_for_a_moment(&view, "this format doesn't draw fenced blocks (Markdown, HTML and others do)");
    assert_eq!(transmitted(&view.text()), 1);
    view.press("q");
    assert!(view.finish().success());

    let dir = crate::view::tmp("fences-list", "shelf");
    std::fs::create_dir_all(&dir).unwrap();
    flow_document("fences-list", "shelf/a.md");
    std::fs::copy(fixture("fences-list", 1), dir.join("b.pdf")).unwrap();
    let mut view = Session::start_on(KITTY, &dir, &[], 100, 40);
    view.wait_until("both tiles", |t| drawn(t, "1/2 · 2 per row"));
    let tiles = pngs(&view.text());
    assert!(tiles.len() == 2 && tiles.iter().any(|(_, tile)| shows_flow(tile)), "the document's tile shows its block drawn");
    view.press("s");
    view.wait_until("said", |t| bar(t).contains("open a document to reflow it"));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(pngs(&view.text()).len(), 2, "the tiles are as they were");
    let shown = ids_after(&view.text(), PLACED).last().copied().unwrap();
    view.press("\r");
    view.wait_until("a opened", |t| bar(t).contains(" a.md "));
    let (opened, page) = next_image(&view, "its page", shown);
    assert!(shows_flow(&page));
    view.press("s");
    assert!(!shows_flow(&next_image(&view, "its block as code", opened).1));
    view.press("\x7f");
    view.wait_until("back in the list", |t| bar(last_draw(t)).contains(" 1/2 ·"));
    view.press("\x1b[C");
    view.wait_until("b chosen", |t| chosen(t).as_deref() == Some("b.pdf"));
    view.press("\r");
    view.wait_until("b opened", |t| bar(last_draw(t)).contains(" b.pdf "));
    view.press("s");
    view.wait_until("said", |t| last_bar(t).trim() == "this format doesn't draw fenced blocks (Markdown, HTML and others do)");
    view.press("q");
    assert!(view.finish().success());
}

/// A block drawn is a picture, with no text for a search; shown as code, its text is the
/// document's. A search kept across `s` starts over on the text as it is then: it counts what the
/// code says, and nothing once the block is drawn again. So too where the document has as many
/// paragraphs either way, a block of one line being one paragraph as its picture is: the text is
/// told apart by the blocks drawn, not by its paragraphs' count.
#[cfg(all(feature = "mermaid", feature = "word"))]
#[test]
fn a_search_reads_a_block_shown_as_code_and_not_one_drawn() {
    if !pty_works() {
        return;
    }
    let one_line = crate::view::tmp("fences-search", "line.md");
    std::fs::write(&one_line, "# A diagram\n\nBefore it.\n\n```mermaid\nflowchart LR; start --> finish\n```\n\nAfter it.\n").unwrap();
    for (path, in_code) in [(flow_document("fences-search", "flow.md"), " /finish 3 "), (one_line, " /finish 1 ")] {
        let mut view = Session::start_on(KITTY, &path, &["--pager"], 100, 40);
        view.wait_until("page 1", |t| drawn(t, "page 1/1"));
        view.press("/finish\r");
        view.wait_until("no text of the block's", |t| last_bar(t).contains(" /finish no match "));
        view.press("s");
        view.wait_until("its code searched", |t| last_bar(t).contains("blocks as code") && last_bar(t).contains(in_code));
        view.press("s");
        view.wait_until("drawn again, no text", |t| last_bar(t).contains(" /finish no match ") && !last_bar(t).contains("blocks as code"));
        view.press("q");
        assert!(view.finish().success());
    }
}

/// `/` searches an Org document's body as it is shown: not what Org hides (a drawer, a comment),
/// nor a source block drawn as a picture, which `s` shows as its code, searched then.
#[cfg(all(feature = "mermaid", feature = "org"))]
#[test]
fn slash_searches_an_org_documents_body_and_s_its_blocks_as_code() {
    if !pty_works() {
        return;
    }
    let flow = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/flow.mmd")).unwrap();
    let path = crate::view::tmp("org-search", "notes.org");
    std::fs::write(
        &path,
        format!(
            "* TODO A heading\n:LOGBOOK:\n- a secret note\n:END:\n# a secret comment\nThe word findme, once.\n#+begin_src mermaid\n{flow}#+end_src\n"
        ),
    )
    .unwrap();
    let mut view = Session::start_on(KITTY, &path, &["--pager"], 100, 40);
    let (drawn, page) = next_image(&view, "the block drawn", 0);
    assert!(shows_flow(&page), "drawn to start with");
    view.press("/findme\r");
    view.wait_until("found", |t| last_bar(t).contains(" /findme 1/1 ") || last_bar(t).contains(" /findme 1 "));
    // The page is sent again tinted, and again as it was once the search is gone: `s` is waited
    // past the last of them.
    let (tinted, _) = next_image(&view, "the match tinted", drawn);
    view.press("\x1b");
    view.wait_until("the search gone", |t| !last_bar(t).contains("/findme"));
    let (shown, page) = next_image(&view, "the tint taken off", tinted);
    assert!(shows_flow(&page), "drawn still");
    view.press("/secret\r");
    view.wait_until("nothing hidden searched", |t| last_bar(t).contains(" /secret no match "));
    view.press("\x1b");
    view.wait_until("the search gone", |t| !last_bar(t).contains("/secret"));
    view.press("/finish\r");
    view.wait_until("no text of the block's", |t| last_bar(t).contains(" /finish no match "));
    view.press("s");
    let (_, page) = next_image(&view, "the block as code", shown);
    assert!(!shows_flow(&page));
    view.wait_until("its code searched", |t| {
        last_bar(t).contains("blocks as code") && last_bar(t).contains(" /finish ") && !last_bar(t).contains("no match")
    });
    view.press("q");
    assert!(view.finish().success());
}

/// `D` on a document with a diagram draws its block again for the paper: in Mermaid's dark theme on
/// a dark terminal's own colour, nothing white behind it, then in the default theme on white, and
/// back. The diagram is laid out once a theme: turned to a paper it was on before, no worker starts.
#[cfg(all(feature = "mermaid", feature = "word"))]
#[test]
fn d_draws_a_documents_block_again_for_its_paper_and_lays_the_diagram_out_once_a_theme() {
    if !pty_works() {
        return;
    }
    let path = crate::view::tmp("pager-paper", "flow.md");
    std::fs::write(&path, "```mermaid\nsequenceDiagram\n  Alice->>Bob: Hello\n  Bob-->>Alice: Hi\n```\n").unwrap();
    let on_dark = |page: &::image::RgbImage| {
        let white = page.pixels().filter(|pixel| pixel.0.iter().all(|v| *v > 240)).count();
        let light_ink = page.pixels().filter(|pixel| pixel.0.iter().all(|v| *v > 150)).count();
        page.get_pixel(0, 0).0 == DARK_COLOUR && white == 0 && light_ink > 200
    };
    let on_light = |page: &::image::RgbImage| {
        let dark_ink = page.pixels().filter(|pixel| pixel.0.iter().all(|v| *v < 128)).count();
        page.get_pixel(0, 0).0 == [255; 3] && dark_ink > 200
    };
    let mut view = Session::start_on(Terminal { background: Some(DARK), ..KITTY }, &path, &["--pager"], 100, 40);
    let (mut shown, page) = next_image(&view, "on the terminal's dark", 0);
    assert!(on_dark(&page), "in the dark theme, on the terminal's colour");
    no_mermaid_worker(&view, "both themes laid out");
    for (turn, dark) in [(1, false), (2, true), (3, false)] {
        view.press("D");
        let (next, page) = next_image(&view, "turned", shown);
        assert!(if dark { on_dark(&page) } else { on_light(&page) }, "turn {turn}");
        assert!(mermaid_workers(&view).is_empty(), "turn {turn}: the diagram laid out again");
        shown = next;
    }
    view.press("q");
    assert!(view.finish().success());
}

/// What the status line last said, each run of spaces made one: from the last time it was written,
/// with the screen or by itself.
pub(crate) fn last_bar(text: &str) -> String {
    const START: &str = "\x1b[2K\x1b[7m";
    let said = text.rfind(START).map_or("", |at| &text[at + START.len()..]);
    bar(said.find("\x1b[0m").map_or("", |end| &said[..end]))
}

/// Wait until no worker reading a PDF's text is left among `view`'s processes.
fn no_text_worker(view: &Session, why: &str) {
    let deadline = Instant::now() + PATIENCE;
    while !descendants_running(view.child.id(), "--worker pdf-text").is_empty() {
        assert!(Instant::now() < deadline, "{why}: a worker is still reading the text");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `/` in a PDF's pages searches its text, read in a worker process from then on: what is typed is
/// counted while the view stays, Enter goes to the first match from the top of the screen, `n`
/// goes on and `N` or `p` back, round the ends, and Esc clears the search, after which `n` and `p`
/// turn pages again. The worker is started by `/`, and goes with the search, and with the pager.
#[test]
fn slash_searches_a_pdfs_text_in_a_worker_that_goes_with_the_search() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("search", 5);
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 100, 60);
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/5"));
    let workers = |view: &Session| descendants_running(view.child.id(), "--worker pdf-text");
    assert!(workers(&view).is_empty(), "a worker before any search");
    view.press("/page 3");
    view.wait_until("counted as typed", |t| last_bar(t).contains(" many.pdf /page 3 1 ⏎ search · esc cancel page 1/5 "));
    assert_eq!(workers(&view).len(), 1, "the text's worker");
    view.press("\r");
    view.wait_until("the match's page", |t| last_bar(t).contains(" many.pdf /page 3 1/1 page 3/5 "));
    // Every page says "Pager test, page N": in any case, five matches, from the top of the screen.
    view.press("/\x7f\x7f\x7f\x7f\x7f\x7fPAGER TEST\r");
    view.wait_until("the first from page 3", |t| last_bar(t).contains(" /PAGER TEST 3/5 page 3/5 "));
    view.press("nn");
    view.wait_until("two on", |t| last_bar(t).contains(" /PAGER TEST 5/5 page 5/5 "));
    view.press("n");
    view.wait_until("round to the first", |t| last_bar(t).contains(" /PAGER TEST 1/5 · search wrapped page 1/5 "));
    view.press("N");
    view.wait_until("and back", |t| last_bar(t).contains(" /PAGER TEST 5/5 · search wrapped page 5/5 "));
    view.press("2N");
    view.wait_until("two back", |t| last_bar(t).contains(" /PAGER TEST 3/5 page 3/5 "));
    // `p` is `N`: the match before, and with a number that many back, round the start.
    view.press("p");
    view.wait_until("one back", |t| last_bar(t).contains(" /PAGER TEST 2/5 page 2/5 "));
    view.press("3p");
    view.wait_until("three back, round", |t| last_bar(t).contains(" /PAGER TEST 4/5 · search wrapped page 4/5 "));
    view.press("p");
    view.wait_until("one more back", |t| last_bar(t).contains(" /PAGER TEST 3/5 page 3/5 "));
    assert_eq!(workers(&view).len(), 1, "one worker for the whole search");
    // Esc clears the search: the worker goes, the view stays, and `n` and `p` turn pages again.
    view.press("\x1b");
    view.wait_until("cleared", |t| !last_bar(t).contains("/PAGER") && last_bar(t).contains(" many.pdf page 3/5 "));
    no_text_worker(&view, "the search was cleared");
    view.press("p");
    view.wait_until("the page before", |t| last_bar(t).contains(" many.pdf page 2/5 "));
    view.press("nn");
    view.wait_until("two pages on", |t| last_bar(t).contains(" many.pdf page 4/5 "));
    // Nothing matches: said once all is read. And quitting with a search open leaves no worker:
    // looked for by its own id, since one that outlived the pager would be nobody's descendant.
    view.press("/zebra");
    view.wait_until("no match", |t| last_bar(t).contains(" /zebra no match ⏎ search · esc cancel page 4/5 "));
    let open = workers(&view);
    assert_eq!(open.len(), 1, "the worker of the search still open");
    view.press("\x03");
    assert!(view.finish().success());
    let deadline = Instant::now() + PATIENCE;
    while open.iter().any(|pid| still_a_text_worker(*pid)) {
        assert!(Instant::now() < deadline, "the worker outlived the pager");
        std::thread::sleep(Duration::from_millis(20));
    }
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// Whether process `pid` is, still, a worker reading a PDF's text: asked of `ps` by its id, so a
/// worker whose viewer is gone, and that is no longer anyone's descendant, is found all the same.
fn still_a_text_worker(pid: u32) -> bool {
    let ps = Command::new("ps").args(["-p", &pid.to_string(), "-o", "command="]).output().unwrap();
    String::from_utf8_lossy(&ps.stdout).contains("--worker pdf-text")
}

/// Quitting while the worker is reading a page kills it with the pager: it isn't left to finish
/// the page, or to run into its cap, with nobody to answer. The page is a heavy one, which takes
/// the worker a second, and the worker is looked for by its own id once the pager has gone. The
/// shell that ran `clv` stays a while after it, as a reader's does: a terminal that closed with
/// the pager would hang the worker up, and hide one left behind.
#[test]
fn quitting_while_a_pages_text_is_read_leaves_no_worker() {
    if !pty_works() {
        return;
    }
    let pdf = with_heavy_pages("search-heavy-quit", 2, &[0]);
    let command = format!("'{BIN}' '{}' --protocol kitty --per-row 1; sleep 2", pdf.display());
    let mut view = Session::start_command(PLAIN, &command, 160, 60);
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/2"));
    view.press("/page");
    let deadline = Instant::now() + PATIENCE;
    let reading = loop {
        let found = descendants_running(view.child.id(), "--worker pdf-text");
        if !found.is_empty() {
            break found;
        }
        assert!(Instant::now() < deadline, "no worker for the search");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(reading.iter().all(|pid| still_a_text_worker(*pid)), "found by its id while it reads");
    view.press("\x03");
    view.wait_until("the pager gone", |t| t.contains("\x1b[?1049l"));
    let gone = Instant::now();
    while reading.iter().any(|pid| still_a_text_worker(*pid)) {
        assert!(gone.elapsed() < Duration::from_millis(400), "the worker outlived the pager, still reading");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// A US Letter document whose pages read `Kill test, page N`, but those of `heavy` (from 0), which
/// hold four million glyphs each, ten to a string, every string placed by itself: more than the
/// text's worker has the heap to read (such a page took 896 MiB to read, measured, against the
/// worker's 768 MiB, the file included: ADR-0027).
fn with_heavy_pages(test: &str, pages: usize, heavy: &[usize]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("clview-pager-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut glyphs = String::from("BT /F1 0.1 Tf\n");
    for i in 0..400_000u32 {
        glyphs.push_str(&format!("1 0 0 1 {:.2} {:.2} Tm (abcdefghij) Tj\n", 5.0 + f64::from(i % 1000) * 0.6, 780.0 - f64::from(i / 1000) * 0.12));
    }
    glyphs.push_str("ET");
    // The catalog, the page tree, the font and the heavy pages' one content; then each page, and
    // its own content if it is a light one.
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 5 + 2 * i)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {pages} /MediaBox [0 0 612 792] >>", kids.join(" ")),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        format!("<< /Length {} >>\nstream\n{glyphs}\nendstream", glyphs.len()),
    ];
    for i in 0..pages {
        let content = format!("BT /F1 36 Tf 72 680 Td (Kill test, page {}) Tj ET", i + 1);
        let contents = if heavy.contains(&i) { 4 } else { 6 + 2 * i };
        objects.push(format!("<< /Type /Page /Parent 2 0 R /Contents {contents} 0 R /Resources << /Font << /F1 3 0 R >> >> >>"));
        objects.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
    }
    let pdf = dir.join("heavy.pdf");
    std::fs::write(&pdf, crate::fixtures::pdf(&objects)).unwrap();
    pdf
}

/// A page whose text takes more memory to read than its worker has kills the worker, and costs the
/// search that page alone: the status line says which, and why, and another worker reads the pages
/// after it. Past three workers killed, the rest is left unread, and the status line says how far
/// the reading got; no worker is left. The keys answer throughout.
#[test]
fn a_page_past_the_workers_heap_costs_the_search_that_page_and_four_of_them_the_rest() {
    if !pty_works() {
        return;
    }
    let pdf = with_heavy_pages("search-heavy-one", 3, &[1]);
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--per-row", "1"], 160, 60);
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/3"));
    view.press("/page\r");
    view.wait_until("the pages around it read", |t| last_bar(t).contains(" /page 1/2 · page 2 unread "));
    assert!(view.text().contains("page 2's text took too much memory to read"), "why, said once: {}", last_bar(&view.text()));
    // The page is still shown, and turned to; only its text is unread.
    view.press("n");
    view.wait_until("the match after the page unread", |t| last_bar(t).contains(" /page 2/2 · page 2 unread ") && last_bar(t).contains(" page 3/3 "));
    view.press("\x7fq");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();

    // Pages 2, 4, 5 and 6: read outward from page 1, the fourth worker killed is one too many.
    let pdf = with_heavy_pages("search-heavy-four", 8, &[1, 3, 4, 5]);
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--per-row", "1"], 160, 60);
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/8"));
    view.press("/page\r");
    view.wait_until("the first page's match", |t| last_bar(t).contains(" /page 1/1… "));
    // A key meanwhile is answered: the reading is nobody's wait.
    view.press("\x1b[C");
    view.wait_until("a page turned while the text is read", |t| last_bar(t).contains("… ") && last_bar(t).contains(" page 2/8 "));
    view.wait_until("the rest left unread", |t| last_bar(t).contains(" /page 1/2 · read 2 of 8 pages "));
    assert!(view.text().contains("reading its text failed too often: the rest is left unread"));
    no_text_worker(&view, "the reading was given up");
    view.press("\x7fq");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();

    // Esc while the worker is on a heavy page, the first read: it is killed at once, not left to
    // run into its cap, which takes it a second.
    let pdf = with_heavy_pages("search-heavy-left", 2, &[0]);
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--per-row", "1"], 160, 60);
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/2"));
    view.press("/page");
    let deadline = Instant::now() + PATIENCE;
    while descendants_running(view.child.id(), "--worker pdf-text").is_empty() {
        assert!(Instant::now() < deadline, "no worker for the search");
        std::thread::sleep(Duration::from_millis(5));
    }
    view.press("\x1b");
    let left = Instant::now();
    while !descendants_running(view.child.id(), "--worker pdf-text").is_empty() {
        assert!(left.elapsed() < Duration::from_millis(500), "the worker outlived the search it was reading for");
        std::thread::sleep(Duration::from_millis(5));
    }
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// Every PNG transmitted whole in `text`, in order: its image id and its pixels. Its chunks may
/// have other escape codes between them, as tmux writes its own between two passthroughs, but no
/// other graphics command; one cut short is left out.
pub(crate) fn pngs(text: &str) -> Vec<(u32, ::image::RgbImage)> {
    use base64::Engine as _;
    const START: &str = "\x1b_Ga=t,f=100,t=d,q=2,i=";
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(START) {
        rest = &rest[at..];
        let digits: String = rest[START.len()..].chars().take_while(char::is_ascii_digit).collect();
        let (mut payload, mut whole, mut next) = (String::new(), false, rest);
        while let Some(body) = next.find("\x1b_G").map(|at| &next[at + 3..]) {
            let Some((control, data)) = body.find("\x1b\\").and_then(|end| body[..end].split_once(';')) else { break };
            // The first chunk is this image's; the rest say whether more follow, and no action.
            if !payload.is_empty() && control.split(',').any(|key| key.starts_with("a=")) {
                break;
            }
            payload.push_str(data);
            next = &body[control.len() + 1 + data.len() + 2..];
            if control.ends_with("m=0") {
                whole = true;
                break;
            }
        }
        rest = if whole { next } else { &rest[START.len()..] };
        let png = base64::engine::general_purpose::STANDARD.decode(&payload).ok().filter(|_| whole);
        if let (Ok(id), Some(Ok(page))) = (digits.parse(), png.map(|png| ::image::load_from_memory(&png))) {
            found.push((id, page.to_rgb8()));
        }
    }
    found
}

/// The ids after each `code` in `text`, in order: of the images placed first on screen, or deleted.
pub(crate) fn ids_after(text: &str, code: &str) -> Vec<u32> {
    let ids = text.match_indices(code).map(|(at, _)| text[at + code.len()..].chars().take_while(char::is_ascii_digit).collect::<String>());
    ids.filter_map(|id| id.parse().ok()).collect()
}

/// The image now placed first on screen, and the images deleted so far.
pub(crate) const PLACED: &str = "\x1b_Ga=p,q=1,i=";
const DELETED: &str = "\x1b_Ga=d,d=I,q=2,i=";

/// Wait until the page on screen is another image than `was`, and say which, with its pixels.
pub(crate) fn next_image(view: &Session, what: &str, was: u32) -> (u32, ::image::RgbImage) {
    view.wait_until(what, |t| {
        let placed = ids_after(t, PLACED).last().copied();
        placed.is_some_and(|id| id != was && pngs(t).iter().any(|(sent, _)| *sent == id))
    });
    let text = view.text();
    let id = ids_after(&text, PLACED).last().copied().unwrap();
    (id, pngs(&text).into_iter().rfind(|(sent, _)| *sent == id).unwrap().1)
}

/// The tint's colours on white paper: the matches', and the match the reader is on.
pub(crate) const TINT: [u8; 3] = [255, 236, 130];
pub(crate) const OWN_TINT: [u8; 3] = [255, 170, 60];

/// The matches of a search kept are tinted into the page as it is sent, where they are and nowhere
/// else, the reader's another colour; `n` moves that colour, the page sent again; `Esc` sends the
/// page as it was. Each page sent again replaces the one before, which is deleted once the new one
/// is placed; and keys read together tint once.
#[test]
fn a_searchs_matches_are_tinted_into_the_page_and_esc_takes_the_tint_off() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("tint", 5);
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 100, 60);
    view.wait_until("page 1 shown, the next two sent ahead", |t| drawn(t, "page 1/5") && transmitted(t) == 3);
    let first = ids_after(&view.text(), PLACED).last().copied().unwrap();
    let plain = pngs(&view.text()).into_iter().find(|(id, _)| *id == first).unwrap().1;
    // "Pager test, page 1": an `e` in each word, three matches a page.
    view.press("/e\r");
    let (tinted_id, tinted) = next_image(&view, "the page tinted", first);
    view.wait_until("the match", |t| last_bar(t).contains(" /e 1/15 "));
    assert_eq!(tinted.dimensions(), plain.dimensions());
    // The text is 36 pt high from 72 pt in, its baseline 112 pt down a page 612 pt wide: every pixel
    // that changed is on that line, and within the words.
    let scale = plain.width() as f32 / 612.0;
    let on_the_line = |x: u32, y: u32| {
        let (x, y) = (x as f32 / scale, y as f32 / scale);
        (70.0..400.0).contains(&x) && (60.0..130.0).contains(&y)
    };
    let changed = |page: &::image::RgbImage| -> Vec<(u32, u32, [u8; 3])> {
        let pixels = page.enumerate_pixels().zip(plain.pixels());
        pixels.filter(|((_, _, is), was)| is != was).map(|((x, y, is), _)| (x, y, is.0)).collect()
    };
    let marked = changed(&tinted);
    assert!(marked.len() > 100, "{} pixels tinted", marked.len());
    assert!(marked.iter().all(|(x, y, _)| on_the_line(*x, *y)), "tinted off the text's line");
    // White paper under a match takes the tint's colour: the first match the reader's, the others theirs.
    let left_of = |colour: [u8; 3], page: &[(u32, u32, [u8; 3])]| page.iter().filter(|(_, _, is)| *is == colour).map(|(x, ..)| *x).min();
    let (own, others) = (left_of(OWN_TINT, &marked).expect("the reader's match"), left_of(TINT, &marked).expect("the other matches"));
    assert!(own < others, "the first match is the reader's: {own} {others}");
    // Three letters tinted of eighteen characters: far less than the line.
    let columns: std::collections::HashSet<u32> = marked.iter().map(|(x, ..)| *x).collect();
    assert!((columns.len() as f32) < 120.0 * scale, "{} columns tinted", columns.len());
    view.wait_until("the page before it deleted", |t| ids_after(t, DELETED).contains(&first));

    // `n`: the page again, the reader's colour on the second match.
    view.press("n");
    let (_, second) = next_image(&view, "the page tinted again", tinted_id);
    let again = changed(&second);
    assert!(left_of(OWN_TINT, &again).unwrap() > own && left_of(TINT, &again).unwrap() == own, "the reader's colour moved on");
    assert_eq!(again.len(), marked.len(), "the same pixels, tinted otherwise");
    view.wait_until("the page before it deleted", |t| ids_after(t, DELETED).contains(&tinted_id));

    // Eight `n` read together, to the third match of page 3: page 1 and page 3 are each sent once
    // more, not once a key.
    view.wait_until("idle", |t| last_bar(t).contains(" /e 2/15 "));
    std::thread::sleep(Duration::from_millis(300));
    let sent = transmitted(&view.text());
    view.press("nnnnnnn");
    view.wait_until("the ninth match", |t| last_bar(t).contains(" /e 9/15 ") && last_bar(t).contains(" page 3/5 "));
    std::thread::sleep(Duration::from_millis(500));
    let more = transmitted(&view.text()) - sent;
    assert!((1..=7).contains(&more), "{more} pages sent for seven keys read together");

    // Twenty more, then `Esc`: every page is sent again untinted, and none sent before it is left.
    for _ in 0..20 {
        view.press("n");
        std::thread::sleep(Duration::from_millis(30));
    }
    view.wait_until("round the file", |t| last_bar(t).contains(" /e 14/15 "));
    let placed_before = ids_after(&view.text(), PLACED).len();
    view.press("\x7fg");
    view.wait_until("cleared, on page 1", |t| !last_bar(t).contains("/e") && last_bar(t).contains(" page 1/5 "));
    // The bar says page 1 before page 1, rendered again, is placed: until then the image placed last
    // is page 5's, tinted. Wait for the image placed last since the Backspace to be page 1 as first sent.
    let deadline = Instant::now() + PATIENCE;
    let last_id = loop {
        let text = view.text();
        let shown = ids_after(&text, PLACED).get(placed_before..).and_then(|ids| ids.last().copied());
        let page = shown.and_then(|id| pngs(&text).into_iter().rfind(|(sent, _)| *sent == id));
        if let Some((id, page)) = page
            && page == plain
        {
            break id;
        }
        assert!(Instant::now() < deadline, "page 1 after Esc isn't the page as first sent");
        std::thread::sleep(Duration::from_millis(20));
    };
    // Until nothing more is sent: at most a page each is left in the terminal, the others deleted.
    let deadline = Instant::now() + PATIENCE;
    loop {
        let before = view.text().len();
        std::thread::sleep(Duration::from_millis(500));
        if view.text().len() == before {
            break;
        }
        assert!(Instant::now() < deadline, "still sending");
    }
    let text = view.text();
    let (sent, deleted): (Vec<u32>, Vec<u32>) = (pngs(&text).into_iter().map(|(id, _)| id).collect(), ids_after(&text, DELETED));
    let left: Vec<&u32> = sent.iter().filter(|id| !deleted.contains(id)).collect();
    assert!(left.len() <= 5 && left.contains(&&last_id), "{} images left of {} sent: more than a page each", left.len(), sent.len());
    assert!(sent.len() > 12, "the test sent {} pages: too few to say anything of the most kept", sent.len());
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// The page on screen shows the match the status line says, however a render's arrival falls among
/// the keys: `n` then `N`, from no time apart to longer than a page takes to render, always ends
/// with the reader's colour back on the first match, in an image the terminal still holds. A page
/// that arrived just as `N` was read was tinted for the match `n` went to, and took the place of
/// the right one.
#[test]
fn the_page_shows_the_match_the_status_line_says_whenever_its_render_arrives() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("tint-race", 2);
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 100, 60);
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/2") && transmitted(t) == 2);
    let first = ids_after(&view.text(), PLACED).last().copied().unwrap();
    view.press("/e\r");
    let (_, tinted) = next_image(&view, "the page tinted", first);
    view.wait_until("the first match", |t| last_bar(t).contains(" /e 1/6 "));
    let own_at = |page: &::image::RgbImage| page.enumerate_pixels().filter(|(_, _, pixel)| pixel.0 == OWN_TINT).map(|(x, ..)| x).min();
    let home = own_at(&tinted).expect("the reader's match");
    for apart in (0..40).step_by(2) {
        view.press("n");
        std::thread::sleep(Duration::from_millis(apart));
        view.press("N");
        view.wait_until("back on the first match", |t| last_bar(t).contains(" /e 1/6 "));
        // Until nothing more is written: every render asked for has arrived or been let go of.
        loop {
            let before = view.text().len();
            std::thread::sleep(Duration::from_millis(150));
            if view.text().len() == before {
                break;
            }
        }
        let text = view.text();
        let placed = ids_after(&text, PLACED).last().copied().unwrap();
        assert!(!ids_after(&text, DELETED).contains(&placed), "{apart} ms apart: the page on screen was deleted");
        let page = pngs(&text).into_iter().rfind(|(id, _)| *id == placed).unwrap().1;
        assert_eq!(own_at(&page), Some(home), "{apart} ms apart: the status line says the first match, the page another");
    }
    view.press("\x7fq");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// Where in `text` each page's upload starts (true) and each page is deleted (false), in order, with
/// the image's id.
fn sent_and_deleted(text: &str) -> Vec<(usize, bool, u32)> {
    const SENT: &str = "\x1b_Ga=t,f=100,t=d,q=2,i=";
    let id_at = |at: usize, code: &str| text[at + code.len()..].chars().take_while(char::is_ascii_digit).collect::<String>().parse::<u32>().ok();
    let mut events: Vec<(usize, bool, u32)> = Vec::new();
    events.extend(text.match_indices(SENT).filter_map(|(at, _)| Some((at, true, id_at(at, SENT)?))));
    events.extend(text.match_indices(DELETED).filter_map(|(at, _)| Some((at, false, id_at(at, DELETED)?))));
    events.sort_unstable();
    events
}

/// The most bytes of pixels the terminal held at once, going by `text`: each page sent counted
/// from where its upload starts, four bytes a pixel, until it is deleted.
fn most_held(text: &str) -> u64 {
    let sizes: std::collections::HashMap<u32, u64> =
        pngs(text).into_iter().map(|(id, page)| (id, u64::from(page.width()) * u64::from(page.height()) * 4)).collect();
    let (mut held, mut most) = (std::collections::HashMap::new(), 0u64);
    for (_, sent, id) in sent_and_deleted(text) {
        match sent {
            true => held.insert(id, sizes.get(&id).copied().unwrap_or(0)),
            false => held.remove(&id),
        };
        most = most.max(held.values().sum());
    }
    most
}

/// The terminal never holds more than its budget of pages while they are sent again with their
/// tints. Three pages side by side of some 15 megapixels, 56 MiB each: the three as they were and
/// the three tinted are 337 MiB, past the 256 MiB the pager keeps to. With the keys up in the
/// pages' place when the tints come, nothing places the new pages over the old, which were all
/// kept; an old copy nobody sees goes as its replacement is sent. And with the pages on screen,
/// where a fourth image fits and a fifth doesn't, the second page sent again waits until the
/// first has taken its old copy's place.
#[test]
fn pages_sent_again_for_their_tints_stay_within_what_the_terminal_keeps() {
    if !pty_works() {
        return;
    }
    const BUDGET: u64 = 256 << 20;
    let pdf = fixture("tint-budget", 3);
    let terminal = Terminal { cell: Some((22, 42)), ..PLAIN };
    let mut view = Session::start_on(terminal, &pdf, &["--protocol", "kitty", "--per-row", "3"], 470, 105);
    view.wait_until("the three pages", |t| last_bar(t).contains(" pages 1–3/3 ") && transmitted(t) == 3 && pngs(t).len() == 3);
    let page: u64 = pngs(&view.text()).iter().map(|(_, page)| u64::from(page.width()) * u64::from(page.height()) * 4).min().unwrap();
    assert!(page * 5 > BUDGET && page * 4 <= BUDGET, "pages of {} MiB: not three that a fourth fits beside and a fifth doesn't", page >> 20);
    // The search kept, then the keys, read together: the tints come while the keys are up.
    view.press("/test\r?");
    view.wait_until("all three sent again, tinted", |t| pngs(t).len() == 6);
    std::thread::sleep(Duration::from_millis(300));
    assert!(most_held(&view.text()) <= BUDGET, "{} MiB held with the keys up", most_held(&view.text()) >> 20);
    // Any key: the pages again. `n`: the reader's match goes from the first page to the second, and
    // both are sent again, each over its old copy, on screen this time.
    view.press("x");
    view.wait_until("the pages back", |t| last_bar(t).contains(" /test 1/3 "));
    let from = view.text().len();
    view.press("n");
    view.wait_until("the second page's match", |t| last_bar(t).contains(" /test 2/3 ") && pngs(t).len() == 8);
    std::thread::sleep(Duration::from_millis(500));
    let text = view.text();
    assert!(most_held(&text) <= BUDGET, "{} MiB held with the pages on screen", most_held(&text) >> 20);
    // The second waited for the room the first's old copy left, and neither was blank meanwhile:
    // the first page, which had the reader's match, goes once its new copy is sent, and only then
    // the second, once its own is. The third keeps the copy it has.
    let pages = pngs(&text);
    let was: Vec<(u32, bool)> = pages.iter().skip(3).take(3).map(|(id, page)| (*id, page.pixels().any(|pixel| pixel.0 == OWN_TINT))).collect();
    let first = was.iter().find(|(_, own)| *own).unwrap().0;
    let order: Vec<(bool, u32)> = sent_and_deleted(&text).into_iter().filter(|(at, ..)| *at >= from).map(|(_, sent, id)| (sent, id)).collect();
    let said = "sent (true) and deleted (false): a page that only had to wait was taken away first";
    assert_eq!(order.get(..3), Some(&[(true, pages[6].0), (false, first), (true, pages[7].0)][..]), "{said}: {order:?}");
    let last = order.get(3).copied().filter(|(sent, id)| !sent && was.iter().any(|(old, own)| old == id && !own));
    assert!(last.is_some() && order.len() == 4, "{said}: {order:?}");
    // And both ended tinted as the status line says: the second page's match the reader's.
    let last: Vec<bool> = pngs(&text).iter().rev().take(2).map(|(_, page)| page.pixels().any(|pixel| pixel.0 == OWN_TINT)).collect();
    assert_eq!(last.iter().filter(|own| **own).count(), 1, "one of the two pages last sent has the reader's match");
    view.press("\x7fq");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// Four pages to a row of some 56 MiB each: the screen alone is 225 MiB of the 256 the pager
/// keeps to, so no page fits beside its own old copy, and no other copy is on its way out to wait
/// for. Each goes first, then comes back with its tint: never a fifth image in the terminal, and
/// never a page left showing matches that aren't the search's. The pager isn't left waiting, nor
/// asking again and again.
#[test]
fn where_the_screen_leaves_no_room_a_page_goes_before_its_tinted_copy_comes() {
    if !pty_works() {
        return;
    }
    const BUDGET: u64 = 256 << 20;
    let pdf = fixture("tint-full", 5);
    let terminal = Terminal { cell: Some((22, 42)), ..PLAIN };
    let mut view = Session::start_on(terminal, &pdf, &["--protocol", "kitty", "--per-row", "4"], 620, 105);
    view.wait_until("four pages", |t| last_bar(t).contains(" pages 1–4/5 ") && !last_bar(t).contains('…') && pngs(t).len() >= 4);
    let page: u64 = pngs(&view.text()).iter().map(|(_, page)| u64::from(page.width()) * u64::from(page.height()) * 4).min().unwrap();
    assert!(page * 4 <= BUDGET && page * 5 > BUDGET, "pages of {} MiB: not four that leave no room for a fifth", page >> 20);
    let shown: Vec<u32> = ids_after(&view.text(), PLACED).into_iter().rev().take(4).collect();
    let before = pngs(&view.text()).len();
    // Until nothing more is written, and then a while: nothing is asked for again and again.
    let quiet = |view: &Session| loop {
        let before = view.text().len();
        std::thread::sleep(Duration::from_millis(400));
        if view.text().len() == before {
            break;
        }
    };
    let has = |page: &::image::RgbImage, colour: [u8; 3]| page.pixels().any(|pixel| pixel.0 == colour);
    // The search kept: the four pages again, the first with the reader's match, the others with theirs.
    view.press("/test\r");
    view.wait_until("the four pages tinted", |t| last_bar(t).contains(" /test 1/5 ") && !last_bar(t).contains('…') && pngs(t).len() == before + 4);
    quiet(&view);
    let text = view.text();
    assert!(most_held(&text) <= BUDGET, "{} MiB held", most_held(&text) >> 20);
    let deleted = ids_after(&text, DELETED);
    assert!(shown.iter().all(|id| deleted.contains(id)), "a page as it was before the search is still held");
    let pages = pngs(&text);
    let tinted: Vec<(bool, bool)> = pages.iter().skip(before).map(|(_, page)| (has(page, OWN_TINT), has(page, TINT))).collect();
    assert_eq!(tinted.iter().filter(|(own, _)| *own).count(), 1, "one page has the reader's match: {tinted:?}");
    assert_eq!(tinted.iter().filter(|(own, other)| !own && *other).count(), 3, "the other three have theirs: {tinted:?}");
    assert_eq!(pages.len(), before + 4, "pages sent again and again");
    // The next match: the page that lost the reader's match and the one that gained it, and no other.
    view.press("n");
    view.wait_until("the next match", |t| last_bar(t).contains(" /test 2/5 ") && !last_bar(t).contains('…') && pngs(t).len() == before + 6);
    quiet(&view);
    let text = view.text();
    assert!(most_held(&text) <= BUDGET, "{} MiB held", most_held(&text) >> 20);
    let pages = pngs(&text);
    assert_eq!(pages.len(), before + 6, "pages sent again and again");
    assert_eq!(pages.iter().rev().take(2).filter(|(_, page)| has(page, OWN_TINT)).count(), 1, "one of the two has the reader's match");
    assert!(!last_bar(&text).contains('…'), "{}", last_bar(&text));
    // The last page alone on its screen, with room beside it: its match is tinted.
    view.press("G");
    view.wait_until("the last page", |t| last_bar(t).contains(" page 5/5 "));
    view.press("n");
    view.wait_until("its match", |t| last_bar(t).contains(" /test 5/5 "));
    // Among the pages sent since: the screens around are sent again too, after it.
    view.wait_until("tinted", |t| pngs(t).iter().skip(before + 6).any(|(_, page)| has(page, OWN_TINT)));
    quiet(&view);
    assert!(most_held(&view.text()) <= BUDGET, "{} MiB held", most_held(&view.text()) >> 20);
    view.press("\x7fq");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// With a search kept and its matches tinted on screen, `c` copies the page as rendered: black on
/// white, none of the tint's colours.
#[test]
fn c_copies_a_page_untinted_while_its_matches_are_tinted() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("tint-copy", 2);
    let terminal = Terminal { clipboard: Some(true), ..PLAIN };
    let mut view = Session::start_on(terminal, &pdf, &["--protocol", "kitty"], 100, 60);
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/2") && transmitted(t) == 2);
    let first = ids_after(&view.text(), PLACED).last().copied().unwrap();
    view.press("/e\r");
    let (_, tinted) = next_image(&view, "the page tinted", first);
    assert!(tinted.pixels().any(|pixel| pixel.0 == OWN_TINT), "the page on screen is tinted");
    view.press("c");
    view.wait_until("page 1 copied", |t| t.contains("copied page 1, 1224 × 1584 pixels"));
    let copy = ::image::load_from_memory(&copied_png(&view.text(), 1)).unwrap().to_rgb8();
    assert_eq!(copy.dimensions(), (1224, 1584));
    let coloured = copy.pixels().filter(|pixel| pixel.0[0] != pixel.0[1] || pixel.0[1] != pixel.0[2]).count();
    assert_eq!(coloured, 0, "the copy carries the tint");
    view.press("\x7fq");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// The colours `text` sets, as its escape codes say them (`38;5;231`, `48;2;255;236;130`).
fn colours_set(text: &str) -> std::collections::HashSet<String> {
    let mut found = std::collections::HashSet::new();
    for code in text.split("\x1b[").skip(1) {
        let Some(end) = code.find(|c: char| !(c.is_ascii_digit() || c == ';')) else { continue };
        if !code[end..].starts_with('m') {
            continue;
        }
        let params: Vec<&str> = code[..end].split(';').collect();
        let mut at = 0;
        while at < params.len() {
            let long = match (params[at], params.get(at + 1)) {
                ("38" | "48", Some(&"5")) => 3,
                ("38" | "48", Some(&"2")) => 5,
                _ => 1,
            };
            if long > 1 && at + long <= params.len() {
                found.insert(params[at..at + long].join(";"));
            }
            at += long;
        }
    }
    found
}

/// In text cells the tint shows as the cells' colours carry it: the rows of a page with matches are
/// drawn again with colours the page hadn't, and `Esc` draws them as they were. Without colour the
/// page doesn't change, and the status line still says where the match is.
#[test]
fn in_text_cells_a_match_shows_as_far_as_the_cells_colours_carry_it() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("tint-cells", 3);
    let mut view = Session::start_on(TEXT_ONLY, &pdf, &["--protocol", "blocks"], 80, 40);
    view.wait_until("page 1 drawn", |t| drawn(t, "page 1/3") && has_cells(t));
    let plain = colours_set(&view.text());
    let from = view.text().len();
    view.press("/e\r");
    view.wait_until("the match, tinted", |t| last_bar(t).contains(" /e 1/9 ") && colours_set(&t[from..]).difference(&plain).count() > 0);
    let from = view.text().len();
    view.press("\x1b");
    view.wait_until("the page as it was", |t| !last_bar(t).contains("/e") && has_cells(&t[from..]));
    std::thread::sleep(Duration::from_millis(300));
    let after = colours_set(&view.text()[from..]);
    assert!(after.is_subset(&plain), "colours left after Esc: {:?}", after.difference(&plain).collect::<Vec<_>>());
    view.press("q");
    assert!(view.finish().success());

    // Letters alone, no colour: nothing to carry the tint, and the search works all the same.
    let mut view = Session::start_command(TEXT_ONLY, &format!("NO_COLOR=1 exec '{BIN}' '{}' --protocol ascii", pdf.display()), 80, 40);
    view.wait_until("page 1 drawn", |t| drawn(t, "page 1/3"));
    assert!(colours_set(&view.text()).is_empty());
    view.press("/e\r");
    view.wait_until("the first match", |t| last_bar(t).contains(" /e 1/9 "));
    view.press("nnn");
    view.wait_until("the fourth match", |t| last_bar(t).contains(" /e 4/9 ") && last_bar(t).contains(" page 2/3 "));
    assert!(colours_set(&view.text()).is_empty(), "{:?}", colours_set(&view.text()));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// A PDF opened from a list is searched as one opened alone, and its worker goes when the reader
/// goes back to the list, where `/` filters. A picture has no text to search, and says so.
#[cfg(feature = "image")]
#[test]
fn a_pdf_opened_from_a_list_is_searched_and_a_picture_has_no_text() {
    if !pty_works() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("clview-pager-search-list-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.pdf"), crate::fixtures::letter("First", 3)).unwrap();
    save(&dir.join("b.png"), 40, 30);
    let mut view = Session::start(&dir, &["--protocol", "kitty"], 100, 60);
    view.wait_until("the list", |t| drawn(t, "1/2") && transmitted(t) == 2);
    view.press("\r");
    view.wait_until("a opened", |t| drawn(t, "page 1/3"));
    view.press("/first, page 2\r");
    view.wait_until("found", |t| last_bar(t).contains(" a.pdf /first, page 2 1/1 page 2/3 "));
    assert_eq!(descendants_running(view.child.id(), "--worker pdf-text").len(), 1);
    // Esc clears the search, then goes back to the list.
    view.press("\x1b");
    view.wait_until("cleared", |t| last_bar(t).contains(" a.pdf page 2/3 "));
    no_text_worker(&view, "the search was cleared");
    view.press("/page\r");
    view.wait_until("searched again", |t| last_bar(t).contains(" a.pdf /page 2/3 page 2/3 "));
    view.press("\x1b\x1b");
    view.wait_until("back to the list", |t| last_bar(t).contains(" a.pdf 1/2 "));
    no_text_worker(&view, "the reader went back to the list");
    // In the list `/` filters; in the picture it says there is no text.
    view.press("/b\r\r");
    view.wait_until("b opened", |t| last_bar(t).contains(" b.png ") && last_bar(t).contains(" page 1/1 "));
    view.press("/");
    view.wait_until("no text", |t| last_bar(t).contains(" b.png no text to search page 1/1 "));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `clv --help` on a `cols` × `rows` pseudo-terminal of type `term`, as it arrives there, its
/// carriage returns included: see [`newlines`].
fn help_on(term: &str, cols: u16, rows: u16) -> String {
    let terminal = Terminal { term: Some(term), ..PLAIN };
    let mut help = Session::start_on(terminal, Path::new("--help"), &[], cols, rows);
    assert!(help.finish().success());
    help.text()
}

/// `text`, as a pseudo-terminal wrote it, with its newlines put back: the pseudo-terminal writes
/// each as `\r\n`, and as `\r\r\n` where its output queue filled between the two (macOS's tty
/// driver, with onlcr, writes the `\r` again when the `\n` didn't fit), at an offset the help's
/// length decides; so any run of `\r` before a `\n` is one `\r\n`, and then a `\n`. A `\r` with no
/// `\n` after it (clv's own, back to column 0) stays.
fn newlines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut returns = 0;
    for c in text.chars() {
        match c {
            '\r' => returns += 1,
            '\n' => {
                returns = 0;
                out.push('\n');
            }
            other => {
                out.extend(std::iter::repeat_n('\r', returns));
                returns = 0;
                out.push(other);
            }
        }
    }
    out.extend(std::iter::repeat_n('\r', returns));
    out
}

/// `clv --help` through a pipe: the text alone.
fn help_text() -> String {
    String::from_utf8(crate::bin::clv().arg("--help").output().unwrap().stdout).unwrap()
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
        let raw = help_on(term, 120, 40);
        let shown = newlines(&raw);
        // 10 × 20 cells, as no pixel size is reported: 5 rows are 10 columns, ending at column 100.
        let up = format!("{top}\x1b[5A\r\x1b[90C\x1b_Ga=T,f=100,t=d,q=2,C=1,c=10,r=5,");
        assert!(shown.starts_with(&up), "{term}: {:?}", &shown[..up.len().min(shown.len())]);
        // clv's `\r` after the icon comes before a newline, where `newlines` folds it with the
        // pseudo-terminal's own: it is checked as written, two `\r` at least before that `\n`.
        assert!(raw.contains("\x1b[5B\r\r\n"), "{term}: no return to column 0 after the icon");
        assert!(shown.ends_with(&format!("\x1b\\\x1b[5B{rest}")), "{term}: {:?}", &shown[shown.len().saturating_sub(rest.len() + 20)..]);
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
    assert_eq!(newlines(&help_on("xterm-256color", 120, 40)), plain, "a terminal not known to show it");
    assert_eq!(newlines(&help_on("xterm-kitty", 70, 40)), plain, "no room beside the lines");
    assert_eq!(newlines(&help_on("xterm-kitty", 120, 3)), plain, "too few rows");
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
    // As the terminal's own line ends, and the carriage return a full queue doubles ([`newlines`]).
    assert_eq!(newlines(&String::from_utf8(out).unwrap()), help_text());
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
    view.wait_until("the number said", |t| t[before..].contains(" 2: g page · . per row"));
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
    view.wait_until("the number said", |t| t[settled..].contains(" 2: g page · . per row"));
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
const KITTY: Terminal = Terminal { term: Some("xterm-kitty"), ..PLAIN };

/// A dark terminal, not grey, as many are: what a page takes from it can't be mistaken.
#[cfg(any(feature = "word", feature = "mermaid"))]
const DARK: &str = "rgb:1e1e/2e2e/3e3e";
#[cfg(any(feature = "word", feature = "mermaid"))]
const DARK_COLOUR: [u8; 3] = [0x1E, 0x2E, 0x3E];

/// What `clv` falls back on for dark paper.
#[cfg(any(feature = "word", feature = "mermaid"))]
const DARK_PAPER: [u8; 3] = [0x1E, 0x1E, 0x1E];

/// The colour at the corner of each page sent so far, in order.
#[cfg(any(feature = "word", feature = "mermaid"))]
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
/// `D` turns it to white and back: sent again each time, and saying so.
#[cfg(feature = "word")]
#[test]
fn markdown_is_on_a_dark_terminals_own_colour_and_d_turns_it() {
    if !pty_works() {
        return;
    }
    let terminal = Terminal { background: Some(DARK), ..KITTY };
    let mut view = Session::start_on(terminal, &markdown("dark"), &["--pager"], 80, 40);
    assert_eq!(last_corner(&view, 1), DARK_COLOUR);
    view.press("D");
    assert_eq!(last_corner(&view, 2), [255; 3]);
    view.wait_until("said", |t| bar(t).contains("dark on light"));
    view.press("D");
    assert_eq!(last_corner(&view, 3), DARK_COLOUR);
    view.wait_until("said", |t| bar(t).contains("light on dark"));
    view.press("q");
    assert!(view.finish().success());
}

/// A light terminal, or one that doesn't say, gets a page as printed; `D` still takes it to dark paper,
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
        view.press("D");
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
    view.press("D");
    assert_eq!(last_corner(&view, 2), DARK_COLOUR);
    view.press("q");
    assert!(view.finish().success());

    let light = Terminal { background: Some("rgb:ffff/ffff/ffff"), ..KITTY };
    let mut view = Session::start_on(light, &markdown("theme"), &["--pager", "--theme", "dark"], 80, 40);
    assert_eq!(last_corner(&view, 1), DARK_PAPER);
    view.press("q");
    assert!(view.finish().success());
}

/// What has no dark form doesn't ask about the background, and `D` says which formats have one and
/// sends nothing: `D` is the backends', and what it says names the formats of each that declares it.
#[cfg(all(feature = "word", feature = "mermaid"))]
#[test]
fn what_has_no_dark_form_isnt_asked_and_d_says_which_formats_go_dark() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("no-dark", 1);
    let mut view = Session::start_on(Terminal { background: Some(DARK), ..KITTY }, &pdf, &["--pager"], 80, 40);
    assert_eq!(last_corner(&view, 1), [255; 3]);
    view.press("D");
    const SAID: &str = "this format doesn't go dark (Mermaid, Markdown and others do)";
    whole_line_for_a_moment(&view, SAID);
    assert_eq!(transmitted(&view.text()), 1, "sent again");
    // A key takes the line back at once, and does what it does: here, nothing to move to.
    view.press("D");
    view.wait_until("said again", |t| last_bar(t).trim() == SAID);
    let pressed = Instant::now();
    view.press("k");
    view.wait_until("the line back with the key", |t| last_bar(t).contains(" page 1/1 "));
    assert!(pressed.elapsed() < Duration::from_millis(700), "{:?}", pressed.elapsed());
    assert!(!view.text().contains("\x1b]11;"), "asked about the background");
    view.press("?");
    // In `?`, what the key does, then two of the formats it does it to: over two rows at 80
    // columns, and faint, as a key that does nothing on a PDF.
    view.wait_until("listed", |t| {
        bar(t).contains("\x1b[2m D light on dark, or dark on light (Mermaid, Markdown and") && t.contains("others)\x1b[22m")
    });
    view.press("qq");
    assert!(view.finish().success());
}

/// A Mermaid diagram shown by itself follows the paper as Markdown does. On a dark terminal, asked
/// its background, it is drawn in Mermaid's dark theme on the terminal's own colour, nothing white
/// behind it; `D` turns it to the default theme on white and back, saying so. The diagram is laid
/// out once a theme: turned to a paper it was on before, no worker starts. On a light terminal it
/// starts on white, and `D` goes to a dark of `clv`'s own; `--theme` says how it starts whatever
/// the terminal is.
#[cfg(feature = "mermaid")]
#[test]
fn a_diagram_shown_by_itself_is_on_a_dark_terminals_colour_and_d_turns_it() {
    if !pty_works() {
        return;
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/sequence.mermaid");
    let on = |paper: [u8; 3]| {
        move |page: &::image::RgbImage| {
            let white = page.pixels().filter(|pixel| pixel.0.iter().all(|v| *v > 240)).count();
            let light_ink = page.pixels().filter(|pixel| pixel.0.iter().all(|v| *v > 150)).count();
            page.get_pixel(0, 0).0 == paper && white == 0 && light_ink > 200
        }
    };
    let on_light = |page: &::image::RgbImage| {
        let dark_ink = page.pixels().filter(|pixel| pixel.0.iter().all(|v| *v < 128)).count();
        page.get_pixel(0, 0).0 == [255; 3] && dark_ink > 200
    };
    let mut view = Session::start_on(Terminal { background: Some(DARK), ..KITTY }, &path, &["--pager"], 100, 40);
    let (mut shown, page) = next_image(&view, "on the terminal's dark", 0);
    assert!(on(DARK_COLOUR)(&page), "in the dark theme, on the terminal's colour");
    assert!(view.text().contains("\x1b]11;"), "the terminal wasn't asked its background");
    no_mermaid_worker(&view, "both themes laid out");
    for (turn, dark) in [(1, false), (2, true), (3, false)] {
        view.press("D");
        let (next, page) = next_image(&view, "turned", shown);
        assert!(if dark { on(DARK_COLOUR)(&page) } else { on_light(&page) }, "turn {turn}");
        view.wait_until("said", |t| last_bar(t).contains(if dark { "light on dark" } else { "dark on light" }));
        assert!(mermaid_workers(&view).is_empty(), "turn {turn}: the diagram laid out again");
        shown = next;
    }
    view.press("q");
    assert!(view.finish().success());

    let light = Terminal { background: Some("rgb:fdfd/f6f6/e3e3"), ..KITTY };
    let mut view = Session::start_on(light, &path, &["--pager"], 100, 40);
    let (shown, page) = next_image(&view, "on white", 0);
    assert!(on_light(&page), "a light terminal: as Mermaid draws it by default");
    view.press("D");
    let (_, page) = next_image(&view, "turned", shown);
    assert!(on(DARK_PAPER)(&page), "a dark of its own");
    view.press("q");
    assert!(view.finish().success());

    let mut view = Session::start_on(Terminal { background: Some(DARK), ..KITTY }, &path, &["--pager", "--theme", "light"], 100, 40);
    let (_, page) = next_image(&view, "on white", 0);
    assert!(on_light(&page), "--theme light on a dark terminal");
    view.press("q");
    assert!(view.finish().success());
}

/// In a list, a diagram's tile is on a dark terminal's colour beside a PDF's on white, and `D` turns
/// it to white, the PDF's as it was: the list's tiles go through their pools, each diagram's dark
/// renders on a thread of its own.
#[cfg(feature = "mermaid")]
#[test]
fn in_a_list_a_diagrams_tile_follows_the_paper_and_a_pdfs_stays() {
    if !pty_works() {
        return;
    }
    let dir = crate::view::tmp("dark-diagrams", "shelf");
    std::fs::create_dir_all(&dir).unwrap();
    let flow = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/flow.mmd");
    std::fs::copy(flow, dir.join("a.mmd")).unwrap();
    std::fs::copy(fixture("dark-diagrams", 1), dir.join("b.pdf")).unwrap();
    let mut view = Session::start_on(Terminal { background: Some(DARK), ..KITTY }, &dir, &[], 80, 40);
    view.wait_until("both tiles", |t| corners(t).len() >= 2);
    let tiles = corners(&view.text());
    assert!(tiles.contains(&DARK_COLOUR) && tiles.contains(&[255; 3]), "{tiles:?}");
    view.press("D");
    view.wait_until("both again", |t| corners(t).len() >= 4);
    assert_eq!(corners(&view.text())[2..], [[255; 3], [255; 3]]);
    view.wait_until("said", |t| last_bar(t).contains("dark on light"));
    view.press("D");
    view.wait_until("and back", |t| corners(t).len() >= 6);
    let tiles = &corners(&view.text())[4..];
    assert!(tiles.contains(&DARK_COLOUR) && tiles.contains(&[255; 3]), "{tiles:?}");
    view.press("q");
    assert!(view.finish().success());
}

/// In a list, `D` turns what has a dark form, and what hasn't stays white.
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
    view.press("D");
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
    const SAID: &str = "this format doesn't reflow (Markdown, HTML and others do)";
    view.wait_until("said", |t| last_bar(t).trim() == SAID);
    // Said again by the next key, for a moment of its own.
    view.press("72[");
    view.wait_until("said again", |t| bar(t).matches(SAID).count() >= 2);
    whole_line_for_a_moment(&view, SAID);
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
    view.press("\x7f");
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
    // Enough text to take a while to lay out: under a second here.
    let path = crate::view::tmp("reflow-threads", "big.md");
    let paragraph = "Words enough in a paragraph to fill a few lines of a page, over and over again. ".repeat(12);
    let big: String = (1..=2800).map(|n| format!("Paragraph {n}. {paragraph}\n\n")).collect();
    std::fs::write(&path, big).unwrap();
    let mut view = Session::start_on(KITTY, &path, &["--pager"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/") || t.contains("  pages 1"));
    // `?` comes with the width, read with it. Pressed only once `width 40…` was seen, it raced the
    // layout through script(1) and the test's own polling, and on a loaded runner the layout ended
    // before `?` was read. Read together, `?` is taken right after the layout starts, and only the
    // viewer can keep it waiting.
    view.press("40]?");
    view.wait_until("under way", |t| t.contains("width 40…"));
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
    // The margin yields to the font: 12 typed is kept and laid out at 6, the text at its size; said so from
    // the first status line after the layout, never from the bounds of one column.
    let before = view.text().len();
    view.press("12}");
    view.wait_until("12, laid out at 6", |t| bar(last_draw(&t[before..])).contains("· margin 12 (6) ·"));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!bar(&view.text()[before..]).contains("· margin 12 ·"), "a line from other bounds: {:?}", bar(&view.text()[before..]));
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

/// While a number is typed on Markdown, the status line says it and every key that takes it here, by
/// name, and nothing else: not the name, not the place.
#[cfg(feature = "word")]
#[test]
fn a_number_typed_on_markdown_names_the_keys_that_take_it() {
    if !pty_works() {
        return;
    }
    let mut view = Session::start_on(KITTY, &long_markdown("typed", "long.md"), &["--pager"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/"));
    view.press("7");
    view.wait_until("the keys", |t| bar(last_draw(t)).contains(" 7: g page · . per row · + font · ] width · } margin "));
    let line = bar(last_draw(&view.text()));
    assert!(!line.contains("long.md") && !line.contains("page 1/") && !line.contains("? help"), "{line:?}");
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
pub(crate) fn place_now(text: &str) -> Option<(usize, usize)> {
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
    view.press("D");
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

/// A search outlives a relayout of its document: kept, the match you are on stays the one you are
/// on, its tint on the new layout's pages and the view on it; being typed, it is typed on, and
/// counts what the whole text holds. A document whose text changed on disk meanwhile starts its
/// search over, the query kept.
#[cfg(feature = "word")]
#[test]
fn a_search_outlives_a_relayout_and_starts_over_on_another_text() {
    if !pty_works() {
        return;
    }
    let path = long_markdown("search-relayout", "long.md");
    let mut view = Session::start_on(KITTY, &path, &["--pager"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/"));
    // Every paragraph says "with" once: the fourth is the one gone to.
    view.press("/with\r");
    view.wait_until("the first", |t| last_bar(t).contains(" /with 1/60 "));
    view.press("3n");
    view.wait_until("the fourth", |t| last_bar(t).contains(" /with 4/60 "));
    let shown = ids_after(&view.text(), PLACED).last().copied().unwrap();
    view.press("40]");
    view.wait_until("laid out again, the same match", |t| bar(t).contains("· width 40 ·") && last_bar(t).contains(" /with 4/60 "));
    // Its pages come first as rendered, then tinted once the match's new place is known: one placed
    // since, two to a row now, has the reader's colour.
    view.wait_until("the new layout's page, tinted", |t| {
        let placed = ids_after(t, PLACED);
        let new = placed.iter().skip_while(|id| **id != shown).filter(|id| **id > shown).collect::<Vec<_>>();
        pngs(t).iter().any(|(sent, page)| new.contains(&sent) && page.pixels().any(|pixel| pixel.0 == OWN_TINT))
    });
    view.press("n");
    view.wait_until("on from it", |t| last_bar(t).contains(" /with 5/60 "));
    // Being typed: laid out again meanwhile, it is typed on, and counts the whole text's.
    view.press("\x7f0]/wit");
    view.wait_until("laid out again while typed", |t| last_bar(t).contains(" /wit 60 ") && last_bar(t).contains(" 1 per row "));
    view.press("h\r");
    view.wait_until("kept", |t| last_bar(t).contains(" /with ") && last_bar(t).contains("/60 "));
    // Another text on disk, in as many paragraphs: its search starts over, the query kept, its
    // matches counted anew, the old text's never kept for its count of paragraphs.
    let half: String = (1..=60).map(|n| format!("Paragraph {n}, {} other words.\n\n", if n % 2 == 0 { "with" } else { "sans" })).collect();
    std::fs::write(&path, format!("# Long\n\n{half}")).unwrap();
    view.press("40]");
    view.wait_until("started over, as many paragraphs", |t| bar(t).contains("· width 40 ·") && last_bar(t).contains(" /with 30 "));
    // And in more.
    let more: String = (1..=80).map(|n| format!("Paragraph {n}, with other words.\n\n")).collect();
    std::fs::write(&path, format!("# Longer\n\n{more}")).unwrap();
    view.press("0]");
    view.wait_until("started over", |t| last_bar(t).contains(" 1 per row ") && last_bar(t).contains(" /with 80 "));
    view.press("q");
    assert!(view.finish().success());
}

/// A directory of fake openers, `open`, `xdg-open` and `setsid`, each writing its name and its
/// arguments to `launched` in the directory, a line each and an empty line after them: `open` and
/// `xdg-open` then exit with `exit`, and `setsid` runs what it was given. Put first on the `PATH`
/// `clv` runs with, before `/bin`, so that no test reaches a real desktop app.
fn openers(test: &str, exit: i32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = std::env::temp_dir().join(format!("clview-openers-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("launched");
    let quoted = log.display().to_string().replace('\'', r"'\''");
    for (name, then) in [("open", format!("exit {exit}")), ("xdg-open", format!("exit {exit}")), ("setsid", "exec \"$@\"".to_owned())] {
        let program = dir.join(name);
        std::fs::write(&program, format!("#!/bin/sh\nprintf '%s\\n' '{name}' \"$@\" '' >> '{quoted}'\n{then}\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

/// What the fake openers in `dir` were run with, in order: each one's name and arguments.
fn launched(dir: &Path) -> Vec<Vec<String>> {
    let log = std::fs::read_to_string(dir.join("launched")).unwrap_or_default();
    log.split("\n\n").filter(|record| !record.is_empty()).map(|record| record.lines().map(str::to_owned).collect()).collect()
}

/// What opening `real` runs on this system, as the fake openers note it: `open` on macOS; elsewhere
/// `xdg-open` through `setsid`.
fn opening(dir: &Path, real: &Path) -> Vec<Vec<String>> {
    let real = real.display().to_string();
    match cfg!(target_os = "macos") {
        true => vec![vec!["open".to_owned(), real]],
        false => vec![vec!["setsid".to_owned(), dir.join("xdg-open").display().to_string(), real.clone()], vec!["xdg-open".to_owned(), real]],
    }
}

/// A terminal on this machine with a desktop session, its `PATH` the fake openers in `dir` and `/bin`.
fn desktop(path: &str) -> Terminal<'_> {
    Terminal { path: Some(path), display: true, ..PLAIN }
}

/// Wait until the fake openers in `dir` were run `count` times; panics if they never are.
fn wait_launched(dir: &Path, count: usize) -> Vec<Vec<String>> {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let launched = launched(dir);
        if launched.len() >= count {
            return launched;
        }
        assert!(Instant::now() < deadline, "launched {launched:?}, not {count}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `o` in a file's pages hands its real path to the opener, on whatever page, a link followed and
/// a name with spaces as it is, then says it opened it; `?` lists it.
#[test]
fn open_a_files_pages_hands_its_real_path_to_the_opener() {
    if !pty_works() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("clview-pager-open-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("the docs")).unwrap();
    let real = dir.join("the docs/-a 'quoted' report.pdf");
    std::fs::write(&real, crate::fixtures::letter("Open me", 3)).unwrap();
    let link = dir.join("link.pdf");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let real = std::fs::canonicalize(&real).unwrap();
    let fakes = openers("pages", 0);
    let path = format!("{}:/bin", fakes.display());
    let mut view = Session::start_on(desktop(&path), &link, &["--protocol", "kitty"], 100, 60);
    view.wait_until("page 1 shown", |t| drawn(t, "page 1/3"));
    view.press("?");
    view.wait_until("the keys", |t| t.contains("open the file in the app the desktop opens it with"));
    view.press(" no");
    view.wait_until("page 2", |t| drawn(t, "page 2/3"));
    view.wait_until("opened", |t| t.contains("opened link.pdf"));
    assert!(bar(&view.text()).contains(" page 2/3 "), "on page 2");
    assert_eq!(launched(&fakes), opening(&fakes, &real));
    view.press("q");
    assert!(view.finish().success());
}

/// A file whose real name doesn't say the format it was shown as isn't opened, the desktop going
/// by that name: a PDF named `.command`, or a `.pdf` linked to it. Nor is one read through
/// `/dev/stdin`, from a file or a pipe, nor any over SSH. An opener's failure is said in its words.
#[test]
fn open_refuses_a_name_that_doesnt_say_its_format_stdin_and_ssh() {
    if !pty_works() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("clview-pager-open-refused-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("report.command");
    std::fs::write(&script, crate::fixtures::letter("Not a script", 1)).unwrap();
    let link = dir.join("report.pdf");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&script, &link).unwrap();
    let fakes = openers("refused", 0);
    let path = format!("{}:/bin", fakes.display());
    let quote = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', r"'\''"));
    let refused = |terminal: Terminal, command: String, said: &str| {
        let mut view = Session::start_command(terminal, &command, 200, 60);
        view.wait_until("shown", |t| drawn(t, "page 1/1"));
        view.press("o");
        view.wait_until(said, |t| t.contains(said));
        view.press("q");
        assert!(view.finish().success());
    };
    let named = "report.command: its name doesn't say PDFs, so it isn't opened";
    refused(desktop(&path), format!("exec '{BIN}' {} --pager", quote(&script)), named);
    refused(desktop(&path), format!("exec '{BIN}' {} --pager", quote(&link)), named);
    let stdin = "stdin: it is read through /dev or /proc, not from a file of its own, so there is none to open";
    refused(desktop(&path), format!("exec '{BIN}' /dev/stdin --pager < {}", quote(&link)), stdin);
    let pipe = "stdin: it isn't a file of its own (a pipe or a device), so there is none to open";
    refused(desktop(&path), format!("cat {} | '{BIN}' /dev/stdin --pager", quote(&link)), pipe);
    let pdf = fixture("open-ssh", 1);
    let ssh = "one.pdf isn't opened: over SSH, its app would open on the machine clv runs on, not yours";
    refused(Terminal { ssh: true, ..desktop(&path) }, format!("exec '{BIN}' {} --pager", quote(&pdf)), ssh);
    assert_eq!(launched(&fakes), Vec::<Vec<String>>::new(), "nothing was opened");
    // The opener says why it didn't, in its words.
    let failing = openers("refused-failing", 1);
    let path = format!("{}:/bin", failing.display());
    let why = match cfg!(target_os = "macos") {
        true => "one.pdf: open couldn't open it (1): it is gone, or no app opens it",
        false => "one.pdf: xdg-open didn't take what it was given (1)",
    };
    refused(desktop(&path), format!("exec '{BIN}' {} --pager", quote(&pdf)), why);
    assert_eq!(wait_launched(&failing, 1), opening(&failing, &std::fs::canonicalize(&pdf).unwrap()));
}

/// Where there is no opener, it says so and runs nothing: on Linux and the BSDs with no desktop
/// session, and anywhere with no opener on `PATH`.
#[test]
fn open_with_no_desktop_or_no_opener_says_so() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("open-nowhere", 1);
    let fakes = openers("nowhere", 0);
    let path = format!("{}:/bin", fakes.display());
    let quote = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', r"'\''"));
    let said = |terminal: Terminal, said: &str| {
        let mut view = Session::start_command(terminal, &format!("exec '{BIN}' {} --pager", quote(&pdf)), 200, 60);
        view.wait_until("shown", |t| drawn(t, "page 1/1"));
        view.press("o");
        view.wait_until(said, |t| t.contains(said));
        view.press("q");
        assert!(view.finish().success());
    };
    if !cfg!(target_os = "macos") {
        said(Terminal { display: false, ..desktop(&path) }, "one.pdf isn't opened: there is no desktop here to open it in");
        assert!(launched(&fakes).is_empty());
    }
    // A PATH of an empty directory alone: not even `/bin`, which may hold the system's opener.
    let empty = std::env::temp_dir().join(format!("clview-openers-none-{}", std::process::id()));
    std::fs::create_dir_all(&empty).unwrap();
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let path = empty.display().to_string();
    said(desktop(&path), &format!("one.pdf isn't opened: there is no {program} to open it with"));
}

/// What showing the folder `real` runs on this system, as the fake openers note it: `open -R` on
/// macOS, revealed in Finder; elsewhere `xdg-open` through `setsid`, as for a file.
#[cfg(feature = "tar")]
fn opening_folder(dir: &Path, real: &Path) -> Vec<Vec<String>> {
    match cfg!(target_os = "macos") {
        true => vec![vec!["open".to_owned(), "-R".to_owned(), real.display().to_string()]],
        false => opening(dir, real),
    }
}

/// Wait until the status line has said `said` since `from` bytes of output.
fn said_since(view: &Session, from: usize, said: &str) {
    view.wait_until(said, |t| t.get(from..).is_some_and(|t| t.contains(said)));
}

/// In a list, `o` opens the tile chosen: a file opened when its tile was wanted, one that turned
/// out broken (the app may read it), a file compressed alone as the compressed file it is, a
/// folder (revealed on macOS); not an archive, nor its files or folders. A file gone into keeps
/// its path, opened at once or when its tile was wanted.
#[cfg(feature = "tar")]
#[test]
fn open_in_a_list_opens_the_tile_chosen_and_refuses_an_archives() {
    if !pty_works() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("clview-pager-open-list-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("e")).unwrap();
    std::fs::write(dir.join("a.pdf"), crate::fixtures::letter("A", 2)).unwrap();
    std::fs::write(dir.join("b-broken.pdf"), b"%PDF-1.7\nnot a PDF after all\n").unwrap();
    std::fs::write(dir.join("c.pdf.gz"), crate::tar::gzip(&crate::fixtures::letter("C", 1))).unwrap();
    let inner = crate::tar::tar(&[("x.pdf", crate::fixtures::letter("X", 1)), ("f/", Vec::new()), ("f/y.pdf", crate::fixtures::letter("Y", 1))]);
    std::fs::write(dir.join("d.tar"), inner).unwrap();
    std::fs::write(dir.join("e/z.pdf"), crate::fixtures::letter("Z", 1)).unwrap();
    let real = |name: &str| std::fs::canonicalize(dir.join(name)).unwrap();
    let fakes = openers("list", 0);
    let path = format!("{}:/bin", fakes.display());
    let mut view = Session::start_on(desktop(&path), &dir, &["--protocol", "kitty"], 100, 60);
    view.wait_until("the list", |t| drawn(t, "1/5"));
    view.press("?");
    view.wait_until("the keys", |t| t.contains("open it in the desktop's app: a file in its app, a folder in the file manager"));
    let mut expected = Vec::new();
    for (keys, said, launched) in [
        (" o", "opened a.pdf", opening(&fakes, &real("a.pdf"))),
        ("lo", "opened b-broken.pdf", opening(&fakes, &real("b-broken.pdf"))),
        ("lo", "opened c.pdf.gz", opening(&fakes, &real("c.pdf.gz"))),
        ("lo", "d.tar/: it is an archive's folder, not one of its own to show", Vec::new()),
        ("lo", "opened e/", opening_folder(&fakes, &real("e"))),
    ] {
        let from = view.text().len();
        view.press(keys);
        said_since(&view, from, said);
        expected.extend(launched);
        assert_eq!(wait_launched(&fakes, expected.len()), expected, "{said}");
    }
    // Into the tar: its file and its folder have none of their own.
    let from = view.text().len();
    view.press("h\r");
    view.wait_until("in the tar", |t| t.get(from..).is_some_and(|t| bar(t).contains(" d.tar/f/ 1/2 ")));
    for (keys, said) in [
        ("o", "d.tar/f/: it is an archive's folder, not one of its own to show"),
        ("lo", "d.tar/x.pdf: it is in an archive, with no file of its own to open"),
    ] {
        let from = view.text().len();
        view.press(keys);
        said_since(&view, from, said);
    }
    // Back, then into a.pdf, opened when its tile was wanted, and c.pdf.gz, opened at once.
    for (keys, name, real) in [("\x7fg\r", "a.pdf", real("a.pdf")), ("\x7fll\r", "c.pdf.gz", real("c.pdf.gz"))] {
        let from = view.text().len();
        view.press(keys);
        view.wait_until(name, |t| t.get(from..).is_some_and(|t| t.contains("  page 1/")));
        let from = view.text().len();
        view.press("o");
        said_since(&view, from, &format!("opened {name}"));
        expected.extend(opening(&fakes, &real));
        assert_eq!(wait_launched(&fakes, expected.len()), expected, "{name}");
    }
    view.press("q");
    assert!(view.finish().success());
}

/// A document laid out again keeps its file: `o` after `40]` opens it as before.
#[cfg(feature = "word")]
#[test]
fn open_a_document_laid_out_again() {
    if !pty_works() {
        return;
    }
    let markdown = long_markdown("open-relaid", "long.md");
    let fakes = openers("relaid", 0);
    let path = format!("{}:/bin", fakes.display());
    let terminal = Terminal { term: Some("xterm-kitty"), ..desktop(&path) };
    let mut view = Session::start_on(terminal, &markdown, &["--pager"], 100, 60);
    view.wait_until("page 1", |t| t.contains("  page 1/"));
    view.press("40]");
    view.wait_until("laid out again", |t| bar(t).contains("· width 40 ·") && last_sent_shaped(t, NARROW, WINDOW.1));
    view.press("o");
    view.wait_until("opened", |t| t.contains("opened long.md"));
    assert_eq!(wait_launched(&fakes, 1), opening(&fakes, &std::fs::canonicalize(&markdown).unwrap()));
    view.press("q");
    assert!(view.finish().success());
}

/// The PDF with links ([`crate::fixtures::linked`]), in a directory of the test's own.
fn linked(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("clview-pager-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pdf = dir.join("links.pdf");
    std::fs::write(&pdf, crate::fixtures::linked()).unwrap();
    pdf
}

/// A letter of a link's label as placed: the image it is a part of, the cell it is placed at (from
/// 1, as the cursor is moved there), and where its part is in the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Letter {
    image: u32,
    row: u16,
    col: u16,
    x: u32,
    y: u32,
}

/// The letters of the links' labels placed since they were last taken off the screen, in order:
/// not the strips of padding at each label's ends, which are parts of the box after the fourteen
/// letters' (12 pixels each, at the tests' cell).
pub(crate) fn letters(text: &str) -> Vec<Letter> {
    parts(text).into_iter().filter(|part| part.x < 14 * 12).collect()
}

/// Every part of the labels' image placed since the labels were last taken off the screen.
fn parts(text: &str) -> Vec<Letter> {
    const HIDDEN: &str = "\x1b_Ga=d,d=i,q=2,i=";
    const PLACED_PART: &str = "\x1b_Ga=p,q=1,C=1,z=1,i=";
    let Some(from) = text.rfind(HIDDEN) else { return Vec::new() };
    let mut found = Vec::new();
    let mut rest = &text[from..];
    while let Some(at) = rest.find(PLACED_PART) {
        // The cursor is moved to its cell just before: `ESC [ row ; col H`.
        let before = &rest[..at];
        let cell = before.rfind("\x1b[").and_then(|c| before[c + 2..].strip_suffix('H')).and_then(|c| c.split_once(';'));
        let code = &rest[at + PLACED_PART.len()..];
        let Some(end) = code.find("\x1b\\") else { break };
        let key = |k: &str| code[..end].split(',').find_map(|kv| kv.strip_prefix(k)?.parse::<u32>().ok());
        let image = code[..end].split(',').next().and_then(|id| id.parse().ok());
        if let (Some(image), Some((row, col)), Some(x), Some(y)) = (image, cell, key("x="), key("y=")) {
            found.push(Letter { image, row: row.parse().unwrap(), col: col.parse().unwrap(), x, y });
        }
        rest = &code[end..];
    }
    found
}

/// The images of the labels' letters sent so far: at the tests' cell of 10 × 20 pixels, fourteen
/// boxes of 12 × 20 and one more without a letter, twice.
fn sheets(text: &str) -> Vec<(u32, ::image::RgbImage)> {
    pngs(text).into_iter().filter(|(_, image)| image.dimensions() == (180, 40)).collect()
}

/// Wait until the status line, since `from` bytes of output, has said `place` with nothing left to
/// render.
fn drawn_since(view: &Session, from: usize, place: &str) {
    view.wait_until(place, |t| t.get(from..).is_some_and(|t| drawn(t, place)));
}

/// `f` labels the links on screen: one image of letters is sent, and each label is a placement of
/// its letter's part of it, over the page, down the page in reading order. A label typed goes to its
/// link's place, the labels gone and their image deleted; Backspace comes back, and at the top goes
/// nowhere. A link that is never followed says what it is, `Esc` drops the labels, and `?` lists
/// the key.
#[test]
fn links_are_labelled_from_one_image_and_a_label_typed_goes_to_its_place() {
    if !pty_works() {
        return;
    }
    let pdf = linked("links");
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 100, 60);
    view.wait_until("the first page", |t| drawn(t, "page 1/3"));
    assert!(sheets(&view.text()).is_empty() && letters(&view.text()).is_empty());
    view.press("f");
    view.wait_until("the labels", |t| letters(t).len() == 14 && sheets(t).len() == 1);
    let text = view.text();
    let (sheet, image) = sheets(&text).remove(0);
    // Yellow boxes, dark letters.
    assert_eq!(image.get_pixel(0, 1).0, [255, 221, 87]);
    assert!(image.pixels().any(|p| p.0 == [48, 37, 5]));
    let placed = letters(&text);
    // One letter each, in the letters' order, each from its own box of the image's first row.
    for (nth, letter) in placed.iter().enumerate() {
        assert_eq!((letter.image, letter.x, letter.y), (sheet, 12 * nth as u32, 0), "label {nth}");
    }
    // Down the page, every link starting at the same place across it.
    assert!(placed.windows(2).all(|pair| pair[0].row < pair[1].row && pair[0].col == pair[1].col), "{placed:?}");
    assert!(placed[0].row > 1 && placed[13].row < 60 && placed[0].col > 1, "{placed:?}");
    // Each between two strips of the box with no letter, the padding at its ends: the first at the
    // link's own place, the letter two pixels on, the second twelve more.
    let all = parts(&text);
    assert_eq!(all.len(), 14 * 3);
    for label in all.chunks(3) {
        assert_eq!((label[0].x, label[0].y, label[2].x, label[2].y), (168, 0, 168, 0), "{label:?}");
        assert!(
            label[0].row == label[1].row && label[1].row == label[2].row && label[0].col <= label[1].col && label[1].col < label[2].col,
            "{label:?}"
        );
    }
    assert!(last_bar(&text).contains(" f: type a label to follow its link · esc cancel "), "{}", last_bar(&text));
    // `s`, the first: to page 3, by `/XYZ`.
    let before = text.len();
    view.press("s");
    drawn_since(&view, before, "page 3/3");
    view.wait_until("the labels' image deleted", |t| t[before..].contains(&format!("{DELETED}{sheet}\x1b")));
    // Backspace comes back, and then has nowhere to go: the pager stays.
    let before = view.text().len();
    view.press("\x7f");
    drawn_since(&view, before, "page 1/3");
    view.press("\x7f\x7f");
    std::thread::sleep(Duration::from_millis(200));
    assert!(view.child.try_wait().unwrap().is_none(), "Backspace quit");
    // `a`, the second: the named destination, on page 2; Backspace comes back again.
    view.press("f");
    view.wait_until("the labels again", |t| sheets(t).len() == 2 && letters(t).len() == 14);
    let before = view.text().len();
    view.press("a");
    drawn_since(&view, before, "page 2/3");
    let before = view.text().len();
    view.press("\x7f");
    drawn_since(&view, before, "page 1/3");
    // `p`, the twelfth: a script, said and not run.
    view.press("f");
    view.wait_until("the labels a third time", |t| sheets(t).len() == 3 && letters(t).len() == 14);
    let before = view.text().len();
    view.press("p");
    said_since(&view, before, "a script, which clv doesn't run");
    // Esc drops them, and does nothing else.
    view.press("f");
    view.wait_until("the labels a fourth time", |t| sheets(t).len() == 4 && letters(t).len() == 14);
    let (last, before) = (sheets(&view.text()).last().unwrap().0, view.text().len());
    view.press("\x1b");
    view.wait_until("the labels dropped", |t| t[before..].contains(&format!("{DELETED}{last}\x1b")));
    std::thread::sleep(Duration::from_millis(200));
    assert!(view.child.try_wait().unwrap().is_none(), "Esc quit");
    assert!(last_bar(&view.text()).contains(" page 1/3 "));
    // A picture's page, or a list, has no links; `?` lists the key.
    view.press("?");
    view.wait_until("the keys", |t| t.contains("label the links on screen") && t.contains("back to where the last link followed was"));
    view.press("qq");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// With more links on screen than letters, every label has two: the first typed greys it in the
/// labels it starts and hides the others, Backspace brings them back, and the label left alone by
/// its first letter is followed by it. Here two pages side by side, fifteen links between them.
#[test]
fn links_on_two_pages_side_by_side_have_two_letters_and_typing_narrows_them() {
    if !pty_works() {
        return;
    }
    let pdf = linked("links-two");
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 200, 60);
    view.wait_until("two pages", |t| drawn(t, "pages 1–2/3"));
    view.press("f");
    view.wait_until("the labels", |t| letters(t).len() == 30);
    let placed = letters(&view.text());
    // `ss`, `sa`, … on the first page, `as` on the second, to its right.
    assert!(placed.iter().all(|letter| letter.y == 0));
    assert_eq!((placed[0].x, placed[1].x, placed[2].x, placed[3].x), (0, 0, 0, 12));
    assert_eq!((placed[28].x, placed[29].x), (12, 0));
    // Two letters side by side: 12 pixels apart, in cells of 10.
    assert!(placed[28].col > placed[0].col + 50 && (1..=2).contains(&(placed[1].col - placed[0].col)), "{placed:?}");
    let before = view.text().len();
    view.press("s");
    view.wait_until("the labels narrowed", |t| t.len() > before && letters(t).len() == 28 && last_bar(t).contains("its link · s · esc cancel"));
    let narrowed = letters(&view.text());
    assert!(narrowed.chunks(2).all(|label| label[0].y == 20 && label[0].x == 0 && label[1].y == 0), "the letter typed, greyed: {narrowed:?}");
    let before = view.text().len();
    view.press("\x7f");
    view.wait_until("the labels back", |t| t.len() > before && letters(t).len() == 30 && letters(t).iter().all(|letter| letter.y == 0));
    // `a` alone is the second page's link, which leads to the first page: on screen already.
    let before = view.text().len();
    view.press("a");
    view.wait_until("the labels gone", |t| t[before..].contains(DELETED));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// In a file opened from a list, Backspace goes back through the links followed, and then to the list;
/// and `f` in the list labels nothing.
#[test]
fn links_followed_in_a_file_from_a_list_are_gone_back_through_before_the_list() {
    if !pty_works() {
        return;
    }
    let pdf = linked("links-list");
    let other = pdf.with_file_name("other.pdf");
    std::fs::write(&other, crate::fixtures::letter("Other", 3)).unwrap();
    let mut view = Session::start(&pdf, &[other.to_str().unwrap(), "--protocol", "kitty"], 100, 60);
    view.wait_until("the list", |t| drawn(t, "1/2"));
    view.press("f");
    std::thread::sleep(Duration::from_millis(300));
    assert!(sheets(&view.text()).is_empty(), "labels in a list");
    view.press("\r");
    view.wait_until("the file", |t| drawn(t, "page 1/3"));
    view.press("f");
    view.wait_until("the labels", |t| letters(t).len() == 14);
    let before = view.text().len();
    view.press("s");
    drawn_since(&view, before, "page 3/3");
    let before = view.text().len();
    view.press("\x7f");
    drawn_since(&view, before, "page 1/3");
    let before = view.text().len();
    view.press("\x7f");
    view.wait_until("back to the list", |t| t.get(before..).is_some_and(|t| bar(t).contains(" links.pdf 1/2 · 2 per row ")));
    // Gone into again, there is nothing to go back through: Backspace is the list at once.
    view.press("\r");
    let before = view.text().len();
    drawn_since(&view, before.saturating_sub(1), "page 1/3");
    let before = view.text().len();
    view.press("\x7f");
    view.wait_until("the list again", |t| t.get(before..).is_some_and(|t| bar(t).contains(" links.pdf 1/2 · 2 per row ")));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// A list opens a PDF for its tile as the file itself, with no links read; gone into, it is opened
/// whole, and `f` at once, while it opens, labels its links when it has, saying nothing of no links
/// meanwhile.
#[test]
fn a_pdf_tile_gone_into_is_opened_whole_and_f_meanwhile_waits_for_its_links() {
    if !pty_works() {
        return;
    }
    let pdf = linked("links-tile");
    let other = pdf.with_file_name("other.pdf");
    std::fs::write(&other, crate::fixtures::letter("Other", 3)).unwrap();
    let mut view = Session::start(&pdf, &[other.to_str().unwrap(), "--protocol", "kitty"], 100, 60);
    view.wait_until("the list", |t| drawn(t, "1/2"));
    let before = view.text().len();
    view.press("\rf");
    view.wait_until("the labels", |t| letters(t).len() == 14);
    let since = &view.text()[before..];
    assert!(!since.contains("no links on screen"), "said there were none");
    // Apart from the key after it: the labels go with Esc, and `q` is one of their letters.
    view.press("\x1b");
    std::thread::sleep(Duration::from_millis(200));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// With `--pages`, a link to a page given goes where that page is first among them, and one to a
/// page not given says so and goes nowhere.
#[test]
fn links_to_a_page_not_among_pages_are_said_and_not_gone_to() {
    if !pty_works() {
        return;
    }
    let pdf = linked("links-pages");
    let mut view = Session::start(&pdf, &["--protocol", "kitty", "--pages", "2,1,2,1"], 100, 60);
    view.wait_until("the second page, given first", |t| drawn(t, "page 2/3 · 1 of 4"));
    // Its one link leads to page 1.
    view.press("f");
    view.wait_until("its label", |t| letters(t).len() == 1);
    let before = view.text().len();
    view.press("s");
    drawn_since(&view, before, "page 1/3 · 2 of 4");
    // From there, the named destination on page 2: where it is given first.
    view.press("f");
    view.wait_until("the first page's labels", |t| letters(t).len() == 14);
    let before = view.text().len();
    view.press("a");
    drawn_since(&view, before, "page 2/3 · 1 of 4");
    let before = view.text().len();
    view.press("\x7f");
    drawn_since(&view, before, "page 1/3 · 2 of 4");
    // And page 3, which isn't given.
    view.press("f");
    view.wait_until("the labels again", |t| sheets(t).len() == 3 && letters(t).len() == 14);
    let before = view.text().len();
    view.press("s");
    said_since(&view, before, "page 3 isn't among --pages");
    std::thread::sleep(Duration::from_millis(200));
    assert!(last_bar(&view.text()).contains(" page 1/3 · 2 of 4 "), "{}", last_bar(&view.text()));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// A file with no links says so, as does a screen with none on it.
#[test]
fn links_where_there_are_none_are_said() {
    if !pty_works() {
        return;
    }
    let pdf = fixture("links-none", 2);
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 100, 60);
    view.wait_until("the first page", |t| drawn(t, "page 1/2"));
    let before = view.text().len();
    view.press("f");
    said_since(&view, before, "no links on screen");
    assert!(sheets(&view.text()).is_empty());
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// In text cells a link's label is written over the page's cells, a capital on yellow in the
/// colours the terminal has: only the rows that carry one are written again, and again as they were
/// once the labels go. A label typed goes to its place.
#[test]
fn links_in_text_cells_are_written_over_the_pages_cells() {
    if !pty_works() {
        return;
    }
    // In the palette's colours, as a terminal that says only `xterm-256color` gets them.
    const LABEL: &str = "\x1b[0;48;5;221;38;5;234m";
    // A row written again: erased, then its line; and the status line, in reverse.
    let rows_written = |text: &str| text.matches("\x1b[2K").count() - text.matches("\x1b[2K\x1b[7m").count();
    let pdf = linked("links-cells");
    let mut view = Session::start_on(TEXT_ONLY, &pdf, &["--protocol", "blocks"], 80, 40);
    view.wait_until("page 1 drawn", |t| drawn(t, "page 1/3") && has_cells(t));
    std::thread::sleep(Duration::from_millis(300));
    let before = view.text().len();
    view.press("f");
    view.wait_until("the labels", |t| t[before..].matches(LABEL).count() == 14 && last_bar(t).contains("esc cancel"));
    let text = view.text();
    let labelled = &text[before..];
    for letter in "SADFJKLEWCMPGH".chars() {
        assert_eq!(labelled.matches(&format!("{LABEL}{letter}\x1b[0m")).count(), 1, "{letter}");
    }
    assert_eq!(rows_written(labelled), 14, "a row for each label, and no other");
    assert!(!text.contains("\x1b_G"), "an image in text cells");
    // Esc: the same rows again, as they were.
    let before = text.len();
    view.press("\x1b");
    view.wait_until("the labels gone", |t| rows_written(&t[before..]) == 14);
    std::thread::sleep(Duration::from_millis(200));
    assert!(!view.text()[before..].contains(LABEL) && rows_written(&view.text()[before..]) == 14);
    // And a label typed is followed.
    let before = view.text().len();
    view.press("f");
    view.wait_until("the labels again", |t| t[before..].matches(LABEL).count() == 14);
    let before = view.text().len();
    view.press("s");
    drawn_since(&view, before, "page 3/3");
    assert!(!view.text()[before..].contains(LABEL));
    view.press("\x7f");
    drawn_since(&view, before, "page 1/3");
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// In a Markdown document a link to a heading goes to it, and Backspace comes back to the paragraph the
/// reader left, whatever the document is laid out at since: the places to come back to are handed
/// to the new layout, and found again in it. The labels are read again from the new layout.
#[cfg(feature = "word")]
#[test]
fn links_followed_in_a_document_are_gone_back_through_after_it_is_laid_out_again() {
    if !pty_works() {
        return;
    }
    let path = crate::view::tmp("links-relayout", "guide.md");
    let filler = |from: usize| {
        (from..from + 60).map(|n| format!("Paragraph {n}, with words enough in it to fill a line or two of a page.\n\n")).collect::<String>()
    };
    std::fs::write(
        &path,
        format!("# Guide\n\n[install](#install) and [usage](#usage)\n\n{}## Install\n\n{}## Usage\n\nThe end.\n", filler(1), filler(100)),
    )
    .unwrap();
    let mut view = Session::start_on(KITTY, &path, &["--pager"], 100, 60);
    view.wait_until("page 1", |t| place_now(t).is_some_and(|(page, _)| page == 1) && last_draw(t).contains("  page 1/"));
    view.press("f");
    view.wait_until("the labels", |t| letters(t).len() == 2);
    // `a`, the second: the heading `Usage`, near the document's end.
    view.press("a");
    view.wait_until("the heading", |t| place_now(t).is_some_and(|(page, count)| page > 2 && page + 1 >= count));
    let (usage, count) = place_now(&view.text()).unwrap();
    // Narrower pages, more of them: the reader stays at the heading.
    view.press("40]");
    view.wait_until("laid out again", |t| bar(t).contains("· width 40 ·") && place_now(t).is_some_and(|(page, pages)| pages > count && page > usage));
    // Backspace goes back to the first paragraph, which is on the first page of any layout.
    let before = view.text().len();
    view.press("\x7f");
    view.wait_until("back at the links", |t| t.len() > before && place_now(&t[before..]).is_some_and(|(page, _)| page == 1));
    // The new layout's links are read from it: `s`, the first, is the heading `Install`.
    view.press("f");
    view.wait_until("the labels of the new layout", |t| t.len() > before && letters(&t[before..]).len() == 2);
    view.press("s");
    view.wait_until("the other heading", |t| place_now(t).is_some_and(|(page, pages)| page > 2 && page + 3 < pages));
    view.press("\x7fq");
    assert!(view.finish().success());
}

/// A directory of the test's own, empty.
#[cfg(feature = "word")]
fn empty_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("clview-pager-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `f` and the letter `label`, once the `links` labels on screen show: the output's length before
/// the letter.
fn follow(view: &mut Session, links: usize, label: &str) -> usize {
    let before = view.text().len();
    view.press("f");
    view.wait_until("the labels", |t| letters(&t[before..]).len() == links);
    let before = view.text().len();
    view.press(label);
    before
}

/// Whether the file called `name` is on screen with nothing still to come: every page rendered,
/// and the document not being laid out again for the window, as one just opened from a link is,
/// which would drop labels asked for meanwhile.
#[cfg(feature = "word")]
fn settled(text: &str, name: &str) -> bool {
    let bar = last_bar(text);
    bar.contains(&format!(" {name} ")) && !bar.contains('…')
}

/// `F` and the letter `label`, likewise.
#[cfg(feature = "word")]
fn copy_link(view: &mut Session, links: usize, label: &str) -> usize {
    let before = view.text().len();
    view.press("F");
    view.wait_until("the labels", |t| letters(&t[before..]).len() == links && last_bar(t).contains("to copy its link's address"));
    let before = view.text().len();
    view.press(label);
    before
}

/// The text clipboard write `id` carried: its `text/plain` packets' data, put together.
#[cfg(feature = "word")]
fn copied_text(text: &str, id: u32) -> String {
    use base64::Engine as _;
    let start = format!("\x1b]5522;type=wdata:mime={}:id=clv-{id};", base64::engine::general_purpose::STANDARD.encode("text/plain"));
    let mut copied = Vec::new();
    for (at, _) in text.match_indices(&start) {
        let data = &text[at + start.len()..];
        let Some(end) = data.find("\x1b\\") else { break };
        copied.extend(base64::engine::general_purpose::STANDARD.decode(&data[..end]).unwrap());
    }
    String::from_utf8(copied).unwrap()
}

/// A link to another file opens it in `clv`, a level of its own, at the place the link names; Backspace
/// comes back to the document, and from there to the list. A folder opens as a list, a PDF at
/// its first page. A path that leads nowhere, or to a pipe, is said, and nothing opens.
#[cfg(feature = "word")]
#[test]
fn links_to_other_files_open_them_as_levels_and_u_comes_back_to_the_document() {
    if !pty_works() {
        return;
    }
    let dir = empty_dir("links-files");
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    let filler = "Filler, with words enough in it to fill a line of a page.\n\n".repeat(80);
    let index = dir.join("index.md");
    std::fs::write(&index, "# Index\n\n[the guide](docs/guide.md#usage) [gone](gone.md) [pipe](pipe.md) [folder](docs) [the pdf](other.pdf)\n")
        .unwrap();
    std::fs::write(dir.join("docs/guide.md"), format!("# Guide\n\n{filler}## Usage\n\nHere.\n\n{filler}")).unwrap();
    let other = dir.join("other.pdf");
    std::fs::write(&other, crate::fixtures::letter("Other", 3)).unwrap();
    assert!(Command::new("mkfifo").arg(dir.join("pipe.md")).status().unwrap().success());
    // Wide, for the whole of what the status line says of a link not followed.
    let mut view = Session::start_on(KITTY, &index, &[other.to_str().unwrap(), "--pager"], 160, 60);
    view.wait_until("the list", |t| drawn(t, "1/2"));
    view.press("\r");
    view.wait_until("the document", |t| settled(t, "index.md") && place_now(t).is_some());
    // The guide, at its heading, well into it.
    let before = follow(&mut view, 5, "s");
    view.wait_until("the guide at its heading", |t| {
        settled(t, "guide.md") && place_now(&t[before..]).is_some_and(|(page, pages)| page > 1 && page < pages)
    });
    std::thread::sleep(Duration::from_millis(500));
    let (heading, pages) = place_now(&view.text()).unwrap();
    assert!(heading > 1 && heading < pages, "at page {heading} of {pages}");
    // Back: the document, as it was left.
    let before = view.text().len();
    view.press("\x7f");
    view.wait_until("back in the document", |t| {
        t.len() > before && settled(t, "index.md") && place_now(&t[before..]).is_some_and(|(page, _)| page == 1)
    });
    // What leads nowhere, and what is no file.
    let before = follow(&mut view, 5, "a");
    said_since(&view, before, "gone.md: it can't be found");
    let before = follow(&mut view, 5, "d");
    said_since(&view, before, "pipe.md: it is no file or folder");
    assert!(last_bar(&view.text()).contains(" index.md "));
    // A folder, as a list; a PDF, at its first page.
    let before = follow(&mut view, 5, "f");
    view.wait_until("the folder listed", |t| bar(&t[before..]).contains(" docs/guide.md 1/1 "));
    let before = view.text().len();
    view.press("\x7f");
    view.wait_until("back from the folder", |t| t.len() > before && settled(t, "index.md"));
    let before = follow(&mut view, 5, "j");
    view.wait_until("the PDF", |t| settled(t, "other.pdf") && drawn(&t[before..], "pages 1–2/3"));
    let before = view.text().len();
    view.press("\x7f");
    view.wait_until("back from the PDF", |t| t.len() > before && settled(t, "index.md"));
    // And from the document, the list.
    let before = view.text().len();
    view.press("\x7f");
    view.wait_until("the list again", |t| t.get(before..).is_some_and(|t| bar(t).contains(" index.md 1/2 ")));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A PDF's link to another PDF opens it at the destination it names, or at the page; one whose
/// file isn't there says so.
#[test]
fn links_from_a_pdf_to_another_pdf_open_it_at_the_place_or_the_page_they_name() {
    if !pty_works() {
        return;
    }
    let pdf = linked("links-remote");
    let other = pdf.with_file_name("other.pdf");
    let mut view = Session::start(&pdf, &["--protocol", "kitty"], 100, 60);
    view.wait_until("the first page", |t| drawn(t, "page 1/3"));
    // No `other.pdf` yet.
    let before = follow(&mut view, 14, "f");
    said_since(&view, before, "other.pdf: it can't be found");
    std::fs::write(&other, crate::fixtures::linked()).unwrap();
    // `f`, the fourth: its destination `intro`, on page 3.
    let before = follow(&mut view, 14, "f");
    view.wait_until("the other PDF at its destination", |t| last_bar(t).contains(" other.pdf ") && drawn(&t[before..], "page 3/3"));
    let before = view.text().len();
    view.press("\x7f");
    view.wait_until("back", |t| t.len() > before && last_bar(t).contains(" links.pdf ") && drawn(&t[before..], "page 1/3"));
    // `j`, the fifth: its third page.
    let before = follow(&mut view, 14, "j");
    view.wait_until("the other PDF at its page", |t| last_bar(t).contains(" other.pdf ") && drawn(&t[before..], "page 3/3"));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// A document read through a pipe has no file of its own to look beside: a link to another file
/// says so, and a link to a place in it still goes there.
#[cfg(feature = "word")]
#[test]
fn links_to_other_files_from_stdin_are_said_and_not_followed() {
    if !pty_works() {
        return;
    }
    let dir = empty_dir("links-stdin");
    let filler = "<p>Filler, with words enough in it to fill a line of a page.</p>".repeat(80);
    let page = dir.join("page.html");
    std::fs::write(
        &page,
        format!(
            "<!DOCTYPE html><html><body><p><a href=\"guide.md\">guide</a> <a href=\"#end\">end</a></p>{filler}<p id=\"end\">End</p></body></html>"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("guide.md"), "# Guide\n").unwrap();
    let mut view = Session::start_command(KITTY, &format!("cd '{}' && cat page.html | '{BIN}' /dev/stdin --pager", dir.display()), 100, 60);
    view.wait_until("the page", |t| place_now(t).is_some_and(|(page, _)| page == 1) && last_draw(t).contains("  page 1/"));
    let before = follow(&mut view, 2, "s");
    // As much of it as a window this wide leaves whole.
    said_since(&view, before, "guide.md: not followed, as");
    assert!(last_bar(&view.text()).contains("to look beside"), "{}", last_bar(&view.text()));
    let before = follow(&mut view, 2, "a");
    view.wait_until("the place in it", |t| place_now(&t[before..]).is_some_and(|(page, _)| page > 1));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A web address goes to the desktop's opener, as its single argument, the status line naming it:
/// `http`, `https` and `mailto`. A `mailto` that names a file to attach is said and never launched,
/// as written or percent-encoded twice over. (Another scheme never gets this far from Markdown:
/// what reads it makes no link of one. `link::web`'s own tests have those.)
#[cfg(feature = "word")]
#[test]
fn links_to_the_web_go_to_the_desktops_opener_and_other_schemes_nowhere() {
    if !pty_works() {
        return;
    }
    let dir = empty_dir("links-web");
    let notes = dir.join("notes.md");
    let text = "# Notes\n\n[site](https://example.com/a?b=c&d=e) [mail](mailto:a@example.com?subject=Hi) [bad](mailto:a@example.com?attach=/etc/passwd) [twice](mailto:a@example.com?%2561ttach=x)\n";
    std::fs::write(&notes, text).unwrap();
    let fakes = openers("links-web", 0);
    let path = format!("{}:/bin", fakes.display());
    let terminal = Terminal { term: Some("xterm-kitty"), ..desktop(&path) };
    let mut view = Session::start_on(terminal, &notes, &["--pager"], 160, 40);
    view.wait_until("the notes", |t| place_now(t).is_some() && last_draw(t).contains("  page 1/"));
    let before = follow(&mut view, 4, "s");
    said_since(&view, before, "opening https://example.com/a?b=c&d=e…");
    said_since(&view, before, "opened https://example.com/a?b=c&d=e");
    assert_eq!(wait_launched(&fakes, 1), opening(&fakes, Path::new("https://example.com/a?b=c&d=e")));
    let count = launched(&fakes).len();
    let before = follow(&mut view, 4, "a");
    said_since(&view, before, "opened mailto:a@example.com?subject=Hi");
    let all = wait_launched(&fakes, count * 2);
    assert_eq!(all[count..], opening(&fakes, Path::new("mailto:a@example.com?subject=Hi"))[..]);
    // Never launched: what would attach a file, as written or encoded twice over.
    let before = follow(&mut view, 4, "d");
    said_since(&view, before, "a mailto: link that names a file for the mail app to read, which isn't followed");
    let before = follow(&mut view, 4, "f");
    said_since(&view, before, "a mailto: link that names a file for the mail app to read, which isn't followed");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(launched(&fakes).len(), count * 2, "{:?}", launched(&fakes));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `F` copies the address of the link whose label is typed, as the document writes it, through the
/// terminal's clipboard protocol as text: a web address, a file's path with its fragment, a place's
/// name. A link with no address says where it goes, and one never followed what it is; neither is
/// copied. Nothing is followed.
#[cfg(feature = "word")]
#[test]
fn links_addresses_are_copied_by_capital_f_as_the_document_writes_them() {
    if !pty_works() {
        return;
    }
    let dir = empty_dir("links-copy");
    let notes = dir.join("notes.md");
    std::fs::write(&notes, "# Notes\n\n[site](https://example.com/a?b=c) [guide](docs/guide.md#usage) [top](#notes) [gone](#nowhere)\n").unwrap();
    let terminal = Terminal { clipboard: Some(true), term: Some("xterm-kitty"), ..PLAIN };
    let mut view = Session::start_on(terminal, &notes, &["--pager"], 120, 40);
    view.wait_until("the notes", |t| place_now(t).is_some() && last_draw(t).contains("  page 1/"));
    let before = copy_link(&mut view, 4, "s");
    said_since(&view, before, "copied https://example.com/a?b=c");
    assert_eq!(copied_text(&view.text(), 1), "https://example.com/a?b=c");
    let before = copy_link(&mut view, 4, "a");
    said_since(&view, before, "copied docs/guide.md#usage");
    assert_eq!(copied_text(&view.text(), 2), "docs/guide.md#usage");
    let before = copy_link(&mut view, 4, "d");
    said_since(&view, before, "copied #notes");
    assert_eq!(copied_text(&view.text(), 3), "#notes");
    let before = copy_link(&mut view, 4, "f");
    said_since(&view, before, "no #nowhere in this document");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(copied_text(&view.text(), 4), "", "a place the document hasn't was copied");
    assert!(last_bar(&view.text()).contains(" notes.md ") && place_now(&view.text()).is_some_and(|(page, _)| page == 1), "something was followed");
    view.press("q");
    assert!(view.finish().success());
    // A PDF's link to a page has no address: where it goes is said. A script says what it is.
    let pdf = linked("links-copy-pdf");
    let mut view = Session::start_on(Terminal { clipboard: Some(true), ..PLAIN }, &pdf, &["--protocol", "kitty"], 100, 60);
    view.wait_until("the first page", |t| drawn(t, "page 1/3"));
    let before = copy_link(&mut view, 14, "s");
    said_since(&view, before, "goes to page 3");
    let before = copy_link(&mut view, 14, "p");
    said_since(&view, before, "a script, which clv doesn't run");
    let before = copy_link(&mut view, 14, "d");
    said_since(&view, before, "copied https://example.com/a?b=c");
    let before = copy_link(&mut view, 14, "f");
    said_since(&view, before, "copied other.pdf#intro");
    let text = view.text();
    assert_eq!(
        (copied_text(&text, 1), copied_text(&text, 2), copied_text(&text, 3)),
        ("https://example.com/a?b=c".into(), "other.pdf#intro".into(), String::new())
    );
    assert!(drawn(last_draw(&text), "page 1/3"), "something was followed");
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(pdf.parent().unwrap()).unwrap();
}

/// Over SSH no app is opened on the machine `clv` runs on: a web address followed is copied
/// instead, through OSC 52 where the terminal has no clipboard protocol, and the status line says
/// so; `F` copies the same way.
#[cfg(feature = "word")]
#[test]
fn links_to_the_web_over_ssh_are_copied_through_osc_52() {
    use base64::Engine as _;
    if !pty_works() {
        return;
    }
    let dir = empty_dir("links-ssh");
    let notes = dir.join("notes.md");
    std::fs::write(&notes, "# Notes\n\n[site](https://example.com/a?b=c) [mail](mailto:a@example.com)\n").unwrap();
    let fakes = openers("links-ssh", 0);
    let path = format!("{}:/bin", fakes.display());
    let terminal = Terminal { clipboard: Some(false), ssh: true, term: Some("xterm-kitty"), ..desktop(&path) };
    let mut view = Session::start_on(terminal, &notes, &["--pager"], 160, 40);
    view.wait_until("the notes", |t| place_now(t).is_some() && last_draw(t).contains("  page 1/"));
    let osc52 = |address: &str| format!("\x1b]52;c;{}\x1b\\", base64::engine::general_purpose::STANDARD.encode(address));
    let before = follow(&mut view, 2, "s");
    said_since(&view, before, "not opened (over SSH): copying https://example.com/a?b=c…");
    said_since(&view, before, "sent https://example.com/a?b=c to the terminal's clipboard (OSC 52)");
    assert_eq!(view.text().matches(&osc52("https://example.com/a?b=c")).count(), 1);
    let before = copy_link(&mut view, 2, "a");
    said_since(&view, before, "sent mailto:a@example.com to the terminal's clipboard (OSC 52)");
    assert_eq!(view.text().matches(&osc52("mailto:a@example.com")).count(), 1);
    assert!(launched(&fakes).is_empty(), "{:?}", launched(&fakes));
    assert!(!view.text().contains("\x1b]5522;"), "a write through the protocol it doesn't speak");
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Links followed into files go sixteen levels deep, two files that link to each other counting
/// each time; the seventeenth says so, and Backspace still comes back.
#[cfg(feature = "word")]
#[test]
fn links_into_files_go_sixteen_levels_deep_and_no_deeper() {
    if !pty_works() {
        return;
    }
    let dir = empty_dir("links-deep");
    let (a, b) = (dir.join("a.md"), dir.join("b.md"));
    std::fs::write(&a, "# A\n\n[to b](b.md)\n").unwrap();
    std::fs::write(&b, "# B\n\n[to a](a.md)\n").unwrap();
    // Wide, for the whole of what the status line says past the last level.
    let mut view = Session::start_on(KITTY, &a, &["--pager"], 160, 40);
    view.wait_until("a", |t| place_now(t).is_some() && last_draw(t).contains("  page 1/"));
    for level in 1..=16 {
        let name = if level % 2 == 1 { "b.md" } else { "a.md" };
        let before = follow(&mut view, 1, "s");
        view.wait_until("the next file", |t| t.len() > before && settled(t, name) && last_bar(t).contains(" page 1/1 "));
    }
    let before = follow(&mut view, 1, "s");
    said_since(&view, before, "b.md: not followed, 16 files deep in links already; backspace goes back");
    assert!(last_bar(&view.text()).contains(" a.md "));
    let before = view.text().len();
    view.press("\x7f");
    view.wait_until("back a level", |t| t.len() > before && settled(t, "b.md"));
    // And from there a link is followed again.
    let before = follow(&mut view, 1, "s");
    view.wait_until("a again", |t| t.len() > before && settled(t, "a.md"));
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Backspace comes back to the page the reader left, where a paragraph runs over several pages: a link
/// followed from the middle of a long paragraph is come back to on its own page, not on the page
/// the paragraph starts on.
#[cfg(feature = "word")]
#[test]
fn links_followed_from_the_middle_of_a_long_paragraph_are_come_back_to_on_their_page() {
    if !pty_works() {
        return;
    }
    let dir = empty_dir("links-long");
    let long = dir.join("long.md");
    let words = |count: usize| "lots of words in one paragraph ".repeat(count);
    std::fs::write(&long, format!("{}[jump](#end) {}\n\n# End\n", words(400), words(400))).unwrap();
    let mut view = Session::start_on(KITTY, &long, &["--pager"], 100, 40);
    view.wait_until("the first page", |t| settled(t, "long.md") && place_now(t).is_some_and(|(page, pages)| page == 1 && pages > 4));
    // On to the page the link is on: the first with a label.
    let mut on = 1;
    loop {
        let before = view.text().len();
        view.press("f");
        view.wait_until("its links", |t| letters(&t[before..]).len() == 1 || t[before..].contains("no links on screen"));
        if letters(&view.text()[before..]).len() == 1 {
            break;
        }
        on += 1;
        assert!(on < 20, "no page has the link");
        let before = view.text().len();
        view.press("n");
        view.wait_until("the next page", |t| settled(t, "long.md") && place_now(&t[before..]).is_some_and(|(page, _)| page == on));
    }
    assert!(on > 1, "the link is on the paragraph's first page");
    let before = view.text().len();
    view.press("s");
    view.wait_until("the end", |t| place_now(&t[before..]).is_some_and(|(page, pages)| page == pages));
    let before = view.text().len();
    view.press("\x7f");
    // Wherever it goes back to, it leaves the last page: then, which page it is on.
    view.wait_until("back", |t| place_now(&t[before..]).is_some_and(|(page, pages)| page != pages));
    assert_eq!(place_now(&view.text()).map(|(page, _)| page), Some(on), "back on the page the link is on");
    view.press("q");
    assert!(view.finish().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A book is searched as one document: a word of its third chapter is found from its first, and
/// the reader taken to that chapter's page. The worker that read its package has answered and is
/// gone by the time the book shows, and neither the search nor a relayout starts another.
#[cfg(all(feature = "epub", feature = "image"))]
#[test]
fn epub_a_search_finds_a_word_in_a_later_chapter_and_the_worker_is_gone() {
    use crate::epub::{Mimetype, three_chapters, write, zipped};
    if !pty_works() {
        return;
    }
    let path = write(crate::view::tmp("pager-epub-search", "book.epub"), &zipped(&three_chapters(), Mimetype::Stored));
    let mut view = Session::start(&path, &["--protocol", "kitty", "--pager"], 100, 60);
    view.wait_until("the book", |t| drawn(t, "page 1/3"));
    let workers = |view: &Session| descendants_running(view.child.id(), "--worker epub");
    assert!(workers(&view).is_empty(), "the worker outlived its answer");
    view.press("/green\r");
    view.wait_until("found in the third chapter", |t| last_bar(t).contains(" /green 1/1 ") && place_now(t).is_some_and(|(page, _)| page == 3));
    // Apart from the key after it: an escape and a `]` together start an escape code.
    view.press("\x1b");
    view.wait_until("the search ended", |t| !last_bar(t).contains("/green"));
    // Laid out again, from the reading order kept: the worker isn't asked again.
    let before = view.text().len();
    view.press("[");
    let mut seen = Vec::new();
    let deadline = Instant::now() + PATIENCE;
    let narrower = |t: &str| last_bar(t).split("width ").nth(1).is_some_and(|width| width.starts_with(|c: char| c.is_ascii_digit()));
    while !view.text().get(before..).is_some_and(|t| narrower(t) && !last_bar(t).contains('…')) {
        assert!(Instant::now() < deadline, "never laid out again: {}", last_bar(&view.text()));
        seen.extend(workers(&view));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(seen.is_empty() && workers(&view).is_empty(), "a worker for a relayout: {seen:?}");
    view.press("q");
    assert!(view.finish().success());
}

/// A list opens a book for its tile as far as its first page; gone into, the book is laid out whole,
/// all its chapters and its Contents page, even in a window whose pages area is the room a list's
/// tiles are laid out for, where nothing else would lay it out again. The status line says where
/// its contents are, as it does when a book opens alone.
#[cfg(all(feature = "epub", feature = "image"))]
#[test]
fn epub_a_tile_gone_into_is_the_whole_book_even_in_the_tiles_room() {
    use crate::epub::{Mimetype, NAV_ITEM, THREE_ENTRIES, book_with, coloured_chapters, nav, write, zipped};
    if !pty_works() {
        return;
    }
    let dir = empty_dir("epub-tile");
    let (chapters, mut pictures) = coloured_chapters();
    pictures.push(("OEBPS/nav.xhtml", nav(THREE_ENTRIES)));
    for name in ["a.epub", "b.epub"] {
        write(dir.join(name), &zipped(&book_with(&chapters, &pictures, "", NAV_ITEM), Mimetype::Stored));
    }
    // The tiles' room is 60 × 40 cells, and the status line takes a row.
    for (cols, rows) in [(60, 41), (100, 60)] {
        let mut view = Session::start(&dir, &["--protocol", "kitty", "--pager"], cols, rows);
        view.wait_until("the list", |t| chosen(t).as_deref() == Some("a.epub"));
        view.press("\r");
        whole_line_for_a_moment(&view, "Contents: T");
        view.wait_until("the whole book", |t| bar(t).contains(" a.epub ") && drawn(t, "page 1/4"));
        view.press("q");
        assert!(view.finish().success(), "{cols} × {rows}");
    }
}

/// A book's link to a file is never followed to one: one to a picture the book holds says it is a
/// file in the book, one to a file it doesn't hold says so, and the reader stays in the book, even
/// with files of those names where a path beside the `.epub` would lead.
#[cfg(all(feature = "epub", feature = "image"))]
#[test]
fn epub_a_link_to_a_file_is_said_and_no_file_beside_the_book_is_opened() {
    use crate::epub::{Mimetype, RED, book_of, plain, write, zipped};
    if !pty_works() {
        return;
    }
    let dir = empty_dir("epub-links");
    let body = r#"<p><a href="../Images/red.png">the plate</a>, <a href="../../notes.png">some notes</a> and <a href="https://example.com/">a site</a>.</p>"#;
    let files = book_of(&[body.to_owned()], &[("OEBPS/Images/red.png", plain(60, RED))]);
    let path = write(dir.join("shelf/book.epub"), &{
        std::fs::create_dir_all(dir.join("shelf")).unwrap();
        zipped(&files, Mimetype::Stored)
    });
    // Where each path would lead on disk: from the book's file as the chapter writes it, as the
    // book's document writes it, and from the book's top.
    for beside in [
        "Images/red.png",
        "notes.png",
        "shelf/OEBPS/Images/red.png",
        "shelf/clv-in-the-book/OEBPS/Images/red.png",
        "shelf/clv-not-in-the-book/notes.png",
    ] {
        std::fs::create_dir_all(dir.join(beside).parent().unwrap()).unwrap();
        std::fs::write(dir.join(beside), plain(60, RED)).unwrap();
    }
    let mut view = Session::start(&path, &["--protocol", "kitty", "--pager"], 100, 60);
    view.wait_until("the book", |t| drawn(t, "page 1/1"));
    let before = follow(&mut view, 3, "s");
    said_since(&view, before, "a file in the book, which clv doesn't open alone");
    let before = follow(&mut view, 3, "a");
    said_since(&view, before, "not in this book");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        last_bar(&view.text()).contains(" book.epub ") && place_now(&view.text()).is_some_and(|(page, _)| page == 1),
        "{}",
        last_bar(&view.text())
    );
    view.press("q");
    assert!(view.finish().success());
}

/// A book's links between its chapters are places in it: `f` and a label on a page of contents
/// lands on the chapter it names, at its heading, `u` comes back, and `F` copies the place's name
/// as the book's document has it.
#[cfg(all(feature = "epub", feature = "image"))]
#[test]
fn epub_links_between_chapters_are_followed_and_backspace_comes_back() {
    use crate::epub::{Mimetype, book_of, coloured_chapters, write, zipped};
    if !pty_works() {
        return;
    }
    let (mut chapters, pictures) = coloured_chapters();
    // The book's own page of contents, its first chapter: the others are the second to the fourth.
    let contents = r##"<h1>Contents</h1><p><a href="ch2.xhtml">Chapter 1</a></p><p><a href="ch3.xhtml#start">Chapter 2</a></p><p><a href="../Text/ch4.xhtml#start">Chapter 3</a></p>"##;
    chapters.insert(0, contents.to_owned());
    let path = write(crate::view::tmp("pager-epub-links", "book.epub"), &zipped(&book_of(&chapters, &pictures), Mimetype::Stored));
    let terminal = Terminal { clipboard: Some(true), ..KITTY };
    let mut view = Session::start_on(terminal, &path, &["--pager"], 100, 60);
    view.wait_until("the book", |t| drawn(t, "page 1/4"));
    let before = follow(&mut view, 3, "a");
    view.wait_until("the third chapter", |t| t.len() > before && place_now(&t[before..]).is_some_and(|(page, _)| page == 3));
    let before = view.text().len();
    view.press("\x7f");
    view.wait_until("back", |t| t.len() > before && place_now(&t[before..]).is_some_and(|(page, _)| page == 1));
    let before = follow(&mut view, 3, "d");
    view.wait_until("the fourth chapter, by a path with ..", |t| t.len() > before && place_now(&t[before..]).is_some_and(|(page, _)| page == 4));
    let before = view.text().len();
    view.press("\x7f");
    view.wait_until("back again", |t| t.len() > before && place_now(&t[before..]).is_some_and(|(page, _)| page == 1));
    let before = copy_link(&mut view, 3, "a");
    said_since(&view, before, "copied #c3-start");
    assert_eq!(copied_text(&view.text(), 1), "#c3-start");
    let before = copy_link(&mut view, 3, "s");
    said_since(&view, before, "copied #c2");
    view.press("q");
    assert!(view.finish().success());
}

/// Where the status line says the reader is, since `from` bytes of output: the page, once it is `page`.
#[cfg(all(feature = "epub", feature = "image"))]
fn at_page(view: &Session, what: &str, from: usize, page: usize) {
    view.wait_until(what, |t| t.len() > from && place_now(&t[from..]).is_some_and(|(at, _)| at == page));
}

/// A book's table of contents is a Contents page at its end, and `T` goes there from any page: `f`
/// and the label of its third entry lands on that entry's chapter, Backspace comes back to the
/// page, and again to where `T` was pressed. When the book opens, the status line says the key,
/// whole and for a moment.
#[cfg(all(feature = "epub", feature = "image"))]
#[test]
fn epub_t_goes_to_the_contents_page_at_the_books_end_and_backspace_comes_back() {
    use crate::epub::{Mimetype, NAV_ITEM, THREE_ENTRIES, book_with, coloured_chapters, nav, write, zipped};
    if !pty_works() {
        return;
    }
    let (chapters, mut pictures) = coloured_chapters();
    pictures.push(("OEBPS/nav.xhtml", nav(THREE_ENTRIES)));
    let book = book_with(&chapters, &pictures, "", NAV_ITEM);
    let path = write(crate::view::tmp("pager-epub-contents", "book.epub"), &zipped(&book, Mimetype::Stored));
    let mut view = Session::start_on(KITTY, &path, &["--pager"], 80, 60);
    // The three chapters, then the Contents page.
    whole_line_for_a_moment(&view, "Contents: T");
    view.wait_until("the book", |t| drawn(t, "page 1/4"));
    assert!(descendants_running(view.child.id(), "--worker epub").is_empty(), "a worker outlived its answers");
    let before = view.text().len();
    view.press("n");
    at_page(&view, "the second chapter", before, 2);
    let before = view.text().len();
    view.press("T");
    at_page(&view, "the Contents page, the last", before, 4);
    let before = follow(&mut view, 3, "d");
    at_page(&view, "the third chapter", before, 3);
    let before = view.text().len();
    view.press("\x7f");
    at_page(&view, "back on the Contents page", before, 4);
    let before = view.text().len();
    view.press("\x7f");
    at_page(&view, "back where T was pressed", before, 2);
    // Said when the book opens, not each time it is laid out again.
    let before = view.text().len();
    view.press("40]");
    view.wait_until("laid out narrower", |t| t.len() > before && bar(&t[before..]).contains("· width 40 ·"));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!view.text()[before..].contains("Contents: T"), "said again for a layout");
    // `?` lists the key.
    view.press("?");
    view.wait_until("listed", |t| bar(t).contains(" T to the contents: backspace comes back"));
    assert!(!bar(&view.text()).contains("\x1b[2m T "), "faint in a book that has contents");
    view.press("qq");
    assert!(view.finish().success());
}

/// `T` goes to the book's own page of contents where it has one: the chapter its package marks
/// (its nav document, in the reading order), or one of its first chapters that lists the others,
/// which the package doesn't mark, as Project Gutenberg's EPUB 3 books have it. Never then to the
/// Contents page made here, the book's last all the same.
#[cfg(all(feature = "epub", feature = "image"))]
#[test]
fn epub_t_goes_to_the_books_own_contents_marked_or_guessed() {
    use crate::epub::{Mimetype, NAV_ITEM, THREE_ENTRIES, book_with, coloured_chapters, nav, write, zipped};
    if !pty_works() {
        return;
    }
    let to_its_own = |name: &str, book: &[(String, Vec<u8>)], pages: usize| {
        let path = write(crate::view::tmp("pager-epub-own", name), &zipped(book, Mimetype::Stored));
        let mut view = Session::start_on(KITTY, &path, &["--pager"], 80, 60);
        whole_line_for_a_moment(&view, "Contents: T");
        view.wait_until("the book", |t| drawn(t, &format!("page 1/{pages}")));
        let before = view.text().len();
        view.press("3g");
        at_page(&view, "a chapter in the middle", before, 3);
        let before = view.text().len();
        view.press("T");
        at_page(&view, "the book's own contents", before, 1);
        let before = view.text().len();
        view.press("\x7f");
        at_page(&view, "back", before, 3);
        view.press("q");
        assert!(view.finish().success());
    };
    // Marked: the nav document first in the reading order, then the three chapters, then clv's page.
    // It lists two chapters, too few to be guessed: it is the book's own because the book says so.
    let (chapters, mut pictures) = coloured_chapters();
    let two = r#"<ol><li><a href="Text/ch1.xhtml">The red one</a></li><li><a href="Text/ch2.xhtml">The blue one</a></li></ol>"#;
    pictures.push(("OEBPS/nav.xhtml", nav(two)));
    let mut marked = book_with(&chapters, &pictures, "", NAV_ITEM);
    let package = marked.iter_mut().find(|(name, _)| name == "OEBPS/content.opf").unwrap();
    package.1 = String::from_utf8(package.1.clone()).unwrap().replace("<spine>", r#"<spine><itemref idref="nav"/>"#).into_bytes();
    to_its_own("marked.epub", &marked, 5);
    // Guessed: a first chapter that lists the three after it, in a book whose nav is no chapter.
    let listing = r#"<h1>A book</h1><p><a href="ch2.xhtml">The red one</a></p><p><a href="ch3.xhtml#start">The blue one</a></p><p><a href="ch4.xhtml">The green one</a></p>"#;
    let four = [vec![listing.to_owned()], chapters].concat();
    let entries = THREE_ENTRIES.replace("ch3", "ch4").replace("ch2", "ch3").replace("ch1", "ch2");
    pictures.pop();
    pictures.push(("OEBPS/nav.xhtml", nav(&entries)));
    to_its_own("guessed.epub", &book_with(&four, &pictures, "", NAV_ITEM), 5);
}

/// What `?` has written since `from` bytes of output, once it is whole: its rows.
fn keys_shown(view: &mut Session, from: usize) -> String {
    view.press("?");
    view.wait_until("the keys", |t| t.len() > from && bar(&t[from..]).contains("these keys: any key goes back"));
    view.text()[from..].to_owned()
}

/// A config for a test of the keys, to give `clv` with `--config`: `text`, in a file of `test`'s own.
fn keys_config(test: &str, text: &str) -> String {
    let path = crate::view::tmp(test, "keys.toml");
    std::fs::write(&path, text).unwrap();
    path.to_str().unwrap().to_owned()
}

/// A key that the config binds does its command, and is first among that command's keys in `?`
/// and in the status line's reminder. With Enter bound again, a number and Enter still go to that
/// page. A key unbound does nothing.
#[test]
fn config_keys_bound_again_do_their_command_and_are_first_in_the_keys_listed() {
    if !pty_works() {
        return;
    }
    let config = keys_config("config-keys", "[keys]\nx = \"quit\"\nq = \"none\"\n\n[keys.pages]\nenter = \"next\"\n");
    let mut view = Session::start_on(KITTY, &fixture("config-keys", 6), &["--pager", "--per-row", "1", "--config", &config], 100, 40);
    view.wait_until("page 1", |t| drawn(t, "page 1/6"));
    // The reminder says the key that quits now.
    assert!(bar(&view.text()).contains(" · x quit"), "{}", last_bar(&view.text()));
    let from = view.text().len();
    let shown = bar(&keys_shown(&mut view, from));
    assert!(shown.contains(" x esc ctrl-c quit"), "x isn't first among the keys that quit: {shown:?}");
    // Beside `p`, as the first key of the next page is beside the first of the previous.
    assert!(shown.contains(" enter p n → ← the next or the previous screen of pages"), "{shown:?}");
    assert!(shown.contains(" j k ↓ ↑ ctrl-n ctrl-p a few rows down or up"), "Enter is still listed as a row down: {shown:?}");
    assert!(shown.contains(" 12 enter 12g page 12"), "{shown:?}");
    // Any key leaves the keys. Enter alone is the next page now; after a number, that page.
    view.press(" ");
    let before = view.text().len();
    view.press("\r");
    view.wait_until("the next page", |t| drawn(&t[before..], "page 2/6"));
    let before = view.text().len();
    view.press("5\r");
    view.wait_until("page 5", |t| drawn(&t[before..], "page 5/6"));
    // `q` is bound to nothing: it does nothing.
    view.press("q");
    std::thread::sleep(Duration::from_millis(300));
    assert!(view.child.try_wait().unwrap().is_none(), "`q` quit, unbound");
    view.press("x");
    assert!(view.finish().success());
}

/// A reader's first config: the generated file, with the line of the key that quits changed to
/// another key. The line is there once, under `[keys]`, and binds the key in a file's pages and
/// in a list alike: `x` quits in both, `q` still does, and `?` lists `x` first.
#[test]
fn config_keys_one_line_of_the_generated_config_changed_binds_the_key_in_both_keymaps() {
    if !pty_works() {
        return;
    }
    let file = crate::view::tmp("config-keys-generated", "generated.toml");
    let _ = std::fs::remove_file(&file);
    let config = file.to_str().unwrap();
    let out = crate::bin::clv().args(["--generate-config", "--config", config]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let template = std::fs::read_to_string(&file).unwrap();
    assert_eq!(template.lines().filter(|line| *line == "# q = \"quit\"").count(), 1, "the line is there once:\n{template}");
    std::fs::write(&file, template.replace("# q = \"quit\"\n", "x = \"quit\"\n")).unwrap();
    let pdf = fixture("config-keys-generated", 2);
    let other = pdf.with_file_name("other.pdf");
    std::fs::write(&other, crate::fixtures::letter("Other", 1)).unwrap();
    let alone = ["--pager", "--per-row", "1", "--config", config];
    let listed = [other.to_str().unwrap(), "--config", config];
    for (args, shows) in [(&alone[..], "page 1/2"), (&listed[..], "1/2 ·")] {
        // `?` lists `x` first among the keys that quit, and `x` quits.
        let mut view = Session::start_on(KITTY, &pdf, args, 100, 40);
        view.wait_until("shown", |t| bar(t).contains(shows));
        assert!(bar(&view.text()).contains(" · x quit"), "{shows}: {}", last_bar(&view.text()));
        let from = view.text().len();
        let shown = bar(&keys_shown(&mut view, from));
        assert!(shown.contains(" x esc q ctrl-c quit"), "{shows}: {shown:?}");
        view.press(" x");
        assert!(view.finish().success(), "{shows}");
        // And `q` still does.
        let mut view = Session::start_on(KITTY, &pdf, args, 100, 40);
        view.wait_until("shown", |t| bar(t).contains(shows));
        view.press("q");
        assert!(view.finish().success(), "{shows}");
    }
}

/// With every key that quits or leaves unbound, `ctrl-c` still quits: a config can't leave the
/// reader without a way out.
#[test]
fn config_keys_ctrl_c_quits_with_q_unbound() {
    if !pty_works() {
        return;
    }
    let config = keys_config("config-keys-out", "[keys]\nq = \"none\"\nesc = \"none\"\nbackspace = \"none\"\n");
    let mut view = Session::start_on(KITTY, &fixture("config-keys-out", 2), &["--pager", "--per-row", "1", "--config", &config], 100, 40);
    view.wait_until("page 1", |t| drawn(t, "page 1/2"));
    assert!(bar(&view.text()).contains(" · ctrl-c quit"), "the reminder names the one way out: {}", last_bar(&view.text()));
    view.press("q\x1b\x7f");
    std::thread::sleep(Duration::from_millis(1500));
    assert!(view.child.try_wait().unwrap().is_none(), "an unbound key quit");
    view.press("\x03");
    assert!(view.finish().success());
}

/// A setting's line in `?` shows its keys as bound, the phrase of the width too: with `w` bound to
/// `increase-width`, `72w sets 72`.
#[cfg(feature = "word")]
#[test]
fn config_keys_show_in_a_settings_line_and_its_phrase() {
    if !pty_works() {
        return;
    }
    let config = keys_config("config-keys-phrase", "[keys]\nw = \"increase-width\"\nM = \"cycle-theme\"\n\"]\" = \"none\"\n");
    let mut view = Session::start_on(KITTY, &long_markdown("config-keys-phrase", "long.md"), &["--pager", "--config", &config], 100, 40);
    view.wait_until("page 1", |t| t.contains("  page 1/"));
    let from = view.text().len();
    let shown = bar(&keys_shown(&mut view, from));
    assert!(shown.contains(" [ w 12w 4 characters a line narrower, or wider; 72w sets 72, 0w the window's"), "{shown:?}");
    assert!(shown.contains(" M D light on dark, or dark on light"), "{shown:?}");
    // And the key does what it is bound to: the document is laid out again, 4 characters wider.
    view.press(" ");
    let before = view.text().len();
    view.press("72w");
    view.wait_until("the width set", |t| bar(&t[before..]).contains("width 72"));
    view.press("q");
    assert!(view.finish().success());
}

/// `?` in a window of 80 columns cuts no line: what a line says goes on over the next row. A
/// setting's line says what its keys do and then two of the formats they do it to. The keys that
/// do nothing on what is shown are faint, by what the file says of itself: on a PDF `D`, `[ ]`,
/// `{ }`, `s` and `T`, and on a Markdown document `T` alone. As text cells too.
#[cfg(feature = "word")]
#[test]
fn the_keys_at_80_columns_are_whole_and_those_that_do_nothing_here_are_faint() {
    if !pty_works() {
        return;
    }
    const SETTINGS: [&str; 4] = ["D", "[ ] 12]", "{ } 12}", "s"];
    let faint = |keys: &str| format!("\x1b[2m {keys} ");
    let listed = |shown: &str, keys: &str| bar(shown).contains(&format!(" {keys} "));
    for terminal in [KITTY, TEXT_ONLY] {
        let mut view = Session::start_on(terminal, &fixture("keys-faint", 2), &["--pager"], 80, 40);
        view.wait_until("page 1", |t| drawn(t, "page 1/2"));
        let from = view.text().len();
        let shown = keys_shown(&mut view, from);
        assert!(!shown.contains('…'), "a line cut: {shown:?}");
        for keys in SETTINGS.iter().chain(&["T"]) {
            assert!(bar(&shown).contains(&faint(keys)), "{keys} isn't faint on a PDF: {shown:?}");
        }
        for keys in ["q", "?", "f", "/", "c", "+ -", "backspace"] {
            assert!(listed(&shown, keys) && !bar(&shown).contains(&faint(keys)), "{keys} is faint, or isn't listed: {shown:?}");
        }
        assert_eq!(shown.matches("\x1b[2m").count(), shown.matches("\x1b[22m").count(), "a faint row goes back to the rest");
        // Each of its lines whole, over two rows where it takes them.
        let said = bar(&shown);
        assert!(said.contains(" D light on dark, or dark on light (Mermaid, Markdown and") && said.contains("others)"), "{said:?}");
        assert!(
            said.contains(" [ ] 12] 4 characters a line narrower, or wider; 72] sets 72, 0] the")
                && said.contains("window's (Markdown, HTML and others)")
        );
        view.press("qq");
        assert!(view.finish().success());
    }
    let mut view = Session::start_on(KITTY, &long_markdown("keys-faint", "long.md"), &["--pager"], 80, 40);
    view.wait_until("page 1", |t| t.contains("  page 1/"));
    let from = view.text().len();
    let shown = keys_shown(&mut view, from);
    assert!(!shown.contains('…'), "a line cut: {shown:?}");
    for keys in SETTINGS {
        assert!(listed(&shown, keys) && !bar(&shown).contains(&faint(keys)), "{keys} is faint on Markdown, or isn't listed: {shown:?}");
    }
    assert!(bar(&shown).contains(&faint("T")) && shown.matches("\x1b[2m").count() == 1, "the contents alone are faint: {shown:?}");
    view.press("qq");
    assert!(view.finish().success());
}

/// A file that says nothing of where its contents are isn't offered the key: a PDF's status line
/// never says it, `T` says there is nowhere to go, and `?` doesn't list it.
#[test]
fn t_is_offered_only_where_contents_are_declared_never_on_a_pdf() {
    if !pty_works() {
        return;
    }
    let mut view = Session::start_on(KITTY, &fixture("no-contents", 3), &["--pager"], 80, 40);
    view.wait_until("page 1", |t| drawn(t, "page 1/3"));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!view.text().contains("Contents"), "said of a PDF");
    view.press("T");
    view.wait_until("said", |t| bar(t).contains("no contents to go to here"));
    assert!(place_now(&view.text()).is_some_and(|(page, _)| page == 1));
    view.press("?");
    view.wait_until("the keys", |t| bar(t).contains("these keys"));
    assert!(bar(&view.text()).contains("\x1b[2m T to the contents: backspace comes back\x1b[22m"), "not listed faint for a PDF");
    view.press("qq");
    assert!(view.finish().success());
}
