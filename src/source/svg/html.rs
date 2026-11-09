//! An SVG's HTML labels, laid out and painted by Blitz (Stylo's styles, Taffy's layout, Parley's
//! text) through anyrender into vello_cpu: the one file that names Blitz and what it paints with
//! (ADR-0024). `svg.rs` finds the labels, places them and writes them out; this lays them out in
//! one document, records each label's painting once, and replays a label into the part of a page
//! it lands on, at whatever scale that page is drawn.
//!
//! Its costs are bounded as ADR-0024 measured them: `data:` URLs only, each font and picture sized
//! from its header before Blitz has it; the painting estimated, after layout, before it is done
//! ([`paint_work`]); what is recorded counted as it is painted ([`Recorder`]).

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use anyrender::recording::RenderCommand;
use anyrender::{PaintScene, Scene, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use base64::Engine as _;
use blitz_dom::{BaseDocument, DocumentConfig, FontContext, NodeId};
use blitz_html::HtmlDocument;
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use kurbo::{Affine, Rect, Shape as _};

use super::Label;

/// Who lays the labels out, credited with the backend's own.
pub(super) const CREDIT: &str = "Blitz";

/// The largest side of a label laid out, in CSS pixels: past it, it is left out (ADR-0024).
pub(super) const MAX_LAYOUT_SIDE: f32 = 16_384.0;

/// How far past its box an `overflow: visible` label is drawn, each way, in CSS pixels.
pub(super) const OVERFLOW: f32 = 4_096.0;

/// The most a `data:` font may unpack to, as its header says, and all of a drawing's.
const MAX_FONT: u64 = 32 << 20;
const MAX_FONTS: u64 = 64 << 20;

/// The most a `data:` picture may decode to, as its header says, and all of a drawing's.
const MAX_PICTURE: u64 = 64 << 20;
const MAX_PICTURES: u64 = 256 << 20;

/// The most `data:` URLs answered for a drawing.
const MAX_REQUESTS: usize = 1_024;

/// The most work painting the labels may take, as [`paint_work`] estimates it: about 0.1 s of
/// painting with nothing kept (ADR-0024).
const MAX_PAINT_WORK: u64 = 20_000_000;

/// The most commands recorded for all of a drawing's labels: 200,000 replay in about 1.5 s
/// (ADR-0024).
const MAX_COMMANDS: usize = 200_000;

/// What a recorded command holds while it is replayed, at most: some 1 KB measured (ADR-0024).
pub(super) const COMMAND_BYTES: u64 = 2 << 10;

/// The stack of the thread that lays the labels out and records them: Blitz recurses once a level
/// of nesting, and 64 nested grids, `svg.rs`'s limit, hold on it in a debug build (ADR-0024).
const STACK: usize = 8 << 20;

/// Between the labels in their document, in CSS pixels, so that painting one culls its neighbours.
const GAP: f32 = 8.0;

/// The widest row of the labels' document, in CSS pixels.
const ROW: f32 = 1_048_576.0;

/// The most links a drawing's labels tell of: what isn't a `data:` URL, not fetched.
const MAX_LINKS: usize = 1_024;

/// A drawing's labels, laid out and recorded: what each paints, to replay for each page.
pub(super) struct Painted {
    /// Each label's recording, or `None` for one left out (laid out past [`MAX_LAYOUT_SIDE`]).
    labels: Vec<Option<Recorded>>,
    /// What the labels asked for that isn't a `data:` URL.
    pub(super) links: Vec<String>,
}

/// One label's painting, recorded at one CSS pixel to the pixel, from its box's top left.
struct Recorded {
    scene: Scene,
    /// What it paints over, in those pixels: its box, and what it draws past it (overflow), at
    /// most [`OVERFLOW`] each way.
    region: Rect,
}

/// The system's fonts, found once a run: some 30 ms for macOS's. Each drawing has a copy, to which
/// the fonts it embeds are added, so they stay its own.
fn system_fonts() -> FontContext {
    static FONTS: OnceLock<Mutex<FontContext>> = OnceLock::new();
    FONTS.get_or_init(|| Mutex::new(FontContext::default())).lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// The `labels`, under the drawing's stylesheets `css`, laid out and recorded, on a thread of
/// their own with an 8 MiB stack; or why they all are left out (a limit passed, Blitz failed, no
/// thread to run it on).
pub(super) fn prepare(css: &str, labels: &[Label]) -> Result<Painted, String> {
    std::thread::scope(|scope| {
        let thread = std::thread::Builder::new()
            .name("labels".into())
            .stack_size(STACK)
            .spawn_scoped(scope, || std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| recorded(css, labels))))
            .map_err(|e| format!("no thread to lay them out on ({e})"))?;
        match thread.join() {
            Ok(Ok(done)) => done,
            // Its panic is printed by the viewer's hook.
            Ok(Err(_)) | Err(_) => Err("Blitz failed on them".into()),
        }
    })
}

/// Where a label's slot is in the labels' document, its top left in CSS pixels.
type At = (f32, f32);

/// Where each label is in the labels' document: rows about as wide as all of them would be square,
/// so the document's coordinates stay small; and the document's size. A label larger than
/// [`MAX_LAYOUT_SIDE`] has no place.
fn packed(labels: &[Label]) -> (Vec<Option<At>>, (f32, f32)) {
    let slot = |l: &Label| {
        let margin = if l.visible { OVERFLOW } else { 0.0 };
        (l.width + 2.0 * margin, l.height + 2.0 * margin)
    };
    let fits = |l: &Label| l.width <= MAX_LAYOUT_SIDE && l.height <= MAX_LAYOUT_SIDE;
    let area: f64 = labels.iter().filter(|l| fits(l)).map(|l| f64::from(slot(l).0 + GAP) * f64::from(slot(l).1 + GAP)).sum();
    let widest = labels.iter().filter(|l| fits(l)).map(|l| slot(l).0 + GAP).fold(0.0f32, f32::max);
    // Within f32: at most 1,024 labels of at most 24,576 pixels a side.
    let row = (area.sqrt() as f32).clamp(widest, ROW.max(widest));
    let (mut x, mut y, mut row_height, mut width) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut at = Vec::with_capacity(labels.len());
    for label in labels {
        if !fits(label) {
            at.push(None);
            continue;
        }
        let (w, h) = slot(label);
        if x > 0.0 && x + w > row {
            (x, y, row_height) = (0.0, y + row_height, 0.0);
        }
        at.push(Some((x, y)));
        x += w.ceil() + GAP;
        width = width.max(x);
        row_height = row_height.max(h.ceil() + GAP);
    }
    (at, (width, y + row_height))
}

/// The labels' document: the drawing's stylesheets, then each label in a slot of its own at
/// `at`, placed by a transform (Blitz culls by a box that counts an element's position twice:
/// placed by `left` and `top`, a slot wouldn't cull its neighbours), its box inside the slot, the
/// containing block of what it holds.
fn document(css: &str, labels: &[Label], at: &[Option<At>]) -> String {
    let mut out = String::from("<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><style><![CDATA[");
    out.push_str(&css.replace("]]>", "]]]]><![CDATA[>"));
    out.push_str(
        "]]></style><style>html,body{margin:0 !important;padding:0 !important;background:transparent !important;\
         overflow:hidden !important}</style></head><body>",
    );
    for (label, at) in labels.iter().zip(at) {
        let Some((x, y)) = at else { continue };
        let margin = if label.visible { OVERFLOW } else { 0.0 };
        let (w, h) = (label.width, label.height);
        out.push_str(&format!(
            "<div style=\"position:absolute;left:0;top:0;transform:translate({x}px,{y}px);width:{}px;height:{}px;overflow:hidden;\
             margin:0;padding:0;border:0\"><div style=\"position:absolute;left:{margin}px;top:{margin}px;width:{w}px;height:{h}px;\
             margin:0;padding:0;border:0\">",
            w + 2.0 * margin,
            h + 2.0 * margin
        ));
        out.push_str(&label.markup);
        out.push_str("</div></div>");
    }
    out.push_str("</body></html>");
    out
}

/// The `labels` laid out in one document and each recorded, or why none can be.
fn recorded(css: &str, labels: &[Label]) -> Result<Painted, String> {
    let (at, size) = packed(labels);
    let markup = document(css, labels, &at);
    let net = Arc::new(DataOnly::default());
    let config = DocumentConfig {
        // Within u32: the labels' document is at most some 800,000 pixels a side.
        viewport: Some(Viewport::new(size.0.ceil().max(1.0) as u32, size.1.ceil().max(1.0) as u32, 1.0, ColorScheme::Light)),
        net_provider: Some(net.clone()),
        font_ctx: Some(system_fonts()),
        // A base a relative link resolves against (Blitz's own, a `data:` URL, can't be one, and
        // it panics), to a file not read: told of, as any other link.
        base_url: Some("file:///".into()),
        ..DocumentConfig::default()
    };
    let mut doc = HtmlDocument::from_xml(&markup, config);
    drop(markup);
    doc.handle_messages();
    // A picture a style asks for (`background-image`) is fetched as the styles resolve, and
    // answered after: they resolve again with it.
    doc.resolve(0.0);
    doc.handle_messages();
    doc.resolve(0.0);
    let passes = u64::try_from(at.iter().flatten().count()).unwrap_or(u64::MAX);
    let work = paint_work(&doc, passes);
    if work.steps > MAX_PAINT_WORK {
        return Err(format!("painting them would take too long ({} million steps)", work.steps / 1_000_000));
    }
    let mut oversized = work.oversized.into_iter();
    let boxes = boxes(&doc);
    let (mut budget, mut past) = (MAX_COMMANDS, 0u64);
    let mut recorded = Vec::with_capacity(labels.len());
    let mut slots = boxes.into_iter();
    for (label, at) in labels.iter().zip(&at) {
        // A label with no place has no slot: the next slot is the next placed label's.
        let Some((x, y)) = at else {
            recorded.push(None);
            continue;
        };
        let (Some(region), false) = (slots.next(), oversized.next().unwrap_or(true)) else {
            recorded.push(None);
            continue;
        };
        let margin = f64::from(if label.visible { OVERFLOW } else { 0.0 });
        let (sw, sh) = (f64::from(label.width) + 2.0 * margin, f64::from(label.height) + 2.0 * margin);
        doc.set_viewport_scroll(blitz_dom::Point { x: f64::from(*x), y: f64::from(*y) });
        let mut recorder = Recorder { scene: Scene::new(), budget: &mut budget, past: &mut past };
        // Within u32: a slot is at most 24,576 pixels a side.
        blitz_paint::paint_scene(&mut recorder, &mut doc, 1.0, sw.ceil() as u32, sh.ceil() as u32, 0, 0);
        let scene = recorder.scene;
        if past > 0 {
            return Err(format!("painting them would take more than {MAX_COMMANDS} steps"));
        }
        // From the slot's top left to the box's.
        let region = (region + kurbo::Vec2::new(-margin, -margin)).intersect(Rect::new(-margin, -margin, sw - margin, sh - margin));
        let scene = Scene { tolerance: scene.tolerance, commands: scene.commands.into_iter().map(|c| moved(c, -margin)).collect() };
        recorded.push(Some(Recorded { scene, region }));
    }
    drop(doc);
    let links = net.links.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Ok(Painted { labels: recorded, links })
}

/// `command` moved by `by` each way.
fn moved(command: RenderCommand, by: f64) -> RenderCommand {
    let shift = |t: Affine| Affine::translate((by, by)) * t;
    match command {
        RenderCommand::PushLayer(mut c) => {
            c.transform = shift(c.transform);
            RenderCommand::PushLayer(c)
        }
        RenderCommand::PushClipLayer(mut c) => {
            c.transform = shift(c.transform);
            RenderCommand::PushClipLayer(c)
        }
        RenderCommand::Stroke(mut c) => {
            c.transform = shift(c.transform);
            RenderCommand::Stroke(c)
        }
        RenderCommand::Fill(mut c) => {
            c.transform = shift(c.transform);
            RenderCommand::Fill(c)
        }
        RenderCommand::GlyphRun(mut c) => {
            c.transform = shift(c.transform);
            RenderCommand::GlyphRun(c)
        }
        RenderCommand::BoxShadow(mut c) => {
            c.transform = shift(c.transform);
            RenderCommand::BoxShadow(c)
        }
        RenderCommand::PopLayer => RenderCommand::PopLayer,
    }
}

/// What each slot of `doc` paints over, in its slot's pixels, in order: its `<foreignObject>`'s
/// box (the first element of that name in the slot), and what overflows it.
fn boxes(doc: &BaseDocument) -> Vec<Rect> {
    let node = |id: NodeId| doc.get_node(id);
    let is_element = |id: &NodeId| node(*id).is_some_and(|n| n.element_data().is_some());
    let named = |id: NodeId, name: &str| node(id).and_then(|n| n.element_data()).is_some_and(|e| &*e.name.local == name);
    let html = doc.root_element();
    let Some(body) = html.children.iter().copied().find(|c| named(*c, "body")).and_then(node) else { return Vec::new() };
    let mut out = Vec::new();
    for slot in body.children.iter().copied().filter(is_element) {
        let mut walk = vec![slot];
        let mut found = None;
        while let Some(id) = walk.pop() {
            if named(id, "foreignObject") {
                found = Some(id);
                break;
            }
            if let Some(n) = node(id) {
                walk.extend(n.children.iter().rev().copied().filter(is_element));
            }
        }
        let region = found.and_then(node).map_or(Rect::ZERO, |fo| {
            let at = fo.absolute_position(0.0, 0.0);
            let size = fo.final_layout().size;
            let border = Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height));
            fo.scrollable_overflow().union(border) + kurbo::Vec2::new(f64::from(at.x), f64::from(at.y))
        });
        out.push(region);
    }
    out
}

impl Painted {
    /// What label `index` paints over, in its box's CSS pixels from its top left: `None` when it
    /// was left out.
    pub(super) fn region(&self, index: usize) -> Option<[f64; 4]> {
        let r = self.labels.get(index)?.as_ref()?.region;
        Some([r.x0, r.y0, r.x1, r.y1])
    }

    /// What replaying label `index` through `to` (from its box's pixels to a part of a page
    /// `size` pixels) costs: its commands, and the pixels they paint there (each command's box
    /// within the part, as the drawing's estimate counts a shape's), so a page can be refused before
    /// it is drawn.
    pub(super) fn cost(&self, index: usize, to: [f64; 6], size: (u32, u32)) -> (u64, u64) {
        let Some(Some(label)) = self.labels.get(index) else { return (0, 0) };
        let (to, part) = (Affine::new(to), Rect::new(0.0, 0.0, f64::from(size.0), f64::from(size.1)));
        let area = |r: Rect| {
            let r = r.intersect(part);
            // `as` saturates, and makes NaN 0.
            (r.width().max(0.0) * r.height().max(0.0)).ceil() as u64
        };
        let mut pixels = 0u64;
        for command in &label.scene.commands {
            let painted = match command {
                RenderCommand::PushLayer(c) => area((to * c.transform).transform_rect_bbox(c.clip.bounding_box())),
                RenderCommand::PushClipLayer(c) => area((to * c.transform).transform_rect_bbox(c.clip.bounding_box())),
                RenderCommand::Stroke(c) => {
                    area((to * c.transform).transform_rect_bbox(c.shape.bounding_box().inflate(c.style.width, c.style.width)))
                }
                RenderCommand::Fill(c) => area((to * c.transform).transform_rect_bbox(c.shape.bounding_box())),
                RenderCommand::BoxShadow(c) => {
                    let spread = c.std_dev * 3.0;
                    area((to * c.transform).transform_rect_bbox(c.rect.inflate(spread, spread)))
                }
                RenderCommand::GlyphRun(c) => {
                    let size = f64::from(c.font_size);
                    let empty = Rect::new(f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
                    let run = c.glyphs.iter().fold(empty, |r, g| r.union_pt(kurbo::Point::new(f64::from(g.x), f64::from(g.y))));
                    let run = if run.x0 <= run.x1 { run.inflate(size, size) } else { Rect::ZERO };
                    let glyphs = c.glyph_transform.unwrap_or(Affine::IDENTITY);
                    area((to * c.transform).transform_rect_bbox(glyphs.transform_rect_bbox(run)))
                }
                RenderCommand::PopLayer => 0,
            };
            pixels = pixels.saturating_add(painted);
        }
        (u64::try_from(label.scene.commands.len()).unwrap_or(u64::MAX), pixels)
    }

    /// Label `index` replayed through `to` into `size` pixels: premultiplied RGBA, row by row.
    /// `None` when it was left out, or its replay failed.
    pub(super) fn paint(&self, index: usize, to: [f64; 6], size: (u32, u32)) -> Option<Vec<u8>> {
        let label = self.labels.get(index)?.as_ref()?;
        if size.0 == 0 || size.1 == 0 || to.iter().any(|v| !v.is_finite()) {
            return None;
        }
        // vello_cpu isn't built never to panic: a panic costs this label only.
        let rgba = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            render_to_buffer::<VelloCpuImageRenderer, _>(|scene| scene.append_scene(label.scene.clone(), Affine::new(to)), size.0, size.1)
        }))
        .ok()?;
        let expected = u64::from(size.0).saturating_mul(u64::from(size.1)).saturating_mul(4);
        (u64::try_from(rgba.len()).ok() == Some(expected)).then_some(rgba)
    }
}

/// An anyrender `Scene` that keeps a command only while `budget` has room: past it, it counts what
/// Blitz paints and keeps nothing, so a recording holds at most the budget's commands. A fill of a
/// colour with nothing of it (a root's `transparent` background) draws nothing, and isn't kept.
struct Recorder<'b> {
    scene: Scene,
    budget: &'b mut usize,
    /// Commands painted past the budget, kept nowhere.
    past: &'b mut u64,
}

impl Recorder<'_> {
    /// Whether one more command may be kept.
    fn take(&mut self) -> bool {
        match self.budget.checked_sub(1) {
            Some(left) => {
                *self.budget = left;
                true
            }
            None => {
                *self.past = self.past.saturating_add(1);
                false
            }
        }
    }
}

impl anyrender::RenderContext for Recorder<'_> {}

impl PaintScene for Recorder<'_> {
    fn reset(&mut self) {
        self.scene.reset();
    }

    fn push_layer(
        &mut self,
        fill: peniko::Fill,
        blend: impl Into<peniko::BlendMode>,
        alpha: f32,
        transform: Affine,
        clip: &impl kurbo::Shape,
        filter: Option<Arc<anyrender::Filter>>,
        backdrop_filter: Option<Arc<anyrender::Filter>>,
    ) {
        if self.take() {
            self.scene.push_layer(fill, blend, alpha, transform, clip, filter, backdrop_filter);
        }
    }

    fn push_clip_layer(&mut self, fill: peniko::Fill, transform: Affine, clip: &impl kurbo::Shape) {
        if self.take() {
            self.scene.push_clip_layer(fill, transform, clip);
        }
    }

    fn pop_layer(&mut self) {
        if self.take() {
            self.scene.pop_layer();
        }
    }

    fn stroke<'a>(
        &mut self,
        style: &kurbo::Stroke,
        transform: Affine,
        brush: impl Into<anyrender::PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl kurbo::Shape,
    ) {
        if self.take() {
            self.scene.stroke(style, transform, brush, brush_transform, shape);
        }
    }

    fn fill<'a>(
        &mut self,
        style: peniko::Fill,
        transform: Affine,
        brush: impl Into<anyrender::PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl kurbo::Shape,
    ) {
        let brush = brush.into();
        if matches!(brush, anyrender::Paint::Solid(colour) if colour.components[3] == 0.0) {
            return;
        }
        if self.take() {
            self.scene.fill(style, transform, brush, brush_transform, shape);
        }
    }

    fn draw_glyphs<'a, 's: 'a>(
        &'s mut self,
        font: &'a peniko::FontData,
        font_size: f32,
        hint: bool,
        normalized_coords: &'a [anyrender::NormalizedCoord],
        embolden: kurbo::Vec2,
        style: impl Into<peniko::StyleRef<'a>>,
        brush: impl Into<anyrender::PaintRef<'a>>,
        brush_alpha: f32,
        transform: Affine,
        glyph_transform: Option<Affine>,
        glyphs: impl Iterator<Item = anyrender::Glyph> + Clone,
    ) {
        if self.take() {
            self.scene.draw_glyphs(font, font_size, hint, normalized_coords, embolden, style, brush, brush_alpha, transform, glyph_transform, glyphs);
        }
    }

    fn draw_box_shadow(&mut self, transform: Affine, rect: Rect, brush: peniko::Color, radius: f64, std_dev: f64) {
        if self.take() {
            self.scene.draw_box_shadow(transform, rect, brush, radius, std_dev);
        }
    }
}

// The paint estimate. It follows blitz-paint 0.3.0-beta.4's painting, which the version guard
// pins: `box_shadow.rs`, and `background.rs`'s `compute_layer_size`,
// `raster_axis_tiling` and `gradient_axis_tiling` (ADR-0024).

/// Commands an inset or outset shadow paints at most: an inset one pushes two layers, fills,
/// shadows and pops.
const PER_SHADOW: u64 = 6;

/// Commands a gradient layer paints at most: blitz-paint paints none past 500 tiles.
const PER_GRADIENT: u64 = 500;

/// A dot of a dotted border, or a dash of a dashed one, in steps: blitz-paint builds an edge's
/// dots into one path, some 170 bytes a dot (ADR-0024), held as a command is.
const DOT_STEPS: u64 = 20;

/// A pixel of a dotted, dashed or wavy text decoration, in steps: a wave is a path of a segment
/// a pixel or two.
const DECORATION_STEPS: u64 = 4;

/// A gradient's stops, as many as hold what a command does (some 1 KB, 20 bytes a stop).
const STOPS_A_STEP: u64 = 32;

/// Commands a layer paints around its tiles: its clip layer, pushed and popped (a mask's two).
const PER_LAYER: u64 = 4;

/// The largest side of any element, or line of text, of a label: a label's own side and what it
/// draws past it each way. blitz-paint builds some paths at their box's size (a dotted border's
/// dots, before anything counts them), so a box past it leaves its label out.
pub(super) const MAX_ELEMENT_SIDE: f32 = MAX_LAYOUT_SIDE + 2.0 * OVERFLOW;

/// What painting the labels' document takes, before it is done.
struct Work {
    /// In commands, where CSS lists, tiling or a box's size multiply them.
    steps: u64,
    /// For each slot, in order, whether an element of it is past [`MAX_ELEMENT_SIDE`].
    oversized: Vec<bool>,
}

/// What painting `doc` takes, its labels painted in `passes` (one each), where CSS lists, tiling or
/// a box's size multiply the commands: each element's shadows; each background and mask layer's
/// tiles, a gradient's stops too; each dotted or dashed border edge's dots or dashes; each line of
/// text a dotted, dashed or wavy decoration may draw along. The document's own `html` and `body`
/// (and their `::before` and `::after`) are painted with every label, so they count once a pass.
/// What else an element paints is a few commands, or one a line of its text, bounded by the labels'
/// elements and text.
fn paint_work(doc: &BaseDocument, passes: u64) -> Work {
    use style::properties::generated::longhands::background_attachment::single_value::computed_value::T as Attachment;
    use style::properties::generated::longhands::text_decoration_style::computed_value::T as DecorationStyle;
    use style::values::computed::{BorderStyle, TextDecorationLine};
    use style::values::generics::image::GenericImage;
    let viewport = doc.viewport().window_size;
    let (mut steps, mut oversized, mut decorated, mut lines) = (0u64, Vec::new(), 0u64, 0u64);
    // Each node, whether it is painted with every label, and its slot.
    let mut walk = vec![(doc.root_element().id, true, None::<usize>)];
    while let Some((id, shared, slot)) = walk.pop() {
        let Some(node) = doc.get_node(id) else { continue };
        let body = node.element_data().is_some_and(|e| &*e.name.local == "body") && shared;
        for child in node.children.iter().copied() {
            let element = doc.get_node(child).is_some_and(|c| c.element_data().is_some());
            let slot = match (body, element) {
                (true, true) => {
                    oversized.push(false);
                    Some(oversized.len().saturating_sub(1))
                }
                _ => slot,
            };
            walk.push((child, shared && slot.is_none(), slot));
        }
        walk.extend([node.before(), node.after()].into_iter().flatten().map(|p| (p, shared, slot)));
        let (Some(element), Some(style)) = (node.element_data(), node.primary_styles()) else { continue };
        let layout = node.final_layout();
        let (width, height) = (f64::from(layout.size.width), f64::from(layout.size.height));
        let text = element.inline_layout_data.as_ref().map(|t| &t.layout);
        let wide = text.is_some_and(|t| t.full_width() > MAX_ELEMENT_SIDE);
        if (layout.size.width > MAX_ELEMENT_SIDE || layout.size.height > MAX_ELEMENT_SIDE || wide)
            && let Some(flag) = slot.and_then(|s| oversized.get_mut(s))
        {
            *flag = true;
        }
        let mut own = 0u64;
        let shadows = u64::try_from(style.get_effects().box_shadow.0.len()).unwrap_or(u64::MAX);
        own = own.saturating_add(PER_SHADOW.saturating_mul(shadows));
        let background = style.get_background();
        for (index, image) in background.background_image.0.iter().enumerate() {
            if matches!(image, GenericImage::None) {
                continue;
            }
            // A fixed layer is placed in the area painted: the slot, at most the viewport.
            let fixed = matches!(cyclic(&background.background_attachment.0, index), Some(Attachment::Fixed));
            let area = if fixed { (width.max(f64::from(viewport.0)), height.max(f64::from(viewport.1))) } else { (width, height) };
            let data = element.background_images.get(index).and_then(Option::as_ref);
            let tiles = layer_tiles(image, data, cyclic(&background.background_repeat.0, index), cyclic(&background.background_size.0, index), area);
            own = own.saturating_add(PER_LAYER).saturating_add(tiles);
        }
        let svg = style.get_svg();
        for (index, image) in svg.mask_image.0.iter().enumerate() {
            if matches!(image, GenericImage::None) {
                continue;
            }
            let data = element.mask_images.get(index).and_then(Option::as_ref);
            let tiles = layer_tiles(image, data, cyclic(&svg.mask_repeat.0, index), cyclic(&svg.mask_size.0, index), (width, height));
            own = own.saturating_add(PER_LAYER).saturating_add(tiles);
        }
        // A dotted edge is a dot each twice its width along it (the whole perimeter, where its
        // corners are rounded), a dashed one fewer: counted as the perimeter over the width.
        let border = style.get_border();
        let edges = [
            (border.border_top_style, layout.border.top),
            (border.border_bottom_style, layout.border.bottom),
            (border.border_left_style, layout.border.left),
            (border.border_right_style, layout.border.right),
        ];
        for (style, thickness) in edges {
            if matches!(style, BorderStyle::Dotted | BorderStyle::Dashed) && thickness > 0.0 {
                // `as` saturates, and makes NaN 0.
                let dots = ((width + height) / f64::from(thickness)).ceil() as u64;
                own = own.saturating_add(DOT_STEPS.saturating_mul(dots)).saturating_add(1);
            }
        }
        let text_style = style.get_text();
        let lines_drawn = TextDecorationLine::UNDERLINE | TextDecorationLine::OVERLINE | TextDecorationLine::LINE_THROUGH;
        if text_style.text_decoration_line.intersects(lines_drawn)
            && !matches!(text_style.text_decoration_style, DecorationStyle::Solid | DecorationStyle::Double)
        {
            decorated = decorated.saturating_add(1);
        }
        if let Some(text) = text {
            // A line's dots, dashes or waves: at most one a pixel of it, each at least a pixel thick.
            for line in text.lines() {
                // `as` saturates, and makes NaN 0.
                lines = lines.saturating_add(line.metrics().advance.ceil() as u64).saturating_add(1);
            }
        }
        steps = steps.saturating_add(if shared { own.saturating_mul(passes) } else { own });
    }
    // Each decorating box may draw its three lines along every line of text: a high count, and one
    // that is none where no decoration is dotted, dashed or wavy.
    steps = steps.saturating_add(decorated.saturating_mul(3).saturating_mul(DECORATION_STEPS).saturating_mul(lines));
    Work { steps, oversized }
}

/// `values[index]`, the list repeated as CSS repeats a layer's shorter lists.
fn cyclic<T>(values: &[T], index: usize) -> Option<&T> {
    values.get(index.checked_rem(values.len())?)
}

/// The tiles one image layer paints over `area`, as blitz-paint tiles it: a gradient at most
/// [`PER_GRADIENT`]; a picture once, but once a tile on an axis it spaces (`space`).
fn layer_tiles(
    image: &style::values::computed::Image,
    data: Option<&blitz_dom::node::ImageResourceData>,
    repeat: Option<&style::values::computed::BackgroundRepeat>,
    size: Option<&style::values::computed::background::BackgroundSize>,
    area: (f64, f64),
) -> u64 {
    use style::values::generics::image::GenericImage;
    use style::values::specified::background::BackgroundRepeatKeyword::Space;
    match image {
        // Each of its tiles a fill holding all its stops.
        GenericImage::Gradient(gradient) => PER_GRADIENT.saturating_mul(1u64.saturating_add(stops(gradient) / STOPS_A_STEP)),
        GenericImage::Url(_) => {
            let Some(blitz_dom::node::ImageData::Raster(raster)) = data.map(|d| &d.image) else { return 0 };
            let (Some(repeat), Some(size)) = (repeat, size) else { return 1 };
            let tile = layer_size(size, area, (f64::from(raster.width), f64::from(raster.height)));
            let count = |keyword, length: f64, tile: f64| if keyword == Space { space_count(length, tile) } else { 1 };
            count(repeat.0, area.0, tile.0).saturating_mul(count(repeat.1, area.1, tile.1))
        }
        _ => 0,
    }
}

/// A gradient's stops (and hints).
fn stops(gradient: &style::values::computed::Gradient) -> u64 {
    use style::values::generics::image::GenericGradient;
    let items = match gradient {
        GenericGradient::Linear { items, .. } | GenericGradient::Radial { items, .. } => items.len(),
        GenericGradient::Conic { items, .. } => items.len(),
    };
    u64::try_from(items).unwrap_or(u64::MAX)
}

/// How many tiles of `tile` fit in `length` spaced, at least one: blitz-paint's
/// `compute_space_count_and_stride`, whose `as u32` saturates.
fn space_count(length: f64, tile: f64) -> u64 {
    let count = (length - length % tile) / tile;
    // `as` saturates, and makes NaN 0.
    u64::from(count as u32).max(1)
}

/// A picture layer's size in `area`, `image` its natural size: blitz-paint's
/// `compute_layer_size`, as it sizes a picture.
fn layer_size(size: &style::values::computed::background::BackgroundSize, area: (f64, f64), image: (f64, f64)) -> (f64, f64) {
    use style::values::computed::Length;
    use style::values::computed::background::BackgroundSize;
    use style::values::generics::length::GenericLengthPercentageOrAuto as Lpa;
    let ((cw, ch), (iw, ih)) = (area, image);
    match size {
        BackgroundSize::ExplicitSize { width, height } => {
            // An area's sides are within f32: a slot's, or the labels' document's.
            let width = width.map(|w| w.0.resolve(Length::new(cw as f32)));
            let height = height.map(|h| h.0.resolve(Length::new(ch as f32)));
            match (width, height) {
                (Lpa::LengthPercentage(w), Lpa::LengthPercentage(h)) => (f64::from(w.px()), f64::from(h.px())),
                (Lpa::LengthPercentage(w), Lpa::Auto) => (f64::from(w.px()), ih / iw * f64::from(w.px())),
                (Lpa::Auto, Lpa::LengthPercentage(h)) => (iw / ih * f64::from(h.px()), f64::from(h.px())),
                (Lpa::Auto, Lpa::Auto) => (iw, ih),
            }
        }
        BackgroundSize::Cover => {
            let ratio = (cw / iw).max(ch / ih);
            (iw * ratio, ih * ratio)
        }
        BackgroundSize::Contain => {
            let ratio = (cw / iw).min(ch / ih);
            (iw * ratio, ih * ratio)
        }
    }
}

// What the labels may fetch: `data:` URLs, a font or a picture each, sized from its header.

/// Fetches nothing but `data:` URLs (no network, no files), and of them only fonts and
/// pictures, within their budgets: anything else, or past them, arrives empty, so nothing waits on
/// it.
#[derive(Default)]
struct DataOnly {
    requests: AtomicUsize,
    fonts: AtomicU64,
    pictures: AtomicU64,
    links: Mutex<Vec<String>>,
}

impl NetProvider for DataOnly {
    fn fetch(&self, _doc: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.as_str().to_owned();
        let bytes = self.answer(&url).unwrap_or_default();
        handler.bytes(url, Bytes::from(bytes));
    }
}

impl DataOnly {
    /// What `url` is answered with, if anything.
    fn answer(&self, url: &str) -> Option<Vec<u8>> {
        let Some(data) = url.strip_prefix("data:") else {
            let mut links = self.links.lock().unwrap_or_else(PoisonError::into_inner);
            if links.len() < MAX_LINKS {
                links.push(url.chars().take(256).collect());
            }
            return None;
        };
        if self.requests.fetch_add(1, Ordering::Relaxed) >= MAX_REQUESTS {
            return None;
        }
        let bytes = data_url(data)?;
        // Counted in its total only when taken.
        let take = |total: &AtomicU64, size: u64, each: u64, all: u64| {
            size <= each && total.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |t| t.checked_add(size).filter(|n| *n <= all)).is_ok()
        };
        if let Some(size) = font_size(&bytes) {
            return take(&self.fonts, size, MAX_FONT, MAX_FONTS).then_some(bytes);
        }
        let picture = matches!(
            imagesize::image_type(&bytes),
            Ok(imagesize::ImageType::Png | imagesize::ImageType::Jpeg | imagesize::ImageType::Gif | imagesize::ImageType::Webp)
        );
        let size = imagesize::blob_size(&bytes).ok().filter(|_| picture)?;
        let decoded = u64::try_from(size.width).unwrap_or(u64::MAX).saturating_mul(u64::try_from(size.height).unwrap_or(u64::MAX)).saturating_mul(4);
        take(&self.pictures, decoded, MAX_PICTURE, MAX_PICTURES).then_some(bytes)
    }
}

/// What a `data:` URL's `data` (after `data:`) holds: base64, or percent-encoded.
fn data_url(data: &str) -> Option<Vec<u8>> {
    let (meta, payload) = data.split_once(',')?;
    if meta.ends_with(";base64") {
        let clean: Vec<u8> = percent_decoded(payload).into_iter().filter(|b| !b.is_ascii_whitespace()).collect();
        return base64::engine::general_purpose::STANDARD.decode(clean).ok();
    }
    Some(percent_decoded(payload))
}

/// `s` with its `%XX` escapes decoded.
fn percent_decoded(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = |at: usize| bytes.get(at).and_then(|c| char::from(*c).to_digit(16));
        match (b, hex(i.saturating_add(1)), hex(i.saturating_add(2))) {
            (b'%', Some(high), Some(low)) => {
                // Two hex digits: at most 255.
                out.push((high * 16 + low) as u8);
                i = i.saturating_add(3);
            }
            _ => {
                out.push(b);
                i = i.saturating_add(1);
            }
        }
    }
    out
}

/// What the font in `bytes` unpacks to, as its header says, or `None` when it isn't a font Blitz
/// reads: a WOFF the larger of its `totalSfntSize` and its tables' sizes, which its decoder (wuff)
/// sizes its output by; a TrueType or OpenType font (or collection) its bytes.
fn font_size(bytes: &[u8]) -> Option<u64> {
    let u32_at = |at: usize| bytes.get(at..at.checked_add(4)?).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(|b| u64::from(u32::from_be_bytes(b)));
    let u16_at = |at: usize| bytes.get(at..at.checked_add(2)?).and_then(|b| <[u8; 2]>::try_from(b).ok()).map(|b| u64::from(u16::from_be_bytes(b)));
    let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    match bytes.get(..4)? {
        b"wOFF" => {
            let mut sum = 0u64;
            for table in 0..u16_at(12)? {
                // Each entry 20 bytes from 44, its `origLength` at 12.
                let at = usize::try_from(table.saturating_mul(20).saturating_add(44 + 12)).ok()?;
                sum = sum.saturating_add(u32_at(at)?);
            }
            Some(u32_at(16)?.max(sum))
        }
        b"wOF2" => {
            let (mut at, mut orig, mut stored) = (48usize, 0u64, 0u64);
            for _ in 0..u16_at(12)? {
                let flags = *bytes.get(at)?;
                at = at.checked_add(if flags & 0x3f == 0x3f { 5 } else { 1 })?;
                let (length, next) = base128(bytes, at)?;
                orig = orig.saturating_add(length);
                at = next;
                // glyf and loca are transformed at version 0, the others at any other.
                let version = flags >> 6;
                let transformed = if matches!(flags & 0x3f, 10 | 11) { version == 0 } else { version != 0 };
                if transformed {
                    let (length, next) = base128(bytes, at)?;
                    stored = stored.saturating_add(length);
                    at = next;
                } else {
                    stored = stored.saturating_add(length);
                }
            }
            Some(u32_at(16)?.max(orig).max(stored))
        }
        b"OTTO" | b"true" | b"ttcf" | [0, 1, 0, 0] => Some(length),
        _ => None,
    }
}

/// The UIntBase128 at `at` in `bytes` (WOFF 2's variable-length number), and where it ends.
fn base128(bytes: &[u8], at: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for i in 0..5usize {
        let b = *bytes.get(at.checked_add(i)?)?;
        if i == 0 && b == 0x80 {
            return None;
        }
        value = value.checked_mul(128)?.checked_add(u64::from(b & 0x7f))?;
        if b & 0x80 == 0 {
            return Some((value, at.checked_add(i)?.checked_add(1)?));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `data:` URL of `bytes`, in base64.
    fn data(bytes: &[u8]) -> String {
        format!("data:application/octet-stream;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    /// A WOFF header (and its table directory) saying it unpacks to `size` bytes.
    fn woff(size: u32) -> Vec<u8> {
        let mut out = b"wOFF".to_vec();
        out.extend(0x0001_0000u32.to_be_bytes());
        out.extend(64u32.to_be_bytes());
        out.extend(1u16.to_be_bytes());
        out.extend(0u16.to_be_bytes());
        out.extend(28u32.to_be_bytes());
        out.extend([0u8; 24]);
        out.extend(b"glyf");
        out.extend([64u32, 4, size, 0].iter().flat_map(|v| v.to_be_bytes()));
        out
    }

    /// The header of a PNG `width` × `height`, as far as its size.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        out.extend(width.to_be_bytes());
        out.extend(height.to_be_bytes());
        out.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        out
    }

    /// A font is sized from its header before Blitz unpacks it: past 32 MiB it is refused, and
    /// past 64 MiB of a drawing's fonts in all.
    #[test]
    fn a_font_is_refused_past_what_its_header_says_it_unpacks_to() {
        assert_eq!(font_size(&woff(32 << 20)), Some(32 << 20));
        let net = DataOnly::default();
        assert!(net.answer(&data(&woff((32 << 20) + 1))).is_none());
        assert!(net.answer(&data(&woff(32 << 20))).is_some());
        assert!(net.answer(&data(&woff(32 << 20))).is_some());
        assert!(net.answer(&data(&woff(1))).is_none(), "past 64 MiB in all");
        // A WOFF 2's tables: one of 33 MiB, its length in UIntBase128.
        let mut woff2 = b"wOF2".to_vec();
        woff2.extend([0u8; 8]);
        woff2.extend(1u16.to_be_bytes());
        woff2.extend([0u8; 2]);
        woff2.extend(10u32.to_be_bytes());
        woff2.extend([0u8; 28]);
        woff2.push(0x3f);
        woff2.extend(b"abcd");
        woff2.extend([0x90, 0xc0, 0x80, 0x00]);
        assert_eq!(font_size(&woff2), Some(33 << 20));
        assert!(DataOnly::default().answer(&data(&woff2)).is_none());
        assert_eq!(font_size(b"OTTO and the rest"), Some(17));
        assert_eq!(font_size(b"wOFF"), None, "cut short");
    }

    /// A picture is sized from its header before it is decoded: past 64 MiB decoded it is
    /// refused, and past 256 MiB of a drawing's in all. Only PNG, JPEG, GIF and WebP are fetched:
    /// an SVG, a stylesheet, a document arrive empty.
    #[test]
    fn a_picture_is_refused_past_what_its_header_says_it_decodes_to() {
        let net = DataOnly::default();
        assert!(net.answer(&data(&png(4097, 4096))).is_none());
        for _ in 0..4 {
            assert!(net.answer(&data(&png(4096, 4096))).is_some());
        }
        assert!(net.answer(&data(&png(1, 1))).is_none(), "past 256 MiB in all");
        let net = DataOnly::default();
        for other in ["data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg'/>", "data:text/css,p{color:red}", "data:text/html,<p>hi</p>"] {
            assert!(net.answer(other).is_none(), "{other}");
        }
        assert_eq!(net.answer("data:,a%41b"), None, "not a font or a picture");
        assert_eq!(data_url(",a%41b%zz"), Some(b"aAb%zz".to_vec()));
    }

    /// Only `data:` URLs are answered, at most 1,024 a drawing; what else a label asks for is
    /// told of, not fetched.
    #[test]
    fn only_so_many_data_urls_are_answered_and_nothing_else() {
        let net = DataOnly::default();
        for _ in 0..MAX_REQUESTS {
            assert!(net.answer(&data(&png(1, 1))).is_some());
        }
        assert!(net.answer(&data(&png(1, 1))).is_none());
        assert!(net.answer("https://example.com/font.woff2").is_none());
        assert!(net.answer("file:///etc/passwd").is_none());
        assert_eq!(*net.links.lock().unwrap(), ["https://example.com/font.woff2", "file:///etc/passwd"]);
    }

    /// Tiles spaced are counted as blitz-paint counts them, a tile of no size once.
    #[test]
    fn spaced_tiles_are_counted_as_blitz_paints_them() {
        assert_eq!(
            [space_count(10.0, 1.0), space_count(10.0, 3.0), space_count(10.0, 0.0), space_count(1e30, 1e-30)],
            [10, 3, 1, u64::from(u32::MAX)]
        );
    }
}
