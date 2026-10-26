---
extends: current
proposed: true
accept:
  spec/default-field-rows/per-field: {title: 'One row per field the default fills, each with the clause''s text', reason: 'each field''s default is found by its own id, and changing one constructor''s field touches only its row'}
  spec/unit-print/parens: {title: The unit value prints as `()`, reason: 'the literal, the type and every surface mention spell it (), and equal values print identically'}
---
