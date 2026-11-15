---
created-at: 2026-11-15
status: open
kind: bug
priority: 230
summary: "set setting accepts a negative count or duration, and the engine reads it as 0"
tags:
  - engine
  - spec
  - class-admission-per-path
---

# `set setting` accepts a negative count or duration, read back as 0

## Report

Found by structure-lang-identity step 10a. `set_setting` (crates/adtb/src/api/command.rs) writes any value of the declared type, so `set setting RetryLimit = { retries: -1 }`, or a negative `RetentionWindow` or `ScrubInterval` duration, is committed. `Settings::set` (crates/adtb/src/api/settings.rs) then reads each as 0 through `unwrap_or(0)`. The client sees its value accepted and a different one applied. catalog.md §10.2 states no range for any setting; the other duration and count settings were not checked.

Ruling: a duration or count setting is non-negative. `set setting` with a negative value is refused, naming the setting and the value; catalog.md §10.2 states the rule. With that in place, a negative value read at open is damage, refused like the other malformed settings rows, and the three clamps go.

Work: the refusal and its reason (through the existing refusal sum for commands), the spec sentence, an engine script per kind of setting (count, duration, optional duration), the open-time refusal, and every setting checked for a range rule of its own (`VacuumBudget`'s fractions).
