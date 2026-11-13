//! What a PDF's text costs pdfcraft to read, a page at a time, as clv's `pdf-text` worker reads
//! it: time, and heap as the allocator counts it, which is what the worker's cap counts. For
//! ADR-0027.
//!
//! `measure run <x.pdf>` reads every page's text and prints one line: the file, the parse, the
//! pages' times (total, median, 90th percentile, slowest), the glyphs and the text, and the most
//! heap one page took over what was held before it. With `--slow`, each page over 50 ms too.
//!
//! `measure gen off <strings> <x.pdf>` and `measure gen grid <strings> <x.pdf>` write the hostile
//! pages, synthetic: one page of `strings` strings of ten glyphs each, drawn one after another
//! off the page's right edge (pdfcraft drops them), or each placed by itself on the page (it keeps
//! them all).

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use pdfcraft_render::{PageRenderer, RenderConfig};

/// The system's allocator, counting what is held and the most that was.
struct Counted;

static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let now = NOW.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(now, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        NOW.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        if new >= layout.size() {
            let now = NOW.fetch_add(new - layout.size(), Ordering::Relaxed) + new - layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        } else {
            NOW.fetch_sub(layout.size() - new, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new) }
    }
}

#[global_allocator]
static ALLOC: Counted = Counted;

fn mib(bytes: usize) -> f64 {
    bytes as f64 / f64::from(1u32 << 20)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args[..] {
        ["run", path] => run(path, false),
        ["run", path, "--slow"] => run(path, true),
        ["gen", kind @ ("off" | "grid"), strings, path] => generate(kind, strings.parse().expect("how many strings"), path),
        _ => eprintln!("measure run <x.pdf> [--slow] | measure gen off|grid <strings> <x.pdf>"),
    }
}

fn run(path: &str, slow: bool) {
    let bytes = Arc::new(std::fs::read(path).expect("a PDF to read"));
    let file = bytes.len();
    let begun = Instant::now();
    // As clv's worker opens it.
    let mut document = PageRenderer::new(bytes, RenderConfig { reject_oversize: true, ..Default::default() });
    let parse = begun.elapsed();
    let parsed = NOW.load(Ordering::Relaxed);
    let pages = document.page_count();
    let mut times: Vec<Duration> = Vec::new();
    let (mut glyphs, mut most_glyphs, mut text, mut most_text, mut failed, mut most_heap) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let all = Instant::now();
    for page in 0..pages {
        let before = NOW.load(Ordering::Relaxed);
        PEAK.store(before, Ordering::Relaxed);
        let begun = Instant::now();
        match document.filtered_text(page, Default::default()) {
            Ok(read) => {
                let plain = read.plain_text();
                glyphs += read.glyphs.len();
                most_glyphs = most_glyphs.max(read.glyphs.len());
                text += plain.len();
                most_text = most_text.max(plain.len());
            }
            Err(_) => failed += 1,
        }
        let took = begun.elapsed();
        let heap = PEAK.load(Ordering::Relaxed).saturating_sub(before);
        if slow && took > Duration::from_millis(50) {
            println!("page {}: {took:?}, {:.1} MiB", page + 1, mib(heap));
        }
        times.push(took);
        most_heap = most_heap.max(heap);
    }
    let total = all.elapsed();
    times.sort();
    let at = |share: f64| times.get(((times.len() as f64 - 1.0) * share) as usize).copied().unwrap_or_default();
    println!(
        "file {:.1} MiB, parse {parse:?}, heap after parse {:.1} MiB (file included), {pages} pages, text {total:?} (median {:?}, p90 {:?}, max {:?}), {glyphs} glyphs (most on a page {most_glyphs}), {text} text bytes (most on a page {most_text}), {failed} failed, most heap a page took {:.1} MiB",
        mib(file),
        mib(parsed),
        at(0.5),
        at(0.9),
        at(1.0),
        mib(most_heap),
    );
}

/// A US Letter page of `strings` strings of ten glyphs in Helvetica: `off` the page, one after
/// another from near its left edge at 2 pt, or on a `grid` across it at 0.1 pt, 1,000 to a row.
fn generate(kind: &str, strings: usize, path: &str) {
    let mut content = Vec::new();
    if kind == "off" {
        content.extend_from_slice(b"BT /F1 2 Tf 1 0 0 1 10 700 Tm\n");
        for _ in 0..strings {
            content.extend_from_slice(b"(aaaaaaaaaa) Tj\n");
        }
    } else {
        content.extend_from_slice(b"BT /F1 0.1 Tf\n");
        for i in 0..strings {
            let (x, y) = (5.0 + (i % 1000) as f64 * 0.6, 780.0 - (i / 1000) as f64 * 0.12);
            writeln!(content, "1 0 0 1 {x:.2} {y:.2} Tm (abcdefghij) Tj").expect("a line");
        }
    }
    content.extend_from_slice(b"ET\n");
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(&content);
    stream.extend_from_slice(b"endstream");
    let objects: [&[u8]; 5] = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
        &stream,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    std::fs::write(path, out).expect("the PDF written");
}
