//! `gen <dir>`: the hostile diagrams, synthetic, written into `dir`.
use std::fmt::Write as _;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "within") { within(std::path::Path::new(&args[2])); return; }
    if args.get(1).is_some_and(|a| a == "worst") { worst(std::path::Path::new(&args[2])); return; }
    let dir = std::path::PathBuf::from(args.get(1).expect("gen <dir>"));
    let put = |name: &str, text: String| std::fs::write(dir.join(name), text).unwrap();
    // A chain of n nodes.
    for n in [1_000, 3_000, 10_000] {
        let mut s = String::from("flowchart TD\n");
        for i in 0..n { writeln!(s, "  n{i}[Node {i}] --> n{}[Node {}]", i + 1, i + 1).unwrap(); }
        put(&format!("chain-{n}.mmd"), s);
    }
    // n nodes, each to every other: n(n-1)/2 edges.
    for n in [40, 80, 160] {
        let mut s = String::from("flowchart LR\n");
        for i in 0..n { for j in i + 1..n { writeln!(s, "  a{i} --> a{j}").unwrap(); } }
        put(&format!("complete-{n}.mmd"), s);
    }
    // Subgraphs nested d deep, a node in each.
    for d in [50, 200, 1_000] {
        let mut s = String::from("flowchart TD\n");
        for i in 0..d { writeln!(s, "subgraph s{i}\n  x{i} --> y{i}").unwrap(); }
        for _ in 0..d { s.push_str("end\n"); }
        put(&format!("nested-{d}.mmd"), s);
    }
    // One label of n bytes, words of five letters.
    for n in [100_000, 1_000_000] {
        let label = "word ".repeat(n / 5);
        put(&format!("label-{n}.mmd"), format!("flowchart TD\n  A[\"{label}\"] --> B\n"));
    }
    // A sequence of n messages among 20 participants.
    for n in [2_000, 20_000] {
        let mut s = String::from("sequenceDiagram\n");
        for i in 0..n { writeln!(s, "  p{} ->> p{}: message {i}", i % 20, (i * 7 + 3) % 20).unwrap(); }
        put(&format!("sequence-{n}.mmd"), s);
    }
    // 1 MiB of edges among 500 nodes, each pair named once or more.
    let mut s = String::from("flowchart LR\n");
    let mut i = 0u64;
    while s.len() < 1 << 20 { writeln!(s, "  m{} --> m{}", i % 500, (i * 31 + 7) % 500).unwrap(); i += 1; }
    put("edges-1mib.mmd", s);
    // Bytes from a fixed generator, after a flowchart header; and with none.
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut bytes = b"flowchart TD\n".to_vec();
    for _ in 0..200_000 { x ^= x << 13; x ^= x >> 7; x ^= x << 17; bytes.push((x & 0xff) as u8); }
    std::fs::write(dir.join("garbage-flowchart.mmd"), &bytes).unwrap();
    std::fs::write(dir.join("garbage.mmd"), &bytes[13..]).unwrap();
    // Brackets opened and never closed, nested deep.
    put("brackets.mmd", format!("flowchart TD\n  A{}\n", "[(".repeat(100_000)));
}

/// `gen within <dir>`: diagrams made to cost merman the most within its limits (500 edges, its
/// constrained profile's 16,000 items, 1 MiB of text, 128 levels).
pub fn within(dir: &std::path::Path) {
    let put = |name: &str, text: String| std::fs::write(dir.join(name), text).unwrap();
    // 32 nodes each to every other: 496 edges, under Mermaid's 500.
    let mut s = String::from("flowchart LR\n");
    for i in 0..32 { for j in i + 1..32 { writeln!(s, "  a{i} --> a{j}").unwrap(); } }
    put("w-complete-32.mmd", s);
    // 499 edges among subgraphs nested 100 deep, edges crossing them.
    let mut s = String::from("flowchart TD\n");
    for i in 0..100 { writeln!(s, "subgraph s{i}\n  x{i}").unwrap(); }
    for _ in 0..100 { s.push_str("end\n"); }
    for i in 0..499 { writeln!(s, "  x{} --> x{}", i % 100, (i * 37 + 11) % 100).unwrap(); }
    put("w-nested-crossing.mmd", s);
    // 15,000 nodes, no edges.
    let mut s = String::from("flowchart TD\n");
    for i in 0..15_000 { writeln!(s, "  n{i}[Node {i}]").unwrap(); }
    put("w-nodes-15000.mmd", s);
    // A class diagram of 300 classes of 50 members each.
    let mut s = String::from("classDiagram\n");
    for c in 0..300 { writeln!(s, "class C{c} {{").unwrap(); for m in 0..50 { writeln!(s, "  +field{m} int").unwrap(); } s.push_str("}\n"); }
    put("w-class-15000.mmd", s);
    // A gantt over ten thousand years, its ticks daily.
    put("w-gantt-years.mmd", "gantt\n  dateFormat YYYY-MM-DD\n  tickInterval 1day\n  section A\n  Long :a, 0001-01-01, 9999-12-31\n".into());
    // A sequence of 15,000 messages.
    let mut s = String::from("sequenceDiagram\n");
    for i in 0..15_000 { writeln!(s, "  p{} ->> p{}: m{i}", i % 20, (i * 7 + 3) % 20).unwrap(); }
    put("w-sequence-15000.mmd", s);
    // A mindmap 120 deep.
    let mut s = String::from("mindmap\n  root\n");
    for d in 0..120 { writeln!(s, "{}n{d}", "  ".repeat(d + 2)).unwrap(); }
    put("w-mindmap-deep.mmd", s);
    // A pie of 10,000 slices; an xychart of 10,000 points.
    let mut s = String::from("pie\n");
    for i in 0..10_000 { writeln!(s, "  \"s{i}\" : {}", i + 1).unwrap(); }
    put("w-pie-10000.mmd", s);
    let pts: Vec<String> = (0..10_000).map(|i| (i % 97).to_string()).collect();
    put("w-xychart-10000.mmd", format!("xychart-beta\n  x-axis 0 --> 10000\n  y-axis 0 --> 100\n  line [{}]\n", pts.join(", ")));
    // A label just under 1 MiB of one word, which can't wrap.
    put("w-label-unbroken.mmd", format!("flowchart TD\n  A[\"{}\"]\n", "x".repeat(1_000_000)));
}

/// `gen worst <dir>`: 15,000 nodes, each labelled with 45 characters of words that wrap: the
/// costliest diagram found within merman's constrained limits.
pub fn worst(dir: &std::path::Path) {
    let mut s = String::from("flowchart TD\n");
    for i in 0..15_000 { writeln!(s, "  n{i}[\"{} {i}\"]", "lorem ipsum dolor sit amet consectetur adip").unwrap(); }
    std::fs::write(dir.join("w-labelled-15000.mmd"), s).unwrap();
}
