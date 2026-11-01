//! The docs' links: every relative link in the repository's Markdown leads to a file that exists,
//! and its anchor, if it has one, to a heading there, as GitHub makes them. A test binary of its
//! own, since this needs no backend, and `tests/it` needs one.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The Markdown files under `dir`: not `target`, nor what is hidden.
fn markdown(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy();
        if path.is_dir() {
            if !name.starts_with('.') && name != "target" {
                markdown(&path, found);
            }
        } else if path.extension().is_some_and(|e| e == "md") {
            found.push(path);
        }
    }
}

/// `text`'s lines outside fenced code blocks.
fn prose(text: &str) -> Vec<&str> {
    let mut fenced = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced {
            lines.push(line);
        }
    }
    lines
}

/// The anchor GitHub makes of a heading: in lower case, without punctuation, its spaces hyphens.
fn slug(title: &str) -> String {
    title
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// The anchors of `text`'s headings, a repeated one numbered as GitHub numbers it.
fn anchors(text: &str) -> HashSet<String> {
    let mut found = HashSet::new();
    for line in prose(text) {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        let Some(title) = line.get(hashes..).filter(|t| (1..=6).contains(&hashes) && t.starts_with(' ')) else { continue };
        let slug = slug(title);
        let mut anchor = slug.clone();
        let mut repeat = 0;
        while !found.insert(anchor.clone()) {
            repeat += 1;
            anchor = format!("{slug}-{repeat}");
        }
    }
    found
}

/// `line` without its code spans, where what looks like a link is not one.
fn without_code(line: &str) -> String {
    line.split('`').step_by(2).collect()
}

/// The targets of the links in `text`: what follows each `](`, up to its `)` or its title.
fn links(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in prose(text) {
        let line = without_code(line);
        for (_, rest) in line.match_indices("](").map(|(at, _)| line.split_at(at + 2)) {
            if let Some(target) = rest.split(')').next().and_then(|t| t.split_whitespace().next()) {
                found.push(target.to_owned());
            }
        }
    }
    found
}

/// Why `target`, linked from the file at `from`, leads nowhere, if it does.
fn broken(from: &Path, text: &str, target: &str) -> Option<String> {
    let (path, anchor) = target.split_once('#').unwrap_or((target, ""));
    let file = from.parent().unwrap().join(path);
    if !path.is_empty() && !file.exists() {
        return Some("no such file".to_owned());
    }
    let is_markdown = path.is_empty() || file.extension().is_some_and(|e| e == "md");
    if anchor.is_empty() || !is_markdown {
        return None;
    }
    let there = if path.is_empty() { text.to_owned() } else { std::fs::read_to_string(&file).unwrap() };
    (!anchors(&there).contains(anchor)).then(|| "no such heading".to_owned())
}

#[test]
fn every_relative_link_in_the_docs_leads_somewhere() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    markdown(root, &mut files);
    let (mut read, mut bad) = (0, Vec::new());
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        for target in links(&text) {
            if target.contains("://") || target.starts_with("mailto:") {
                continue;
            }
            read += 1;
            if let Some(why) = broken(file, &text, &target) {
                bad.push(format!("{}: {target}: {why}", file.strip_prefix(root).unwrap().display()));
            }
        }
    }
    assert!(read >= 50, "read {read} links, in {} files", files.len());
    assert!(bad.is_empty(), "links that lead nowhere:\n{}", bad.join("\n"));
}

#[test]
fn an_anchor_is_made_as_github_makes_it() {
    assert_eq!(slug("Copying to the clipboard (`clip.rs`)"), "copying-to-the-clipboard-cliprs");
    assert_eq!(slug("Directly, or inside tmux"), "directly-or-inside-tmux");
    let made = anchors("# Notes\n\n```sh\n# not a heading\n```\n\n## Notes\n");
    assert_eq!(made, HashSet::from(["notes".to_owned(), "notes-1".to_owned()]));
}

#[test]
fn what_looks_like_a_link_in_code_is_not_one() {
    assert_eq!(links("[a](b.md) and `[c](d.md)`\n```\n[e](f.md)\n```\n[g](h.md \"title\")\n"), ["b.md", "h.md"]);
}
