---
created-at: 2026-11-02
status: open
kind: bug
priority: 136
summary: "The host redefine rule misses bundles not presented at open, a revert, and Wasm byte identity"
---

# Host rule gaps

## Report

From the review of `fix: refuse a pinned fill whose function changed since the fill`, by reading, not run. Rule: evolution.md 7.1, interop.md 9.5.

- crates/adtb/src/api/mod.rs readers and registered/mod.rs: `readers` keeps only `regs`, `commanded` restores only command origin. A live bundle not presented at open escapes: web-1.0 Active, open log plus a host redefine, `registrations: []`, is admitted. Supersession relies on this; a fix must exclude superseded only.
- crates/adtb-lang/src/translate/mod.rs `host_conflict` fails open: no matching `Revert` or `RedefineFn`, or a `resolved` error, returns `None` and admits. Decide the conflict first, blame after.
- No test of `revert` of a redefine under a live pin; deleting the `Revert` arm admits silently.
- crates/adtb-lang/src/evolve/resolved.rs: `Code::Wasm` compares module name and export, where 9.5 says a hash of bytes, so replacement under one name goes unchecked; `Code::Host` compares symbol and version, where 9.5 says declared version.
- `install` and `uninstall` change host code unseen by `functions`, and 7.1 places install under the rule. Reachability unverified.
- crates/adtb-sim/tests/bundles.rs: the retired half of the redefine test cannot fail, its last open passes `vec![]`; pass `regs()`. Untested under a live pin: language redefine, pin predating the function, same code redefine, expired bundle. `hs.key(65)` pins a sequence batch size.

Lead rulings needed, safe either way today: a bundle first registered in the redefining open is admitted; `drop fn f` then `define fn f` with other host code gets a fresh id and blocks no non-calling bundle; language at the pin to host now, and a Wasm pin with host before, are over-refused, with `retire bundle` as the way out.
