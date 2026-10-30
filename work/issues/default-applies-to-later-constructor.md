---
created-at: 2026-10-29
status: open
kind: bug
priority: 520
summary: The checker fills a default into a constructor added after the default
---

# The checker fills a default into a constructor added after the default

## Report

Lead ruling 2026-10-29 (spec revision 30, G14): a default applies only to the constructors its
log entry names. A constructor added later with field `f` does not get an existing `default f`.
The checker applies it anyway at crates/adtb-lang/src/check/write.rs:639. Fix: fill only the
named constructors; test with an `add constructor` after `add default`.
