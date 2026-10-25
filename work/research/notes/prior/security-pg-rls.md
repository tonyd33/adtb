---
title: Postgres row-level security
---
Policies per table and command: USING filters visible rows, WITH CHECK
constrains written rows; permissive policies OR, restrictive AND; no
policy means default deny. Owners and BYPASSRLS roles bypass. Unique,
primary key and foreign key checks bypass RLS, which the docs name a
covert channel. security_barrier views and LEAKPROOF functions keep
user expressions from seeing rows before the filter.
https://www.postgresql.org/docs/current/ddl-rowsecurity.html
https://www.postgresql.org/docs/current/rules-privileges.html
