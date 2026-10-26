---
extends: current
proposed: true
accept:
  spec/audit-relation/seq: {title: 'relation catalog::audit : AuditEvent key seq', reason: 'catalog::audit is declared relation audit : AuditEvent key seq'}
  spec/catalog-key-values/declared: {title: 'The relation''s declared key type: I64 for a catalog id, Bytes for catalog::wasm_modules', reason: '.key(n) of a catalog or sys relation has that relation''s declared key type: Bytes for catalog::wasm_modules, I64 where the key is a catalog id'}
  spec/catalog-refs/key-value: {title: 'They hold the object''s key value, an I64 catalog id, not a Key; the engine keeps their integrity', reason: 'the model rule stays absolute; those fields hold the target''s key value, not a Key; the engine keeps their integrity, and a reader joins on them'}
  spec/late-relation-state/type-state: {title: 'The constructor''s state in its type, from catalog::value_states', reason: a relation added after its type gained a constructor takes the type-level write state for that constructor}
  spec/record-add-ctor/allowed: {title: 'Allowed, as on any sum: a record is a one-constructor sum; registered queries are rechecked', reason: 'a record is a one-constructor sum; registered queries are rechecked, and the migration is refused if one fails'}
---
