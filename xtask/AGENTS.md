# xtask

The `cargo xtask` commands: `check`, `test`, `lint` and their stages.

- Build every cargo call with `Workspace::cargo`, or `Workspace::cargo_of` for another cargo
  such as the nightly one Miri runs under. Both pass `--locked`, so a stale `Cargo.lock` fails
  the check instead of being rewritten. `Workspace::fmt` is the one call without it: `cargo fmt`
  resolves no dependencies and refuses `--locked`. `test::reuse` runs nextest over a listing
  `test::build` wrote, builds nothing and cannot take `--locked`.
  The lint query `lint/cargo-calls.tql` finds a cargo command made any other way, `Workspace::find`
  aside, which runs before a `Workspace` exists and passes `--locked` itself. It catches only a
  program written as the word `cargo`, a literal or a variable so named; review catches any other
  spelling.
- The `xtask` and `conformance` aliases in `.cargo/config.toml` pass `--locked` too.
