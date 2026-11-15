//! Opening a file in the app the desktop opens it with: the pager's `o`. ADR-0034 has why it goes
//! the way it does.
//!
//! The platform's opener is run with the file's real path, a single argument and no shell, and is
//! never waited for: its stdin, stdout and stderr are null, so nothing it writes reaches the screen
//! and no pipe ties it, or an app it starts, to `clv`; it has a process group of its own (`open`),
//! or through `setsid` a session of its own (`xdg-open`, never run without it), so `Ctrl-C` in `clv`
//! doesn't reach it and an app for the terminal can't take `clv`'s. A thread waits for it as long as it runs, so it is never left a zombie; the
//! status line says how it went, or past [`SAID_WAIT`] that it was handed over.
//!
//! A web address a link leads to goes the same way ([`Launches::start_address`]): the same
//! opener, the address its single argument, once the viewer has checked it is one. Where no opener
//! may be run (over SSH, with no desktop, with none installed), it says so, for the address to be
//! copied instead.
//!
//! Each opener is a [`Launcher`] in [`LAUNCHERS`], tried in order as the backends are: another
//! desktop or platform is another entry. Named so beside `Opener`, which opens a list's files.

use std::ffi::OsString;
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::{Duration, Instant};

/// The openers, in the order they are asked whether they apply here: the first that does opens.
pub(crate) const LAUNCHERS: &[&dyn Launcher] = &[&Open, &XdgOpen];

/// How long the status line waits for an opener to answer before it says the file was handed to it:
/// some of `xdg-open`'s ways wait for the app to exit. The thread waits on, as long as it runs.
pub(crate) const SAID_WAIT: Duration = Duration::from_secs(5);
/// The most launches under way at once, their openers not yet ended: `o` past them waits for one.
pub(crate) const MAX_UNDER_WAY: usize = 4;
/// How often a launch is looked at while the status line waits for it.
const POLL: Duration = Duration::from_millis(50);

/// What a launcher decides by: the system and its environment, given, never read by a launcher
/// itself, so that each is tested on any machine.
#[derive(Clone, Debug, Default)]
pub(crate) struct Here {
    /// The operating system, as `std::env::consts::OS` names it: `macos`, `linux`, `freebsd`.
    pub(crate) os: &'static str,
    /// Whether a desktop session is there to open an app in: `DISPLAY` or `WAYLAND_DISPLAY` set,
    /// and not empty. Inside tmux, as the pane has them, even if stale from an earlier attach.
    pub(crate) display: bool,
    /// Where programs are found: `PATH`.
    pub(crate) path: Option<OsString>,
}

impl Here {
    /// This process's.
    pub(crate) fn now() -> Self {
        let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
        Self { os: std::env::consts::OS, display: set("DISPLAY") || set("WAYLAND_DISPLAY"), path: std::env::var_os("PATH") }
    }

    /// The program called `name`: in the first of `PATH`'s directories that has it, an executable
    /// file. Only absolute directories: an empty or relative one would be wherever `clv` was started.
    pub(crate) fn find(&self, name: &str) -> Option<PathBuf> {
        use std::os::unix::fs::PermissionsExt as _;
        let executable = |path: &Path| std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
        let path = self.path.as_ref()?;
        std::env::split_paths(path).filter(|dir| dir.is_absolute()).map(|dir| dir.join(name)).find(|program| executable(program))
    }
}

/// A platform's opener: the program that opens a file in the app the desktop opens it with, and a
/// folder in the file manager. Choosing the app (`O`) would add a method listing the apps, and a
/// command for the one picked, each with a default, so that no launcher breaks.
pub(crate) trait Launcher: Sync {
    /// Its program's name, as the status line says it: `open`, `xdg-open`.
    fn name(&self) -> &'static str;
    /// Whether it opens files `here`.
    fn applies(&self, here: &Here) -> bool;
    /// The command that opens the file at `path`, real and absolute, or with `folder` shows the
    /// folder at `path` in the file manager, its program found on `here`'s `PATH`; or why it can't
    /// be had. No shell: the path is one argument, as it is. The caller nulls its stdin, stdout
    /// and stderr; the command puts it in a process group, or a session, of its own.
    fn command(&self, path: &Path, folder: bool, here: &Here) -> Result<Command, String>;
    /// Its exit status, not a success, in words: what it says of why it didn't open the file.
    fn why(&self, status: ExitStatus) -> String;
}

/// macOS's `open`, through Launch Services. A folder is shown in Finder (`-R`), never opened: Finder
/// takes a folder for a package (an app, which `open` launches) by its extension or by a flag in its
/// metadata, so no name tells them apart.
pub(crate) struct Open;

impl Launcher for Open {
    fn name(&self) -> &'static str {
        "open"
    }

    fn applies(&self, here: &Here) -> bool {
        here.os == "macos"
    }

    fn command(&self, path: &Path, folder: bool, here: &Here) -> Result<Command, String> {
        let program = here.find(self.name()).ok_or_else(|| not_found(self.name()))?;
        let mut command = Command::new(program);
        if folder {
            command.arg("-R");
        }
        // Absolute, so never taken for an option.
        command.arg(path).process_group(0);
        Ok(command)
    }

    fn why(&self, status: ExitStatus) -> String {
        match status.code() {
            Some(1) => "open couldn't open it (1): it is gone, or no app opens it".into(),
            _ => failed(self.name(), status),
        }
    }
}

/// freedesktop.org's `xdg-open`, on Linux and the BSDs, where a desktop session is there
/// (`DISPLAY` or `WAYLAND_DISPLAY`). It reaches most desktops through their own tools. Run through
/// `setsid`, so it has no controlling terminal: an app for the terminal it starts fails rather than
/// taking `clv`'s, or writing to it through `/dev/tty`. Without `setsid` on `PATH` it isn't run at
/// all: in a process group of its own it would keep `clv`'s terminal, and such an app could write
/// raw bytes to the screen. `setsid` forks when it starts as a process group's leader, and its
/// parent then exits at once, its program's exit status lost; so it isn't put in a group of its own
/// first, and gives itself a session, a group with it.
pub(crate) struct XdgOpen;

/// The systems `xdg-open` serves.
const FREEDESKTOP: &[&str] = &["linux", "freebsd", "openbsd", "netbsd", "dragonfly"];

impl Launcher for XdgOpen {
    fn name(&self) -> &'static str {
        "xdg-open"
    }

    fn applies(&self, here: &Here) -> bool {
        FREEDESKTOP.contains(&here.os) && here.display
    }

    /// A folder opens in the file manager as a file does in its app.
    fn command(&self, path: &Path, _folder: bool, here: &Here) -> Result<Command, String> {
        let program = here.find(self.name()).ok_or_else(|| not_found(self.name()))?;
        let setsid = here.find("setsid").ok_or(NO_SETSID)?;
        let mut command = Command::new(setsid);
        // Absolute, so never taken for an option.
        command.arg(program).arg(path);
        Ok(command)
    }

    /// As its manual has them.
    fn why(&self, status: ExitStatus) -> String {
        match status.code() {
            Some(1) => "xdg-open didn't take what it was given (1)".into(),
            Some(2) => "xdg-open didn't find it (2)".into(),
            Some(3) => "xdg-open lacks a tool it needs (3)".into(),
            Some(4) => "xdg-open's action failed (4)".into(),
            _ => failed(self.name(), status),
        }
    }
}

/// Why `xdg-open` isn't run without `setsid` ([`XdgOpen`]).
const NO_SETSID: &str = "there is no setsid here to keep clv's terminal from its app";

/// Why `program` can't be had.
fn not_found(program: &str) -> String {
    format!("there is no {program} to open it with")
}

/// `status` of `program`, in words, where it says nothing of its own.
fn failed(program: &str, status: ExitStatus) -> String {
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("{program} failed ({code})"),
        (None, Some(signal)) => format!("{program} was killed (signal {signal})"),
        (None, None) => format!("{program} failed"),
    }
}

/// Why no opener is run here.
enum Refused {
    /// The terminal is on another machine: an app opened here isn't on the reader's screen.
    Elsewhere,
    /// No launcher applies: no desktop session to open an app in.
    NoDesktop,
    /// The launcher's program can't be had, and why.
    NoOpener(String),
}

/// A launch under way: its opener started, waited for on a thread of its own.
struct Launch {
    /// What the status line calls it: `report.pdf`.
    what: String,
    launcher: &'static dyn Launcher,
    since: Instant,
    /// Whether the status line has said it was handed over, having waited [`SAID_WAIT`].
    said: bool,
    /// The opener's process.
    #[cfg_attr(not(test), expect(dead_code, reason = "tests look for its process"))]
    pid: u32,
    /// How it ended, from its thread.
    ended: Receiver<std::io::Result<ExitStatus>>,
}

/// The launches under way for the pager, never making it wait.
pub(crate) struct Launches {
    /// The terminal runs on this machine, so an app opened here is on the reader's screen.
    local: bool,
    here: Here,
    launchers: &'static [&'static dyn Launcher],
    under_way: Vec<Launch>,
}

impl Launches {
    /// For a terminal on this machine or not (`local`), with this process's environment.
    pub(crate) fn new(local: bool) -> Self {
        Self::with(local, Here::now(), LAUNCHERS)
    }

    fn with(local: bool, here: Here, launchers: &'static [&'static dyn Launcher]) -> Self {
        Self { local, here, launchers, under_way: Vec::new() }
    }

    /// Open `what`, the file at `path` (real and absolute), or with `folder` show the folder at
    /// `path`, with the first launcher that applies here: its opener started, not waited for. What
    /// the status line says.
    pub(crate) fn start(&mut self, what: &str, path: &Path, folder: bool, now: Instant) -> String {
        match self.started(what, path, folder, now) {
            Ok(said) => said,
            Err(Refused::Elsewhere) => format!("{what} isn't opened: over SSH, its app would open on the machine clv runs on, not yours"),
            Err(Refused::NoDesktop) => format!("{what} isn't opened: there is no desktop here to open it in"),
            Err(Refused::NoOpener(why)) => format!("{what} isn't opened: {why}"),
        }
    }

    /// Open the web address `address`, which the viewer has checked is one (`http`, `https` or
    /// `mailto`, so never taken for an option), called `what` on the status line, in the app the
    /// desktop opens it with: the same openers, the address their single argument. What the status
    /// line says; or, where no opener may be run here, why not, in a few words, for the address to
    /// be copied instead: over SSH, with no desktop, or with no opener to run.
    pub(crate) fn start_address(&mut self, what: &str, address: &str, now: Instant) -> Result<String, String> {
        self.started(what, Path::new(address), false, now).map_err(|refused| match refused {
            Refused::Elsewhere => "over SSH".to_owned(),
            Refused::NoDesktop => "no desktop here".to_owned(),
            Refused::NoOpener(why) => why,
        })
    }

    /// [`Launches::start`], or why no opener is run here.
    fn started(&mut self, what: &str, path: &Path, folder: bool, now: Instant) -> Result<String, Refused> {
        if !self.local {
            return Err(Refused::Elsewhere);
        }
        if self.under_way.len() >= MAX_UNDER_WAY
            && let Some(launch) = self.under_way.first()
        {
            return Ok(format!("{} is still being opened: o waits for it", launch.what));
        }
        let Some(launcher) = self.launchers.iter().copied().find(|launcher| launcher.applies(&self.here)) else {
            return Err(Refused::NoDesktop);
        };
        let mut command = match launcher.command(path, folder, &self.here) {
            Ok(command) => command,
            Err(why) => return Err(Refused::NoOpener(why)),
        };
        command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        // The thread first, so that an opener started always has one to wait for it.
        let (give, take) = sync_channel::<Child>(1);
        let (end, ended) = sync_channel(1);
        let thread = std::thread::Builder::new().name("view-launch".into()).spawn(move || {
            if let Ok(mut child) = take.recv() {
                let _ = end.send(child.wait());
            }
        });
        if let Err(e) = thread {
            return Ok(format!("{what} isn't opened: {e}"));
        }
        let child = match command.spawn() {
            Ok(child) => child,
            // Its thread ends, with nothing sent to it.
            Err(e) => return Ok(format!("{what} isn't opened: {} didn't start ({e})", launcher.name())),
        };
        let pid = child.id();
        // The thread waits for it, and never ends before it has it. Should it be gone all the
        // same, the opener is reaped here if it has ended, and let go of otherwise.
        if let Err(mut back) = give.send(child) {
            let _ = back.0.try_wait();
            return Ok(format!("{what}: handed to {}, which can't be waited for", launcher.name()));
        }
        self.under_way.push(Launch { what: what.to_owned(), launcher, since: now, said: false, pid, ended });
        Ok(format!("opening {what}…"))
    }

    /// What became of the launches, at `now`: one ended (opened, or why not), or one handed over
    /// [`SAID_WAIT`] ago without a word. The latest news, when there is any. Once said to be handed
    /// over, a launch is still waited for, and only its failure said.
    pub(crate) fn on(&mut self, now: Instant) -> Option<String> {
        let mut news = None;
        self.under_way.retain_mut(|launch| {
            let (what, name) = (&launch.what, launch.launcher.name());
            match launch.ended.try_recv() {
                Err(TryRecvError::Empty) => {
                    if !launch.said && now.saturating_duration_since(launch.since) >= SAID_WAIT {
                        launch.said = true;
                        news = Some(format!("{what}: handed to {name}, which hasn't said how it went"));
                    }
                    true
                }
                Ok(Ok(status)) if status.success() => {
                    if !launch.said {
                        news = Some(format!("opened {what}"));
                    }
                    false
                }
                Ok(Ok(status)) => {
                    news = Some(format!("{what}: {}", launch.launcher.why(status)));
                    false
                }
                Ok(Err(e)) => {
                    news = Some(format!("{what}: waiting for {name}: {e}"));
                    false
                }
                Err(TryRecvError::Disconnected) => {
                    news = Some(format!("{what}: {name} can't be waited for"));
                    false
                }
            }
        });
        news
    }

    /// How soon [`Launches::on`] has news to look for: soon while the status line waits for a launch.
    /// `None` with none it waits for.
    pub(crate) fn wait(&self) -> Option<Duration> {
        self.under_way.iter().any(|launch| !launch.said).then_some(POLL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `PATH` of these directories.
    fn path(dirs: &[&Path]) -> Option<OsString> {
        std::env::join_paths(dirs).ok()
    }

    /// A directory of its own for `test`, holding executable scripts `programs`, each its name and
    /// its body.
    fn bin(test: &str, programs: &[(&str, &str)]) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("clview-launch-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, body) in programs {
            let program = dir.join(name);
            std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir
    }

    /// The arguments `command` runs with, its program first.
    fn argv(command: &Command) -> Vec<String> {
        std::iter::once(command.get_program()).chain(command.get_args()).map(|a| a.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn which_launcher_applies_goes_by_the_system_and_a_desktop_session() {
        let here = |os, display| Here { os, display, path: None };
        let applying = |here: &Here| LAUNCHERS.iter().find(|l| l.applies(here)).map(|l| l.name());
        // macOS has Launch Services, with or without X11's variables.
        assert_eq!(applying(&here("macos", false)), Some("open"));
        assert_eq!(applying(&here("macos", true)), Some("open"));
        // Linux and the BSDs, only with a desktop session.
        for os in ["linux", "freebsd", "openbsd", "netbsd", "dragonfly"] {
            assert_eq!(applying(&here(os, true)), Some("xdg-open"), "{os}");
            assert_eq!(applying(&here(os, false)), None, "{os}");
        }
        // Nothing elsewhere, yet.
        assert_eq!(applying(&here("windows", true)), None);
        assert_eq!(applying(&here("haiku", true)), None);
    }

    #[test]
    fn the_environment_says_a_desktop_session_only_when_set_and_not_empty() {
        // Read here as `Here::now` reads it, without changing this process's environment.
        let here = Here::now();
        let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
        assert_eq!(here.display, set("DISPLAY") || set("WAYLAND_DISPLAY"));
        assert_eq!(here.os, std::env::consts::OS);
    }

    #[test]
    fn a_launch_command_has_the_path_as_one_argument_and_its_program_found_on_path() {
        let fakes = bin("argv", &[("open", "exit 0"), ("xdg-open", "exit 0"), ("setsid", "exec \"$@\"")]);
        let not_executable = bin("argv-plain", &[]);
        std::fs::write(not_executable.join("open"), "#!/bin/sh\n").unwrap();
        let odd = Path::new("/home/a b/-x 'q\" $HOME.pdf");
        let mac = Here { os: "macos", display: false, path: path(&[&not_executable, Path::new("relative"), &fakes]) };
        let open = |p, folder| argv(&Open.command(p, folder, &mac).unwrap());
        let program = fakes.join("open").to_string_lossy().into_owned();
        assert_eq!(open(odd, false), [program.as_str(), "/home/a b/-x 'q\" $HOME.pdf"]);
        // A folder is shown in Finder, never opened.
        assert_eq!(open(Path::new("/Users/a/Pictures.app"), true), [program.as_str(), "-R", "/Users/a/Pictures.app"]);
        // On Linux through setsid, and not without it: the opener, and an app for the terminal it
        // starts, would keep clv's terminal and could write to it.
        let linux = Here { os: "linux", display: true, path: path(&[&fakes]) };
        let (setsid, xdg) = (fakes.join("setsid").to_string_lossy().into_owned(), fakes.join("xdg-open").to_string_lossy().into_owned());
        let through = argv(&XdgOpen.command(odd, false, &linux).unwrap());
        assert_eq!(through, [setsid.as_str(), &xdg, "/home/a b/-x 'q\" $HOME.pdf"]);
        let alone = bin("argv-alone", &[("xdg-open", "exit 0")]);
        let no_setsid = Here { path: path(&[&alone]), ..linux.clone() };
        assert_eq!(XdgOpen.command(odd, true, &no_setsid).err().as_deref(), Some(NO_SETSID));
        // No program, no command; and no PATH, none either.
        let empty = Here { path: path(&[&not_executable]), ..linux };
        assert_eq!(XdgOpen.command(odd, false, &empty).err().as_deref(), Some("there is no xdg-open to open it with"));
        assert!(Open.command(odd, false, &Here { path: None, ..mac }).is_err());
    }

    /// Whether the process that ran `command`, a fake that notes its own `pid pgid` in `noted`,
    /// led a process group of its own.
    fn group_leader(mut command: Command, noted: &Path) -> bool {
        let _ = std::fs::remove_file(noted);
        assert!(command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success());
        let ids: Vec<String> = std::fs::read_to_string(noted).unwrap().split_whitespace().map(str::to_owned).collect();
        assert_eq!(ids.len(), 2, "{ids:?}");
        ids.first() == ids.get(1)
    }

    /// `setsid` forks when it starts as a process group's leader, and the opener's exit status is
    /// lost: so with it the command has no group of its own, and `setsid` makes one. `open` leads
    /// one of its own.
    #[test]
    fn with_setsid_the_opener_leads_no_group_and_open_leads_its_own() {
        let noted = std::env::temp_dir().join(format!("clview-launch-group-{}", std::process::id()));
        let note = format!("ps -o pid= -o pgid= -p $$ > '{}'", noted.display());
        let with = bin("group-setsid", &[("setsid", &format!("{note}; exec \"$@\"")), ("xdg-open", "exit 0"), ("open", &note)]);
        let here = |dir: &Path| Here { os: "linux", display: true, path: path(&[dir]) };
        assert!(!group_leader(XdgOpen.command(Path::new("/a.pdf"), false, &here(&with)).unwrap(), &noted), "setsid started as a group's leader");
        let mac = Here { os: "macos", ..here(&with) };
        assert!(group_leader(Open.command(Path::new("/a.pdf"), false, &mac).unwrap(), &noted), "open in no group of its own");
        let _ = std::fs::remove_file(&noted);
    }

    #[test]
    fn an_exit_status_is_said_in_the_launchers_words() {
        let status = |code: i32| ExitStatus::from_raw(code << 8);
        assert_eq!(XdgOpen.why(status(2)), "xdg-open didn't find it (2)");
        assert_eq!(XdgOpen.why(status(3)), "xdg-open lacks a tool it needs (3)");
        assert_eq!(XdgOpen.why(status(4)), "xdg-open's action failed (4)");
        assert_eq!(XdgOpen.why(status(9)), "xdg-open failed (9)");
        assert_eq!(Open.why(status(1)), "open couldn't open it (1): it is gone, or no app opens it");
        // Killed by SIGKILL.
        assert_eq!(Open.why(ExitStatus::from_raw(9)), "open was killed (signal 9)");
    }

    /// A launcher for the tests, applying everywhere, whose opener is `sh -c` running its script
    /// with the path as `$1`.
    struct Fake(&'static str);

    impl Launcher for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }

        fn applies(&self, _here: &Here) -> bool {
            true
        }

        fn command(&self, path: &Path, _folder: bool, _here: &Here) -> Result<Command, String> {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", self.0, "fake"]).arg(path).process_group(0);
            Ok(command)
        }

        fn why(&self, status: ExitStatus) -> String {
            failed(self.name(), status)
        }
    }

    const TRUE: &dyn Launcher = &Fake("true");
    const FALSE: &dyn Launcher = &Fake("exit 3");
    const SLOW: &dyn Launcher = &Fake("sleep 1");
    const STUCK: &dyn Launcher = &Fake("sleep 30");

    fn launches(launcher: &'static &'static dyn Launcher, local: bool) -> Launches {
        Launches::with(local, Here::default(), std::slice::from_ref(launcher))
    }

    /// `launches` looked at until it has news, which it gives.
    fn news(launches: &mut Launches, now: Instant) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(said) = launches.on(now) {
                return said;
            }
            assert!(Instant::now() < deadline, "no news");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Whether process `pid` is there at all, a zombie or not: `ps` lists it.
    fn exists(pid: u32) -> bool {
        let out = Command::new("ps").args(["-o", "pid=", "-p", &pid.to_string()]).output().unwrap();
        !String::from_utf8_lossy(&out.stdout).trim().is_empty()
    }

    #[test]
    fn a_launch_says_it_is_opening_then_that_it_opened_or_why_not() {
        let now = Instant::now();
        let mut opened = launches(&TRUE, true);
        assert_eq!(opened.start("report.pdf", Path::new("/r/report.pdf"), false, now), "opening report.pdf…");
        assert_eq!(opened.wait(), Some(POLL));
        assert_eq!(news(&mut opened, now), "opened report.pdf");
        assert!(opened.under_way.is_empty() && opened.wait().is_none());
        let mut failing = launches(&FALSE, true);
        failing.start("report.pdf", Path::new("/r/report.pdf"), false, now);
        assert_eq!(news(&mut failing, now), "report.pdf: fake failed (3)");
    }

    #[test]
    fn over_ssh_or_with_no_launcher_here_nothing_is_started() {
        let now = Instant::now();
        let mut remote = launches(&TRUE, false);
        assert_eq!(
            remote.start("a.pdf", Path::new("/a.pdf"), false, now),
            "a.pdf isn't opened: over SSH, its app would open on the machine clv runs on, not yours"
        );
        assert!(remote.under_way.is_empty());
        let here = Here { os: "linux", display: false, path: None };
        let mut nowhere = Launches::with(true, here, LAUNCHERS);
        assert_eq!(nowhere.start("a.pdf", Path::new("/a.pdf"), false, now), "a.pdf isn't opened: there is no desktop here to open it in");
        let here = Here { os: "linux", display: true, path: None };
        let mut no_program = Launches::with(true, here, LAUNCHERS);
        assert_eq!(no_program.start("a.pdf", Path::new("/a.pdf"), false, now), "a.pdf isn't opened: there is no xdg-open to open it with");
        assert!(nowhere.under_way.is_empty() && no_program.under_way.is_empty());
    }

    /// A web address goes to the same openers, as their single argument, whatever is in it, and is
    /// said opened as a file is; where no opener may be run (over SSH, with no desktop, with none
    /// installed) nothing is started, and why comes back in a few words, for it to be copied instead.
    #[test]
    fn a_web_address_is_launched_as_one_argument_or_refused_with_why() {
        let now = Instant::now();
        let address = "https://example.com/a b?c=$(touch /tmp/x)&d='e';f";
        // The fake opener notes what it is run with: its one argument, as it is.
        let noted = std::env::temp_dir().join(format!("clview-launch-address-{}", std::process::id()));
        let _ = std::fs::remove_file(&noted);
        let script: &'static str = Box::leak(format!("printf '%s\\n' \"$#\" \"$1\" > '{}'", noted.display()).into_boxed_str());
        let fake: &'static &'static dyn Launcher = Box::leak(Box::new(Box::leak(Box::new(Fake(script))) as &'static dyn Launcher));
        let mut opened = launches(fake, true);
        assert_eq!(opened.start_address("example.com", address, now), Ok("opening example.com…".into()));
        assert_eq!(news(&mut opened, now), "opened example.com");
        assert_eq!(std::fs::read_to_string(&noted).unwrap(), format!("1\n{address}\n"));
        std::fs::remove_file(&noted).unwrap();
        // The real ones: `open` takes it as it takes a path, and `xdg-open` after `setsid`.
        let here = |os| Here { os, display: true, path: path(&[&bin("address", &[("open", "true"), ("xdg-open", "true"), ("setsid", "true")])]) };
        let open = Open.command(Path::new("mailto:a@example.com"), false, &here("macos")).unwrap();
        assert_eq!(argv(&open).get(1..), Some(&["mailto:a@example.com".to_owned()][..]));
        let xdg = XdgOpen.command(Path::new("https://example.com/"), false, &here("linux")).unwrap();
        assert_eq!(argv(&xdg).last().map(String::as_str), Some("https://example.com/"));
        // Refused, with why, and nothing started.
        let mut remote = launches(&TRUE, false);
        assert_eq!(remote.start_address("example.com", address, now), Err("over SSH".into()));
        let mut nowhere = Launches::with(true, Here { os: "linux", display: false, path: None }, LAUNCHERS);
        assert_eq!(nowhere.start_address("example.com", address, now), Err("no desktop here".into()));
        let mut no_program = Launches::with(true, Here { os: "macos", display: false, path: None }, LAUNCHERS);
        assert_eq!(no_program.start_address("example.com", address, now), Err("there is no open to open it with".into()));
        assert!(remote.under_way.is_empty() && nowhere.under_way.is_empty() && no_program.under_way.is_empty());
    }

    /// Past five seconds the status line says the file was handed over; the thread waits on, and
    /// the opener, once it ends, is reaped: no zombie is left.
    #[test]
    fn a_launch_is_said_handed_over_after_five_seconds_and_reaped_when_it_ends() {
        let then = Instant::now();
        let mut slow = launches(&SLOW, true);
        slow.start("big.pdf", Path::new("/big.pdf"), false, then);
        let pid = slow.under_way.first().unwrap().pid;
        assert_eq!(slow.on(then), None);
        assert_eq!(slow.on(then + SAID_WAIT).as_deref(), Some("big.pdf: handed to fake, which hasn't said how it went"));
        // Said once, and no longer looked at often.
        assert_eq!((slow.on(then + SAID_WAIT * 2), slow.wait()), (None, None));
        assert!(exists(pid), "still running");
        // Its success, once it ends, isn't said: the reader was told already.
        let deadline = Instant::now() + Duration::from_secs(20);
        while !slow.under_way.is_empty() {
            assert_eq!(slow.on(then + SAID_WAIT * 3), None);
            assert!(Instant::now() < deadline, "never ended");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!exists(pid), "reaped, not left a zombie");
    }

    #[test]
    fn at_most_four_launches_are_under_way_and_the_fifth_waits() {
        let now = Instant::now();
        let mut stuck = launches(&STUCK, true);
        for n in 1..=4 {
            assert_eq!(stuck.start(&format!("{n}.pdf"), Path::new("/n.pdf"), false, now), format!("opening {n}.pdf…"));
        }
        assert_eq!(stuck.start("5.pdf", Path::new("/5.pdf"), false, now), "1.pdf is still being opened: o waits for it");
        assert_eq!(stuck.under_way.len(), 4);
        // Ended, which reaps them.
        for launch in &stuck.under_way {
            let _ = Command::new("kill").arg(launch.pid.to_string()).status();
        }
        let said = news(&mut stuck, now);
        assert!(said.ends_with("fake was killed (signal 15)"), "{said}");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !stuck.under_way.is_empty() {
            stuck.on(now);
            assert!(Instant::now() < deadline, "never ended");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(stuck.start("5.pdf", Path::new("/5.pdf"), false, now), "opening 5.pdf…");
        let _ = Command::new("kill").arg(stuck.under_way.first().unwrap().pid.to_string()).status();
    }
}
