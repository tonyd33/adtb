---
created-at: 2026-10-28
status: resolved
kind: feature
priority: 1040
summary: "adtb has no Nix package, so nothing outside the dev shell can build or install its binaries and library"
closed: 2026-10-28
tags:
  - engine
---

# Feature: package adtb with Nix

## Report

The repository has a devenv shell and no flake outputs. Nothing builds adtb reproducibly outside that shell, so CI, users and SDK test harnesses cannot depend on a pinned adtb build.

## Request

- A flake exposing packages for the binaries (adtb-shell, adtb-codegen, adtb-probe, adtb-conformance, and the server once it has a binary) and a default package, built from `Cargo.lock` with the toolchain of `rust-toolchain.toml`.
- `nix build` and `nix run` work from a clean checkout; `nix flake check` builds the packages and runs the tests that need no machine-wide lock.
- The devenv shell and the flake share one nixpkgs pin.
- CI builds the package (`.github/workflows/`, setup through `.github/actions/setup`).
- Land with or before client-sdks: each SDK's end-to-end tests start a server from this package.

## 2026-10-28: dispatched

A fresh writer owns this in workspace nix-package on main 1eb25c53, with no Rust source changes: flake packages for the binaries, one nixpkgs pin with devenv, `nix flake check`, a CI workflow through .github/actions/setup with the Cachix token, and a docs/development.md section. Gates: `nix build .#`, `nix run .#NAME -- --help`, `nix flake check`, the dev shell still loads. Local brief: work/briefs/brief-nix-package.md.

## 2026-10-28: writer froze the source

Frozen 1e443e3aea6a64642bbc0cac88462b873f43689f on main 1eb25c53 (workspace nix-package): flake.nix, nix/workspace.nix, .github/workflows/ci.yml, a docs/development.md section, work/kb/cargo2nix.md. Builder rustPlatform.buildRustPackage; cargo2nix was considered and declined (four inputs, a committed generated Cargo.nix to regenerate on every Cargo.lock change, per-crate wiring of `../../` test data, small caching gain on a 91 s build). The flake has no inputs: it reads devenv.lock and fetches the locked devenv-nixpkgs with fetchTree, so `devenv update` moves both; nix/workspace.nix asserts nixpkgs' rustc equals rust-toolchain.toml. `nix flake check` builds the packages and runs the full nextest suite in the sandbox (2518 passed, 7 skipped; about 12 minutes on 6 cores). adtb-codegen and adtb-conformance have no `--help` flag. Running: a fresh review.

## 2026-10-28: landed

The independent review (work/briefs/review-nix-package.md, local) blocked on one finding: the tests check set `cargoCheckType`, which buildRustPackage overrides, so the suite ran in release. Fixed with `checkType = "test"` (log shows `--cargo-profile test`); the package now builds from Cargo.toml, Cargo.lock and crates only, and the docs say x86_64 Linux. Writer gates at 72733f70: `nix build .#` 133 s, `nix flake check` passed with the tests check 2518 passed, 7 skipped, 757 s. Landed as 6b9e357d with no Rust change; CI on GitHub runs the flake check. Follow-ups: adtb-codegen and adtb-conformance have no `--help` flag; an adtb-server package once it has a binary; crane for dependency caching if CI time hurts; the flake reads its nixpkgs from devenv.lock and should move to a flake.lock with a drift check once it needs a second input; aarch64-linux is listed but never built.

## 2026-10-28: resolved
