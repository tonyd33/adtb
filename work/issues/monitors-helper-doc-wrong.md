---
created-at: 2026-10-30
status: resolved
kind: bug
priority: 850
summary: "server_authority.rs's monitors helper documents a grant_option it does not return"
closed: 2026-10-30
tags:
  - quality
  - test
---

# server_authority.rs's monitors helper documents a grant_option it does not return

## Report

`crates/adtb-sim/tests/server_authority.rs:557` says "Returns `(grantee, grantor, grant_option)`", and the query selects `g.grantee ++ " " ++ g.grantor`. Tests built on it, such as `a_revoke_leaving_another_option_keeps_the_rows_it_covers`, read as if they check the option and cannot see it.

Fix direction: select the option too and update the expected rows, or correct the doc.

## 2026-10-30

Fixed in the quality-fixes32 commit; check passed with 3023 tests.

## 2026-10-30: resolved
