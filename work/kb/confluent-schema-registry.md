# Confluent Schema Registry

kind: system
home: https://docs.confluent.io/platform/current/schema-registry

## Compatibility

- BACKWARD: upgrade all consumers before producing with the new schema. FORWARD: upgrade producers first. FULL: independent. Transitive variants extend the check to all earlier versions. [V 2026-10-25] https://docs.confluent.io/platform/current/schema-registry/fundamentals/schema-evolution.html
