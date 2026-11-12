# Coding standards

Review checks every pull request against these rules, whether a person or an agent reviews it. What `cargo fmt`, clippy and CI enforce isn't repeated here, and the commit rules stay in [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Errors

**An error a user can see names the file or thing it is about through `with_context`, as `cannot <verb> <path>` when an operation fails and `invalid <path>` when what it holds is wrong, and a test pins its text.**

- Look for: a `?` on a file, socket or database call with no context, or a new message no test asserts.
- Example: `schema::open` in `cloneyard-store` wraps `connect` in `cannot open <path>`, and `names_the_file_when_it_is_not_a_database` pins that text.

## Resources

**Resources are released in reverse order of acquisition, on the error path too.**

- Look for: a `?` above explicit `drop()` calls, which on an error skips them and drops the values in reverse order of declaration instead.
- Example: `Daemon::serve` in `cloneyard-daemon` keeps the result of removing the socket in `removed` and returns it only after the store and both locks are dropped.

## Tests

**An end-to-end test runs a `cy` subcommand through `Node::cy`, and a test of a fallback changes only the variable it is about.**

- Look for: a test that builds its own `Command` for a `cy` subcommand, or a fallback test that sets or removes a variable its name doesn't mention.
- Example: `keeps_the_database_in_home_when_xdg_data_home_is_relative` in `cloneyard-cli` builds `cy daemon` with `Node::cy` and makes only `XDG_DATA_HOME` relative.

## Comments

**A comment goes only where the code is surprising, in one line; in a test the name is the comment.**

- Look for: a comment that repeats its line, one longer than a line, or one in a test that says what the test checks.
- Example: the `EMFILE` comment in `Daemon::serve` says why a failed accept waits before the next one.

## Adding a rule

A rule goes here only when no tool can check it. Anything a script, clippy or CI can catch becomes a check instead.
