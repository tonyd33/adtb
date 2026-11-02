---
created-at: 2026-11-02
status: open
kind: bug
priority: 520
summary: A pinned insert built under a selection or fix is refused lacking an added field
---

# Pinned insert fill under fix selection

## Report

Refused with `Untranslatable`, against evolution.md 7.4, pinned by `a_pinned_insert_built_under_a_selection_or_fix_is_refused_lacking_the_added_field`. Safe, an over-refusal. An attempt to fill was dropped: a generated default under `Fix` is admitted then fails every call with `StatementLimit(Rounds)`, since crates/adtb/src/write/fresh.rs puts the fix variable in the placeholder record and rounds never equal; and filling through `Restrict` fills excluded constructors, refusing migrations main admits.
