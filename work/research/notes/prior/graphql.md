---
title: GraphQL unions and change classification
---
Fragments on union members are conditional; covering every member is not
required. graphql-js `findBreakingChanges` classes `TYPE_ADDED_TO_UNION`
as dangerous and `TYPE_REMOVED_FROM_UNION` as breaking. Clients carry
the cost: Apollo iOS represents unknown cases, Apollo Kotlin threw on
unknown union members in fragments (issue 2097).
https://raw.githubusercontent.com/graphql/graphql-js/v16.9.0/src/utilities/findBreakingChanges.ts
