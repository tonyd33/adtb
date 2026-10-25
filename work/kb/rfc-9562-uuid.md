# RFC 9562, UUIDs

kind: format
home: https://www.rfc-editor.org/rfc/rfc9562.html

## UUIDv7

- Layout: 48-bit big-endian unix_ts_ms, 4-bit ver (7), 12-bit rand_a, 2-bit var, 62-bit rand_b. [V 2026-10-25] https://www.rfc-editor.org/rfc/rfc9562.html sec. 5.7
- Monotonicity within a millisecond, sec. 6.2: Method 1 a fixed-length counter in (leftmost bits of) rand_a; Method 2 monotonic random, random data incremented in the least significant position; Method 3 leftmost rand_a bits replaced by sub-millisecond clock precision. [V 2026-10-25] same
- UUIDs should be stored in databases as the 128-bit binary value; time ordering improves index locality; the timestamp is a small attack surface. [V 2026-10-25] same sec. 6.13
