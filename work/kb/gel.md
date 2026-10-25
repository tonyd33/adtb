# Gel (formerly EdgeDB)

kind: system
home: https://docs.geldata.com

## Data model

- Every object type has a required, globally unique, readonly, indexed `id: uuid` assigned on creation; users cannot set it. [V 2026-10-25] https://docs.geldata.com/reference/datamodel/objects
- Links are directional, `single` (default) or `multi`, optionally `required`; links may carry link properties. [V 2026-10-25] https://docs.geldata.com/reference/datamodel/links
- Link targets may be abstract types, so one link references any concrete subtype. [V 2026-10-25] https://docs.geldata.com/reference/datamodel/links
- `on target delete` policies: restrict, delete source (cascade), allow, deferred restrict; `on source delete`: allow, delete target, delete target if orphan. [V 2026-10-25] https://docs.geldata.com/reference/datamodel/links
- Backlinks are computed as `.<link[is Type]` rather than declared twice. [V 2026-10-25] https://docs.geldata.com/reference/datamodel/links
- Objects are never embedded as values; links are the only object-to-object relationship. [S 2026-10-25] https://docs.geldata.com/reference/datamodel/links

## Query language (EdgeQL)

- Path expressions traverse links/properties; a path not ending in a property yields a distinct set of objects; one ending in a property yields a list of values per object. [S 2026-10-25] https://docs.geldata.com/reference/reference/edgeql/paths
- Five static cardinality ranges: Empty, One, AtMostOne, AtLeastOne, Many; used to statically check queries. [S 2026-10-25] https://docs.geldata.com/reference/reference/edgeql/cardinality
- Missing value is the empty set; `??` coalesces; backlinks written `.<link[is Type]`; shapes give nested results. [U]

## Migrations

- `gel migration create` sends the `.gel` SDL files to the server, which generates a migration plan and sends it back to the CLI. [V 2026-10-25] https://docs.geldata.com/reference/datamodel/migrations
- When the plan needs clarification the CLI asks a sequence of questions until the plan is clear and approved; each proposed step carries `"confidence": (0..1)`. [V 2026-10-25] https://docs.geldata.com/reference/datamodel/migrations
- Migration files in `dbschema/migrations` hold DDL commands (e.g. `create type User { create required property name: str; }`). [V 2026-10-25] https://docs.geldata.com/reference/datamodel/migrations
- Migration ids are content hashes chained by `onto` a parent migration. [U]

## Protocol

- Parse returns `CommandDataDescription` with input and output type descriptors; a client declaring a mismatched input descriptor id gets the description then `ParameterTypeMismatchError`; a mismatched output descriptor gets a new `CommandDataDescription` before any Data. [V 2026-10-25] https://docs.geldata.com/reference/reference/protocol
- State carries a type descriptor id; mismatch returns `StateDataDescription` then `StateMismatchError`; the zero id for default state always matches. [V 2026-10-25] https://docs.geldata.com/reference/reference/protocol

## Codegen

- `npx @gel/generate queries` scans `*.edgeql` files, sends each query to the database for parameter and return type information, and emits a typed function per file. [S 2026-10-25] https://docs.geldata.com/reference/using/js/generation
## Constraints

- Standard constraints: `exclusive`, `expression on (...)`, `one_of`, `max_value`, `max_ex_value`, `min_value`, `min_ex_value`, `max_len_value`, `min_len_value`, `regexp`. [V 2026-10-25] https://docs.geldata.com/reference/datamodel/constraints
- Object-level composite uniqueness: `constraint exclusive on ((.x, .y))`. [S 2026-10-25] https://docs.geldata.com/reference/datamodel/constraints
- Expression constraints may reference only `single` properties and links of the object type; paths of more than one hop are refused. [V 2026-10-25] https://docs.geldata.com/reference/datamodel/constraints
- A constraint on a supertype applies globally across all inheriting types (e.g. one exclusive across subtypes); `delegated constraint` applies it per subtype instead. [V 2026-10-25] https://docs.geldata.com/reference/datamodel/constraints


## Writes

- `insert T { f := e }` returns the inserted object, by default only `id`; wrap in `select (insert ...) { shape }` for more fields. [V 2026-10-25] https://docs.geldata.com/reference/edgeql/insert
- Nested inserts: an `insert` expression may appear as a link value inside another `insert`. [V 2026-10-25] https://docs.geldata.com/reference/edgeql/insert
- Upsert: `insert ... unless conflict on .title else (update Movie set {...})`; works only against an `exclusive` constraint. [V 2026-10-25] https://docs.geldata.com/reference/edgeql/insert
- Bulk insert: `for item in json_array_unpack($data) union (insert ...)`. [V 2026-10-25] https://docs.geldata.com/reference/edgeql/insert
- Whether `unless conflict` works on an exclusive constraint inherited from a parent type: docs silent. [U]
- `update T filter ... set { ... }` with `:=`, `+=`, `-=` on multi links; returns ids by default. Docs say nothing on changing an object's type by update. [V 2026-10-25] https://docs.geldata.com/reference/edgeql/update

## Constraints

- Link deletion policies, on target delete: `restrict` (default), `delete source`, `allow`, `deferred restrict` (error deferred to end of transaction if still linked). On source delete: `allow`, `delete target`, `delete target if orphan`. [V 2026-10-25] https://docs.geldata.com/reference/datamodel/links

## Query

- `[is T]` type intersection filters out other types; in shapes `[is T].field` gives `{}` for non-matching; `.__type__.name` exposes the tag. No match construct or exhaustiveness check found. [V 2026-10-25] https://docs.geldata.com/reference/edgeql/paths , https://docs.geldata.com/reference/edgeql/select

## Links

- Link to an abstract type can target any non-abstract subtype. [V 2026-10-25] https://docs.geldata.com/reference/datamodel/links
- Union link targets `Named | Text`. [V 2026-10-25] https://docs.geldata.com/reference/stdlib/type
- Union-link bug "more than one row returned by a subquery", fixed in PR 1022. [V 2026-10-25] https://github.com/edgedb/edgedb/issues/1010
- Multiple inheritance of object types. [V 2026-10-25] https://docs.geldata.com/reference/datamodel/objects
- Abstract types pass properties to subtypes. [S 2026-10-25] https://docs.geldata.com/reference/datamodel/inheritance

## Migrations

- Interactive migrations prompt for an EdgeQL expression; `ALTER ... USING (expr)` converts data. [S 2026-10-25] https://docs.edgedb.com/guides/migrations/guide
- Hosted cloud service retired. [U]

## Dump

- Dump file: magic, format version, header block (type H, SHA1, protocol version, schema DDL, type ids, descriptors), then data blocks in the binary protocol's encoding. [S 2026-10-25] https://docs.edgedb.com/database/reference/dump_format (404 after rename)
- No data-only dump or restore. [S 2026-10-25] https://github.com/geldata/gel-cli/issues/1111

## Access policies

- `access policy name [when (cond)] {allow|deny} action[, ...] [using (expr)] [{ errmessage := ... }]`; actions select, insert, delete, update read, update write, all. No policies: access allowed. With any policy, only what an allow grants; deny subtracts, overriding allows. [V 2026-10-25] https://docs.geldata.com/reference/datamodel/access_policies
- Insert and update write violations are errors; select, delete, update read filter. Subtypes inherit all policies of supertypes. Policies use global variables; `apply_access_policies := false` disables them. [V 2026-10-25] same
