---
created-at: 2026-10-31
status: open
kind: bug
priority: 510
summary: add unique on a named set fails migration-log resolution
tags:
  - quality
  - engine
  - lang
---

# Named set index scope resolver

## Report

An accepted migration adding a unique over a named constructor set
panics while the catalog resolves its logged operands. This was
executed in the hidden-conflict test fixture; it is not introduced by
that change.

The relevant migration declares `Note` with `Plain` and `Other`, a
relation `notes`, then:

```adt
add set Shared of notes = Plain
add unique (slug) in notes.Shared
```

`Database::open_in` reaches `crates/adtb/src/catalog/oplog.rs:27` and
panics with `every logged migration resolves`, wrapping the diagnostic
`an object this operator creates or names is gone by the migration's
end` for the `add unique` operator. The already-existing named set is
valid and should resolve without a panic.

`crates/adtb-lang/src/evolve/resolved.rs` handles `AddUnique` and
`AddIndex` together around lines 575-584. It flattens scope identifiers
with `sel_idents` and passes each spelling to the constructor lookup.
The named set `Shared` is therefore treated as a constructor. The same
code path for `add index` is affected by reading; only the unique case
was executed.

Class: catalog identity by spelling and copied scope rules. Resolve a
constructor selection through the shared scope resolver into typed
constructor identities, retaining its named-set meaning where the log
contract requires it. Exercise public open/apply, not just a low-level
lookup, and retain an exact accepted result plus index/unique coverage.

## 2026-10-31: clarify the executed entry point

The initial report overstates the executed entry point. The failure
occurred in the bundle fixture's `hash` helper while it called
`catalog::version_hash`, before `Database::open_in` was invoked.
`Source::migrate` and `Source::schema` had accepted the migration;
log resolution then failed as described. A panic through public open
itself remains unconfirmed and must not be claimed from this run.

The resolver defect is measured; its propagation through public
open/apply still needs an executable probe. Some API paths use
`try_version_hash`, while catalog rows and registration also call the
panicking wrapper. Trace and run the user entry point before choosing
the error-boundary fix.

A later inline-unique-before-Shared workaround was correctly refused:
evolution section 3.2 permits forward named sets specifically in Key
reference selections, not arbitrary index scopes. That refusal is not
another defect.
