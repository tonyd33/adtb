---
created-at: 2026-10-25
status: open
kind: feature
priority: 315
summary: "there is no real runtime: no real clock, threads or entropy behind the runtime interfaces"
tags: [sim]
---

# Feature: real runtime

## Report

The runtime interfaces have only the deterministic simulator behind
them. A real runtime needs a clock, threads and entropy. Entropy needs
`getrandom` or `libc` approved in `deny.toml`.
