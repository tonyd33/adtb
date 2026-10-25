---
title: Protobuf and Avro union evolution
---
Protobuf: adding enum values is safe and unknown values are preserved;
moving fields into or out of a `oneof` loses data, except into a new
oneof. Avro: a writer's union branch absent from the reader's union is
an error, so adding a branch is backward but not forward compatible.
https://protobuf.dev/programming-guides/proto3/
https://avro.apache.org/docs/1.11.1/specification/
