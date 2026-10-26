---
created-at: 2026-10-25
status: open
kind: feature
priority: 711
summary: "the operator log encoder maps keys through the heap layout, not the wire encoding"
---

# Feature: one wire value encoder

## Report

`catalog/oplog.rs` `encode` goes through the heap `FieldType`, so a `uuidv7` key would come out length-prefixed where the wire wants 16 raw bytes, and `ZonedDateTime` has no UTC offset in `Value`. Neither occurs in operator values today. Move the encoder to a module the server will share, following wire.md.
