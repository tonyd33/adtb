---
title: Where audit records live
pick: one
raised-by: [security/audit/auth, security/audit/ops, security/audit/calls, security/audit/reads, security/audit/writes]
options:
  relation:
    title: An append-only system relation whose type is a sum of event kinds
    needs: [engine/catalog-relations]
  stream:
    title: A file or socket stream outside the database
---
An audit record written in the audited transaction rolls back with it;
a denied call must still be recorded.

## security/audit-store/relation

```
type AuditEvent = Login { ... } | Call { bundle: Text, query: Text, caller: Principal, params: Bytes }
                | Grant { ... } | SchemaOp { op: I64, author: Principal } | Denied { ... }
relation audit : AuditEvent key seq
```

Written by the engine in its own transaction; no role may update or
delete. Queried with the same language and matched exhaustively.

## security/audit-store/stream

Syslog or a JSON stream to a collector.
