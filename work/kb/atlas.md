# Atlas

kind: system
home: https://atlasgo.io

## Migrations

- Declarative: desired schema state is input; the engine plans and executes actions to reach it. Versioned: developers keep source-controlled migration files. [V 2026-10-25] https://atlasgo.io/concepts/declarative-vs-versioned
- `atlas migrate diff` computes the difference between migration history and the new schema and writes a migration file; `atlas migrate lint` analyzes migration files for issues and policy violations. [V 2026-10-25] https://atlasgo.io/concepts/declarative-vs-versioned
- Teams often work declaratively in development and switch to versioned files when sharing. [V 2026-10-25] https://atlasgo.io/concepts/declarative-vs-versioned
