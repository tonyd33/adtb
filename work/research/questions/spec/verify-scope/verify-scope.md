---
title: "Which findings a later verify or scrub replaces"
pick: one
options:
  per-scope:
    title: "A run replaces the findings of the last run over the same scope; scrub is a scope of its own"
  latest:
    title: "A run replaces every finding it could have reproduced"
---
Raised by the milestone 9 audit. `verify users`, then a scrub that finds a bad page in `orders`, then `verify users`: the scrub's finding must survive.

## spec/verify-scope/per-scope

The scopes are the whole database, one relation variable, and the scrub. Each run replaces only the findings of the previous run over its scope, so a narrow run never erases what a wide one found. A finding repeats across scopes until each is rerun.

## spec/verify-scope/latest

Replacing by overlap makes `verify users` erase a whole run's finding about `users` that it could not reproduce, such as a latent one on a page since rewritten. That hides history a user may need.
