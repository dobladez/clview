# ADR-0017: HTML's style rules apply, as far as wordcraft reads them

- **Status:** Accepted
- **Date:** 2026-11-02

## Context

At `5b5810f`, wordcraft's HTML importer read no stylesheet, and `html::import_on` read a fragment. By
`bbe3547` (0.14.0) `import_on` reads a web page, as Word opens one: Times New Roman on every run, over
any style of the document's, headings at a browser's sizes and no code style. `import_with` reads a
fragment, which takes the document's formatting. Both now apply the simple rules of a page's `style`
elements (`p`, `.note`, `p.note`, `*`) to its paragraphs and runs, and skip the rest: descendants,
pseudo-classes, at-rules.

[ADR-0016](0016-how-markdown-and-html-look.md) said to revisit then, as HTML's own would come first. clv
moves its pin often, and wants as little as it can of reading HTML to be its own.

## Decision

- **Markdown and HTML are read as a fragment**, by `import_with`, so that `styled` still decides how they
  look.
- **What wordcraft reads of a page's `style` elements applies**, as its `style` attributes do, and comes
  before that look. clv has no CSS of its own: what wordcraft learns to read, clv gets by moving its pin.

## Evidence

wordcraft `5b5810f` and `bbe3547`, Apple M3 Max, macOS 26.6.2. `clv FILE --protocol kitty --no-pager
--compression fast --theme light` on a pseudo-terminal of 100 × 60 cells of 10 × 20 pixels, each PNG taken
from the escape codes, and the two builds' pages compared. Pages fetched on 2026-11-02:

| Page | Pages, before and after | Most of a page changed by over 16 levels | What shows |
|---|---|---|---|
| `news.ycombinator.com` | 5, 5 | none | The same |
| `www.w3.org/TR/CSS2/selector.html` | 24, 24 | 0.00% | The same to the eye |
| `jvns.ca/blog/2023/11/06/rebasing-what-can-go-wrong-/` | 9, 9 | 0.17% | The same to the eye |
| `danluu.com/octopress-speedup/` | 5, 5 | 0.44% | The same to the eye |
| `ls(1)` from `mandoc -T html` | 13, 13 | 0.62% | The same to the eye |
| `example.com` | 1, 1 | 0.69% | The same to the eye |
| `herman.bearblog.dev` | 1, 1 | 6.2% | Its own faces and sizes, and fine |
| `bettermotherfuckingwebsite.com` | 2, 2 | 11.5% | Its headings at the size of its text: below |
| `paulgraham.com/greatwork.html` | 26, 34 | 13.9% | No `style` element: the column keeps its `td width="435"` |
| `til.simonwillison.net` | 5, 5 | 15.7% | Its own faces and sizes, and fine |
| A Word document saved by LibreOffice 25.2 as HTML | 69, 69 | 20.6% | Its own faces, and fine |

Markdown has no `style` elements, unless raw HTML brings one. `README.md`, `docs/usage.md` and
`docs/architecture.md` keep their page counts and look the same.

What comes out worse, each as small as it goes:

| Rules | What comes out |
|---|---|
| `pre { background: #272822; color: #f8f8f2 } code { background: #eee } pre code { background: none; color: inherit }` | The last isn't read: pale code on a pale background, hard to read |
| `body { background: #111418; color: #e6e6e6 }` | No page background: light text on a dark patch behind each run, on white paper |
| `body { font-size: 18px }` | On every run, so headings come out at the size of the text |

What rules cost, inside [ADR-0012](0012-bound-what-markdown-and-html-cost-by-their-marks-and-text.md)'s
limits, with `--pages 1`. wordcraft caps them: 10,000 rules, 64 MiB of matching, 512 declarations to an
element.

| Page | `5b5810f` | `bbe3547` |
|---|---|---|
| 450,000 `p`s, no rules | 6.0 s, 2.6 GB | 5.7 s, 2.6 GB |
| The same, under 10,000 `*` rules | 5.7 s, 2.7 GB | 6.9 s, 2.8 GB |
| 200,000 `p`s of 32 classes each, under 10,000 class rules | 2.4 s, 1.3 GB | 5.3 s, 1.3 GB |
| 100,000 `p`s, under 60 MB of `*` rules of 64 KB each | 1.4 s, 0.7 GB | 2.1 s, 0.9 GB |

## Alternatives considered

- **Cutting `style` elements out before the importer**: HTML would look as it did. It takes about 40 lines
  walking HTML as wordcraft's lexer does (comments, `script`, `textarea`), so that a script writing
  `<style>` isn't taken for one, to be kept in step with it each time the pin moves.
- **Reading a web page, `import_on`**: `styled`'s styles lose to what is on each run, so ADR-0016's look
  goes.

## Consequences

- **Positive:** a page with simple CSS looks more as its author meant, and clv holds no CSS of its own.
- **Accepted:** a page that counts on rules wordcraft doesn't read can come out oddly, and now and then
  hard to read. HTML no longer all looks the same.
- **Follow-ups:** the three above are worth raising with wordcraft.
- **Revisit when:** wordcraft reads descendant selectors and a page's background; or pages that come out
  badly turn out to be common.
