# Spec-0001: Let a file declare its settings and their keys

- **Status:** Ready
- **Date:** 2026-11-07
- **Related:** [Spec-0002](0002-reflow-markdown-and-html-to-a-width.md), which builds on it;
  [ADR-0015](../adr/0015-markdown-and-html-on-dark-paper-the-terminals-own-colour.md) (dark paper);
  [What a backend says of itself](../architecture.md#what-a-backend-says-of-itself);
  [Keys](../usage.md#keys), [Documents](../usage.md#documents)

## Summary

The viewer handles every key itself, so a key that applies to one kind of file has to be known to
the viewer: `d` is the pager's, it asks `Source::dark_form()`, and when the answer is no, the viewer
itself says "only Markdown and HTML go dark", naming formats it shouldn't know. This spec lets a
backend declare **settings**, as it already declares its command-line options
(`Backend::options`). A setting has a name, its values (named ones such as `light` and `dark`, a
number range, or both), its keys (one that cycles, or two that step down and up), the
command-line option that sets it, and its help. The viewer holds the current values for the whole
run, a list and the files opened from it alike, and they travel with each render request, as the
paper does now. It steps them, takes a typed count (`72]`), and lists their keys in `?`. A test
checks that no backend claims a viewer key, and that a key means the same setting in every
backend. As the first setting, `d` and `--theme` move to the documents' backend, with **no change a
user would notice**. Settings here only redraw; one that lays a file out again (relayout) comes
with its first user, in [Spec-0002](0002-reflow-markdown-and-html-to-a-width.md).

## Requirements

**Declaring**

- A backend declares its settings (`Backend::settings() -> &'static [Setting]`, `&[]` by default),
  beside `options()`. A `Setting` has a name; its named values; optionally a number range (min,
  max, step, unit); its keys, `Cycle(c)` or `Step(down, up)`; the command-line option that sets it;
  its line in `?`; and the message for a file it doesn't apply to.
- A setting's value is a `Value`: `Named` or `Number`.
- A setting may say that `auto` is one named value on a light terminal and another on a dark one.
  The viewer resolves `auto` at start, as `Theme::papers` does now, without knowing what paper is.
- Which settings a file has is the source's, not the backend's: `Source::settings()` names them.
  The documents' backend gives paper to Markdown and HTML, never to docx or RTF. `Deferred` names
  them for a file not yet opened, in place of `dark_form`, and `Joined` has the union of its
  parts', as it has `any` of their `dark_form` now.

**Values**

- The viewer holds one value per setting for the whole run: a list and every file opened from it,
  as `d` does now.
- They start from the command line, checked against the declaration. A wrong value fails as it
  does today (`--theme: expected auto, light or dark, got …`).
- A render request carries the values in place of `paper`, and the colour of dark paper here (the
  terminal's background, or `DARK_PAPER`); the backend decides from them what it draws. So do the
  requests of printing and of `c`, which copies a page on the paper it is shown on.

**Keys**

- The viewer's keys come first. A key it doesn't take is looked up among the settings of what is
  shown.
- A `Cycle` key goes to the next named value.
- `Step` keys go a step down or up the number range, and stop at either end with a message.
- Digits typed before a setting's key set it to that number, clamped to the range (`72]`).
  Digits before `g` or `Enter` still go to a page.
- A setting's key on a file that hasn't that setting gives the setting's message, as `d` does now.
- `?` lists its keys from the settings this build declares.
- Steps, numbers and the count are tested in `Nav`'s unit tests, with a setting the test
  declares: nothing ships with them until Spec-0002.

**`d` and `--theme`**

- They become the documents' backend's `paper` setting: values `light` and `dark`, `auto` by the
  terminal's background, key `Cycle('d')`, option `--theme`.
- Pages, messages (`light on dark`, `dark on light`, `only Markdown and HTML go dark`), `?`'s line
  and `--help`'s words stay as they are. `--help` lists `--theme` among the documents' options
  rather than the viewer's.
- In a build without `word`, there is no `--theme` and no `d`, as there is no `--password` without
  `pdf`.

**Checked by tests**

- No backend's setting key is one of the viewer's keys.
- A key means the same setting in every backend that declares it.
- The tests of `d` and `--theme` pass as they are, wherever they now live.

## Out of scope

- **Relayout**: a setting that changes a file's pages, laying it out again with the new value.
  [Spec-0002](0002-reflow-markdown-and-html-to-a-width.md) brings it, with its first user.
- **A value in the status line.** `d` shows none now, and showing one would be a change a user
  would notice; Spec-0002 adds it, for the width.
- **Text values** typed at a prompt: no setting needs one.
- **Keys the user chooses** (a key map, a config file), and settings remembered between runs.
- **New settings for other backends** (dark PDFs, image settings): this spec only moves `d`.
- **A value per file**: a value holds for the whole run, as `d` does now.

## Approach

**Files:**

- `src/source.rs` — `Setting`, `Value`, `Numbers` and `Keys`; `Backend::settings()` and
  `Source::settings()`; `Request.paper` becomes `values: Values` and `dark: [u8; 3]`;
  `Deferred.dark_form` becomes the settings' names; `Paper` and `dark_form()` go. The tests that
  no backend claims a viewer key and that a key means one setting go here, beside
  `the_viewer_names_no_backend`.
- `src/source/wordcraft.rs` — declares `paper`, gives it to Markdown and HTML, and reads it from the
  request where it matches on `Paper` now (`Document::rendered`).
  `only_markdown_and_html_have_a_dark_form` stays here.
- `src/source/joined.rs`, `src/source/later.rs`, and `deferred` in `gridcraft.rs`, `svg.rs` and
  `pdfcraft.rs` — settings in place of `dark_form`.
- `src/view/mod.rs` — `Theme` and the viewer's `--theme` go. It makes this build's table of
  settings and their starting values from the command line, `auto` by the background;
  `print_pages`' requests carry them. `the_theme_says_what_pages_start_on` becomes a test of
  `auto`.
- `src/args.rs` and `--help` — a setting's option is parsed and listed with its backend's options,
  as an `OptionSpec` is.
- `src/view/pager.rs` — `Nav` looks a key it doesn't take up among the settings: cycle, step, the
  count (`typed`), the message for a file without it. `turn_paper` becomes "a value changed": the
  pages sent go and are made again, as for `d` now. `?`'s lines (`HELP`) come from the settings;
  the copier's requests carry the values.
- `tests/it/view_pager.rs` — the tests of `d`, as they are.
- `docs/architecture.md` — settings in [What a backend says of itself](../architecture.md#what-a-backend-says-of-itself).

**Patterns to follow:**

- Settings are declared as options are: `Backend::options` and `OptionSpec` (`src/args.rs`).
- What keys do stays in `Nav`, which does no IO, and is unit-tested there, as
  `a_typed_number_goes_to_that_page` is.
- A check over every backend built, as `the_extensions_a_backend_lists_are_the_names_it_opens_by`.

**Decided:**

- Values travel in the request; sources hold none, and stay as they were opened.
- `Values` is `Copy`, as `Request` is: a fixed array of at most 8 `(name, Value)`. A test fails if a
  build declares more.
- The viewer's keys come first: a setting's key is looked up only where the viewer takes none.

## Open questions

None.

## Tasks

- [ ] **Settings, `d` and `--theme` the first** — the declaration, values in requests, the viewer's
  table, cycling, `?`, the move to the documents' backend, the tests of keys. One commit, as `d`
  must keep working. *Files:* as in Approach. *Verify:* the gates;
  `cargo test -j 2 --test it view_pager`; manual: in kitty, open a `.md`, press `d` twice (light on
  dark, then dark on light), open a `.pdf` and press `d` (`only Markdown and HTML go dark`).
- [ ] **Numbers** — `Step` keys, their ends, and the count before a setting's key, in `Nav`, with a
  setting the tests declare. *Files:* `src/source.rs`, `src/view/pager.rs`. *Verify:*
  `cargo test -j 2 --lib pager`.

## Done

- [ ] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass, each
  backend alone too (it touches `cfg` attributes).
- [ ] Tests added, each seen failing without the change: the key tests in `source.rs`, the step,
  end and count tests in `pager.rs`.
- [ ] The tests of `d` and `--theme` pass, unchanged but for where they live.
- [ ] Manual: in kitty, `d` on Markdown, on a PDF, and in a list mixing them, as before.
- [ ] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)):
  `docs/architecture.md`; no line in `CHANGELOG.md`, as a user would notice nothing.

## Changelog

- 2026-11-07 — Ready (spec-new).
