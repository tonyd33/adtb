---
created-at: 2026-10-31
status: open
kind: bug
priority: 445
summary: "a file read refused outside file_roots writes no Denied row and no Call row, so the attempt leaves nothing in catalog::audit"
tags:
  - quality
  - engine
---

# Confined refusal unaudited

## Report

`Local::scan` (crates/adtb/src/write/input.rs:208) answers a confined read outside the file roots with `write::Error::Denied`. `Call::audited` (crates/adtb/src/api/server/call.rs:198) assumes every `Denied` already has its row, clears `role_call` and so writes no `Call` row, and nothing on this path calls `audit_denied`. `refuse_witnesses` (crates/adtb/src/api/command.rs:418) writes its row by hand; the file scan does not. security.md 8.2 says the rows of a denied call are written in a system transaction, so a denied call is recorded.

Found by reading; not run. crates/adtb-sim/tests/server_files.rs checks the refusal's detail and never reads `catalog::audit`, so the suite passes either way.

Repro: on a session holding `read_files`, submit a bundle call or text reading `out/secret.csv` with `file_roots` not covering it, as `an_empty_file_roots_refuses_every_read_with_read_files_held` does, then `from a in catalog::audit.Denied where a.role == "web" select a.origin`.

Actual: no row. Expected: one `Denied` row with the call's origin, and no `Call` row.

Direction: write the `Denied` row in `Call::audited` for a refusal raised while running, or have `audited` tell apart one already recorded by `readmit` and `refuse_witnesses`. Add the audit assertion to server_files.rs for a bundle call and a text.
