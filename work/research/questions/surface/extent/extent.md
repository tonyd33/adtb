---
title: Whether a relation-level sum's type and its extent are declared apart
pick: one
raised-by: [sum/relation]
options:
  fused:
    title: Declaring the sum declares its one stored extent
    links:
      - {rel: prior-art, to: prior/gel}
  separate:
    title: A sum is a type; relations of that type are declared apart
    links:
      - {rel: prior-art, to: prior/third-manifesto}
---
A sum used as a field type is a value type; a relation-level sum is an
entity set with heaps, keys and FKs. One keyword or two, and whether
one type may have several extents.

## surface/extent/fused

```
sum Shape = Circle { r: Float } | Rect { w: Float, h: Float }
entity Identity key id = EmailPass { ... } | Google { ... }
```

`entity` declares the type and its heaps together, as a Gel type is
its table. A value sum never has an extent.

## surface/extent/separate

```
type Identity = EmailPass { ... } | Google { ... }
relation identities          : Identity key id
relation archived_identities : Identity key id
```

Tutorial D relvars: the type is a value type, each relation variable
gets its own per-constructor heaps, keys and indexes. A constructor
relation is named through its relation variable, `identities.Google`,
and a constructor set as `identities.(Google | Apple)`. There is one
declaration form for types, `type`; a record type is a type with one
constructor, written without alternatives.
