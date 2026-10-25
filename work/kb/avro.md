# Apache Avro

kind: format
home: https://avro.apache.org/

## Schema resolution

- A writer union branch matching nothing in the reader's union signals an error: adding a branch is backward but not forward compatible. [V 2026-10-25] https://avro.apache.org/docs/1.11.1/specification/
