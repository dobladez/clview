# Spec-NNNN: Short imperative title

- **Status:** Ready
- **Date:** YYYY-MM-DD
- **Related:** (ADRs, specs, doc sections, by link; the README Plans bullet it delivers)

## Summary

The problem this slice solves and the gist of how, in a paragraph that stands alone for a reader
who hasn't read what it links.

## Requirements

What this slice delivers, in enough detail that the implementer doesn't guess: prose, bullets or
subsections, as the content needs. Each is something to point at when the spec is `Done`.

- What clv does, or a property it keeps.

## Out of scope

What could reasonably be in scope and isn't.

- An adjacent feature left out.
- A refactor put off.

## Approach

*Left out for a small spec.* Where this goes in the code, what it mirrors, what is already
decided, so an implementer knows where to start and what to match.

**Files:**

- `src/...` — what it does, and why it matters here.

**Patterns to follow:**

- A convention to match, with a file that shows it.

**Decided:**

- A choice already made, with its ADR if it has one.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- 🔴 **Question** — …

## Tasks

One checkbox each: a session's work, and a commit that builds and passes on its own.

- [ ] **Noun phrase** — what gets built. *Files:* `src/...`, `tests/it/...`. *Verify:* `cargo test -j 2 ...`, or `Manual: in kitty, press X, see Y`.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [ ] Tests added, each seen failing without the change: name them.
- [ ] Manual: …
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- YYYY-MM-DD — Ready.
