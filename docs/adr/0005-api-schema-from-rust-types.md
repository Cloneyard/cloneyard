# The API schema comes from its Rust types

Every method of the API is a type in `cloneyard-api` that names the method and the types of its params and its result, and the OpenRPC 1.4.1 document is built from those types with schemars, in JSON Schema draft-07, the dialect of the OpenRPC meta-schema. The daemon registers its handlers and clients make their calls through the same types, so neither can drift from the document. The document is committed as `schema/cloneyard.openrpc.json`, a test in the CI `test` job fails when it differs from the generated one, and the daemon serves the same document from `rpc.discover`. `cloneyard-api` is MIT OR Apache-2.0 and never depends on an AGPL crate, because the permissive plugin SDK will depend on it.

## Considered options

- **A hand-written schema with Rust generated from it**: the schema would lead, but every type would pass through a code generator in the build, while the daemon and the plugin SDK are Rust anyway.
- **Types and a schema written side by side**: only review would keep them in step.
- **JSON Schema 2020-12, the schemars default**: OpenRPC tools read draft-07, so keywords such as `$defs` and `prefixItems` would mean nothing to them.
- **A `Ulid` newtype that implements `JsonSchema`**: every use of an id would wrap and unwrap it, while `schema_with` on the field keeps `ulid::Ulid` in the types.
- **`cy rpc schema` printing a copy built into the CLI**: it would show the CLI's version of the API, not the one the running daemon serves.

## Consequences

- A change to a method's types comes with the regenerated schema file, so the API change shows in review.
- The protocol types, such as the JSON-RPC error object, stay in `cloneyard-rpc`, because `cloneyard-api` can't depend on it.
- Methods take no params yet; the first method with params adds them to the document as by-name content descriptors.
