---
created-at: 2026-10-27
status: open
kind: bug
priority: 856
summary: "a Registered handle keeps the engine, its file and lock alive after Database::close, so close returns Ok without closing the pager"
tags: [quality, engine, design]
---

# Database::close returns Ok while a Registered handle keeps the database open

## Report

`Registered` holds an `Arc<Engine<R>>` (`crates/adtb/src/api/registered.rs:351-355`). `Database::close` (`crates/adtb/src/api/mod.rs:236`) drops only its own Arc, so while any `Registered` is alive the pager is not closed and no checkpoint is taken, though close returns Ok.

Repro:

```rust
let r = db.registered("app")?;
db.close()?;
Database::open(path, ..)?;
```

Actual: close returns Ok, and the reopen fails on the file lock.

Expected: what `docs/design/server.md` (Registrations) states: "A registration lives as long as the handle that opened with it."

Fix: borrow `&Database` from `Registered`, or hold a `Weak` and fail calls after close with a closed error, or have close refuse while a `Registered` is alive. Whichever is chosen, state it in the contract and on `close`.

## 2026-10-27: ruling

A `Registered` holds a `Weak` reference to the engine. `Database::close` closes the pager whatever handles remain, and a call through a handle after close fails with a closed-database error. State it in `docs/design/server.md` section 4 in place of the current lifetime sentence.
