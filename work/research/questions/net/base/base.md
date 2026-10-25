---
title: What the server protocol is built on
pick: one
raised-by: [surface/wire/binary]
options:
  own:
    title: An own framed protocol over a byte stream
    links:
      - {rel: prior-art, to: prior/net-pg-protocol}
      - {rel: prior-art, to: prior/net-mongo-wire}
  flight:
    title: Arrow Flight over gRPC, with registered calls as Flight commands
    links:
      - {rel: prior-art, to: prior/net-flight}
  grpc:
    title: A gRPC service with protobuf messages
    links:
      - {rel: prior-art, to: prior/net-quic}
  capnp:
    title: Cap'n Proto RPC with promise pipelining
    links:
      - {rel: prior-art, to: prior/net-capnp-rpc}
---
No SQL wire protocol is reused. The base decides framing, multiplexing,
flow control and which clients exist without writing a driver.

## net/base/own

```
[type u8][len u32][body]        -- Hello, Call, Batch, End, Error, Push
```

Postgres v3, MongoDB OP_MSG, TigerBeetle. Full control of framing,
cancel and pinning; every language needs a driver.

## net/base/flight

```
GetFlightInfo(cmd = Call { query: login_key, args: [id] }) -> tickets
DoGet(ticket) -> Arrow IPC stream
```

Results are already Arrow batches (`exec/batch/arrow`). Flight SQL
clients and ADBC exist, but their command set is SQL text and
catalog metadata; a registered call is an opaque command
(https://arrow.apache.org/docs/format/FlightSql.html). Transactions
are actions, so an interactive transaction is several RPCs tied by a
handle.

## net/base/grpc

```
service Adt { rpc Call(CallRequest) returns (stream Batch); }
```

HTTP/2 streams give multiplexing and flow control. Protobuf `oneof`
lacks totality (`surface/sum-encoding/field-presence`), so sums travel
as bytes inside messages.

## net/base/capnp

```
let u = db.call(find_user, email)       -- promise
db.call(add_identity, u.key, g)         -- sent before u returns
```

Promise pipelining removes round trips in an interactive transaction
(https://capnproto.org/rpc.html). Unions carry a tag and ordinals, as
the stable tag does; few language implementations (unverified).
