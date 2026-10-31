---
created-at: 2026-10-30
status: open
kind: feature
priority: 508
summary: "registered and facade Call rows, Register, SchemaOp, Commit and Login rows are unwritten; args hashes canonical text with no wire encoding; seq order is not commit order, which the ledger and archive audit need"
---

# Feature: Audit follow ups

## Request

## Proposal

## Open questions

Also: registered writes commit under a mark on the audit, since they write no row yet; they must fail `Quarantined` once they do. A mark between the guard of `set setting` at `start` and its own transaction leaves the setting changed with no row, since the swap and the row are separate transactions. A `configure` holder can tell a schema name from any other by naming a parameter after it (`Denied` against runs); closing that needs parameters resolved first in the checker. `Tables::next_id` still scans `catalog::audit`.

## 2026-10-30: from nonowner-calls

`Commit` audit rows are not written for any call, ad hoc or registered. security.md 8.2 puts the row in the transaction's commit with its position, which exists only once `txns.commit` appends the commit record, and `changes` needs the wire encoding. The spec stands: the engine learns to fill the position at commit, with no post-commit system transaction. `a_registered_commit_writes_its_commit_row` in crates/adtb-sim/tests/server_binding.rs is ignored until then.
