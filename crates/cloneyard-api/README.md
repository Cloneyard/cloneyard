# cloneyard-api

The methods of the Cloneyard API as Rust types, and the OpenRPC document built from them. It is `MIT OR Apache-2.0`, so permissive code such as the plugin SDK can depend on it.

## Scope

- `Method`: a method's name, its summary or description, and the types of its params, its result and its errors.
- One type per method, named after it, with `<Method>Params`, `<Method>Result` and `<Method>Error` for the types that are its own: `system::Ping`, `system::Version`, `project::Add`, `project::List`, `account::Add`, `account::List`, `clone::Run`, `clone::List`, `clone::Show`, `clone::Stop`, `clone::Events`, `usage::Tools` and `rpc::Discover`, whose result is the `Document`. `clone::Show` and `clone::Stop` share `HandleParams`.
- `clone::Handle`, a clone's handle such as `CT-0042`, which reads `ct-42` too.
- `MethodError`: the errors a method has of its own, each with a code from 1000 up and its fields as data, and `NoError` for a method without any.
- `document()`: the OpenRPC 1.4.1 document, in JSON Schema draft-07, with params as by-name content descriptors, the errors of each method, and the shared types under `components.schemas`.
- `schema/cloneyard.openrpc.json` at the repo root, which a test keeps equal to `document()`; `UPDATE_SCHEMA=1 cargo test -p cloneyard-api --test schema` rewrites it.

## Out of scope

- The protocol, which lives in `cloneyard-rpc`, and the handlers, which live in the daemon.
- Any dependency on `cloneyard-rpc` or another AGPL crate: `scripts/check-licence-direction` fails CI on a normal or build one.
- Events in the document, until a method needs them.

[ADR 0005](../../docs/adr/0005-api-schema-from-rust-types.md) explains why the schema comes from the types, [ADR 0006](../../docs/adr/0006-permissive-dependency-licences.md) why it never depends on an AGPL crate, and [ADR 0007](../../docs/adr/0007-typed-method-errors.md) how methods fail with errors of their own.
