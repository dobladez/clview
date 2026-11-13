//! `t-merman <in.mmd> <out.svg> [html]`: one diagram to SVG through merman, labels as SVG
//! `<text>` unless `html`, the time on stderr. `t-merman batch <dir>`: every `.mmd` in `dir`,
//! one process, each time on stderr.
use std::time::Instant;

use merman::model::MermaidConfig;
use merman::{OperationControl, RenderOutput, RenderRequest, Renderer, SemanticParser, SvgRequest};

fn renderer(html: bool) -> Renderer {
    let config = serde_json::json!({
        "htmlLabels": html,
        "flowchart": { "htmlLabels": html },
        "secure": ["secure", "securityLevel", "startOnLoad", "maxTextSize", "suppressErrorRendering", "maxEdges", "htmlLabels"],
    });
    let r = Renderer::new().with_semantic_parser(SemanticParser::new().with_site_config(MermaidConfig::from_value(config)));
    // `MERMAN_PROFILE=interactive`: merman's default limits; otherwise its constrained ones, as clv would.
    let profile = if std::env::var_os("MERMAN_PROFILE").is_some_and(|p| p == "interactive") { merman::model::resources::ResourceProfile::Interactive } else { merman::model::resources::ResourceProfile::Constrained };
    r.with_resource_policy(merman::model::resources::InputResourcePolicy::for_profile(profile))
}

/// `MERMAN_DEADLINE_MS=<n>`: merman stops itself past n ms.
fn control() -> OperationControl {
    match std::env::var("MERMAN_DEADLINE_MS").ok().and_then(|n| n.parse().ok()) {
        Some(ms) => OperationControl::new().with_deadline(std::time::Duration::from_millis(ms)),
        None => OperationControl::new(),
    }
}

fn one(r: &Renderer, text: &str) -> Result<String, String> {
    match r.render(RenderRequest::svg(text, control(), SvgRequest::default())) {
        Ok(RenderOutput::Svg(Some(svg))) => Ok(svg.svg().to_string()),
        Ok(_) => Err("no diagram".into()),
        Err(e) => Err(format!("{e}")),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args[1] == "kinds" {
        for f in merman::diagram_header_facts() { println!("{}\t{}\t{}", f.diagram_type, f.label, f.detail); }
        println!("supported: {:?}", merman::supported_diagrams());
        return;
    }
    if args[1] == "batch" {
        let r = renderer(false);
        let mut files: Vec<_> = std::fs::read_dir(&args[2]).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "mmd")).collect();
        files.sort();
        let all = Instant::now();
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            let t = Instant::now();
            let out = one(&r, &text);
            eprintln!("{} {:.1} ms {}", f.display(), t.elapsed().as_secs_f64() * 1e3, out.map(|s| s.len().to_string()).unwrap_or_else(|e| e));
        }
        eprintln!("all {:.1} ms", all.elapsed().as_secs_f64() * 1e3);
        return;
    }
    let text = std::fs::read_to_string(&args[1]).unwrap_or_else(|_| String::from_utf8_lossy(&std::fs::read(&args[1]).unwrap()).into_owned());
    let r = renderer(args.get(3).is_some_and(|a| a == "html"));
    let t = Instant::now();
    let out = one(&r, &text);
    let ms = t.elapsed().as_secs_f64() * 1e3;
    match out {
        Ok(svg) => {
            std::fs::write(&args[2], &svg).unwrap();
            eprintln!("ok {ms:.1} ms {} bytes", svg.len());
        }
        Err(e) => eprintln!("err {ms:.1} ms {}", e.chars().take(200).collect::<String>()),
    }
}
