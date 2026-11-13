---
description: Claim a Ready spec and implement it to the end: its status, its notes, a commit per task, then Done
argument-hint: <NNNN or the spec's file name> (optional)
---

**Read `.agents/skills/spec-shared/conventions.md` and `docs/specs/project.md` first, whole.**
They hold what the skills share, and this project's commands and conventions. The lifecycle,
the notes and what to update at `Done` are in `docs/specs/README.md`.

You implement a spec: claim a `Ready` one, take it to `Done`, and keep its notes as you go.

`$ARGUMENTS` may be empty, a number (`0005`), or a file name (`0005-show-avif-pictures`).

## Workflow

### 1. Which spec

With `$ARGUMENTS`: the file it names. **No fuzzy matching**: if none matches exactly, say so and
stop.

Empty: `spec_list ready`. Take the lowest number, unless one delivers an earlier bullet of
README's Plans. `AskUserQuestion`: `Implement Spec-NNNN: <title> (Recommended)` /
`Another Ready spec` / `Cancel`; for another, list up to three more and `Cancel`. With none
`Ready`, say so, stop, and suggest `/spec-new` or lifting a `Draft`.

### 2. Whether it can be

Read the spec. Refuse, saying why, if it is:

- `Draft`: its scope is to settle and lift to `Ready` first.
- `Done` or `Superseded by …`: never.
- `Shelved`: set aside on purpose. It can be lifted back to `Ready` (`/spec-update`), with why
  it is being picked up again.
- without a `- [ ]` under `## Tasks`: its tasks are to write first.

If it's `Implementing`, `AskUserQuestion`: `Continue it` / `Cancel`. To continue, skip step 3's
status change and use its latest `{spec}-impl-*.md`, opening one only if it has none.

### 3. Branch, status and notes

Once, at the start: `today` = `YYYY-MM-DD`, `stamp` = `YYYYMMDD_HHMM`; `spec` = its file name
without `.md`.

1. **Branch.** On `main`, make `spec-NNNN` from it and switch to it; on `spec-NNNN` already, stay.
   On any other branch, ask.
2. `**Status:** Ready` → `**Status:** Implementing`, and rewrite the Index (`--index`).
3. Open `{spec}-impl-{stamp}.md` beside the spec:
   ```md
   # Spec-{NNNN}: implementation notes ({today})

   Notes made while implementing [Spec-{NNNN}]({spec}.md): decisions, changes from its plan,
   surprises, and what a later reader would want.
   ```
4. Add to `## Changelog`: `- {today} — Implementing ([notes]({spec}-impl-{stamp}.md)).`
5. Commit those: `Spec-NNNN: Implementing`.

### 4. The tasks

For each `- [ ]` under `## Tasks`, **in order**:

1. Do the work. Its *Files:* and *Verify:* are on its line. A file not in *Files:* is fine: say
   which and why in the notes.
2. **Each test is seen failing.** For each test the task adds, revert the fix it covers (copy the
   file aside, change it, run the test, copy it back, `cmp` to be sure) and watch the test fail.
   A test that can't fail tests nothing. Say in the notes what you reverted and how it failed.
3. Run *Verify:*. A command must **pass**; a manual check must be **confirmed** by the user,
   asked precisely ("In kitty, open X and press Y: do you see Z?"). No pass, no tick.
4. Before the commit, the quality gates in `project.md`, under a `timeout`: the build that ships,
   every feature on; other feature sets only as `project.md` says.
5. Tick that one box. **Not all at the end.**
6. Write in the notes what was decided, what changed from the plan, what surprised you. "<task>:
   as planned" is fine.
7. **The docs, in the same commit**, by `CONTRIBUTING.md`'s Where a fact goes: a `CHANGELOG.md`
   line under `## Unreleased` for what a user would notice, `docs/usage.md` for the same,
   `docs/architecture.md` for what names the code, and an ADR, linked from the doc that describes
   the behaviour, for a choice between real alternatives.
8. **Commit.** One per task, on `spec-NNNN`: the code, its tests, its docs, the ticked box, the
   notes. Never push. This recipe is the request to commit; don't wait to be asked. If the user
   said not to, say so once and carry on.

When a task can't be done as written (an assumption broken, the spec wrong), **stop**. Note it,
talk it over with the user, then either revise the spec (and say so in `## Changelog`) or pause.

**Raise discoveries when they happen**, per the conventions. The notes are for what concerns this
spec; what reaches further goes to its home *now*, not at the end.

Commit as you go because later tasks touch the same files: five tasks left to the end can't be
split into commits that each build, and the branch becomes one commit nobody can review or
bisect. A change the user asks for along the way (a rename, a reworded message) is a task: add it
to `## Tasks`, and commit it like the others.

### 5. The Done checklist

With every task ticked:

1. Go through `## Done`. Tick each item **only once checked**. If one can't be, stop and deal with
   it; never tick on faith.
2. **The docs say what shipped.** Read `git diff main --stat` and the docs it touched against
   "When a spec is Done" in `docs/specs/README.md`: each user-visible change is in
   `CHANGELOG.md` and `docs/usage.md`, rewritten to say what clv does now rather than added as
   history; what names the code is in `docs/architecture.md`; each choice between alternatives
   has its ADR, in the ADR index. Take out of README's Plans the bullets this delivered.
3. The rest of the gates, if any weren't run since the last commit.

### 6. Discoveries, last time

Before marking it `Done`, ask the user once more whether anything came up that belongs outside
this spec, going briefly through the table in `docs/specs/README.md`. Capture it now; the `Done`
mustn't bury it.

### 7. Done

`AskUserQuestion`: `Mark Spec-NNNN Done (Recommended)` / `Hold: something is pending` /
`Cancel`.

To mark it: `**Status:** Done`, `- {today} — Done.` in `## Changelog`, rewrite the Index,
commit (`Spec-NNNN: Done`). Print the spec, its notes, its commits (`git log --oneline main..`),
and which docs you touched. Don't merge or push unless asked.

To hold: leave it `Implementing` and stop; a later run takes it up again at step 2.

## Don't

- Mark it `Done` with a Tasks or Done box open.
- Pick a spec without the user's confirmation.
- Start the next task with the last one uncommitted.
- Skip the notes. Even a small spec has them, even if all they say is "as planned".
- Refactor or touch code the spec doesn't need. Something worth fixing goes in as a discovery.
