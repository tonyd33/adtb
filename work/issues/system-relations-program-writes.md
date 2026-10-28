---
created-at: 2026-10-27
status: open
kind: bug
priority: 699
summary: ordinary program writes can modify catalog and sys relations instead of Denied
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
