# ADR-0016: How Markdown and HTML look

- **Status:** Accepted
- **Date:** 2026-11-02

## Context

What wordcraft's importer gives is a word processor's defaults, meant to be changed: blue headings that
hardly differ in size, a quote centred and in italics, code in Courier with nothing to mark a block, links
the colour of the text, tables in black grid lines, and no room below a table. A Word document has
styles of its own, chosen by whoever wrote it. Markdown has none, and HTML's stylesheets aren't read
([Documents](../usage.md#documents)), so this look is all most such files will ever have.

## Decision

- **Set after importing, by `styled`, on the document**: in its styles where those can say it, and on its
  paragraphs and runs where it depends on what is beside one or in it. Only Markdown and HTML go through it.
  What HTML's inline styles say stays, being on the text itself.
- **One set of colours, for white paper.** wordcraft's dark page ([ADR-0015](0015-markdown-and-html-on-dark-paper-the-terminals-own-colour.md))
  turns each, and they are chosen so that what they turn into suits dark paper. Nothing is kept per paper,
  and `d` needs no new layout.
- **Faces that come with wordcraft, Source Sans 3 and JetBrains Mono**, so a page looks the same on any
  machine, rather than as Aptos's substitutes fall there. Source Sans has a true italic and a bold, which
  Inter, the other sans wordcraft brings, doesn't.
- **Text at 11 points**, its headings and spacing in step: 12 looked a little large in a real terminal.
- **Code among text is its face alone**, at 88% of the text's size. No chip behind it.
- **A code block is one box as wide as the text**, each line a paragraph, joined as Word joins paragraphs
  with the same borders: its wash, edges of the same colour for room, and indents of that room.
  **9.4 points, at which 80 columns fit on a line.**
- **US Letter, as before, with 54 points above and below rather than 72.** The sides stay at 72: the
  importer caps a picture at 468 points, the width that leaves, and a line of text holds about 95 characters.
- **Alerts (`> [!NOTE]`) say what they are**, in a colour of their own, on the line above their text in an
  ordinary quote.
- **A task's box takes its bullet's place**, in a bulleted list; a numbered task keeps its number.

## Evidence

wordcraft at `5b5810f`, pulldown-cmark 0.13.6, Apple M3 Max, macOS 26.6.2. Pages rendered with
`clv FILE --protocol kitty --no-pager --zoom 1 --theme light|dark` on a window from script(1) and
`stty rows 75 cols 200`, each PNG taken from the escape codes and looked at, among them a sampler of every
part and real READMEs (wordcraft, highlight.js, biff, `aws-icons-for-plantuml`'s `AWSSymbols.md`).

What each colour becomes, by `RenderOptions::ink` on `dark_paper` `0x1E`, and its contrast:

| Colour | On white | Contrast | On dark | Contrast |
|---|---|---|---|---|
| Text | `#1F2328` | 15.8 | `#DBDFE3` | 12.4 |
| Quiet text | `#59636E` | 6.1 | `#9DA6B0` | 6.8 |
| Link, and Note | `#0969DA` | 5.2 | `#3E93F7` | 5.3 |
| Tip | `#1A7F37` | 5.1 | `#8EE8A8` | 11.3 |
| Warning | `#9A6700` | 4.9 | `#FFD277` | 11.7 |
| Hairline | `#D0D7DE` | | `#3B4147` | |
| Wash | `#F6F8FA` | | `#222425` | |

A colour of middling lightness barely moves when inverted: `#8250DF`, a purple fine on white, came out at
`#663AB8`, 2.3 to 1 on dark paper, and `#CF222E` at 4.2. Important and Caution are darker on white for that
reason (`#4F2D94`, `#A40E26`). `what_is_coloured_can_be_read_on_either_paper` holds them all to 4.5.

A shading behind code among text, measured by looking at it at 1.87 pixels a point: wordcraft fills each
character's background as a rectangle of its own (`character_backgrounds`), which meet on parts of a pixel,
so a seam shows between each two letters. On dark paper the fill is about 40 levels from the page
(`ink_bg`, which takes no paper into account), and the seams are plain to see. The same happens down a code
block, where each line is a fill, but its wash is 9 levels from the page, and the seams, measured down a column at
three window sizes, are at most 2 levels on white and 1 on dark, which don't show.

Keeping a table's rows whole (`cant_split`): a row longer than a page isn't split then. A row of 3,000
words left a page holding only a heading, one holding only the table's header, and one holding the part of
the row that fit: the rest of it was gone. So rows split, as they did.

Cost, on `AWSSymbols.md` with its pictures, as printed, the median of five runs: 0.67 s and 116 MB without
this, 0.69 s and 111 MB with. Pages, before and after: `docs/usage.md` 19 and 18, `docs/architecture.md` 19
and 17, wordcraft's README 14 and 14, `AWSSymbols.md` 114 and 111.

## Alternatives considered

- **wordcraft's styles as they are**: see Context.
- **Writing it as CSS for the importer**: it reads no stylesheet, and `style` attributes say nothing of
  margins, padding or borders, except that a table has one.
- **A change in wordcraft**: these are clv's choices for pages on a screen, not defaults for a word
  processor's new document.
- **The system's faces**: wordcraft's default, Aptos, isn't on most machines, so it falls to whatever
  substitute is there, and a page's look and length would differ from machine to machine.
- **A shading behind code among text**, GitHub's way: see Evidence. It would also fill the whole height of
  a line, there being no other height to give it.
- **A fine box around code among text** (`CharProps::border`, which has no seams): busy on a page of prose
  with much code in it, and heavy in a heading.
- **Code among text in a colour**: noisy, as most of a key table or a manual is code.
- **Colouring an alert's bar by its kind**: a style can't, and paragraphs' borders must match to join.
- **A page narrower than Letter**, for larger text in the same window: pictures would be capped narrower,
  and a page of a fixed size still wouldn't follow the window.
- **Syntax highlighting in code blocks**: another decision, with a dependency to choose.

## Consequences

- **Positive:** a README looks like a README, the same on any machine, on either paper.
- **Accepted:**
  - Seams of a level or two down a code block.
  - Code among text is marked less strongly than by a chip.
  - The bar beside a quote is one, however deep the quote is nested, as the importer keeps no depth.
  - In a loose list, items are as close as in a tight one.
- **Revisit when:** wordcraft draws a run's background as one rectangle: a chip would then have no seams, and
  it wants a dark colour of its own too. Or when its importer reads a stylesheet: then HTML's own would come
  first.

## Update

**2026-11-02.** wordcraft now reads the simple rules of `style` elements, so the second "Revisit when" has
come: see [ADR-0017](0017-htmls-style-rules-apply-as-far-as-wordcraft-reads-them.md). "HTML's stylesheets aren't read" above, and the alternative that rests
on it, are as things stood at `5b5810f`.
