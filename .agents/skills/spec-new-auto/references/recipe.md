---
description: Research and write a new Draft spec unattended, for a person to review afterwards
argument-hint: <a description of the change> (optional: else the next item in README's Plans)
---

**Read `.agents/skills/spec-shared/conventions.md`, `docs/specs/project.md` and `spec-new`'s
recipe (`../../spec-new/references/recipe.md`) first, whole.** This is `spec-new` run
**unattended**: the same template, conventions and writing, the opposite way of working. Below is
only what differs.

You research deeply, write the whole spec yourself, attack your own draft, and write it as a
**`Draft`**. A person reviews it afterwards; you never ask along the way.

## Working principles

- **Don't ask: decide, and say so.** No `AskUserQuestion`, no waiting. Wherever `spec-new` would
  ask, make the call best supported by what you read, and write it in **Assumptions**. The review
  is the approval.
- **Research before writing.** Step 3 stands in for the user's knowledge. Too little research is
  *the* way this skill fails.
- **Attack your own draft.** `spec-new`'s challenge, once per answer, becomes step 7's review of
  the whole, and it can't be skipped.
- **Always `Draft`.** Unreviewed by definition; `Draft` keeps `/spec-implement` off it until a
  person lifts it.
- **Two files.** The new spec and the Index. Nothing else: changes to README's Plans or other
  docs rest on unreviewed reasoning, so you *propose* them.
- **Never let something invented read as settled.**

## Differences from `spec-new`

**Steps 1–2 (what, number, slug, title)**: made the same way, not confirmed. With `$ARGUMENTS`
empty, take README's first Plans bullet no spec covers, and say *which* and *why* in Assumptions.

**Step 3 (research)**: where this skill earns its keep.

- Read the docs `project.md` lists, as `spec-new` does.
- Read **beyond** `spec-new`'s 5–15 files: with no one to fill the gaps, find every place the
  change touches, the patterns they follow, and what ADRs already decided.
- Where the change reaches a terminal's behaviour, read the specification (the kitty graphics
  protocol, the tmux manual), and say what only a terminal could settle as an open question,
  with the terminal and what to watch. **Clean room**: never another viewer's source.
- For a change across several parts (the viewer, a backend, the docs), send out **read-only
  research subagents in parallel**, one each, if the host has them: neighbouring specs and ADRs;
  the conventions to mirror, with files; the `Source`/`Render`/`Pool` traits it goes through; the
  tests it needs and where their kind lives. Each returns a short digest you write from; never
  paste one into the spec.

**Step 4 (writing)**: all four sections outright, no drafts to confirm.

**Step 5 (references)**: checked the same way, settled alone: **correct** a link when its target
is plain, else **drop** it. List each in Assumptions.

**Step 6 (Plans)**: decide which bullets it delivers, but **don't edit the README**: propose it.

**Step 7 (attacking the draft, required)**: before writing, try to refute your own draft; for
anything not small, give it to a fresh subagent whose only job is to refute it. Look for: scope
nothing you read supports; "what happens when…?" left open (a broken file, a pipe, tmux, a
terminal without the protocol, a huge input); a conflict with an ADR, a spec, or a rule in
`CONTRIBUTING.md` (above all: never panic, a backend's library only in its own file, allocations
capped); an Approach naming files that don't exist; the wrong size. Fix what it finds. Whatever
stays unresolved goes into Assumptions as a risk, never dropped.

**Step 8 (writing the file)**: `Draft`, always. Fill the template as `spec-new` does, but **do**
break down `## Tasks` (that is part of the work, sized by the conventions), and give `## Done` real
checks, not placeholders. Changelog: `- <today> — Draft (spec-new-auto, for review).` Rewrite the
Index. Don't commit.

Then print the summary for review, the handoff, easy to scan:

```
Wrote <path> (Draft, for your review).

Spec-NNNN: <title>
Summary:      <one line>
Requirements: <bullets>
Out of scope: <bullets>
Tasks:        <count>: <one line each>

⚠ Assumptions made unattended, to check:
- <a call spec-new would have asked about, and what you chose>
- <a link corrected or dropped in step 5>
- <a risk left from step 7>

Proposed, NOT done:
- Plans: <bullet> is delivered by this | none
- Discoveries: <a question, a later feature, an ADR worth writing>

Next: review it. If it holds → apply the proposals, lift it to Ready, then /spec-implement NNNN.
```

With no assumptions, proposals or discoveries, **say so** (`Assumptions: none`) rather than
leaving the block out: that there are none is something the reviewer needs to know.

## Don't

- Ask the user anything, or wait for input.
- Write `Ready`.
- Change the README's Plans or any doc but the spec and the Index: propose instead.
- Skip step 7, or hide a risk it found.
- Let something invented read as settled.
