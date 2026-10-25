---
title: Cap'n Proto unions
---
A union keeps a separate tag naming the set member, read by `which()`.
Adding members to a union is safe because readers already check the
tag; a field may move into a new union only if the union and its other
members are new. Members carry ordinals, so names may change freely.
https://capnproto.org/language.html
