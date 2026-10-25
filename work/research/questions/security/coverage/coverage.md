---
title: What the encryption covers besides data pages
pick: some
raised-by: [security/encrypt/page, security/encrypt/file-stream]
options:
  log:
    title: The redo log, including full-page images
  spill:
    title: Temporary spill files of bounded per-query memory
forbid:
  - all: [security/encrypt/page, not security/coverage/log]
    why: full-page images put every page's plaintext in the log
---
Every page of the database file is covered, catalog and large-value
stores included; pg_tde leaves catalogs, statistics and server logs
plaintext. Full-page images (`layout/torn/fpw`) put whole pages in the
log.

## security/coverage/log

Log records encrypted per aligned slot (`buffer/io-path/log-align/slot-pad`),
nonce from the log position.

## security/coverage/spill

Spill files under an ephemeral key generated per process.
