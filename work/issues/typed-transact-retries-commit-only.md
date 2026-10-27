---
created-at: 2026-10-27
status: open
kind: bug
priority: 793
summary: "the typed Bundle::transact retries a serialization failure only at commit, not one a call inside the closure returns"
tags:
  - quality
  - engine
---

# The typed Bundle::transact retries only a serialization failure at commit

## Report

`Bundle::transact` (`crates/adtb/src/api/typed.rs:633-655`) runs `f(&mut t)?` and retries only when `t.txn.commit` fails with `is_serialization()`; an error from `f` goes straight back. The untyped `AdHocTransaction::transact` (`crates/adtb/src/api/txn.rs:120-143`) retries a serialization failure from either `f` or commit. Under SSI a failure often surfaces at a call inside the transaction, so a typed `transact` whose `Transaction::call` meets one fails after one attempt whatever `attempts` says. Root cause: `f` returns `B::Error`, which `transact` cannot inspect.

Repro: two typed `transact` blocks that read and write the same row concurrently under the simulator; the loser fails inside `f` and is not rerun.

Fix: after `f` fails, check whether `t.txn` ended on a serialization failure (its `State::Ended` error) and rerun while attempts remain, or require `B::Error` to expose the wrapped `adtb::Error`. Add the test.
