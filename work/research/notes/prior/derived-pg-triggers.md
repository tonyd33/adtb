---
title: Postgres rules and triggers
---
Rules rewrite queries before planning; views are SELECT rules. Triggers are BEFORE/AFTER/INSTEAD OF, row or statement, with transition tables; same-event order is alphabetical; cascades have no depth limit. https://www.postgresql.org/docs/current/rules.html https://www.postgresql.org/docs/current/trigger-definition.html
