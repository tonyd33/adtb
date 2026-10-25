---
title: Gel access policies
---
Per-type `allow`/`deny` policies on select, insert, update read, update
write, delete, guarded by `when` and `using` over global variables.
Allows union, denies subtract; with any policy present the default is
deny. Reads filter, writes error. Subtypes inherit policies.
https://docs.geldata.com/reference/datamodel/access_policies
