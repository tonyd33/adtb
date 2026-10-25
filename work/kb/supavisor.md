# Supavisor

kind: system
home: https://github.com/supabase/supavisor

## Pooling

- Multi-tenant Postgres connection pooler in Elixir by Supabase; README claims 1 million connections on a cluster, 250,000 idle connections on one 16-core 64GB node, about 90% of PgBouncer throughput locally. [V 2026-10-25] https://github.com/supabase/supavisor
- Transaction mode supported; session mode listed as future work in the README as fetched. [S 2026-10-25] https://github.com/supabase/supavisor
