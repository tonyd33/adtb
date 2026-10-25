---
title: Where the old payload of a retagged row is visible
pick: any
raised-by: [union-index/retag/move]
options:
  in-scope:
    title: Old fields are readable while computing the new payload
  returning-old:
    title: The write returns the old constructor value
    links:
      - {rel: prior-art, to: prior/write-pg-constraints}
---
After a retag the old fields exist nowhere. Without either option a
program reads them in a separate query first, racing concurrent writers.

## write/retag/old/in-scope

```
update i in identities where i.id == k
set match i
  | EmailPass e -> Google { gid: derive(e.email) }
  | Google g    -> g
```

`e` is bound to the pre-state `EmailPass` row.

## write/retag/old/returning-old

```
from o in (update i in identities.EmailPass where i.id == k set Google { gid: g } returning old)
select match o | EmailPass e -> e.email | Google g -> g.gid
```

Postgres `RETURNING old.*`. The result is the sum, matched as usual.
