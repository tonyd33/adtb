---
extends: current
proposed: true
accept:
  spec/bytes-form/hex: {title: 0x followed by two lowercase hex digits per byte, reason: 'a byte reads off the text, and the empty value is not an empty cell'}
  spec/constraint-faults/accepted: {title: Partial bodies are accepted; a fault while checking fails the statement, reason: a fault while checking already fails the statement}
  spec/field-faults/any: {title: Any fault some field raises; the fields of one record or constructor value are not ordered, reason: which fault a value reports is no more fixed than which binding faults}
  spec/fix/bounded/element-flow: {title: Only where its result can become an element of a relation of the stratum, reason: a value only compared never enters the fixpoint}
  spec/fix/projection/not-constructing: {title: 'Not constructing: start, end, unwrap and value return a component of an argument', reason: a component of an argument is drawn from values already present}
  spec/fix/restriction/positive: {title: 'Positive: if e ▷ S is positive, so is e', reason: 'restriction keeps a subset of its operand, so it is monotone'}
  spec/key-preserving/params-and-groups: {title: Also the binder of a generator over a Set T parameter or over a group's rows, reason: 'such a binder ranges over distinct elements, so its select merges nothing'}
  spec/only/none: {title: No surface only; a row of the snapshot is reached by a path through its key or by match on the key, reason: 'no example needs it yet, and adding it later changes no program'}
  spec/take-negative/fault: {title: 'Faults with NegativeCount, as top does', reason: one rule for the counts of take and top}
  spec/textual-forms/interop: {title: 'interop.md §1 owns them: NaN, inf, -inf, ISO 8601 times, P{m}M{d}D periods; a query writes them as T.parse calls', reason: 'one part owns the forms, and the lexer gains no tokens for them'}
  spec/totality/add-period/partial: {title: 'Partial: it can fault with ArithmeticOverflow', reason: it faults with ArithmeticOverflow under either rule}
  spec/totality/at-block/partial: {title: 'Partial, because HistoryUnavailable is an abort', reason: 'HistoryUnavailable is an abort, and a hoisted block could reject a program that never reads it'}
---
