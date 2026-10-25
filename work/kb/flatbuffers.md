# FlatBuffers

kind: format
home: https://flatbuffers.dev

## Unions

- A union field is two fields: a u8 `<name>_type` and an offset to a table; type 0 is NONE. Must sit in a table. Structs and strings as members, and vectors of unions, are experimental. [V 2026-10-25] https://flatbuffers.dev/schema/
