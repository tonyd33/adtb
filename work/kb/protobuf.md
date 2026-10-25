# Protocol Buffers

kind: format
home: https://protobuf.dev

## Oneof

- A oneof has no discriminant on the wire: members are ordinary field numbers, the last parsed wins, and no member set is valid. [U]
- Go codegen represents a oneof as an interface implemented by one wrapper struct per member. [U] https://protobuf.dev/reference/go/go-generated/

## Evolution

- Adding enum values is safe; unknown values preserved on deserialize. Moving fields into or out of a oneof loses data except one field into a new oneof; oneof is at most one. Deletion clears oneof data. [V 2026-10-25] https://protobuf.dev/programming-guides/proto3/
