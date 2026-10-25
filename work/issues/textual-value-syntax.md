---
created-at: 2026-10-25
status: open
kind: feature
priority: 240
summary: "values cannot be parsed from or printed to the textual value syntax of interop.md §1"
tags: [reference]
---

# Feature: textual value syntax

## Report

`docs/spec/interop.md` §1 defines one textual syntax for values and
versioned documents (§1.1, §1.2). No crate parses or prints it. The
reference interpreter and the conformance runner need both directions
to state inputs and expected outputs.
