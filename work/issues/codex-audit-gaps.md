---
created-at: 2026-11-01
status: open
kind: bug
priority: 410
summary: "Low-severity gaps from auditing 9396ba9c..main: roster and runner holes, tool reach, stale docs"
---

# Codex audit gaps

## Report
An audit of 9396ba9c..main by reading found no wrong answer, lost data or lost test. It left these gaps.

- `crates/adtb-lang/src/check/tests/runtime.rs:394-474`: the probe roster suppresses dead_code, so deleting a `#[test]` stops its probes silently. `:64`: a new `foo.cases` is ignored where sibling drivers fail.
- `crates/adtb-conformance/src/lib.rs:474-501`: `query rowsort` ignores top-level shape, so a list passes for a set.
- `crates/adtb-reference/tests/writes/identity.rs:83`: `id != offset` replaced `id >= 1024`.
- NFC checker fixtures hold raw bytes. Normalising a file makes the test vacuous, with nothing to detect it.
- `crates/adtb-lang/src/translate/walk.rs:583`: a differently renamed field plus a new enabled constructor on pinned upsert `old` gives Internal. Not reproduced. `:553`: an `expect` replaced a refusal; no reachable input found.
- `work/tools/workspace-gc:64-74,135-147,213`: `release`, `loom`, `miri` and evidence directories are excluded from its SIZE column and reached by no mode. Measured: 47.9 GB of 86 GB.
- The `native` feature of adtb-bench is compiled by neither `check` nor CI.
- `work/tools/suite:33,45`: dropped `--workspace`, which shrinks silently if `default-members` is added.
- `work/tools/slot-test:308-311`: likely passes through its process scan and not the flock. Mutation not tried.
- `python3` is absent from `devenv.nix`, and the bench tools are absent from AGENTS.md.
- `check` builds the tree-sitter parser in `/tmp`, so a full `/tmp` fails it with `Disk quota exceeded` after the suite has passed. `check` should set `TMPDIR` under its target directory.
- The writes fixture driver has 32 modes for 78 families, 16 used once, and is twice the old text. It costs more on a schema change. Reduce it when next touched.
