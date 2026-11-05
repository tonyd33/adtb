---
created-at: 2026-11-05
status: open
kind: feature
priority: 650
summary: "the Arrow IPC reader refuses a utf8 column holding a non-text field's textual syntax and a timestamp zone other than UTC, which interop.md 2.2's reader paragraph allows"
---

# Feature: Arrow reader conversions for external files

## Request

## Proposal

## Open questions

## Report

interop.md §2.2 says a reader converts a text column holding the field's textual syntax, and fails only per value. The S5.5a reader (crates/adtb/src/batch/ipc/schema.rs, the expected-against-stream check) refuses at schema check a utf8 column for a non-text field, and a timestamp whose zone string is not exactly "UTC". A dump restore never meets either; an external file read through `read_arrow` or the Arrow IPC file scan will.

Found in review of S5.5a, 2026-11-05.

## Proposal

When the Arrow IPC file scan lands: convert a utf8 column through the type's textual parser, per value, refusing per value; accept timestamp zones equal to UTC under any spelling the format allows (UTC, +00:00, Etc/UTC), and decide what a non-UTC zone means for `Instant`. Test with pyarrow-written files if pyarrow is available, else arrow-rs.
