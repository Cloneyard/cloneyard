# An account logs in through the CLI, in a config directory of its own

`cy account add <name>` logs in to Claude Code in your terminal, in a new directory, `accounts/<name>/claude` in the data dir, and the daemon only records the account with that directory: `account.add` takes the directory as `project.add` takes a path, and never runs `claude`. The login is `claude auth login` with `CLAUDE_CONFIG_DIR` set to the directory and without the `ANTHROPIC_*` and `CLAUDE*` variables, because an API key or a token there counts as a login that isn't in the directory. It counts only when `claude auth status --json` in the directory says `loggedIn`. `claude auth login` doesn't show Claude Code's first-run screens, the theme picker and the welcome screen, so the CLI sets `hasCompletedOnboarding: true` in the directory's `.claude.json`, which makes Claude Code skip them; the contract test's `first_run` checks that it still does. Cloneyard never uses `~/.claude`, since a config directory shared with your own sessions broke the OAuth token refresh in the contract test.

## Considered options

- **The login in the daemon**: the daemon has no terminal, and the browser flow may ask you to paste a code into one; a PTY streamed to the CLI waits for `cy attach`.
- **Interactive `claude` after the login, to show the first-run screens**: after the theme you would land in a session you have to leave, past a trust dialog for the current directory that the account would keep.
- **Answering the first-run screens in a PTY, as the contract test does**: it breaks whenever Claude Code rewords a screen, and the theme would be the default anyway.
- **`~/.claude`, or another directory you already have**: one shared with your own sessions breaks the OAuth token refresh.
- **The daemon deriving the directory from the name**: the CLI and the daemon could see different data dirs, and a login without a browser through `claude setup-token` would need a method of its own.

## Consequences

- `hasCompletedOnboarding` isn't documented: when Claude Code drops it, `first_run` fails, and an interactive clone would stop at the theme picker.
- The theme stays at Claude Code's default until `/theme` changes it for the account.
- A directory left in `accounts/` is never reused: `cy account add` with its name fails until you remove it.
