//! Measures what an SVG's HTML labels cost through Blitz, for Spec-0004 and ADR-0024.
//!
//! - `measure gen <kind> [args…] > x.svg` writes a synthetic drawing (`measure gen` lists them).
//! - `measure run <x.svg> [--each] [--scale S] [--out page.png]` finds its `<foreignObject>`s,
//!   lays them out in one Blitz document (or, with `--each`, one a label), records each label's
//!   paint into an anyrender `Scene` of its own at scale 1, and replays each through its
//!   transform into the part of the page it lands on, the page being the drawing at `S` pixels a
//!   user unit. It prints what each stage took and counted. Under `/usr/bin/time -l`, its memory.
//! - `measure stack <depth> [kind]` builds, lays out, paints, replays and drops a label nested
//!   `depth` deep (`div`, `span`, or divs of `flex`, `grid`, `table`, `inline-block`) on a thread
//!   with the stack a spawned thread gets (clv's render threads: 2 MiB), or `STACK` MiB; `measure
//!   stack-search [kind]` runs that in child processes, deeper and deeper, to the depth that holds.
//!
//! `COUNTS=1` prints each label's commands, `DUMP=<i>` label i's; `--no-replay` stops before
//! the replay. A tool, not clv: it unwraps, its input being its own.
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyrender::{PaintScene, Scene, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use base64::Engine;
use blitz_dom::{BaseDocument, DocumentConfig, FontContext};
use blitz_html::HtmlDocument;
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use kurbo::{Affine, Rect};

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";
/// Room around an `overflow: visible` label for what it draws past its box, in CSS pixels a side.
const OVERFLOW: f32 = 4_096.0;
/// The widest row of the one document's labels, in CSS pixels.
const ROW: f32 = 1_048_576.0;
/// Between the one document's labels, so painting one culls its neighbours.
const GAP: f32 = 8.0;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("gen") => {
            let svg = generate(&args[1..]);
            std::io::stdout().write_all(svg.as_bytes()).unwrap();
        }
        Some("run") => run(&args[1..]),
        Some("stack") => {
            let depth: usize = args[1].parse().unwrap();
            let tag = args.get(2).cloned().unwrap_or_else(|| "div".into());
            // A spawned thread's stack, 2 MiB unless `STACK` (MiB) says otherwise.
            let mut builder = std::thread::Builder::new();
            if let Ok(mib) = std::env::var("STACK") {
                builder = builder.stack_size(mib.parse::<usize>().unwrap() << 20);
            }
            let joined = builder.spawn(move || stack(depth, &tag)).unwrap().join();
            println!("{}", if joined.is_ok() { "held" } else { "panicked" });
        }
        Some("stack-search") => stack_search(args.get(1).map(String::as_str).unwrap_or("div")),
        _ => eprintln!(
            "usage: measure gen <kind> … | run <x.svg> [--each] [--scale S] [--out p.png] | stack <depth> [div|span] | stack-search [div|span]"
        ),
    }
}

// ---------------------------------------------------------------------------------------------
// Finding the labels: as Spec-0004 asks, simplified (no <switch>, no nested <svg>).

struct Label {
    /// The ancestors' copies, opened and closed, `display: contents`.
    open: String,
    close: String,
    /// The `<foreignObject>`'s own id, class and style.
    attrs: String,
    inner: String,
    w: f32,
    h: f32,
    visible: bool,
    /// From the label's box (its user units, origin at its x, y) to the drawing's size at scale 1.
    place: Affine,
}

impl Label {
    fn margin(&self) -> f32 {
        if self.visible { OVERFLOW } else { 0.0 }
    }
    fn slot(&self) -> (f32, f32) {
        (self.w + 2.0 * self.margin(), self.h + 2.0 * self.margin())
    }
    /// How many elements deep its copy goes, from its slot to the `<foreignObject>`'s.
    fn depth(&self) -> usize {
        self.open.matches('<').count() + 1
    }
}

struct Drawing {
    css: String,
    labels: Vec<Label>,
    size: (f32, f32),
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn length(n: roxmltree::Node, a: &str, of: f32) -> Option<f32> {
    let l = n.attribute(a)?.parse::<svgtypes::Length>().ok()?;
    Some(if l.unit == svgtypes::LengthUnit::Percent { l.number as f32 * of / 100.0 } else { l.number as f32 })
}

fn transform(s: &str) -> Affine {
    s.parse::<svgtypes::Transform>().map(|t| Affine::new([t.a, t.b, t.c, t.d, t.e, t.f])).unwrap_or(Affine::IDENTITY)
}

/// The tree written out as XHTML: elements, attributes and text escaped, namespaces declared.
fn written(n: roxmltree::Node, out: &mut String) {
    if n.is_text() {
        out.push_str(&esc(n.text().unwrap_or("")));
        return;
    }
    if !n.is_element() {
        return;
    }
    let name = n.tag_name().name();
    let _ = write!(out, "<{name}");
    if n.tag_name().namespace() == Some(XHTML_NS) && n.parent_element().is_none_or(|p| p.tag_name().namespace() != Some(XHTML_NS)) {
        let _ = write!(out, " xmlns=\"{XHTML_NS}\"");
    }
    for a in n.attributes() {
        if a.namespace().is_none() {
            let _ = write!(out, " {}=\"{}\"", a.name(), esc(a.value()));
        }
    }
    out.push('>');
    for c in n.children() {
        written(c, out);
    }
    let _ = write!(out, "</{name}>");
}

fn drawing(text: &str) -> Drawing {
    let options = roxmltree::ParsingOptions { allow_dtd: true, nodes_limit: 4_000_000, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(text, options).unwrap();
    let root = doc.root_element();
    let vb = root.attribute("viewBox").and_then(|v| v.parse::<svgtypes::ViewBox>().ok());
    let w = length(root, "width", 0.0).filter(|v| *v > 0.0).or(vb.map(|v| v.w as f32)).unwrap_or(100.0);
    let h = length(root, "height", 0.0).filter(|v| *v > 0.0).or(vb.map(|v| v.h as f32)).unwrap_or(100.0);
    let (vw, vh) = vb.map(|v| (v.w as f32, v.h as f32)).unwrap_or((w, h));
    let view = match vb {
        Some(v) if v.w > 0.0 && v.h > 0.0 => {
            let s = (f64::from(w) / v.w).min(f64::from(h) / v.h);
            Affine::new([s, 0.0, 0.0, s, -v.x * s + (f64::from(w) - v.w * s) / 2.0, -v.y * s + (f64::from(h) - v.h * s) / 2.0])
        }
        _ => Affine::IDENTITY,
    };
    let css: String = doc.descendants().filter(|n| n.has_tag_name((SVG_NS, "style"))).filter_map(|n| n.text()).collect::<Vec<_>>().join("\n");
    let mut labels = Vec::new();
    for fo in doc.descendants().filter(|n| n.has_tag_name((SVG_NS, "foreignObject"))) {
        let (x, y) = (length(fo, "x", vw).unwrap_or(0.0), length(fo, "y", vh).unwrap_or(0.0));
        let (fw, fh) = (length(fo, "width", vw).unwrap_or(0.0), length(fo, "height", vh).unwrap_or(0.0));
        if !(fw > 0.0 && fh > 0.0) {
            continue;
        }
        let mut chain: Vec<roxmltree::Node> = fo.ancestors().skip(1).filter(|a| a.is_element()).collect();
        chain.reverse();
        let mut place = view;
        for a in chain.iter().skip(1).chain(std::iter::once(&fo)) {
            if let Some(t) = a.attribute("transform") {
                place *= transform(t);
            }
        }
        place *= Affine::translate((f64::from(x), f64::from(y)));
        let copy = |a: &roxmltree::Node| {
            let mut s = String::new();
            for name in ["id", "class"] {
                if let Some(v) = a.attribute(name) {
                    let _ = write!(s, " {name}=\"{}\"", esc(v));
                }
            }
            s
        };
        let name = |a: &roxmltree::Node| if a.has_tag_name((SVG_NS, "svg")) { "div".to_string() } else { a.tag_name().name().to_string() };
        let mut open = String::new();
        for a in &chain {
            let _ = write!(open, "<{}{} style=\"{};display:contents !important\">", name(a), copy(a), esc(a.attribute("style").unwrap_or("")));
        }
        let close: String = chain.iter().rev().map(|a| format!("</{}>", name(a))).collect();
        let style = fo.attribute("style").unwrap_or("");
        let visible = fo.attribute("overflow") == Some("visible") || style.replace(' ', "").contains("overflow:visible");
        let attrs = format!("{} style=\"{}\"", copy(&fo), esc(style));
        let mut inner = String::new();
        for c in fo.children() {
            written(c, &mut inner);
        }
        labels.push(Label { open, close, attrs, inner, w: fw, h: fh, visible, place });
    }
    Drawing { css, labels, size: (w, h) }
}

// ---------------------------------------------------------------------------------------------
// The documents.

/// An XHTML document holding `slots`: each label in a box of its own at (x, y), none overlapping.
fn document(css: &str, slots: &[(&Label, f32, f32)]) -> String {
    let mut out = format!(
        "<html xmlns=\"{XHTML_NS}\"><head><style><![CDATA[{}]]></style><style>html,body{{margin:0;padding:0;background:transparent;overflow:hidden}}</style></head><body>",
        css.replace("]]>", "]]]]><![CDATA[>")
    );
    for (label, x, y) in slots {
        let (sw, sh) = label.slot();
        let m = label.margin();
        let ov = if label.visible { "visible" } else { "hidden" };
        let (w, h) = (label.w, label.h);
        // The slot, then the ancestors' copies, then the `<foreignObject>`'s (its id, class and
        // style, `display: contents`), then its box, the containing block of what it holds.
        let _ = write!(
            out,
            "<div style=\"position:absolute;left:0;top:0;transform:translate({x}px,{y}px);width:{sw}px;height:{sh}px;overflow:hidden;margin:0;padding:0;border:0\">{}\
             <foreignObject{}>{}</foreignObject>{}</div>",
            label.open,
            label.attrs.replacen(
                " style=\"",
                &format!(
                    " style=\"display:block !important;position:absolute !important;left:{m}px !important;top:{m}px !important;\
                  width:{w}px !important;height:{h}px !important;margin:0 !important;overflow:{ov} !important;box-sizing:border-box !important;"
                ),
                1
            ),
            label.inner,
            label.close
        );
    }
    out.push_str("</body></html>");
    out
}

struct Counts {
    requests: AtomicU64,
    bytes: AtomicU64,
    fonts: AtomicU64,
    font_declared: AtomicU64,
    pictures: AtomicU64,
    picture_pixels: AtomicU64,
    other: AtomicU64,
    log: Mutex<Vec<String>>,
}

static COUNTS: Counts = Counts {
    requests: AtomicU64::new(0),
    bytes: AtomicU64::new(0),
    fonts: AtomicU64::new(0),
    font_declared: AtomicU64::new(0),
    pictures: AtomicU64::new(0),
    picture_pixels: AtomicU64::new(0),
    other: AtomicU64::new(0),
    log: Mutex::new(Vec::new()),
};

/// What a font's header says it unpacks to: a WOFF's `totalSfntSize` and its tables' sizes.
fn font_declared(b: &[u8]) -> Option<u64> {
    let u32_at = |i: usize| b.get(i..i + 4).map(|s| u64::from(u32::from_be_bytes([s[0], s[1], s[2], s[3]])));
    match b.get(0..4)? {
        b"wOFF" => {
            let tables = u32_at(12)? >> 16;
            let mut sum = 0u64;
            for t in 0..tables {
                sum += u32_at(44 + 20 * t as usize + 12)?;
            }
            Some(u32_at(16)?.max(sum))
        }
        b"wOF2" => u32_at(16),
        b"OTTO" | b"true" | [0, 1, 0, 0] => Some(b.len() as u64),
        _ => None,
    }
}

/// Fetches nothing but `data:` URLs, and counts them: no network, no files.
struct DataOnly;
impl NetProvider for DataOnly {
    fn fetch(&self, _doc: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.as_str().to_owned();
        let bytes = data_url(&url).unwrap_or_default();
        COUNTS.requests.fetch_add(1, Ordering::Relaxed);
        COUNTS.bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        if let Some(declared) = font_declared(&bytes) {
            COUNTS.fonts.fetch_add(1, Ordering::Relaxed);
            COUNTS.font_declared.fetch_add(declared, Ordering::Relaxed);
        } else if let Ok(size) = imagesize::blob_size(&bytes) {
            COUNTS.pictures.fetch_add(1, Ordering::Relaxed);
            COUNTS.picture_pixels.fetch_add(size.width as u64 * size.height as u64, Ordering::Relaxed);
        } else {
            COUNTS.other.fetch_add(1, Ordering::Relaxed);
            COUNTS.log.lock().unwrap().push(url.chars().take(60).collect());
        }
        handler.bytes(url, Bytes::from(bytes));
    }
}

fn data_url(url: &str) -> Option<Vec<u8>> {
    let (meta, data) = url.strip_prefix("data:")?.split_once(',')?;
    if meta.ends_with(";base64") { base64::engine::general_purpose::STANDARD.decode(data.trim()).ok() } else { Some(data.as_bytes().to_vec()) }
}

fn config(width: u32, height: u32, fonts: &FontContext) -> DocumentConfig {
    DocumentConfig {
        viewport: Some(Viewport::new(width.max(1), height.max(1), 1.0, ColorScheme::Light)),
        net_provider: Some(Arc::new(DataOnly)),
        font_ctx: Some(fonts.clone()),
        ..Default::default()
    }
}

/// The `<foreignObject>` copy of each slot of `doc`, in order: body's children, then down the
/// first element child `depth` times.
fn boxes(doc: &BaseDocument, depths: &[usize]) -> Vec<(Rect, Rect)> {
    let node = |id| doc.get_node(id).unwrap();
    let is = |id: &blitz_dom::NodeId| node(*id).element_data().is_some();
    let html = doc.root_element();
    let body = html.children.iter().copied().find(|c| node(*c).element_data().is_some_and(|e| &*e.name.local == "body")).unwrap();
    let slots: Vec<blitz_dom::NodeId> = node(body).children.iter().copied().filter(is).collect();
    slots
        .iter()
        .zip(depths)
        .map(|(slot, depth)| {
            let mut at = *slot;
            for _ in 0..*depth {
                match node(at).children.iter().copied().find(is) {
                    Some(c) => at = c,
                    None => break,
                }
            }
            let n = node(at);
            let l = n.final_layout();
            let abs = n.absolute_position(0.0, 0.0);
            let border = Rect::new(0.0, 0.0, f64::from(l.size.width), f64::from(l.size.height));
            let overflow = n.scrollable_overflow().union(border);
            let s = node(*slot).final_layout();
            let into_slot = kurbo::Vec2::new(f64::from(abs.x - s.location.x), f64::from(abs.y - s.location.y));
            (Rect::from_origin_size((f64::from(abs.x), f64::from(abs.y)), border.size()), overflow + into_slot)
        })
        .collect()
}

fn run(args: &[String]) {
    let path = &args[0];
    let each = args.iter().any(|a| a == "--each");
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let scale: f64 = flag("--scale").map(|s| s.parse().unwrap()).unwrap_or(1.0);
    let out = flag("--out");

    let t = Instant::now();
    let text = std::fs::read_to_string(path).unwrap();
    let d = drawing(&text);
    let found = t.elapsed();
    let t = Instant::now();
    let fonts = FontContext::default();
    // Finding the system's fonts happens on the first use, once a process.
    let _ = fonts.clone();
    let font_ms = t.elapsed();

    let (mut built, mut laid, mut recorded) = (std::time::Duration::ZERO, std::time::Duration::ZERO, std::time::Duration::ZERO);
    let mut scenes: Vec<(Scene, Rect)> = Vec::with_capacity(d.labels.len());
    let (mut markup, mut documents) = (0usize, 0usize);
    let mut largest = (0.0f64, 0.0f64);
    if each {
        for label in &d.labels {
            let xhtml = document(&d.css, &[(label, 0.0, 0.0)]);
            markup += xhtml.len();
            documents += 1;
            let (sw, sh) = label.slot();
            let t = Instant::now();
            let mut doc = HtmlDocument::from_xml(&xhtml, config(sw.ceil() as u32, sh.ceil() as u32, &fonts));
            doc.handle_messages();
            built += t.elapsed();
            let t = Instant::now();
            doc.resolve(0.0);
            laid += t.elapsed();
            let t = Instant::now();
            let (border, region) = boxes(&doc, &[label.depth()])[0];
            largest = (largest.0.max(border.width()), largest.1.max(border.height()));
            let mut scene = Scene::new();
            blitz_paint::paint_scene(&mut scene, &mut doc, 1.0, sw.ceil() as u32, sh.ceil() as u32, 0, 0);
            recorded += t.elapsed();
            scenes.push((scene, region));
        }
    } else {
        // Shelves: left to right, a row about as wide as all of them would be square, at most
        // ROW, so the document's coordinates stay small.
        let area: f64 = d
            .labels
            .iter()
            .map(|l| {
                let (w, h) = l.slot();
                f64::from(w + GAP) * f64::from(h + GAP)
            })
            .sum();
        let widest = d.labels.iter().map(|l| l.slot().0 + GAP).fold(0.0f32, f32::max);
        let row = (area.sqrt() as f32).clamp(widest, ROW.max(widest));
        let (mut x, mut y, mut row_h, mut width) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let mut slots = Vec::with_capacity(d.labels.len());
        for label in &d.labels {
            let (sw, sh) = label.slot();
            if x > 0.0 && x + sw > row {
                (x, y, row_h) = (0.0, y + row_h, 0.0);
            }
            slots.push((label, x, y));
            x += sw.ceil() + GAP;
            width = width.max(x);
            row_h = row_h.max(sh.ceil() + GAP);
        }
        let height = y + row_h;
        eprintln!("the one document: {width} × {height} CSS pixels");
        let xhtml = document(&d.css, &slots);
        markup = xhtml.len();
        documents = 1;
        let t = Instant::now();
        let mut doc = HtmlDocument::from_xml(&xhtml, config(width.ceil() as u32, height.ceil() as u32, &fonts));
        doc.handle_messages();
        built = t.elapsed();
        let t = Instant::now();
        doc.resolve(0.0);
        laid = t.elapsed();
        let t = Instant::now();
        let depths: Vec<usize> = d.labels.iter().map(Label::depth).collect();
        let found = boxes(&doc, &depths);
        for ((label, x, y), (border, region)) in slots.iter().zip(found) {
            largest = (largest.0.max(border.width()), largest.1.max(border.height()));
            let (sw, sh) = label.slot();
            doc.set_viewport_scroll(blitz_dom::Point { x: f64::from(*x), y: f64::from(*y) });
            let mut scene = Scene::new();
            blitz_paint::paint_scene(&mut scene, &mut doc, 1.0, sw.ceil() as u32, sh.ceil() as u32, 0, 0);
            scenes.push((scene, region));
        }
        recorded = t.elapsed();
        let t = Instant::now();
        drop(doc);
        eprintln!("dropped the document in {:?}", t.elapsed());
    }
    if std::env::var("COUNTS").is_ok() {
        eprintln!("{:?}", scenes.iter().map(|(s, _)| s.commands.len()).collect::<Vec<_>>());
    }
    if std::env::var("DUMP").is_ok() {
        for c in &scenes[std::env::var("DUMP").unwrap().parse::<usize>().unwrap()].0.commands {
            use anyrender::recording::RenderCommand as C;
            match c {
                C::PushLayer(l) => eprintln!("push layer {:?}", kurbo::Shape::bounding_box(&l.clip)),
                C::PushClipLayer(l) => eprintln!("push clip {:?}", kurbo::Shape::bounding_box(&l.clip)),
                C::PopLayer => eprintln!("pop"),
                C::Fill(f) => eprintln!("fill {:?} {:?}", kurbo::Shape::bounding_box(&f.shape), f.transform.translation()),
                C::Stroke(_) => eprintln!("stroke"),
                C::GlyphRun(g) => eprintln!("glyphs {}", g.glyphs.len()),
                C::BoxShadow(_) => eprintln!("shadow"),
            }
        }
    }
    let commands: usize = scenes.iter().map(|(s, _)| s.commands.len()).sum();
    let glyphs: usize = scenes
        .iter()
        .flat_map(|(s, _)| &s.commands)
        .map(|c| if let anyrender::recording::RenderCommand::GlyphRun(g) = c { g.glyphs.len() } else { 0 })
        .sum();

    // The replay: each label through its transform into the part of the page it lands on.
    let t = Instant::now();
    let scale = if args.iter().any(|a| a == "--no-replay") { 0.0 } else { scale };
    let (pw, ph) = ((f64::from(d.size.0) * scale).ceil() as u32, (f64::from(d.size.1) * scale).ceil() as u32);
    let mut page = vec![0u8; pw as usize * ph as usize * 4];
    let mut painted = 0u64;
    for (label, (scene, region)) in d.labels.iter().zip(&scenes) {
        let m = f64::from(label.margin());
        let to = Affine::scale(scale) * label.place * Affine::translate((-m, -m));
        let bbox = to.transform_rect_bbox(*region);
        let (x0, y0) = (bbox.x0.floor().max(0.0), bbox.y0.floor().max(0.0));
        let (x1, y1) = (bbox.x1.ceil().min(f64::from(pw)), bbox.y1.ceil().min(f64::from(ph)));
        if !(x1 > x0 && y1 > y0) {
            continue;
        }
        let (ox, oy, ow, oh) = (x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32);
        painted += u64::from(ow) * u64::from(oh);
        let shifted = Affine::translate((-x0, -y0)) * to;
        let rgba = render_to_buffer::<VelloCpuImageRenderer, _>(|s| s.append_scene(scene.clone(), shifted), ow, oh);
        for row in 0..oh {
            for col in 0..ow {
                let src = ((row * ow + col) * 4) as usize;
                let dst = (((oy + row) * pw + ox + col) * 4) as usize;
                let a = u32::from(rgba[src + 3]);
                for k in 0..4 {
                    page[dst + k] = (u32::from(rgba[src + k]) + u32::from(page[dst + k]) * (255 - a) / 255) as u8;
                }
            }
        }
    }
    let replayed = t.elapsed();
    let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
    println!(
        "{path}: {} labels, {} {}; markup {:.1} MiB, css {:.1} KiB",
        d.labels.len(),
        documents,
        if each { "documents (one a label)" } else { "document" },
        markup as f64 / 1048576.0,
        d.css.len() as f64 / 1024.0
    );
    println!(
        "  found {:.0} ms, fonts {:.0} ms, built {:.0} ms, laid out {:.0} ms, recorded {:.0} ms, replayed {:.0} ms",
        ms(found),
        ms(font_ms),
        ms(built),
        ms(laid),
        ms(recorded),
        ms(replayed)
    );
    println!(
        "  {commands} commands ({glyphs} glyphs); page {pw} × {ph}, {painted} pixels painted; largest label {:.0} × {:.0}",
        largest.0, largest.1
    );
    println!(
        "  data: {} requests, {:.1} MiB; fonts {} declaring {:.1} MiB; pictures {} of {:.1} Mpx; other {}",
        COUNTS.requests.load(Ordering::Relaxed),
        COUNTS.bytes.load(Ordering::Relaxed) as f64 / 1048576.0,
        COUNTS.fonts.load(Ordering::Relaxed),
        COUNTS.font_declared.load(Ordering::Relaxed) as f64 / 1048576.0,
        COUNTS.pictures.load(Ordering::Relaxed),
        COUNTS.picture_pixels.load(Ordering::Relaxed) as f64 / 1e6,
        COUNTS.other.load(Ordering::Relaxed)
    );
    if let Some(out) = out {
        let file = std::fs::File::create(out).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), pw, ph);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // Premultiplied to straight, over white so it can be looked at.
        let over: Vec<u8> = page
            .chunks(4)
            .flat_map(|p| {
                let a = 255 - u32::from(p[3]);
                [p[0], p[1], p[2]].map(|c| (u32::from(c) + a).min(255) as u8).into_iter().chain([255])
            })
            .collect();
        encoder.write_header().unwrap().write_image_data(&over).unwrap();
    }
}

// ---------------------------------------------------------------------------------------------
// The stack.

fn stack(depth: usize, tag: &str) {
    let mut inner = String::new();
    // `flex`, `grid`, `table`, `inline-block`: divs of that display; else elements of that name.
    let (name, display) = match tag {
        "flex" | "grid" | "table" | "inline-block" => ("div", tag),
        other => (other, ""),
    };
    for _ in 0..depth {
        let _ =
            write!(inner, "<{name} style=\"padding-left:1px;{}\">", if display.is_empty() { String::new() } else { format!("display:{display}") });
    }
    inner.push('x');
    for _ in 0..depth {
        let _ = write!(inner, "</{name}>");
    }
    let label =
        Label { open: String::new(), close: String::new(), attrs: String::new(), inner, w: 400.0, h: 100.0, visible: false, place: Affine::IDENTITY };
    let xhtml = document("", &[(&label, 0.0, 0.0)]);
    let fonts = FontContext::default();
    let mut doc = HtmlDocument::from_xml(&xhtml, config(400, 100, &fonts));
    doc.handle_messages();
    doc.resolve(0.0);
    let mut scene = Scene::new();
    blitz_paint::paint_scene(&mut scene, &mut doc, 1.0, 400, 100, 0, 0);
    let _ = render_to_buffer::<VelloCpuImageRenderer, _>(|s| s.append_scene(scene, Affine::scale(2.0)), 800, 200);
    drop(doc);
}

fn stack_search(tag: &str) {
    let me = std::env::current_exe().unwrap();
    let holds = |depth: usize| {
        let out = std::process::Command::new(&me).args(["stack", &depth.to_string(), tag]).output().unwrap();
        let held = out.status.success() && String::from_utf8_lossy(&out.stdout).contains("held");
        println!("  {tag} × {depth}: {}", if held { "held".to_string() } else { format!("{}", out.status) });
        held
    };
    let (mut good, mut bad) = (0usize, 0usize);
    let mut depth = 64;
    while depth <= 1 << 20 {
        if holds(depth) {
            good = depth;
            depth *= 2;
        } else {
            bad = depth;
            break;
        }
    }
    if bad == 0 {
        println!("{tag}: held to {good}");
        return;
    }
    while bad - good > good / 32 + 1 {
        let mid = (good + bad) / 2;
        if holds(mid) { good = mid } else { bad = mid }
    }
    println!("{tag}: held at {good}, failed at {bad}");
}

// ---------------------------------------------------------------------------------------------
// The drawings, synthetic.

fn svg(w: u32, h: u32, css: &str, body: &str) -> String {
    format!("<svg xmlns=\"{SVG_NS}\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\"><style><![CDATA[{css}]]></style>{body}</svg>")
}

fn label(x: u32, y: u32, w: u32, h: u32, html: &str) -> String {
    format!("<foreignObject x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\"><div xmlns=\"{XHTML_NS}\">{html}</div></foreignObject>")
}

/// `n` labels of `w` × `h` in a grid `columns` wide, each `html(i)`.
fn grid(n: u32, columns: u32, w: u32, h: u32, html: impl Fn(u32) -> String) -> (u32, u32, String) {
    let mut body = String::new();
    for i in 0..n {
        body.push_str(&label((i % columns) * (w + 10), (i / columns) * (h + 10), w, h, &html(i)));
    }
    (columns.min(n) * (w + 10), n.div_ceil(columns) * (h + 10), body)
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn png_bytes(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    let data: Vec<u8> = rgba.iter().copied().cycle().take(w as usize * h as usize * 4).collect();
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&data).unwrap();
    writer.finish().unwrap();
    out
}

const BOX: &str = ".box{background:#3a7;border:2px solid #135;width:100%;height:100%;box-sizing:border-box}";

fn generate(args: &[String]) -> String {
    let n = |i: usize, default: u64| args.get(i).map(|s| s.parse::<u64>().unwrap()).unwrap_or(default);
    match args.first().map(String::as_str) {
        // N labels under K KiB of stylesheet rules, matching none of them.
        Some("styles") => {
            let (count, kib) = (n(1, 1024) as u32, n(2, 3072));
            let mut css = String::from(BOX);
            let mut i = 0;
            while (css.len() as u64) < kib * 1024 {
                let _ = write!(css, "\n.r{i} .s{i} > .t{i}{{color:#123456;background:#abcdef;border:1px solid #fedcba}}");
                i += 1;
            }
            let (w, h, body) = grid(count, 32, 120, 40, |i| format!("<div class=\"box\">Label {i}</div>"));
            svg(w, h, &css, &body)
        }
        // N labels sharing one @font-face of K KB, a synthetic TrueType font.
        Some("font") => {
            let (count, kb) = (n(1, 50) as u32, n(2, 400));
            let font = ttf(kb as usize * 1000);
            let css =
                format!("@font-face{{font-family:Synth;src:url(data:font/ttf;base64,{})}}\n.t{{font-family:Synth;font-size:20px}}{BOX}", b64(&font));
            let (w, h, body) = grid(count, 10, 160, 40, |i| format!("<div class=\"box t\">Label {i}</div>"));
            svg(w, h, &css, &body)
        }
        // One label with B bytes of text.
        Some("text") => {
            let bytes = n(1, 1 << 20) as usize;
            let words: String = "lorem ipsum dolor sit amet ".chars().cycle().take(bytes).collect();
            svg(800, 600, "", &label(0, 0, 800, 600, &words))
        }
        // R rules in the universal bucket, over E elements in one label: R × E rule tries.
        Some("rules") => {
            let (rules, elements) = (n(1, 10_000), n(2, 5_000));
            let mut css = String::new();
            for i in 0..rules {
                let _ = write!(css, ":not(.c{i}){{--v{}:1}}", i % 7);
            }
            // Each its own class, so Stylo can't share one's style with the next.
            let spans: String = (0..elements).map(|i| format!("<span class=\"u{i}\">x</span>")).collect();
            svg(800, 600, &css, &label(0, 0, 800, 600, &spans))
        }
        // One label nested D elements deep.
        Some("deep") => {
            let depth = n(1, 64) as usize;
            let inner = format!("{}x{}", "<div style=\"padding-left:1px\">".repeat(depth), "</div>".repeat(depth));
            svg(800, 200, "", &label(0, 0, 800, 200, &inner))
        }
        // A WOFF font whose one table unpacks to G MiB of zeros (its header says so).
        Some("woff-bomb") => {
            let mib = n(1, 1024);
            let font = woff_bomb(mib << 20);
            let css = format!("@font-face{{font-family:Bomb;src:url(data:font/woff;base64,{})}}\n.t{{font-family:Bomb}}", b64(&font));
            svg(400, 100, &css, &label(0, 0, 400, 100, "<div class=\"t\">bomb</div>"))
        }
        // N labels, each a different S × S PNG.
        Some("pictures") => {
            let (count, side) = (n(1, 64) as u32, n(2, 4096) as u32);
            let (w, h, body) = grid(count, 8, 100, 100, |i| {
                let png = png_bytes(side, side, [i as u8, (i >> 8) as u8, 200, 255]);
                format!("<img style=\"width:100px;height:100px\" src=\"data:image/png;base64,{}\"/>", b64(&png))
            });
            svg(w, h, "", &body)
        }
        // One S × S label (CSS pixels) with a 1-pixel PNG tiled over it.
        Some("tile") => {
            let side = n(1, 16_384) as u32;
            let css = format!(
                ".tile{{width:100%;height:100%;background-image:url(data:image/png;base64,{});background-size:1px 1px}}",
                b64(&png_bytes(1, 1, [200, 30, 30, 255]))
            );
            svg(side, side, &css, &label(0, 0, side, side, "<div class=\"tile\"></div>"))
        }
        // One label of N small boxes, each a background and a border.
        Some("commands") => {
            let count = n(1, 100_000) as usize;
            let side = n(2, 2000) as u32;
            let css = ".c{display:inline-block;width:4px;height:4px;background:#48c;border:1px solid #024;border-radius:1px}";
            svg(side, side, css, &label(0, 0, side, side, &"<i class=\"c\"></i>".repeat(count)))
        }
        // One label of N words, each on a line of its own: a glyph run each.
        Some("lines") => {
            let count = n(1, 100_000) as usize;
            let side = n(2, 16_384) as u32;
            let css = ".l{width:1px;font-size:2px;line-height:2px;word-break:normal}";
            svg(side, side, css, &label(0, 0, side, side, &format!("<div class=\"l\">{}</div>", "ab ".repeat(count))))
        }
        // N labels of `overflow: visible`, each with a box three boxes to its right.
        Some("visible") => {
            let count = n(1, 1024) as u32;
            let mut body = String::new();
            for i in 0..count {
                let (x, y) = ((i % 8) * 200, (i / 8) * 30);
                body.push_str(&format!(
                    "<foreignObject x=\"{x}\" y=\"{y}\" width=\"40\" height=\"20\" style=\"overflow:visible\"><div xmlns=\"{XHTML_NS}\" class=\"box\" style=\"position:relative\">{i}<b style=\"position:absolute;left:120px;top:0;width:40px;height:20px;background:#c33\"></b></div></foreignObject>"
                ));
            }
            svg(8 * 200, count.div_ceil(8) * 30, BOX, &body)
        }
        // One S × S label of N boxes stacked, each over the whole label.
        Some("stacked") => {
            let count = n(1, 100) as usize;
            let side = n(2, 4096) as u32;
            let css = ".f{position:absolute;left:0;top:0;width:100%;height:100%;background:rgba(40,80,200,0.02)}";
            svg(side, side, css, &label(0, 0, side, side, &"<i class=\"f\"></i>".repeat(count)))
        }
        // One label of N boxes, each with a blurred shadow.
        Some("shadows") => {
            let count = n(1, 10_000) as usize;
            let side = n(2, 2000) as u32;
            let css = ".c{display:inline-block;width:16px;height:16px;margin:4px;background:#48c;box-shadow:2px 2px 6px #000}";
            svg(side, side, css, &label(0, 0, side, side, &"<i class=\"c\"></i>".repeat(count)))
        }
        // N plain labels.
        Some("labels") => {
            let count = n(1, 1024) as u32;
            let (w, h, body) = grid(count, 32, 120, 40, |i| format!("<div class=\"box\">Label {i}</div>"));
            svg(w, h, BOX, &body)
        }
        _ => {
            eprintln!(
                "kinds: styles N KiB | font N KB | text BYTES | rules R E | deep D | woff-bomb MiB | pictures N SIDE | tile SIDE | \
                 commands N SIDE | lines N SIDE | stacked N SIDE | shadows N SIDE | visible N | labels N"
            );
            std::process::exit(2);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// A TrueType font, made here: glyphs that are boxes, padded with more of them to a size.

fn ttf(size: usize) -> Vec<u8> {
    // Glyph 0 .notdef, 1..=95 printable ASCII, then fillers to reach `size`.
    let glyph = |w: i16, h: i16, extra: usize| {
        let points = 4 + extra;
        let mut g = Vec::new();
        g.extend(1i16.to_be_bytes()); // one contour
        g.extend([0i16, 0, w, h].iter().flat_map(|v| v.to_be_bytes()));
        g.extend(((points - 1) as u16).to_be_bytes());
        g.extend(0u16.to_be_bytes()); // no instructions
        // Flags: on curve, x and y as shorts.
        g.extend(std::iter::repeat_n(0x01u8, points));
        let mut xs = vec![0i16, w, 0, -w];
        let mut ys = vec![0i16, 0, h, 0];
        xs.extend(std::iter::repeat_n(0, extra));
        ys.extend(std::iter::repeat_n(0, extra));
        g.extend(xs.iter().flat_map(|v| v.to_be_bytes()));
        g.extend(ys.iter().flat_map(|v| v.to_be_bytes()));
        while g.len() % 4 != 0 {
            g.push(0);
        }
        g
    };
    let mut glyphs: Vec<Vec<u8>> = vec![glyph(500, 700, 0)];
    for c in 0x20u16..=0x7e {
        glyphs.push(if c == 0x20 { Vec::new() } else { glyph(400 + (c as i16 % 7) * 30, 500 + (c as i16 % 5) * 60, 0) });
    }
    let filler = glyph(500, 500, 20);
    while glyphs.iter().map(Vec::len).sum::<usize>() + glyphs.len() * 8 < size {
        glyphs.push(filler.clone());
    }
    let count = glyphs.len() as u16;
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for g in &glyphs {
        loca.extend((glyf.len() as u32).to_be_bytes());
        glyf.extend(g);
    }
    loca.extend((glyf.len() as u32).to_be_bytes());
    let be16 = |v: &[u16]| v.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<u8>>();
    let mut head = be16(&[1, 0, 1, 0]);
    head.extend([0u8; 4]); // checksum adjustment
    head.extend(0x5F0F3CF5u32.to_be_bytes());
    head.extend(be16(&[0, 1000]));
    head.extend([0u8; 16]); // created, modified
    head.extend(be16(&[0, 0, 1000, 1000, 0, 8, 2, 1, 0])); // bbox, macStyle, lowest, direction, loca long, glyf format
    let mut hhea = be16(&[1, 0, 800, 0xff38, 0, 1000, 0, 0, 1000, 1, 0, 0, 0, 0, 0, 0, 0]);
    hhea.extend(count.to_be_bytes());
    let mut maxp = be16(&[1, 0, count, 64, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]);
    maxp.truncate(32);
    let hmtx: Vec<u8> = (0..count).flat_map(|i| be16(&[if i == 1 { 300 } else { 600 }, 0])).collect();
    // cmap format 4: 0x20..=0x7E to glyphs 1..=95.
    let mut sub = be16(&[4, 0, 0, 4, 4, 1, 0]);
    sub.extend(be16(&[0x7e, 0xffff, 0, 0x20, 0xffff]));
    sub.extend(be16(&[(1i16 - 0x20) as u16, 1, 0, 0]));
    let len = sub.len() as u16;
    sub[2..4].copy_from_slice(&len.to_be_bytes());
    let mut cmap = be16(&[0, 1, 3, 1, 0, 12]);
    cmap.extend(sub);
    let family: Vec<u8> = "Synth".encode_utf16().flat_map(|c| c.to_be_bytes()).collect();
    let mut name = be16(&[0, 2, 6 + 24]);
    for id in [1u16, 4] {
        name.extend(be16(&[3, 1, 0x409, id, family.len() as u16, 0]));
    }
    name.extend(&family);
    let mut os2 = be16(&[4, 500, 400, 5, 0]);
    os2.extend([0u8; 22 + 10 + 16 + 4]);
    os2.extend(be16(&[0x40, 0x20, 0x7e, 800, 200, 0, 800, 200]));
    os2.extend([0u8; 8]);
    os2.extend(be16(&[500, 700, 0, 0x20, 1]));
    let post = [0u8, 3, 0, 0].into_iter().chain([0u8; 28]).collect::<Vec<u8>>();
    let tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"OS/2", os2),
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"name", name),
        (b"post", post),
    ];
    let mut out = be16(&[1, 0, tables.len() as u16, 128, 3, 32]);
    let mut offset = 12 + 16 * tables.len();
    let mut data = Vec::new();
    for (tag, t) in &tables {
        out.extend(*tag);
        out.extend([0u8; 4]);
        out.extend((offset as u32).to_be_bytes());
        out.extend((t.len() as u32).to_be_bytes());
        data.extend(t);
        while data.len() % 4 != 0 {
            data.push(0);
        }
        offset = 12 + 16 * tables.len() + data.len();
    }
    out.extend(data);
    out
}

/// A WOFF 1 file of one table, `unpacked` bytes of zeros, zlib-compressed.
fn woff_bomb(unpacked: u64) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    let zeros = vec![0u8; 1 << 20];
    let mut left = unpacked;
    while left > 0 {
        let take = left.min(zeros.len() as u64) as usize;
        z.write_all(&zeros[..take]).unwrap();
        left -= take as u64;
    }
    let packed = z.finish().unwrap();
    // The file's length, padded to four bytes, as its header must say.
    let length = (44 + 20 + packed.len()).div_ceil(4) * 4;
    let mut out = Vec::new();
    out.extend(b"wOFF");
    out.extend(0x00010000u32.to_be_bytes());
    out.extend((length as u32).to_be_bytes());
    out.extend(1u16.to_be_bytes());
    out.extend(0u16.to_be_bytes());
    out.extend(((12 + 16) as u64 + unpacked).min(u64::from(u32::MAX)).to_be_bytes()[4..].iter());
    out.extend([0u8; 24]);
    out.extend(b"glyf");
    out.extend(64u32.to_be_bytes());
    out.extend((packed.len() as u32).to_be_bytes());
    out.extend((unpacked.min(u64::from(u32::MAX)) as u32).to_be_bytes());
    out.extend(0u32.to_be_bytes());
    out.extend(packed);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out
}
