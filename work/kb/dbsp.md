# DBSP

kind: paper
home: https://docs.feldera.com/vldb23.pdf

## Incremental view maintenance

- Budiu et al., "DBSP: Automatic Incremental View Maintenance for Rich Query Languages", PVLDB 16 (VLDB 2023), 1601-1614; best paper. [S 2026-10-25] https://docs.feldera.com/vldb23.pdf
- Incrementalizes full relational algebra over sets and bags, nested relations, aggregation, flatmap, monotonic and non-monotonic recursion, stratified negation. [S 2026-10-25] https://docs.feldera.com/vldb23.pdf


## Model (read from text)

- Database modelled as a stream of snapshots indexed by transaction count, not wall-clock time; incremental query is `Q^Δ = D ∘ ↑Q ∘ I` (differentiate, lift, integrate). [V 2026-10-25] https://docs.feldera.com/vldb23.pdf
- Only 4 operators; results machine-checked in Lean. [V 2026-10-25] https://docs.feldera.com/vldb23.pdf
- Linear time-invariant operators are their own incremental version (Thm 3.3); bilinear ones (join, product) expand to `Δa×Δb + a×Δb + Δa×b`; chain rule incrementalizes a composite query operator by operator (Algorithm 4.6). [V 2026-10-25] https://docs.feldera.com/vldb23.pdf
- Z-sets: finite maps from rows to integer weights, possibly negative; a set has weight 1 per row; `distinct` keeps positive-weight rows at weight 1; union is `distinct(a+b)`, difference `distinct(a-b)`. Incremental distinct does work proportional to the change (Prop 4.7). [V 2026-10-25] https://docs.feldera.com/vldb23.pdf
- Cost: linear operators store nothing; incremental distinct, join, I and D store O(|DB|), their input or output in full; bilinear ops cost O(|DB|·|ΔDB|) unless indexed. [V 2026-10-25] https://docs.feldera.com/vldb23.pdf
- SUM is linear (O(1) per change); MIN is not and needs the full set, O(|DB|) brute force or O(log |DB|) amortized with a custom operator (Reactive Aggregator). [V 2026-10-25] https://docs.feldera.com/vldb23.pdf
- An IVM plan is fixed before data arrives and runs for all future updates; optimality is harder than for ad hoc queries. [V 2026-10-25] https://docs.feldera.com/vldb23.pdf
