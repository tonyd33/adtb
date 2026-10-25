# TigerBeetle

kind: system
home: https://tigerbeetle.com/

## Data model and types

- Fixed schema: two record kinds only, Account and Transfer, each exactly 128 bytes. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/reference/account.md
- Account fields: `id` u128 (client-chosen), `debits_pending`, `debits_posted`, `credits_pending`, `credits_posted` (u128 each), `user_data_128` u128, `user_data_64` u64, `user_data_32` u32, `reserved` 4 bytes, `ledger` u32, `code` u16, `flags` u16, `timestamp` u64 (ns since UNIX epoch). [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/reference/account.md
- `code` is documented as a user-defined enum for categorising accounts and transfers; `ledger` partitions accounts, and transfers may only move between accounts on the same ledger. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/reference/account.md, https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/reference/requests/create_transfers.md
- Generic `user_data_128/64/32` slots stand in for user columns; there are no user-defined fields or types. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/reference/account.md
- Accounts and transfers are immutable after creation (only account balance fields change via transfers); transfers cannot be deleted; corrections are new "correcting transfers". [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/reference/transfer.md
- Transfer variants are encoded as mutually exclusive flag bits in `flags` u16 rather than a tag: `pending`, `post_pending_transfer`, `void_pending_transfer`; plus `linked`, `balancing_debit`, `balancing_credit`, `closing_debit`, `closing_credit`, `imported`. Flag compatibility is validated per event. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/reference/transfer.md
- Two-phase transfers: pending reserves funds; a later transfer posts or voids it via `pending_id`; a pending transfer resolves at most once; `timeout` (seconds) expiry is best effort. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/reference/transfer.md
- Every object has a globally unique u64 creation timestamp assigned by the cluster, which serves as the synthetic primary key. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md

## Query interface

- No query language; the whole API is eight operations: `create_accounts`, `create_transfers`, `lookup_accounts`, `lookup_transfers`, `get_account_transfers`, `get_account_balances`, `query_accounts`, `query_transfers`. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/reference/requests/README.md
- `QueryFilter` fields: `user_data_128/64/32`, `ledger`, `code`, inclusive `timestamp_min`/`timestamp_max` (0 disables the bound), `limit` (non-zero u32), `flags.reversed` for timestamp order. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/reference/query-filter.md
- Requests are batches; each event gets its own result code, and only the highest-precedence error is reported per event. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/reference/requests/create_transfers.md
- `flags.linked` chains consecutive events into an all-or-nothing unit; other events fail with `linked_event_failed` if one in the chain fails. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/reference/requests/create_transfers.md
- Business logic (balance checks, limits) runs inside the database state machine, so a client never reads-then-writes over the network. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md

## Storage and indexes

- Storage is an LSM forest: many LSM trees each holding fixed-size values, not one tree with logical tables. Objects tree sorted by timestamp plus auxiliary index trees, e.g. (debit_account_id, timestamp) sorted by account id. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md
- A "groove" is one object type's object tree plus its index trees, declared at compile time with `ignored` (unindexed) fields and `derived` indexes computed from the object. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/src/state_machine.zig
- Account index trees: id, user_data_128, user_data_64, user_data_32, ledger, code, timestamp, plus derived `imported`, `closed`; balances, flags, reserved are ignored. Transfer index trees: id, debit_account_id, credit_account_id, amount, pending_id, user_data_*, ledger, code, timestamp, derived `expires_at` (timestamp + timeout for pending), `imported`, `closing`. Each tree has a fixed numeric tree id. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/src/state_machine.zig
- Derived indexes are sparse: they return a value only when a flag holds (e.g. `closed`), giving partial indexes over a variant-like flag. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/src/state_machine.zig
- All trees live in a grid of uniform 0.5 MiB blocks; block references are (u64 address, u128 checksum) pairs, hash-chained up to a superblock stored in 4 copies. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md
- Direct I/O bypasses the page cache; io_uring is the only I/O interface. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md

## Concurrency and replication

- Strict serializability by single-threaded deterministic sequential execution; no locks. Contended accounts would serialise anyway. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md
- Replication is Viewstamped Replication over 6 replicas with flexible quorums (3 to replicate, 4 for view change). The primary batches requests into prepares of about 8,000 transfers. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md
- The primary injects the timestamp into the state machine when building a prepare; cluster time comes from a replication quorum and is strictly monotonic. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md

## Durability and recovery

- WAL is two on-disk circular buffers: `wal_headers` (redundant headers) and `wal_prepares` (prepares padded to max message size); reserved slots carry their slot index as `op` to detect misdirected I/O; recovery is a decision table over header/prepare validity (remote repair, local repair, truncate, reserve). [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vsr/journal.zig
- Protocol-aware recovery: nacks from enough replicas prove a locally corrupt prepare was never committed, so it may be discarded. Corrupt grid blocks are repaired from peers by checksum. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md
- VOPR deterministic simulator runs a whole cluster on one thread, injects storage and network faults, and accelerates time. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md

## Evolution

- No user schema to evolve. Engine upgrades: replace the binary; replicas coordinate upgrade; data files from a past version migrate to any later version, possibly via staged intermediate releases. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/operating/upgrading.md
- Multiversion binaries embed past releases (`MultiversionHeader.PastReleases`, with offsets and checksums), and a replica re-execs the release the cluster runs. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/src/multiversion.zig
- `reserved` bytes in Account are kept for future fields. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/reference/account.md

## Design principles (TIGER_STYLE)

- All memory statically allocated at startup; nothing allocated or freed after initialisation. Every concurrent task is a statically allocated struct, bounding concurrency. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md
- Put a limit on everything: every loop and queue has a fixed upper bound; no recursion. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md
- At least two assertions per function on average; assert positive and negative space; pair assertions across write and read paths. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md
- 70-line function limit, 100-column lines, zero dependencies beyond the Zig toolchain. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md

## Data file

- Data file zones: superblock (4 copies; latest with at least 2 copies written wins), WAL ring of prepares, grid. The grid is an elastic array of 512 KiB blocks addressed by (u64 index, u128 checksum), the checksum held by the referrer so misdirected writes are caught; copy-on-write. Superblock references the manifest log (oldest, newest) and free set blocks. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/data_file.md

## Storage fault model

- Assumes the disk will fail: corruption (cites 0.031% of SSDs per year, Maneas FAST 2020; 1.4% enterprise HDDs, Bairavasundaram FAST 2008), misdirected I/O (0.023% SSD, 0.466% nearline HDD), gray failure. All data immutable, checksummed, hash-chained; latent sector errors repaired from peers without operator. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/concepts/safety.md
- Own page cache, O_DIRECT writes, can run on a raw block device with no file system. [V 2026-10-25] same page
- VOPR runs real code under network, storage and process faults at about 1000x speed, 24/7 on 1024 cores. [V 2026-10-25] same page
- If a record is corrupt on every replica the system halts. [V 2026-10-25] same page
- Static allocation cited as designing away fragmentation, OOM and use-after-free. [V 2026-10-25] same page
- io_uring since Linux 5.1 (2019); TigerBeetle argues it is safe for them because they handle only trusted fixed-size integer data. [V 2026-10-25] same page
- VSR quorums: replication quorum, view-change quorum, nack quorum (to truncate an uncommitted op); 2/2/2 for 3 replicas. Start-view-change then join-view protocol. Primary assigns op and timestamp converting request to prepare. Clients register a session; request-reply hashchain. Lagging replicas state-sync checkpointed grid and superblock. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vsr.md

## Operations

- Upgrade by replacing the binary on each replica; the cluster coordinates the upgrade once all replicas have the new version; about 5 s unavailability. A client may not be newer than the replicas. Data files from any past version migrate to any later version. No downgrade policy stated. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/operating/upgrading.md

## TIGER_STYLE (more)

- "Zero technical debt" policy. Assertions detect programmer errors, distinct from operating errors that must be handled; the only correct handling of an assertion failure is to crash. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md
- All errors must be handled; cites Yuan et al. OSDI 2014: 92% of catastrophic failures came from incorrect handling of non-fatal explicitly signalled errors. [V 2026-10-25] same
- Explicitly sized types (u32) over usize. [V 2026-10-25] same

## Client protocol

- Default max batch 8189 events, set by the server; a client shared across threads batches concurrent requests automatically; clients are thread-safe and one instance should be shared. [V 2026-10-25] https://docs.tigerbeetle.com/coding/clients/go/
- The client retries indefinitely with no per-request timeout; end-to-end idempotency via client-chosen ids. [V 2026-10-25] https://docs.tigerbeetle.com/coding/clients/go/
- Sessions: a client registers before its first request and gets an ephemeral random 128-bit client id; at most one in-flight request per session so the cluster can statically bound its message queue; `clients_max` default 64; at the limit the session that committed longest ago is evicted and its future requests never execute. Sessions read their own writes and never see posted balances decrease. [V 2026-10-25] https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/reference/sessions.md
