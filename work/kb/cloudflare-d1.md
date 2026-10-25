# Cloudflare D1

kind: system
home: https://developers.cloudflare.com/d1/

## Replication

- Read replicas updated asynchronously from one primary; a replica may be arbitrarily out of date. Sessions API gives sequential consistency within a session (monotonic reads, read-your-writes, writes-follow-reads) by carrying a bookmark; a new session can start from a bookmark or `first-primary`. [V 2026-10-25] https://developers.cloudflare.com/d1/best-practices/read-replication/

## Backup

- Time Travel: restore to any minute in the last 30 days (7 on free); bookmarks derived from timestamps, lexicographically sortable; restore overwrites in place and returns a bookmark to undo it. [V 2026-10-25] https://developers.cloudflare.com/d1/reference/time-travel/
