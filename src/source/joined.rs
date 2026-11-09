//! Several sources shown as one: the pages each gives, one after another, numbered on through
//! them. Several files are shown this way, and a directory's files.
//!
//! What a part renders with is its own: a renderer or a pool of the part's, made when one of its
//! pages is first asked for. They are let go of when the reader is elsewhere, since what they
//! hold (an image's pool keeps its decoded pixels) adds up over hundreds of files.

use std::rc::Rc;
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{Frame, Pool, Render, Request, Source};

/// The most parts with a pool: the ones being rendered, and the ones rendered most recently, so
/// a reader going back and forth between neighbouring files doesn't wait for their pools again.
const KEPT: usize = 4;

/// A source among others: one the viewer may show on its own too, so shared.
pub(crate) struct Part {
    /// Its file's name, as its errors give it.
    pub(crate) name: String,
    pub(crate) source: Rc<dyn Source>,
    /// Its pages shown, in order.
    pub(crate) pages: Vec<usize>,
}

/// Sources shown one after another. Its pools share it, so they need no borrow of it.
pub(crate) struct Joined(Rc<Parts>);

struct Parts {
    parts: Vec<Part>,
    /// Where each part's pages start among the joined ones.
    starts: Vec<usize>,
    pages: usize,
}

impl Joined {
    /// `parts`, in order; one with no pages to show is left out.
    pub(crate) fn new(parts: Vec<Part>) -> Self {
        let parts: Vec<Part> = parts.into_iter().filter(|p| !p.pages.is_empty()).collect();
        let (mut starts, mut pages) = (Vec::with_capacity(parts.len()), 0usize);
        for part in &parts {
            starts.push(pages);
            pages = pages.saturating_add(part.pages.len());
        }
        Self(Rc::new(Parts { parts, starts, pages }))
    }
}

impl Parts {
    /// The part joined page `page` comes from, and its page there.
    fn locate(&self, page: usize) -> Option<(usize, usize)> {
        let at = self.starts.partition_point(|start| *start <= page).checked_sub(1)?;
        let first = *self.starts.get(at)?;
        let local = *self.parts.get(at)?.pages.get(page.checked_sub(first)?)?;
        Some((at, local))
    }

    /// `frame`, which part `part` rendered for `asked`: as asked, and failed in words that name
    /// the file.
    fn joined(&self, part: usize, mut frame: Frame, asked: Request) -> Frame {
        frame.request = asked;
        if let (Some(error), Some(part)) = (frame.error.as_mut(), self.parts.get(part)) {
            *error = format!("{}: {error}", part.name);
        }
        frame
    }
}

impl Source for Joined {
    fn pages(&self) -> usize {
        self.0.pages
    }

    fn size(&self, page: usize) -> Option<(f32, f32)> {
        let (part, local) = self.0.locate(page)?;
        self.0.parts.get(part)?.source.size(local)
    }

    fn natural(&self, page: usize) -> bool {
        self.0.locate(page).and_then(|(part, local)| Some(self.0.parts.get(part)?.source.natural(local))).unwrap_or(false)
    }

    /// Its part's: a document's tile is never larger than its font size either.
    fn largest_scale(&self, page: usize) -> Option<f32> {
        let (part, local) = self.0.locate(page)?;
        self.0.parts.get(part)?.source.largest_scale(local)
    }

    /// A list has those any of its files has: only those files change.
    fn settings(&self) -> Vec<&'static str> {
        let mut settings = Vec::new();
        for name in self.0.parts.iter().flat_map(|part| part.source.settings()) {
            if !settings.contains(&name) {
                settings.push(name);
            }
        }
        settings
    }

    fn settle(&self, page: usize) {
        if let Some((part, local)) = self.0.locate(page).and_then(|(part, local)| Some((self.0.parts.get(part)?, local))) {
            part.source.settle(local);
        }
    }

    fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
        Ok(Box::new(Renderer { joined: &self.0, current: None }))
    }

    fn pool(&self, threads: usize) -> Box<dyn Pool> {
        Box::new(Pools { joined: self.0.clone(), threads: threads.max(1), live: Mutex::new(Vec::new()) })
    }
}

/// Renders with the renderer of the part asked for, keeping only the last part's: pages printed
/// in order need each part's once.
struct Renderer<'a> {
    joined: &'a Parts,
    current: Option<(usize, Made<'a>)>,
}

/// A part's renderer, or why it has none.
type Made<'a> = Result<Box<dyn Render + 'a>, String>;

impl Render for Renderer<'_> {
    fn render(&mut self, request: Request) -> Frame {
        let joined = self.joined;
        let Some((part, local)) = joined.locate(request.page) else {
            return Frame::failed(request, format!("no page {}", request.page.saturating_add(1)));
        };
        if self.current.as_ref().is_none_or(|(at, _)| *at != part) {
            // The last part's renderer goes first, with what it holds.
            self.current = None;
            let made = joined.parts.get(part).ok_or_else(|| "no such file".to_string()).and_then(|p| p.source.renderer());
            self.current = Some((part, made));
        }
        let frame = match &mut self.current {
            Some((_, Ok(renderer))) => renderer.render(Request { page: local, ..request }),
            Some((_, Err(e))) => Frame::failed(request, e.clone()),
            None => Frame::failed(request, "no renderer".into()),
        };
        joined.joined(part, frame, request)
    }
}

/// The parts' own pools: those of the most urgent parts, as many parts at once as there are
/// threads, so hundreds of files never render at once; the others wait their turn.
struct Pools {
    joined: Rc<Parts>,
    threads: usize,
    /// The least recently wanted first.
    live: Mutex<Vec<Live>>,
}

/// A part's pool, and the joined requests it was given.
struct Live {
    part: usize,
    pool: Box<dyn Pool>,
    asked: Vec<Request>,
}

impl Pool for Pools {
    fn set_queue(&self, requests: Vec<Request>) {
        let mut live = lock(&self.live);
        // Each part's requests with its own page, the parts in the order of their most urgent.
        let mut wanted: Vec<(usize, Vec<(Request, usize)>)> = Vec::new();
        for request in requests {
            // A page past the end has no layout, so the viewer never asks for one.
            let Some((part, local)) = self.joined.locate(request.page) else { continue };
            let parts = wanted.len();
            match wanted.iter_mut().find(|(p, _)| *p == part) {
                Some((_, asked)) => asked.push((request, local)),
                None if parts < self.threads => wanted.push((part, vec![(request, local)])),
                None => {}
            }
        }
        // The parts not wanted now stop: a late result would be for a page no longer asked.
        for l in live.iter_mut().filter(|l| !l.asked.is_empty() && !wanted.iter().any(|(p, _)| *p == l.part)) {
            l.pool.set_queue(Vec::new());
            l.asked.clear();
        }
        for (part, asked) in wanted {
            let mut l = match live.iter().position(|l| l.part == part) {
                Some(at) => live.remove(at),
                None => match self.joined.parts.get(part) {
                    Some(p) => Live { part, pool: p.source.pool(self.threads), asked: Vec::new() },
                    None => continue,
                },
            };
            l.pool.set_queue(asked.iter().map(|(request, local)| Request { page: *local, ..*request }).collect());
            l.asked = asked.into_iter().map(|(request, _)| request).collect();
            live.push(l);
        }
        // Past KEPT, the pools wanted least recently go, never one at work.
        while live.len() > KEPT {
            let Some(at) = live.iter().position(|l| l.asked.is_empty()) else { break };
            live.remove(at);
        }
    }

    fn try_recv(&self) -> Option<Frame> {
        let live = lock(&self.live);
        for l in live.iter() {
            while let Some(frame) = l.pool.try_recv() {
                // Otherwise a late result, for a page the part was asked for before.
                if let Some(asked) = l.asked.iter().find(|r| r.tag == frame.request.tag) {
                    return Some(self.joined.joined(l.part, frame, *asked));
                }
            }
        }
        None
    }
}

/// The lock, even after a panic while it was held: the list is whole between statements.
fn lock(live: &Mutex<Vec<Live>>) -> MutexGuard<'_, Vec<Live>> {
    live.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::source::Pixels;

    /// What the fakes count: renderers and pools made, and pools alive.
    #[derive(Default)]
    struct Counts {
        renderers: AtomicUsize,
        pools: AtomicUsize,
        alive: AtomicUsize,
    }

    /// `pages` pages, page `n` `n + 1` points wide (natural ones for id 3), whose renders are one
    /// pixel of value `id`, and whose pools render whatever is queued at once.
    struct Fake {
        id: u8,
        pages: usize,
        counts: Arc<Counts>,
    }

    fn frame(id: u8, request: Request) -> Frame {
        Frame { request, width: 1, height: 1, rgba: Pixels::new(vec![id; 4]), error: None, warnings: Vec::new(), again: false }
    }

    impl Source for Fake {
        fn pages(&self) -> usize {
            self.pages
        }
        fn size(&self, page: usize) -> Option<(f32, f32)> {
            (page < self.pages).then_some((page as f32 + 1.0, 10.0))
        }
        fn natural(&self, _page: usize) -> bool {
            self.id == 3
        }
        fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
            self.counts.renderers.fetch_add(1, Ordering::Relaxed);
            struct One(u8);
            impl Render for One {
                fn render(&mut self, request: Request) -> Frame {
                    if request.page == 9 { Frame::failed(request, "broken".into()) } else { frame(self.0, request) }
                }
            }
            Ok(Box::new(One(self.id)))
        }
        fn pool(&self, _threads: usize) -> Box<dyn Pool> {
            self.counts.pools.fetch_add(1, Ordering::Relaxed);
            self.counts.alive.fetch_add(1, Ordering::Relaxed);
            Box::new(Instant { id: self.id, queue: Mutex::new(Vec::new()), counts: self.counts.clone() })
        }
    }

    /// A pool whose renders are done as soon as they are queued.
    struct Instant {
        id: u8,
        queue: Mutex<Vec<Request>>,
        counts: Arc<Counts>,
    }

    impl Pool for Instant {
        fn set_queue(&self, requests: Vec<Request>) {
            *self.queue.lock().unwrap() = requests;
        }
        fn try_recv(&self) -> Option<Frame> {
            let mut queue = self.queue.lock().unwrap();
            (!queue.is_empty()).then(|| frame(self.id, queue.remove(0)))
        }
    }

    impl Drop for Instant {
        fn drop(&mut self) {
            self.counts.alive.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Parts of `pages` pages each, all shown, named `f1`, `f2`…, the first with id 1.
    fn joined(pages: &[usize]) -> (Joined, Arc<Counts>) {
        let counts = Arc::new(Counts::default());
        let parts = (0..pages.len())
            .map(|i| Part {
                name: format!("f{}", i + 1),
                source: Rc::new(Fake { id: u8::try_from(i + 1).unwrap(), pages: pages[i], counts: counts.clone() }),
                pages: (0..pages[i]).collect(),
            })
            .collect();
        (Joined::new(parts), counts)
    }

    fn request(page: usize, tag: u64) -> Request {
        Request { page, scale: 1.0, tag, ..Default::default() }
    }

    #[test]
    fn pages_follow_on_from_part_to_part() {
        let (joined, _) = joined(&[2, 0, 3]);
        assert_eq!(joined.pages(), 5);
        // The empty part is left out: f3 is the second.
        assert_eq!(
            (0..6).map(|p| joined.0.locate(p)).collect::<Vec<_>>(),
            [Some((0, 0)), Some((0, 1)), Some((1, 0)), Some((1, 1)), Some((1, 2)), None]
        );
        assert_eq!((joined.size(1), joined.size(4), joined.size(5)), (Some((2.0, 10.0)), Some((3.0, 10.0)), None));
        assert!(!joined.natural(0) && joined.natural(2) && !joined.natural(9));
        // A part shows the pages it was given, in their order.
        let counts = Arc::new(Counts::default());
        let part = Part { name: "a".into(), source: Rc::new(Fake { id: 1, pages: 5, counts }), pages: vec![4, 0] };
        let picked = Joined::new(vec![part]);
        assert_eq!((picked.pages(), picked.0.locate(0), picked.0.locate(1)), (2, Some((0, 4)), Some((0, 0))));
    }

    #[test]
    fn a_renderer_renders_each_part_with_its_own_and_keeps_the_last() {
        let (joined, counts) = joined(&[2, 1, 10]);
        let mut renderer = joined.renderer().unwrap();
        let rendered: Vec<(usize, u8)> =
            [0, 1, 2, 3].map(|p| renderer.render(request(p, p as u64))).iter().map(|f| (f.request.page, f.rgba[0])).collect();
        assert_eq!(rendered, [(0, 1), (1, 1), (2, 2), (3, 3)]);
        assert_eq!(counts.renderers.load(Ordering::Relaxed), 3, "one renderer a part, in order");
        renderer.render(request(0, 9));
        assert_eq!(counts.renderers.load(Ordering::Relaxed), 4, "the first part's was let go");
        // f3's page 10 is joined page 12; its error names the file.
        let failed = renderer.render(request(12, 1));
        assert_eq!((failed.request.page, failed.error.as_deref()), (12, Some("f3: broken")));
        assert!(renderer.render(request(13, 1)).error.is_some_and(|e| e == "no page 14"));
    }

    /// Everything `pool` has ready: each result's page and pixel.
    fn ready(pool: &dyn Pool) -> Vec<(usize, u8)> {
        std::iter::from_fn(|| pool.try_recv()).map(|f| (f.request.page, f.rgba[0])).collect()
    }

    #[test]
    fn a_pool_renders_as_many_parts_at_once_as_it_has_threads() {
        let (joined, counts) = joined(&[1, 1, 1, 1, 1, 1, 1, 1]);
        let pool = joined.pool(2);
        // Pages 1–3 wanted, from three files: the two most urgent first.
        pool.set_queue(vec![request(2, 12), request(0, 10), request(1, 11)]);
        assert_eq!(ready(&*pool), [(2, 3), (0, 1)]);
        assert_eq!(counts.pools.load(Ordering::Relaxed), 2);
        // Delivered, they are no longer wanted: the third's turn.
        pool.set_queue(vec![request(1, 11)]);
        assert_eq!(ready(&*pool), [(1, 2)]);
        // Over the files one after another: never more than KEPT pools at once.
        for page in 3..8 {
            pool.set_queue(vec![request(page, page as u64)]);
            assert_eq!(ready(&*pool), [(page, u8::try_from(page + 1).unwrap())]);
            assert!(counts.alive.load(Ordering::Relaxed) <= KEPT);
        }
        assert_eq!(counts.alive.load(Ordering::Relaxed), KEPT);
        drop(pool);
        assert_eq!(counts.alive.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_part_no_longer_wanted_delivers_nothing() {
        let (joined, _) = joined(&[3, 3]);
        let pool = joined.pool(1);
        pool.set_queue(vec![request(0, 1), request(1, 2)]);
        // The reader moved to the second file before the first's renders were taken.
        pool.set_queue(vec![request(3, 3)]);
        assert_eq!(ready(&*pool), [(3, 2)]);
    }
}
