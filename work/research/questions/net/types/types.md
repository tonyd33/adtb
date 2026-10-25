---
title: How call arguments and results are typed on the wire
pick: one
raised-by: [surface/wire/binary]
options:
  signature:
    title: Fixed by the registered signature at the bound version; values carry no type information
  descriptor:
    title: Each call states descriptor ids; a mismatch returns the current descriptor first
    links:
      - {rel: prior-art, to: prior/surface-gel-migrations}
  self-describing:
    title: Every value carries its type, as in BSON or RESP3
    links:
      - {rel: prior-art, to: prior/net-resp3}
      - {rel: prior-art, to: prior/net-mongo-wire}
---
A registered query's parameter and result types are known at
registration (`lang/params/declared`, `exec/plan-cache/at-register`).

## net/types/signature

```
query login_key (id: Uuid) -> Set (EmailPass { email: Text } | Google { gid: Text })
-> Call 3 [16 bytes]
<- Batch [02 07 "g-123"]
```

Codegen and the engine agree because both read the bundle; a wrong
argument length is a protocol error. Ad hoc calls need a descriptor
anyway.

## net/types/descriptor

Gel: `CommandDataDescription` before data when the client's id is
stale. Survives a changed signature without re-registration.

## net/types/self-describing

Generic tools decode any reply; tags and field names cost bytes on
every row.
