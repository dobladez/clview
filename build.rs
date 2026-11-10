//! The version `--help` and `--version` show, handed to the code as `CLV_VERSION`.
//!
//! A release, built at its tag with nothing changed, shows `Cargo.toml`'s version as it is. Any
//! other build from a checkout of clview's repository also says which commit, as SemVer build
//! metadata: `0.3.0+50.g3f677cb` is 50 commits past `v0.3.0`, at commit `3f677cb`;
//! `0.4.0+g3f677cb` a commit whose version no tag has yet, or past no tag the checkout has; and
//! `.dirty` at the end, tracked files changed since that commit. Built without git, or from
//! files that are not that checkout, the version alone.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let pkg = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let root = std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from).unwrap_or_default();
    // What a build is made of, so a change to it asks again whether the tree is dirty; the rest
    // (docs, tests) doesn't change the binary.
    for path in ["build.rs", "Cargo.toml", "Cargo.lock", "src", "assets"] {
        println!("cargo::rerun-if-changed={path}");
    }
    let shown = match checkout(&root) {
        Some(git) => {
            git.watch();
            version(&pkg, git.describe().as_deref(), git.dirty())
        }
        None => pkg,
    };
    println!("cargo::rustc-env=CLV_VERSION={shown}");
}

/// The git repository clview's files are the working tree of.
struct Checkout {
    root: PathBuf,
    /// Its `HEAD` and index: a worktree's own.
    git_dir: PathBuf,
    /// Its refs: shared by its worktrees.
    common_dir: PathBuf,
}

/// The checkout `root` is the top of, or none: no git, or `root` inside some other repository
/// (unpacked in someone's home under git, say), whose commits are not clview's.
fn checkout(root: &Path) -> Option<Checkout> {
    let out = git(root, &["rev-parse", "--path-format=absolute", "--show-toplevel", "--git-dir", "--git-common-dir"])?;
    let mut lines = out.lines().map(PathBuf::from);
    let (top, git_dir, common_dir) = (lines.next()?, lines.next()?, lines.next()?);
    let same = top.canonicalize().ok()? == root.canonicalize().ok()?;
    same.then(|| Checkout { root: root.to_owned(), git_dir, common_dir })
}

impl Checkout {
    /// Asks cargo to run this again when a commit is made or checked out, a tag added, or a file
    /// staged. Only paths that exist: cargo runs it on every build while one is missing.
    fn watch(&self) {
        let paths = [
            self.git_dir.join("HEAD"),
            self.git_dir.join("index"),
            self.common_dir.join("refs/heads"),
            self.common_dir.join("refs/tags"),
            self.common_dir.join("packed-refs"),
        ];
        for path in paths.iter().filter(|path| path.exists()) {
            if let Some(path) = path.to_str() {
                println!("cargo::rerun-if-changed={path}");
            }
        }
    }

    /// `git describe`: the release tag last reached, the commits since and this one's hash
    /// (`v0.3.0-50-g3f677cb`), or the hash alone where no tag is reached.
    fn describe(&self) -> Option<String> {
        git(&self.root, &["describe", "--tags", "--long", "--always", "--match", "v[0-9]*"])
    }

    /// Whether tracked files differ from the commit. Asked of `git status`, which with
    /// `GIT_OPTIONAL_LOCKS=0` (in `git`) doesn't write the index it is watched by, so it doesn't
    /// make cargo run this again on every build.
    fn dirty(&self) -> bool {
        git(&self.root, &["status", "--porcelain", "--untracked-files=no"]).is_some_and(|changed| !changed.trim().is_empty())
    }
}

/// What git prints for `args`, run in `dir`, or none if it can't be run or fails.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).env("GIT_OPTIONAL_LOCKS", "0").output().ok()?;
    out.status.success().then_some(())?;
    String::from_utf8(out.stdout).ok()
}

/// The version shown for `pkg`, `Cargo.toml`'s, built at the commit `git describe` says, `dirty`
/// or not. Only `pkg`, digits and hex digits reach it from git, so nothing a tag's name holds
/// reaches the terminal.
fn version(pkg: &str, described: Option<&str>, dirty: bool) -> String {
    let Some(described) = described.map(str::trim) else { return pkg.to_owned() };
    let hex = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit());
    let mut parts = described.rsplitn(3, '-');
    // The commits since `pkg`'s tag, when that is the tag reached, and the hash.
    let (since, hash) = match (parts.next(), parts.next(), parts.next()) {
        (Some(hash), Some(count), Some(tag)) => {
            let Some(hash) = hash.strip_prefix('g').filter(|h| hex(h)) else { return pkg.to_owned() };
            let Ok(count) = count.parse::<u32>() else { return pkg.to_owned() };
            (tag.strip_prefix('v').is_some_and(|tagged| tagged == pkg).then_some(count), hash)
        }
        (Some(hash), None, None) if hex(hash) => (None, hash),
        _ => return pkg.to_owned(),
    };
    let dirty = if dirty { ".dirty" } else { "" };
    match since {
        Some(0) if dirty.is_empty() => pkg.to_owned(),
        Some(count) => format!("{pkg}+{count}.g{hash}{dirty}"),
        None => format!("{pkg}+g{hash}{dirty}"),
    }
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn a_release_at_its_tag_is_its_version_alone() {
        assert_eq!(version("0.3.0", Some("v0.3.0-0-g3f677cb\n"), false), "0.3.0");
        assert_eq!(version("0.3.0", None, false), "0.3.0");
        assert_eq!(version("0.3.0", None, true), "0.3.0");
    }

    #[test]
    fn past_the_tag_says_how_far_and_at_which_commit() {
        assert_eq!(version("0.3.0", Some("v0.3.0-50-g3f677cb"), false), "0.3.0+50.g3f677cb");
        assert_eq!(version("0.3.0", Some("v0.3.0-50-g3f677cb"), true), "0.3.0+50.g3f677cb.dirty");
        assert_eq!(version("0.3.0", Some("v0.3.0-0-g3f677cb"), true), "0.3.0+0.g3f677cb.dirty");
        assert_eq!(version("1.0.0-rc.1", Some("v1.0.0-rc.1-3-gabc1234"), false), "1.0.0-rc.1+3.gabc1234");
    }

    #[test]
    fn without_its_versions_tag_only_the_commit() {
        // Bumped in Cargo.toml, not tagged yet: commits since the old tag would mislead.
        assert_eq!(version("0.4.0", Some("v0.3.0-51-g3f677cb"), false), "0.4.0+g3f677cb");
        // A shallow clone with no tags.
        assert_eq!(version("0.3.0", Some("3f677cb"), true), "0.3.0+g3f677cb.dirty");
    }

    #[test]
    fn what_git_says_unexpectedly_is_left_out() {
        for described in ["", "v0.3.0-50", "v0.3.0-x-g3f677cb", "v0.3.0-50-3f677cb", "v0.3.0-50-g\x1b[2J", "\x1b[2J"] {
            assert_eq!(version("0.3.0", Some(described), true), "0.3.0", "{described:?}");
        }
    }
}
