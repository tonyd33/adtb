# Cargo build disk use

kind: system
home: https://doc.rust-lang.org/cargo/

## build-dir and target-dir

- `build.build-dir` (env `CARGO_BUILD_BUILD_DIR`) holds intermediate artifacts and defaults to `build.target-dir`; it takes the templates `{workspace-root}`, `{cargo-cache-home}`, `{workspace-path-hash}`. [V 2026-10-29] https://doc.rust-lang.org/cargo/reference/config.html
- `build.build-dir` was stabilized in Cargo 1.91 (2025-10-30). [V 2026-10-29] https://doc.rust-lang.org/cargo/CHANGELOG.html
- `-Zbuild-dir-new-layout` and `-Zfine-grain-locking` are still nightly-only; the 1.98 changelog still lists both under nightly. [V 2026-10-29] https://doc.rust-lang.org/cargo/reference/unstable.html
- Cargo 1.96 split the build-dir concurrency lock into a dedicated lock while keeping a shared lock on `.cargo-lock`. [V 2026-10-29] https://doc.rust-lang.org/cargo/CHANGELOG.html

## Shared cache and garbage collection

- There is no stable cross-workspace build cache. The issue is open, labelled S-needs-design, with prototypes in Sep 2026 and a design doc restart on 2026-10-06. [S 2026-10-29] https://github.com/rust-lang/cargo/issues/5931
- The 2026 project goal aims for a nightly cross-workspace cache first, stabilization later. [S 2026-10-29] https://rust-lang.github.io/rust-project-goals/2026/cargo-cross-workspace-cache.html
- Cargo's automatic gc covers only the global cache in CARGO_HOME; build artifact tracking is not implemented (cargo#13136). [V 2026-10-29] https://doc.rust-lang.org/cargo/reference/config.html
- `CARGO_INCREMENTAL=0` overrides the profile and config setting. [V 2026-10-29] https://doc.rust-lang.org/cargo/reference/config.html

## Debug info

- On ELF platforms `-C split-debuginfo` defaults to `off`, leaving DWARF in the executable; `packed` and `unpacked` also work on Linux on stable. [V 2026-10-29] https://doc.rust-lang.org/rustc/codegen-options/index.html
- Precedence: `[profile.X.package.name]`, then `[profile.X.package."*"]` (non-workspace only), then build-override, then the profile. The `test` profile inherits `dev`. [V 2026-10-29] https://doc.rust-lang.org/cargo/reference/profiles.html
- `-C link-dead-code` is off by default and `-C prefer-dynamic` affects only libraries with a dynamic form available. [V 2026-10-29] https://doc.rust-lang.org/rustc/codegen-options/index.html

## sccache

- sccache cannot cache crates that invoke the system linker (bin, dylib, cdylib, proc-macro) and requires incremental off to cache a crate. [V 2026-10-29] https://raw.githubusercontent.com/mozilla/sccache/main/docs/Rust.md
- `SCCACHE_BASEDIRS` strips absolute prefixes from the cache key; relative paths are an error. The disk cache default limit is 10G. [V 2026-10-29] https://github.com/mozilla/sccache/blob/main/docs/Configuration.md
- A second git worktree missed on all Rust compilations with sccache 0.14.0 because absolute paths remained in inputs, per one downstream report. [S 2026-10-29] https://github.com/NVIDIA/OpenShell/pull/2379

## Measured on adtb

- sccache 0.19.0 with Rust 1.98.1 and incremental on caches only dependencies in a 58 MiB cache: 62 of 204 compile requests were cacheable, and all 62 hit from a second path. [V 2026-10-30] local run, /home/tony/code/adtb-disk-budget
- With incremental off and `SCCACHE_BASEDIRS` covering both paths, local crates still missed from a second path: 11 further misses, no further hits, before the run stopped. [V 2026-10-30] local run, /home/tony/code/adtb-disk-budget
- Two cargo builds on one target directory block: "Blocking waiting for file lock on build directory". [V 2026-10-30] local run with cargo 1.98.0
