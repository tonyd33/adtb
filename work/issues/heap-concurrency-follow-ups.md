---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 555
summary: "every heap write latches its directory root, a conversion can name many store pages in one update, and log verify is never incremental"
branch: adtb-heapf
depends-on:
  - vacuum-concurrent
tags:
  - engine
---

# Feature: Heap concurrency follow-ups

## Report

From the concurrent vacuum:
- every heap write latches its heap's directory root exclusively, which
  serializes a heap's writers on the root. Latch the root only for
  structural changes, and update categories under the leaf's latch;
- counting new out-of-line values sits in the version's update, so a
  page conversion that moves many values out of line can name many
  store pages in one update, against the pool's latch bound;
- verifying the log from a remembered position needs a pager call
  (a TODO in `docs/design/verify.md`);
- the engine harness in `crates/adtb-sim/tests/engine/` still wraps
  each heap in its own lock, which is no longer needed.
