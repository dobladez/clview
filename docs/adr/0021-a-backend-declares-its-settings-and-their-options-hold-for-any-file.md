# ADR-0021: A backend declares its settings, and their options hold for any file

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

[ADR-0015](0015-markdown-and-html-on-dark-paper-the-terminals-own-colour.md) made `--theme` and `d` the
viewer's own, since a key had to be: the pager handled every key itself, asked `Source::dark_form`, and
said "only Markdown and HTML go dark" when the answer was no, naming formats the viewer shouldn't know.
A width for Markdown and HTML ([Spec-0002](../specs/0002-reflow-markdown-and-html-to-a-width.md)) would
have been a second such key and option, and every one after it more of the same in the viewer.
[Spec-0001](../specs/0001-settings-a-file-declares.md) set out to give a backend its keys.

## Decision

- **A backend declares its settings** (`Backend::settings`), as it declares its options: a name, its
  named values and what each says, a number range if it takes numbers, a default, optionally `auto` by
  the terminal's background, its keys (`Cycle` or `Step`), its option, its line in `?`, and its message
  for a file that hasn't it. Which settings a file has is its source's (`Source::settings`).
- **`--theme` and `d` are the documents' backend's `paper` setting.** In a full build nothing a user sees
  changes but `--help`, which lists `--theme` among the documents' options; a build without documents
  has neither.
- **A setting's option is taken for any file**, as ADR-0015 had it: a PDF given `--theme dark` is shown
  as ever, not refused. Unlike `--password`, the value holds for the whole run, a list and the files
  opened from it alike, and a file without the setting has nothing to do with it.
- **The viewer's keys come first**; one it doesn't take is looked up among the settings this build
  declares, then whether what is shown has that setting. So no setting can take a key from the viewer,
  and a test fails if one declares one of its keys, or if a key means two settings.
- **The values travel in every request** (`Request::values`, with dark paper's colour), as the paper
  did: a fixed array of at most eight, so a request stays `Copy`.

## Evidence

No measurement: a choice of where code lives. `cargo test -j 2 --bin clv a_settings_keys_are_its_own_in_every_backend`
checks the keys, `a_settings_option_is_taken_for_a_file_that_hasnt_the_setting` the option, and
`tests/it/wordcraft.rs`'s `the_theme_turns_markdown_and_html_dark_and_nothing_else` that a PDF given
`--theme dark` is shown as printed.

## Alternatives considered

- **Keep `--theme` and `d` the viewer's**, as ADR-0015 decided: each new setting a format has is more
  of the viewer knowing formats.
- **A setting's option a backend's like any other**, refused unless a file's backend takes it: a list of
  Markdown and PDFs would be fine, a PDF alone with `--theme dark` refused, and a shell alias with
  `--theme dark` would refuse every PDF.
- **Keys a backend handles itself**: the viewer would have to hand it keys and their counts, and two
  backends in one list could mean different things by one key.
- **Values held by the source**: a pool's threads would have to share them, and a render under way
  could come back with the wrong one (ADR-0015's reason for the request carrying the paper).

## Consequences

- **Positive:** a backend adds a setting, its keys and its option without the viewer knowing its formats.
- **Accepted:** a setting's option is taken where it does nothing, as `--theme` always was; its keys
  are listed in `?` whatever is shown, and say on a file without it that they don't apply.
- **Follow-ups:** settings that lay a file out again (relayout), with the width
  ([Spec-0002](../specs/0002-reflow-markdown-and-html-to-a-width.md)).
- **Revisit when:** a build declares more than eight settings, or the reader chooses keys.
