---
title: Confluent Schema Registry compatibility
---
BACKWARD: upgrade all consumers before producing with the new schema.
FORWARD: producers first. FULL: either order. Transitive variants check
against every earlier version, not only the last.
https://docs.confluent.io/platform/current/schema-registry/fundamentals/schema-evolution.html
