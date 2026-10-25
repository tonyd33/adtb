# Schema changes

Evolution applies schema operators ([../spec/evolution.md](../spec/evolution.md))
as writes. The catalog stores the result and the planner replans
against it.

## 1. The catalog is data

The catalog is a set of relations in the engine's own model, read by
the query language and written by ordinary transactions
([../spec/catalog.md](../spec/catalog.md)).

Invariant: every transaction sees the catalog of its snapshot, so a
schema change is a write like any other.

A schema operator runs in a transaction and commits or rolls back with
the data writes beside it.

Rulings: engine/catalog-relations, engine/transactional-ddl

## 2. Staged states

Operators that need intermediate states pass through them as staged
states, each compatible with the last
([../spec/evolution.md §3.4](../spec/evolution.md#34-staged-changes)):

| Object | States |
| --- | --- |
| constructor added by `add constructor` | declared (readers can type it, no writes) then writable, then deprecated, then removed when empty; `disable` returns a writable constructor with no rows to declared |
| constructor of a value-level sum added by `add constructor` | the same states; while declared, no field of any relation stores a value of it |
| constructor created by `add type` | writable from the start |
| product of split, merge, nest or unnest | writable when every source constructor is writable |
| index | write-only, backfilled, readable |

The write state of each constructor is a row of `catalog::ctor_states`,
keyed by (relation variable, constructor).

Invariant: each state is committed and seen by every running
transaction before the next begins.

The user's transaction commits the first state; a background job runs
the rest and records its phase in the catalog.

Postconditions of a migration with staged jobs:

- The first state is one no query at version `n - 1` can observe; the
  migration is pending from its commit.
- The migration's schema version commits only when every staged job
  of the migration completes. The last state commits version `n`
  together with the logical effect of every operator of the migration
  and its `script` writes, computed at that commit's snapshot; only
  then is the migration appended to the log.
- A failed job discards the pending migration: the operator log is
  unchanged, the physical states the job committed are removed, the
  report names the operator and the rows that caused the failure, and
  no inverse operators are appended.
- Every check the migration's operators made when they applied reruns
  at the commit of its last staged job, and a failure discards the
  pending migration as a failed job does.

A host function must be bound before any staged job or plan needs it
([../spec/embedding.md §10](../spec/embedding.md#10-host-functions)).

Registered queries pinned to an older schema version keep running
through translation by the operators' inverses while the staged change
rolls out ([../spec/evolution.md §7](../spec/evolution.md#7-version-translation)).

A query `q` registered at version `n` and called at version `m` is
translated once into a query `q'` over `Σ_m`. Postcondition: the
translation is correct when `⟦q'⟧_{Σ_m}(D_m) = ⟦q⟧^{n}(D_m)` for every
valid `D_m`, with `⟦q⟧^{n}` as
[../spec/semantics.md §9.6](../spec/semantics.md#96-version-translation)
defines it.

TODO: the contract of the recheck pipeline for registered queries is undecided.

Rulings: engine/online-states, engine/ddl-online-txn/staged-job, engine/pinned-version

## 3. Log records

Schema operators and stage transitions are log records
([storage.md §5.1](storage.md#51-records)), applied in log order with
the data on recovery and on replicas.

Invariant: a replica applies the operator that declares a constructor
before any row of it, and a stage transition before any change that
depends on it.

A migration whose operators need staged jobs writes the record naming
its schema version only when its last staged job completes; a failed
job discards the pending migration, and the operator log is unchanged.

Applying a migration writes the schema version; the transactions layer
serializes it against the transactions that ran at the previous version
([mvcc.md §8.2](mvcc.md#82-siread-locks)).

Rulings: repl/schema/in-log
