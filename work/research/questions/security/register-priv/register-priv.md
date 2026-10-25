---
title: What registering a bundle requires
pick: one
raised-by: [security/rights/definer, security/rights/declared]
options:
  holds-reads:
    title: The registrant must hold, with grant option, every relation grant the bundle uses
  deploy-role:
    title: A deploy role may register any bundle
---
Under definer rights a registered bundle is a grant of its reads to
whoever may call it.

## security/register-priv/holds-reads

```
$ adt register --bundle web-2.4 --as web_owner queries/
admin_dump  refused: web_owner lacks read identities.EmailPass { pw_hash }
```

Postgres grant options and their cascade on revoke.

## security/register-priv/deploy-role

Bundles are reviewed outside the database; the deploy role holds all.
