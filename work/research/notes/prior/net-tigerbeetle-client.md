---
title: TigerBeetle client sessions and batching
---
One shared thread-safe client batches concurrent calls, up to 8189
events per request. A session registers, holds at most one in-flight
request, and is evicted at `clients_max` (64); the client retries
forever, idempotent through client-chosen ids.
https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/reference/sessions.md
