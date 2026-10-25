# openraft

kind: system
home: https://github.com/databendlabs/openraft

## Library

- Async Rust Raft (Tokio runtime by default); user implements log storage and `RaftStateMachine`; joint-consensus membership; snapshots; linearizable reads via `ensure_linearizable`, read index or leader lease; used by Databend; API unstable before 1.0. [V 2026-10-25] https://docs.rs/openraft/latest/openraft/
