---
created-at: 2026-10-29
status: open
kind: bug
priority: 610
summary: insert rejects into reads under an unlimited uncharged grant on the embedded path
---

# insert rejects into reads under an unlimited uncharged grant on the embedded path

## Report

Found by the exec-scan review (2026-10-29). `write::eval::Stmt::rows` reads under an unlimited,
uncharged grant when the statement has no grant. On the embedded `Txn` path crates/adtb/src/api/stop.rs:116
always sets the grant to None, so `insert ... rejects into` reaches it through `Stmt::load`.
Server calls set a grant. Not a regression. Fix: give embedded statements a grant, as server
calls have, and test that a large `rejects into` is refused at the limit.
