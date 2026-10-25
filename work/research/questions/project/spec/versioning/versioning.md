---
title: How the spec is versioned
pick: one
options:
  editions:
    title: Dated editions, each replacing the last
    links:
      - {rel: prior-art, to: prior/project-sql-standard-structure}
  semver:
    title: Major.minor, with minor versions only adding
  engine-release:
    title: The spec version is the engine release
---
Distinct from the database's schema version (`surface/version/counter`)
and the file format version (`layout/format-version/read-old`). Adding
a feature to the language can break exhaustive client matches only if
it adds constructors to a prelude sum; everything else is additive.

## project/spec/versioning/editions

SQL:1999 ... SQL:2023; "each version replaces the previous one, so
claims of conformance to earlier versions have no official merit"
(Postgres Appendix D).

## project/spec/versioning/semver

`adtdb-spec 1.3`: a 1.x engine runs every 1.0 program unchanged.

## project/spec/versioning/engine-release

SQLite and Postgres: the manual of release N is the spec of release N.
