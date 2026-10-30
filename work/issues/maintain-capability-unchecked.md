---
created-at: 2026-10-30
status: resolved
kind: bug
priority: 405
summary: "Capability::Maintain is declared and no command requires it, so verify and vacuum have no check over a server"
closed: 2026-10-30
---

# Maintain capability unchecked

## Report

Found by the verify-online track. Catalog section 9 names `maintain` for these commands. Check every command against its named capability, and test a role lacking it.

## 2026-10-30

Fixed in the server-fixes commit; check passed with 3047 tests. Follow-ups: command-values-over-refused, capability-check-at-execution, audit-adhoc-call-retire, supersede-reveals-absence.

## 2026-10-30: resolved
