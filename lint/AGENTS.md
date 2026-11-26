# Lint rules

The rules `cargo xtask lint` runs with `tql`. `rules.toml` gives each query its include and
exclude globs, `*` within a path component and `**` across, and its rows, zero or ratchet;
`allowlist.toml` holds each ratchet row's count per file. The runner is `xtask/src/lint/mod.rs`.

- A query sees syntax, not types: it matches node kinds and their text. A name imported under an
  alias, a method on an inferred type or a call inside a macro escapes it;
  `capability-names.tql` catches the renames of the capability names. A query's header comment
  names each row and what it counts.
- A row a query emits that `rules.toml` does not declare is refused.
- Each row declares its `kind`, `why` (the harm it stops, one sentence) and `fix` (what to write
  instead, naming the rewrites that do not count as a fix). `cargo xtask lint` prints both once
  under each failing row's sites and refuses a row lacking either. A row two rules feed declares
  the same text in both. Reviewers judge a lowered count against the row's `fix`.
- A row's query counts every rewrite that keeps the access the row stops: a rename, an alias, a
  helper of another name, a macro, another spelling of the same call. A query's header comment
  lists the forms the row cannot see.
- A ratchet count equals its entry: lower the entry when sites go and delete it at 0; an entry
  at 0 is refused. Counts only fall: `--against` fails an entry above the allowlist at that
  revision, so a rise is a maintainer's act on main. `--counts` prints the allowlist for the
  tree.

Test a query on a fixture under `xtask/src/lint/fixtures/` with a test in
`xtask/src/lint/tests.rs`: `cargo nextest run -p xtask -E 'test(/^lint::/)'`. Run every rule
with `cargo xtask lint`.
