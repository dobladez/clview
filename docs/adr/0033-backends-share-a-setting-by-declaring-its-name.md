# ADR-0033: Backends share a setting by declaring its name, and the viewer says of it only what they declare

- **Status:** Accepted
- **Date:** 2026-11-14

## Context

A Mermaid diagram in a document followed its paper
([ADR-0032](0032-a-fenced-block-is-drawn-by-the-backend-that-lists-its-language.md)), but a `.mmd`
shown by itself stayed on white, on a dark terminal too. To follow the paper by itself it needs what
Markdown has: the paper's value, the terminal asked for its background, `d`, and `--theme`. All of
that is the `paper` setting, which the documents' backend declares
([ADR-0021](0021-a-backend-declares-its-settings-and-their-options-hold-for-any-file.md)), and no
backend may name another ([Layers](../architecture.md#layers)).

A first plan, which the backlog and the README's Plans carried, was that `paper` had to move into
the contract (`src/source.rs`) so that a second backend could read it. That misread the contract,
which already gave everything needed:

- `view::settings` merges the settings the backends declare by name, each once, and holds one value
  of each for the whole run;
- a setting's option is taken for any file (`takes_all`);
- every request carries the values (`Request::values`) and dark paper's colour (`Request::dark`);
- a source says which settings it has (`Source::settings`), and the terminal is asked for its
  background when what is shown has one that goes by it.

One thing did stand in the way. What a setting's key says on a file without it was a sentence
written in the backend, `only Markdown and HTML go dark`, and the viewer showed the first
declarer's. With two backends declaring `paper`, either sentence is false for the other's files,
and a backend can't name another's formats to make it true. The same went for the setting's line in
`?`.

## Decision

1. **A backend that wants a setting another has declares it too, under the same name.** Mermaid's
   backend declares its own `paper`: the named values `light` and `dark`, the key `d`, the option
   `--theme`, `auto` by the background, a redraw. A setting's name is the contract's convention, not
   a backend's: sharing it names no one. The viewer then holds one value for both, `d` and `--theme`
   turn whatever follows its paper, and each backend draws its own files by the value as it likes.
   Nothing in the contract changed for this, and nothing in the viewer.
2. **Declared alike, but for the option's help.** The viewer takes a shared setting from the first
   backend to declare it, so the declarations must agree: a test over the backends built
   (`a_setting_is_listed_by_its_backends_formats_and_alike_in_every_backend`) fails if they differ
   in anything but the help of their option, which `--help` lists under each backend in its own
   words.
3. **The viewer says of a setting only what is declared.** A `Format` lists the settings its files
   have, by name (`Format::settings`, as it lists its fences), and a `Setting` gives two phrases to
   be said after formats' names: what its keys do (`help`: `light on dark, or dark on light`) and
   what the files that have it do (`does`: `go dark`). The viewer joins them to the names of the
   formats that list the setting, in the backends' order (`view::having`): `?` lists `Mermaid,
   Markdown and HTML light on dark, or dark on light`, and `d` on a PDF says `only Mermaid, Markdown
   and HTML go dark`. No backend writes a sentence about formats, and the viewer knows none.
4. **Which files have a setting stays the source's to say** (`Source::settings`): a format's list is
   what is said of the format. The documents' backend decides by a document's kind when it opens,
   so a test holds its formats' lists to that (`a_format_lists_the_settings_its_documents_have`).
5. **By itself on dark paper, a diagram is its dark drawing on the paper's colour.** A request whose
   `paper` is `dark` gets Mermaid's dark theme put on `Request::dark`, opaque, in the room the light
   drawing takes, as a document's picture is (ADR-0032) but for what is behind it: a document's
   paper shows through, and a diagram shown alone has no other paper. In the pager the dark renders
   go to a thread of the backend's own, started when the first is wanted, which has merman draw the
   dark SVG: the viewer never waits for it, and the light drawing's pool is as it was.

## Evidence

No measurement: a choice of where code lives. What holds it:

- `cargo test -j 2 --bin clv a_setting_is_listed_by_its_backends_formats_and_alike_in_every_backend`:
  every format lists only what its backend declares, every setting is listed by a format, and a
  shared setting is declared alike.
- `cargo test -j 2 --bin clv the_formats_that_have_a_setting_are_those_their_backends_list`: what
  the viewer says, from fake backends.
- `tests/it/mermaid.rs`'s `a_diagram_shown_by_itself_follows_the_theme` and
  `tests/it/view_pager.rs`'s `d_turns_a_diagram_shown_by_itself_and_a_pdf_says_which_formats_go_dark`:
  `--theme`, `d`, the background asked, and the message, with no line of the viewer or of the
  contract naming Mermaid (`source::tests::the_viewer_names_no_backend`).

## Alternatives considered

- **Move `paper` into the contract** (a field of the request, or a setting the contract defines):
  the plan this replaces. The contract would own one setting among those backends declare, with its
  key, its option and its words, which is where ADR-0015 was and what ADR-0021 undid; and the next
  setting two backends want would move there too.
- **Mermaid's backend reads the documents' setting by name, declaring nothing**: it would read
  `paper` from a request's values and say its diagrams have it. In a build without documents there
  would be no `d` and no `--theme`; in one with them, the setting's words would be the documents'
  backend's, saying nothing of diagrams, unless that backend named Mermaid.
- **The viewer owns a notion of paper**, asking a source whether it has a dark form
  (`Source::dark_form`) for a file shown alone, as a document asks for its pictures: ADR-0015's
  design. The viewer would know again what `d` means and which files it applies to.
- **Each backend keeps its own sentence** for a file without the setting: with two declarers the
  viewer shows one, `only Mermaid diagrams go dark` on a PDF, which is false, and a backend can't
  name the other's formats to make its own true.
- **The dark drawing opened where the pool is told what is wanted** (`Pool::set_queue`), then
  rendered by the dark SVG's own pool: the viewer's thread would wait for merman, up to its 6 s on
  the clock, with the keys dead.

## Consequences

- **Positive:** adding a setting to a backend, or a backend to a setting, touches that backend
  alone: never the contract, never the viewer, and what is said stays true as formats come and go
  with a build's features.
- **Accepted:** a shared setting is written out in each backend that declares it, held alike by a
  test rather than by one definition. `--help` lists `--theme` under each. The formats are named in
  the backends' order, which is the order files are asked in, not the order a reader would choose
  (`Mermaid, Markdown and HTML`). A diagram on a dark terminal is drawn twice by merman, once in
  each theme, since its size is the light drawing's.
- **Follow-ups:** an SVG shown alone following the paper ([backlog](../backlog.md)), which needs a
  dark form of a drawing, not anything of the contract.
- **Revisit when:** two backends want one name to mean different values or keys, or a setting's
  words can't be put as a phrase after its formats' names.

## Update 2026-11-16: what a key says where its setting isn't, shortly and on the whole line

A third backend came to declare the documents' settings, the epub's
([Spec-0012](../specs/0012-show-an-epub-as-a-book.md)), and the sentence a key says on a file
without its setting grew with it: `only Mermaid, Markdown, HTML and EPUB go dark` no longer fitted
its place on a status line of 80 columns, and was cut in its middle. The owner's answer,
2026-11-16:

- **The sentence names two formats, whatever their number**: `this format doesn't go dark (Mermaid,
  Markdown and others do)`. The viewer still puts it together, from the formats that list the
  setting, in the backends' order (`view::some_do`), and from the setting's own phrase
  (`Setting::does`), now said of a file that doesn't and of those that do, so a phrase that reads
  either way: `draw fenced blocks`, where it was `draw their blocks`. Decision 3 stands: the viewer
  names no format of its own.
- **It has the whole status line, for a second**, then the line comes back; a key brings it back
  at once and does what it does (`Nav::notice`, `pager::Moment`).

`?` still lists every format that has a setting, after its keys (`Mermaid, Markdown, HTML and EPUB
light on dark, or dark on light`): its lines are cut in their middle where a window is too narrow
for them, as two of them already were at 80 columns.
