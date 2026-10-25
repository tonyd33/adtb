---
title: "How an operator's preconditions meet concurrent writes"
pick: one
options:
  predicate-read:
    title: "Every precondition is a predicate read of the migration's transaction"
  snapshot-only:
    title: "Preconditions are tested on the migration's snapshot and not tracked"
---
Raised by the implementability review of the spec in docs/spec/.
Operators have preconditions on the data (a removed constructor is
empty, an added constraint is satisfied, a disabled constructor holds
no stored value). The spec did not say how such a test meets a write
that commits concurrently.

## spec/evolution/preconditions/predicate-read

Every precondition of an operator is a predicate read of the
migration's transaction, in the sense of the isolation semantics
(semantics §8.2). An insert into `identities.Apple` that commits
concurrently with `remove constructor Identity.Apple` makes the read's
matches change, so one of the two fails with a serialization failure.

## spec/evolution/preconditions/snapshot-only

The precondition is tested on the snapshot and not recorded as a read.
The concurrent insert above commits and the remove commits too,
leaving a row of a removed constructor. Not serializable.
