---
created-at: 2026-10-27
status: in-progress
kind: bug
priority: 699
summary: ordinary program writes can modify catalog and sys relations instead of Denied
branch: system-write-guard
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
