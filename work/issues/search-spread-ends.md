---
created-at: 2026-11-08
status: open
kind: bug
priority: 860
summary: "the crash searches' spread returns only the first item when most is below 2, against its doc's both ends, so the quick profile never samples the last op or event"
tags:
  - quality
  - sim
  - test
---

# The quick crash searches sample only the first op, not both ends

## Report

`spread` (crates/adtb-harness/src/search.rs:18) is documented "Returns up to `most` of `items`, evenly spread and holding both ends", but for `most < 2` it returns only the first item, and one item even for `most == 0`. Under the quick profile `count(n)` is `n.min(1)` (crates/adtb-testkit/src/profile.rs:21), so every call (search.rs:52, 53, 256) samples only the first op or event: the quick run of the op search and `each_crash_image` never reaches the last one.

Fix direction: decide whether one point means the first or the last end, include the last when `most` allows, and state the `most == 0` result; or correct the doc.
