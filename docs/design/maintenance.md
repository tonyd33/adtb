# Maintenance of constraints and stored views

The constraint and view compiler turns each admitted constraint and
each stored view into incremental work over a transaction's change set.
The transactions layer runs that work before commit. The meaning of
each constraint is in [../spec/constraints.md](../spec/constraints.md);
the meaning of each stored view in
[../spec/derived.md §2](../spec/derived.md#2-stored-views).

## 1. One compiler

Stored view maintenance uses the compiler that checks constraints:
each view gets a delta query per input constructor relation, run over
the transaction's change set.

For every admitted constraint that reads a relation the change set
touches, the transactions layer runs the constraint's incremental check
against the change set and the states before and after it. Constraints
reading no touched relation are not checked.

Postcondition: an incremental check `δc` reads `Δ*`, probes of `D` and
`D*`, and returns a set. It is correct when, for every valid `D` and
every change set:

```text
δc(D, Δ*, D*) = ⟦c⟧(D, Δ*, D*)
```

The compiler emits only correct checks. A constraint whose query reads
no constructor relation that `Δ*` touches returns the same set as in
`D`, which is empty, so skipping it is correct.

Rulings: derived/ivm/delta-rules, constraint/enforce/delta, constraint/admit/bounded

## 2. Stored views

A stored view is held like a base relation, with heaps per constructor
and indexes ([mvcc.md §1](mvcc.md#1-one-heap-per-constructor)), and is
maintained in the transaction that changes its inputs.

Postcondition: at the commit of a transaction that changes a view's
inputs, the view's stored rows reflect the change set.

A set-valued view keeps a derivation count per stored row and drops a
row when its count reaches zero.

A bundle pinned before a view's redefinition reads the view by
evaluating the body of its own version, not the stored rows
([../spec/semantics.md §9.6](../spec/semantics.md#96-version-translation)).

## 3. Delta queries

Delta queries probe base relations through their indexes at the
transaction's snapshot and keep no other state.

Admission precondition: a view is admitted only when every join
reached from a delta has an index.

Rulings: derived/ivm/state/base-indexes

## 4. Maintained aggregates and range exclusions

A constraint's `count`, `sum`, `min` or `max` over a group whose key is
the leading fields of an index is maintained per group from each change
set, as an index is maintained. `min` and `max` probe the index on the
aggregated field within the group when a delete removes the current
extreme.

An exclusion under `overlaps` is checked by probing its `interval
index` with each written row's equality fields and range.

Rulings: constraint/aggregates/maintained, constraint/overlap/range-index

## 5. Commutative increments

Count and sum maintenance writes are commutative increments merged at
commit. The maintainer records no read, so two writers to one group do
not conflict unless one of them reads the aggregate.

Integer overflow of a merged value is detected at commit. The commit
whose increment would overflow the stored value fails and applies
nothing; the error it reports is stated in
[../spec/derived.md §2.4](../spec/derived.md#24-concurrent-maintenance).

## 6. Checking a large change set

A load's change set is checked at a cost independent of how the rows
arrived ([../spec/interop.md §4.4](../spec/interop.md#44-checking-a-large-change-set)).
Its constraint checks take SIREAD locks as any read does
([mvcc.md §8.2](mvcc.md#82-siread-locks)). The index maintenance a load
chooses is in [indexes.md §7](indexes.md#7-building-and-rebuilding).

The compiler must meet that cost bound this way:

- each constraint's delta query runs once over the whole change set,
  as one planned query (a reference check is one anti-join of the
  inserted rows against the target's key index), never once per row;
- a uniqueness into an index built for the load is checked while the
  index is built from sorted runs, and equal adjacent keys are its
  violations.

Rulings: interop/load/check/set-at-a-time, interop/load/check/build-time
