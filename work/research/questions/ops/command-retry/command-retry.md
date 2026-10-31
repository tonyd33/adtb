---
title: Whether a text is rerun on a serialization failure after a command ran
pick: one
options:
  surface:
    title: A text in which a command ran is not rerun; the failure reaches the client
  rerun:
    title: Rerun it, and the command takes effect again
  refuse:
    title: Refuse a command beside a write
---
wire.md 6.4 reruns a CallText that writes because it has no effect outside the database. catalog.md 9.1 says an abort does not undo a command, so this text has one.

```adt
set setting RetryLimit = { retries: 5 }
update i in items set { i with value: i.value + 1 }
```

## ops/command-retry/surface

A failure before any command ran still reruns. After one ran, the client gets `Serialization` at once and End never reports a second attempt. A client that resends decides for itself, knowing what its text does. No list of which commands count: `explain` beside a write also loses the rerun, which costs little in a rare text, and one rule is easier to state and to hold.

## ops/command-retry/rerun

A retire reruns into an error it caused itself, and a setting's history gains rows nobody asked for. Contradicts the premise wire.md gives for rerunning.

## ops/command-retry/refuse

Holds the rule by construction, and takes away a form catalog.md allows, for the sake of a retry only the failing case needs.
