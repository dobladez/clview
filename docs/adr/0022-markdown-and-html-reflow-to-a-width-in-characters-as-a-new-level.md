# ADR-0022: Markdown and HTML reflow to a width in characters, as a new level

- **Status:** Accepted; its US Letter page, and `page` among the widths, superseded by
  [ADR-0023](0023-the-window-is-the-page-of-markdown-and-html.md)
- **Date:** 2026-11-07

## Context

Markdown and HTML give no page size, so clv lays them out on US Letter
([Documents](../usage.md#documents)): some 83 characters of code to a line, however wide the window.
A wide window shows narrow pages with room to spare, a narrow one small text. The reader should be
able to choose the length of a line, which means laying the document out again
([Spec-0002](../specs/0002-reflow-markdown-and-html-to-a-width.md)), where every other setting only
drew the pages again ([ADR-0021](0021-a-backend-declares-its-settings-and-their-options-hold-for-any-file.md)).
A layout is the costliest thing a document does ([ADR-0012](0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md)),
and can't be stopped once started; and the viewer's parts (`Nav`'s page count, the pools, renderers
borrowing their source, `Later`'s files opened at once) all take a source as fixed once opened.

## Decision

- **The width is a number of characters of the code face**, JetBrains Mono at 9.4 pt, 5.64 pt each,
  from 20 to 200, or `page`: Letter, as before, which sits among the numbers at 83, where its 468 pt
  of text come to. The text area is that wide, between the same side margins, and as tall as Letter's;
  a code block, indented 8 pt on each side, holds about 3 characters fewer. The page goes into
  wordcraft's conversion of the fragment (`parse_with`, then `to_doc_in` with the page), where tables
  take their widths from it; `import_with` would size them to the default page.
- **A relayout is the document opened again, as a new level.** The source makes a new source of itself
  at the new width (`Source::relayout`, a `Job` owning the bytes it keeps and the values); the pager
  makes a new `Level` for it, with a new pool and id, at the reader's page, and lets the old one go as
  `back` does. No source, pool or level changes in place, and the old pages stay on screen until the
  new ones are ready.
- **Only the document on screen reflows.** A list's tiles keep the layout they were opened with
  (`--width` among the options a list's files open with); in a list, `[` and `]` say to open a
  document. The value is one for the run, so a document gone into after a change is laid out at the
  new width as it opens.
- **A change while a layout runs waits, and replaces what waits**, rather than stopping the layout
  under way, which can't be stopped: at most one job under way and one set of values waiting, a stale
  result dropped.

## Evidence

The widths: `cargo test -j 2 --bin clv a_width_is_so_many_characters_of_the_code_face_and_page_is_letter`
(37 columns of code fit a line at width 40, 38 don't) and `a_table_is_as_wide_as_the_text_at_any_width`
(a table's grid sums to the text width at `page` as `import_with` gave it, and at 40).

The cost at 20 characters a line, the least: the files ADR-0012 made to cost the most, within the
limits, written again by a script (499,900 marks each, so that they open), under `/usr/bin/time -l`
on a release build, `clv FILE --protocol kitty --pages 1 --width page` and `--width 20`. Apple M3 Max,
macOS 26.6.2, wordcraft at `bbe3547`, with a `cargo clippy -j 2` running alongside, so the times
are a little high:

| File | `--width page` | `--width 20` |
|---|---|---|
| `<ul>`, then the `<li>` sharing out 8 MiB of text (9.5 MiB) | 15.2 s, 5.9 GB | 14.6 s, 5.9 GB |
| `<ul>`, then `<li>a` (2.4 MiB) | 11.2 s, 4.7 GB | 10.8 s, 4.7 GB |
| `<p>` each with a page break (16 MiB) | 6.7 s, 3.9 GB | 6.7 s, 3.9 GB |
| `<p>a` (1.9 MiB) | 6.2 s, 3.5 GB | 6.1 s, 3.5 GB |
| lines in a code block, Markdown (0.95 MiB) | 7.0 s, 4.3 GB | 7.0 s, 4.3 GB |
| 8 MiB of text in one paragraph | 2.7 s, 1.5 GB | 3.2 s, 1.5 GB |

The width changes neither, since importing is what costs: the least stays at 20. The first file in
the pager (`clv --pager --protocol kitty FILE` on a pseudo-terminal, 100 × 60 cells), `]` pressed
once it shows, then `q` once the status line says `width 84`: 21.9 s in all, the layout again 7.5 s
of it, and a peak of 9.5 GB, the old layout (held by its pool) and the new together. It has 13,887
pages at either width, its items a line each.

## Alternatives considered

- **A shared wrapper holding a source that changes** (an `Rc<RefCell<Box<dyn Source>>>` the level
  keeps): every part of the viewer would have to learn that a source's page count and sizes change
  under it, mid-layout and mid-render.
- **A width in points, or a page size**: the reader thinks in columns of text, and a count the status
  line can say (`width 72`) is what the keys step.
- **Reflowing a list's tiles too**: hundreds of files laid out again on each step, for previews.
- **Stopping a layout under way**: wordcraft's layout can't be stopped (ADR-0012); a thread abandoned
  would keep its memory.
- **A page the window's size** (`window`): [Spec-0003](../specs/0003-lay-markdown-and-html-out-to-the-window.md).

## Consequences

- **Positive:** a long line of code or a narrow window gets a width to suit it, by two keys, and the
  rest of the viewer sees only another source opened.
- **Accepted:** laying out again costs what opening did, and while it runs the old layout is held too;
  a document's tile and its pages may be at different widths; a page picked by `--pages` is one as laid
  out, so the keys are refused with it.
- **Follow-ups:** the width kept between runs; a width for other backends that lay text out.
- **Revisit when:** wordcraft's layout can be stopped, or lays out incrementally.
