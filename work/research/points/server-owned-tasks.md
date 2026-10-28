---
extends: current
proposed: true
accept:
  net/task-boundary/owned: {title: An engine-owned resumable task and result handle independent of the socket runtime, reason: 'Engine ownership keeps authority, snapshot lifetime, cancellation and retries under one runtime-independent contract. Resumable waits and bounded production are required to preserve shared-worker liveness and actual credit suspension; a blocking wrapper alone does not provide those guarantees.'}
  net/task-resumption-cost/implement: {title: Add resumable engine waits and bounded result production before claiming server credit support, reason: 'Engine ownership keeps authority, snapshot lifetime, cancellation and retries under one runtime-independent contract. Resumable waits and bounded production are required to preserve shared-worker liveness and actual credit suspension; a blocking wrapper alone does not provide those guarantees.'}
---
