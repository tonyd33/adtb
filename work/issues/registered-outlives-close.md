---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 856
summary: "a Registered handle keeps the engine, its file and lock alive after Database::close, so close returns Ok without closing the pager"
closed: 2026-10-27
tags:
  - quality
  - engine
  - design
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

## 2026-10-27: done

`Registered` holds a `Weak` engine and an `Arc` of its checked registration, so `name` and `ordinal` still answer after close. `Database::close` waits for running calls, then closes the pager and releases the lock whatever registrations and clones remain; every later call through any of them, `begin`, `registered` and a transaction's calls and commit included, is refused with `Invalid` naming the database closed. Stated in `docs/design/server.md` §4. Tests in `crates/adtb-sim/tests/embedding.rs` and `api_real_file.rs` (`a_closed_database_holds_no_file_open` now expects a clone not to keep the file open).

Still open: the spec's error sums have no closed-database reason; `Invalid` stands in.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the embed track. What stays open is in `embedded-open-follow-ups`, `host-fns-in-keys`, `spec-closed-database-reason` and `conformance-refused-schema`.
