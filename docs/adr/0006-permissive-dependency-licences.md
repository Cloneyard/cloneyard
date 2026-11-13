# Dependencies under permissive licences

Third-party dependencies are under Apache-2.0, MIT, Unicode-3.0 or Zlib, and a crate of the workspace that isn't AGPL never depends on one that is. [ADR 0001](0001-agpl-only-with-cla.md) keeps two things possible that copyleft dependencies would rule out: the permissive `cloneyard-api` and plugin SDK, and a commercial licence of Cloneyard. In the CI `deps` job, `cargo deny` checks the licence of every crate in the graph, with all features and on every target, and `scripts/check-licence-direction` fails when a crate outside AGPL reaches an AGPL crate of the workspace through normal or build dependencies. Dev-dependencies don't count there, because they never reach the users of a crate.

## Considered options

- **Every licence compatible with AGPL-3.0, such as GPL-3.0, LGPL-3.0 or MPL-2.0**: enough for the AGPL binary, but their copyleft would follow the code into a commercial licence and into the permissive crates.
- **cargo-deny `[bans]` with `wrappers` for the direction**: each AGPL crate would be listed by hand with the crates allowed to depend on it directly, so a new AGPL crate would go unchecked until someone listed it, while the script reads the licences from the manifests.

## Consequences

- A new AGPL crate goes into `exceptions` in `deny.toml`, or `cargo deny` fails on its licence.
- The allow list holds only the licences in the graph today: a dependency under another permissive licence, such as BSD-3-Clause or ISC, adds it in the same pull request, and a copyleft one needs a new ADR.
