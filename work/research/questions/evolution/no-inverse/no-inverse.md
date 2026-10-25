---
title: Scripted migrations have no inverse to serve old versions
pick: any
raised-by: [[evolution/scripted, evolution/breaks/versioned]]
options:
  escape-only:
    title: Scripts only as an escape hatch, versioned only through operators
    needs-one-of: [evolution/smo, evolution/lens]
  live:
    title: Live with old versions frozen read-only after a script
    excludes: [evolution/no-inverse/escape-only]
---
An old schema version is served by translating through each change's
inverse. A script has none, and nothing checks it preserves information.

## evolution/no-inverse/escape-only

A version boundary crossed by a script ends the old version.

## evolution/no-inverse/live

The old version keeps a snapshot.
