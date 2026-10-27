---
created-at: 2026-10-26
status: open
kind: feature
priority: 871
summary: "a new user at the adtb CLI still meets misleading name errors, cascading and repeated refusals, a migration log passed by hand, and output that differs from what was written"
tags:
  - shell
---

# Feature: shell follow-ups for a new user

## Report

Found by the shell usability track while writing the getting-started page:
- A typo in a relation name reads "`nope` is a parameter of this text, but the call supplies no value for it". Ad hoc text in the shell supplies no parameters, so an unknown name should say it is unknown.
- Adding a migration means passing every earlier one, in order, as repeated `--schema` flags; passing only the new one fails with `Diverged`. `apply migrations` in ad hoc text is refused as not running in this engine yet. The shell needs a way to apply one new migration to an open database.
- A field error cascades: "no type `Int`" is followed by "no type `P`" for the type that failed. One refusal also printed the same message twice: "no module `demo`; a migration of a new module starts with `add module demo`".
- `true` is accepted as input but prints as `True`.
- A record prints its fields in name order, not declaration order.
- `order by` after a select of a `match` is refused.
- A foreign-key refusal reads "constraint `shelter::adoptions.Adoption.pet` is violated by 1 row" and never says a reference names a missing row.

Check each against the spec before changing it: printing order, the spelling of `True` and the `order by` refusal may be specified.

## 2026-10-27: found by the manual

- At a terminal the shell runs a `match` as soon as its first arm parses, so a multi-line match cannot be typed at the prompt.
- `--create` without `-c` opens the prompt and waits on stdin.
- `\help` says ad hoc text may be "a migration", but migration text is refused as ad hoc text.
