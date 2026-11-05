---
title: When a dump of a file quarantined whole runs
pick: one
options:
  prove-then-dump:
    title: The dump first proves the commit log and catalog pages sound with verify's checks; any finding refuses it as Quarantined naming the database
  settled-status:
    title: The dump reads every row whose writer's status is settled, from undamaged status pages or the retained log, and refuses only rows it cannot settle
  refuse:
    title: A file quarantined whole refuses dump
---
Catalog 11.2 says a file quarantined whole opens for verify and dump only, so the readable data can be got out. Its usual causes are a damaged commit log or catalog page, often a lost write that an open checking only checksums does not catch; a dump reading visibility from such a page can omit rows silently under a manifest claiming completeness.

## repl/dump-quarantined/prove-then-dump

Sound by construction: the dump runs only over a commit log and catalog verify finds intact, so its rows are the snapshot's. A file whose damage lies elsewhere dumps; one whose log or catalog is damaged refuses, so the promise of 11.2 holds only for damage outside them, which the spec must say.

## repl/dump-quarantined/settled-status

Recovers more: a damaged status page loses only the transactions it covers. Needs a reader of the retained log for visibility and a dump manifest that names what it could not settle, which the dump format does not have.

## repl/dump-quarantined/refuse

Contradicts catalog 11.2 and leaves a damaged file with no logical way out but backup, which copies the damage.
