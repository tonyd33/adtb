---
created-at: 2026-10-27
status: open
kind: bug
priority: 896
summary: "a refused deployment command inside ad hoc text is passed back as a write::Error::Internal sentinel string, with the real error kept in a captured Option"
tags:
  - quality
  - engine
---

# A refused deployment command inside ad hoc text travels as an Internal sentinel string

## Report

`crates/adtb/src/api/mod.rs:585-604` stores the API error in a captured `refused` and returns `write::Error::Internal("a deployment command was refused")` to stop `statements_with`, then matches `(Err(_), Some(e))` and throws the sentinel away. It is correct only while nothing between the callback and the match inspects or rewraps `Internal`, and a real internal error raised after a refusal is lost.

Fix: give `write::Error` a variant that carries the refusal, or let the `Between` callback return the API error type.
