---
title: Cap'n Proto RPC promise pipelining
---
A call's result may be passed to further calls to the same server
before it returns; `bar(foo())` costs one round trip. References are
capabilities.
https://capnproto.org/rpc.html
