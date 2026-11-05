---
created-at: 2026-11-05
status: open
kind: bug
priority: 300
summary: "a lost write to a commit-log status page is undone at reopen: abort_unresolved rewrites the acknowledged commit as aborted with a full-page image, verify then finds nothing, and the rows are gone"
---

# A lost commit-log write turns an acknowledged commit into an abort

## Report

## Report

Found by the quarantine-dump author while building a test, 2026-11-05; reproduced in the sim (crates/adtb-sim/tests/sim/dump_quarantine.rs builds the lost write, then marks the file quarantined whole before reverting the page because the plain case heals silently). A commit acknowledged to its client whose commit-log status page write is lost: at the next open, recovery sees the transaction unresolved, `abort_unresolved` writes it as aborted with a full-page image, verify afterwards finds nothing, and the transaction's rows are invisible. Acknowledged data lost with no report. `Transactions::open` checks status pages by checksum only, not against the log.

## Proposal

Status in the commit log must agree with the WAL: a commit record in the retained log is authoritative over the status page. At recovery, settle each transaction from the log's commit or abort record before `abort_unresolved` treats it as unresolved; a status page whose LSN is behind the log's last record for it is a lost write (damage, reported), not an unresolved transaction. Test: the lost write above, reopened, the rows visible and a finding reported, never an abort.
