---
title: How a sum value in a result is encoded on the wire
pick: one
raised-by: [sum/value, sum/relation]
options:
  index-tag:
    title: A tag indexing the constructor list, then the payload
    links:
      - {rel: prior-art, to: prior/surface-spacetimedb}
  stable-tag:
    title: A tag carrying the constructor's stable id, then the payload
    links:
      - {rel: prior-art, to: prior/surface-capnp}
  name-tag:
    title: The constructor name as tag, then the payload
  field-presence:
    title: One present field per constructor, no separate tag
    assessment: {grade: costly, why: zero or several present fields are valid on the wire so the decoder checks totality}
    links:
      - {rel: prior-art, to: prior/surface-protobuf-oneof}
---
A query over `identities` returns rows of a sum; a value-level sum field
does the same inside a row. Each result needs the constructor and its
payload.

## surface/sum-encoding/index-tag

`Google { gid: "g-123" }` is `01 07 "g-123"`: tag 1 of
`[EmailPass, Google]`. SpacetimeDB BSATN, a `u8` tag. Adding,
removing or reordering constructors shifts tags.

## surface/sum-encoding/stable-tag

```
type Identity = EmailPass { ... } | Google { ... }
-- catalog constructor ids: EmailPass 1, Google 2
```

`02 07 "g-123"`. Renames and removals keep every other tag. Cap'n
Proto ordinals, protobuf field numbers.

## surface/sum-encoding/name-tag

`{"Google": {"gid": "g-123"}}`. Serde's externally tagged enum
(https://serde.rs/enum-representations.html). A rename breaks every
old reader.

## surface/sum-encoding/field-presence

Protobuf `oneof`: the member's field number is the only discriminant;
the last parsed wins.
