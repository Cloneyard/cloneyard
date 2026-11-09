# cloneyard-rpc

JSON-RPC 2.0 with one message per line (NDJSON), over any byte stream: the daemon's Unix socket now, a plugin's stdio later.

## Scope

- The message types and the standard error codes.
- `Router`: method names mapped to async handlers with typed params and results.
- `serve`: answers requests on a reader and writer pair until the reader ends.
- `Client`: calls methods one at a time over the same kind of pair.

## Out of scope

- Transports: the caller opens the socket, pipe or WebSocket and passes its two halves in.
- Method names and their params and results: the crate that serves a method defines them.
- Batches, which get -32600 until something needs them.

[ADR 0002](../../docs/adr/0002-json-rpc-over-ndjson.md) explains NDJSON and the own dispatcher.
