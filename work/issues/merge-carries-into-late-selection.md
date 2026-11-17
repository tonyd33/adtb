---
created-at: 2026-11-16
status: open
kind: bug
priority: 500
summary: "a merge without by carries a Key rs field into a field typed Key rs.Settled when the set is added after the merge, while the same migration with the set first is refused"
tags:
  - lang
  - quality
  - class-admission-per-path
---

# A merge carries a key into a selection that resolves at the migration's end, past the check a set added first gets

## Report

Found by the quality review of reviewed..main (c93c399e..92dbbc64) on main 92dbbc64.

crates/adtb-lang/src/evolve/ctor/merge.rs:297 carries a source field into the merged constructor by name when `g.ty.alike(&f.ty)`, and `Ty::alike` (crates/adtb-lang/src/types.rs:439) counts `KeyCtors::Late` equal to `KeyCtors::All`. evolution.md states a selection resolves at the migration's end, and that a merge without `by` maps fields by name and type and refuses a field with no counterpart. The end-of-migration recheck rebuilds declarations but does not decide the carry again. The writer gates of d610a136 (script, fill, routing) do not cover the merge's carry.

Repro as a sim script run through the scripted binary (`ADTB_SCRIPTS=FILE target/debug/deps/scripted-HASH --exact scripts::by_hand`):

```
seed 1

migrate
  module m
  add module m
  add type R { id: Key rs } = Wild | Town
  add relation rs : R key id generate sequence
  add type P { id: Key ps } = A { home: Key rs } | B { home: Key rs }
  add relation ps : P key id generate sequence
  add type Kind = A | B
  merge constructor P.(A | B) into M { kind: Kind, home: Key rs.Settled }
  add set Settled of rs = Town

command
  print schema
```

It applies, and `print schema` shows `type P { id: Key ps } = M { kind: Kind, home: Key rs.Settled }` with `set Settled of rs = Town`. With `add set Settled` moved before the merge, the same migration is refused: field `A.home` has no counterpart in `M`. A `Key rs` row holding a `Wild` key would be carried into a `Key rs.Town` field.

Not live in the engine: a late selection arises only on a relation added in the same migration, which holds rows only through a script, and the engine refuses every script.

Fix direction: refuse a by-name carry into a target field whose type holds a late selection, naming the merge as the writer, or compare against the selection the migration ends with. Test both operator orders. Check split's carries the same way.
