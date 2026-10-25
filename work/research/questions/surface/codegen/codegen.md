---
title: What typed client code is generated
pick: any
options:
  schema-types:
    title: Host types for every sum and constructor in the schema
    links:
      - {rel: prior-art, to: prior/surface-prisma}
  query-functions:
    title: One function per registered query with exact argument and result types
    links:
      - {rel: prior-art, to: prior/surface-sqlc}
      - {rel: prior-art, to: prior/surface-gel-migrations}
  builder:
    title: A typed query builder in the host language
---
Generated from the engine's descriptors, so the client's types are the
checked types.

## surface/codegen/schema-types

```rust
pub enum Identity { EmailPass { id: Uuid, email: String, hash: Vec<u8> },
                    Google { id: Uuid, gid: String } }
```

```ts
type Identity = { tag: "EmailPass"; email: string } | { tag: "Google"; gid: string };
```

Prisma Client from `schema.prisma`.

## surface/codegen/query-functions

```rust
pub enum LoginKey { ByEmail { email: String }, ByGoogle { gid: String } }
pub fn login_key(c: &Conn, u: UsersKey) -> Result<Vec<LoginKey>>;
```

A result sum is the one in the query signature, built by the match's
arms, not the schema's sum. sqlc,
Gel `generate queries`.

## surface/codegen/builder

```ts
e.select(e.Identity, i => ({ key: e.match(i, { EmailPass: p => p.email, Google: g => g.gid }) }))
```

Gel's `edgeql-js` builder, jOOQ.
