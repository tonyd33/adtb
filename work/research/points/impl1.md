---
extends: current
proposed: true
accept:
  spec/adhoc/names/every-module: {title: 'Ad hoc text belongs to no module; an unqualified name resolves in every user module, then the prelude', reason: 'only an ambiguous name needs qualifying, and no session state is added'}
  spec/after/cursor-fields/name-order: {title: The fields correspond to the ordering keys in the code-point order of their names, reason: 'a record type has no written order, and most ordering keys have no name to borrow'}
  spec/anon-sum/one-ctor/one-ctor: {title: 'C { ... } alone is a one-constructor anonymous sum', reason: 'a record is already a one-constructor sum, so a lone constructor needs no second one'}
  spec/anon-sum/row-types/row-types: {title: An anonymous sum has row types T.C and T.(C | D) with the subtyping and joins of a declared sum, reason: 'without row types, q.C and a match binder over a query result would have no type'}
  spec/ctor/type-args/expected-then-fields: {title: Those fixed by the expected type and the fields' synthesized types; an unfixed one is an error, reason: 'Some { value: 1 } then needs neither an annotation nor written type arguments'}
  spec/examples/mixed-blocks/split: {title: Split it into an adt block and a text block, reason: 'a text block leaves its adt lines unchecked, and skip marks would be one more syntax'}
  spec/fix/typing/least: {title: Each binder's element type is the least type at which every body checks against Set of it, reason: 'it fixes which programs are accepted without an annotation, a bottom type or one inference procedure'}
  spec/group/binder-type/binder-only: {title: 'Group K T is only the type of a group binder; passed on, a group is its nonempty Set T', reason: 'a function then receives an ordinary set, and no aggregate ranges over a caller''s group'}
  spec/join/no-expected/least: {title: The least type T such that every arm checks against T; none is an error, reason: 'if c then None else Some { value: 1 } types without an annotation'}
  spec/lambda/params/curried: {title: 'fn (a: T, b: U) -> e has type T -> U -> R; a call applies its arguments in order', reason: 'a named fn with several parameters can be passed, and arguments stay out of record rules'}
  spec/pattern/named-set/like-ctor: {title: 'The payload forms of a constructor pattern, over the fields every member shares', reason: 'a named set reads like a constructor in the same match, with no value bound only to be ignored or projected'}
  spec/pattern/redundant-alternative/refused: {title: 'Refused, as an unreachable arm is', reason: a dead alternative is as much a mistake as an unreachable arm}
  spec/top/operand/comprehension: {title: The operand is a comprehension; the keys are written over its select scope, reason: 'the select scope binds the keys, so no new binder form is needed'}
  spec/type-args/stop/clause-words: {title: A type argument list stops before a contextual clause word and before a name followed by (, reason: 'clause words stay usable as names, and generic relation types need no parentheses'}
---
