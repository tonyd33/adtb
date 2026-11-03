---
created-at: 2026-11-03
status: open
kind: bug
priority: 470
summary: a latent read error on an allocation bitmap page leaves a data page refused on reopen with no finding naming it
---

# Unreadable bitmap refuses a page with no finding

## Report

Found by the seed-floors track while measuring rates in crates/adtb-sim/tests/pager.rs: with a latent read error on the allocation bitmap (`Page { Pid(2), Unreadable }`), a data page is refused on reopen and no finding names it ("Pid(N) refused without a finding"). Seen at `every_fault_at_once` seed 1011 and deep `bit_rot_and_latent_errors_are_reported_never_returned` seed 248 round 2, which fails under deep on main. Nothing wrong is returned and no acknowledged row is lost: a refusal verify does not report. Read with the scrub ruling that an Unreadable page marks nothing (docs/design/storage.md) before deciding which side is wrong.
