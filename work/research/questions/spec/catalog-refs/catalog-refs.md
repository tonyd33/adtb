---
title: "How the catalog's value-level sums name catalog objects"
pick: one
options:
  key-value:
    title: "They hold the object's key value, an I64 catalog id, not a Key; the engine keeps their integrity"
  exempt:
    title: "They hold Key fields, the engine's declarations exempt from the reference rule"
  relation-level:
    title: "They become relation-level sums with their own relation variables"
---
Raised by checking the engine's own declarations. catalog.md §2.4
declared `Operator`, `ClauseBody`, `CtorTarget`, `Place` and
`ArmTarget` with `Key` fields, and `MigrationOp.op: Operator` uses one
as a field type, which model.md §8.2 forbids: a reference is a direct
field of a relation's element type.

## spec/catalog-refs/key-value

Each field that names an object holds its catalog id:

```adt
type Place = Last | Before { ctor: I64 } | After { ctor: I64 }
```

The model rule stays absolute. The engine keeps every id naming a row,
and a reader joins on it with `catalog::constructors.key(p.ctor)`.

## spec/catalog-refs/exempt

The fields stay `Key catalog::r`, and the engine's declarations are
exempt from model.md §8.2, as they are from the qualified-name and
`volatile` rules. A reference inside a value would need foreign-key
checks the model has no form for.

## spec/catalog-refs/relation-level

Each such sum becomes a relation of its own, so its references are
direct fields. It multiplies the catalog's relations and splits the
operator log across them.
