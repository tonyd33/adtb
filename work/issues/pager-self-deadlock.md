---
created-at: 2026-10-25
status: open
kind: bug
priority: 407
summary: "a page guard held across a later pager call on the same page waits forever, with no diagnosis"
depends-on:
  - vmcache
tags:
  - engine
---

# Bug: A held guard deadlocks its own thread

## Report

A `PageShared` or `PageExclusive` held across a later pager call on the
same page blocks that call forever. The `&mut` pager made this
impossible; the shared pager lets it compile. The heap and B-link tree
copy pages to avoid it.

My ruling: state it as a §2.3 precondition, and have the pager detect
it. In debug builds and under the simulator, a thread that asks for a
latch it already holds on the same page panics, naming the page,
instead of waiting.
