# cloneyard-api

The methods of the Cloneyard API as Rust types, and the OpenRPC document built from them. It is `MIT OR Apache-2.0`, so permissive code such as the plugin SDK can depend on it.

## Scope

- `Method`: a method's name, its summary or description, and the types of its params and its result.
- One type per method, named after it, with `<Method>Result` for its result: `system::Ping`, `system::Version` and `rpc::Discover`, whose result is the `Document`.
- `document()`: the OpenRPC 1.4.1 document, in JSON Schema draft-07, with the shared types under `components.schemas`.
- `schema/cloneyard.openrpc.json` at the repo root, which a test keeps equal to `document()`; `UPDATE_SCHEMA=1 cargo test -p cloneyard-api --test schema` rewrites it.

## Out of scope

- The protocol, which lives in `cloneyard-rpc`, and the handlers, which live in the daemon.
- Any dependency on `cloneyard-rpc` or another AGPL crate: `scripts/check-licence-direction` fails CI on a normal or build one.
- Params, errors and events in the document, until a method needs them.

[ADR 0005](../../docs/adr/0005-api-schema-from-rust-types.md) explains why the schema comes from the types, and [ADR 0006](../../docs/adr/0006-permissive-dependency-licences.md) why it never depends on an AGPL crate.
