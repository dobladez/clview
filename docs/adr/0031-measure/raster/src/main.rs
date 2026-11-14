//! `raster <in.svg> <out.png> [scale]`: an SVG drawn as clv's SVG backend draws it, resvg 0.49
//! with the system's fonts, on white.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let scale: f32 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(1.5);
    let mut opt = resvg::usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    let data = std::fs::read(&a[1]).unwrap();
    let tree = resvg::usvg::Tree::from_data(&data, &opt).unwrap();
    let size = tree.size().to_int_size().scale_by(scale).unwrap();
    let mut pix = resvg::tiny_skia::Pixmap::new(size.width(), size.height()).unwrap();
    pix.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pix.as_mut());
    pix.save_png(&a[2]).unwrap();
}
