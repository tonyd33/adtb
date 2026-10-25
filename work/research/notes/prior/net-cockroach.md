---
title: CockroachDB retries and changefeeds
---
Retry errors are SQLSTATE 40001. The server retries automatically
while no result has reached the client (16 KiB buffer) for implicit
and single-batch transactions. Changefeeds emit each row change at
least once, ordered per key.
https://docs.cockroachlabs.com/docs/stable/transaction-retry-error-reference
https://docs.cockroachlabs.com/docs/stable/change-data-capture-overview
