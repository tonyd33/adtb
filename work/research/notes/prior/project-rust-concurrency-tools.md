---
title: loom, shuttle and Miri
---
loom exhaustively permutes executions under the C11 memory model with
state reduction. shuttle schedules threads randomly (PCT) and replays
failing schedules. Miri interprets Rust and reports undefined
behaviour in unsafe code, data races and some weak-memory effects.
https://github.com/tokio-rs/loom
https://github.com/awslabs/shuttle
https://github.com/rust-lang/miri
