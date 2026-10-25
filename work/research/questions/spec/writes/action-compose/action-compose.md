---
title: "How several delete actions reaching one row combine"
pick: one
options:
  one-value:
    title: "Declarative: the gone keys close under cascade; each surviving referrer takes one new value from every action reaching it"
  per-action:
    title: "Each action builds its own replacement of the referrer"
---
Raised by the implementability review of docs/spec/. With two `clear`
fields on one row whose targets are both deleted, semantics §6.4
derived two different replacements of the row, a `KeyViolation`.

```adt
type Pair = { id: Key pairs, a: Option (Key users) on delete clear,
              b: Option (Key users) on delete clear }
-- deleting both users that one pair references
```

## spec/writes/action-compose/one-value

The gone keys are the least set containing the deleted keys and closed
under `cascade`. Every surviving pre-state row that references a gone
key takes one new value: deleted if a `cascade` reaches it; otherwise,
if a `retag` reaches it, retagged, with every `clear` field that the
target constructor still has set to `None`; otherwise every `clear`
field set to `None`, all at once. Two `retag` actions to different
constructors on one type are a schema error at declaration.

## spec/writes/action-compose/per-action

Each action derives its own replacement of the referrer, as semantics
§6.4 did. Two clears on one row, or a clear and a retag, become a
`KeyViolation` the author cannot avoid.
