---
description: Write a new change spec through a short interview
argument-hint: <a description of the change> (optional)
---

**Read `.agents/skills/spec-shared/conventions.md` and `docs/specs/project.md` first, whole.**
They hold what the skills share: finding specs, numbers and slugs, references, filling the
template, discoveries, constraints. Below is what is this skill's own.

You write a spec **with the user**: a short dialog, then a `Ready` (or `Draft`) file,
`docs/specs/NNNN-<slug>.md`, from `docs/specs/template.md`.

`$ARGUMENTS` is a **description** (maybe empty), not a slug: the slug and title come from it.

## Working principles

- **Ask, don't assume.** More questions than the four sections is fine.
- **Propose, then refine.** From step 3 on, every question comes with a draft answer from the
  docs and code. Silence is not agreement.
- **Challenge each answer, once.** Name a real hole, or say it's sound. Don't rubber-stamp; don't
  push past one nudge.
- **One section at a time**: Summary, Requirements, Out of scope, Approach, each its own turn.

## Workflow

### 1. What to spec

A description in `$ARGUMENTS` → step 2.

None: take the first bullet of README's `## Plans` that no spec covers yet (`spec_list` lists the
specs). `AskUserQuestion`: `Plans: <bullet> (Recommended)` / `Describe another`. On the second,
ask for a sentence or two.

### 2. Number, slug and title

Make them per the conventions. `AskUserQuestion` to confirm `<NNNN-slug>` and `<title>`:
`Use these (Recommended)` / `Edit slug` / `Edit title` / `Edit both`. On an edit, ask for it as
text.

### 3. Read, then look at the code

Read the docs `project.md` lists, in parallel; don't summarise them back. Then look at the code
the description touches, in the places `project.md` names: some 5–15 files, and which patterns
apply, so step 4's Approach proposes real files. Note the ADRs it must respect and the specs it
borders, done or in flight, to link them and keep clear of them.

### 4. The interview, a section at a time

For **each** of Summary, Requirements, Out of scope and Approach, in that order:

**a.** Draft an answer from step 3: a few sentences or bullets.

**b.** One message, the question with the draft as a starting point: *"Here's a draft
**Summary**: does it say it, or would you change something?"* Plain text.

**c.** Wait. Then **challenge the reply once**: a hole, an addition, a conflict with what you
read. *"You said X; what does it do on a terminal without the protocol? Inside tmux? Through a
pipe?"* / *"This goes against ADR-NNNN: on purpose?"* If it's sound, say so and go on.

**d.** The reply after that is final.

What each section is for is in `docs/specs/template.md`. Leave out Approach for a small spec
(Size, in the conventions).

If the change touches none of README's Plans, say so **here**, not later.

### 5. References

Check every link per the conventions. For any that leads nowhere, one `AskUserQuestion` listing
them: `Correct it` / `Note it for later` / `Drop it`.

### 6. Plans

`AskUserQuestion` (multi-select): "Which of README's Plans does this deliver?", an option for each
likely bullet and `None`. When the spec is `Done`, `spec-implement` takes those bullets out; note
them in the spec's Summary or Related so it knows which.

### 7. What doesn't belong here

Look through the answers for what is out of this spec's scope but worth keeping. Propose a home
for each, per the conventions; `AskUserQuestion` for each: `Capture it in <where> now` /
`Note it and move on` / `Skip`. Capture before writing, so the spec can link it.

### 8. Confirm and write

Show a short summary: title, the Plans it delivers, the gist of each section, the open questions.

**Status.** `Ready` by default. `Draft` only when the scope really is unsettled: questions the
user couldn't answer, decisions still to make. If the summary looks complete, write `Ready`
without asking; if shaky, `AskUserQuestion`: `Write it Ready (Recommended)` / `Write it Draft,
scope unsettled`.

Then `AskUserQuestion`: `Write it (Recommended)` / `Revise a section first` / `Cancel`. To
revise, take that section through step 4 again and come back. To cancel, write nothing.

To write, copy `docs/specs/template.md` and fill it: `# Spec-NNNN: <title>`, `Status`, `Date`
(today), `Related` from the checked links, the four sections, and `## Open questions`.

Leave `## Tasks` and `## Done` as the template has them **unless the user described the
breakdown**: an honest skeleton is better than filler. `## Changelog`:
`- <today> — <Status> (spec-new).`

Then rewrite the Index (`--index`), and print what you wrote and what comes next:
`/spec-implement NNNN` for `Ready`; settling the scope and lifting it for `Draft`. Don't commit
unless asked.

## Don't

- Write anything before step 8 confirms.
- Ask the four sections in one message.
- Rubber-stamp: step 4c's challenge is required.
- Fill Tasks or Done with filler to look complete.
