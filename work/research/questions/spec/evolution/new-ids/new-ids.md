---
title: "Which ids the constructors an operator creates take"
pick: one
options:
  listed-order:
    title: "In listed order, after the operator's other created objects; every product and re-added constructor takes a new id"
  unspecified:
    title: "Each operator takes fresh ids in an order it does not state"
---
Raised by the implementability review of the spec in docs/spec/. Ids
are a function of the log, but the order in which one operator's new
constructors take ids was stated only for `add type`.

## spec/evolution/new-ids/listed-order

New constructors take ids in listed order, after the operator's other
created objects. This covers the products of split, merge, nest and
unnest and a constructor re-added by an inverse, which takes a new id;
ids are never reused:

```adt
-- next id 1050
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text } by match ...
-- Google 1050, Apple 1051
```

## spec/evolution/new-ids/unspecified

Each operator takes fresh ids without a stated order. Two tools that
replay the log can assign different ids, so code generation and the
database disagree on wire tags.
