# Datomic

kind: system
home: https://docs.datomic.com

## Identity and references

- Entities are named by entity ids, lookup refs (unique attribute plus value) or idents (keywords). [V 2026-10-25] https://docs.datomic.com/schema/schema-reference.html
- `:db.unique/identity` upserts: a tempid asserting an existing identity value resolves to that entity. `:db.unique/value` rejects the duplicate instead. [V 2026-10-25] https://docs.datomic.com/schema/schema-reference.html
- `:db.type/ref` attributes reference entities; with `:db/isComponent true`, retracting the parent retracts component entities. [V 2026-10-25] https://docs.datomic.com/schema/schema-reference.html
- `:db.cardinality/many` makes an attribute a set of values per entity. [V 2026-10-25] https://docs.datomic.com/schema/schema-reference.html
- Tuple types: composite (derived from other attributes), heterogeneous (fixed length, typed slots), homogeneous (variable length, one type). [V 2026-10-25] https://docs.datomic.com/schema/schema-reference.html

## Client API

- `q` takes the query as list data `[:find ?var ... :where clause ...]`; `db` gets a database value; `as-of`/`since` give values at a time-point; `with` applies tx-data speculatively. [V 2026-10-25] https://docs.datomic.com/client-api/datomic.client.api.html
## Constraints

- Entity specs: an entity with `:db.entity/attrs` (required attributes) and/or `:db.entity/preds` (entity predicates `[db eid]` run against db-after, all must succeed or the transaction aborts). [S 2026-10-25] https://docs.datomic.com/schema/schema-reference.html
- An entity spec applies only when the transaction asserts `:db/ensure` for that entity; never automatic. [S 2026-10-25] https://docs.datomic.com/schema/schema-reference.html
- Single transactor serializes all writes. [U] https://docs.datomic.com/


## Transactions

- Transactions processed serially by the transactor; report holds `:db-before`, `:db-after`, `:tx-data`, `:tempids`. [V 2026-10-25] https://docs.datomic.com/transactions/transaction-processing.html
- Tx data is a set with no order: "there is and can be no order" inside a transaction; `:db.error/datoms-conflict` for inconsistent sets. [V 2026-10-25] https://docs.datomic.com/transactions/model.html
- Tempids unify with existing entities through `:db.unique/identity` attributes (upsert). [V 2026-10-25] https://docs.datomic.com/transactions/model.html
- Entity predicates / specs see db-after, the proposed end-of-transaction value. [V 2026-10-25] https://docs.datomic.com/transactions/model.html
- `:db/cas` (entity, attr, expected, new) aborts if db-before differs. `:db/retractEntity` retracts every datom with the entity as e or v, recursing into components. Transaction functions are `[db-before args] -> tx-data` and see only their args. [V 2026-10-25] https://docs.datomic.com/transactions/transaction-functions.html

## Schema

- Any entity can carry any attribute; refs untyped; required attributes only via `:db/ensure`, never automatic or retroactive. [V 2026-10-25] https://docs.datomic.com/schema/schema-reference.html

## Time

- Filters: `as-of` ignores transactions after a point (tx id, basis t or instant); `since` keeps only datoms added after a point; `history` includes all past assertions and retractions and may see several values of one fact. Unfiltered db is fastest; for time-only questions use the Log API, since as-of/since filter scans. [V 2026-10-25] https://docs.datomic.com/reference/filters.html
- Excision removes datoms matching a predicate from index and log, irrevocably, for privacy or retention; asynchronous at indexing; large excisions cost time proportional to the whole database; not for error correction. [V 2026-10-25] https://docs.datomic.com/operation/excision.html
