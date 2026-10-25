---
title: "How request ids work"
pick: one
options:
  recorded:
    title: "Optional, increasing per client; committed replies recorded with rows; low-water mark; RequestExpired"
  window-only:
    title: "A dedup window of any ids"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/idempotency/recorded

Optional, increasing per client; committed replies recorded with rows; low-water mark; RequestExpired. A purged record cannot be told from a new request without increasing ids.

## spec/idempotency/window-only

A dedup window of any ids. Double application after purge.
