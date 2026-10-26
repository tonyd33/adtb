---
extends: current
proposed: true
accept:
  spec/agg-params/one: {title: Exactly one; several values pass as a record; a declaration with more is refused, reason: an aggregate takes exactly one parameter; several values pass as a record; a declaration with more is refused}
  spec/dump-damaged/offset: {title: 'DumpDamaged { offset: I64 }, the byte offset of the first part that fails to authenticate or decode', reason: 'a restore that cannot authenticate or decode a dump fails with DumpDamaged { offset: I64 }, the byte offset of the first part that fails'}
  spec/examples/self-contained/in-block: {title: Each block declares or restates every temp relation and type it uses beyond the running schema, reason: each block declares or restates what it needs; the corpus harness keeps no table of continuations}
  spec/ids/move-down/first-declared: {title: The listed constructor first in the type's declaration order; the others take new ids in that order, reason: a move down keeps the id on the first listed constructor in the type's declaration order}
  spec/ids/moved-field/keeps: {title: The moved field keeps its id; a move down gives each further constructor a new field, reason: 'a move keeps the field''s data, as a rename does, so it keeps its identity; a move down gives each further constructor a new field'}
  spec/ids/prelude-fns/none: {title: 'No: the prelude is not part of a schema, so its functions have no id and no row', reason: 'prelude functions have no catalog id and no row: the prelude is not part of a schema'}
  spec/ids/rescope/named: {title: 'Split, merge, nest and unnest re-scope an index whose clause names a replaced constructor; one with no in keeps its id', reason: 'unnest re-scopes indexes as split, merge and nest do; an index with no in is not re-scoped and keeps its id'}
  spec/ids/rescoped-index/new: {title: 'It is a new index with a new id, created after the operator''s constructors; the old one is dropped', reason: 'a re-scoped index may merge several into one, so no single id can survive; a new index with a new id is unambiguous'}
  spec/limit-reason/limit-sum: {title: 'StatementLimit carries limit: Limit = Time { after: Duration } | Memory { bytes: I64 } | Rounds { rounds: I64 }', reason: 'StatementLimit carries limit: Limit, with Limit = Time { after: Duration } | Memory { bytes: I64 } | Rounds { rounds: I64 }; the last is the fixpoint round limit'}
  spec/temp-relations/names/declared: {title: 'No name a module the text reads declares: relation, function, type or constructor', reason: 'a temp relation may not take any name a module the text reads declares: relation, function, type or constructor'}
  spec/temp-relations/sum-key/declared: {title: It must declare its key; one without a key clause is refused, reason: a temp relation of a sum with several constructors must declare its key}
  spec/writer-wait-cycle/fail-one: {title: One transaction of the cycle fails with a serialization failure; which one is the engine's choice, reason: 'a cycle of waits fails one of its transactions with a serialization failure, which one being a freedom'}
---
