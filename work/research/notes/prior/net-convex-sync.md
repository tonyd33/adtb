---
title: Convex sync protocol
---
One WebSocket per client; the server keeps each subscribed query's
read set and reruns it when a commit overlaps; a client's query set is
at one timestamp. Mutations are OCC transactions rerun on conflict by
the server.
https://stack.convex.dev/how-convex-works
