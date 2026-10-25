---
title: Supabase auth over Postgres RLS
---
Requests run as `anon` or `authenticated`; policies read `auth.uid()`,
null when unauthenticated, so comparisons silently fail. A service key
bypasses RLS.
https://supabase.com/docs/guides/database/postgres/row-level-security
