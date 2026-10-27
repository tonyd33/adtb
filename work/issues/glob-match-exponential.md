---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 763
summary: "the file pattern matcher backtracks at every star, so a pattern with several stars takes exponential time"
closed: 2026-10-26
tags:
  - quality
  - engine
---

# A file pattern with several stars takes exponential time to match

## Report

`matches` (`crates/adtb-file/src/lib.rs:136-147`) backtracks recursively at every `*`, so its time is exponential in the number of stars.

`read_csv("*a*a*a*a*a*a*a*a*a*b", Small)` over a directory holding one file named 40 `a`s plus `.csv` took 10 s at the test profile.

Fix: use the linear wildcard matcher that remembers only the last star position and its match point, and add a test with such a pattern.

## 2026-10-26: done

`matches` in `crates/adtb-file/src/lib.rs` is the iterative wildcard matcher that keeps only the last star and the name position it matched from: O(pattern x name). Tests: `a_glob_with_many_stars_matches_in_polynomial_time` (twelve stars over a 40-byte name, both a miss and a match) in both `files.rs`, and `a_glob_star_matches_an_empty_run_and_question_one_byte` in the reference's.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the fbug track.
