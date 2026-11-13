//! `t-ariel-rs <in.mmd> <out.svg>`, or `batch <dir>`: as t-merman, through ariel-rs.
use std::time::Instant;
fn one(text: &str) -> Result<String, String> {
    ariel_rs::try_render(text, ariel_rs::theme::Theme::Default).map_err(|e| format!("{e}"))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args[1] == "batch" {
        let mut files: Vec<_> = std::fs::read_dir(&args[2]).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "mmd")).collect();
        files.sort();
        let all = Instant::now();
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            let t = Instant::now();
            let out = one(&text);
            eprintln!("{} {:.1} ms {}", f.display(), t.elapsed().as_secs_f64() * 1e3, out.map(|s| s.len().to_string()).unwrap_or_else(|e| e));
        }
        eprintln!("all {:.1} ms", all.elapsed().as_secs_f64() * 1e3);
        return;
    }
    let text = std::fs::read_to_string(&args[1]).unwrap_or_else(|_| String::from_utf8_lossy(&std::fs::read(&args[1]).unwrap()).into_owned());
    let t = Instant::now();
    let out = one(&text);
    let ms = t.elapsed().as_secs_f64() * 1e3;
    match out {
        Ok(svg) => { std::fs::write(&args[2], &svg).unwrap(); eprintln!("ok {ms:.1} ms {} bytes", svg.len()); }
        Err(e) => eprintln!("err {ms:.1} ms {}", e.chars().take(200).collect::<String>()),
    }
}
