---
title: "When full-page images are logged"
pick: one
options:
  clean-to-dirty:
    title: "On each clean-to-dirty transition, skipped when page writes are atomic"
  per-checkpoint:
    title: "First change after each checkpoint"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/fpi/clean-to-dirty

On each clean-to-dirty transition, skipped when page writes are atomic. Fuzzy checkpoints start redo before the checkpoint, so per-checkpoint images can miss a torn page.

## spec/fpi/per-checkpoint

First change after each checkpoint. A torn page can lack an image.
