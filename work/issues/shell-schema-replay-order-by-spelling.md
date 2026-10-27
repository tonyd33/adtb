---
created-at: 2026-10-27
status: open
kind: bug
priority: 872
summary: "the shell orders a schema file's declarations by matching token spellings against declared names, so a fn named like a field is placed before the relation it reads and the create is refused"
tags: [quality, lang]
---

# A schema file whose fn shares a name with a field is refused by --schema

## Report

`order` and `place` at `crates/adtb-shell/src/schema.rs:48-90` decide the replay order of a schema file's declarations by lexing each one and adding an edge for every `Lower` or `Upper` token whose text equals a declared name, whatever it is: a key field, a binder, a record field. Inside a cycle `place` emits members in visiting order.

`s.adt`:

```adt
module demo
relation pets : Pet key name
type Pet = { name: Text }
fn name () -> I64 = count(1 for p in pets)
```

```
adtb --create --schema s.adt db -c 'from p in pets select p'
```

Actual:

```
db: migration 1 is refused: no `pets` in scope
```

Expected: the create succeeds and the query prints no rows, as it does when the fn is renamed `npets`.

Fix: order by the names the checker resolves (the dependency edges of the schema build or resolve pass), not by lexed spellings, and add the repro as a shell test.

## 2026-10-27: ruling

The shell does not order declarations itself. The checker supplies the dependency order of a schema file's declarations, through a call stated in `docs/design/kernel.md`, and the shell replays in that order.
