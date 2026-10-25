---
title: How query results, writes and isolation are tested beyond storage
pick: some
options:
  differential:
    title: Random queries and databases, engine checked against the reference
    needs-one-of: [project/oracle/rust-reference, project/oracle/prover-model]
    links:
      - {rel: prior-art, to: prior/project-cedar}
      - {rel: prior-art, to: prior/project-formal-sql}
  ctor-partition:
    title: A query over a sum equals the union of the same query per constructor
    links:
      - {rel: prior-art, to: prior/project-sqlancer}
  tlp:
    title: A query equals the union of its restrictions to p and not p
    links:
      - {rel: prior-art, to: prior/project-sqlancer}
  plan-toggle:
    title: Every query gives the same result with each rewrite disabled
    links:
      - {rel: prior-art, to: prior/project-sqlite-testing}
      - {rel: prior-art, to: prior/project-sqlancer}
  pqs:
    title: A synthesised query must return a chosen pivot row
    links:
      - {rel: prior-art, to: prior/project-sqlancer}
  front-fuzz:
    title: Coverage-guided fuzzing of parser, type checker and wire decoder
    links:
      - {rel: prior-art, to: prior/project-sqlite-testing}
  isolation-check:
    title: Histories of concurrent transactions checked for Adya anomalies
    links:
      - {rel: prior-art, to: prior/project-elle}
  smo-laws:
    title: Property tests of schema operator inverses and version translation
    links:
      - {rel: prior-art, to: prior/prism-bidel}
---
`buffer/test` settles storage: deterministic simulation with injected
faults, a fault-injecting file system, crash-prefix replay, all on the
simulated scheduler. Those show the engine survives faults. They do not
show a query returns the right rows. SQLancer's oracles found hundreds
of logic bugs in mature engines; most need adapting, since there is no
NULL and the language has sums.

## project/testing/differential

```
seed → (schema S, database D, query q)
assert engine(q, D) == reference(q, D)     // as sets, nested as nested sets
```

Cedar's DRT; Guagliardo and Libkin's 100,000 random queries.

## project/testing/ctor-partition

A metamorphic relation native to this model: the sum is the disjoint
union of its constructor relations (`query/union-all`), so

```
from i in identities where p(i) select f(i)
  ==  (from i in identities.EmailPass where p(i) select f(i))
    ∪ (from i in identities.Google where p(i) select f(i))
```

The left side goes through the union index and merge-append; the
right through per-heap scans and constructor pruning. The same holds
for each `match` against its arms. Query Partitioning with constructors
as the partition.

## project/testing/tlp

TLP's three partitions (p, NOT p, p IS NULL) become two: no NULL and
`Bool` is a two-constructor sum. Also partitions on `group` keys and
aggregate decomposition (`count(x for x in g) == count(x for x in g where p(x)) + count(x for x in g where not p(x))`).

## project/testing/plan-toggle

SQLite reruns its whole suite with optimizations disabled. NoREC gets
the same effect by rewriting a query so the optimizer cannot touch it.
Here the engine has the switches: arm distribution
(`exec/join/cost`), constructor pruning (`exec/prune/compile`), probe
fusion (`exec/probe/fuse`), compiled vs interpreted (`exec/model/adaptive`).

```
for flags in subsets(rewrites): assert run(q, D, flags) == run(q, D, none)
```

## project/testing/pqs

Pick a row, generate a predicate, rewrite it to be true on that row,
assert containment. Needs an evaluator for expressions, which the
reference already is.

## project/testing/front-fuzz

cargo-fuzz on bytes for crash-freedom: malformed queries are refused
with a typed error, never a panic. SQLite's dbsqlfuzz mutates the query
and the database file together.

## project/testing/isolation-check

Elle infers dependency cycles from what clients observed and is sound
for the histories its workloads produce. The settled promise is
serializability (`engine/serializable-only`), so any G0, G1 or G2 cycle
is a bug.

## project/testing/smo-laws

For every operator with an inverse: `inv(op)(op(D)) == D`; for a query
`q` pinned to version v and its translation `q'` at v+1:
`q(D) == q'(op(D))`. A declared loss weakens the first law to the
declared projection.

```
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text }
  by match
    | OAuth { provider: Google, sub } -> Google { gid: sub }
    | OAuth { provider: Apple, sub }  -> Apple { sub: sub }
-- law: the inverse merge, keeping the tag as `provider`, restores D
```
