---
created-at: 2026-11-02
status: resolved
kind: bug
priority: 135
summary: "A pinned fill calling a function redefined and back uses current code, and a host redefine may pass under a live pin"
closed: 2026-11-02
---

# Fill function code at fill time

## Report

From the review of `fix: translate a fill or default through its own schema`, traced, not run.

- crates/adtb-lang/src/translate/mod.rs `same_code` compares pin and current, never the fill's version. `define fn f = host "a"` (pin); `redefine fn f = host "b"`; `add field H.R.x: Text fill f("s")`; `redefine fn f = host "a"`; a bundle at the pin inserts. Backfilled rows hold `"b"`, the pinned insert stores `"a"`, so the column mixes both. Lead ruling: require the code at the fill's schema to match too, else refuse. A refusal is never a wrong answer. Language functions inline at fill time, so only host and Wasm calls are affected. A default uses current code, per evolution.md 7.4.
- docs/spec/evolution.md near 1181 refuses a host redefine under a live differing pin; no check was found in crates/adtb/src or crates/adtb-lang/src. If absent, the case above is reachable with the bundle live.
- `uncrossed` does not run on fill or default terms; the later `Map` refusal is not traced.

Test gaps in crates/adtb-sim/tests/bundles.rs from the same review: a tag in a default past a removal; a moved aggregate; drop and re-add of a function; the stored value sweep's name claims each shape though `f`, `g`, `en` never hold the removed constructor; `swept` and `reached` count a loop over a constant; about half of 12 s may be redundant (`padded_tag_type`, the full refusal cross).

## 2026-11-02: fixed

`fix: refuse a pinned fill whose function changed since the fill`. Reproduced: backfilled `"b"`, pinned insert `"a"`. `same_code` now also compares the code where the fill was written. The host redefine rule was only partly built: a bundle not calling `f` was admitted; `host_conflict` in `recheck` now refuses it. Class: authority per entry path. Full check exit 0, seven mutants die, review found no blocker. Gaps on host-rule-gaps.

## 2026-11-02: resolved
