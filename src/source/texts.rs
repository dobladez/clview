//! The queue a backend's [`Texts`] keeps: a thread of its own that takes the most urgent request
//! not yet answered, has the backend's [`Answer`] give it, and holds the answer for the viewer
//! while its request is still listed. A backend lends it how its text is read and its matches
//! placed, and what to do when the search ends ([`Ending`]), as `threads.rs` serves [`Pool`].
//!
//! A backend's [`Links`] keeps one too: the queue is the same whatever is asked of it and
//! answered ([`Reply`]), a search's [`Got`] or a page's links ([`Told`]), a backend lending it
//! how each is answered ([`Queue::serve`]).
//!
//! Nothing here is looked up by going through a list: a document has as many requests as it has
//! chunks, and the lock is the one the viewer takes between keys ([`Texts::try_recv`],
//! [`Texts::set_queue`]).
//!
//! [`Pool`]: super::Pool

use std::collections::{HashSet, VecDeque};
use std::hash::Hash;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use super::{Ask, Got, Link, Target, Told, Want};
#[cfg(doc)]
use super::{Links, Texts};

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

/// What a queue's thread gives: an answer that says what it answers, and what it weighs while
/// it waits to be taken.
pub(crate) trait Reply: Send + 'static {
    /// What is asked for it.
    type Asked: Clone + Eq + Hash + Send + 'static;
    /// What this answers.
    fn asked(&self) -> Self::Asked;
    /// The bytes it holds while it waits, near enough: what it gives, or why not, and itself.
    fn weight(&self) -> usize;
}

/// The queue: what is wanted, and the answers waiting for the viewer. A backend's [`Texts`] hands
/// it [`Texts::set_queue`] and [`Texts::try_recv`], and its [`Links`] theirs. Dropped, the thread
/// stops at its next request, and what the backend set to be done when the search ends
/// ([`Ending::on_end`]) is done, without waiting for the thread.
pub(crate) struct Queue<G: Reply = Got> {
    shared: Arc<Shared<G>>,
}

/// The reader's hold on its queue: what to do when the search ends, which may change as it reads
/// (a PDF's worker, started and killed).
pub(crate) struct Ending<G: Reply = Got>(Arc<Shared<G>>);

struct Shared<G: Reply> {
    state: Mutex<State<G>>,
    /// Work wanted, room for its answer, or time to stop.
    wake: Condvar,
}

struct State<G: Reply> {
    /// The latest queue, the most urgent first; what it lists, to look one up; and how far into it
    /// every request has been given, where the next one to read is looked for from.
    queue: Vec<G::Asked>,
    listed: HashSet<G::Asked>,
    from: usize,
    /// Given while still in the queue: not read again when listed again.
    given: HashSet<G::Asked>,
    /// The answers given and not yet taken, each to a request still in the queue, and the bytes
    /// they hold ([`Reply::weight`]): the thread reads no more while that is [`MAX_WAITING`] or over.
    answers: VecDeque<G>,
    waiting: usize,
    /// What to do when the search ends, as the reader last set it.
    end: Option<Box<dyn FnOnce() + Send>>,
    stop: bool,
}

impl<G: Reply> Default for State<G> {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            listed: HashSet::new(),
            from: 0,
            given: HashSet::new(),
            answers: VecDeque::new(),
            waiting: 0,
            end: None,
            stop: false,
        }
    }
}

impl<G: Reply> Shared<G> {
    /// The most urgent thing wanted and not yet given, waited for, and for room for its answer.
    /// `None` when the search has ended.
    fn next(&self) -> Option<G::Asked> {
        let mut state = lock(&self.state);
        loop {
            if state.stop {
                return None;
            }
            // Past what was given, each looked at once for a queue: a request given is never ungiven
            // while its queue stands.
            while state.waiting < MAX_WAITING
                && let Some(want) = state.queue.get(state.from).cloned()
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
    fn give(&self, want: G::Asked, got: G) -> bool {
        let mut state = lock(&self.state);
        if state.listed.contains(&want) && !state.stop {
            state.given.insert(want);
            state.waiting = state.waiting.saturating_add(got.weight());
            state.answers.push_back(got);
        }
        !state.stop
    }
}

impl Reply for Got {
    type Asked = Want;

    fn asked(&self) -> Want {
        match self {
            Self::Text(chunk, _) => Want::Text(*chunk),
            Self::Rects(want, _) => *want,
        }
    }

    /// Its text or its rectangles, or why not, and itself.
    fn weight(&self) -> usize {
        let held = match self {
            Self::Text(_, Ok(text)) | Self::Text(_, Err(text)) | Self::Rects(_, Err(text)) => text.len(),
            Self::Rects(_, Ok(rects)) => rects.len().saturating_mul(std::mem::size_of::<(usize, [f32; 4])>()),
        };
        held.saturating_add(std::mem::size_of::<Self>())
    }
}

impl Reply for Told {
    type Asked = Ask;

    fn asked(&self) -> Ask {
        match self {
            Self::Page { page, .. } => Ask::Page(*page),
            Self::Name { name, .. } => Ask::Name(name.clone()),
        }
    }

    /// Its links, each with its rectangles, its address and where it leads, or why not, its name,
    /// and itself.
    fn weight(&self) -> usize {
        let link = |link: &Link| {
            let target = match &link.target {
                Target::Place { .. } => 0,
                Target::File { path, fragment, .. } => path.len().saturating_add(fragment.as_ref().map_or(0, String::len)),
                Target::Web(text) | Target::Other(text) => text.len(),
            };
            let rects = link.rects.len().saturating_mul(std::mem::size_of::<[f32; 4]>());
            let held = rects.saturating_add(link.address.as_ref().map_or(0, String::len)).saturating_add(target);
            held.saturating_add(std::mem::size_of::<Link>())
        };
        let held = match self {
            Self::Page { links: Ok(links), .. } => links.iter().map(link).fold(0, usize::saturating_add),
            Self::Page { links: Err(why), .. } => why.len(),
            Self::Name { name, at } => name.len().saturating_add(at.as_ref().err().map_or(0, String::len)),
        };
        held.saturating_add(std::mem::size_of::<Self>())
    }
}

impl Queue {
    /// Answer what is wanted with `read`, on a thread named `name`. A thread that can't start
    /// leaves the text unread, which the search shows as still reading.
    pub(crate) fn start(&self, name: &str, mut read: impl Answer) {
        self.serve(name, move |want| match want {
            Want::Text(chunk) => Got::Text(chunk, read.text(chunk)),
            Want::Rects { chunk, from, to } => Got::Rects(want, read.rects(chunk, from, to)),
        });
    }
}

impl<G: Reply> Queue<G> {
    /// A queue with nothing wanted and no thread yet ([`Queue::start`], [`Queue::serve`]).
    pub(crate) fn new() -> Self {
        Self { shared: Arc::new(Shared { state: Mutex::new(State::default()), wake: Condvar::new() }) }
    }

    /// The hold a reader of this queue keeps on it.
    #[cfg_attr(not(any(feature = "pdf", test)), expect(dead_code, reason = "only a PDF's reader has something to end"))]
    pub(crate) fn ending(&self) -> Ending<G> {
        Ending(self.shared.clone())
    }

    /// Answer what is wanted with `answer`, on a thread named `name`: what it gives is held as the
    /// answer to what it was asked, whatever it says it answers. A thread that can't start leaves
    /// everything unanswered.
    pub(crate) fn serve(&self, name: &str, mut answer: impl FnMut(G::Asked) -> G + Send + 'static) {
        let shared = self.shared.clone();
        let _ = std::thread::Builder::new().name(name.into()).spawn(move || {
            while let Some(want) = shared.next() {
                let got = answer(want.clone());
                if !shared.give(want, got) {
                    return;
                }
            }
        });
    }

    /// [`Texts::set_queue`]. An answer waiting for a request no longer listed is dropped with it:
    /// listed again, it is read again.
    pub(crate) fn set_queue(&self, wanted: Vec<G::Asked>) {
        // Made before the lock is taken: the thread reads on meanwhile.
        let listed: HashSet<G::Asked> = wanted.iter().cloned().collect();
        let mut state = lock(&self.shared.state);
        state.given.retain(|given| listed.contains(given));
        state.answers.retain(|got| listed.contains(&got.asked()));
        state.waiting = state.answers.iter().map(Reply::weight).fold(0, usize::saturating_add);
        state.queue = wanted;
        state.listed = listed;
        state.from = 0;
        self.shared.wake.notify_all();
    }

    /// [`Texts::try_recv`].
    pub(crate) fn try_recv(&self) -> Option<G> {
        let mut state = lock(&self.shared.state);
        let got = state.answers.pop_front()?;
        state.waiting = state.waiting.saturating_sub(got.weight());
        // Room again, maybe: the thread reads on.
        self.shared.wake.notify_all();
        Some(got)
    }
}

impl<G: Reply> Drop for Queue<G> {
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

impl<G: Reply> Ending<G> {
    /// What to do when the search ends, from now on, in place of what was set before; nothing,
    /// for `None`. `false` if it has ended already, when `end` is left undone: the reader is to do
    /// it itself.
    #[cfg_attr(not(any(feature = "pdf", test)), expect(dead_code, reason = "only a PDF's reader has something to end"))]
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
fn lock<G: Reply>(state: &Mutex<State<G>>) -> MutexGuard<'_, State<G>> {
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
        let queue: Queue = Queue::new();
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
        let queue: Queue = Queue::new();
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

    /// The queue serves a source's links as it does its text: what is asked, a page's links or a
    /// name's place, is answered once while listed, and what waits is weighed by what it holds, its
    /// links' rectangles, addresses and paths among it.
    #[test]
    fn a_queue_of_links_answers_pages_and_names_and_weighs_what_waits() {
        let queue: Queue<Told> = Queue::new();
        let link = |page| Link {
            page,
            rects: vec![[0.0; 4]; 3],
            address: Some("x".repeat(100)),
            target: Target::File { path: "y".repeat(1000), fragment: Some("z".repeat(10)), page: None },
        };
        queue.serve("links", move |ask| match ask {
            Ask::Page(page) if page < 2 => Told::Page { page, links: Ok(vec![link(page), link(page)]), more: 0 },
            Ask::Page(page) => Told::Page { page, links: Err(format!("no page {page}")), more: 0 },
            Ask::Name(name) => Told::Name { at: Ok((name == "usage").then_some((1, 5.0))), name },
        });
        let wanted = vec![Ask::Name("usage".into()), Ask::Page(1), Ask::Page(7), Ask::Name("nowhere".into())];
        queue.set_queue(wanted.clone());
        let begun = Instant::now();
        while lock(&queue.shared.state).answers.len() < 4 {
            assert!(begun.elapsed() < Duration::from_secs(10), "its answers never came");
            std::thread::sleep(Duration::from_millis(1));
        }
        // Two links of 48 bytes of rectangles, 100 of address and 1,010 of path and fragment, in one
        // answer; the others weigh their words.
        let held = 2 * (3 * 16 + 100 + 1010 + std::mem::size_of::<Link>()) + "usage".len() + "no page 7".len() + "nowhere".len();
        assert_eq!(lock(&queue.shared.state).waiting, held + 4 * std::mem::size_of::<Told>());
        // Listed again while they wait: given once, in the order asked.
        queue.set_queue(wanted);
        let got: Vec<Told> = std::iter::from_fn(|| queue.try_recv()).collect();
        let [
            Told::Name { name, at: Ok(Some((1, y))) },
            Told::Page { page: 1, links: Ok(links), .. },
            Told::Page { page: 7, links: Err(why), .. },
            Told::Name { at: Ok(None), .. },
        ] = &got[..]
        else {
            panic!("{} told", got.len())
        };
        assert_eq!((name.as_str(), *y, links.len(), why.as_str()), ("usage", 5.0, 2, "no page 7"));
        assert_eq!(lock(&queue.shared.state).waiting, 0);
        // An answer to what is no longer asked never comes.
        queue.set_queue(vec![Ask::Page(0)]);
        while lock(&queue.shared.state).answers.is_empty() {
            assert!(begun.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(1));
        }
        queue.set_queue(vec![Ask::Name("usage".into())]);
        assert!(!matches!(queue.try_recv(), Some(Told::Page { .. })));
    }

    /// What the reader set to be done when the search ends is done when the queue is dropped, the
    /// latest of what it set, once; set after, it is refused, for the reader to do itself.
    #[test]
    fn the_search_ending_does_what_the_reader_set_last() {
        let queue: Queue = Queue::new();
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
