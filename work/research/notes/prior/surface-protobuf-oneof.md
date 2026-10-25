---
title: Protobuf oneof on the wire
---
A oneof has no discriminant on the wire: each member is an ordinary
field number, and the last member parsed wins. Zero members set is a
valid message. Generated Go represents a oneof as an interface with one
wrapper struct per member. Field numbers, not names, are the identity,
so renames are wire-compatible.
https://protobuf.dev/programming-guides/proto3/
https://protobuf.dev/reference/go/go-generated/
