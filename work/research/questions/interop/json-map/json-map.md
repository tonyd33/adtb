---
title: How a sum value is written in JSON
pick: one
raised-by: [interop/formats/ndjson, interop/text/json]
options:
  external:
    title: An object with one key, the constructor name, holding the payload
  internal:
    title: The payload object with an added tag field
  adjacent:
    title: An object with a tag field and a payload field
---
Serde's three representations
(https://serde.rs/enum-representations.html). Records are objects in
every option; nullary constructors are strings in every option.

## interop/json-map/external

`{"Google": {"gid": "g-123"}}`. Unambiguous for any payload.

## interop/json-map/internal

`{"kind": "Google", "gid": "g-123"}`. Matches the jsonb-with-type-field
shape migrated systems already hold (`interop/migrate/patterns`). A
payload field named `kind` collides; the tag name must be reserved per
sum.

## interop/json-map/adjacent

`{"t": "Google", "c": {"gid": "g-123"}}`. No collision; verbose.
