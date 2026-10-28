---
created-at: 2026-10-28
status: resolved
closed: 2026-10-28
kind: bug
priority: 799
summary: bundles without a module import resolve the default Principal instead of the custom root type
tags:
  - engine
  - lang
  - quality
  - test
---

# Custom principal global lookup

## Report

During the shared principal validation baseline, sourceafd90d40 on c14a6594
opened a custom Principal in module identity and registered a bundle with:

```adt
query me () -> Principal = caller
```

Both fixtures failed open with RegistrationRefused/Invalid:

```text
query `me`: expected Principal, found Principal
```

Adding import identity let registration proceed and exposed the independent
asserted-value bug. That import is NOT required by security1.1: the schema's
Principal belongs to root and resolves from every module and bundle. Adopted
spec/schema-principal/root-first is the authority. The apparent mismatch is
consistent with unimported signature lookup selecting the fixed prelude type
while caller has the custom type; exact resolver site still needs audit.

Evidence: adtb-principal-validation/target/principal-baseline-before-import,
originalafd90d40 and captured RegistrationRefused in both tests. The initial
setup-error diagnosis was wrong about user validity. This is not a successful
principal-validator baseline because it fails earlier, but it is public engine
lookup evidence. Reproduce a minimal exact-current main regression, fix the shared
lookup boundary across modules/bundles and test default/custom/import controls.
No import workaround, changed adopted root semantics or new design choice.
Current asserted-principal-value-domain remains separate and does not claim this
resolved; finish its frozen checks before another source change. All provider
and campaign expansion remains paused behind engine correctness.

## 2026-10-28: resumed engine-first track

Repair shared root name resolution from main5debace3; no import workaround or spec change. Minimal current-main failing regression, default/custom/import and public registration controls; affected lang/sim tests, strict clippy, fmt, house and size checks. Lead owns fresh independent review and full post-land check. Other UUID track is evidence-only; lookup lands first.

Workspace /home/tony/code/adtb-principal-lookup; local brief work/briefs/brief-principal-lookup.md. Preserve baseline and failed evidence; receipt must name exact tested inputs, statuses and unresolved checks. No dependencies added.

## 2026-10-28: shared repair reviewed, integration pending

Frozen trackda86e55295b34a50bc84a229f997ee2c18a5b1fd on
5debace3ea0f97765bc72282bbb0d721b8cfc851 moves the existing root Principal
branch in Schema::lookup_type ahead of module/PRELUDE lookup, after explicit
qualification. The empty-import resolver falls back through PRELUDE, which used
to return the stale default type before reaching the root branch. No new lookup
layer, signature, dependency or spec change. Constructor lookup already uses the
root identity. Default IDs37–40 and operator-assigned custom IDs preserved.

Minimal public no-import registration reproduced original Invalid expected
Principal/foundPrincipal before fix (baseline4aeb354c, warm repeat100). Initial
wrapper status capture failed on zsh reserved status variable; both logs retained.
First fixed-source iteration had2new test-fixture mistakes (ctor ID expectations
and add fn syntax); preserved, corrected to operator-assigned IDs and define fn.
Final643lang +141public tests, strict lang/sim clippy, fmt/house/size all passed0;
recorded source/Cargo config hashes unchanged. Added public tests each<0.5s.
Root owns full check as explicitly assigned branch exception; it has not run yet.

Fresh independent source/contract review read all5paths and verified30hashes plus
4absent configs in both checkouts, full-range house0, no blocker. No reviewer
compilation. Receipts/review/handoff in work/briefs/*principal-lookup*. Logs under
/home/tony/code/adtb-principal-lookup/target/principal-lookup/. Final revision and
scope above are durable if ignored notes are missing. Next: lead land via
work/tools/land principal-lookup 'fix: resolve the root principal in every scope',
then full root/campaign/policy/docs check; resolve only after that result.

## 2026-10-28: resolved

Landed1be643fa via work/tools/land after fresh independent review. Rebase delta
versus testedda86e552 contains only6ticket/design-evidence files; all5source paths
byte-identical. Full MAIN check collected0 on clean
2a118de56dd7e3863c5b6885d11227c75e16b44b, parent1be643fa,17:47:34Z–17:51:11Z
(217s).2,392root tests passed/8skipped50.106s;47campaign passed/1skipped21.177s.
All lint/fmt/parity/dependency/size/docs passed. Docs10tests10.165s,420syntax
inputs,61pages/6730links/4995fragments/0failures. Tracked and root/app local
Cargo config hashes + revision unchanged throughout. No remaining source/check
blocker. Permission/queue/execution-only timing unknown. Evidence
 target/principal-lookup-integration/{check.log,check.status,inputs-*,revision-*}
logSHA256548dc267a8605ed86dac2f344f64a5f62570710ab0e1b2a1e1fb6a27deb3f50a.
Receipt work/briefs/receipt-principal-lookup-main.md. Pushing deferred per user
because Tangled is down; local landing is complete.
