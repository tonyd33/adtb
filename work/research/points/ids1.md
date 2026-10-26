---
extends: current
proposed: true
accept:
  spec/ids/engine-objects/fixed-table: {title: 'catalog.md §4.6 lists them per id space, below 1024, per catalog version', reason: the engine's types already take a fixed table; fixed ids per catalog version stay stable when the engine's declarations change}
  spec/ids/moved-field/keeps: {title: The moved field keeps its id; a move down gives each further constructor a new field, reason: 'a move keeps the field''s data, as a rename does, so it keeps its identity; a move down gives each further constructor a new field'}
  spec/ids/operator-ids/implied: {title: Only the first id is recorded; the rest follow from §1.2's order and the decl term, reason: the ids follow from the order of catalog.md 1.2 and the decl term; recording them again could only disagree}
  spec/ids/package/as-operators: {title: 'As if the package''s declarations were operators of the installing migration, after install, in package order', reason: one id space and no reuse follow from treating the package's declarations as operators of the installing migration}
  spec/ids/rescoped-index/new: {title: 'It is a new index with a new id, created after the operator''s constructors; the old one is dropped', reason: 'a re-scoped index may merge several into one, so no single id can survive; a new index with a new id is unambiguous'}
  spec/ids/view-type/anonymous-only: {title: 'store f creates a one-constructor type, after the view''s relation variable, only when the view''s element type is anonymous; a view of a declared type is a relation of that type', reason: store f creates a wrapper type only when the view's element type is anonymous; a view of a nominal type is a relation of that type}
---
