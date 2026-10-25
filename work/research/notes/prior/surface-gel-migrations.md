---
title: Gel SDL, migrations and codegen
---
Schema in `.gel` SDL files. `gel migration create` sends the SDL to the
server, which plans the change and asks a sequence of questions until
the plan is clear; each proposed step carries a confidence in 0..1.
The plan is written as DDL to `dbschema/migrations`. Migration ids are
content hashes chained by `onto` (unverified). The binary protocol
returns input and output type descriptors on Parse; a client declaring a
stale descriptor id gets the new descriptor or a mismatch error.
`@gel/generate queries` sends each `.edgeql` file to the server and
emits a typed function from the returned descriptors.
https://docs.geldata.com/reference/datamodel/migrations
https://docs.geldata.com/reference/reference/protocol
https://docs.geldata.com/reference/using/js/generation
