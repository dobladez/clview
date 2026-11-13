//! The queue a backend's [`Texts`] keeps: a thread of its own that takes the most urgent request
//! not yet answered, has the backend's [`Answer`] give it, and holds the answer for the viewer
//! while its request is still listed. A backend lends it how its text is read and its matches
//! placed, and what to do when the search ends ([`Ending`]), as `threads.rs` serves [`Pool`].
//!
//! Nothing here is looked up by going through a list: a document has as many requests as it has
//! chunks, and the lock is the one the viewer takes between keys ([`Texts::try_recv`],
//! [`Texts::set_queue`]).
//!
//! [`Pool`]: super::Pool

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

#[cfg(doc)]
use super::Texts;
use super::{Got, Want};

/// The most bytes of answers that wait to be taken: at it, no more is read until the viewer has
/// taken some. What a viewer busy elsewhere (writing to a slow terminal) leaves waiting is that
/// and one answer more, the one being read, whatever the document.
const MAX_WAITING: usize = 4 << 20;

/// How a backend answers what the viewer wants of its text, on the queue's thread.
pub(crate) trait Answer: Send + 'static {
    /// Chunk `chunk`'s text, or why not, in words for the reader.
    fn text(&mut self, chunk: usize) -> Result<String, String>;
    /// Where the characters `from..to` of chunk `chunk`'s text are ([`Got::Rects`]), or why not.
    fn rects(&mut self, chunk: usize, from: usize, to: usize) -> Result<Vec<(usize, [f32; 4])>, String>;
}

/// The queue: what is wanted, and the answers waiting for the viewer. A backend's [`Texts`] hands
/// it [`Texts::set_queue`] and [`Texts::try_recv`]. Dropped, the thread stops at its next request,
/// and what the backend set to be done when the search ends ([`Ending::on_end`]) is done, without
/// waiting for the thread.
pub(crate) struct Queue {
    shared: Arc<Shared>,
}

/// The reader's hold on its queue: what to do when the search ends, which may change as it reads
/// (a PDF's worker, started and killed).
pub(crate) struct Ending(Arc<Shared>);

struct Shared {
    state: Mutex<State>,
    /// Work wanted, room for its answer, or time to stop.
    wake: Condvar,
}

#[derive(Default)]
struct State {
    /// The latest queue, the most urgent first; what it lists, to look one up; and how far into it
    /// every request has been given, where the next one to read is looked for from.
    queue: Vec<Want>,
    listed: HashSet<Want>,
    from: usize,
    /// Given while still in the queue: not read again when listed again.
    given: HashSet<Want>,
    /// The answers given and not yet taken, each to a request still in the queue, and the bytes
    /// they hold ([`weight`]): the thread reads no more while that is [`MAX_WAITING`] or over.
    answers: VecDeque<Got>,
    waiting: usize,
    /// What to do when the search ends, as the reader last set it.
    end: Option<Box<dyn FnOnce() + Send>>,
    stop: bool,
}

impl Shared {
    /// The most urgent thing wanted and not yet given, waited for, and for room for its answer.
    /// `None` when the search has ended.
    fn next(&self) -> Option<Want> {
        let mut state = lock(&self.state);
        loop {
            if state.stop {
                return None;
            }
            // Past what was given, each looked at once for a queue: a request given is never ungiven
            // while its queue stands.
            while state.waiting < MAX_WAITING
                && let Some(want) = state.queue.get(state.from).copied()
            {
                if !state.given.contains(&want) {
                    return Some(want);
                }
                state.from = state.from.saturating_add(1);
            }
            state = self.wake.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The answer to `want`, for the viewer to take, if it is still wanted: under the lock the queue
    /// is replaced under, so none is ever given for a request no longer listed. Whether the search
    /// goes on.
    fn give(&self, want: Want, got: Got) -> bool {
        let mut state = lock(&self.state);
        if state.listed.contains(&want) && !state.stop {
            state.given.insert(want);
            state.waiting = state.waiting.saturating_add(weight(&got));
            state.answers.push_back(got);
        }
        !state.stop
    }
}

/// What `got` answers.
fn asked(got: &Got) -> Want {
    match got {
        Got::Text(chunk, _) => Want::Text(*chunk),
        Got::Rects(want, _) => *want,
    }
}

/// The bytes an answer holds while it waits, near enough: its text or its rectangles, or why not,
/// and itself.
fn weight(got: &Got) -> usize {
    let held = match got {
        Got::Text(_, Ok(text)) | Got::Text(_, Err(text)) | Got::Rects(_, Err(text)) => text.len(),
        Got::Rects(_, Ok(rects)) => rects.len().saturating_mul(std::mem::size_of::<(usize, [f32; 4])>()),
    };
    held.saturating_add(std::mem::size_of::<Got>())
}

impl Queue {
    /// A queue with nothing wanted and no thread yet ([`Queue::start`]).
    pub(crate) fn new() -> Self {
        Self { shared: Arc::new(Shared { state: Mutex::new(State::default()), wake: Condvar::new() }) }
    }

    /// The hold a reader of this queue keeps on it.
    pub(crate) fn ending(&self) -> Ending {
        Ending(self.shared.clone())
    }

    /// Answer what is wanted with `read`, on a thread named `name`. A thread that can't start
    /// leaves the text unread, which the search shows as still reading.
    pub(crate) fn start(&self, name: &str, mut read: impl Answer) {
        let shared = self.shared.clone();
        let _ = std::thread::Builder::new().name(name.into()).spawn(move || {
            while let Some(want) = shared.next() {
                let got = match want {
                    Want::Text(chunk) => Got::Text(chunk, read.text(chunk)),
                    Want::Rects { chunk, from, to } => Got::Rects(want, read.rects(chunk, from, to)),
                };
                if !shared.give(want, got) {
                    return;
                }
            }
        });
    }

    /// [`Texts::set_queue`]. An answer waiting for a request no longer listed is dropped with it:
    /// listed again, it is read again.
    pub(crate) fn set_queue(&self, wanted: Vec<Want>) {
        // Made before the lock is taken: the thread reads on meanwhile.
        let listed: HashSet<Want> = wanted.iter().copied().collect();
        let mut state = lock(&self.shared.state);
        state.given.retain(|given| listed.contains(given));
        state.answers.retain(|got| listed.contains(&asked(got)));
        state.waiting = state.answers.iter().map(weight).fold(0, usize::saturating_add);
        state.queue = wanted;
        state.listed = listed;
        state.from = 0;
        self.shared.wake.notify_all();
    }

    /// [`Texts::try_recv`].
    pub(crate) fn try_recv(&self) -> Option<Got> {
        let mut state = lock(&self.shared.state);
        let got = state.answers.pop_front()?;
        state.waiting = state.waiting.saturating_sub(weight(&got));
        // Room again, maybe: the thread reads on.
        self.shared.wake.notify_all();
        Some(got)
    }
}

impl Drop for Queue {
    /// The search has ended: the thread stops, and what the reader set to be done is done, outside
    /// the lock; nothing waits for the thread.
    fn drop(&mut self) {
        let end = {
            let mut state = lock(&self.shared.state);
            state.stop = true;
            state.end.take()
        };
        self.shared.wake.notify_all();
        if let Some(end) = end {
            end();
        }
    }
}

impl Ending {
    /// What to do when the search ends, from now on, in place of what was set before; nothing,
    /// for `None`. `false` if it has ended already, when `end` is left undone: the reader is to do
    /// it itself.
    #[cfg_attr(not(feature = "pdf"), expect(dead_code, reason = "only a PDF's reader has something to end"))]
    pub(crate) fn on_end(&self, end: Option<Box<dyn FnOnce() + Send>>) -> bool {
        let mut state = lock(&self.0.state);
        if state.stop {
            return false;
        }
        state.end = end;
        true
    }
}

/// The lock, even after a thread panicked holding it: the state stays consistent between statements.
fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use super::*;

    /// A reader of two chunks, `"zero"` and `"one"`, each range a rectangle as wide as it.
    struct Two;

    impl Answer for Two {
        fn text(&mut self, chunk: usize) -> Result<String, String> {
            ["zero", "one"].get(chunk).map(|text| (*text).to_owned()).ok_or_else(|| format!("no chunk {chunk}"))
        }
        fn rects(&mut self, chunk: usize, from: usize, to: usize) -> Result<Vec<(usize, [f32; 4])>, String> {
            Ok(vec![(chunk, [from as f32, 0.0, to as f32, 10.0])])
        }
    }

    /// What `queue` gives for `wanted`, each once, within ten seconds.
    fn gave(queue: &Queue, wanted: Vec<Want>) -> Vec<Got> {
        let (mut got, begun) = (Vec::new(), Instant::now());
        queue.set_queue(wanted.clone());
        while got.len() < wanted.len() && begun.elapsed() < Duration::from_secs(10) {
            // Listed again while waited for, as the viewer may: given once all the same.
            queue.set_queue(wanted.clone());
            match queue.try_recv() {
                Some(one) => got.push(one),
                None => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        std::thread::sleep(Duration::from_millis(20));
        assert!(queue.try_recv().is_none(), "given twice");
        got
    }

    /// What is wanted is answered by the reader, in the queue's order, each once while it stays
    /// listed, and what isn't wanted any more never.
    #[test]
    fn what_is_wanted_is_read_in_order_and_given_once() {
        let queue = Queue::new();
        queue.start("text", Two);
        let rects = Want::Rects { chunk: 0, from: 1, to: 3 };
        let got = gave(&queue, vec![Want::Text(1), rects, Want::Text(0), Want::Text(7)]);
        let [Got::Text(1, Ok(one)), Got::Rects(asked, Ok(placed)), Got::Text(0, Ok(zero)), Got::Text(7, Err(why))] = &got[..] else {
            panic!("{} given", got.len())
        };
        assert_eq!((one.as_str(), zero.as_str(), asked, why.as_str()), ("one", "zero", &rects, "no chunk 7"));
        assert_eq!(placed, &[(0, [1.0, 0.0, 3.0, 10.0])]);
        queue.set_queue(Vec::new());
        std::thread::sleep(Duration::from_millis(20));
        assert!(queue.try_recv().is_none());
    }

    /// An answer waiting when its request is taken off the queue is never given, nor is the request
    /// counted as given: listed again, it is read again, and given once.
    #[test]
    fn an_answer_whose_request_was_dropped_never_comes() {
        let queue = Queue::new();
        queue.start("text", Two);
        let rects = Want::Rects { chunk: 0, from: 1, to: 3 };
        queue.set_queue(vec![Want::Text(0), Want::Text(1), rects]);
        let begun = Instant::now();
        while lock(&queue.shared.state).answers.len() < 3 {
            assert!(begun.elapsed() < Duration::from_secs(10), "its answers never came");
            std::thread::sleep(Duration::from_millis(1));
        }
        // All three wait; only the second chunk's is still wanted.
        queue.set_queue(vec![Want::Text(1)]);
        assert!(matches!(queue.try_recv(), Some(Got::Text(1, Ok(one))) if one == "one"));
        assert!(queue.try_recv().is_none(), "an answer to a request no longer listed");
        queue.set_queue(Vec::new());
        {
            let state = lock(&queue.shared.state);
            assert!(state.answers.is_empty() && state.given.is_empty() && state.waiting == 0);
        }
        // Listed again: read again, given once.
        let got = gave(&queue, vec![rects, Want::Text(0)]);
        assert!(
            matches!(&got[..], [Got::Rects(asked, Ok(placed)), Got::Text(0, Ok(_))] if *asked == rects && placed.len() == 1),
            "{} given",
            got.len()
        );
        // And one being read when its request is dropped isn't given when it is done.
        let queue = Queue::new();
        let shared = queue.shared.clone();
        queue.set_queue(vec![Want::Text(0)]);
        assert_eq!(shared.next(), Some(Want::Text(0)));
        queue.set_queue(Vec::new());
        assert!(shared.give(Want::Text(0), Got::Text(0, Ok("read too late".into()))), "the search goes on");
        assert!(queue.try_recv().is_none() && lock(&shared.state).given.is_empty());
    }

    /// The answers that wait to be taken are bounded: with [`MAX_WAITING`] bytes of them waiting,
    /// nothing more is read until the viewer takes some, so a viewer that takes none for a while
    /// holds that and no more; taking them lets the reading go on, to the end. Here a megabyte a
    /// chunk, given as the thread gives them.
    #[test]
    fn answers_wait_for_the_viewer_within_a_budget() {
        let queue = Queue::new();
        let shared = queue.shared.clone();
        queue.set_queue((0..12).map(Want::Text).collect());
        let reader = std::thread::spawn(move || {
            let mut read = 0;
            while let Some(want) = shared.next() {
                let Want::Text(chunk) = want else { break };
                if !shared.give(want, Got::Text(chunk, Ok("x".repeat(1 << 20)))) {
                    break;
                }
                read += 1;
            }
            read
        });
        // Left alone, it stops at the budget: four chunks of a megabyte, never a fifth.
        std::thread::sleep(Duration::from_millis(200));
        {
            let state = lock(&queue.shared.state);
            assert_eq!(state.answers.len(), 4, "{} bytes waiting", state.waiting);
            assert!((MAX_WAITING..MAX_WAITING + (1 << 20)).contains(&state.waiting), "{}", state.waiting);
        }
        // Each one taken makes room for one more, to the last.
        let (mut chunks, begun) = (Vec::new(), Instant::now());
        while chunks.len() < 12 && begun.elapsed() < Duration::from_secs(10) {
            match queue.try_recv() {
                Some(Got::Text(chunk, Ok(text))) if text.len() == 1 << 20 => chunks.push(chunk),
                Some(_) => panic!("not what was given"),
                None => std::thread::sleep(Duration::from_millis(1)),
            }
            assert!(lock(&queue.shared.state).waiting < MAX_WAITING + (1 << 20));
        }
        assert_eq!(chunks, (0..12).collect::<Vec<_>>());
        assert_eq!(lock(&queue.shared.state).waiting, 0);
        // The search ended: the thread stops, though more may be wanted.
        queue.set_queue(vec![Want::Text(99)]);
        drop(queue);
        assert!((12..=13).contains(&reader.join().unwrap()));
    }

    /// What the viewer's side does stays quick whatever the chunks: sixty thousand of them, empty,
    /// are asked for, given and taken, the queue set again half way, in well under the time one
    /// search of them by hand would take. Looking each request up among those given, one after
    /// another, under the lock the viewer takes, made this the square of the chunks and worse.
    #[test]
    fn the_queue_stays_quick_with_tens_of_thousands_of_pages() {
        const PAGES: usize = 60_000;
        let queue = Queue::new();
        let shared = queue.shared.clone();
        let all: Vec<Want> = (0..PAGES).map(Want::Text).collect();
        let begun = Instant::now();
        queue.set_queue(all.clone());
        let reader = std::thread::spawn(move || {
            while let Some(want) = shared.next() {
                let Want::Text(page) = want else { break };
                if !shared.give(want, Got::Text(page, Ok(String::new()))) {
                    break;
                }
            }
        });
        let (mut taken, mut slowest, mut relisted) = (0usize, Duration::ZERO, Duration::ZERO);
        while taken < PAGES {
            assert!(begun.elapsed() < Duration::from_secs(20), "{taken} of {PAGES} pages taken in 20 s");
            // As the pager does between keys: all there is, then away for a moment.
            let asked = Instant::now();
            let got = queue.try_recv();
            slowest = slowest.max(asked.elapsed());
            match got {
                Some(_) => taken += 1,
                None => std::thread::sleep(Duration::from_millis(1)),
            }
            // Half way, the reader moves: the same pages wanted, in another order.
            if taken == PAGES / 2 && relisted.is_zero() {
                let asked = Instant::now();
                queue.set_queue(all.iter().rev().copied().collect());
                relisted = asked.elapsed().max(Duration::from_nanos(1));
            }
        }
        let took = begun.elapsed();
        drop(queue);
        reader.join().unwrap();
        eprintln!("{PAGES} pages in {took:?}; the slowest try_recv {slowest:?}; the queue set again in {relisted:?}");
        assert!(took < Duration::from_secs(5), "{took:?}");
        assert!(relisted < Duration::from_millis(500), "the queue set again in {relisted:?}");
    }

    /// What the reader set to be done when the search ends is done when the queue is dropped, the
    /// latest of what it set, once; set after, it is refused, for the reader to do itself.
    #[test]
    fn the_search_ending_does_what_the_reader_set_last() {
        let queue = Queue::new();
        let ending = queue.ending();
        let (first, last) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let flag = |done: &Arc<AtomicBool>| {
            let done = done.clone();
            Some(Box::new(move || done.store(true, Ordering::SeqCst)) as Box<dyn FnOnce() + Send>)
        };
        assert!(ending.on_end(flag(&first)) && ending.on_end(None) && ending.on_end(flag(&last)));
        drop(queue);
        assert!(!first.load(Ordering::SeqCst) && last.load(Ordering::SeqCst));
        let late = Arc::new(AtomicBool::new(false));
        assert!(!ending.on_end(flag(&late)), "set after the end");
        assert!(!late.load(Ordering::SeqCst));
    }
}
