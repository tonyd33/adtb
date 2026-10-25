---
title: Whether assertions run in release builds
pick: one
raised-by: [substrate/new-engine]
options:
  always:
    title: Assertions on in production
    links:
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
  tiered:
    title: Cheap assertions always; expensive checks in debug and simulation builds
    links:
      - {rel: prior-art, to: prior/project-sqlite-testing}
  debug-only:
    title: debug_assert only
---
In simulation an assertion turns a silent corruption into a failing
seed. In production it turns it into a crash and log replay.

## project/arch/assertions/always

TIGER_STYLE: at least two assertions per function; "assertions
downgrade catastrophic correctness bugs into liveness bugs".

## project/arch/assertions/tiered

```
assert!(slot < page.slot_count());          // always
check_expensive!(btree.verify_order());      // debug and sim builds
```

SQLite's ALWAYS()/NEVER() pass through in release and assert in
testing.

## project/arch/assertions/debug-only

Rust's `debug_assert!`; release builds trust the code.
