---
title: "How a long-lived old client avoids blocking enable"
pick: one
options:
  hide:
    title: "A bundle may declare hide new constructors: rows of unknown constructors are invisible to it, and following a reference to one faults NoSuchRow"
  block:
    title: "Old readers always block enable until retired or expired"
---
Raised by the second review of the spec in docs/spec/.

## spec/stragglers/hide

A bundle may declare hide new constructors: rows of unknown constructors are invisible to it, and following a reference to one faults NoSuchRow. One old phone calling forever would hold back a release, and retiring breaks every old client.

## spec/stragglers/block

Old readers always block enable until retired or expired. No exit short of retirement.
