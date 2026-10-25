---
title: Postgres privileges and definer functions
---
GRANT per object and per column; PUBLIC gets EXECUTE on functions by
default; owners hold every privilege. SECURITY DEFINER functions run
with the owner's rights and must pin search_path.
https://www.postgresql.org/docs/current/ddl-priv.html
https://www.postgresql.org/docs/current/sql-createfunction.html
