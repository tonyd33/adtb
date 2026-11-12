---
created-at: 2026-11-11
status: open
kind: bug
priority: 200
summary: "work/tools scripts that build resolve the workspace from their own path, so main's copy run with a slot's CARGO_TARGET_DIR builds main's sources into the slot's target and cargo treats them as fresh"
tags:
  - tools
  - class-none
---

# A tool from one checkout builds into another's target

## Report

`work/tools/adtb-probe`, `script` and `conf` set `root` from `$(dirname "$0")/../..` and build from `$root/Cargo.toml`. Run as the main checkout's `work/tools/adtb-probe` from a slot with the slot's `CARGO_TARGET_DIR`, it built main's sources into the slot's `target/debug`. Cargo's artifact hashes do not depend on the checkout path, so the slot's next build took main's adtb-lang as fresh and `spec_corpus` failed to compile; a test could as easily have passed against main's code. Seen in the structure-lang-identity step 2 review; the reviewer removed the stale fingerprint.

Direction: a building tool refuses when `CARGO_TARGET_DIR` is set and is not under `$root`, naming both paths. Test in the tools' scratch-repo harness: run one checkout's tool with the other's target directory and expect the refusal.

Defect class: none.
