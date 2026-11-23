# Each clone works in a `git clone --shared` of the repo

A clone works in a checkout of its own, `clones/<id>/checkout` in the data dir: a `git clone --shared` of the repo on a branch of its own, such as `cloneyard/ct-0042`, from the repo's `HEAD` as committed. The checkout has its own `.git`, which reads the repo's objects through `objects/info/alternates` and never writes to the repo. Cloneyard runs git there only to create the checkout, before the agent starts; after that neither Cloneyard nor you run git or an IDE in a clone's directory outside its sandbox, since its `.git`, with its config and hooks, is the agent's. A clone's branch will reach the repo through a fetch that Cloneyard runs from the repo, with `upload-pack` inside the clone's sandbox.

## Considered options

- **A `git worktree` of the repo**: a worktree writes into the repo's own `.git`, its objects and refs, so the sandbox would have to let the agent write there, and the agent could corrupt objects or move other branches.
- **Your own checkout**: Landlock grants whole trees, so the agent could read your `.env` and other untracked files, and its changes would mix with yours.
- **`git clone --local`, which hardlinks the objects**: a hardlinked object is the same file in both, so an agent that makes it writable changes the repo's object too.
- **A full copy, or `--dissociate`**: every object copied for each clone, slow and big for a large repo.
- **Git config that keeps pushes out of the repo, such as a push URL that fails**: the agent owns the checkout's config, so only the sandbox can stop a push.

## Consequences

- Uncommitted changes in your working tree don't reach the clone.
- `git gc` in the repo can prune objects a checkout still needs once the branch it started from is rewritten or deleted and they are older than `gc.pruneExpire`; git in the checkout then fails on the missing objects.
- `origin` in the checkout is the repo, so the agent can fetch from it. The sandbox stops a push into it; until there is one, Claude Code's default permission mode does, as it denies the command.
- Each clone keeps its directory, checkout included, until removing a clone exists.
