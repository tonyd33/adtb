---
title: FoundationDB Record Layer metadata evolution
---
Metadata carries a version incremented on every change, plus versions
for when each record type and index was added. `MetaDataEvolutionValidator`
refuses new metadata that would need stored data migrated. Record types
are members of a protobuf union descriptor; field numbers carry identity,
so removal is deprecation.
https://foundationdb.github.io/fdb-record-layer/SchemaEvolution.html
