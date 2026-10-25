# OpenTelemetry

kind: format
home: https://opentelemetry.io/docs/specs/semconv/

## Database spans

- Attributes: db.system.name required; db.namespace, db.collection.name, db.operation.name conditionally required; db.query.text, db.query.summary, db.stored_procedure.name, server.address recommended; db.response.status_code and error.type required on failure. Marked stable. [V 2026-10-25] https://opentelemetry.io/docs/specs/semconv/database/database-spans/
- Span name is db.query.summary when available, else "{db.operation.name} {target}", else target, else db.system.name. Non-parameterized query text collected by default only if sanitized (literals to ?); parameterized query text SHOULD NOT be sanitized. [V 2026-10-25] https://opentelemetry.io/docs/specs/semconv/database/database-spans/
