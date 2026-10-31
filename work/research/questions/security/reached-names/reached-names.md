---
title: What a role learns of a relation it reaches only through a reference it may read
pick: one
options:
  shown:
    title: The relation's name and key form, as the value prints them
  masked:
    title: Nothing, so a reference to an unreadable relation prints without its target
---
`from n in notes select n.about`, where `about: Key secrets` and the role holds no grant on `secrets`. The value prints as `secrets.key(...)`.

## security/reached-names/shown

The field's declared type names the target and the role was granted the field. The key is the value it was granted. Rows of the target stay charged, and naming the target in text still answers Denied.

## security/reached-names/masked

A key without its relation cannot be read or used by a client, and every printer and driver would carry a masking rule for a name the grant on the field has already given out.
