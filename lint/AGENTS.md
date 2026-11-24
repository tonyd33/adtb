# Lint rules

The rules `cargo xtask lint` runs with `tql`. `rules.toml` gives each query its include and
exclude globs, `*` within a path component and `**` across, and its rows, zero or ratchet;
`allowlist.toml` holds each ratchet row's count per file. The runner is `xtask/src/lint/mod.rs`.

- A query sees syntax, not types: it matches node kinds and their text. A name imported under an
  alias, a method on an inferred type or a call inside a macro escapes it;
  `capability-names.tql` catches the renames of the capability names. A query's header comment
  names each row and what it counts.
- A row a query emits that `rules.toml` does not declare is refused.
- A ratchet count equals its entry: lower the entry when sites go and delete it at 0; an entry
  at 0 is refused. Counts only fall: `--against` fails an entry above the allowlist at that
  revision, so a rise is a maintainer's act on main. `--counts` prints the allowlist for the
  tree.

Test a query on a fixture under `xtask/src/lint/fixtures/` with a test in
`xtask/src/lint/tests.rs`: `cargo nextest run -p xtask -E 'test(/^lint::/)'`. Run every rule
with `cargo xtask lint`.
