# RelationalAI Rel

kind: language
home: https://rel.relational.ai

## Entities and value types

- Entity and value type declarations create constructor relations prefixed `^`. [S 2026-10-25] https://rel.relational.ai/rel/ref/data-types/entity
- An entity is a hash of the values used to construct it plus the type name, so entities of different types never coincide; after creation it is not tied to those values. [S 2026-10-25] https://rel.relational.ai/rel/ref/data-types/entity
- A value type (phone number, email, unitful measure) is a separate data type identified by its data. [S 2026-10-25] https://rel.relational.ai/rel/ref/declarations/entities
