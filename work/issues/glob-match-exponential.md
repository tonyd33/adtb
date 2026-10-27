---
created-at: 2026-10-26
status: open
kind: bug
priority: 763
summary: "the file pattern matcher backtracks at every star, so a pattern with several stars takes exponential time"
tags:
  - quality
  - engine
---

# A file pattern with several stars takes exponential time to match

## Report

`matches` (`crates/adtb-file/src/lib.rs:136-147`) backtracks recursively at every `*`, so its time is exponential in the number of stars.

`read_csv("*a*a*a*a*a*a*a*a*a*b", Small)` over a directory holding one file named 40 `a`s plus `.csv` took 10 s at the test profile.

Fix: use the linear wildcard matcher that remembers only the last star position and its match point, and add a test with such a pattern.
