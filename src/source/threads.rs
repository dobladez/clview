//! Render threads for a backend without threads of its own: each takes the most urgent render
//! no thread has taken, and its result is delivered only while it is still wanted. A render
//! can't be stopped once begun; one no longer wanted is dropped when it finishes.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use super::{Frame, Pool, Request, guarded};

/// What a thread renders with.
pub(crate) type RenderFn = Arc<dyn Fn(Request) -> Frame + Send + Sync>;

/// [`Pool`] over threads that call a [`RenderFn`].
pub(crate) struct Threads {
    shared: Arc<Shared>,
    results: Receiver<Frame>,
}

struct Shared {
    state: Mutex<State>,
    /// Work wanted, or time to stop.
    wake: Condvar,
}

#[derive(Default)]
struct State {
    /// The latest queue, the most urgent first.
    queue: Vec<Request>,
    /// Being rendered.
    taken: Vec<Request>,
    /// Rendered and delivered while still in the queue: not rendered again when listed again,
    /// as the pager may before it has received the result.
    delivered: Vec<Request>,
    stop: bool,
}

impl State {
    /// The most urgent render no thread has taken, now taken.
    fn take(&mut self) -> Option<Request> {
        let next = *self.queue.iter().find(|r| !self.taken.contains(r) && !self.delivered.contains(r))?;
        self.taken.push(next);
        Some(next)
    }
}

impl Threads {
    pub(crate) fn new(threads: usize, render: RenderFn) -> Self {
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), wake: Condvar::new() });
        let (sender, results) = channel();
        for _ in 0..threads.max(1) {
            let (shared, render, sender) = (shared.clone(), render.clone(), sender.clone());
            // A thread that can't start leaves the others; with none, renders wait for good,
            // which the pager shows as a page still rendering.
            let _ = std::thread::Builder::new().name("render".into()).spawn(move || work(&shared, &*render, &sender));
        }
        Self { shared, results }
    }
}

impl Pool for Threads {
    fn set_queue(&self, requests: Vec<Request>) {
        let mut state = lock(&self.shared.state);
        state.delivered.retain(|r| requests.contains(r));
        state.queue = requests;
        self.shared.wake.notify_all();
    }

    fn try_recv(&self) -> Option<Frame> {
        self.results.try_recv().ok()
    }
}

impl Drop for Threads {
    /// The threads stop once their render is done; nothing waits for them.
    fn drop(&mut self) {
        lock(&self.shared.state).stop = true;
        self.shared.wake.notify_all();
    }
}

fn work(shared: &Shared, render: &(dyn Fn(Request) -> Frame + Send + Sync), results: &Sender<Frame>) {
    loop {
        let request = {
            let mut state = lock(&shared.state);
            loop {
                if state.stop {
                    return;
                }
                if let Some(request) = state.take() {
                    break request;
                }
                state = shared.wake.wait(state).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let frame = guarded(request, || render(request));
        let mut state = lock(&shared.state);
        state.taken.retain(|r| *r != request);
        if state.queue.contains(&request) {
            state.delivered.push(request);
            drop(state);
            // The pool is gone: the thread stops at the top of the loop.
            let _ = results.send(frame);
        }
    }
}

/// The lock, even after a thread panicked holding it: the state stays consistent between
/// statements, and renders run unlocked.
fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::source::Pixels;

    fn request(page: usize) -> Request {
        Request { page, scale: 1.0, tag: page as u64, ..Default::default() }
    }

    /// A 1 × 1 page whose one pixel is its page number.
    fn frame(request: Request) -> Frame {
        let pixel = u8::try_from(request.page).unwrap();
        Frame { request, width: 1, height: 1, rgba: Pixels::new(vec![pixel; 4]), error: None, warnings: Vec::new(), again: false }
    }

    /// Every result until `count` have come, or five seconds have passed.
    fn results(pool: &Threads, count: usize) -> Vec<Frame> {
        let (mut out, start) = (Vec::new(), Instant::now());
        while out.len() < count && start.elapsed() < Duration::from_secs(5) {
            match pool.try_recv() {
                Some(frame) => out.push(frame),
                None => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        out
    }

    #[test]
    fn every_render_queued_arrives_once() {
        let renders = Arc::new(AtomicUsize::new(0));
        let counted = renders.clone();
        let pool = Threads::new(
            3,
            Arc::new(move |r| {
                counted.fetch_add(1, Ordering::Relaxed);
                frame(r)
            }),
        );
        let queue: Vec<Request> = (0..20).map(request).collect();
        pool.set_queue(queue.clone());
        // Listed again before the results are taken, as the pager does: nothing renders twice.
        pool.set_queue(queue.clone());
        let mut pages: Vec<usize> = results(&pool, 20).iter().map(|f| f.request.page).collect();
        pool.set_queue(queue);
        pages.sort_unstable();
        assert_eq!(pages, (0..20).collect::<Vec<_>>());
        std::thread::sleep(Duration::from_millis(20));
        assert!(pool.try_recv().is_none());
        assert_eq!(renders.load(Ordering::Relaxed), 20);
    }

    #[test]
    fn a_render_no_longer_wanted_is_not_delivered() {
        let (started, release) = (Arc::new(Mutex::new(false)), Arc::new((Mutex::new(false), Condvar::new())));
        let (seen, gate) = (started.clone(), release.clone());
        let pool = Threads::new(
            1,
            Arc::new(move |r| {
                *seen.lock().unwrap() = true;
                let (open, opened) = &*gate;
                let mut open = open.lock().unwrap();
                while !*open {
                    open = opened.wait(open).unwrap();
                }
                frame(r)
            }),
        );
        pool.set_queue(vec![request(1)]);
        while !*started.lock().unwrap() {
            std::thread::yield_now();
        }
        // Page 1 is rendering when the reader moves on to page 2.
        pool.set_queue(vec![request(2)]);
        *release.0.lock().unwrap() = true;
        release.1.notify_all();
        let pages: Vec<usize> = results(&pool, 1).iter().map(|f| f.request.page).collect();
        assert_eq!(pages, [2]);
        std::thread::sleep(Duration::from_millis(20));
        assert!(pool.try_recv().is_none());
    }

    #[test]
    fn a_render_that_panics_comes_back_failed_and_the_threads_go_on() {
        let pool = Threads::new(1, Arc::new(|r: Request| if r.page == 1 { panic!("a decoder's bug") } else { frame(r) }));
        pool.set_queue(vec![request(1), request(2)]);
        let mut frames = results(&pool, 2);
        frames.sort_by_key(|f| f.request.page);
        assert_eq!(frames.iter().map(|f| f.request.page).collect::<Vec<_>>(), [1, 2]);
        assert!(frames[0].error.as_deref().is_some_and(|e| e.contains("failed")), "{:?}", frames[0].error);
        assert_eq!((frames[1].error.as_deref(), &*frames[1].rgba), (None, &[2u8; 4][..]));
    }
}
