---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 867
summary: "a spill past spill_bytes and any I/O error on the spill file both come back as StatementLimit Memory with the grant's limit"
closed: 2026-10-27
tags:
  - quality
  - engine
---

# A spill file I/O error is reported as exceeding the memory grant

## Report

`Spill::full` at `crates/adtb/src/exec/spill.rs:57-62` returns `StatementLimit(Exceeded::Memory { limit: grant })`. It is used both for the `spill_bytes` bound (`:83`, `:91`) and through `map_err(|_| self.full())` for creating the spill directory, opening and writing the file (`:67`, `:69`, `:100`).

- ENOSPC, EACCES or EIO on the spill file is discarded and reported as a statement limit.
- Exceeding `spill_bytes` reports the grant's byte count, not `spill_bytes`.
- `a_build_past_its_grant_with_no_spill_space_is_rejected_for_its_statement_limit` (`crates/adtb-sim/tests/exec.rs`) asserts only that the error is a limit, so it cannot tell the cases apart, and no test injects a fault on the spill file.

Fix: add an `Exceeded` variant for the spill bound carrying `spill_bytes`, mapped in `crates/adtb/src/api/error.rs`; pass I/O errors through as I/O errors; add sim tests for a write fault on the spill file and for a nonzero `spill_bytes` overflowing.

## 2026-10-27: done

`Exceeded::Spill { limit, grant }` for a spill past `spill_bytes`, mapped to `Limit::Memory { bytes: grant }` as the spec states; spill file I/O errors are `exec::Error::Spill(IoError)` and `write::Error::Spill`, reported as `Fatal { reason: Storage }`; a failed read of a spilled extent is `Spill`, a short one stays `Corrupt`. Tests: a spill past a 4 KiB bound, and a spill file whose writes fail.

Open: the runtime's `IoError` cannot tell ENOSPC from EIO, and the spec says a call with no spill space to spare is `StatementLimit`; a full spill device is reported as `Storage`.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qexec track; what stays open is in `exec-scan-follow-ups`.
