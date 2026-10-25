# Cap'n Proto

kind: format
home: https://capnproto.org

## Unions

- A union keeps a separate tag tracking which member is set; by default the lowest-numbered member is set. [V 2026-10-25] https://capnproto.org/language.html
- Adding members to an existing union is safe because readers already check the tag. [V 2026-10-25] https://capnproto.org/language.html
- A field may move into a group or union only if the union and all its other fields are new; old readers may expect the original field. [V 2026-10-25] https://capnproto.org/language.html

## Encoding

- 8-byte words, little-endian; struct pointer is 2-bit kind, 30-bit signed word offset, 16-bit data words, 16-bit pointer words; data section then pointer section; later fields may fill padding left by earlier ones; data fields stored XORed with their defaults. [V 2026-10-25] https://capnproto.org/encoding.html

## RPC

- Promise pipelining: a call's result can be used in further calls to the same server before it returns, so `bar(foo())` is sent together in one round trip; the four-step nested file open example takes one round trip. [V 2026-10-25] https://capnproto.org/rpc.html
- Interface references are capabilities: they designate an object and confer permission to call it. Levels: 1 object references and pipelining, 2 persistent capabilities, 3 three-party handoff, 4 reference equality. [V 2026-10-25] https://capnproto.org/rpc.html
