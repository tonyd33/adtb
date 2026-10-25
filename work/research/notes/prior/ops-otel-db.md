---
title: OpenTelemetry database client semantic conventions
---
Stable span attributes db.system.name, db.namespace, db.collection.name,
db.operation.name, db.query.text, db.query.summary; span named by the
summary. Parameterised query text is safe to record unsanitised.
https://opentelemetry.io/docs/specs/semconv/database/database-spans/
