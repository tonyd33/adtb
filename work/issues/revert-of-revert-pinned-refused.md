---
created-at: 2026-11-16
status: open
kind: bug
priority: 160
summary: "a pinned write is refused across a revert of a migration whose functions name a relation it adds, with no ws in scope"
tags:
  - lang
  - class-identity-by-spelling
---

# Revert of revert pinned refused

## Report

Found by the review of committed-schema-type part 2b-i (2026-11-16), reproduced on its tree. History: 0001 `O {id, n, x}`; 0002 `drop field O.x`; 0003 `add type W`, `define fn which (k: Option (Key ws))`, `revert 0002` with `fill O.x = match | O o -> which(if o.n > 0 then None else None)`, `add relation ws`; 0004 `revert 0003`. A bundle pinned at 2 inserting `os { n: 5 }`, whose schema equals the current one, is refused at registration: "across `revert 0003`: no `ws` in scope; no `ws` in scope". A further `revert 0004` gives the same refusal. A valid write is refused.

Likely cause: translate `reverted()` (crates/adtb-lang/src/translate/mod.rs, near 680) resolves each inverse step's text against a source that does not build at that step (after `drop relation ws`, `which` still names `Key ws`), resolving by name where the inverse names objects by id (evolution.md §5.1).

Fix direction: resolve revert steps by the ids the inverse carries, or where the schema builds; a searching test that generates reverts of reverts and of older migrations (the 2b-i generator widened) over histories whose functions name relations added in the same migration.

Reproduced on main 14bbbd23 by the 2b-i author (engine script, bundle `web-1.0 pins 2`): `reopen` gives `RegistrationRefused { bundle: "web-1.0", reason: Rejected { reason: Untranslatable { query: "web-1.0::put", detail: "across `revert 0003`: no `ws` in scope; no `ws` in scope" } } }`. The detail repeats its reason.
