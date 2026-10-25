---
title: pgroll versioned schemas
---
Each migration creates a versioned schema of views over the physical
tables; old and new versions are served at once, selected by
`search_path`, with triggers keeping expanded columns in sync until the
migration completes and the old version is dropped.
https://github.com/xataio/pgroll
