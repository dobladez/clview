// Times one look at N directories and M files, as clv's watch does: a metadata call each.
use std::time::Instant;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (dirs, files): (usize, usize) = (args[1].parse().unwrap(), args[2].parse().unwrap());
    let root = std::env::temp_dir().join("clv-stat-measure");
    let _ = std::fs::remove_dir_all(&root);
    for d in 0..dirs { std::fs::create_dir_all(root.join(format!("d{d}"))).unwrap(); }
    for f in 0..files { std::fs::write(root.join(format!("d0/f{f}")), b"x").unwrap(); }
    let paths: Vec<_> = (0..dirs).map(|d| root.join(format!("d{d}"))).chain((0..files).map(|f| root.join(format!("d0/f{f}")))).collect();
    let mut best = f64::MAX;
    for _ in 0..20 {
        let t = Instant::now();
        let mut n = 0u64;
        for p in &paths { if let Ok(m) = std::fs::metadata(p) { n += m.len(); } }
        std::hint::black_box(n);
        best = best.min(t.elapsed().as_secs_f64());
    }
    println!("{dirs} directories + {files} files: best of 20 looks {:.3} ms", best * 1000.0);
    let _ = std::fs::remove_dir_all(&root);
}
