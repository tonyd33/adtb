---
title: "How a bundle calls a schema function"
pick: one
options:
  import:
    title: "import fn NAME gives it a bundle ordinal"
  by-name:
    title: "Call by name on the wire"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/schema-fn-calls/import

import fn NAME gives it a bundle ordinal. Calls stay ordinal-addressed and checked at registration.

## spec/schema-fn-calls/by-name

Call by name on the wire. Unchecked names on the wire.
