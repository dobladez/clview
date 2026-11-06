---
name: spec-new-auto
description: Research and write a new Draft change spec under docs/specs unattended, with no interview. Use when the user asks for unattended or automatic spec writing, or a first-pass Draft from a description or the next item in the README's Plans.
---

# Spec new, unattended

Read these, in order, then follow the recipe:

1. [`../spec-shared/conventions.md`](../spec-shared/conventions.md): what every `spec-*` skill shares.
2. `docs/specs/project.md`: this project's commands, code and docs.
3. [The recipe](references/recipe.md), all of it, before acting. It is written as differences from
   [`spec-new`'s](../spec-new/references/recipe.md), so read that one too.

**Invariants:** this runs unattended: never ask the user anything along the way, and never use a
question tool. Write the spec as `Draft`, say every assumption in the summary at the end, and
change only two files: the new spec, and the specs' README, whose Index `--index` rewrites.
