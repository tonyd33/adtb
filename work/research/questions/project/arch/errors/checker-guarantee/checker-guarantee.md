---
title: What a broken checker guarantee does
pick: one
raised-by: [project/arch/errors/fatal-split]
options:
  crash:
    title: Crash and recover from the log, like every other invariant
  fail-statement:
    title: Fail the statement with an internal error; the process and the transaction stay up
---
The checker guarantees properties of a submitted program: the types and
normal forms that reach planning, evaluation and write evaluation. A
plan, an evaluator or a write evaluator that finds one broken has met a
checker bug, not damaged engine state. Invariants over engine state
(pages, log, latches, heap and index layout, the stored catalog) are
not in this question: they crash and recover from the log.

## project/arch/errors/checker-guarantee/crash

```
let Value::Int(n) = v else { panic!("checker guarantees an Int here") };
```

The process crashes and replays the log. One checker bug in one query
takes down every session.

## project/arch/errors/checker-guarantee/fail-statement

```
let Value::Int(n) = v else {
    return Err(Error::Internal("checker guarantees an Int here".into()));
};
```

The statement fails and its effects are discarded; the transaction stays
open and the process keeps running. A client receives the failure as an
`Internal { what: Text }` constructor of the statement error sum. Engine
state is untouched, so nothing needs recovery.
