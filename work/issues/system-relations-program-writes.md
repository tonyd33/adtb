---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 699
summary: ordinary program writes can modify catalog and sys relations instead of Denied
closed: 2026-10-27
tags:
  - engine
  - test
---

# System relations program writes

## Report

catalog.md1.1 forbids ordinary program insert/update/delete on all catalog:: and
sys:: relations with Rejected/Denied, even through embedded owner. Role provider
fixture probe inserted catalog::peer_mappings and read back the row. Existing
binary exit0, SHA256f216edbe3280421ce2c00ff328014ef695451fbc7a097aa6fdc6bb9925e4892a;
source provenance unknown, so exact-current regression must confirm before fix.
No provider fixture may rely on forbidden raw catalog writes.

## 2026-10-28: full engine priority

Campaign459ee9ff and provider81c2bbc2 preserved/paused. Provider probe evidence in
role-authority/target/authority-diagnostics/system-write-readback.{log,status},
system-write.adt/schema.adt; receipt-role-authority-finish-paused.md. Fresh general
engine guard track must reproduce on exact current main, then centrally enforce
front-door protection across ad hoc/interactive/registered/typed write paths,
including nested or indirect changes and no-op writes. Preserve internal catalog/
bootstrap/deployment/maintenance writes through principled internal ownership,
not special magic fixture text or blanket raw-storage rejection. No app/provider
workaround. No new normative behavior or dependency needed; existing rule is clear.

Acceptance: exact typed Denied and no catalog/sys mutation across native client
paths, user-relation writes and legitimate deployment/command behavior unchanged,
failing baseline then regression/generalfix, independent review and full branch/
main checks. Use targeted tests/lint/fmt/house while iterating; retain snapshots/
logs/stability/exits/timings. Exact implementation ownership in fresh brief.

## 2026-10-28: exact-main baseline and settled boundary

Rebuilt probe via cargo run --profiletest using currentmaina795a2c4 code and own
main target, then insert+readback returned PeerMapping, exit0. No rootRust edits
during diagnostic; whole-tree beforehash not captured for this probe. Exact command
work/tools/adtb-probe --schema role-authority/target/authority-diagnostics/schema.adt
--engine with the two statements in system-write.adt. Logs main target/system-
write-baseline/probe.{log,status}. Fresh system-write-guard workspace on a795a2c4
owns regressions/generalfix and firstRustbuildslot; further exact unit baseline
required. Brief brief-system-write-guard.md.

Approved implementation owner: single private API relation/program policy checks
typed checked kernels BEFORE normalization, follows checked function bodies with
cycle-safe traversal, uses engine-moduleidentity. Shared public program admission
at Prepared::new and registeredchecked/imported path; directnative appender checks
targetbinding through same policy. Cover textual/interactive/registered/typed/
Arrow/load no-op and nested/indirect writes. Legitimate internal catalogdeploy,
bootstrap, commands and trusted owner mutations use their internal paths; no
publicmagic bypass token, storageblanketblock, perendpointduplicated guards or
newpermissionframework. Existing typedDenied constructor/propagation only.

## 2026-10-28: refined lowest-tier diagnostic ownership

Exactcurrent public API baseline failed nextest100: expectedDenied gotOk([]),
0.147s after60s coldcompile; source baselineonlytest on a795a2c4, production
unchanged. Worker target/system-write-baseline retainsstate/revision/log/status.
API-only postcheck proved too late: check_adhoc/check_txn volatile-dependency
analysis rejects sys writes as Invalid before checkedkernel reachesAPI.

Approved refined owner supersedes earlier APIwalker plan: structural systemwrite
policy in languageanalysis before volatile checks/normalization, minimal typed
DiagKind Invalid/Denied preserving existingrendering. signature::calls has2callers;
return structuredDiag there directly, bundle retainspublicStringcontract via
.message, engine mapscategory to existingAPI Denied. Directappender targetbinding
reuses same exported relationpolicy. Remove obsoleteAPIwalker, no parallelwrapper,
stringmessage matching or newpermissionframework. Narrowlangspan/signature/analysis
andAPI errorconversion ownership added to guardtrack. No normativechange/dependency.

## 2026-10-28: embedded-open oracle corrected

The guard's initial no-header-after-denial assertion was wrong. A fresh audit
checked applications.md8, embedding.md2.1 and spec/embedded-open/register-first:
early unchecked header recording and supersession deliberately survive later
failure; only step-5 callable registrations are atomic. No provider repair was
made. bundle-registration-refusal-side-effects is closed wontfix with evidence.
The original guard worker remains active on the same track, correcting the test
to check no callable registration/renewal while preserving required headers.
Ordinary program DML on catalog/sys still must return typed Denied.

## 2026-10-28: resolved

Landed `d2716fb0`, `fix: deny program writes to system relations`. Language
acceptance owns one resolved-module policy before normalization and volatile
checks. It traverses the checked graph iteratively, including nested/dead writes,
and exhaustively classifies write variants. Small typed diagnostics preserve
`Denied` through signature checking and API conversion. Native/typed appenders
reuse the target policy at binding, including empty loads. The obsolete API
walker was removed; no message matching, permission framework, public bypass,
dependency or normative change was added. Trusted internal deployment, settings
and maintenance writers retain their existing paths.

Fresh independent architecture/correctness review cleared exact source
`cf4f5d8db9824933cc3c4e4858aef30cb4e5e963` on
`3c2ffb06cd33f40ea5b01ab33e9356af806d9f06`. Nine public simulator regressions
cover the accepted catalog-insert witness, system relation/constructor targets,
no-op and nested writes, mixed-call atomicity, interactive continuation,
registered/bundle/typed/Arrow entry points, native bulk binding, and legal user,
temp, system-read and native-command controls. New tests took0.252–0.599s each
in the full run. Fixed seed and existing higher-order suite coverage are limits;
no socket or schedule-sweep claim. Embedded-open headers/supersession retain
required early durability; only callable publication/renewal is checked atomic.
The rejected no-header oracle and its correction are preserved.

Authoritative branch check exited0 in409.557s,05:16:29.551432Z–05:23:19.108884Z.
All tracked/local Cargo hashes and revision were unchanged. Results:2366 engine
passed/8 skipped,46 campaign passed/1 skipped, all clippy/fmt/lint parity/dependency/
size/docs checks passed. Native docs:10 Python tests,420 syntax inputs,61 pages,
6730 links,4995 fragments,0 failures. Branch log SHA256:
`dc09891ed70275177f554f62f85138f50c7e96f42ccdffb0198509c141717b29`.

The lead verified the landed tree equals the reviewed source, then main's full
check exited0 from2026-10-28T05:25:28Z to05:32:05Z (397s). Tested clean working
copy `d8f46ebd225392b61d287cf78a5e5a51eb16dd3d`, parent `d2716fb0`; tracked/local
configuration hashes and revision match before/after. Same counts and all checks
passed, including the corrected native docs integration. Exact command from main:

```sh
direnv exec /home/tony/code/adtb env DEVENV_ROOT="$PWD" CARGO_TARGET_DIR="$PWD/target" check
```

Rust1.98.1/Cargo1.98.0, configured default profiles/features. Permission, queue
and execution-only timing are unknown. Main log SHA256:
`be8492557625be0e5f5d3d43efbe4dbe48e8fa322eff0c673b1c4788de3b7e43`.
Evidence: `target/system-guard-integration/`, `target/system-write-guard-evidence/`;
receipt/handoff and independent review under `work/briefs/`. All failures and
incorrect-oracle history remain. Only completed worker `target/debug` was removed
after preserving and verifying non-debug evidence. No active checks or workers.

## 2026-10-27: resolved
