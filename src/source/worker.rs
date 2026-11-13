//! A worker process of clv's own: clv started again as `clv --worker <name>`, for work on a file's
//! bytes that can't be bounded before it is done, so is bounded as it is done, where the kernel
//! and the allocator enforce it ([ADR-0026](../../docs/adr/0026-an-svgs-html-labels-in-a-worker-process-capped-on-time-and-heap.md)).
//!
//! The viewer's side ([`spawn`], [`Link`]) starts one, its stdin and stdout pipes of frames and its
//! stderr null, and asks it one request at a time under one deadline, its sending and its answer
//! both: at the deadline the worker is killed, which ends a write or a read blocked on it. It reads
//! a frame only when an answer is awaited, so it holds at most one, and refuses one that isn't the
//! answer. A worker that dies is never asked again. Another thread may kill it meanwhile
//! ([`Kill`]): whoever waits on its answer then has it gone.
//!
//! The worker's side ([`serve`]) sets its heap a limit, its kind's ([`Worker::heap`]), past which an
//! allocation fails and it aborts, and arms the kernel's limit on its CPU time before each request
//! (`RLIMIT_CPU`), past which it is killed. It answers requests in turn, on a thread with a stack
//! of [`STACK`], until its stdin ends.
//!
//! A frame is its kind (a byte), the request's number it is or answers (four, little-endian), its
//! payload's length (four) and its payload. The kinds are a [`Worker`]'s, but for [`HELLO`], which
//! starts every worker and says which clv it is, and [`FAILED`].

// Only the SVG backend's labels and the PDF backend's text start a worker, and each uses its part
// of this (the labels no shared payload, nor a kill from another thread); a build without them
// still serves `--worker`, and tests the viewer's side.
#![cfg_attr(
    not(all(feature = "svg-html", feature = "pdf")),
    allow(dead_code, reason = "what is used of this depends on which of svg-html and pdf start a worker")
)]

use std::alloc::System;
use std::io::{Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// The allocator of the whole process: the system's, counted. The viewer's has no limit; a
/// worker's is its kind's ([`Worker::heap`]).
#[global_allocator]
static HEAP: cap::Cap<System> = cap::Cap::new(System, usize::MAX);

/// The most workers alive at once, of every kind, for all the viewer shows together. Past it, one
/// isn't started.
const MAX_WORKERS: usize = 6;

/// The stack of the thread a worker answers on: the work it does may recurse once a level of what
/// it is given, which the viewer bounds to 1,024 levels.
const STACK: usize = 64 << 20;

/// What this protocol is, as [`HELLO`] says it, with clv's version.
const PROTOCOL: u16 = 1;

/// The first request to every worker, and its answer: the protocol and clv's version, which a
/// worker of another clv (the binary replaced while the viewer runs) answers with [`FAILED`].
pub(crate) const HELLO: u8 = 0;

/// A worker's answer that it couldn't do what it was asked: its payload one byte, why.
pub(crate) const FAILED: u8 = 255;

/// Why a worker answered [`FAILED`]: it is another clv's.
const VERSION: u8 = 1;

/// The longest [`HELLO`]: its protocol and a version.
const MAX_HELLO: usize = 256;

/// The CPU seconds a worker may take answering [`HELLO`], and how long the viewer waits for it,
/// which starting the worker comes into.
const HELLO_CPU: u64 = 1;
const HELLO_WALL: Duration = Duration::from_secs(10);

/// How many times its CPU budget the viewer waits for a worker's answer before killing it: CPU
/// time comes slowly on a busy machine, and a worker that isn't running uses none.
pub(crate) const WALL: u32 = 3;

/// A kind of work done in a worker process, which answers requests.
pub(crate) trait Worker: Sync {
    /// Its name, as `--worker` takes it.
    fn name(&self) -> &'static str;
    /// Whether it takes requests of `kind`: the CPU seconds one may take, and the longest it is.
    fn takes(&self, kind: u8) -> Option<(u64, usize)>;
    /// The most its heap holds, what it is given included: past it, an allocation fails and the
    /// worker aborts.
    fn heap(&self) -> usize;
    /// A worker's answers to a viewer, in turn, and what it keeps between them.
    fn session(&self) -> Box<dyn Session>;
}

/// What answers a worker's requests, in turn.
pub(crate) trait Session {
    /// The answer to a request of `kind`, a kind the worker takes: its kind and payload.
    fn answer(&mut self, kind: u8, payload: Vec<u8>) -> (u8, Vec<u8>);
}

/// Why a worker is gone, or never started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Died {
    /// [`MAX_WORKERS`] were alive.
    TooMany,
    /// It couldn't be started.
    Unstarted(String),
    /// It hadn't answered, or taken its request, by the deadline: killed.
    TimedOut,
    /// The kernel stopped it past its CPU time.
    Cpu,
    /// It aborted: past its heap, as nothing else it does aborts.
    Memory,
    /// It ended otherwise: an exit status, or a signal.
    Ended(String),
    /// It said what it isn't asked to say: killed.
    Malformed,
    /// It is another clv's.
    Version,
}

/// The workers alive.
#[cfg(not(test))]
static ALIVE: AtomicUsize = AtomicUsize::new(0);

/// A place among the workers `alive` counts, given up when dropped.
struct Alive(&'static AtomicUsize);

impl Alive {
    /// A place among `alive`, if fewer than `most` are taken.
    fn take(alive: &'static AtomicUsize, most: usize) -> Result<Self, Died> {
        alive.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| (n < most).then(|| n.saturating_add(1))).map_err(|_| Died::TooMany)?;
        Ok(Self(alive))
    }
}

impl Drop for Alive {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A frame read: its kind, the request's number it answers, and its payload.
type Frame = (u8, u32, Vec<u8>);

/// A request's payload: the viewer's own, or one it shares with whoever else holds it (a file's
/// bytes), written from where it is.
enum Payload {
    Owned(Vec<u8>),
    Shared(Arc<Vec<u8>>),
}

impl Payload {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Shared(bytes) => bytes,
        }
    }
}

/// A worker, from the viewer's side: what it is asked, one request at a time.
pub(crate) struct Link {
    shared: Arc<Shared>,
    /// Frames to write, each its header and its payload, to the writer's thread, and what writing
    /// each came to.
    to: SyncSender<([u8; 9], Payload)>,
    written: Receiver<std::io::Result<()>>,
    /// The longest frame to read next, to the reader's thread, and the frame it read.
    ask: SyncSender<usize>,
    read: Receiver<Result<Frame, Died>>,
    /// The last request's number.
    number: u32,
}

/// What the viewer and the thread reading a worker's answers both hold of it: its process, its
/// place among the workers alive, and why it is gone, once it is. The reader ends it when its
/// answers end, so a worker that dies between requests is reaped, and its place given up, before
/// anything asks it again.
struct Shared(Mutex<Inner>);

struct Inner {
    process: Process,
    /// Its place among the workers alive, given up once it is gone.
    alive: Option<Alive>,
    /// Why it is gone, once it is: never asked again.
    dead: Option<Died>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Why it is gone, if it is.
    fn dead(&self) -> Option<Died> {
        self.lock().dead.clone()
    }

    /// The worker killed and reaped, gone for `died` (the first reason given stands), which is
    /// returned; its place among the workers alive given up.
    fn end(&self, died: Died) -> Died {
        let mut inner = self.lock();
        inner.process.kill();
        inner.alive = None;
        inner.dead.get_or_insert(died).clone()
    }

    /// Why the worker is gone, as it ended by itself: its status, killed if it hasn't ended.
    fn ended(&self) -> Died {
        let status = self.status().or_else(|| {
            // Its pipes closed, it may be ending: a moment to end before it is killed.
            std::thread::sleep(Duration::from_millis(50));
            self.status()
        });
        self.end(status.map_or_else(|| Died::Ended("its pipes closed".into()), why))
    }

    /// How the worker's process ended, if it has.
    fn status(&self) -> Option<ExitStatus> {
        match &mut self.lock().process {
            Process::Child(child) => child.try_wait().ok().flatten(),
            #[cfg(test)]
            Process::Thread => None,
        }
    }
}

/// What a worker runs as.
enum Process {
    Child(Child),
    /// The worker's code on a thread of the test harness (which can't be run again as `clv
    /// --worker`), through pipes: nothing to kill, and no limits.
    #[cfg(test)]
    Thread,
}

impl Process {
    /// Killed and reaped, if it is a process.
    fn kill(&mut self) {
        match self {
            Self::Child(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
            #[cfg(test)]
            Self::Thread => {}
        }
    }
}

/// The worker `worker`, started and greeted: this clv run again as `--worker <name>`, with
/// what fonts need of the environment and nothing else.
#[cfg(not(test))]
pub(crate) fn spawn(worker: &'static dyn Worker) -> Result<Link, Died> {
    let exe = std::env::current_exe().map_err(|e| Died::Unstarted(e.to_string()))?;
    let mut command = Command::new(exe);
    command.args(["--worker", worker.name()]).env_clear();
    for (key, value) in std::env::vars_os() {
        let kept = key.to_str().is_some_and(|k| k == "HOME" || k.starts_with("XDG_") || k.starts_with("FONTCONFIG_"));
        if kept {
            command.env(key, value);
        }
    }
    greeted(started(command, Alive::take(&ALIVE, MAX_WORKERS)?)?)
}

/// The worker `worker`'s code on a thread, through pipes, as a process's would be.
#[cfg(test)]
pub(crate) fn spawn(worker: &'static dyn Worker) -> Result<Link, Died> {
    // Counted apart from the workers of `worker.rs`'s tests, which run beside them.
    static THREADS: AtomicUsize = AtomicUsize::new(0);
    let alive = Alive::take(&THREADS, MAX_WORKERS)?;
    let unstarted = |e: std::io::Error| Died::Unstarted(e.to_string());
    let (mut from_viewer, to_worker) = std::io::pipe().map_err(unstarted)?;
    let (from_worker, mut to_viewer) = std::io::pipe().map_err(unstarted)?;
    std::thread::Builder::new()
        .name("worker".into())
        .stack_size(STACK)
        .spawn(move || serve_on(worker, &mut from_viewer, &mut to_viewer, false))
        .map_err(unstarted)?;
    greeted(linked(Process::Thread, Box::new(to_worker), Box::new(from_worker), alive)?)
}

/// `command` started as a worker in its place `alive`, its stdin and stdout pipes and its stderr
/// null: not yet greeted.
fn started(mut command: Command, alive: Alive) -> Result<Link, Died> {
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| Died::Unstarted(e.to_string()))?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Died::Unstarted("no pipes to it".into()));
    };
    linked(Process::Child(child), Box::new(stdin), Box::new(stdout), alive)
}

/// A link to a worker writing to `to` and reading from `from`, through a thread each. The reader
/// takes each frame's first byte as it comes, so a worker whose answers end between requests is
/// seen ending, and the rest of the frame only when an answer is awaited, within its length: so
/// the viewer holds at most one answer, and what a worker writes unasked waits in the pipe.
fn linked(process: Process, mut to: Box<dyn Write + Send>, mut from: Box<dyn Read + Send>, alive: Alive) -> Result<Link, Died> {
    let (frames, take) = sync_channel::<([u8; 9], Payload)>(1);
    let (wrote, written) = sync_channel(1);
    let (ask, asked) = sync_channel::<usize>(1);
    let (give, read) = sync_channel(1);
    let shared = Arc::new(Shared(Mutex::new(Inner { process, alive: Some(alive), dead: None })));
    let mut link = Link { shared: shared.clone(), to: frames, written, ask, read, number: 0 };
    let writer = std::thread::Builder::new().name("to worker".into()).spawn(move || {
        while let Ok((header, payload)) = take.recv() {
            let written = to.write_all(&header).and_then(|()| to.write_all(payload.bytes())).and_then(|()| to.flush());
            // Let go of before the answer is waited for: a file's bytes may be the last thing shared.
            drop(payload);
            if wrote.send(written).is_err() {
                return;
            }
        }
    });
    let reader = std::thread::Builder::new().name("from worker".into()).spawn(move || {
        loop {
            let Some(kind) = first_byte(&mut from) else {
                let _ = give.send(Err(shared.ended()));
                return;
            };
            let Ok(longest) = asked.recv() else { return };
            let frame = rest_of_frame(kind, &mut from, longest).map_err(|_| Died::Malformed);
            let ended = frame.is_err();
            if give.send(frame).is_err() || ended {
                return;
            }
        }
    });
    if let Err(e) = writer.and(reader) {
        link.end(Died::Unstarted(e.to_string()));
        return Err(Died::Unstarted(e.to_string()));
    }
    Ok(link)
}

/// `link` once its worker said it is this clv's.
fn greeted(mut link: Link) -> Result<Link, Died> {
    let (kind, payload) = link.request(HELLO, hello(), HELLO_WALL, MAX_HELLO)?;
    match (kind, payload) {
        (HELLO, said) if said == hello() => Ok(link),
        (FAILED, why) if why == [VERSION] => Err(link.end(Died::Version)),
        _ => Err(link.end(Died::Malformed)),
    }
}

/// What [`HELLO`] says: the protocol, and clv's version.
fn hello() -> Vec<u8> {
    let mut out = PROTOCOL.to_le_bytes().to_vec();
    out.extend_from_slice(env!("CARGO_PKG_VERSION").as_bytes());
    out
}

impl Link {
    /// The worker's answer to a request of `kind`, its `payload`, by `wall` from now, at most
    /// `longest` bytes: its kind and payload. The worker is killed, and never asked again, past
    /// the deadline, on an answer that isn't one (longer, or another request's), or when it is gone.
    pub(crate) fn request(&mut self, kind: u8, payload: Vec<u8>, wall: Duration, longest: usize) -> Result<(u8, Vec<u8>), Died> {
        self.ask(kind, Payload::Owned(payload), wall, longest)
    }

    /// [`Link::request`], with a payload the viewer shares (a file's bytes), written to the worker
    /// from where it is, never copied here.
    pub(crate) fn request_shared(&mut self, kind: u8, payload: Arc<Vec<u8>>, wall: Duration, longest: usize) -> Result<(u8, Vec<u8>), Died> {
        self.ask(kind, Payload::Shared(payload), wall, longest)
    }

    fn ask(&mut self, kind: u8, payload: Payload, wall: Duration, longest: usize) -> Result<(u8, Vec<u8>), Died> {
        if let Some(died) = self.shared.dead() {
            return Err(died);
        }
        let deadline = Instant::now().checked_add(wall);
        self.number = self.number.wrapping_add(1);
        let Ok(length) = u32::try_from(payload.bytes().len()) else { return Err(self.end(Died::Malformed)) };
        let ([n0, n1, n2, n3], [l0, l1, l2, l3]) = (self.number.to_le_bytes(), length.to_le_bytes());
        let header = [kind, n0, n1, n2, n3, l0, l1, l2, l3];
        if self.to.send((header, payload)).is_err() || self.ask.send(longest).is_err() {
            return Err(self.ended());
        }
        let left = || deadline.map_or(Duration::MAX, |d| d.saturating_duration_since(Instant::now()));
        match self.written.recv_timeout(left()) {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => return Err(self.ended()),
            Err(RecvTimeoutError::Timeout) => return Err(self.end(Died::TimedOut)),
        }
        match self.read.recv_timeout(left()) {
            Ok(Ok((kind, number, payload))) if number == self.number => Ok((kind, payload)),
            Ok(Ok(_)) => Err(self.end(Died::Malformed)),
            Ok(Err(Died::Malformed)) => Err(self.end(Died::Malformed)),
            Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => Err(self.ended()),
            Err(RecvTimeoutError::Timeout) => Err(self.end(Died::TimedOut)),
        }
    }

    /// Why the worker is gone, as it ended by itself: its status, killed if it hasn't ended.
    fn ended(&mut self) -> Died {
        self.shared.ended()
    }

    /// The worker killed and reaped, gone for `died`, which is returned; its place among the
    /// workers alive given up.
    fn end(&mut self, died: Died) -> Died {
        self.shared.end(died)
    }

    /// The worker killed for an answer that isn't one, which its caller found: why it is gone.
    pub(crate) fn refuse(&mut self) -> Died {
        self.end(Died::Malformed)
    }

    /// A way to kill the worker from another thread, while this one waits on its answer.
    pub(crate) fn killer(&self) -> Kill {
        Kill(self.shared.clone())
    }

    /// Whether the worker's process has ended and been reaped.
    #[cfg(test)]
    fn reaped(&self) -> bool {
        match &mut self.shared.lock().process {
            Process::Child(child) => matches!(child.try_wait(), Ok(Some(_))),
            Process::Thread => true,
        }
    }
}

impl Drop for Link {
    /// The worker killed and reaped: nothing outlives the viewer's use of it.
    fn drop(&mut self) {
        self.shared.end(Died::Ended("let go of".into()));
    }
}

/// A worker to kill from a thread that doesn't hold its [`Link`] ([`Link::killer`]): the reader left
/// what it was working for, and nothing waits for it to answer first.
pub(crate) struct Kill(Arc<Shared>);

impl Kill {
    /// The worker killed and reaped, now: a request waiting on its answer finds it gone, let go of.
    pub(crate) fn kill(&self) {
        self.0.end(Died::Ended("let go of".into()));
    }
}

/// The first byte of the next frame of `from`, waited for; `None` once `from` ends, or fails.
fn first_byte(from: &mut dyn Read) -> Option<u8> {
    let mut first = [0u8; 1];
    loop {
        match from.read(&mut first) {
            Ok(1) => return Some(first[0]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            _ => return None,
        }
    }
}

/// Why a worker that ended with `status` is gone.
fn why(status: ExitStatus) -> Died {
    use rustix::process::Signal;
    use std::os::unix::process::ExitStatusExt as _;
    match status.signal() {
        Some(signal) if signal == Signal::XCPU.as_raw() => Died::Cpu,
        Some(signal) if signal == Signal::ABORT.as_raw() => Died::Memory,
        Some(signal) => Died::Ended(format!("signal {signal}")),
        None => Died::Ended(status.code().map_or_else(|| "no status".into(), |c| format!("exit {c}"))),
    }
}

/// The next frame of `from`, of at most `longest(kind)` bytes: its kind, its request's number and
/// its payload; `None` where `from` ends before one starts. A kind `longest` refuses, or a frame
/// longer, is an error.
fn read_frame(from: &mut dyn Read, longest: impl Fn(u8) -> Option<usize>) -> std::io::Result<Option<Frame>> {
    let Some(kind) = first_byte(from) else { return Ok(None) };
    let Some(longest) = longest(kind) else { return Err(std::io::Error::from(std::io::ErrorKind::InvalidData)) };
    rest_of_frame(kind, from, longest).map(Some)
}

/// The rest of a frame of `kind` from `from`, its first byte read: its request's number and its
/// payload, of at most `longest` bytes, else an error.
fn rest_of_frame(kind: u8, from: &mut dyn Read, longest: usize) -> std::io::Result<Frame> {
    use std::io::{Error, ErrorKind};
    let mut header = [0u8; 8];
    from.read_exact(&mut header)?;
    let [n0, n1, n2, n3, l0, l1, l2, l3] = header;
    let (number, length) = (u32::from_le_bytes([n0, n1, n2, n3]), u32::from_le_bytes([l0, l1, l2, l3]));
    let length = usize::try_from(length).map_err(|_| Error::from(ErrorKind::InvalidData))?;
    if length > longest {
        return Err(Error::from(ErrorKind::InvalidData));
    }
    let mut payload = Vec::new();
    payload.try_reserve_exact(length).map_err(|_| Error::from(ErrorKind::OutOfMemory))?;
    from.take(u64::try_from(length).unwrap_or(u64::MAX)).read_to_end(&mut payload)?;
    if payload.len() != length {
        return Err(Error::from(ErrorKind::UnexpectedEof));
    }
    Ok((kind, number, payload))
}

/// A frame of `kind`, answering request `number`, written to `to`.
fn write_frame(to: &mut dyn Write, kind: u8, number: u32, payload: &[u8]) -> std::io::Result<()> {
    let length = u32::try_from(payload.len()).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    to.write_all(&[kind])?;
    to.write_all(&number.to_le_bytes())?;
    to.write_all(&length.to_le_bytes())?;
    to.write_all(payload)?;
    to.flush()
}

/// This process as the worker `name` of `workers`, answering on stdin and stdout until stdin
/// ends, its heap and CPU time limited.
pub(crate) fn serve(name: &str, workers: &[&'static dyn Worker]) -> Result<(), String> {
    let Some(worker) = workers.iter().copied().find(|w| w.name() == name) else {
        // Not the name: it is the command line's, and goes to stderr as it is.
        return Err("no such worker in this build".into());
    };
    HEAP.set_limit(worker.heap()).map_err(|()| "its heap is already past its limit".to_string())?;
    let thread = std::thread::Builder::new()
        .name("worker".into())
        .stack_size(STACK)
        .spawn(move || serve_on(worker, &mut std::io::stdin().lock(), &mut std::io::stdout().lock(), true))
        .map_err(|e| format!("no thread to work on: {e}"))?;
    thread.join().map_err(|_| "it failed".to_string())?
}

/// `worker`'s answers to the requests read from `from`, written to `to`, until `from` ends; with
/// `limited`, its CPU time limited before each request.
fn serve_on(worker: &dyn Worker, from: &mut dyn Read, to: &mut dyn Write, limited: bool) -> Result<(), String> {
    let mut session = worker.session();
    let longest = |kind: u8| if kind == HELLO { Some(MAX_HELLO) } else { worker.takes(kind).map(|(_, longest)| longest) };
    loop {
        let Some((kind, number, payload)) = read_frame(from, longest).map_err(|e| format!("not a request: {e}"))? else { return Ok(()) };
        if limited {
            let budget = if kind == HELLO { HELLO_CPU } else { worker.takes(kind).map_or(HELLO_CPU, |(cpu, _)| cpu) };
            limit_cpu(budget)?;
        }
        let (kind, payload) = match kind {
            HELLO if payload == hello() => (HELLO, payload),
            HELLO => (FAILED, vec![VERSION]),
            _ => session.answer(kind, payload),
        };
        write_frame(to, kind, number, &payload).map_err(|e| format!("answering: {e}"))?;
    }
}

/// The kernel's limit on this process's CPU time set to `budget` seconds more than it has used,
/// within its hard limit: past it, the kernel stops it (`SIGXCPU`). A second at a time, so it is
/// stopped within a second past the budget.
fn limit_cpu(budget: u64) -> Result<(), String> {
    use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
    use rustix::time::{ClockId, clock_gettime};
    let used = clock_gettime(ClockId::ProcessCPUTime);
    let used = u64::try_from(used.tv_sec).unwrap_or(0).saturating_add(u64::from(used.tv_nsec > 0));
    let hard = getrlimit(Resource::Cpu).maximum;
    let soft = used.saturating_add(budget);
    let soft = hard.map_or(soft, |hard| soft.min(hard));
    setrlimit(Resource::Cpu, Rlimit { current: Some(soft), maximum: hard }).map_err(|e| format!("its CPU time can't be limited: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fake workers alive, counted apart from the others.
    static FAKES: AtomicUsize = AtomicUsize::new(0);

    /// A fake worker: `script`, run by `sh`.
    fn fake(script: &str) -> Result<Link, Died> {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        started(command, Alive::take(&FAKES, usize::MAX)?)
    }

    /// A request to a fake, of no kind it knows: its answer, or why there is none.
    fn ask(link: &mut Link, wall: u64, longest: usize) -> Result<(u8, Vec<u8>), Died> {
        link.request(7, b"x".to_vec(), Duration::from_secs(wall), longest)
    }

    /// A worker that doesn't answer is killed at the deadline, and reaped.
    #[test]
    fn a_worker_that_doesnt_answer_is_killed_at_its_deadline() {
        let mut link = fake("exec sleep 60").unwrap();
        let begun = Instant::now();
        assert_eq!(ask(&mut link, 1, 64), Err(Died::TimedOut));
        assert!(begun.elapsed() < Duration::from_secs(5), "{:?}", begun.elapsed());
        assert!(link.reaped(), "reaped");
        assert_eq!(ask(&mut link, 1, 64), Err(Died::TimedOut), "never asked again");
    }

    /// A worker that stops reading is killed at the deadline, though the request is larger than
    /// the pipe holds and its writing is blocked, not the answer awaited.
    #[test]
    fn a_worker_that_stops_reading_is_killed_at_its_deadline() {
        let mut link = fake("exec sleep 60").unwrap();
        let begun = Instant::now();
        assert_eq!(link.request(7, vec![0; 16 << 20], Duration::from_secs(1), 64), Err(Died::TimedOut));
        assert!(link.reaped(), "reaped");
        assert!(begun.elapsed() < Duration::from_secs(5), "{:?}", begun.elapsed());
    }

    /// A worker that ends between requests, after answering, is seen ending: reaped, and its place
    /// among the workers alive given up, before anything asks it again; asked again, it is gone.
    #[test]
    fn a_worker_that_ends_between_requests_is_reaped_and_gives_up_its_place() {
        static COUNTED: AtomicUsize = AtomicUsize::new(0);
        let mut command = Command::new("sh");
        command.args(["-c", r"head -c 10 >/dev/null; printf '\007\001\000\000\000\001\000\000\000x'; exit 0"]);
        let mut link = started(command, Alive::take(&COUNTED, MAX_WORKERS).unwrap()).unwrap();
        assert_eq!(COUNTED.load(Ordering::SeqCst), 1);
        assert_eq!(ask(&mut link, 10, 64), Ok((7, b"x".to_vec())));
        let begun = Instant::now();
        while COUNTED.load(Ordering::SeqCst) == 1 || !link.reaped() {
            assert!(begun.elapsed() < Duration::from_secs(5), "still counted among the workers alive, or a zombie");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(ask(&mut link, 10, 64), Err(Died::Ended("exit 0".into())));
    }

    /// A worker killed from another thread while a request waits on its answer: the request ends at
    /// once, the worker reaped and gone for having been let go of, well before its deadline.
    #[test]
    fn a_worker_is_killed_from_another_thread_while_its_answer_is_awaited() {
        let mut link = fake("exec sleep 60").unwrap();
        let kill = link.killer();
        let begun = Instant::now();
        let killer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            kill.kill();
        });
        assert_eq!(ask(&mut link, 30, 64), Err(Died::Ended("let go of".into())));
        assert!(begun.elapsed() < Duration::from_secs(5), "{:?}", begun.elapsed());
        killer.join().unwrap();
        assert!(link.reaped(), "reaped");
        assert_eq!(ask(&mut link, 30, 64), Err(Died::Ended("let go of".into())), "never asked again");
    }

    /// A payload shared is written as one owned is: its header, then its bytes from where they are.
    #[test]
    fn a_shared_payload_is_written_as_an_owned_one_is() {
        // `cat` writes each frame back as it came: a request's kind, number and payload are its answer's.
        let mut link = fake("exec cat").unwrap();
        let shared = Arc::new(b"wxyz!".to_vec());
        assert_eq!(link.request_shared(7, shared.clone(), Duration::from_secs(10), 64), Ok((7, b"wxyz!".to_vec())));
        assert_eq!(link.request(8, b"owned".to_vec(), Duration::from_secs(10), 64), Ok((8, b"owned".to_vec())));
        assert_eq!(link.request_shared(9, Arc::new(Vec::new()), Duration::from_secs(10), 64), Ok((9, Vec::new())));
        let begun = Instant::now();
        while Arc::strong_count(&shared) > 1 {
            assert!(begun.elapsed() < Duration::from_secs(5), "the link still holds the payload");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// How a worker ended is told: an exit, a crash, its CPU time, its heap.
    #[test]
    fn how_a_worker_ended_is_told() {
        assert_eq!(ask(&mut fake("exit 3").unwrap(), 10, 64), Err(Died::Ended("exit 3".into())));
        assert_eq!(ask(&mut fake("kill -SEGV $$").unwrap(), 10, 64), Err(Died::Ended(format!("signal {}", rustix::process::Signal::SEGV.as_raw()))));
        assert_eq!(ask(&mut fake("kill -XCPU $$").unwrap(), 10, 64), Err(Died::Cpu));
        assert_eq!(ask(&mut fake("kill -ABRT $$").unwrap(), 10, 64), Err(Died::Memory));
    }

    /// An answer longer than it may be, of a kind refused, or another request's, kills the worker;
    /// and one sent unasked is taken for none.
    #[test]
    fn an_answer_that_isnt_one_kills_the_worker() {
        // `yes`: a frame of kind `y`, request 0x0a790a79, 0x0a790a79 bytes long.
        let mut link = fake("exec yes").unwrap();
        assert_eq!(ask(&mut link, 10, 1 << 20), Err(Died::Malformed));
        assert!(link.reaped(), "killed and reaped");
        // An answer to request 2, to request 1.
        let mut link = fake(r"head -c 10 >/dev/null; printf '\007\002\000\000\000\001\000\000\000x'; sleep 60").unwrap();
        assert_eq!(ask(&mut link, 10, 64), Err(Died::Malformed));
        // Answers to request 1 without end: the first is its answer, the next taken for none.
        let flood = r"head -c 10 >/dev/null; while :; do printf '\007\001\000\000\000\001\000\000\000x'; done";
        let mut link = fake(flood).unwrap();
        assert_eq!(ask(&mut link, 10, 64), Ok((7, b"x".to_vec())));
        assert_eq!(ask(&mut link, 10, 64), Err(Died::Malformed));
    }

    /// Past the most workers alive, the next isn't started; one gone frees its place.
    #[test]
    fn past_the_most_workers_the_next_isnt_started() {
        static COUNTED: AtomicUsize = AtomicUsize::new(0);
        let start = || {
            let mut command = Command::new("sleep");
            command.arg("60");
            started(command, Alive::take(&COUNTED, MAX_WORKERS)?)
        };
        let mut alive: Vec<Link> = (0..MAX_WORKERS).map(|_| start().unwrap()).collect();
        assert_eq!(start().err(), Some(Died::TooMany), "the seventh");
        alive.pop();
        assert!(start().is_ok(), "one gone frees its place");
    }

    /// The worker's own side, through pipes: a request answered, the frames round-tripped; one of
    /// a kind it doesn't take, or longer than it takes, ends it.
    #[test]
    fn a_worker_answers_its_requests_in_turn() {
        struct Echo;
        impl Worker for Echo {
            fn name(&self) -> &'static str {
                "echo"
            }
            fn takes(&self, kind: u8) -> Option<(u64, usize)> {
                (kind == 1).then_some((1, 4))
            }
            fn heap(&self) -> usize {
                usize::MAX
            }
            fn session(&self) -> Box<dyn Session> {
                struct Count(u8);
                impl Session for Count {
                    fn answer(&mut self, kind: u8, mut payload: Vec<u8>) -> (u8, Vec<u8>) {
                        self.0 = self.0.wrapping_add(1);
                        payload.push(self.0);
                        (kind, payload)
                    }
                }
                Box::new(Count(0))
            }
        }
        static ECHO: Echo = Echo;
        let mut link = spawn(&ECHO).unwrap();
        assert_eq!(link.request(1, b"ab".to_vec(), Duration::from_secs(10), 64), Ok((1, b"ab\x01".to_vec())));
        assert_eq!(link.request(1, b"cd".to_vec(), Duration::from_secs(10), 64), Ok((1, b"cd\x02".to_vec())));
        assert!(link.request(1, b"abcde".to_vec(), Duration::from_secs(10), 64).is_err(), "longer than it takes");
        let mut link = spawn(&ECHO).unwrap();
        assert!(link.request(2, Vec::new(), Duration::from_secs(10), 64).is_err(), "a kind it doesn't take");
    }

    /// A worker whose limit on its CPU time is set: it spins, allocating nothing, and the kernel
    /// stops it. Run as a process of its own by [`a_worker_is_stopped_past_its_cpu_time`].
    #[test]
    fn spinning_worker() {
        if std::env::var_os("CLV_SPINNING_WORKER").is_none() {
            return;
        }
        struct Spin;
        impl Worker for Spin {
            fn name(&self) -> &'static str {
                "spin"
            }
            fn takes(&self, _kind: u8) -> Option<(u64, usize)> {
                Some((1, 64))
            }
            fn heap(&self) -> usize {
                usize::MAX
            }
            fn session(&self) -> Box<dyn Session> {
                struct Spinning;
                impl Session for Spinning {
                    fn answer(&mut self, _kind: u8, _payload: Vec<u8>) -> (u8, Vec<u8>) {
                        let mut n = 0u64;
                        loop {
                            n = std::hint::black_box(n.wrapping_add(1));
                        }
                    }
                }
                Box::new(Spinning)
            }
        }
        let _ = serve_on(&Spin, &mut std::io::stdin().lock(), &mut std::io::sink(), true);
    }

    /// A worker is stopped by the kernel past its CPU time, 1 s here, not by a deadline: this
    /// test's binary run again as [`spinning_worker`] (its stdout the harness's, so not read).
    #[test]
    fn a_worker_is_stopped_past_its_cpu_time() {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "source::worker::tests::spinning_worker", "--nocapture", "--test-threads", "1"])
            .env("CLV_SPINNING_WORKER", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        write_frame(&mut stdin, 7, 1, b"x").unwrap();
        let begun = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if begun.elapsed() > Duration::from_secs(20) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("still spinning after 20 s");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(why(status), Died::Cpu);
        assert!(begun.elapsed() < Duration::from_secs(10), "{:?}", begun.elapsed());
    }
}
