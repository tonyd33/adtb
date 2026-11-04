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

## 2026-11-04: dumps carry no blobs

Dump S5.2 found nothing in the engine can name a blob (`install` and `adopt tzdb` have no packages to take, a Wasm function is refused), so a dump writes no `blobs/` and a restore treats any `blobs/` path as manifest damage. When install lands, add `blobs/` to dump and restore per interop.md 6.1, each blob under its BLAKE3-256 hash.
