# Server protocol

This part specifies the protocol a client uses to reach an adtb server:
transports and TLS, the frame layout, the handshake that authenticates
the client and binds it to one bundle, the encoding of arguments and
results, calls, interactive transactions, flow control, cancellation,
feeds of committed change sets, server pushes, errors, compression and
version negotiation. Every request message is an operation of the
in-process API ([embedding.md](embedding.md)) serialized; this part
states only what the network adds. What a call means is stated by the
parts that own it: queries in [queries.md](queries.md), writes in
[writes.md](writes.md), transactions in
[transactions.md](transactions.md), bundles and leases in
[applications.md](applications.md), privileges in
[security.md](security.md).

Byte layouts below use these primitives:

| Name | Encoding |
| --- | --- |
| `u8`, `u16`, `u32`, `u64` | Unsigned integer, little-endian, fixed width |
| `i32`, `i64` | Two's complement, little-endian, fixed width |
| `uvarint` | Unsigned LEB128: 7 bits per byte, low group first, high bit set on every byte but the last; at most 10 bytes, and the shortest form only |
| `bytes` | `uvarint` length, then that many bytes |
| `text` | `bytes` holding UTF-8 in Unicode NFC |
| `opt T` | `u8` 0 (absent), or `u8` 1 followed by `T` |
| `list T` | `uvarint` count, then that many `T` |
| `position` | `u32` epoch, then `u64` log sequence number: a value of the prelude type `Position = { epoch: I64, lsn: I64 }` within those ranges, a commit position on one history as returned by commit ([transactions.md](transactions.md#8-commit-positions)) |
| `version` | `u64` schema version (the operator log position), then 32 bytes of its content hash |

A position whose epoch and log sequence number are not on this
database's history is refused with Error `PositionNotOnHistory`.

A receiver that meets a non-shortest `uvarint`, a `uvarint` longer than
10 bytes, text that is not valid UTF-8 or not in NFC, or an `opt` tag
other than 0 or 1 must treat the message as malformed (§11.3).

## 1. Transports and the server

### 1.1 Transports

A server listens on one or more listeners. Each listener is a TCP
address or a Unix domain socket path. The protocol is the same byte
stream over each.

```text
adt://db.internal:7433/app        -- TCP, database "app"
adt:///run/adt/app.sock           -- Unix socket, default database
```

The path component of a TCP address, or the `database` field of Hello
(§3.2), names one database file served by the process. A server may
serve many database files; each has its own catalog, operator log,
bundles and keys ([security.md](security.md#10-tenancy)).

The default TCP port is 7433.

### 1.2 TLS

Every TCP connection must be TLS from its first byte. There is no
plaintext TCP and no in-band upgrade. The client must offer the ALPN
protocol id `adt` and the server must refuse the TLS handshake when the
client offers no ALPN id or none it serves. TLS 1.3 is the minimum
version. Unix socket listeners carry the protocol without TLS.

### 1.3 Preface

After TLS completes (TCP) or on connect (Unix socket), the client must
send the 4 bytes `61 64 74 00` (`adt` and NUL) followed immediately by
its Hello frame. A server that reads any other 4 bytes must close the
connection without replying.

### 1.4 Server scheduling

An open connection with no running call must hold no execution
thread, only its socket and buffers. Connections are cheap
enough that clients connect directly; the protocol has no feature that
requires a pooling proxy, and a session holds no state a proxy would
have to track (§3.6).

Rulings: net/base/own, surface/wire/binary, net/transport/tcp, net/transport/unix, net/tls/direct, net/server/async, net/pool/none-needed

## 2. Frames

### 2.1 Frame header

Every message travels in one or more frames. A frame is a 12-byte
header followed by a body.

| Offset | Size | Field | Meaning |
| --- | --- | --- | --- |
| 0 | 4 | `length` | `u32` byte length of the body, excluding the header |
| 4 | 4 | `stream` | `u32` stream id |
| 8 | 1 | `kind` | `u8` message kind (§2.4) |
| 9 | 1 | `flags` | bit 0 `MORE`, bit 1 `COMPRESSED`; other bits must be 0 |
| 10 | 2 | `reserved` | must be 0 |

`length` must not exceed the receiver's `max_frame` (§3.2); a frame
over it, a nonzero reserved field or an unknown flag bit is a malformed
connection (§11.3).

### 2.2 Messages and chunks

A message is the concatenated bodies of a run of frames on one stream
with one kind: every frame but the last has `MORE` set. A sender must
not begin another message on a stream until the last frame of the
current one, but it may send frames of other streams between the
frames of one message. A sender may split any message at any byte
boundary.

The server must interleave frames fairly: while several streams have
frames ready and credit to send them (§5.5), it sends one frame of
each in turn. A stream with a large result therefore delays a short
reply on another stream by at most one frame. The server must not
emit a frame body larger than 64 KiB on a stream while another stream
has a frame ready, whatever `max_frame` allows.

### 2.3 Streams

A stream is one request and its replies. Stream 0 is the connection
stream: it carries the handshake, Ping and Pong, and server pushes.
Every other stream is opened by the client, by sending a request
message (Call, CallText, Begin, Commit, Rollback, Subscribe, Append) on a
stream id it has not used before on this connection. Stream ids must
increase: a request on an id not greater than every id the client has
used is malformed. A client that has used stream id 2^32 - 1 must open
a new connection for further requests. A stream closes when the server sends its final
message, End or Error, or for Begin, TxnOpened or Error. The server
never opens a stream.

While a stream is open the client may send Credit (§5.5), Cancel
(§5.7), on a feed stream FeedAck (§9.4), and on an append stream
AppendData and AppendEnd (§7.3) on it. A Credit or Cancel
for a stream that has closed is ignored. Streams are independent: each
call carries its own transaction or none (§6), and the failure of one
stream never closes another.

The server states in Ready the most streams one connection may hold
open at once (`max_streams`); a request beyond it is answered with
Error `Overloaded` on that stream (§8).

### 2.4 Message kinds

| Kind | Name | From | Stream | Final | Body |
| --- | --- | --- | --- | --- | --- |
| 0x01 | Hello | client | 0 | | §3.2 |
| 0x02 | AuthResponse | client | 0 | | §3.3 |
| 0x03 | Ping | client | 0 | | `u64` nonce |
| 0x04 | Cancel | client | target | | empty |
| 0x05 | Credit | either | target | | `u32` increment |
| 0x06 | Goodbye | client | 0 | | empty |
| 0x10 | Call | client | new | | §5.1 |
| 0x11 | CallText | client | new | | §7.1 |
| 0x12 | Begin | client | new | | §6.2 |
| 0x13 | Commit | client | new | | §6.3 |
| 0x14 | Rollback | client | new | | §6.3 |
| 0x15 | Subscribe | client | new | | §9.2 |
| 0x16 | FeedAck | client | feed | | `position` |
| 0x17 | Append | client | new | | §7.3 |
| 0x18 | AppendData | client | append | | One Arrow IPC encapsulated message |
| 0x19 | AppendEnd | client | append | | empty |
| 0x40 | AuthChallenge | server | 0 | | §3.3 |
| 0x41 | Ready | server | 0 | | §3.4 |
| 0x42 | Pong | server | 0 | | `u64` nonce of the Ping |
| 0x43 | Push | server | 0 | | §10 |
| 0x50 | ResultType | server | call | | §7.2 |
| 0x51 | Rows | server | call | | §5.3 |
| 0x52 | ArrowSchema | server | call | | §5.4 |
| 0x53 | ArrowBatch | server | call | | §5.4 |
| 0x54 | End | server | request | yes | §5.6 |
| 0x55 | Error | server | any | yes | §11 |
| 0x56 | TxnOpened | server | Begin | yes | `u64` transaction id |
| 0x57 | FeedSnapshot | server | feed | | §9.3 |
| 0x58 | FeedSnapshotEnd | server | feed | | `position` |
| 0x59 | FeedChanges | server | feed | | §9.3 |

A kind not in this table, or one sent in the wrong direction or on the
wrong stream, is malformed. Kinds 0x07 to 0x0F, 0x1A to 0x3F, 0x44 to
0x4F and 0x5A to 0x7F are reserved for later protocol versions; kinds
0x80 to 0xFF are never assigned.

### 2.5 Optional fields

Several bodies end in an option list: `list option`, where an
`option` is a `uvarint` key followed by `bytes` holding the option's
value encoded as the table for that message says. Keys must appear in
increasing order and at most once. A key the receiver does not know is
malformed: options are added only with a protocol version (§13), so a
peer never sends one the negotiated version lacks.

Rulings: net/concurrency/multiplexed, net/concurrency/hol/chunked

## 3. Connection lifecycle

### 3.1 Sequence

```text
-> 0 Hello { proto 1.0..1.0, auth SCRAM-SHA-256-PLUS, bundle web-2.4 at 12 (c41e...) }
<- 0 AuthChallenge { server-first }
-> 0 AuthResponse { client-final }
<- 0 Ready { proto 1.0, codec zstd, max_streams 256, ... }
-> 1 Call login_key ...
```

A connection is: preface, Hello, zero or more AuthChallenge and
AuthResponse rounds, then Ready or Error on stream 0. Before Ready the
client must send nothing but AuthResponse; after Ready it may open
streams. An Error on stream 0 is followed by the server closing the
connection.

Before Ready no edition or protocol version is shared, so frames are
read with fixed rules: every frame body is at most 16384 bytes, and an
Error has a fixed body, a `u16` code and a `text` message, that does
not depend on any version:

| Code | Reason |
| --- | --- |
| 1 | `UnsupportedProtocol`; the message states the server's range as `min..max` |
| 2 | `AuthFailed` |
| 3 | `Overloaded` |
| 4 | `Malformed` |
| 5 | `UnknownBundle` |
| 6 | `VersionMismatch` |
| 7 | `Retired` |
| 8 | `BundleExpired` |
| 9 | `Denied` |
| 10 | `BundleSuperseded` |
| 11 | `UnknownDatabase` |
| 12 | `CatalogMismatch` |
| 13 | `EditionLocked` |
| 14 | `UnknownEdition` |

A pre-Ready `Overloaded` carries no `retry_after`.

After Ready an Error body is a value of the prelude sum `Error` (§11).

### 3.2 Hello

| Field | Encoding | Meaning |
| --- | --- | --- |
| `proto_min` | `u16` major, `u16` minor | Oldest protocol version the client speaks |
| `proto_max` | `u16` major, `u16` minor | Newest protocol version the client speaks |
| `features` | `list text` | Optional features the client wants (§13.2) |
| `codecs` | `list text` | Compression codecs the client accepts, in preference order (§12) |
| `max_frame` | `u32` | Largest frame body the client accepts; at least 16384 |
| `client_id` | 16 bytes | Client instance id for idempotent writes (§6.5); all zero for none |
| `client_name` | `text` | Driver name and release, for monitoring |
| `database` | `text` | Database served by the process; empty for the listener's default. A name the process does not serve is refused with `UnknownDatabase` |
| `binding` | `u8` then fields | 0: no bundle (ad hoc session); 1: `text` bundle name, then `version`, then `opt version` catalog version, present exactly when the bundle pins one |
| `mechanism` | `text` | `SCRAM-SHA-256-PLUS`, `SCRAM-SHA-256`, `certificate` or `peer` |
| `role` | `text` | Role to authenticate as |
| `initial` | `bytes` | SCRAM client-first message; empty for the other mechanisms |
| options | `list option` | Key 1 `edition` (`text`): for an ad hoc session, the edition whose `Error`, `Notice` and prelude sums the session receives; refused with `EditionLocked` on a bound connection, whose edition is the bundle's, and with `UnknownEdition` when the server implements no such edition |

### 3.3 Authentication

A listener accepts a configured set of mechanisms
([security.md](security.md#2-authentication)). A Hello naming one the
listener does not accept is answered with Error `AuthFailed`.

| Mechanism | Transport | Exchange |
| --- | --- | --- |
| `SCRAM-SHA-256-PLUS` | TCP | SCRAM-SHA-256 (RFC 7677) with channel binding `tls-server-end-point` (RFC 5929). Hello carries client-first; AuthChallenge carries server-first; AuthResponse carries client-final; Ready carries server-final. The SCRAM username attribute must be empty: the role is the Hello field. |
| `SCRAM-SHA-256` | Unix socket | As above without channel binding. A TCP listener must refuse it. |
| `certificate` | TCP | The client certificate of the TLS handshake, mapped to a role by the listener's mapping; no AuthChallenge. |
| `peer` | Unix socket | The OS credentials of the socket peer, mapped to a role by the listener's mapping; no AuthChallenge. |

AuthChallenge and AuthResponse each carry one `bytes` field. When the
mapped or verified role differs from the Hello `role`, or the role does
not exist, or the proof is wrong, the server answers Error `AuthFailed`
with no detail that tells these cases apart, and records the attempt in
the audit relation ([security.md](security.md#8-audit)).

### 3.4 Binding and Ready

When `binding` is 1, the server checks after authentication that the
bundle is registered in the database at exactly the given version
(both position and hash), that the catalog version given matches the
one it pins (both position and hash), that it is not retired or
superseded, that its lease has not expired (below), and
that the role holds the call grant on the bundle, on at least one of
its queries, or on a function it imports. Failures are answered with Error `UnknownBundle`,
`VersionMismatch`, `CatalogMismatch`, `Retired`, `BundleSuperseded`,
`BundleExpired` or `Denied`. Every call on the connection is checked
and encoded against the bundle at its pinned version.

Lease expiry is computed from the last renewal record in the log, on
the primary and on a replica alike. From expiry a replica refuses
binds and calls on the bundle with `BundleExpired`. The primary treats
the lease as expired only after the further `LeaseGrace` setting, one
hour by default. Binding to an expired bundle, calling it and renewing
its lease, directly or forwarded, are then refused with
`BundleExpired` on the primary too; the bundle is live again only once
registered again ([applications.md §4.2](applications.md#42-leases)).

Binding renews the bundle's lease, and so does every call on the
connection. A server writes a renewal for a bundle at most once per
eighth of the lease ([applications.md](applications.md#42-leases)).
Once a bundle is retired, calls on connections already bound to it
are refused with `Retired`; once it is superseded, with
`BundleSuperseded`.

When `binding` is 0, the connection is an ad hoc session: only
CallText and Append (§7) and the transaction messages on transactions
opened by it are accepted. Call and Subscribe are answered with Error
`UnknownQuery`. A bound connection also accepts CallText and Append
from a role that holds the grants §7 requires.

Ready:

| Field | Encoding | Meaning |
| --- | --- | --- |
| `proto` | `u16`, `u16` | The chosen protocol version |
| `features` | `list text` | Features granted, a subset of those asked |
| `codec` | `text` | Chosen codec, empty for none |
| `max_frame` | `u32` | Largest frame body the server accepts; at least 16384 |
| `max_streams` | `u32` | Open streams allowed at once (§2.3) |
| `connection` | `u64` | Connection id, as shown in `sys::sessions` |
| `server` | `text` | Server release |
| `edition` | `text` | The edition of the sums this connection receives: the bound bundle's, the one an ad hoc Hello asked for, or the newest the server implements |
| `final` | `bytes` | SCRAM server-final message; empty for other mechanisms |
| `lease_until` | `opt i64` | When bound, the bundle's lease expiry as an Instant (§4.2) |
| `stream_idle` | `i64` | Stream idle limit as a Duration (§5.8) |
| `txn_limit` | `opt i64` | The `TransactionLimit` file setting as a Duration, absent for no limit (§6.2) |
| `idle_limit` | `opt i64` | The `IdleLimit` file setting as a Duration, absent for no limit (§6.2) |
| options | `list option` | None are defined in protocol 1.0 |

### 3.5 Ping and Goodbye

A client may send Ping at any time after Ready; the server answers Pong
with the same nonce after every frame it queued before the Ping. A
client ends a connection with Goodbye: the server stops reading,
finishes no stream, and closes. Closing a connection cancels every
call still running on it (§5.7) but does not end interactive
transactions (§6.2).

### 3.6 Sessions hold no state

A session is a connection's authenticated role, its binding and its
negotiated parameters. It holds no settings, no variables, no prepared
statements, no temp relations and no transaction. Every call states all it needs in its
own message; an interactive transaction is named by an id (§6.2) and
may be continued on any connection with the same role and binding, one
connection at a time.

Rulings: net/version/range, ops/protocol/negotiate, net/authn/scram, net/authn/mtls, net/authn/peer, net/bind/connection, net/session/stateless, surface/both

## 4. Value encoding

### 4.1 Types come from the signature

A value on the wire carries no type information. Its type is fixed by
the registered signature of the call at the bound version: arguments
by the declared parameter types, results by the declared result type.
Client code generated from the bundle and the server read the same
signature, so both know every layout. Ad hoc results, which have no
signature on the client, are preceded by a ResultType message (§7.2).

### 4.2 Base types

| Type | Encoding |
| --- | --- |
| `I8`, `I16`, `I32`, `I64` | Fixed-width two's complement, little-endian |
| `F32`, `F64` | IEEE 754 binary32 or binary64, little-endian; every NaN as the canonical quiet NaN, never -0.0 |
| `Decimal s` | The unscaled value as a 128-bit two's complement integer, little-endian; the scale `s` is in the type |
| `Text` | `text` |
| `Bytes` | `bytes` |
| `Instant` | `i64` microseconds since 1970-01-01T00:00:00Z on the smeared time scale |
| `Date` | `i32` days since 1970-01-01 |
| `Time` | `i64` microseconds since midnight |
| `DateTime` | `i64` microseconds since 1970-01-01T00:00:00 civil |
| `ZonedDateTime` | `DateTime` encoding, then `text` IANA zone name, then `i32` UTC offset in seconds |
| `Duration` | `i64` microseconds |
| `Period` | `i32` months, then `i32` days |
| `SortKey` | `bytes` |
| `Key r.S` | The key of `r`: `i64` for a key generated by `sequence`; 16 bytes in RFC 9562 byte order for one generated by `uuidv7`; otherwise the key's fields as a record (§4.3) |

A newtype is encoded as the type it wraps. A reference is encoded as
the key it holds, whatever constructor set its type names. A server receiving an argument
that is not a value of its type (text not in NFC, a NaN other than
the canonical one, -0.0) must refuse the call with Error `Malformed`.
The base types are those of [model.md](model.md#3-base-types).

### 4.3 Records and sums

A record (a one-constructor sum) is its fields in the order the type
declares them, with no tag and no field names. An anonymous record in
a signature has its fields in the order written.

A value carries a tag exactly when its static constructor set at the
bound version has two or more members. Such a value is a `uvarint`
tag followed by the constructor's payload record: the header fields
of the sum in declared order, then the constructor's own fields in
declared order. A nullary constructor has an empty payload, so an
enum value is its tag alone. A value whose static set has one member,
such as a row of `identities.Google` or a record, carries no tag.

The tag of a constructor of a declared type is its constructor id.
Ids are a function of the operator log, assigned as
[catalog.md §1.2](catalog.md#12-identity-of-catalog-objects) states,
so every database and every tool that replays one log, code generation
included, computes the same ids. An id belongs to the
declared constructor and is shared by every instantiation of a
parametric sum. Prelude types (`Bool`, `Option`, `Result`, `Error` and its
reason sums, `Notice`, `Outcome` and the others) and the types of the
`catalog` and `sys` modules have fixed ids below 1024: prelude ids per
edition, catalog ids per catalog version, listed in
[catalog.md](catalog.md#4-error-sums); ids from the log start at
1024. The tag of a constructor of a result sum anonymous to the query
is its position in the query signature, counting from 1. In an ad hoc
statement, which has no signature, the constructors of an anonymous
result sum are numbered by their first appearance in the statement
text, counting from 1. Numbering does not change type equality
([queries.md §4.3](queries.md#43-result-sums)).

With constructor ids Identity 1040, EmailPass 1041 and Google 1042,
the `identities` key generated by `uuidv7` and the `users` key by
`sequence`, the row

```adt
-- given k: Key identities
Google { id: k, user: users.key(7), gid: "g-123" }
```

is encoded as

```text
92 08  <16 bytes of k>  07 00 00 00 00 00 00 00  05 67 2d 31 32 33
tag 1042, id, user, text length 5, "g-123"
```

and a result of

```adt fragment
query login_keys (u: Key users) -> Set (ByEmail { email: Text } | ByGoogle { gid: Text }) = ...
```

encodes its element `ByGoogle { gid: "g-123" }` as

```text
02 05 67 2d 31 32 33
```

The server encodes every result at the bound version. A client never
receives a tag its pinned version lacks, because writers never produce
a constructor a live reader cannot type: constructors of relations and
of value-level sums alike start declared and are enabled readers-first
([evolution.md](evolution.md#8-readers-first)). A client that receives
one must close the connection as malformed.

### 4.4 Nested collections

A field of type `Set T` is a `uvarint` element count followed by the
elements. Nested collections travel inline in their parent row: a row
is complete when its last nested element is.

```adt fragment
query user_logins (n: Text) -> Set ({ name: Text, logins: Set Text }) = ...
```

```text
{ name: "ann", logins: ["a@example.com", "g-123"] }
=> 03 61 6e 6e  02  0d 61 40 ... 05 67 2d 31 32 33
```

Rulings: net/types/signature, surface/sum-encoding/stable-tag, surface/query-scoped, net/nested/inline

## 5. Calls

### 5.1 Call

A Call names a registered query or transaction of the bound bundle, or
a schema function the bundle imports, by its ordinal. The `query`,
`txn`, `import fn` and `feed` declarations of a bundle file take
ordinals 1, 2, 3 and so on in file order; bundle-local `fn`
declarations and module imports take none. A bundle that
calls a schema function directly declares `import fn NAME`, which takes
an ordinal like a query ([security.md](security.md#47-schema-functions-called-directly)).
Code generation computes ordinals from the file, and the engine
assigns the same ones at registration.

```adt fragment
-- bundles/mobile.adb
bundle mobile-1.0 pins 12 (c41e...) edition "2026-10"
import fn display_name                                 -- ordinal 1
query login_key (u: Key users) -> Set Identity = ...   -- ordinal 2
txn sign_up (n: Text, g: Text) -> Key identities = ... -- ordinal 3
```

| Field | Encoding | Meaning |
| --- | --- | --- |
| `ordinal` | `uvarint` | The query or transaction |
| `format` | `u8` | 0: rows (§5.3); 1: Arrow (§5.4) |
| `credit` | `u64` | Initial result credit in bytes (§5.5) |
| `args` | `bytes` | The arguments, each encoded per its declared type (§4), concatenated in declared order |
| options | `list option` | See below |

| Key | Option | Value encoding | Meaning |
| --- | --- | --- | --- |
| 1 | `txn` | `u64` | Run inside this interactive transaction (§6.2) |
| 2 | `request` | `u64` | Request number for deduplication (§6.5) |
| 3 | `deadline` | `i64` Instant | Stop at this time ([transactions.md](transactions.md#54-time-limit)) |
| 4 | `trace` | `text` | W3C `traceparent` of the caller's span |
| 5 | `assert` | Principal value (§4.3) | End-user principal for this call ([security.md](security.md#13-where-the-principal-comes-from)) |
| 6 | `durability` | `u8` 0 local, 1 quorum | Commit level ([transactions.md](transactions.md#7-durability-levels)) |
| 7 | `after` | `position` | Do not read a snapshot older than this commit ([transactions.md](transactions.md#8-commit-positions)) |
| 8 | `limit` | `i64` Duration | Statement limit for this call, at most the declared one ([catalog.md](catalog.md#83-limits-and-deadlines)) |
| 9 | `priority` | `u8` | Priority within the bundle's priority class; higher runs first (§8) |
| 10 | `retry` | `uvarint` | For a registered transaction, the most reruns on serialization failure: `n` allows at most `n + 1` attempts; by default the transaction's declared limit, else the file setting (§6.4) |
| 11 | `compress` | empty | Compress this stream's result frames (§12) |

An unknown ordinal is answered with Error `UnknownQuery`. A call the
role holds no call grant for is answered with Error `Denied`. A call
grant on the whole bundle covers its registered queries and
transactions; a call of an imported function needs `grant call fn
NAME` on that function. `args` whose length differs from what the
signature implies are answered with Error `Malformed` on the stream.
`assert`, `durability`, `after`, `request` and `retry` are refused
with Error `Malformed` when `txn` is present: `assert`, `durability`
and `after` belong to Begin, a request number to Commit, and a call
inside an interactive transaction is never retried.

An `assert` value is a value of the schema's `Principal` type, the
one in the root namespace, encoded with its tags (§4.3); when the
schema declares none it is the prelude's default `Principal`.

Engine spans for the call are named by the query and joined to the
`trace` context when one is given ([catalog.md](catalog.md#132-traces)).

### 5.2 Replies

A successful call is answered by zero or more result messages and then
End. In rows format the result messages are Rows. In Arrow format they
are one ArrowSchema followed by zero or more ArrowBatch. A failed call
is answered by Error, possibly after some result messages of a read
query; a client must discard partial results of a stream that ends in
Error.

A registered query that only reads streams its result as it is
produced. A registered transaction ([transactions.md](transactions.md#4-registered-transactions))
sends no result message until it has committed: its result rows follow
the commit and precede End.

### 5.3 Rows format

A Rows message body is a `uvarint` row count followed by that many
rows, each encoded as the result element type (§4). A result that is
not a set is sent as one Rows message holding one element. A result is a set:
the server sends each element once and in no particular order unless
the query orders its result ([queries.md](queries.md#14-ordering-top-n-and-pagination)), in
which case rows arrive in that order across messages.

```adt
query login_key (u: Key users) -> Set Identity = from i in identities where i.user == u select i
```

```text
<- 1 Rows  02  92 08 <id> <user> 05 "g-123"  91 08 <id> <user> 0d "a@example.com" 20 <32 bytes of hash>
```

### 5.4 Arrow format

The ArrowSchema body is one Arrow IPC encapsulated message holding a
Schema; each ArrowBatch body is one encapsulated message holding a
RecordBatch, followed by its body buffers, as in the Arrow IPC
streaming format. Dictionary batches are not used. Arrow IPC body
compression must not be used; compression is per frame (§12).

The result type maps to Arrow as follows.

| Result type | Arrow type |
| --- | --- |
| A record result | One column per field, in declared order |
| A sum result of two or more constructors | One column `row` of a dense union |
| Any other result element (a base type, a newtype, a key, an `Option`, a `Set`) | One column `value` of the element's Arrow type |
| A field of a base type | The Arrow type of the same width: `Int8` to `Int64`, `Float32`, `Float64`, `Decimal128(38, s)`, `Utf8`, `Binary`, `Timestamp(us, UTC)` for `Instant`, `Date32`, `Time64(us)`, `Timestamp(us)` for `DateTime`, `Duration(us)` |
| A field of type `SortKey` | `Binary` |
| A field of a newtype | The Arrow type of the wrapped type |
| A field of type `Key r.S` | `Int64` for a `sequence` key, `FixedSizeBinary(16)` for a `uuidv7` key, otherwise the Arrow type of the key's fields as a record |
| A field of a record type | `Struct` |
| A field of type `Bool` | `Boolean` |
| A field of any other sum all of whose constructors are nullary | `UInt32` holding the tag (§4.3), with metadata `adt.enum` listing `tag=name` pairs |
| A field of a sum of two or more constructors, some with fields | Dense union, one child per constructor: a `Struct` for a constructor with fields, `Null` for a nullary one |
| A field of type `Option T`, `T` neither a sum nor an `Option` | The Arrow type of `T`, nullable; `None` is null |
| A field of type `Option T`, `T` a sum or an `Option` | Dense union of a `Null` child for `None` and the children of `T`'s mapping for `Some` |
| A field of type `Set T` | `List` of the Arrow type of `T` |

`ZonedDateTime` and `Period` map to a `Struct` of their components in
the order of §4.2.

Arrow union type ids are 8-bit and local to the stream. Each union
child field carries the metadata key `adt.ctor` whose value is the
decimal tag of its constructor (§4.3); the `None` child of an `Option`
carries the tag of `None`. The client reads constructor identity from
this map, never from the type id. A union lists the constructors of the
call's result at the bound version
([queries.md §12.4](queries.md#124-the-constructors-a-statement-reads)).
A union of more than 127
constructors nests as a union of unions: the outer union's children
are unions of at most 127 constructors each, in increasing tag order,
nesting again wherever a level would exceed 127 children, as deep as
needed. Only the innermost children carry `adt.ctor`.

### 5.5 Credit

The server sends result data on a stream only within credit the client
has granted for it. Result data is the bodies of ResultType, Rows,
ArrowSchema, ArrowBatch, FeedSnapshot, FeedSnapshotEnd and FeedChanges
frames, counted as the bytes on the wire after compression. A request
grants initial credit; Credit adds its increment to the stream's
credit, saturating at 2^64 - 1. The server must not send a data frame
whose body exceeds the remaining credit; it may split a message (§2.2)
to use exactly what remains. End, Error and TxnOpened consume no
credit. A request with an initial credit below 16384 is answered with
Error `Malformed`.

A stream with no credit holds its call: the call waits, and its
snapshot stays held.

On an Append stream the server grants credit to the client for
AppendData bodies, by the same rules in the other direction (§7.3).

### 5.6 End

| Field | Encoding | Meaning |
| --- | --- | --- |
| `rows` | `u64` | Number of result elements sent |
| `attempts` | `uvarint` | Times the server ran the call (§6.4); 1 when not retried |
| `commit` | `opt position` | The commit position, for a call that committed |
| `snapshot` | `opt position` | The position of the snapshot a call that only read was evaluated at |
| options | `list option` | Key 1 `incomplete` (empty): the call committed but its result rows were not all sent (§5.8) |

Positions are shown in examples as `epoch/lsn`, as `3/7741`.

### 5.7 Cancel

Cancel on an open stream stops its call promptly: the call does not
run to the end of its statement first. The server answers Error `Cancelled` on the stream unless
the call finished first, in which case its own End or Error stands.
Cancel on a stream whose final message the server already sent is
ignored, and so is Cancel on a Begin, Commit or Rollback stream: each
runs to its own reply.

A cancelled call has no effect: a registered transaction cancelled
before its commit applies nothing; one whose commit has completed when
the cancel arrives is answered with End. A cancelled call inside an
interactive transaction ends that call only: the transaction stays open
without the call's effects, and the client may continue it or roll it
back ([transactions.md](transactions.md#5-interactive-transactions)).

### 5.8 Stream idle limit

A stream whose call has result data ready but no credit for longer
than the stream idle limit is cancelled by the server and answered
with Error `StreamIdle`. The limit is a file setting
([catalog.md](catalog.md#10-configuration)), 30 seconds by default; its
value is sent in Ready.

A stream whose call has committed is never cancelled. If it stays
without credit past the limit, the server drops the result rows not
yet sent and ends the stream with End carrying the commit position and
the option `incomplete`; the commit stands. A client that resends the
call with its request number receives the recorded rows (§6.5).

Rulings: net/call/ordinal, net/results/rows, net/results/arrow, net/results/tag-width/type-map, net/flow/credit, net/flow/held/idle-limit, net/cancel/in-band, ops/cancel/cancel, spec/reads/static/static-types

## 6. Transactions

### 6.1 Single-call transactions

A Call without `txn` is its own transaction. A call of a registered
query that only reads runs on the primary as a read-only transaction
under serializable snapshot isolation; it never waits for a safe
snapshot, and may fail with `Serialization`, which the client answers
by calling again. Only reads on a replica, and transactions begun with
`deferrable`, wait for a safe snapshot
([transactions.md](transactions.md#64-read-only-transactions)). A call
of a registered transaction runs its block as one transaction and
commits before it replies.

### 6.2 Interactive transactions

Begin opens an interactive transaction and is answered with TxnOpened
carrying its id, a value of the prelude type `TxnId`: a random
positive integer below 2^63, sent as `u64`; the client names the
transaction with it in the `txn` option of later calls and in Commit
or Rollback, on any connection authenticated as the same role and
bound to the same bundle (or, for an ad hoc session, also unbound). A
call naming a transaction that does not exist, has committed or been
rolled back or aborted other than by its limits (below), or belongs
to another role or binding is answered with Error
`UnknownTransaction`, with no detail telling these apart. On a Commit
that carries a request number, it means definitely that the
transaction did not commit
([transactions.md §9.2](transactions.md#92-deduplication)); without
one, it tells nothing about whether the transaction committed.

Begin body is an option list:

| Key | Option | Value encoding | Meaning |
| --- | --- | --- | --- |
| 1 | `read_only` | empty | The transaction only reads |
| 2 | `deadline` | `i64` Instant | Abort the transaction at this time |
| 3 | `assert` | Principal value | End-user principal for the whole transaction |
| 4 | `durability` | `u8` 0 local, 1 quorum | Commit level |
| 5 | `after` | `position` | Do not read a snapshot older than this commit |
| 6 | `deferrable` | empty | With `read_only`: wait for a safe snapshot, then never fail with `Serialization` |
| 7 | `limit` | `i64` Duration | Statement limit for each call in the transaction |
| 8 | `priority` | `u8` | Priority within the bundle's class, as in Call |

A transaction is used from one connection at a time. While a call,
Commit or Rollback naming it is running or queued from one connection,
a request naming it from another connection is answered with Error
`TransactionBusy` and not executed. Requests naming one transaction
from one connection run one at a time, in the order the server
receives them. Once none is running, the next request may come from
any connection with the same role and binding.

A transaction open longer than the interactive transaction time limit,
or idle between requests longer than the idle limit, is aborted by the
engine; later calls naming it, and its Commit and Rollback, are
answered with Error `TransactionExpired`. The limits are the `TransactionLimit` and
`IdleLimit` file settings ([catalog.md](catalog.md#102-file-settings));
`TransactionLimit` is 5 seconds by default, and Ready sends both
(§3.4). Losing a connection does not end a
transaction; the time limit does.

### 6.3 Commit and Rollback

Commit names the transaction and may carry a `request` number and a
durability level:

| Field | Encoding |
| --- | --- |
| `txn` | `u64` |
| options | `list option`; key 1 `request` `u64`; key 2 `durability` `u8`, 0 local, 1 quorum |

The transaction commits at the strongest level named by its Begin, its
Commit or any block run in it
([transactions.md §7.1](transactions.md#71-levels)).

It is answered with End carrying the commit position, or with Error.
The server does not retry an interactive transaction: a serialization
failure at any call or at Commit is answered with Error
`Serialization`, the transaction is aborted, and the client reruns its
work ([transactions.md](transactions.md#55-retry)). Rollback (body: `u64`
transaction id) aborts the transaction and is answered with End.

```text
-> 5 Begin { assert: EndUser { user: k } }  <- 5 TxnOpened 0x1a1f...
-> 6 Call find_user { txn: 0x1a1f... }      <- 6 Rows ... <- 6 End
-> 7 Call add_identity { txn: 0x1a1f... }   <- 7 End
-> 8 Commit 0x1a1f... { request: 812 }      <- 8 End { commit: 3/7741 }
```

### 6.4 Server retry

A registered transaction called without `txn` has no effect outside
the database, and no result of it reaches the client before commit
(§5.2). The same holds for a CallText without `txn` that writes
(§7.1). When either fails with a serialization error the server reruns it
at a new snapshot before replying, up to the retry limit the
transaction declares or, by default, the file setting `RetryLimit`
([catalog.md §10.2](catalog.md#102-file-settings),
[transactions.md](transactions.md#62-retryable-units)). End reports the number of
attempts. Only when every attempt fails does the client receive Error
`Serialization`.

### 6.5 Idempotent writes

Request numbers are optional; a client needs them only to resend a
write safely after losing its reply. A client that sends a nonzero
`client_id` in Hello may attach a `request` number to a Call of a
registered transaction, to a CallText that writes and to an Append,
each outside an interactive transaction, and to a Commit. Each new request's number must exceed every
earlier number of that client id; a resend reuses the number of the
request it resends.

For each client id the engine keeps, for the deduplication window, a
file setting ([catalog.md](catalog.md#10-configuration)) of 24 hours by
default:

- for each request that committed, its End and its result rows;
- a low-water mark below which it keeps no records.

A request carrying a number is handled in this order:

1. If the number has a record, the request is not executed again: if
   the record was made for the same call and arguments, the server
   answers with the recorded rows and End; otherwise with Error
   `RequestReused`. Each record holds the hash of its request's call
   and arguments for this check.
2. Otherwise, if a request with the same number is still running, a
   resend for a different call or arguments is answered with Error
   `RequestReused`; a resend for the same call and arguments waits for
   it and returns its outcome: its rows and End if it commits, or its
   error if it fails, without running again.
3. Otherwise, if the number is strictly below the low-water mark, the
   request is answered with Error `RequestExpired` and not executed.
4. Otherwise the request runs.

Errors are
not recorded: once a request has failed, a later resend with its
number runs again. The records are kept in a catalog relation and
survive restart and failover
([transactions.md](transactions.md#9-idempotent-write-calls)).

A `request` on a Call of a query that only reads, on a request with
`txn` other than Commit, or with an all-zero `client_id`, is answered
with Error `Malformed`. A `request` on a CallText that only reads is
ignored.

### 6.6 Replicas

A server that runs as a replica of a primary
([replication.md](replication.md)) answers calls that only read at
a safe point it has applied. A call with `after` waits until a safe
point covers that position, for at most the file setting `ReplicaWait`
(5 seconds by default, [catalog.md](catalog.md#10-configuration)), and
then fails with Error `SafePointTimeout`: safe points may not form
while long transactions run on the primary. A call
that would write, and Begin without `read_only`, are refused with
Error `ReadOnlyReplica`, which names the primary. Only an embedded
replica forwards writes to its primary
([embedding.md](embedding.md#122-embedded-replicas)). The protocol
between a primary and its replicas is out of scope of this edition.

Rulings: net/interactive/time-limit, net/retry/server, net/unknown/idempotency, net/session/stateless, spec/txn/resend-unknown/not-committed, spec/txn/durability/strongest

## 7. Ad hoc text and appends

### 7.1 CallText

CallText carries text instead of an ordinal. It is accepted only from
a role holding the ad hoc grant
([security.md](security.md#3-grants)), on an ad hoc session or on a
bound connection; from any other role it is answered with Error
`Denied`, whatever the text. The text is checked against the schema
and catalog versions of its transaction's snapshot, and its reads and writes are checked against the
role's grants on constructor sets. Ad hoc text may hold queries,
writes, `transact` blocks, `temp relation` declarations and
operational commands, among them `register bundle`, `apply
migrations`, `print schema`, `archive audit`, `rotate key`, `dump`,
`restore` and `backup` ([catalog.md](catalog.md#9-operational-commands),
[applications.md](applications.md#7-the-ad-hoc-role)). Several items
in one text form one transaction, each item a statement; their
results come back in order. The first item that fails, by a fault, a
`fail`, a violation or a rejection, ends the text: later items do not
run, nothing of the text is applied, and the Error names the failing
item by its ordinal in the text, counting from 1.

A CallText without `txn` that writes sends no result message until it
has committed, and is rerun by the server on a serialization failure
as a registered transaction is (§6.4); End reports its attempts. A
CallText that only reads streams its results as they are produced.

A call with `txn` that would run at another schema or catalog version
than the transaction's earlier calls is refused with Error
`MixedVersions` and not executed; the transaction stays open. In
particular, a CallText naming a transaction that has made a call of a
registered query or transaction, and a Call naming a transaction that
has run a CallText, are refused.

| Field | Encoding |
| --- | --- |
| `text` | `text` |
| `format` | `u8`, as in Call |
| `credit` | `u64` |
| `params` | `list` of (`text` name, type descriptor (§7.2), value encoded as §4) |
| options | `list option`, with keys 1 `txn`, 2 `request`, 3 `deadline`, 4 `trace`, 5 `assert`, 6 `durability`, 7 `after`, 8 `limit`, 9 `priority` and 11 `compress` as in Call, and refused with `txn` as in Call |

A free name in the text is a parameter: `params` must supply it, with
its type descriptor, and a free name with no supplied parameter is
answered with Error `Invalid`. Values such as a password in `alter
role web password p` therefore never appear in the text. A text that
fails to parse or check is answered with Error `Invalid` carrying the
diagnostics.

Results of ad hoc text are encoded at the current version, on a bound
connection too; their tags are those the ResultType descriptor gives.

### 7.2 ResultType

In rows format, each item of a CallText that returns a result is
answered by one ResultType message describing its element type, then
its Rows; one End follows the last item. In Arrow format each such
item is answered by its own ArrowSchema and ArrowBatch messages, the
Arrow schema describing the result, and no ResultType is sent. A type descriptor is:

| First byte | Type | Followed by |
| --- | --- | --- |
| 0x01 to 0x04 | `I8`, `I16`, `I32`, `I64` | nothing |
| 0x05, 0x06 | `F32`, `F64` | nothing |
| 0x07 | `Decimal s` | `uvarint` scale |
| 0x08, 0x09 | `Text`, `Bytes` | nothing |
| 0x0A to 0x10 | `Instant`, `Date`, `Time`, `DateTime`, `ZonedDateTime`, `Duration`, `Period` | nothing |
| 0x11 | `SortKey` | nothing |
| 0x20 | record | `text` type name (empty if anonymous), `list` of (`text` field name, descriptor) |
| 0x21 | sum | `text` type name (empty if anonymous), `list` of (`uvarint` tag, `text` constructor name, `list` of (`text` field name, descriptor)) |
| 0x22 | `Set T` | descriptor of `T` |
| 0x23 | `Key r.S` | `text` relation name, `list uvarint` constructor ids, descriptor of the key value |
| 0x24 | newtype | `text` type name, descriptor of the wrapped type |

Values that follow are encoded exactly as §4 would encode a value of
the described type.

### 7.3 Append

An Append stream writes Arrow record batches into one constructor
relation, into a staging relation of a load
([interop.md](interop.md#4-loading)), or into a temp relation of the
interactive transaction it names. The role needs `insert` on the
target; a temp relation needs none.

A temp relation, declared by ad hoc text `temp relation NAME : T` with
`T` a named type or an anonymous record type, lives outside the log,
has no constraint but its key, and belongs to the interactive
transaction whose CallText declared it, or, without `txn`, to that one
CallText. It is gone when its transaction or its CallText ends. One
declared with no key is keyed by its whole row. A remote load that
needs routing or rejects begins an interactive transaction, appends
its rows, with text columns where parsing is needed, into a temp
relation of it, then inserts from it with an ad hoc insert that routes
the rows and sends failures `rejects into` a rejects relation
([interop.md](interop.md#45-rejects)), and commits:

```text
-> 1 Begin                                                        <- 1 TxnOpened t
-> 2 CallText { txn: t } temp relation raw : { kind: Text, user: I64, email: Text, gid: Text }
-> 3 Append raw { txn: t } ... AppendEnd                         <- 3 End
-> 4 CallText { txn: t } insert identities from (from x in raw select ...) rejects into identity_rejects
-> 5 Commit t                                                     <- 5 End { commit: 3/7802 }
```

| Field | Encoding | Meaning |
| --- | --- | --- |
| `relation` | `text` | Qualified name of the target relation variable |
| `ctor` | `uvarint` | Constructor tag of the target constructor relation (§4.3); 0 for a temp relation |
| `credit` | `u64` | Initial credit the client grants for the server's replies |
| options | `list option` | Keys 1 `txn`, 2 `request`, 6 `durability` as in Call, and refused with `txn` as in Call |

The client then sends AppendData messages: the first holds an Arrow
IPC Schema message, each later one a RecordBatch. The schema must be
the constructor relation's row type as §5.4 maps it, nullable only for
`Option` fields; a field filled by a key generator or a relation
default may be left out. Any other schema is answered with Error
`SchemaMismatch`. Text in a batch is normalized to NFC and floats are
canonicalized, as an in-process appender does
([embedding.md](embedding.md#9-bulk-append)); the refusal of §4.2
applies to arguments, not to appended batches. The server grants the client credit for AppendData
bodies with Credit messages on the stream, starting from the
`max_frame` it sent in Ready; the client must not send AppendData
beyond it.

AppendEnd ends the input. Without `txn`, the rows are inserted as one
change set, checked set-at-a-time like any change set, and the stream
ends with End carrying the row count and the commit position. With
`txn`, the rows are one statement of that interactive transaction and
End carries no commit position. An Append fails with `SchemaMismatch`
(the schema is not the target's), `Denied` (no `insert`),
`KeyViolation` (two appended rows with one key), `Violated` (a
unique or another constraint), `Serialization`, `Cancelled`, `DeadlineExceeded` or
`Overloaded`; on any of them nothing is inserted.

Rulings: net/adhoc/principal, surface/adhoc/scoped, interop/load/append-api, spec/txn/one-version/one-pair

## 8. Admission and overload

Each listener bounds its open connections. A connection beyond the
bound is answered, after Hello, with Error `Overloaded` on stream 0 and
closed. Each connection bounds its open streams (`max_streams`, §2.3).

A call that cannot start at once waits in the engine's admission queue,
ordered by priority class and then by transaction start
([catalog.md](catalog.md#81-admission)). Calls of registered queries take
their bundle's priority class, and within it the `priority` the call
or its transaction gives; CallText and Append always rank below every
registered class. The queue is bounded, in calls and in the bytes its
calls hold while they wait
([catalog.md](catalog.md#81-admission)); a call that would exceed it is
answered at once with Error `Overloaded` carrying a `retry_after`
Duration, and is not executed. A call that alone holds more than the
whole byte bound is answered at once with `Rejected { reason:
StatementLimit { limit: Memory { bytes } } }`, `bytes` the byte bound,
never with `Retry`, and is not executed. A waiting call keeps its stream open
and may be cancelled.

Rulings: net/admission/connections, net/admission/queue, net/admission/classes

## 9. Feeds

### 9.1 What a feed carries

A feed delivers the committed change sets
([writes.md](writes.md#1-change-sets)) of a set of constructor relations,
in commit order. A feed is declared in a bundle with `feed NAME of
r.S`, which gives it an ordinal like a query and registers a row in
`catalog::feeds`. Its subscription belongs to that bundle and is
encoded at the bundle's pinned version, so the subscriber counts as a
reader of that version
([evolution.md](evolution.md#8-readers-first)). The feed's constructor
set is read as its membership at the pinned version, as any read of a
named or listed set is; enabling a constructor in the relation follows
readers-first like any read, and a constructor that joins a named set
later is not part of this bundle's feed.

A feed runs with definer rights like a registered query: the bundle's
owner must hold `read` on the feed's constructor set, checked at
registration and at every recheck; subscribing needs the `call` grant
on the bundle or on the feed.

```adt
-- bundles/sync.adb
bundle sync-1.0 pins 12 (c41e...) edition "2026-10"
feed oauth_changes of identities.OAuth
```

Each change set lists, per constructor relation of the feed, the rows
inserted and the rows deleted by one commit. A retag is a deletion
from one constructor relation and an insertion into another in the
same change set. An update that keeps the constructor is a deletion of
the old row and an insertion of the new one. Constructor relations of
the feed that a commit did not change are omitted from its change set,
and a commit that changed none of them is not delivered.

### 9.2 Subscribe

| Field | Encoding | Meaning |
| --- | --- | --- |
| `ordinal` | `uvarint` | The feed |
| `start` | `u8` then fields | 0: snapshot; 1: after a `position`; 2: from the named checkpoint `text` |
| `credit` | `u64` | Initial credit (§5.5) |
| options | `list option` | Keys 4 `trace` and 11 `compress` as in Call |

A subscription from a snapshot first sends the current rows of every
constructor relation of the feed as FeedSnapshot messages, then
FeedSnapshotEnd with the position the snapshot was taken at, then
FeedChanges for every later commit. A subscription after a position
sends FeedChanges for every commit after it. A subscription from a
checkpoint starts after the position last acknowledged under that name
for this bundle, or from a snapshot if none was. A position before the
feed was registered is answered with Error `HistoryUnavailable`,
carrying the oldest position kept as `Position { epoch, lsn }`. A
client that resnapshots by a read call instead may subscribe after the
`snapshot` position its End carried (§5.6). The stream stays open
until the client cancels it or the server ends it with Error.

### 9.3 Feed messages

FeedSnapshot:

| Field | Encoding |
| --- | --- |
| `relation` | `uvarint` relation variable id (§4.3), which code generation computes from the log |
| `ctor` | `uvarint` constructor tag (§4.3) |
| `rows` | `list` of rows encoded as the constructor's row type |

FeedChanges:

| Field | Encoding |
| --- | --- |
| `position` | `position` of the commit |
| `changes` | `list` of (`uvarint` relation variable id, `uvarint` constructor tag, `list` inserted rows, `list` deleted rows) |

Rows are encoded as the constructor's row type at the bundle's pinned
version (§4).

### 9.4 Retention and checkpoints

The server keeps change sets for a retention window, a file setting
([catalog.md](catalog.md#10-configuration)) of 7 days by default. A subscription
after a position older than the window is answered with Error `TooOld`
carrying the oldest position still kept as `Position { epoch, lsn }`; the client must subscribe
again from a snapshot. A client acknowledges with FeedAck on the feed
stream; when the subscription started from a named checkpoint, the
server stores the acknowledged position under that name, so a later
subscription resumes after it. A named checkpoint holds no change set
beyond the window: once its position falls out of the window, resuming
from it is answered with `TooOld`. Delivery after a resume is at least
once from the acknowledged position; a client must ignore change sets
at or before a position it has already applied.

Rulings: net/feed/changes, net/feed/retain/window, net/feed/retain/checkpoint, net/feed/version/bundle

## 10. Pushes

The server may send Push on stream 0 at any time after Ready, between
any two frames, including while the client's transactions are open. A
Push body is one value of the prelude sum `Notice`
([catalog.md](catalog.md#45-notices)), as `bytes` holding its encoding
with stable tags (§4.3), and matched exhaustively by clients. A server
sends only constructors of `Notice` as it stands in the edition named
in Ready.

```adt
type Notice = LeaseExpiring      { bundle: Text, at: Instant }
            | ShuttingDown       { at: Instant }
            | ConstructorEnabled { relation: Text, ctor: Text }
            | BundleRetired      { bundle: Text }
            | BlockingMigration  { bundle: Text, version: I64 }
            | UpgradeRequired    { bundle: Text }
```

| Notice | Meaning |
| --- | --- |
| `LeaseExpiring` | The bound bundle's lease ends at `at` unless a client renews it |
| `ShuttingDown` | The server accepts calls on this connection until `at` and opens no new stream after it |
| `ConstructorEnabled` | Writes to constructor `ctor` of `relation` are enabled, or, after `enable T.C`, storing values of `ctor` of the value-level sum named in `relation` ([evolution.md](evolution.md#8-readers-first)) |
| `BundleRetired` | The bound bundle has been retired; new bindings to it, and calls on connections bound to it, are refused |
| `BlockingMigration` | The bound bundle blocked a migration that would produce `version`, or a revoke issued at `version`, and it was refused; nothing waits. Sent to the bundle's sessions for the `BlockingNoticePeriod` setting, 24 hours by default, after the refusal ([evolution.md](evolution.md#8-readers-first)) |
| `UpgradeRequired` | Clients of the bound bundle are asked to upgrade to a newer bundle |

Field types are those catalog.md gives.

Rulings: net/push/frames

## 11. Errors

### 11.1 Errors are values

An Error body after Ready is one value of the prelude sum `Error`
([catalog.md](catalog.md#4-error-sums)), as `bytes` holding its
encoding with stable tags (§4.3), and each reason inside it is
likewise length-prefixed, so a client can skip a reason it cannot
decode and still act on the constructor. Its constructors say what the client does:

```adt
type Error = Retry    { reason: RetryReason }
           | Rejected { reason: RejectReason }
           | Fatal    { reason: FatalReason }
```

Each reason is a closed sum pinned by edition: the server sends only
constructors of the edition named in Ready. Edition versioning covers
`Error`, `Notice` and the other prelude sums only; catalog sums
evolve through the catalog log. Client code
generated from that edition matches the top level exhaustively and may
match a reason. catalog.md owns the full constructor lists and their
fields; this part names an error by its reason.

### 11.2 Errors of the protocol

The reasons this part produces:

| Constructor | Reason | When | Client action |
| --- | --- | --- | --- |
| `Retry` | `Serialization` | A transaction lost a serialization conflict and was not, or no longer, retried by the server | Rerun the transaction |
| `Retry` | `Overloaded` | Connection, stream or queue bound reached (§8); carries `retry_after` after Ready | Retry after the delay |
| `Retry` | `StreamIdle` | No credit for longer than the limit before commit (§5.8) | Read faster or grant more credit |
| `Rejected` | `TransactionExpired` | The interactive transaction was aborted at its time or idle limit (§6.2) | Shorten the transaction |
| `Rejected` | `TransactionBusy` | The transaction is in use from another connection (§6.2) | Wait for the other connection's request |
| `Rejected` | `Cancelled` | The client cancelled the call (§5.7) | None |
| `Rejected` | `DeadlineExceeded` | The call's deadline passed | None |
| `Rejected` | `StatementLimit` | The call ran past its statement limit, or its arguments exceed the admission byte bound (§8) | None |
| `Rejected` | `Violated` | The change set violated a constraint ([constraints.md](constraints.md#7-checking-and-violation-reports)); carries one constraint, its report and the redacted count (below) | Report to the user |
| `Rejected` | `Denied` | The role lacks the grant the request needs | None |
| `Rejected` | `Invalid` | Ad hoc text failed to parse or check; carries diagnostics | Fix the text |
| `Rejected` | `SchemaMismatch` | An Append schema is not the target's row type (§7.3) | Fix the batches |
| `Rejected` | `UnknownQuery` | No query, function or feed with that ordinal in the binding | Regenerate client code |
| `Rejected` | `KeyViolation` | Two rows of one statement's change set have one key, an insert gives the key of an existing row the statement does not delete, or an appended batch repeats a key; a repeated unique is `Violated` | Fix the input |
| `Rejected` | `UnknownTransaction` | The id names no live transaction of this role and binding; on a Commit with a request number, the transaction did not commit, and without one nothing is known of its outcome | Rerun the transaction when it did not commit |
| `Rejected` | `UnknownBundle` | The bundle is not registered | Register or deploy it |
| `Rejected` | `VersionMismatch` | The bundle is registered at another version or hash | Regenerate client code |
| `Rejected` | `Retired` | The bundle is retired | Deploy a newer bundle |
| `Rejected` | `BundleExpired` | The bundle's lease has expired: a bind, call or renewal of it, on the primary or a replica (§3.4) | Register the bundle again, or deploy a newer one |
| `Rejected` | `BundleSuperseded` | The bundle was superseded, by an embedded open or by `supersede bundle`, and not registered again ([applications.md](applications.md#8-embedded-databases)) | Register the bundle again |
| `Rejected` | `CatalogMismatch` | Hello names a catalog version other than the one the bundle pins (§3.4) | Regenerate client code |
| `Rejected` | `EditionLocked` | Hello asks for an edition on a bound connection (§3.2) | Drop the option |
| `Rejected` | `UnknownEdition` | Hello asks for an edition the server does not implement (§3.2) | Ask for an edition the server implements |
| `Rejected` | `MixedVersions` | A call would run at another schema or catalog version than its transaction's earlier calls, as when a transaction mixes ad hoc text and registered calls (§7.1) | Split the transaction |
| `Rejected` | `TooOld` | A feed position is past retention (§9.4); carries the oldest position kept as `oldest: Position { epoch, lsn }` | Resubscribe from a snapshot |
| `Rejected` | `HistoryUnavailable` | A feed position precedes the feed's registration (§9.2); carries the oldest position kept as `oldest: Position { epoch, lsn }` | Subscribe from a snapshot |
| `Rejected` | `PositionNotOnHistory` | A position is not on this database's history | Drop the position |
| `Rejected` | `ReadOnlyReplica` | A write reached a replica (§6.6); names the primary | Send it to the primary |
| `Retry` | `SafePointTimeout` | A replica read with `after` found no safe point covering it within `ReplicaWait` (§6.6) | Retry, or read on the primary |
| `Retry` | `ReplayConflict` | A replica read was cancelled after log replay waited `ReplicaVacuumWait` to remove a row version its snapshot sees | Retry, or read on the primary |
| `Rejected` | `RequestReused` | A request number was reused for a different call or arguments (§6.5) | Fix the client's numbering |
| `Rejected` | `RequestExpired` | A request number with no record is strictly below the deduplication low-water mark (§6.5) | Check the outcome by a read |
| `Rejected` | `UnknownDatabase` | Hello names a database the process does not serve | Fix the address |
| `Rejected` | `AuthFailed` | Authentication failed (§3.3) | None |
| `Rejected` | `UnsupportedProtocol` | No common protocol version (§13); carries the server's range | Upgrade one side |
| `Fatal` | `Malformed` | The message violates this part | Fix the driver |

`Violated` names one constraint, the violated constraint with the
smallest name, and carries its report: a ResultType descriptor (§7.2)
followed by a Rows body of the denial query's rows the caller may
read, and the number of rows redacted. A registered call reads with
its bundle owner's rights, ad hoc text with its role's own grants
([security.md](security.md#61-constraint-violations)).

### 11.3 Malformed messages

A malformed message on a stream other than 0 whose header was valid is
answered with Error `Malformed` on that stream. A malformed frame
header, a malformed message on stream 0, or any malformed message
before Ready is answered with Error `Malformed` on stream 0, after
which the server closes the connection. A client that receives a
malformed message must close the connection.

Rulings: net/errors/sum

## 12. Compression

Hello lists the codecs the client accepts, from `zstd` and `lz4`
(LZ4 frame format); Ready names the one the server chose, or none.
Compression is per call and off by default. A request that carries the
option `compress`, on a connection with a chosen codec, has the
server's data frames on its stream compressed, and allows the client
to compress its own AppendData frames on it. A compressed frame sets
`COMPRESSED`: its body is the codec's compressed form of the frame's
uncompressed body, `length` is the compressed length, and the
uncompressed body must not exceed the receiver's `max_frame`. Each
frame is compressed on its own, and the server compresses a data frame
of such a stream only when its uncompressed body is at least 1024
bytes. Frames before Ready are never compressed. Compression covers
rows and Arrow results alike; Arrow body compression is not used
(§5.4).

Compressing a frame that holds both values an attacker can influence
and values the attacker must not learn lets an observer of frame
sizes infer the latter, even under TLS. A client should ask for
compression only for results that do not mix the two.

Rulings: net/compression/frame

## 13. Protocol versions

### 13.1 Negotiation

A protocol version is a major and a minor number. This edition defines
protocol 1.0. The client offers the range `proto_min` to `proto_max` in
Hello. The server chooses the highest version in that range it
supports and names it in Ready, or, if there is none, answers Error
`UnsupportedProtocol` with the range it supports and closes. Within a
major version a later minor version only adds message kinds, options,
features or flag bits; it never changes the meaning of what an earlier
minor version defined.

### 13.2 Features

A feature is a named capability offered in Hello and granted in
Ready. A peer must not use a feature that was not granted. Protocol
1.0 defines the feature `arrow` (Arrow format results, §5.4) and
`feeds` (Subscribe, §9); a server that grants neither still conforms.

### 13.3 Compatibility promise

Each server release states the oldest protocol version it accepts. A
server release must accept every protocol version that any server
release of the preceding 24 months accepted, so a client keeps working
across that window of server upgrades. The schema version a bundle
pins is separate from the protocol version and is never negotiated:
it is served by version translation
([evolution.md](evolution.md#7-version-translation)).

Rulings: net/version/range, ops/protocol/negotiate, ops/promise/protocol

## 14. Parity with the in-process API

Every request message is an operation of the in-process API with the
same meaning; the server is a client of the library and adds only
transport, authentication, flow control and encoding.

| Message | In-process operation ([embedding.md](embedding.md)) |
| --- | --- |
| Hello binding | `Database::bundle` |
| Call | `Bundle::call`, `Bundle::call_arrow` |
| CallText | `Database::query_text` |
| Begin | `Bundle::begin` |
| Call with `txn` | `Transaction::call` |
| Commit, Rollback | `Transaction::commit`, `Transaction::rollback` |
| Subscribe, FeedAck | `Bundle::subscribe`, `Feed::ack` |
| Append, AppendData, AppendEnd into one relation | `Database::load` with one appender, `Appender::push`, `Load::commit` |
| Begin in an ad hoc session | `Database::begin` |
| CallText with `txn` | `Transaction::query_text` |
| Cancel | `CancelToken::cancel` |
| Push | `Bundle::notices` |

The in-process API has operations with no message: opening a file,
host functions and the metrics callback
([embedding.md](embedding.md#11-parity-with-the-server)). Credit,
compression, authentication, Ping and request numbers exist only on
the wire.

Rulings: net/parity/mirror, surface/wire/embedded
