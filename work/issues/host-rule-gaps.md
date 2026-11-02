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

## 2026-11-02: first pass landed, rulings

`fix: close the escapes of the host redefine rule`. Live defect: a redefine in a migration that did not resolve failed open, since the blame lookup doubled as the decision. Now decided first, blamed after. Revert under a live pin was already refused. Wasm and extensions cannot change a pinned function today: any `wasm` definition is refused at apply, and `install`, `uninstall` are refused since nothing fills `Source::packages`. Class: authority per entry path. Check exit 0, eight mutants die, review found no blocker.

Lead rulings:

- Not presented at open: spec stands. A bundle opened and not presented is superseded by this open, before migrating. The rule reads live after the open's supersession.
- A bundle going live at a pin whose host code differs, first registered or re-registered, proposal: refuse at registration, one host check, about 10 lines, which also closes pins ahead of the file. `a_redefine_back_to_the_code...` and two fill tests change. Build it next here. A call of such a function is refused by translation today, so this is not a wrong answer, except that the behaviour of such a call is untested.
- Identity of host code covers symbol and version. interop.md 9.5 says version only: amend it with that build. Ignoring the symbol would admit a redefine that changes what runs.
- Wasm by hash waits until a migration can install a module. It needs bytes or a hash in `Source` and the log, and lifting the `recordable` refusal. Requirement for that work.

Left from the review:

- crates/adtb-lang/src/translate/mod.rs `host_conflict`, false refusal new with this fix, traced: f host at a live pin, language now, body naming `hs`, then `rename relation hs to ..` rewrites the body, so the code differs with no `redefine fn`. Gate on a resolved `RedefineFn` or `Revert` when the migration resolves. Way out today: retire or repin.
- A `revert` before a `redefine fn f` in one migration blames the revert; an unresolved migration blames operator 0, pinned by a test with no contract. Wasm conflict text says host function.
- bundles.rs: `refused` counts over a static table; the four case wasm part has empty `live`, so "each way against each state" overclaims; `LiveOmitted` is not live, expect superseded; no language to host or held host was language case. Summed 4.4 s, over the 3 s budget.
