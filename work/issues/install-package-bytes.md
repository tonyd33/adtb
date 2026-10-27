---
created-at: 2026-10-26
status: open
kind: feature
priority: 875
summary: "install reads its package from Source::packages, which nothing in the engine fills, so every engine install is refused"
tags:
  - engine
  - lang
---

# Feature: fill install packages from package files

## Report

`install` in `adtb-lang` now parses the package named by its file from `Source::packages` (declaration text keyed by file name) and refuses "the package `f` is not available" when it is missing. The engine never fills `packages`, so any migration that installs a package is refused there. The `.adtx` format is unspecified (see `spec-evolve-gaps`), and the checker assumes the package is a module of declarations without its `module` line.

Fix: once the format is settled, read the package file's declarations and Wasm code in the engine, fill `Source::packages`, and record the code in `wasm_modules`.
