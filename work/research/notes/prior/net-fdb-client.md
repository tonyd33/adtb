---
title: FoundationDB client retry loop
---
`db.run`/`@fdb.transactional` loops on retryable errors via
`on_error`. `commit_unknown_result` leaves the outcome unknown: it
committed or never will, so retries must be idempotent. 5 s and 10 MB
limits.
https://apple.github.io/foundationdb/developer-guide.html
