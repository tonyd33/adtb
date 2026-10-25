---
title: "What a quarantined index fails"
pick: one
raised-by: [ops/halt-scope/heap]
options:
  cost-only:
    title: "No read fails: queries do not use the index, so only cost changes; a write to a constructor relation the index covers fails"
  plan-fails:
    title: "A call fails when its plan uses the index"
---
Raised by the tier audit. Catalog §11.2 failed a call "when it uses a
quarantined index", so whether a read fails depended on the plan the
planner chose.

## spec/quarantine/index/cost-only

A quarantined index never fails a read. Queries do not use it, so
only their cost changes. A statement that writes a constructor
relation the index covers fails with `Fatal { reason: Quarantined {..}
}`, since the index cannot be maintained. `reindex` restores it.

```adt
-- index (user) in identities.(Google | Apple) quarantined
from i in identities.Google where i.user == $u select i.gid   -- runs, without the index
insert identities.Google { user: $u, gid: "g-2" }             -- fails
```

## spec/quarantine/index/plan-fails

A call fails when its plan uses the quarantined index. The same call
fails or runs by what the planner picks, and changes when the planner
improves; a user cannot tell from the statement whether it will run.
