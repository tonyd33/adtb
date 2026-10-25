# GraphQL

kind: format
home: https://spec.graphql.org/

## Unions

- Fragments on union members are conditional; covering every member not required. [V 2026-10-25] https://spec.graphql.org/October2021/
- graphql-js `findBreakingChanges`: `TYPE_ADDED_TO_UNION`, `VALUE_ADDED_TO_ENUM` dangerous; `TYPE_REMOVED_FROM_UNION` breaking. [V 2026-10-25] https://raw.githubusercontent.com/graphql/graphql-js/v16.9.0/src/utilities/findBreakingChanges.ts
- Apollo iOS codegen represents unknown union cases. [S 2026-10-25] https://github.com/apollographql/apollo-ios/blob/main/CodegenProposal.md
- Apollo Kotlin adds `UNKNOWN__` to enums but threw `ApolloParseException` on unknown union members in fragments. [S 2026-10-25] https://github.com/apollographql/apollo-kotlin/issues/2097
- Apollo Client needs a `possibleTypes` map for union fragments. [V 2026-10-25] https://www.apollographql.com/docs/react/data/fragments
