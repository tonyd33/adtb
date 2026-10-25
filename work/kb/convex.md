# Convex

kind: system
home: https://www.convex.dev

## Queries

- Queries are TypeScript functions in `convex/` deployed with the backend; clients call them through a generated typed `api` object; non-JS clients name them as strings `myFunctions:myQuery`. [V 2026-10-25] https://docs.convex.dev/functions/query-functions
- Clients cannot run ad hoc queries, only deployed functions. [S 2026-10-25] https://docs.convex.dev/functions/query-functions

## Reactivity

- Tracks each query function's dependencies (read set) and pushes updates to client subscriptions; all subscriptions of a client update to the same database snapshot together. [V 2026-10-25] https://docs.convex.dev/realtime

## Sync

- Client holds one WebSocket to the deployment; the sync worker keeps each subscribed query's read set in the session and refreshes it when a committed write overlaps it; all queries in a client's query set are at one timestamp. [V 2026-10-25] https://stack.convex.dev/how-convex-works
- Mutations are serializable OCC transactions: read set, write set and begin timestamp go to the committer; on conflict the function is rerun automatically at a later timestamp. [V 2026-10-25] https://stack.convex.dev/how-convex-works

## Auth

- Functions read `ctx.auth.getUserIdentity()` (tokenIdentifier, subject, issuer guaranteed); authorization is checked by hand in each function; internalQuery and internalMutation are not callable by clients. [V 2026-10-25] https://docs.convex.dev/auth/functions-auth
