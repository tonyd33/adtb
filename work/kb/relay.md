# Relay

kind: system
home: https://relay.dev

## Persisted queries

- The compiler converts operation texts to md5 hashes; the request carries an `id`; the server looks up the text from the stored `queryMap.json`; servers can allowlist queries. [V 2026-10-25] https://relay.dev/docs/guides/persisted-queries/
