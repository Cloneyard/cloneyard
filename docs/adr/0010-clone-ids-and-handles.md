# A clone is keyed by a ULID and named by a handle from its node

A clone's key is a ULID, like every entity's, and sync and the hub only ever go by it. For typing in the CLI it also has a handle, `CT-` and a number of at least four digits such as `CT-0042`, which its node gives out from a counter in `meta`, in the transaction that adds the clone. Two nodes can give out the same handle, so a handle names a clone only on its node, and the hub shows it with the node's name. The counter, not the largest handle plus one, gives the next number, so no handle is given out twice once clones can be removed. A handle reads case-insensitively and with or without leading zeros, so `ct-42` is `CT-0042`, and the API always gives the printed form.

## Considered options

- **The ULID alone**: 26 characters nobody types.
- **The handle as the key**: the clones of two nodes would collide in sync.
- **A handle unique across nodes, with the node in it**: longer to type for a clash that only the hub sees, and the hub can add the node's name itself.
- **The largest handle plus one**: removing the newest clone would give its handle to the next one, while logs and messages still mean the old one.
- **A short random code, such as `ct-7fq2`**: not in the order clones started, and harder to say.

## Consequences

- A handle in a log or message from another node can mean a different clone here, so anything that crosses nodes carries the id.
- `clone.show` and `clone.stop` take a handle, which is enough while each node has an API of its own.
