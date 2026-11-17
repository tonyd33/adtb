---
created-at: 2026-11-17
status: open
kind: bug
priority: 700
summary: "a Wasm function's identity across a migration is decided by module name and export, where interop.md 9.5 states the BLAKE3 hash of the module bytes"
tags:
  - lang
  - class-identity-by-spelling
---

# Wasm identity by name

## Report

Found by the fn-dependents review (2026-11-17). `same_code` in crates/adtb-lang/src/depends.rs compares a Wasm function by module file name and export; interop.md §9.5 makes its identity the BLAKE3 hash of the module bytes, and the schema holds no bytes or hash. Latent: the engine refuses Wasm functions at migrate today (crates/adtb/src/catalog/migration.rs).

Expected: the checked schema carries the module hash and `same_code` compares it.
