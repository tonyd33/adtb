---
created-at: 2026-10-30
status: in-progress
branch: quality-fixes32
kind: bug
priority: 850
summary: "server_authority.rs's monitors helper documents a grant_option it does not return"
tags:
  - quality
  - test
---

# server_authority.rs's monitors helper documents a grant_option it does not return

## Report

`crates/adtb-sim/tests/server_authority.rs:557` says "Returns `(grantee, grantor, grant_option)`", and the query selects `g.grantee ++ " " ++ g.grantor`. Tests built on it, such as `a_revoke_leaving_another_option_keeps_the_rows_it_covers`, read as if they check the option and cannot see it.

Fix direction: select the option too and update the expected rows, or correct the doc.
