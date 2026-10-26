---
created-at: 2026-10-25
status: open
kind: feature
priority: 92
summary: "no module states what it owns, and the design README names no code entry points"
tags: [tooling]
---

# Feature: module docs and design entry points

## Report

Agents spend 2 to 6 calls per track learning crate layout. Give each module root and top-level file one `//!` line stating what it owns, in the behaviour register (for example "Hold one heap per constructor: pages, slots, versions, large values and the visibility map."). No item-level docs for navigation. Add an entry point column (module path and main types) to the boundary table in `docs/design/README.md` section 3. Run after lint-ratchet lands.
