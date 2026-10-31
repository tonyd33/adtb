---
created-at: 2026-10-31
status: resolved
kind: bug
priority: 480
summary: open_returns_on_a_real_file_cut_before_the_first_roles_page waits 5 s of wall clock and failed once under load
closed: 2026-10-31
---

# Api real file cut open flake

## Report

`api_real_file open_returns_on_a_real_file_cut_before_the_first_roles_page`: `open returns: Timeout`, in one full suite run of four on the heap-cap tree, the slowest at 148.8 s. It waits 5 s of wall clock with `recv_timeout`. Not investigated. Replace the wall clock wait with the suite's timeout.

## 2026-10-31: fixed

Slow I/O under a full suite, not a hang, by one reproduction in 542 runs and one measured slow run. The wall-clock wait is gone; nextest's bound stays, so a real hang shows as TIMEOUT.

## 2026-10-31: resolved
