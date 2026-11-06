# clview — instructions for agents

A terminal viewer: pages drawn through the kitty graphics protocol, a pager over them.
`docs/usage.md` says what it does, and `docs/architecture.md` how it works. When a change needs
documenting, `CONTRIBUTING.md`'s "Where a fact goes" says which doc it belongs in.

## Layout
- `src/view/`: the viewer. It knows what it shows only through the traits in `src/source.rs`
  (`Source`, `Render`, `Pool`, `Frame`, `Container`), never a backend; `the_viewer_names_no_backend` checks it.
- `src/source/<backend>.rs`: one backend each, behind a cargo feature, listed in
  `source::BACKENDS`. A backend says whether it opens a file (from its first bytes) and which
  options it takes; the first that opens a file opens it. Options that belong to one kind of file
  (a PDF's `--password`) are the backend's, never the viewer's. `docs/architecture.md` has their
  table: the order they are asked in and why, how each tells its files, and what each keeps.
- `src/source/`'s other files serve them: `dir.rs` (a directory as a `Container`), `joined.rs`,
  `later.rs` and `folder.rs` (a list's tiles, a file not opened until its tile is wanted, and its
  folders), `picture.rs` (the pictures a document refers to, drawn by the backends), `threads.rs` (render threads for a backend with nothing better), `head.rs` and
  `tree.rs` (what a zip's first files say, and the directories an archive's names make).

## Specs
A change larger than a commit goes through a spec in `docs/specs/` (its README has the lifecycle,
`project.md` this project's commands): the `spec-*` skills in `.agents/skills/` write, review,
list and implement them.

@CONTRIBUTING.md
