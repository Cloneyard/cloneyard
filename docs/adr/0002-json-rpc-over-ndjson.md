# JSON-RPC 2.0 over NDJSON with our own dispatcher

Everything that talks to the daemon speaks JSON-RPC 2.0 with one message per line (NDJSON): local clients over the Unix socket, plugins over stdio and, later, remote clients over WebSocket, one message per text frame. One framing means one set of message types and one dispatcher for every transport. We wrote that dispatcher ourselves in `cloneyard-rpc`, a few hundred lines on top of `serde_json` and `tokio`, instead of taking jsonrpsee. Batches get -32600 until something needs them.

## Considered options

- **jsonrpsee**: its server speaks HTTP and WebSocket, so NDJSON over a socket or stdio would need our own transport on top of a large dependency tree anyway.
- **`Content-Length` headers, as in LSP**: they allow raw newlines inside a message, which serialized JSON never needs, and make the protocol harder to drive by hand.
- **Length-prefixed binary frames**: compact, but nobody can write a message by hand or read one with `socat` and `jq`.
