# cargo2nix

kind: system
home: https://github.com/cargo2nix/cargo2nix

## Build model

- Generates a `Cargo.nix` from `Cargo.lock` that the project commits; the build imports it as `packageFun` and builds one derivation per crate. [V 2026-10-28] https://github.com/cargo2nix/cargo2nix README
- `makePackageSet` checks a hash of `Cargo.lock` against `Cargo.nix` unless `ignoreLockHash = true`, so every lockfile change needs `Cargo.nix` regenerated with the cargo2nix tool. [V 2026-10-28] https://github.com/cargo2nix/cargo2nix flake.nix comments
- The toolchain comes from rust-overlay, set by `rustChannel`/`rustVersion` or `rustToolchain`. [V 2026-10-28] https://github.com/cargo2nix/cargo2nix flake.nix comments

## Maintenance

- Flake inputs: nixpkgs (release-23.11), rust-overlay, flake-utils, flake-compat. [V 2026-10-28] https://github.com/cargo2nix/cargo2nix flake.nix
- Last release v0.12.0 on 2025-06-19, the previous v0.11.0 on 2022-05-14; last push 2025-06-19; 79 open issues. [V 2026-10-28] GitHub API repos/cargo2nix/cargo2nix
