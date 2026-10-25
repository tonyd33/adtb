---
title: Atlas declarative and versioned migrations
---
Declarative: desired state in, engine plans and applies. Versioned:
`atlas migrate diff` writes the planned change as a file into a
migrations directory, reviewed and linted (`atlas migrate lint`)
before apply. Teams develop declaratively and ship versioned.
https://atlasgo.io/concepts/declarative-vs-versioned
