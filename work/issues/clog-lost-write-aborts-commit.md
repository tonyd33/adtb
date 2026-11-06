---
created-at: 2026-11-05
status: resolved
closed: 2026-11-05
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

## 2026-11-05 resolved

Landed as `fix: settle a transaction from its logged commit when its status page lost the write`.

Rule: at open, an id still in progress reads committed exactly when its status page or a commit record of the retained log says so, and aborted otherwise. `CommitLog::unresolved` reads the log from the oldest LSN of a status page holding an id in progress up to the end recovery kept. If that stretch is not whole and readable, settle writes nothing, the open quarantines the whole database with `Damage::LogUnproven` naming that page, and `admit_dump` refuses through the same function. Before the first present segment the status page is authoritative; a lost write whose record lies there is undetectable (mvcc.md §4.2).

Cause: `abort_unresolved` trusted the status page alone. Defect class: Damage reported as nothing.

Also in the change: `verify_holding` takes the vacuum hold for its whole run (order: vacuum hold, verify snapshot, page latches), and the index checks under it use `check_indexes_held`. Scrub tests time the pass from the job row and rot a page inside the file. Pinned `renewed` instants in four registration scripts moved with the I/O done at open; they are sim clock only.

Reviews: a full review and a scoped review of the coverage check found a false-evidence test, Aborted written on incomplete log evidence, a restored log opening on a continuation block quarantined as damaged, and a dump admitted on an unproven log. All four are fixed. Mutants on the settle decision, the coverage check and each damage arm are killed, except the list or open failure arm, which the sim cannot fault.

Follow-ups: verify-marks-whole-returns-fatal, vacuum-erases-lost-write, catalog-settled-inprogress, root-resolved-vestigial, dump-refusal-log-unproven-spec.
