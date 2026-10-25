---
extends: current
proposed: true
accept:
  spec/catalog/physical/per-ctor-relation: {title: Keyed by constructor relation; fields naming physical detail are Text with implementation-defined content, reason: heap-keyed rows with typed physical fields would make the storage layout a promise}
  spec/index/kinds/declared-capabilities: {title: The index forms a schema declares and what each lets a query do; not the structure behind them, reason: promising ordered trees would rule out hash indexes and tie the spec to one structure}
  spec/quarantine/index/cost-only: {title: 'No read fails: queries do not use the index, so only cost changes; a write to a constructor relation the index covers fails', reason: whether a read runs must not depend on which plan the planner picks}
  spec/reads/static/static-types: {title: The constructors in the types of the relation expressions of the normal form; parameters and pruning do not change it, reason: whether a call fails under quarantine must not depend on the planner or on its arguments}
  spec/tiers/three-tiers: {title: 'Three tiers: docs/spec what users observe, docs/design contracts between subsystems, the rest with the code', reason: annex text in the spec is read as a promise and couples the spec to one implementation}
---
