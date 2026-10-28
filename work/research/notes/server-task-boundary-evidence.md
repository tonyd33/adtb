---
title: Current engine evidence for the server task boundary
---
Repository inspection at ef7361f9, not an external-system comparison:

- crates/adtb/src/exec/sched.rs:10 defines FnOnce tasks; submit:60 queues
  without admission, work:95 executes to completion. Existing scheduler is
  reusable machinery, not yet a resumable call contract.
- crates/adtb/src/mvcc/wait.rs:51-85 parks the current runtime thread until
  the other transaction ends. An interactive transaction may require a later
  network call to commit; a separate bounded blocking pool needs a liveness
  proof, not an assumption that more threads suffice.
- crates/adtb/src/api/text.rs:21-46 stores DynRows in Vec::IntoIter;
  api/arrow.rs:69-114 similarly materializes Arrow batches.
- crates/adtb/src/api/mod.rs:521 uses pool:None; current public embedding
  represents the owner. Role-scoped server authority is a distinct contract.
- docs/design/server.md:6-12 requires calls on the same engine scheduler;
  wire.md sections5.5/5.7 require genuine credit suspension and cancellation.

Static assessment and alternatives are retained in the server-engine-interface
ticket. No new runtime or authentication dependency has been evaluated here.
Names in the question's pseudocode are illustrative. The exact Rust API still
requires a single provider owner and review before transport consumers branch.
