# Supabase

kind: system
home: https://supabase.com/docs

## Auth and RLS

- Requests run as Postgres roles anon or authenticated; policies read `auth.uid()` (null when unauthenticated) and `auth.jwt()`; only raw_app_meta_data is safe for authorization. Wrapping as `(select auth.uid())` makes an initPlan evaluated once per statement; index policy filter columns. Security definer functions with `search_path = ''` break recursive policies; secret keys or bypassrls roles bypass RLS. A missing grant errors (42501) before any policy runs. [V 2026-10-25] https://supabase.com/docs/guides/database/postgres/row-level-security
