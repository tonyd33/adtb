---
created-at: 2026-10-28
status: open
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
