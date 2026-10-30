# Cargo and nextest build time

kind: system
home: https://doc.rust-lang.org/cargo/

## Feature unification

- `resolver.feature-unification` is ignored on cargo 1.98.0: it prints "warning: ignoring `resolver.feature-unification` without `-Zfeature-unification`". [V 2026-10-30] local run, cargo 1.98.0
- `cargo -Zhelp` lists `-Z feature-unification` as "Enable new feature unification modes in workspaces". [V 2026-10-30] local run, cargo 1.98.0

## nextest

- `cargo nextest run --timings` is listed as unstable in nextest 0.9.146; `cargo test --no-run --timings` gives the same test profile build. [V 2026-10-30] local run, `cargo nextest run --help`
- `cargo nextest run` takes `--config KEY=VALUE`, passed to cargo, so a profile can be overridden per run. [V 2026-10-30] local run, nextest 0.9.146
