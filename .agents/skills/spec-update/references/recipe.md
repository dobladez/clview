---
description: Review and tighten a spec not yet Done (its structure, its match with the code, its status), without implementing it
argument-hint: <NNNN or the spec's file name> (optional)
---

**Read `.agents/skills/spec-shared/conventions.md` and `docs/specs/project.md` first, whole.**
The lifecycle is in `docs/specs/README.md`.

You revise a spec: bring it to `docs/specs/template.md`'s structure, tighten its scope, set it
straight with the code, or change its status. **You write no code**: that is `/spec-implement`'s,
once it's `Ready`.

`$ARGUMENTS` may be empty, a number, or a file name.

## Workflow

### 1. Which spec

With `$ARGUMENTS`: the file it names. **No fuzzy matching**: say so and stop if none matches.

Empty: the `Draft`, `Ready` and `Implementing` specs (`spec_list`). `AskUserQuestion` with up to
four, lowest number first, each with its status. None → say so, stop, suggest `/spec-new`.

### 2. Whether it can be

Refuse if it's `Done` or `Superseded by …`: those are records, not rewritten; a real change is a
**new** spec, through `/spec-new`, linking the old. A `Shelved` spec can be: it was set aside,
not settled for ever, so it can be updated or lifted back to `Draft` or `Ready`. Refuse too if it
has no header.

### 3. Read

In parallel: the spec; `docs/specs/template.md`; its latest implementation notes, if any; README's
Plans (is what it delivers still there?); the ADRs and specs it links.

### 4. The audit

Work it all out before showing anything.

**Structure.** Its headings against the template's: a section missing, one under another name,
content under the wrong heading. Note it; rewrite nothing yet.

**Against the code.** For one `Implementing` (or `Ready` that the user says is under way), check
what it claims. For each `- [x]` task, read or grep what it says it changed: is that function,
file, test or behaviour there? For each requirement naming a function, file, option or key,
check it. Each `- [ ]` is pending; assume nothing. The point is the drift: a box ticked that isn't
true, or a spec `Implementing` that hasn't moved. `git log main..spec-NNNN` shows what its branch
has.

**References.** Every link in `**Related:**`, `## Open questions` and the text, per the
conventions.

**Status.** `Draft` with its scope settled and no 🔴 → could be `Ready`. `Ready` with real open
questions or contradictions → could go back to `Draft`. `Implementing` with nothing since its
notes' last entry → stalled.

### 5. What it found

A short plain-text audit, no questions yet. Number and status first, then each lens, **with
files and lines**:

```
Spec-NNNN: <title> (Status: Implementing)

Structure: as the template has it.

Against the code:
  - Task 1, decode AVIF: done, src/source/image.rs:212.
  - Task 2, colour profiles: NOT done, nothing reads an ICC profile.
  - Task 3, its limits: PARTIAL. src/source/image.rs:240 caps the width, but the height at :244
    is still taken from the file as it is.

References: all lead somewhere.

Status: Implementing is right; task 2 hasn't started.

Suggested:
  - Leave its status.
```

A `PARTIAL` with the line that shows it is worth more than a bare NOT done. If the audit is
clean, say so and stop.

### 6. What to change

`AskUserQuestion` (multi-select) over what step 5 found. Where it applies: bring it to the
template · set the Tasks straight with the code · tighten Requirements or Out of scope (as text) ·
fix each dead link (one option each) · lift `Draft → Ready` (scope settled, no 🔴) · back
`Ready → Draft` · pause `Implementing → Ready` · or only a Changelog line.

**Never offer `→ Done`**: `/spec-implement` owns it, since it goes through the Done checklist and
the docs.

At most one round of questions on answers given as text.

### 7. What doesn't belong here

What the audit found outside this spec gets a home, per the conventions, captured **before** you
edit, so the spec can link it.

### 8. Confirm and apply

Show the edits, the status change, and the Changelog line. `AskUserQuestion`: `Apply
(Recommended)` / `Revise one first` / `Cancel`. Revising goes back to step 6; cancelling writes
nothing.

To apply: edit the spec, **keeping all its content**: restructuring moves text under new
headings, never drops it; content the template has no place for stays, after `## Done`. A status
change goes on its `**Status:**` line, then rewrite the Index. Add
`- {today} — {one line} (spec-update).` to `## Changelog`. Print what changed and what comes next.
Don't commit unless asked.

## Don't

- Edit code. Say what code needs changing, and stop.
- Restructure without showing the user first.
- Tick or untick a box without a finding behind it.
- Invent Tasks that weren't there: that's `/spec-new`'s, or the user's.
- Drop a task or requirement quietly: the Changelog says which, and why.
- Open implementation notes: those are `/spec-implement`'s.
- Go round again: one review per invocation.
