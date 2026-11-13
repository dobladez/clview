# Backlog

Ideas and loose ends not yet planned: not promised, and not in [README's Plans](../README.md#plans)
until they are. What `clv` does today is in [usage](usage.md).

## Ask before opening a file past a limit

Today a file past one of [usage's Limits](usage.md#limits) is refused: a PDF of more than 100,000
pages, a Word document over 256 MiB, a Markdown, HTML, SVG or spreadsheet file over 64 MiB, and so
on. In the pager, `clv` could ask instead (`y` to open it all the same), and refuse only when there
is no one to ask, as when printing.

- The contract has no way for a backend to ask yet: a backend refuses with why, or opens.
- A list opens its files as their tiles are reached: a question there needs a design (on the tile,
  or only when the file is opened).
- A PDF's page count is known only once its page tree is parsed, some 2 KB a page, so the question
  would come after that cost: about 3.7 s and 2 GB for a million pages, 21 GB for ten million.
  Asking before the cost is paid needs the count from elsewhere. One option is a cap inside
  pdfcraft's page-tree walk, which clv can't count on. The other is a capped worker process that
  counts the pages first, as a search reads a PDF's text, at the cost of parsing every PDF once
  more.
