---
name: release
description: Cut a clv release end to end - pick the version, bump it, tag it (annotated), push, watch the Release workflow, check the GitHub release and its notes, the Homebrew tap's formula, and the brew-installed clv. Use when the user asks to release, cut, tag or publish a new version.
disable-model-invocation: true
---

# Release

`CONTRIBUTING.md`'s "Releases" says what a release is and what the workflow does; this is the
order to do it in, and what to check at each step. Stop and tell the user at the first check that
fails: a tag pushed is public, and a release published is never moved (step 9).

`$V` is the version without its `v` (`0.5.0`), `$S` the scratchpad directory.

## 1. Preflight: release only a green `main`

```sh
git switch main && git fetch && git status -sb   # clean, and level with origin/main
gh run list --workflow CI --branch main --limit 5
gh run list --workflow CI --event schedule --limit 2
```

- The latest push run must be green **at `HEAD`** (`headSha`), and so must the latest nightly
  (it builds the feature sets the push run doesn't: no backend, `pdf,svg`). Read why a red one
  failed (`gh api repos/dobladez/clview/actions/jobs/<job>/logs`: `gh run view --log-failed`
  cuts the panic's message off). A real failure is fixed first, in its own commit, and CI run
  again on it, the nightly's sets too (`gh workflow run CI`). A timing test that times out on a
  slow runner, a different one each run, is flaky: rerun it (`gh run rerun <id> --failed`), and
  if the user agrees to release past it, say so in the report.
- `cargo deny check advisories`, if it is installed (CI runs it on every push anyway), and
  GitHub's open alerts, which can come before RustSec has them:
  `gh api 'repos/dobladez/clview/dependabot/alerts?state=open' --jq '.[] | [.security_advisory.severity, .dependency.package.name, .security_advisory.ghsa_id, .security_advisory.summary] | @tsv'`.
  For each, which crate brings it (`cargo tree -i <crate>@<version>`) and whether clv can reach
  what it is about, as `deny.toml`'s `ignore` reasons; tell the user, who decides.
- `## Unreleased` in `CHANGELOG.md` has entries. If it is empty, there is nothing to release.

## 2. Pick the version

Ask the user unless they named it, with a recommendation from the changelog: before 1.0, the minor
(`0.4.0` → `0.5.0`) for anything `Added`, or a key or option changed; the patch (`0.4.1`) for
fixes alone.

## 3. The release commit

- `Cargo.toml`: `version = "$V"`.
- `Cargo.lock`: `cargo metadata --format-version 1 >/dev/null` updates clview's own entry. The
  workflow builds `--locked`, so a stale lock fails it after the tag.
- `CHANGELOG.md`: `## Unreleased` becomes `## $V (<today, YYYY-MM-DD>)`, under a new empty
  `## Unreleased` and a blank line.
- Read the notes as GitHub will show them, and show the user how they start:
  `awk -v version=$V -f .github/release-notes.awk CHANGELOG.md`.
- The gates, under a `timeout` (`CONTRIBUTING.md`'s "Quality gates"); `tests/docs.rs` checks the
  changelog has `$V`.
- Commit `Release $V`, those three files only, with the attribution lines.

## 4. Tag and push

An **annotated** tag, its message the tag's name, as `v0.1.0`–`v0.3.0` are:

```sh
git tag -a v$V -m v$V
git push --atomic origin main v$V
```

`--atomic`: the commit and its tag reach GitHub together, or neither.

## 5. Watch the Release workflow

```sh
gh run list --workflow Release --limit 1          # the run for v$V
gh run watch <id> --exit-status                   # in the background: builds take ~15-30 min
```

Jobs: `build` ×4 (two macOS, two Linux musl), then `publish`, then `homebrew`. A `build` that
fails on the version check means `Cargo.toml` and the tag disagree, or the binary isn't at its
tag. See step 9.

## 6. Check the release

```sh
gh release view v$V --json isDraft,isPrerelease,name,tagName,body,assets \
  --jq '{isDraft, isPrerelease, name, tagName, assets: [.assets[].name]}'
gh release view v$V --json body --jq .body > $S/body.md
awk -v version=$V -f .github/release-notes.awk CHANGELOG.md > $S/notes.md
```

- Not a draft, not a prerelease, named `v$V`.
- Eight assets: `clv-$V-<target>.tar.gz` and its `.sha256` for `aarch64-apple-darwin`,
  `x86_64-apple-darwin`, `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl`.
- The body starts with `notes.md` exactly (`diff` them, the body cut to `notes.md`'s length), and
  then GitHub's "Full Changelog" link from the previous tag.
- `gh release list --limit 1` shows it as Latest.

## 7. Check the Homebrew tap

```sh
gh run view <id> --json jobs --jq '.jobs[] | "\(.name) \(.conclusion)"'   # homebrew: success
gh api repos/dobladez/homebrew-tap/commits --jq '.[0].commit.message'      # "clv $V"
gh api repos/dobladez/homebrew-tap/contents/Formula/clv.rb --jq .content | base64 -d > $S/clv.rb
gh release download v$V --pattern '*.sha256' --dir $S/sums
```

Every `sha256` in `clv.rb` is one of the four sums, and every `url` names `v$V`. If the job failed
with no `HOMEBREW_TAP_TOKEN`, or a 401/403, the token has expired: the release stands; the user
makes a new one (`CONTRIBUTING.md`'s "Releases" says how), then `gh run rerun <id> --failed`.

## 8. Install it, through Homebrew

```sh
brew update                       # brew upgrade alone doesn't fetch the tap
brew list --versions clv          # installed? then: brew upgrade dobladez/tap/clv
"$(brew --prefix)/bin/clv" --version     # exactly "clv $V", no "+…" after it
brew test dobladez/tap/clv
"$(brew --prefix)/bin/clv" --protocol ascii tests/data/flow.mmd | head
```

If it isn't installed, ask before `brew install dobladez/tap/clv`: `$(brew --prefix)/bin` may come
before `~/.cargo/bin` on the user's `PATH`, so it would then be the `clv` they run. Always call it
by its full path here, and say which `clv` `command -v clv` finds.

## 9. When something fails after the tag

- **No release published yet** (a `build` or `publish` failed): fix it on `main`, then move the
  tag: `git tag -d v$V && git push origin :refs/tags/v$V`, and steps 3 (as needed) and 4 again.
- **Release published**: never move or delete its tag; Homebrew and anyone who downloaded it have
  it. Fix forward with the next patch version. A wrong note can be edited on GitHub
  (`gh release edit v$V --notes-file …`), the changelog fixed in a commit.
- **Only `homebrew` failed**: step 7's rerun.

## 10. Report

The release's URL, the version `brew`'s `clv` says, and anything skipped or rerun on the way.
Nothing to bump after it: between releases `build.rs` gives `--version` the commit.
