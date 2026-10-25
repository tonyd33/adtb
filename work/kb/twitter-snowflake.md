# Twitter Snowflake ids

kind: format
home: https://en.wikipedia.org/wiki/Snowflake_ID

## Layout

- 64 bits: top bit 0, 41-bit milliseconds since a chosen epoch, 10-bit machine id, 12-bit per-machine sequence; announced by Twitter 2010-06-01; sortable by creation time. Discord (epoch 2015), Instagram (41-bit time, 13-bit shard, 10-bit sequence), Mastodon (48-bit time, 16-bit sequence) use variants. [S 2026-10-25] https://en.wikipedia.org/wiki/Snowflake_ID
