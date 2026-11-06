# ADR-0020: New versions through a Homebrew tap, not a check inside clv

- **Status:** Accepted
- **Date:** 2026-11-06

## Context

clv was installed from a release's archive or with `cargo install`, and neither says when a new
version is out, nor updates one. A release's notes said only where to compare it, until they came
from `CHANGELOG.md` ([Releases](../../CONTRIBUTING.md#releases)). clv makes no network connections:
a document's pictures at a URL show as their alt text
([ADR-0013](0013-a-documents-pictures-are-drawn-by-the-backends.md)). And its binaries aren't signed,
so on macOS one a browser downloaded wants `xattr -d com.apple.quarantine` before it runs.

## Decision

- clv is a formula, `clv`, in a tap of its own owner's, [dobladez/homebrew-tap](https://github.com/dobladez/homebrew-tap),
  named for no one program so that others can join it. `brew install dobladez/tap/clv` installs it
  and `brew upgrade` updates it, on macOS and Linux.
- The formula installs the release's own archives, for each system and processor, by their
  checksums. It builds nothing.
- The release workflow writes it (`.github/homebrew-formula.sh`) and puts it in the tap through
  GitHub's contents API, with a token for that repository alone, in a job after the release is
  published.
- clv itself never asks anything about new versions. GitHub's release notifications (Watch, or the
  releases feed) tell those who installed it otherwise.

## Evidence

Homebrew 7.0.12 on macOS 26.6.2, Apple M3 Max, against the tap with clv 0.3.0's formula:

```sh
brew install dobladez/tap/clv                  # installs, and trusts that formula
xattr "$(readlink -f "$(brew --prefix)/bin/clv")"    # prints nothing: no quarantine
brew test dobladez/tap/clv                     # clv --version says 0.3.0
brew audit --strict --online dobladez/tap/clv  # nothing, once the version scanned from the URL went
```

Homebrew since 6.0.0 refuses to load a formula from a tap not its own unless it is trusted; installing
by the full name trusts that formula alone ([Tap Trust](https://docs.brew.sh/Tap-Trust)).

clv 0.3.0's release built each target from nothing in 9 to 16.5 minutes on GitHub's runners (the
`build` jobs of the Release workflow's run for `v0.3.0`).

## Alternatives considered

- **A check for new versions inside clv.** It would reach those who installed it any way, but it
  brings clv's first network connection, a TLS stack, an answer from a server to distrust, a wait or
  a thread at start, and a question sent each time clv runs, from a program meant to work offline and
  over SSH. Those with a package manager would be told twice.
- **homebrew-core**, for `brew install clv` without a tap: it takes notable projects only, and builds
  from source ([Acceptable Formulae](https://docs.brew.sh/Acceptable-Formulae)).
- **A formula that builds from source**, in the tap: a Rust toolchain on every machine and a build of
  minutes, from git dependencies, where the release has already built the same binary.
- **A cask:** casks are for macOS applications, and Homebrew quarantines what they download, which is
  the step this removes.
- **A tap named for clv** (`homebrew-clv`): one tap per program.
- **A deploy key or a GitHub App** to write to the tap: the first needs git over SSH in the job, the
  second an App to make and keep. A fine-grained token for the one repository is the least to set up.

## Consequences

- **Positive:** one command to install and one to update, on macOS and Linux, with no quarantine
  step; clv still makes no connections.
- **Accepted:** the token expires, and the `homebrew` job fails on the first release after it does,
  the release itself standing ([Releases](../../CONTRIBUTING.md#releases) says what to do). A user who
  wants the short name, `brew install clv`, trusts the formula first.
- **Revisit when:** clv is notable enough for homebrew-core, or its binaries are signed.
