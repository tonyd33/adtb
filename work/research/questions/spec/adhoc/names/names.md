---
title: "How unqualified names resolve in ad hoc text"
pick: one
options:
  every-module:
    title: "Ad hoc text belongs to no module; an unqualified name resolves in every user module, then the prelude"
  session-module:
    title: "The session names a module, which resolves first"
  qualified:
    title: "Every user name in ad hoc text is qualified"
---
Raised while implementing the front end against docs/spec/. A file
resolves in its own module and a bundle in its imports, but ad hoc text
has neither, and the spec said nothing on how its names resolve.

## spec/adhoc/names/every-module

Ad hoc text belongs to no module. An unqualified name resolves in
every user module and then in the prelude; a name declared in several
user modules is refused unqualified and must be qualified.

```adt
from i in identities select i      -- identities found in module auth
```

## spec/adhoc/names/session-module

The connection or session names a default module, as a search path.
Adds session state the stateless sessions otherwise avoid.

## spec/adhoc/names/qualified

Every schema name in ad hoc text is written `auth::identities`. Safe
and verbose for the one place people type queries by hand.
