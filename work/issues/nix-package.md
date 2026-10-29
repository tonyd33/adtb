---
created-at: 2026-10-28
status: in-progress
branch: nix-package
kind: feature
priority: 1040
summary: "adtb has no Nix package, so nothing outside the dev shell can build or install its binaries and library"
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
