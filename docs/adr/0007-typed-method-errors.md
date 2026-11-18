# Methods fail with typed errors that have codes of their own

The errors a method has of its own are an enum in `cloneyard-api`, its `Method::Error`, and `NoError` for a method without any. Each variant has a code from 1000 up, with a block of 100 per namespace in the order namespaces get their first error (`project.*` has 1000 to 1099), a message that is the same for every error with that code, and its fields as the `data` of the JSON-RPC error object. The router in `cloneyard-rpc` sends a handler's own error that way and anything else it fails with as it is, usually -32603, and `Client::request::<M>` gives the error back as `ClientError::Method(M::Error)`, so a client tells errors apart by their codes, not their text. The OpenRPC document lists the errors of each method with their codes and messages, and puts the schema of the fields in `data`, since OpenRPC has no other place for it. The JSON-RPC error object stays in `cloneyard-rpc`, as [ADR 0005](0005-api-schema-from-rust-types.md) has it.

## Considered options

- **Messages that clients match on**: the text would be part of the API, and a reworded message would break clients.
- **Codes from -32000 to -32099, which JSON-RPC sets aside for server errors**: 100 codes for the whole API, in a range meant for failures of the implementation.
- **One code per namespace with the kind of error in `data`**: a client would read `data` to tell errors apart, and the document couldn't list them.
- **Handlers that return the untyped `cloneyard_rpc::Error`**: a handler could send the code of another method's error, which its client would decode as one of its own.
- **A message built from the fields, such as `project app already exists`**: the message on the wire would differ from the one the document gives for its code, so a client builds that text from the variant instead, as `cy` does.

## Consequences

- A code stands for one error for good: a variant that goes away retires its code.
- The fields of a variant are part of the API, because they are its `data`.
