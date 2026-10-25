# Wasmtime

kind: system
home: https://wasmtime.dev/

## Limits

- consume_fuel: deterministic, instruments code to count fuel; higher overhead. epoch_interruption: cheaper check of a global counter, non-deterministic. Bound heap with a ResourceLimiter for strict limits. [V 2026-10-25] https://docs.wasmtime.dev/api/wasmtime/struct.Config.html
