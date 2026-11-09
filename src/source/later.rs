//! A file of a list, not opened until its tile is wanted: what its backend spends opening it (a
//! document's whole layout, a PDF's whole file, a workbook printed) follows the screen, not the list
//! ([ADR-0019](../../docs/adr/0019-a-list-opens-its-files-when-their-tiles-are-wanted.md)).
//!
//! Its size isn't known until it is open, and what a render is asked for depends on it. So a request
//! made before then is answered by a frame that asks for it again ([`Frame::asked_again`]), and the viewer
//! lays the tile out afresh with the size now known. A size becomes known only when such a frame is
//! taken, or the viewer asks for the file ([`Later::outcome`]), neither of them during a layout: never
//! between the viewer asking for a size and listing the renders it worked out from it, so a render is
//! only ever done at a scale from the true size.
//!
//! Files are opened on [`Openers`]' threads, so that keys keep working. What they open stays open
//! here, as what a source holds while idle is little: what opening one found isn't spent twice when
//! the reader comes back, or goes into it. The same threads run a job that lays a document out again
//! ([`Openers::run`]), first, since the reader is waiting on it.

use std::cell::{OnceCell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use super::{Again, Backend, Frame, Input, Job, Opened, Pool, Render, Request, Rest, Room, Source};
use crate::args::OwnedOptions;

/// The most files opened at once. A document's first layout takes a core, and a large PDF is read
/// whole into memory, so not as many as there are cores; and a reader moving on from a file being
/// opened leaves it opening, so more than the pool's two tiles at a time.
const THREADS: usize = 4;

/// The stack of a thread that opens files, as large as the main thread's by default, where they
/// were opened before: what a file's nesting takes reading it mustn't be what overflows.
const STACK: usize = 8 << 20;

/// What an unopened file is taken to measure: a US Letter page, in points. Nothing is drawn at it,
/// since a request made at it only opens the file.
const UNKNOWN: (f32, f32) = (612.0, 792.0);

/// A file to open again: closed meanwhile, so hundreds of them hold no file descriptors.
#[derive(Clone)]
pub(crate) enum Reopen {
    /// On disk, by its path.
    Disk(String),
    /// Held in another file: its path, first bytes, and how to read it from there.
    Held(String, Vec<u8>, Again),
}

impl Reopen {
    /// The file again, to be opened.
    fn input(&self) -> Result<Input, String> {
        match self {
            Self::Disk(path) => Input::open(path),
            Self::Held(path, head, again) => Ok(Input::held(path.clone(), head.clone(), again.clone())),
        }
    }
}

impl Input {
    /// What `backend` says a list may know of this file unopened ([`Backend::deferred`]).
    pub(crate) fn deferred_by(&self, backend: &dyn Backend) -> Option<super::Deferred> {
        backend.deferred(&self.path, &self.head)
    }

    /// How to come back to this file for a list that may never need it, closing it now; or the file
    /// itself, if it can't be come back to: a pipe, or anything named in `/dev`. Redirected, `/dev/stdin`
    /// is a regular file, but on macOS opening its path again goes on from where this has read to.
    pub(crate) fn later(self) -> Result<Reopen, Self> {
        let Self { path, head, rest } = self;
        match rest {
            Rest::File(file) if file.metadata().is_ok_and(|m| m.is_file()) && has_a_path_of_its_own(&path) => Ok(Reopen::Disk(path)),
            Rest::File(file) => Err(Self { path, head, rest: Rest::File(file) }),
            Rest::Held(again) => Ok(Reopen::Held(path, head, again)),
        }
    }
}

/// Whether opening `path` again opens the file anew, at its start: where it leads isn't a device's
/// name for something already open.
fn has_a_path_of_its_own(path: &str) -> bool {
    std::fs::canonicalize(path).is_ok_and(|real| !real.starts_with("/dev"))
}

/// The threads files are opened on, for all a run's lists, started when first needed.
#[derive(Default)]
pub(crate) struct Openers(Arc<Shared>);

#[derive(Default)]
struct Shared {
    state: Mutex<Queue>,
    /// A file to open, or time to stop.
    wake: Condvar,
}

#[derive(Default)]
struct Queue {
    /// The most urgent first.
    waiting: VecDeque<Work>,
    started: bool,
    stop: bool,
}

/// What a thread takes from the queue: a file to open, or a job to run.
enum Work {
    Open(Arc<Slot>),
    Run(Arc<Running>),
}

impl Work {
    /// Whether this is `slot`'s opening.
    fn opens(&self, slot: &Arc<Slot>) -> bool {
        matches!(self, Self::Open(waiting) if Arc::ptr_eq(waiting, slot))
    }
}

/// A job given to the threads, and its result once it has run: shared by the thread and the
/// [`Handle`] that polls it.
struct Running {
    /// Taken by the thread that runs it, or by the handle dropped before one did.
    job: Mutex<Option<Job>>,
    /// Its result, until the handle takes it.
    done: Mutex<Option<Result<Box<dyn Source + Send>, String>>>,
}

/// A job on the threads ([`Openers::run`]), polled for its result, never waited for. Dropped before
/// the job has begun, the job never runs; begun, it runs to its end and its result is dropped.
pub(crate) struct Handle(Arc<Running>);

impl Handle {
    /// The job's result, once, when it has run. Never blocks.
    pub(crate) fn try_take(&self) -> Option<Result<Box<dyn Source + Send>, String>> {
        lock(&self.0.done).take()
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // Not begun: the thread finds nothing to run.
        drop(lock(&self.0.job).take());
    }
}

impl Openers {
    /// Open `slot`, the next file if `first`, unless it is already on its way.
    fn want(&self, slot: &Arc<Slot>, first: bool) {
        let mut queue = lock(&self.0.state);
        let mut stage = lock(&slot.stage);
        match *stage {
            Stage::Idle => *stage = Stage::Queued,
            // Not begun: it moves up.
            Stage::Queued if first => queue.waiting.retain(|waiting| !waiting.opens(slot)),
            Stage::Queued | Stage::Opening | Stage::Done(_) => return,
        }
        drop(stage);
        self.queue(queue, Work::Open(slot.clone()), first);
    }

    /// Run `job` on the threads, before any file waiting: the reader is waiting on it. Its result
    /// comes through the handle.
    pub(crate) fn run(&self, job: Job) -> Handle {
        let running = Arc::new(Running { job: Mutex::new(Some(job)), done: Mutex::new(None) });
        self.queue(lock(&self.0.state), Work::Run(running.clone()), true);
        Handle(running)
    }

    /// `item` into the queue, first or last, the threads started if they aren't yet.
    fn queue(&self, mut queue: MutexGuard<'_, Queue>, item: Work, first: bool) {
        match first {
            true => queue.waiting.push_front(item),
            false => queue.waiting.push_back(item),
        }
        if !std::mem::replace(&mut queue.started, true) {
            for _ in 0..THREADS {
                let shared = self.0.clone();
                // A thread that can't start leaves the others; with none, tiles wait for good,
                // which the pager shows as tiles still rendering.
                let _ = std::thread::Builder::new().name("open".into()).stack_size(STACK).spawn(move || work(&shared));
            }
        }
        drop(queue);
        self.0.wake.notify_one();
    }

    /// `slot` no longer wanted: unless it has begun, it isn't opened.
    fn forget(&self, slot: &Arc<Slot>) {
        let mut queue = lock(&self.0.state);
        let mut stage = lock(&slot.stage);
        if matches!(*stage, Stage::Queued) {
            *stage = Stage::Idle;
            queue.waiting.retain(|waiting| !waiting.opens(slot));
        }
    }
}

impl Drop for Openers {
    /// The threads stop once the file they are opening is done; nothing waits for them.
    fn drop(&mut self) {
        lock(&self.0.state).stop = true;
        self.0.wake.notify_all();
    }
}

fn work(shared: &Shared) {
    loop {
        let work = {
            let mut queue = lock(&shared.state);
            loop {
                if queue.stop {
                    return;
                }
                if let Some(work) = queue.waiting.pop_front() {
                    if let Work::Open(slot) = &work {
                        *lock(&slot.stage) = Stage::Opening;
                    }
                    break work;
                }
                queue = shared.wake.wait(queue).unwrap_or_else(PoisonError::into_inner);
            }
        };
        match work {
            Work::Open(slot) => {
                let opened = slot.open();
                *lock(&slot.stage) = Stage::Done(Some(opened));
            }
            Work::Run(running) => {
                // Its handle dropped before now took the job: nothing to run. A panic in it costs
                // the job, not the viewer, as an opening's does.
                let Some(job) = lock(&running.job).take() else { continue };
                let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).unwrap_or_else(|_| Err("the layout failed".into()));
                *lock(&running.done) = Some(done);
            }
        }
    }
}

/// The lock, even after a thread panicked holding it: each is whole between statements, and files
/// open unlocked.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A file as its thread opened it, or why it can't be shown.
type Outcome = Result<Box<dyn Source + Send>, String>;

/// How to open a file, and how far that has got: shared with the thread that opens it.
struct Slot {
    file: Reopen,
    backend: &'static dyn Backend,
    options: Arc<OwnedOptions>,
    /// The room it is laid out for: a list's preview page.
    room: Room,
    stage: Mutex<Stage>,
}

enum Stage {
    Idle,
    Queued,
    Opening,
    /// `None` once [`Later`] has taken it.
    Done(Option<Outcome>),
}

impl Slot {
    /// The file opened, here on this thread. A backend's bug costs the file, not the viewer.
    fn open(&self) -> Outcome {
        let open = || -> Outcome {
            match self.backend.open_at(self.file.input()?, &self.options.parsed(), self.room)? {
                Opened::Pages(source) if source.pages() == 0 => Err("no pages to show".into()),
                Opened::Pages(source) => Ok(source),
                // A backend that defers opening says it has pages to show: this is its bug.
                Opened::Files(_) => Err("it holds files, not pages".into()),
            }
        };
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(open)).unwrap_or_else(|_| Err("the file couldn't be opened".into()))
    }
}

/// A file of a list, opened when it is wanted ([module docs](self)).
pub(crate) struct Later(Rc<Inner>);

struct Inner {
    path: String,
    settings: &'static [&'static str],
    slot: Arc<Slot>,
    openers: Rc<Openers>,
    /// Its opening, once asked for after it finished: this never changes again.
    outcome: OnceCell<Result<Rc<dyn Source>, String>>,
}

impl Later {
    /// The file `file`, which `backend` opens with `options`, laid out for `room`, and which has
    /// `settings`, as [`Backend::deferred`] said; opened on `openers`.
    pub(crate) fn new(
        file: Reopen,
        path: String,
        settings: &'static [&'static str],
        backend: &'static dyn Backend,
        options: Arc<OwnedOptions>,
        openers: Rc<Openers>,
        room: Room,
    ) -> Self {
        let slot = Arc::new(Slot { file, backend, options, room, stage: Mutex::new(Stage::Idle) });
        Self(Rc::new(Inner { path, settings, slot, openers, outcome: OnceCell::new() }))
    }

    /// Its pages, or why it can't be shown, if it has been opened. Otherwise `None`, and it opens
    /// next: a reader waiting for it asks again and again, so however little of it is on screen, it
    /// stays wanted.
    pub(crate) fn outcome(&self) -> Option<Result<Rc<dyn Source>, String>> {
        let inner = &*self.0;
        let known = inner.known().cloned();
        if known.is_none() {
            inner.openers.want(&inner.slot, true);
        }
        known
    }
}

impl Inner {
    /// What opening it came to, if it has.
    fn known(&self) -> Option<&Result<Rc<dyn Source>, String>> {
        if self.outcome.get().is_none() {
            let done = match &mut *lock(&self.slot.stage) {
                Stage::Done(done) => done.take(),
                _ => None,
            };
            if let Some(done) = done {
                // Why a file can't be shown names it by its path, where the list names it itself.
                let done = done.map(Rc::<dyn Source + Send>::from).map(|source| -> Rc<dyn Source> { source }).map_err(|why| self.unnamed(why));
                let _ = self.outcome.set(done);
            }
        }
        self.outcome.get()
    }

    /// `why`, without the file's path at its start: [`super::Joined`] names the file.
    fn unnamed(&self, why: String) -> String {
        why.strip_prefix(&format!("{}: ", self.path)).map_or_else(|| why.clone(), str::to_owned)
    }

    /// Whether it has finished opening, whether or not that is known here yet.
    fn finished(&self) -> bool {
        self.outcome.get().is_some() || matches!(*lock(&self.slot.stage), Stage::Done(_))
    }

    /// Opened, here on this thread, if it isn't yet: for pages printed, which wait in any case.
    fn settled(&self) -> &Result<Rc<dyn Source>, String> {
        self.outcome.get_or_init(|| {
            // Taken from the queue, or finished: printing asks for nothing else, so it can't be
            // under way, but should it be, it is opened here too and the other result dropped.
            self.openers.forget(&self.slot);
            let done = match &mut *lock(&self.slot.stage) {
                Stage::Done(done) => done.take(),
                _ => None,
            };
            let opened = done.unwrap_or_else(|| self.slot.open());
            opened.map(Rc::<dyn Source + Send>::from).map(|source| -> Rc<dyn Source> { source }).map_err(|why| self.unnamed(why))
        })
    }
}

impl Source for Later {
    /// A list shows one, its first; going into the file gets them all ([`Later::outcome`]).
    fn pages(&self) -> usize {
        1
    }

    /// Its true size, once known; until then only a stand-in, at which nothing is drawn. This only
    /// looks: a thread finishing can't change what a layout under way reads (module docs).
    fn size(&self, page: usize) -> Option<(f32, f32)> {
        match self.0.outcome.get() {
            Some(Ok(source)) => source.size(page),
            _ => (page == 0).then_some(UNKNOWN),
        }
    }

    fn natural(&self, page: usize) -> bool {
        matches!(self.0.outcome.get(), Some(Ok(source)) if source.natural(page))
    }

    /// Its own, once known, as its size is.
    fn largest_scale(&self, page: usize) -> Option<f32> {
        match self.0.outcome.get() {
            Some(Ok(source)) => source.largest_scale(page),
            _ => None,
        }
    }

    fn settings(&self) -> Vec<&'static str> {
        self.0.settings.to_vec()
    }

    fn settle(&self, _page: usize) {
        self.0.settled();
    }

    fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
        self.0.settled().as_ref().map_err(Clone::clone)?.renderer()
    }

    fn pool(&self, threads: usize) -> Box<dyn Pool> {
        Box::new(LaterPool { later: self.0.clone(), threads, pool: OnceCell::new(), held: RefCell::default(), told: RefCell::default() })
    }
}

/// A later file's renders: its own pool's, once it is open. Until then the renders wanted are held,
/// and it opens.
struct LaterPool {
    later: Rc<Inner>,
    threads: usize,
    /// The opened source's own.
    pool: OnceCell<Box<dyn Pool>>,
    /// Asked for before the true size was known, or for a file that can't be shown: to answer.
    held: RefCell<Vec<Request>>,
    /// Answered, and maybe not yet received: not answered again if listed again.
    told: RefCell<Vec<u64>>,
}

impl Pool for LaterPool {
    fn set_queue(&self, requests: Vec<Request>) {
        let later = &*self.later;
        let told = {
            let mut told = self.told.borrow_mut();
            told.retain(|tag| requests.iter().any(|r| r.tag == *tag));
            told.clone()
        };
        // What was already answered stays answered: told to ask again, it was from before the size.
        let wanted = requests.iter().filter(|r| !told.contains(&r.tag)).copied();
        // Only looks, so that what is listed is from the size the viewer last saw: known, it has laid
        // this list out with the true one (module docs).
        if let Some(Ok(source)) = later.outcome.get() {
            self.pool.get_or_init(|| source.pool(self.threads)).set_queue(wanted.collect());
            self.held.borrow_mut().clear();
            return;
        }
        match requests.is_empty() {
            false => later.openers.want(&later.slot, false),
            true => later.openers.forget(&later.slot),
        }
        *self.held.borrow_mut() = wanted.collect();
    }

    fn try_recv(&self) -> Option<Frame> {
        let later = &*self.later;
        if !later.finished() {
            return None;
        }
        if let Some(request) = {
            let mut held = self.held.borrow_mut();
            (!held.is_empty()).then(|| held.remove(0))
        } {
            self.told.borrow_mut().push(request.tag);
            // Now it is known, with no layout under way. Failed, this is the answer, whatever size it
            // was asked at; otherwise the viewer is to ask again, at the true one.
            return Some(match later.known() {
                Some(Err(why)) => Frame::failed(request, why.clone()),
                _ => Frame::asked_again(request),
            });
        }
        self.pool.get()?.try_recv()
    }
}

impl Drop for LaterPool {
    /// No renders wanted any more, so no opening. One begun goes on: its result is kept.
    fn drop(&mut self) {
        self.later.openers.forget(&self.later.slot);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::args::{OptionSpec, Parsed};
    use crate::source::{Format, Pixels};

    /// How many files a test's backend has opened and been asked to, and what they answer. A static,
    /// as a backend is; each test has its own.
    struct Test {
        opened: AtomicUsize,
        /// How long opening takes.
        millis: u64,
        size: (f32, f32),
        broken: bool,
    }

    impl Backend for Test {
        fn name(&self) -> &'static str {
            "test"
        }
        fn shows(&self) -> &'static str {
            "tests"
        }
        fn formats(&self) -> Vec<Format> {
            Vec::new()
        }
        fn credits(&self) -> &[&'static str] {
            &[]
        }
        fn options(&self) -> &'static [OptionSpec] {
            &[]
        }
        fn opens(&self, _path: &str, _head: &[u8]) -> bool {
            true
        }
        fn open(&self, input: Input, _options: &Parsed<'_>) -> Result<Opened, String> {
            self.opened.fetch_add(1, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(self.millis));
            if self.broken {
                return Err(format!("{}: broken", input.path));
            }
            Ok(Opened::Pages(Box::new(Fake(self.size))))
        }
    }

    struct Fake((f32, f32));

    impl Source for Fake {
        fn pages(&self) -> usize {
            3
        }
        fn size(&self, page: usize) -> Option<(f32, f32)> {
            (page < 3).then_some(self.0)
        }
        fn natural(&self, _page: usize) -> bool {
            true
        }
        fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
            Ok(Box::new(FakeRender))
        }
        fn pool(&self, _threads: usize) -> Box<dyn Pool> {
            Box::new(FakePool(Mutex::new(Vec::new())))
        }
    }

    fn frame(request: Request) -> Frame {
        Frame { request, width: 1, height: 1, rgba: Pixels::new(vec![9u8; 4]), error: None, warnings: Vec::new(), again: false }
    }

    struct FakeRender;

    impl Render for FakeRender {
        fn render(&mut self, request: Request) -> Frame {
            frame(request)
        }
    }

    struct FakePool(Mutex<Vec<Request>>);

    impl Pool for FakePool {
        fn set_queue(&self, requests: Vec<Request>) {
            *self.0.lock().unwrap() = requests;
        }
        fn try_recv(&self) -> Option<Frame> {
            let mut queue = self.0.lock().unwrap();
            (!queue.is_empty()).then(|| frame(queue.remove(0)))
        }
    }

    /// The room the tests' files are laid out for.
    const ROOM: Room = Room { cols: 60, rows: 40, cell_w: 10.0, cell_h: 20.0, gap: 2 };

    fn later(backend: &'static Test) -> Later {
        let file = std::env::temp_dir().join(format!("clview-later-{}", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        let path = file.to_str().unwrap().to_owned();
        Later::new(Reopen::Disk(path.clone()), path, &[], backend, Arc::new(Parsed::default().owned()), Rc::new(Openers::default()), ROOM)
    }

    fn request(tag: u64) -> Request {
        Request { page: 0, scale: 1.0, tag, ..Default::default() }
    }

    /// The next frame from `pool`, within five seconds.
    fn next(pool: &dyn Pool) -> Frame {
        let start = Instant::now();
        loop {
            if let Some(frame) = pool.try_recv() {
                return frame;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "no frame");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn nothing_opens_until_it_is_wanted_and_then_once() {
        static BACKEND: Test = Test { opened: AtomicUsize::new(0), millis: 0, size: (100.0, 50.0), broken: false };
        let later = later(&BACKEND);
        assert_eq!((later.pages(), later.size(0), later.size(1), later.natural(0)), (1, Some(UNKNOWN), None, false));
        let pool = later.pool(1);
        assert!(pool.try_recv().is_none());
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(BACKEND.opened.load(Ordering::Relaxed), 0, "asked for nothing yet");
        pool.set_queue(vec![request(1)]);
        pool.set_queue(vec![request(1)]);
        // Asked before its size was known: nothing was rendered, and the viewer is to ask again.
        let again = next(&*pool);
        assert!(again.again && again.request.tag == 1 && again.error.is_none());
        assert!(pool.try_recv().is_none(), "answered once");
        pool.set_queue(vec![request(1)]);
        assert!(pool.try_recv().is_none(), "listed again before it was received: answered once");
        // The viewer lays the tile out afresh, at the size now known.
        assert_eq!((later.size(0), later.size(2), later.natural(0)), (Some((100.0, 50.0)), Some((100.0, 50.0)), true));
        pool.set_queue(vec![request(2)]);
        let rendered = next(&*pool);
        assert!(!rendered.again && rendered.width == 1 && rendered.request.tag == 2);
        assert_eq!(BACKEND.opened.load(Ordering::Relaxed), 1);
        // A new pool, the reader back: nothing is opened again, and no render asked again.
        let pool = later.pool(1);
        pool.set_queue(vec![request(3)]);
        assert!(!next(&*pool).again);
        assert_eq!(BACKEND.opened.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_pool_let_go_of_leaves_what_it_began_opening_for_the_next() {
        static BACKEND: Test = Test { opened: AtomicUsize::new(0), millis: 30, size: (100.0, 50.0), broken: false };
        let later = later(&BACKEND);
        let pool = later.pool(1);
        pool.set_queue(vec![request(1)]);
        let start = Instant::now();
        while BACKEND.opened.load(Ordering::Relaxed) == 0 {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::yield_now();
        }
        drop(pool);
        while !later.0.finished() {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(later.outcome().is_some_and(|opened| opened.is_ok()));
        assert_eq!(later.size(0), Some((100.0, 50.0)));
        assert_eq!(BACKEND.opened.load(Ordering::Relaxed), 1);
    }

    /// A thread finishing changes nothing a layout reads, or a layout under way could have its size
    /// from before and the rest from after, and what it asks for be rendered at the wrong scale. Only
    /// a frame taken between layouts, which also says to lay out again, makes the size known.
    #[test]
    fn a_size_comes_to_be_known_only_between_layouts() {
        static BACKEND: Test = Test { opened: AtomicUsize::new(0), millis: 0, size: (999.0, 200.0), broken: false };
        let later = later(&BACKEND);
        let pool = later.pool(1);
        pool.set_queue(vec![request(1)]);
        let start = Instant::now();
        while !later.0.finished() {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!((later.size(0), later.natural(0)), (Some(UNKNOWN), false), "finished, and not yet taken");
        // What was worked out from that, listed after it finished, is still not rendered.
        pool.set_queue(vec![request(2)]);
        let again = next(&*pool);
        assert!(again.again && again.request.tag == 2);
        assert_eq!((later.size(0), later.natural(0)), (Some((999.0, 200.0)), true));
        pool.set_queue(vec![request(3)]);
        assert!(!next(&*pool).again);
    }

    #[test]
    fn work_not_begun_is_forgotten_when_nothing_wants_it() {
        static BACKEND: Test = Test { opened: AtomicUsize::new(0), millis: 40, size: (1.0, 1.0), broken: false };
        let openers = Rc::new(Openers::default());
        let file = std::env::temp_dir().join(format!("clview-later-forgotten-{}", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        let path = file.to_str().unwrap().to_owned();
        let made: Vec<Later> = (0..THREADS + 3)
            .map(|_| Later::new(Reopen::Disk(path.clone()), path.clone(), &[], &BACKEND, Arc::new(Parsed::default().owned()), openers.clone(), ROOM))
            .collect();
        let pools: Vec<Box<dyn Pool>> = made.iter().map(|later| later.pool(1)).collect();
        for pool in &pools {
            pool.set_queue(vec![request(1)]);
        }
        // Those not begun are left, and the threads go quiet once those begun are done: none is
        // queued, so none begins after. Waited for rather than slept on, as a slow machine's 40 ms
        // open outlasted a sleep of 150.
        for pool in &pools {
            pool.set_queue(Vec::new());
        }
        let busy = |later: &Later| matches!(*lock(&later.0.slot.stage), Stage::Queued | Stage::Opening);
        let start = Instant::now();
        while made.iter().any(busy) {
            assert!(start.elapsed() < Duration::from_secs(5), "still opening");
            std::thread::sleep(Duration::from_millis(1));
        }
        let opened = BACKEND.opened.load(Ordering::Relaxed);
        assert!(opened <= THREADS, "{opened} opened: at most what the threads had begun");
        assert_eq!(made.iter().filter(|later| later.0.finished()).count(), opened);
    }

    #[test]
    fn a_file_that_cant_be_shown_says_why_each_time_it_is_listed_anew() {
        static BACKEND: Test = Test { opened: AtomicUsize::new(0), millis: 0, size: (1.0, 1.0), broken: true };
        let later = later(&BACKEND);
        let pool = later.pool(1);
        pool.set_queue(vec![request(1)]);
        assert_eq!(next(&*pool).error.as_deref(), Some("broken"), "named by the list, not by its path");
        assert_eq!(later.size(0), Some(UNKNOWN));
        assert_eq!(later.outcome().map(|o| o.err()), Some(Some("broken".to_owned())));
        pool.set_queue(vec![request(2)]);
        assert_eq!(next(&*pool).error.as_deref(), Some("broken"));
        assert!(pool.try_recv().is_none());
        pool.set_queue(vec![request(2)]);
        assert!(pool.try_recv().is_none(), "said once");
        assert_eq!(BACKEND.opened.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn going_into_it_keeps_it_wanted_and_gets_all_its_pages() {
        static BACKEND: Test = Test { opened: AtomicUsize::new(0), millis: 0, size: (100.0, 50.0), broken: false };
        let later = later(&BACKEND);
        let start = Instant::now();
        let pool = later.pool(1);
        // Off screen as the reader goes in, its tile forgotten once, as a list does: asked again, it
        // opens all the same.
        assert!(later.outcome().is_none());
        pool.set_queue(Vec::new());
        let opened = loop {
            if let Some(opened) = later.outcome() {
                break opened.unwrap();
            }
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!((opened.pages(), later.pages()), (3, 1));
    }

    #[test]
    fn a_file_is_closed_until_wanted_unless_it_couldnt_be_opened_again() {
        let file = std::env::temp_dir().join(format!("clview-later-input-{}", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        let path = file.to_str().unwrap();
        assert!(matches!(Input::open(path).unwrap().later(), Ok(Reopen::Disk(p)) if p == path));
        // A device, as a pipe is: read now or never.
        assert!(Input::open("/dev/null").unwrap().later().is_err());
        let again: Again = Arc::new(|| Ok(b"whole".to_vec()));
        let Ok(held) = Input::held("a.zip/b".into(), b"wh".to_vec(), again).later() else { panic!("held") };
        assert_eq!(held.input().unwrap().bytes().unwrap(), b"whole");
    }

    /// A job runs on the threads before the files waiting, its result taken once through its handle;
    /// one that panics costs itself alone; a handle dropped before its job began leaves it unrun.
    #[test]
    fn a_job_runs_first_and_its_panic_is_caught() {
        // The first file opens quickly and the others slowly, so its thread alone is free when the
        // job is taken: the others, finishing together, would begin the files waiting as it ran.
        static FIRST: Test = Test { opened: AtomicUsize::new(0), millis: 50, size: (1.0, 1.0), broken: false };
        static BACKEND: Test = Test { opened: AtomicUsize::new(0), millis: 400, size: (1.0, 1.0), broken: false };
        let openers = Rc::new(Openers::default());
        let file = std::env::temp_dir().join(format!("clview-later-job-{}", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        let path = file.to_str().unwrap().to_owned();
        // More files than threads: the last of them wait.
        let made: Vec<Later> = (0..THREADS + 3)
            .map(|i| {
                let backend: &'static Test = if i == 0 { &FIRST } else { &BACKEND };
                Later::new(Reopen::Disk(path.clone()), path.clone(), &[], backend, Arc::new(Parsed::default().owned()), openers.clone(), ROOM)
            })
            .collect();
        let pools: Vec<Box<dyn Pool>> = made.iter().map(|later| later.pool(1)).collect();
        for pool in &pools {
            pool.set_queue(vec![request(1)]);
        }
        let ran = Arc::new(AtomicUsize::new(0));
        let counted = ran.clone();
        // Counted as the job runs: once it is done, the threads are free to begin the files waiting.
        let begun = Arc::new(AtomicUsize::new(usize::MAX));
        let seen = begun.clone();
        let handle = openers.run(Box::new(move || {
            seen.store(FIRST.opened.load(Ordering::SeqCst) + BACKEND.opened.load(Ordering::SeqCst), Ordering::SeqCst);
            counted.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(Fake((7.0, 7.0))) as Box<dyn Source + Send>)
        }));
        let start = Instant::now();
        let source = loop {
            if let Some(done) = handle.try_take() {
                break done.unwrap();
            }
            assert!(start.elapsed() < Duration::from_secs(5), "no result");
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!((source.size(0), ran.load(Ordering::Relaxed)), (Some((7.0, 7.0)), 1));
        let begun = begun.load(Ordering::SeqCst);
        assert!(begun <= THREADS, "the job went before the files waiting, yet {begun} files had begun");
        assert!(handle.try_take().is_none(), "taken once");
        let panics = openers.run(Box::new(|| panic!("a bug in the layout")));
        let start = Instant::now();
        while panics.try_take().is_none_or(|done| {
            assert_eq!(done.err().as_deref(), Some("the layout failed"));
            false
        }) {
            assert!(start.elapsed() < Duration::from_secs(5), "no result");
            std::thread::sleep(Duration::from_millis(1));
        }
        // Queued while every thread opens a slow file, and dropped before it begins: never run.
        static SLOW: Test = Test { opened: AtomicUsize::new(0), millis: 300, size: (1.0, 1.0), broken: false };
        let slow: Vec<Later> = (0..THREADS)
            .map(|_| Later::new(Reopen::Disk(path.clone()), path.clone(), &[], &SLOW, Arc::new(Parsed::default().owned()), openers.clone(), ROOM))
            .collect();
        let slow_pools: Vec<Box<dyn Pool>> = slow.iter().map(|later| later.pool(1)).collect();
        for pool in &slow_pools {
            pool.set_queue(vec![request(1)]);
        }
        let start = Instant::now();
        while SLOW.opened.load(Ordering::Relaxed) < THREADS {
            assert!(start.elapsed() < Duration::from_secs(5), "the slow files never began");
            std::thread::sleep(Duration::from_millis(1));
        }
        let never = Arc::new(AtomicUsize::new(0));
        let counted = never.clone();
        let dropped = openers.run(Box::new(move || {
            counted.fetch_add(1, Ordering::Relaxed);
            Err("ran".into())
        }));
        drop(dropped);
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(never.load(Ordering::Relaxed), 0);
        drop((pools, slow_pools));
    }

    #[test]
    fn printing_opens_it_on_the_spot() {
        static BACKEND: Test = Test { opened: AtomicUsize::new(0), millis: 0, size: (100.0, 50.0), broken: false };
        let later = later(&BACKEND);
        later.settle(0);
        assert_eq!((later.size(0), BACKEND.opened.load(Ordering::Relaxed)), (Some((100.0, 50.0)), 1));
        assert_eq!(later.renderer().unwrap().render(request(1)).width, 1);
        static BROKEN: Test = Test { opened: AtomicUsize::new(0), millis: 0, size: (1.0, 1.0), broken: true };
        assert!(self::later(&BROKEN).renderer().is_err_and(|e| e == "broken"));
    }
}
