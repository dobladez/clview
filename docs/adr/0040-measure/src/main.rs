//! The two Org parsers weighed for ADR-0040: orgize and org-rust-parser, at the versions pinned in
//! `Cargo.toml`.
//!
//! - `measure count <orgize|orgrust> <file>…` prints a line a file, as `reference.el` prints what
//!   Org itself makes of it: its path, then `kind=count` for the kinds it names. orgize is given
//!   the file's own `#+TODO:`, `#+SEQ_TODO:` and `#+TYP_TODO:` keywords, as clv gives them;
//!   org-rust-parser takes none. A parse that panics prints `panic` instead of counts.
//! - `measure compare <reference.tsv> <parser.tsv>` says, kind by kind, how many the reference
//!   found, how many the parser did, and in how many files they agree.
//! - `measure time <orgize|orgrust> <file>…` parses each file and prints the time and the most
//!   heap held, as the allocator counts it, then the total.
//! - `measure cut <orgize|orgrust> <seed> <rounds> <file>…`: the property test, each round a file
//!   cut at two places anywhere (by bytes, brought to a char boundary) and a prefix or a slice
//!   kept, or two files' pieces joined, or a few bytes changed to Org's marks; parsed under
//!   `catch_unwind`. Prints how many rounds panicked, and writes the first few that did to
//!   `cut-panics/`.
//! - `measure hostile <orgize|orgrust> <kind> <n> [stack MiB]` parses one hostile file of size
//!   `n`, on a thread with that stack (8 MiB, the main thread's, by default), and prints its size,
//!   the time, the heap and the nodes made. A stack overflow aborts the process: the shell sees
//!   the signal. Kinds are listed in `hostile()`.
//! - `measure links <file>` prints the path of each link orgize finds in a file, a line each.
//! - `measure tree <file>` prints orgize's tree of a file, to see where it and Org differ.
//! - `measure html <file>…` prints each file's size and lines, and the HTML orgize's own exporter
//!   writes of it, its size and its tags.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use orgize::rowan::ast::AstNode;

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
}

#[global_allocator]
static HEAP: Counted = Counted;

const KINDS: [&str; 34] = [
    "headline",
    "todo",
    "priority",
    "tags",
    "planning",
    "property-drawer",
    "drawer",
    "bracket-link",
    "plain-link",
    "footnote-reference",
    "footnote-definition",
    "src-block",
    "example-block",
    "quote-block",
    "verse-block",
    "center-block",
    "special-block",
    "export-block",
    "comment-block",
    "table",
    "table-row",
    "plain-list",
    "item",
    "checkbox",
    "target",
    "keyword",
    "fixed-width",
    "timestamp",
    "bold",
    "italic",
    "underline",
    "strike-through",
    "verbatim",
    "code",
];

type Counts = BTreeMap<&'static str, usize>;

/// The TODO keywords a file declares, not-done and done, as Org reads `#+TODO:` lines: words
/// before a `|` not done, after it done; with no `|`, the last done.
fn todo_keywords(text: &str) -> (Vec<String>, Vec<String>) {
    let (mut todo, mut done) = (Vec::new(), Vec::new());
    for line in text.lines() {
        let line = line.trim_start();
        let head: String = line.chars().take(12).collect::<String>().to_ascii_uppercase();
        let Some(k) = ["#+TODO:", "#+SEQ_TODO:", "#+TYP_TODO:"].iter().find(|k| head.starts_with(*k)) else {
            continue;
        };
        let rest = &line[k.len()..];
        // `TODO(t)` and `DONE(d!)` name a key and a log: the word before the parenthesis.
        let words: Vec<String> =
            rest.split_whitespace().map(|w| w.split('(').next().unwrap_or(w).to_owned()).collect();
        match words.iter().position(|w| w == "|") {
            Some(bar) => {
                todo.extend(words[..bar].iter().cloned());
                done.extend(words[bar + 1..].iter().cloned());
            }
            None => {
                if let Some((last, first)) = words.split_last() {
                    todo.extend(first.iter().cloned());
                    done.push(last.clone());
                }
            }
        }
    }
    if todo.is_empty() && done.is_empty() { (vec!["TODO".into()], vec!["DONE".into()]) } else { (todo, done) }
}

fn orgize_parse(text: &str) -> orgize::Org {
    let config = orgize::ParseConfig { todo_keywords: todo_keywords(text), ..Default::default() };
    config.parse(text)
}

fn orgize_counts(text: &str) -> Counts {
    use orgize::SyntaxKind::*;
    let org = orgize_parse(text);
    let mut counts: Counts = KINDS.iter().map(|k| (*k, 0)).collect();
    let mut add = |k: &'static str| *counts.get_mut(k).unwrap() += 1;
    for node in org.document().syntax().descendants() {
        let has = |k| node.children_with_tokens().any(|c| c.kind() == k);
        match node.kind() {
            HEADLINE => {
                add("headline");
                if has(HEADLINE_KEYWORD_TODO) || has(HEADLINE_KEYWORD_DONE) {
                    add("todo");
                }
                if has(HEADLINE_PRIORITY) {
                    add("priority");
                }
                if has(HEADLINE_TAGS) {
                    add("tags");
                }
            }
            PLANNING => add("planning"),
            PROPERTY_DRAWER => add("property-drawer"),
            DRAWER => add("drawer"),
            LINK => add("bracket-link"),
            FN_REF => add("footnote-reference"),
            FN_DEF => add("footnote-definition"),
            SOURCE_BLOCK => add("src-block"),
            EXAMPLE_BLOCK => add("example-block"),
            QUOTE_BLOCK => add("quote-block"),
            VERSE_BLOCK => add("verse-block"),
            CENTER_BLOCK => add("center-block"),
            SPECIAL_BLOCK => add("special-block"),
            EXPORT_BLOCK => add("export-block"),
            COMMENT_BLOCK => add("comment-block"),
            ORG_TABLE => add("table"),
            ORG_TABLE_RULE_ROW | ORG_TABLE_STANDARD_ROW => add("table-row"),
            LIST => add("plain-list"),
            LIST_ITEM => {
                add("item");
                if has(LIST_ITEM_CHECK_BOX) {
                    add("checkbox");
                }
            }
            TARGET => add("target"),
            KEYWORD => add("keyword"),
            FIXED_WIDTH => add("fixed-width"),
            TIMESTAMP_ACTIVE | TIMESTAMP_INACTIVE | TIMESTAMP_DIARY => add("timestamp"),
            BOLD => add("bold"),
            ITALIC => add("italic"),
            UNDERLINE => add("underline"),
            STRIKE => add("strike-through"),
            VERBATIM => add("verbatim"),
            CODE => add("code"),
            _ => {}
        }
    }
    counts
}

fn orgrust_counts(text: &str) -> Counts {
    use org_rust_parser::element::{Block, TableRow};
    use org_rust_parser::object::PlainOrRec;
    use org_rust_parser::{Expr, NodeID};
    let parsed = org_rust_parser::parse_org(text);
    let pool = &parsed.pool;
    let mut counts: Counts = KINDS.iter().map(|k| (*k, 0)).collect();
    let mut stack: Vec<NodeID> = vec![pool.root_id()];
    while let Some(id) = stack.pop() {
        let Some(node) = pool.get(id) else { continue };
        let mut add = |k: &'static str| *counts.get_mut(k).unwrap() += 1;
        let mut ids: Vec<NodeID> = Vec::new();
        match &node.obj {
            Expr::Root(c) => ids.extend(c),
            Expr::Paragraph(p) => ids.extend(&p.0),
            Expr::Heading(h) => {
                add("headline");
                if h.keyword.is_some() {
                    add("todo");
                }
                if h.priority.is_some() {
                    add("priority");
                }
                // Its own tags: a `Loc` is one inherited from a heading above it.
                let own = |t: &org_rust_parser::element::Tag<'_>| matches!(t, org_rust_parser::element::Tag::Raw(_));
                if h.tags.as_ref().is_some_and(|t| t.iter().any(own)) {
                    add("tags");
                }
                if h.properties.is_some() {
                    add("property-drawer");
                }
                if let Some((_, c)) = &h.title {
                    ids.extend(c);
                }
                if let Some(c) = &h.children {
                    ids.extend(c);
                }
            }
            Expr::Block(b) => match b {
                Block::Center { contents, .. } => {
                    add("center-block");
                    ids.extend(contents);
                }
                Block::Quote { contents, .. } => {
                    add("quote-block");
                    ids.extend(contents);
                }
                Block::Special { contents, .. } => {
                    add("special-block");
                    ids.extend(contents);
                }
                Block::Comment { .. } => add("comment-block"),
                Block::Example { .. } => add("example-block"),
                Block::Export { .. } => add("export-block"),
                Block::Src { .. } => add("src-block"),
                Block::Verse { .. } => add("verse-block"),
            },
            Expr::RegularLink(l) => {
                add("bracket-link");
                if let Some(c) = &l.description {
                    ids.extend(c);
                }
            }
            Expr::PlainLink(_) => add("plain-link"),
            Expr::Italic(x) => {
                add("italic");
                ids.extend(&x.0);
            }
            Expr::Bold(x) => {
                add("bold");
                ids.extend(&x.0);
            }
            Expr::StrikeThrough(x) => {
                add("strike-through");
                ids.extend(&x.0);
            }
            Expr::Underline(x) => {
                add("underline");
                ids.extend(&x.0);
            }
            Expr::Verbatim(_) => add("verbatim"),
            Expr::Code(_) => add("code"),
            Expr::PlainList(l) => {
                add("plain-list");
                ids.extend(&l.children);
            }
            Expr::Item(i) => {
                add("item");
                if i.check_box.is_some() {
                    add("checkbox");
                }
                ids.extend(&i.children);
            }
            Expr::Table(t) => {
                add("table");
                ids.extend(&t.children);
            }
            Expr::TableRow(r) => {
                add("table-row");
                if let TableRow::Standard(c) = r {
                    ids.extend(c);
                }
            }
            Expr::TableCell(c) => ids.extend(&c.0),
            Expr::Superscript(s) => {
                if let PlainOrRec::Rec(c) = &s.0 {
                    ids.extend(c);
                }
            }
            Expr::Subscript(s) => {
                if let PlainOrRec::Rec(c) = &s.0 {
                    ids.extend(c);
                }
            }
            Expr::Drawer(d) => {
                add("drawer");
                ids.extend(&d.children);
            }
            Expr::FootnoteDef(f) => {
                add("footnote-definition");
                ids.extend(&f.children);
            }
            Expr::FootnoteRef(f) => {
                add("footnote-reference");
                if let Some(c) = &f.children {
                    ids.extend(c);
                }
            }
            Expr::Keyword(_) | Expr::MacroDef(_) => add("keyword"),
            Expr::Target(_) => add("target"),
            _ => {}
        }
        stack.extend(ids);
    }
    counts
}

fn counts(parser: &str, text: &str) -> Counts {
    match parser {
        "orgize" => orgize_counts(text),
        "orgrust" => orgrust_counts(text),
        _ => panic!("orgize or orgrust"),
    }
}

/// Parses `text` with `parser`, building its tree and nothing else; the number of nodes.
fn parse(parser: &str, text: &str) -> usize {
    match parser {
        "orgize" => orgize_parse(text).document().syntax().descendants().count(),
        "orgrust" => org_rust_parser::parse_org(text).pool.inner_vec.len(),
        _ => panic!("orgize or orgrust"),
    }
}

fn read(path: &str) -> String {
    String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned()
}

fn quiet() {
    std::panic::set_hook(Box::new(|_| {}));
}

fn count(parser: &str, files: &[String]) {
    quiet();
    for file in files {
        let text = read(file);
        match catch_unwind(AssertUnwindSafe(|| counts(parser, &text))) {
            Ok(counts) => {
                print!("{file}");
                for (k, v) in counts {
                    print!("\t{k}={v}");
                }
                println!();
            }
            Err(_) => println!("{file}\tpanic"),
        }
    }
}

type Table = BTreeMap<String, Option<BTreeMap<String, usize>>>;

fn table(path: &str) -> Table {
    let mut files = BTreeMap::new();
    for line in std::fs::read_to_string(path).unwrap().lines() {
        let mut fields = line.split('\t');
        let Some(file) = fields.next() else { continue };
        let rest: Vec<&str> = fields.collect();
        let counts = (rest != ["panic"]).then(|| {
            rest.iter().filter_map(|f| f.split_once('=')).map(|(k, v)| (k.to_owned(), v.parse().unwrap())).collect()
        });
        files.insert(file.to_owned(), counts);
    }
    files
}

fn compare(reference: &str, parser: &str) {
    let (reference, parser) = (table(reference), table(parser));
    let panics = parser.values().filter(|c| c.is_none()).count();
    let read: Vec<_> = reference.iter().filter_map(|(f, r)| Some((r.as_ref()?, parser.get(f)?.as_ref()?))).collect();
    println!("{} files in the reference, {} parsed, {panics} panicked", reference.len(), read.len());
    println!("{:<20} {:>7} {:>7} {:>6}  files agreeing", "kind", "Org", "parser", "found");
    for kind in KINDS {
        let (mut want, mut got, mut having, mut agree) = (0, 0, 0, 0);
        for (r, p) in &read {
            let (w, g) = (r.get(kind).copied().unwrap_or(0), p.get(kind).copied().unwrap_or(0));
            want += w;
            got += g;
            if w > 0 || g > 0 {
                having += 1;
                if w == g {
                    agree += 1;
                }
            }
        }
        let found = if want == 0 { "-".to_owned() } else { format!("{:.0}%", 100.0 * got as f64 / want as f64) };
        println!("{kind:<20} {want:>7} {got:>7} {found:>6}  {agree} of {having}");
    }
}

fn time(parser: &str, files: &[String]) {
    let texts: Vec<(String, String)> = files.iter().map(|f| (f.clone(), read(f))).collect();
    quiet();
    let (mut total, mut bytes) = (0.0, 0);
    for (file, text) in &texts {
        let before = NOW.load(Ordering::Relaxed);
        PEAK.store(before, Ordering::Relaxed);
        let start = Instant::now();
        let nodes = catch_unwind(AssertUnwindSafe(|| parse(parser, text)));
        let took = start.elapsed().as_secs_f64();
        let heap = PEAK.load(Ordering::Relaxed) - before;
        total += took;
        bytes += text.len();
        match nodes {
            Ok(nodes) => {
                println!("{file}\t{} bytes\t{:.2} ms\t{} KiB\t{nodes} nodes", text.len(), took * 1e3, heap >> 10)
            }
            Err(_) => println!("{file}\t{} bytes\tpanic", text.len()),
        }
    }
    println!("all {} files, {} KiB: {:.1} ms", texts.len(), bytes >> 10, total * 1e3);
}

/// A small generator of its own, so that a seed gives the same rounds again.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

fn boundary(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn cut(parser: &str, seed: u64, rounds: usize, files: &[String]) {
    let texts: Vec<String> = files.iter().map(|f| read(f)).collect();
    quiet();
    let _ = std::fs::create_dir_all("cut-panics");
    let mut rng = Rng(seed.max(1));
    let mut panics = 0;
    for round in 0..rounds {
        let a = &texts[rng.below(texts.len())];
        let (x, y) = (boundary(a, rng.below(a.len() + 1)), boundary(a, rng.below(a.len() + 1)));
        let input = match rng.below(4) {
            0 => a[..x.max(y)].to_owned(),
            1 => a[x.min(y)..x.max(y)].to_owned(),
            2 => {
                let b = &texts[rng.below(texts.len())];
                let z = boundary(b, rng.below(b.len() + 1));
                format!("{}{}", &a[..x], &b[z..])
            }
            _ => {
                let mut bytes = a.as_bytes().to_vec();
                for _ in 0..1 + rng.below(8) {
                    if !bytes.is_empty() {
                        let at = rng.below(bytes.len());
                        let marks = b"*/_+=~[]<>:#|-\n \\{}^";
                        bytes[at] = marks[rng.below(marks.len())];
                    }
                }
                String::from_utf8_lossy(&bytes).into_owned()
            }
        };
        if catch_unwind(AssertUnwindSafe(|| parse(parser, &input))).is_err() {
            panics += 1;
            if panics <= 20 {
                std::fs::write(format!("cut-panics/{parser}-{round}.org"), &input).unwrap();
            }
        }
    }
    println!("{parser}: {panics} of {rounds} rounds panicked");
}

/// A hostile file of size `n`.
fn hostile(kind: &str, n: usize) -> String {
    match kind {
        // Headlines one level deeper each: `*`, `**`, … n stars.
        "headlines" => (1..=n).map(|i| format!("{} h\n", "*".repeat(i))).collect(),
        // A list nested n deep, each item indented two more.
        "list" => (0..n).map(|i| format!("{}- item\n", "  ".repeat(i))).collect(),
        // Quote blocks nested n deep.
        "quotes" => "#+begin_quote\n".repeat(n) + "x\n" + &"#+end_quote\n".repeat(n),
        // Special blocks nested n deep, each its own name.
        "specials" => {
            (0..n).map(|i| format!("#+begin_b{i}\n")).collect::<String>()
                + "x\n"
                + &(0..n).rev().map(|i| format!("#+end_b{i}\n")).collect::<String>()
        }
        // Drawers nested n deep.
        "drawers-nested" => {
            (0..n).map(|i| format!(":D{i}:\n")).collect::<String>() + "x\n" + &":END:\n".repeat(n)
        }
        // Emphasis nested n deep, `*/_+*/_+…x…+_/*`.
        "emphasis" => {
            let marks = ['*', '/', '_', '+'];
            let open: String = (0..n).map(|i| marks[i % 4]).collect();
            let close: String = open.chars().rev().collect();
            format!("{open}x{close}\n")
        }
        // Emphasis nested n deep, a word between marks: `*a /a _a +a … a+ a_ a/ a*`.
        "emphasis-words" => {
            let marks = ['*', '/', '_', '+'];
            let open: String = (0..n).map(|i| format!("{}a ", marks[i % 4])).collect();
            let close: String = (0..n).rev().map(|i| format!(" a{}", marks[i % 4])).collect();
            format!("{open}x{close}\n")
        }
        // Unclosed emphasis: n marks that open and are never closed, one line.
        "unclosed" => "*a /b _c +d ".repeat(n / 4 + 1) + "\n",
        // Unclosed emphasis across lines of one paragraph.
        "unclosed-lines" => "*a /b _c +d =e ~f\n".repeat(n),
        // Links opened and never closed.
        "brackets" => "[[".repeat(n) + "\n",
        // Links nested in descriptions.
        "links" => "[[a][".repeat(n) + &"]]".repeat(n) + "\n",
        // Footnotes inline, nested n deep.
        "footnotes" => "[fn:: ".repeat(n) + "x" + &"]".repeat(n) + "\n",
        // A table of n rows of 10 cells.
        "table" => "| a | b | c | d | e | f | g | h | i | j |\n".repeat(n),
        // A drawer opened n times and never closed.
        "drawers" => ":DRAWER:\n".repeat(n),
        // Blocks opened n times and never closed.
        "blocks" => "#+begin_src\n".repeat(n),
        // One line of n characters.
        "line" => "a".repeat(n) + "\n",
        // n `\` and `{` that look like entities and LaTeX.
        "latex" => "\\a{".repeat(n) + "\n",
        // Sub- and superscripts nested: `a_{b_{c_{…}}}`.
        "scripts" => "a_{".repeat(n) + &"}".repeat(n) + "\n",
        // Timestamps opened and never closed.
        "timestamps" => "<2024-01-01 ".repeat(n) + "\n",
        // A heading's title of n tags.
        "tags" => format!("* h {}\n", ":t".repeat(n) + ":"),
        // n lines of a paragraph, a realistic body: the size alone.
        "prose" => "Some /words/ and *more*, a [[https://example.com][link]] and =code=.\n".repeat(n),
        _ => panic!("no such kind: {kind}"),
    }
}

fn run_hostile(parser: &str, kind: &str, n: usize, stack: usize) {
    let text = hostile(kind, n);
    let parser = parser.to_owned();
    let kind = kind.to_owned();
    let thread = std::thread::Builder::new().stack_size(stack << 20).spawn(move || {
        quiet();
        let before = NOW.load(Ordering::Relaxed);
        PEAK.store(before, Ordering::Relaxed);
        let start = Instant::now();
        let nodes = catch_unwind(AssertUnwindSafe(|| parse(&parser, &text)));
        let took = start.elapsed().as_secs_f64();
        let heap = PEAK.load(Ordering::Relaxed) - before;
        let size = text.len() >> 10;
        match nodes {
            Ok(nodes) => println!("{parser}\t{kind}\t{n}\t{size} KiB\t{:.1} ms\t{} MiB\t{nodes} nodes", took * 1e3, heap >> 20),
            Err(_) => println!("{parser}\t{kind}\t{n}\t{size} KiB\t{:.1} ms\tpanic", took * 1e3),
        }
    });
    thread.unwrap().join().unwrap();
}

fn html(files: &[String]) {
    quiet();
    let (mut org, mut out, mut tags_total) = (0, 0, 0);
    for file in files {
        let text = read(file);
        let Ok(html) = catch_unwind(AssertUnwindSafe(|| orgize_parse(&text).to_html())) else { continue };
        let tags = html.matches('<').count();
        let lines = text.lines().count();
        let ratio = html.len() as f64 / text.len().max(1) as f64;
        println!("{file}\t{} bytes\t{lines} lines\t{} bytes of HTML\t{tags} tags\t{ratio:.2}x", text.len(), html.len());
        org += text.len();
        out += html.len();
        tags_total += tags;
    }
    println!("all: {org} bytes of Org, {out} of HTML ({:.2}x), {tags_total} tags", out as f64 / org.max(1) as f64);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("count") => count(&args[1], &args[2..]),
        Some("compare") => compare(&args[1], &args[2]),
        Some("time") => time(&args[1], &args[2..]),
        Some("cut") => cut(&args[1], args[2].parse().unwrap(), args[3].parse().unwrap(), &args[4..]),
        Some("hostile") => {
            run_hostile(&args[1], &args[2], args[3].parse().unwrap(), args.get(4).map_or(8, |s| s.parse().unwrap()))
        }
        Some("html") => html(&args[1..]),
        Some("links") => {
            let org = orgize_parse(&read(&args[1]));
            for node in org.document().syntax().descendants() {
                if let Some(link) = orgize::ast::Link::cast(node) {
                    println!("{}", link.path());
                }
            }
        }
        Some("tree") => println!("{:#?}", orgize_parse(&read(&args[1])).document().syntax()),
        _ => eprintln!("measure <count|compare|time|cut|hostile|html> …: see src/main.rs"),
    }
}
