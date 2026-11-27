# Claude Code contract test

`cloneyard-spike.py` checks that Claude Code still does what Cloneyard relies on: the flags and subcommands it calls, headless sessions in stream-json, hooks through `--plugin-dir` and `--settings`, which a checkout's `disableAllHooks` must not turn off, OpenTelemetry to a local receiver, also with the exporter variables and token of a clone and with the tool call ids of the hooks, `--json-schema`, the OSC 7501 terminal states, background sessions, the first-run screens of a new config directory, Landlock and the seccomp filter of a clone, and the native sandbox outside and inside them. `--list` names every test and what it checks.

Run it after every Claude Code update, and on a new machine before Cloneyard runs there. It isn't part of CI, because it needs a logged-in account.

## Requirements

Linux, Python 3.9 or later and Claude Code; the script uses only the standard library. The sandbox tests also need `bubblewrap` and `socat`.

## An account directory of its own

Give the script its own `CLAUDE_CONFIG_DIR` with `--config-dir`: when it shares `~/.claude` with your open sessions, the OAuth token refresh fails. Log that directory in once:

```bash
mkdir -p ~/.local/share/cloneyard/accounts/spike/claude
CLAUDE_CONFIG_DIR=~/.local/share/cloneyard/accounts/spike/claude claude auth login
```

On a machine without a browser, run `claude setup-token` on another one and export the token as `CLAUDE_CODE_OAUTH_TOKEN` before the run.

## Running it

```bash
tools/spike/cloneyard-spike.py --config-dir ~/.local/share/cloneyard/accounts/spike/claude
```

The default set makes a few small calls to Haiku on your plan; `--model` picks another model.

- `--offline` skips every test that calls the model.
- `--only` and `--skip` take test ids, comma-separated.
- `--with-rc` adds Remote Control, which registers a session in your account.
- `--accounts DIR_A,DIR_B` adds two accounts working in parallel.

`--help` lists the rest.

## The report

Each run writes `report.md`, `report.json` and the raw output of every test to `cloneyard-spike-<timestamp>/` in the current directory, or to the folder `--out` names. The script exits with 1 when a test FAILs; a WARN doesn't fail the run.

The report masks e-mail addresses, tokens and the account's organization and ids, but it keeps the prompts, paths and transcripts of the test sessions, so read it before you paste it anywhere. At the end the script removes what Claude Code stored about its scratch repos; `--keep-cc-data` keeps it.

A FAIL means that Claude Code no longer does what Cloneyard expects, or that the test misreads it: open an issue with that test's section of `report.md` and the Claude Code version.
