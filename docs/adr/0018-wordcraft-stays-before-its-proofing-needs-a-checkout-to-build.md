# ADR-0018: wordcraft stays before its proofing needs a checkout to build

- **Status:** Accepted
- **Date:** 2026-11-02

## Context

clv means to follow wordcraft's `main` closely. The three merges after `bbe3547` (#5202, #5205 and #5208,
2026-11-02) move its proofing out to `craft-proof`, in `storytold/craft-libs`, and its hyphenation
patterns with it, into `storytold/craft-i18n`. `craft-proof`'s build script embeds them from a checkout
named by `CRAFT_I18N_DIR`. With that unset it warns and leaves them out, and Cargo doesn't show a
dependency's warnings. Everything still builds, and every test but a hyphenation one would pass.

A Word document can ask for automatic hyphenation (`w:autoHyphenation`), and clv lays Word documents out
through that crate.

## Decision

The pin stays at `bbe3547`, the last commit with the patterns in wordcraft. `tests/it/wordcraft.rs` fails on
a build where a document that asks for hyphenation draws the same page as one that doesn't, so a later move
can't lose it unseen.

## Evidence

wordcraft `bbe3547` and `866c1f9`, Apple M3 Max, macOS 26.6.2, `CRAFT_I18N_DIR` unset. A paragraph of long
words on a narrow page, in en-US, es-ES and de-DE, with `<w:autoHyphenation/>` in `word/settings.xml` and
without, through `clv FILE --protocol kitty --no-pager`, each PNG taken from the escape codes:

- At `866c1f9` nothing is hyphenated, in any of them. A word too long for the line is cut where it ends, with
  no hyphen (`Donaudampfschifffahrtsgesellschaf` / `tskapitän`).
- At `bbe3547` all three are hyphenated, and their pages are the same as at `5b5810f`.
- `git diff bbe3547 866c1f9 -- crates/doc crates/docx crates/docbin crates/formats crates/layout crates/render crates/fonts crates/geom crates/metafile`
  is empty: nothing else clv builds changes between them.

## Alternatives considered

- **`866c1f9`, without the patterns**: documents that ask for hyphenation lose it, with nothing to say so.
- **`CRAFT_I18N_DIR` in CI and the release workflow**, from a checkout of `storytold/craft-i18n` at the rev
  wordcraft's `craft-i18n.rev` names: the released binaries would hyphenate, but `cargo install --git`, which
  the README offers, can't set it, so the same source would lay out differently. It would also embed spelling
  data clv doesn't use (its size not measured).

## Consequences

- **Positive:** hyphenation stays, and nothing is lost today.
- **Accepted:** the pin can't follow wordcraft past this commit, so a later fix in its importer or layout waits
  until this is settled.
- **Follow-ups:** ask wordcraft for a way to have the patterns without a checkout.
- **Revisit when:** `craft-proof` can embed hyphenation without a checkout, or a fix clv needs lands past
  `bbe3547`.
