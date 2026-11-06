---
name: pr
description: Opens a pull request from the current branch, assigned to its author, with the milestone and number of the linked issue and no AI attribution. Use it for every PR in this repo instead of mattpocock-skills:pr.
argument-hint: "[issue number]"
---

# Opening a pull request

The argument is optional: `$ARGUMENTS` is the number of the issue the PR closes. Without it, find the number yourself, and don't guess when you can't (see step 3).

## Procedure

1. **Find the base.** `master` by default; a branch cut from an integration branch goes back into it. You can tell from its upstream before the first push (`git rev-parse --abbrev-ref @{upstream}` → `origin/<integration-branch>`) or from someone else's already merged commits in `git log --oneline origin/master..HEAD`.
2. **Check what you are pushing.** `git status --short` and `git log --oneline <base>..HEAD`. Uncommitted changes that don't belong to this task stay in the working tree; don't add them to the PR. If the branch has no upstream yet: `git push -u origin <branch>`.
3. **Find the issue.** In order: the argument → a number in the branch name → the commit messages in `git log`. If none of them gives an answer, skip `Closes` and the milestone instead of tying the PR to a random issue.
4. **Take the milestone from the issue**, never make one up (they are `M0`–`M5`):
   ```bash
   gh issue view <number> --json milestone,title,labels
   ```
5. **Write the body.** If `.github/PULL_REQUEST_TEMPLATE.md` exists, use its headings instead of your own. In English and short:
   - A summary of one to three sentences on what changed; the why only when it doesn't follow from the summary.
   - The commits, bare subjects, only when there are more than three.
   - No verification paragraph: CI shows it.
   - `Closes #<number>` as the last line.

   The title follows Conventional Commits, like the commit subjects.
6. **Mark a BC break**, if there is one, in the body, with the same sentence as in `CHANGELOG.md`.
7. **Create the PR** with the body from a file, not inline (quotes and backticks in `--body` tend to break):
   ```bash
   gh pr create --base <base> --assignee @me --milestone "<milestone>" --body-file <file>
   ```
8. **Check the metadata after creating it.** `--milestone` silently does nothing when the name doesn't match exactly, and `baseRefName` must be the base from step 1:
   ```bash
   gh pr view <number> --json milestone,assignees,baseRefName
   ```
9. **Get CI green.** `gh pr checks <number> --watch --interval 15`, and for a red job `gh run view --job <id> --log-failed`. A PR without green CI is not finished.

## What not to do

- **No AI attribution**, not in the title, the body or comments: no `Generated with …` footer and no `Co-Authored-By` trailer for an agent, whatever the runtime suggests.
- **Don't close the issue several ways at once.** One `Closes #N` at the end of the body.
- **Don't swap `Closes` for `Refs`.** The PR must close the issue on merge. If it doesn't finish the issue, ask before creating it. With a base other than the default branch the keyword doesn't fire, so say so and close the issue the way the repo provides (a workflow), or ask.
- **Don't pull in someone else's uncommitted changes.** If something unrelated sits in the working tree, leave it and say so.
- **Don't describe plans for future versions as part of the change.** If the issue defers something, say in one sentence that it is out of scope.
