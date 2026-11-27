# The engine crate

- An error keeps its class across every layer: damage stays damage naming its page, a refusal
  stays a refusal naming what it refused, a cancellation and a limit stay theirs. A layer's
  `Broken` holds only its own causes or the lower layer's `Broken`, and a `Rot` only `Rot`; lift
  every other lower error through the layer's exhaustive `From`. Only a fact the engine built in
  memory and found false is `Broken`. Never write a wildcard arm that builds a `Broken`; the
  ratchet row `wildcard_broken` of `lint/rules.toml` counts them and only falls.
- Reach the engine only through `Gated<Engine>::enter`, which marks the thread as running engine
  code: a panic while marked ends the process before any latch, transaction or hold is released.
  Never hold an `Arc<Engine>` or `Weak<Engine>`; the lint rows `engine_handles` and
  `gated_detach` refuse them. Run code the embedder supplied (a host function body, an observer
  `Waker`) under `adtb_sys::panics::outside()`, so its panic stays the embedder's.
