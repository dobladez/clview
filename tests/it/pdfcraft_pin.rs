//! The pdfcraft pin in Cargo.toml. pdfcraft builds its renderer against patched copies of
//! crates.io crates (its `vendor/`), applied by a [patch.crates-io] that Cargo honours only in
//! the root manifest: ours has to repeat it, at the same commit. Forgetting one still builds,
//! with the stock crate and without pdfcraft's fixes, so this checks the resolved graph.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

const PDFCRAFT: &str = "git+https://github.com/storytold/pdfcraft?rev=";

/// The resolved dependency graph, as `cargo metadata` gives it.
fn metadata() -> Value {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let out = Command::new(cargo).args(["metadata", "--format-version", "1", "--manifest-path"]).arg(&manifest).output().unwrap();
    assert!(out.status.success(), "cargo metadata: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

/// `[patch.crates-io]` in the manifest `path`: each name, and its `path` if it has one.
fn patches(manifest: &Path) -> Vec<(String, Option<String>)> {
    let table: toml::Table = std::fs::read_to_string(manifest).unwrap().parse().unwrap();
    let patches = table.get("patch").and_then(|p| p.get("crates-io")).and_then(toml::Value::as_table).cloned().unwrap_or_default();
    patches.into_iter().map(|(name, entry)| (name, entry.get("path").and_then(toml::Value::as_str).map(String::from))).collect()
}

/// pdfcraft's patched copies, from its root manifest `root`: each crate's name and the version
/// in `vendor/`. A patch replaces only that version: another release of the same crate (pdfcraft's
/// graph holds a stock `vello_common` 0.0.9 beside its patched 0.0.8) is stock in pdfcraft too.
fn vendored(root: &Path) -> Vec<(String, String)> {
    let dir = root.parent().unwrap();
    patches(root)
        .into_iter()
        .filter_map(|(name, path)| Some((name, path.filter(|p| p.starts_with("vendor/"))?)))
        .map(|(name, path)| {
            let manifest: toml::Table = std::fs::read_to_string(dir.join(&path).join("Cargo.toml")).unwrap().parse().unwrap();
            let version = manifest["package"]["version"].as_str().unwrap_or_else(|| panic!("{path}: no literal package.version")).to_string();
            (name, version)
        })
        .collect()
}

#[test]
fn every_crate_pdfcraft_patches_comes_from_the_pinned_commit() {
    let metadata = metadata();
    let packages = metadata["packages"].as_array().unwrap();
    let render = packages.iter().find(|p| p["name"] == "pdfcraft-render").unwrap();
    let pinned = render["source"].as_str().unwrap();
    assert!(pinned.starts_with(PDFCRAFT), "pdfcraft-render from {pinned}");

    // One commit for every pdfcraft crate.
    for p in packages.iter().filter(|p| p["name"].as_str().is_some_and(|n| n.starts_with("pdfcraft-"))) {
        assert_eq!(p["source"].as_str(), Some(pinned), "{} is not at the pinned commit", p["name"]);
    }

    // pdfcraft's root manifest at that commit: <root>/crates/render/Cargo.toml → <root>/Cargo.toml.
    let render_manifest = Path::new(render["manifest_path"].as_str().unwrap());
    let root = render_manifest.ancestors().nth(3).unwrap().join("Cargo.toml");
    let vendored = vendored(&root);
    let source = |name: &str, version: &str| -> Vec<Option<&str>> {
        packages.iter().filter(|p| p["name"] == name && p["version"] == version).map(|p| p["source"].as_str()).collect()
    };
    // Not vacuous: the renderer's parser is among them, and in our graph from the pin.
    let syntax = vendored.iter().find(|(n, _)| n == "hayro-syntax").unwrap_or_else(|| panic!("{vendored:?} read from {}", root.display()));
    assert_eq!(source(&syntax.0, &syntax.1), [Some(pinned)], "hayro-syntax {}", syntax.1);

    // Each one we build comes from that commit too, never from crates.io.
    for (name, version) in &vendored {
        for from in source(name, version) {
            assert_eq!(
                from,
                Some(pinned),
                "{name} {version} is built from {from:?}: add `{name} = {{ git = …, rev = … }}` to [patch.crates-io] at the pinned commit"
            );
        }
    }

    // And we patch nothing pdfcraft no longer does (Cargo would only warn that it went unused).
    for (name, _) in patches(&Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")) {
        assert!(vendored.iter().any(|(n, _)| *n == name), "[patch.crates-io] {name}: pdfcraft no longer patches it at the pinned commit");
    }
}
