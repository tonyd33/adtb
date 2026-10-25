# gRPC

kind: system
home: https://grpc.io/

## Flow control

- Flow control uses the transport's acknowledgements of read data; a write hands data to the framework, which pauses the write when the receiver lacks capacity, giving backpressure on streaming RPCs. [V 2026-10-25] https://grpc.io/docs/guides/flow-control/
- Runs over HTTP/2 with per-stream and per-connection windows. [U]
