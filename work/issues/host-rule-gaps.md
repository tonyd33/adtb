---
created-at: 2026-11-02
status: open
kind: bug
priority: 610
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

## 2026-11-02: registration check landed

`fix: refuse a bundle going live at a pin whose host code differs`. A bundle first registered, or registered again after it expired or was superseded, is refused when its pin holds a host or Wasm function with other code. The check runs in `registered::registration`, open step 9 and the register command, since step 5 would miss a redefine in the same open. `host_conflict` now needs a resolved `RedefineFn` or `Revert`, so the rename false refusal is gone, run and regressed. evolution.md 7.3 and applications.md 4.1 list the refusal. Class: authority per entry path. Check exit 0, five mutants die, review found no blocker.

Lead ruling, changing the earlier one: the identity hash stays over the declared version for now, and interop.md 9.5 stands. Hashing the symbol as well panics on a stored file at the next `redefine fn` (`every row deleted is visible`), because old rows are deleted by re-derived value. The rule compares symbol and version already, so no wrong answer follows. Change the hash with the next file format break, or after rows delete by id.

Left, severe first:

- crates/adtb/src/api/bundle.rs:230, api/mod.rs:1028: a recorded row never registered counts as a reader, and first registration refused now leaves one often. Read, not run: open `[BARE, CODED, TO_B]` with web-1.0, refused; reopen with a further host redefine and the same bundle gives `MigrationRefused` naming web-1.0, where embedding.md step 5 and evolution.md 8.4 say the migration applies, then `RegistrationRefused`. Way out: omit the bundle or redefine back.
- bundle.rs `going_live`: an Active row with no mark counts as going live, so a file from before this change holding a live bundle at other code refuses at next open. Always-true mutant survives since the API cannot make that state.
- registered/mod.rs `registration`: a bundle with no entries reports query `web-1.0::`.
- A language function at the pin, host now, stays over-refused under a live pin; no test. Untested: pin holds host `f`, `f` dropped now. A `revert` before a `redefine fn f` blames the revert.

## 2026-11-02: recorded row is no reader

`fix: hold back no migration for a bundle never registered`. Run and reproduced, `MigrationRefused` naming web-1.0. Open now takes its readers from `Stored::readers`, live with a registered mark; grant paths keep `live` and still refuse an unmarked row. Class: authority per entry path. Check exit 0, three mutants die, review found no blocker.

Lead ruling: a file written before marks landed, three days ago, is not supported past this. Its served bundle has no mark, so a migration it would have blocked commits and registration then refuses. No release has shipped.

Left:

- `a_bundle_never_registered_and_not_presented_again...` passes with `live` restored at api/mod.rs open, as `regs` is empty. It pins supersession only, so its name overclaims.
- A rename under a live registered pin of a function host at the pin, language now, has no API test. The API cannot make that pin, since the redefine is refused under it; the `host_conflict` unit test stays.

## 2026-11-03: reprioritised

Lead ruling: priority 136 to 610. Every item left is a test that overclaims, a detail text (`web-1.0::`, blame on a `revert` before `redefine fn f`), an over-refusal with `retire bundle` as the way out, or an untested path; none gives a wrong answer or loses data.
