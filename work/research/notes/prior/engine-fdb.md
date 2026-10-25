---
title: FoundationDB transactions
---
Strict serializable. MVCC reads, optimistic writes: a transaction
commits only if no key in its read conflict ranges was written between
its read version and commit. Transactions over five seconds fail; keys
up to 10 kB, values up to 100 kB.
https://apple.github.io/foundationdb/developer-guide.html
