# Spec-0001: Let a file declare its settings and their keys

- **Status:** Implementing
- **Date:** 2026-11-07
- **Related:** [Spec-0002](0002-reflow-markdown-and-html-to-a-width.md), which builds on it;
  [ADR-0015](../adr/0015-markdown-and-html-on-dark-paper-the-terminals-own-colour.md) (dark paper,
  whose `--theme` being the viewer's this replaces);
  [What a backend says of itself](../architecture.md#what-a-backend-says-of-itself);
  [Keys](../usage.md#keys), [Documents](../usage.md#documents)

## Summary

The viewer handles every key itself, so a key that applies to one kind of file has to be known to
the viewer: `d` is the pager's, it asks `Source::dark_form()`, and when the answer is no, the viewer
itself says "only Markdown and HTML go dark", naming formats it shouldn't know. This spec lets a
backend declare **settings**, as it already declares its command-line options
(`Backend::options`). A setting has a name, its values (named ones such as `light` and `dark`, a
number range, or both), its default, its keys (one that cycles, or two that step down and up), the
command-line option that sets it, and what it says. The viewer holds the current values for the
whole run, a list and the files opened from it alike, and they travel with each render request, as
the paper does now. It steps them, takes a typed count (`72]`), and lists their keys in `?`. A test
checks that no backend claims a viewer key, and that a key means the same setting in every
backend. As the first setting, `d` and `--theme` move to the documents' backend. What a user sees
of them stays as it is in a full build, `--help` aside, which lists `--theme` among the documents'
options. Settings here only redraw; one that lays a file out again (relayout) comes with its first
user, in [Spec-0002](0002-reflow-markdown-and-html-to-a-width.md).

## Requirements

**Declaring**

- A backend declares its settings (`Backend::settings() -> &'static [Setting]`, `&[]` by default),
  beside `options()`. A `Setting` has:
  - a name;
  - its named values, each with what is said on going to it (`light on dark`);
  - optionally a number range: least, most, step, unit;
  - its default, a named value or a number;
  - optionally `auto`: one named value on a light terminal and another on a dark one;
  - its keys, `Cycle(c)` or `Step(down, up)`;
  - the command-line option that sets it;
  - its line in `?`, and the message for a file that hasn't it.
- A setting's value is a `Value`: `Named` or `Number`.
- Which settings a file has is the source's, not the backend's: `Source::settings()` names them.
  The documents' backend gives paper to Markdown and HTML, never to docx or RTF. `Deferred` names
  them for a file not yet opened, in place of `dark_form`, and `Joined` has the union of its
  parts', as it has `any` of their `dark_form` now.

**Values**

- The viewer holds one value per setting for the whole run: a list and every file opened from it,
  as `d` does now. It starts at the setting's default, or what its option says.
- A setting's option is taken whatever the files, as `--theme` is now: a PDF given `--theme dark` is
  shown as ever, not refused (ADR-0015). `takes_all` takes every setting's option this build
  declares.
- An option's value is checked against the declaration, and a wrong one fails with what it expects,
  as `--theme: expected auto, light or dark, got …` does now. A number out of the range fails too.
  What a value says goes through `printable` before it reaches the terminal.
- `auto` is resolved at start, by the terminal's background, as `Theme::papers` does now, the viewer
  knowing nothing of paper. The terminal is asked for its background only when what is shown has a
  setting with `auto`, or is a list, as it is asked now only when it has a dark form or is a list.
- A render request carries the values in place of `paper`, and the colour of dark paper here (the
  terminal's background, or `DARK_PAPER`); the backend decides from them what it draws. Every
  request carries them: the pager's, printing's, `c`'s (a page copied on the paper it is shown on),
  and those a picture inside a document is drawn with (the defaults, as `Paper::Printed` now).

**Keys**

- A key is the viewer's first. One it doesn't take is looked up among the settings this build
  declares; then, if what is shown hasn't that setting, the setting's message is said and its value
  stays (`d` on a PDF: `only Markdown and HTML go dark`). A key no setting has does nothing, as now.
- A `Cycle` key goes to the next named value, and says what that value says.
- `Step` keys go to the next step down or up: a multiple of the step, so from 73 with a step of 4,
  `]` goes to 76 and `[` to 72. At the least or the most, the value stays and the line says so
  (`width 20, the least`).
- Digits typed before a numbered setting's key set it to that number, clamped to the range (`72]`),
  and are used up, whatever happens. Before a setting without numbers, they are left as they are,
  as `d` leaves them now. Digits before `g` or `Enter` still go to a page.
- `?` lists the keys of the settings this build declares, after the viewer's.
- Steps, numbers and the count are tested in `Nav`'s unit tests, with a setting the test declares:
  nothing ships with them until Spec-0002.

**`d` and `--theme`**

- They become the documents' backend's `paper` setting: named values `light` (`dark on light`) and
  `dark` (`light on dark`), default `auto`, `auto` dark on a dark terminal, key `Cycle('d')`,
  option `--theme`, message `only Markdown and HTML go dark`.
- In a full build, pages, messages, `?`'s line and what `--theme` takes stay as they are. `--help`
  lists `--theme` among the documents' options rather than the viewer's.
- In a build without `word`, there is no `--theme` and no `d`: `--theme` is refused as an option no
  backend built takes, and `d` does nothing.

**Checked by tests**

- No backend's setting key is one of the viewer's keys.
- A key means the same setting in every backend that declares it.
- A setting's option is taken for a file whose backend hasn't the setting (`--theme dark` on a PDF).
- The assertions of the tests of `d` and `--theme` hold as they are. A test that needs the
  documents' backend says so (`#[cfg(feature = "word")]`, as `what_has_no_dark_form_isnt_asked_and_d_says_so`
  in `tests/it/view_pager.rs` does now through its Markdown file), and the pager's unit tests of
  `d` (`pager.rs`, `assert!(nav.turn_paper, …)`) become tests of a setting the test declares.

## Out of scope

- **Relayout**: a setting that changes a file's pages, laying it out again with the new value.
  [Spec-0002](0002-reflow-markdown-and-html-to-a-width.md) brings it, with its first user.
- **A value in the status line.** `d` shows none now; Spec-0002 adds it, for the width.
- **Text values** typed at a prompt: no setting needs one.
- **Keys the user chooses** (a key map, a config file), and settings remembered between runs.
- **New settings for other backends** (dark PDFs, image settings): this spec only moves `d`.
- **A value per file**: a value holds for the whole run, as `d` does now.

## Approach

**Files:**

- `src/source.rs` — `Setting`, `Value`, `Numbers`, `Keys`; `Backend::settings()`,
  `Source::settings()`; `Request.paper` becomes `values: Values` and `dark: [u8; 3]`;
  `Deferred.dark_form` becomes the settings' names; `Paper` and `dark_form()` go. The tests that no
  backend claims a viewer key and that a key means one setting go here, beside
  `the_viewer_names_no_backend`.
- `src/source/wordcraft.rs` — declares `paper`, gives it to Markdown and HTML, and reads it from the
  request where `Document::rendered` matches on `Paper` now. `only_markdown_and_html_have_a_dark_form`
  stays here.
- Every other place a `Request` is made or a `Deferred` given, which must change with it:
  `src/source/joined.rs`, `later.rs`, `picture.rs`, `threads.rs`, `folder.rs`, `image.rs`,
  `pdfcraft.rs` (its requests as well as `deferred`), `svg.rs`, `gridcraft.rs`, and
  `src/view/clip.rs`, `files.rs`. `grep -rn 'Request {\|Paper\|dark_form' src tests` finds them.
- `src/view/mod.rs` — `Theme` and the viewer's `--theme` go. It makes this build's table of
  settings and their starting values from the command line, `auto` by the background, and asks for
  the background when a setting has `auto` (`may_have_dark` now). `takes_all` takes the settings'
  options. `print_pages`' requests carry the values. `the_theme_says_what_pages_start_on` becomes a
  test of `auto`.
- `src/args.rs` and `--help` — a setting's option is parsed, and listed with its backend's options,
  as an `OptionSpec` is.
- `src/view/pager.rs` — `Nav` looks a key it doesn't take up among the settings, as in Keys.
  `turn_paper` becomes "a value changed": the pages sent go and are made again, as for `d` now.
  `?`'s lines (`HELP`) come from the settings; the copier's requests carry the values.
- `tests/it/view_pager.rs`, `tests/it/wordcraft.rs` — the tests of `d` and `--theme`, gated where
  they need the documents' backend.
- Docs: `docs/architecture.md` ([What a backend says of itself](../architecture.md#what-a-backend-says-of-itself));
  `docs/usage.md` (`--theme` and `d` where they are listed, said to come with documents);
  `CHANGELOG.md` (`--help` lists `--theme` with documents); and an ADR replacing ADR-0015's
  `--theme` being the viewer's: a backend declares a setting, and its option is still taken for any
  file. ADR-0015 says it is superseded on that point.

**Patterns to follow:**

- Settings are declared as options are: `Backend::options` and `OptionSpec` (`src/args.rs`).
- What keys do stays in `Nav`, which does no IO, and is unit-tested there, as
  `a_typed_number_goes_to_that_page` is.
- A check over every backend built, as `the_extensions_a_backend_lists_are_the_names_it_opens_by`.

**Decided:**

- Values that only redraw travel in the request; sources hold none of them. (A value that lays a
  file out again is the source's own, given when it is laid out: Spec-0002.)
- `Values` is `Copy`, as `Request` is: a fixed array of at most 8 `(name, Value)`. A test fails if a
  build declares more.
- The viewer's keys come first; then the settings this build declares; then whether what is shown
  has the setting.

## Open questions

None.

## Tasks

- [x] **Settings, `d` and `--theme` the first** — the declaration, values in every request, the
  viewer's table, the options taken for any file, cycling, `?`, the move to the documents' backend,
  the tests of keys and options, the docs and the ADR. One commit, as `d` must keep working.
  *Files:* as in Approach. *Verify:* the gates, each backend alone too;
  `cargo test -j 2 --test it view_pager`, `cargo test -j 2 --test it wordcraft`; manual: in kitty,
  `clv --pager --theme light README.md`, press `d` (`light on dark`) and `d` (`dark on light`);
  `clv --pager --theme dark report.pdf` (any PDF), shown as ever, and `d` says
  `only Markdown and HTML go dark`.
- [x] **Numbers** — `Step` keys, their multiples and ends, the count before a setting's key, used up
  or left, in `Nav`, with a setting the tests declare. *Files:* `src/source.rs`,
  `src/view/pager.rs`. *Verify:* `cargo test -j 2 --bin clv view::pager`.

## Done

- [x] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass, each
  backend alone too (it touches `cfg` attributes).
- [x] Tests added, each seen failing without the change: the key and option tests in `source.rs`
  and `mod.rs`, the step, end and count tests in `pager.rs`.
- [x] The assertions of the tests of `d` and `--theme` hold, gated where they need documents.
- [ ] Manual: in kitty, `d` on Markdown, on a PDF, and in a list mixing them, as before.
- [x] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)), with the
  ADR, and ADR-0015 marked superseded on `--theme`.

## Changelog

- 2026-11-07 — Ready (spec-new).
- 2026-11-07 — Revised after a review: a setting's option taken for any file, keys looked up among
  the build's settings before the file's, defaults and what each value says, counts and steps
  defined, every place a request is made, docs and an ADR.
- 2026-11-07 — Implementing ([notes](0001-settings-a-file-declares-impl-20261107_1610.md)).
