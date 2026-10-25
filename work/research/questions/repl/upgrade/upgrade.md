---
title: Upgrading the engine across replicas
pick: some
raised-by: [repl/unit/physical, repl/unit/change-set, repl/unit/command]
options:
  replicas-first:
    title: A new engine replays the old log format; replicas upgrade before the primary
    links:
      - {rel: variant-of, to: evolution/old-readers/readers-first}
  logical-bridge:
    title: A change-set replica on the new engine version catches up, then becomes primary
    needs: [repl/unit/change-set]
    links:
      - {rel: prior-art, to: prior/repl-pg-logical}
  multiversion:
    title: One binary embeds past releases and runs the version the group runs
    links:
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
---
A physical replica must read every page and log record the primary
writes. Engine versions face the same ordering as schema versions:
readers before writers.

## repl/upgrade/replicas-first

The primary writes the new format only after every replica reports
the new version, the readers-first rule applied to the engine.

## repl/upgrade/logical-bridge

The usual Postgres major-version upgrade path (unverified as the
documented one).

## repl/upgrade/multiversion

TigerBeetle multiversion binaries re-exec the release the cluster
runs, and data files migrate through staged releases.
