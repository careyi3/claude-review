---
description: Register this work as a claude-review session the user can review
argument-hint: "[session name]"
allowed-tools: Bash(claude-review register:*), Bash(git rev-parse:*)
---
<!-- claude-review:managed -->

Register the work you are doing as a session in `claude-review`, so the user
can review your changes in its browser UI and send comments back.

The name the user asked for, if any: `$ARGUMENTS`

1. **Work out the path.** Use the top level of the git worktree you are making
   changes in (`git rev-parse --show-toplevel` from that directory). If you are
   working in a separate worktree, use that worktree, not the main checkout.
   If you are not in a git repository, tell the user `claude-review` needs one,
   and stop.
2. **Work out the name.** Use the name the user gave. Otherwise write a short
   summary of the task, three to six words, such as `fix auth retry backoff`.
3. **Register it:**

   ```
   claude-review register --by claude --path "<path>" --name "<name>"
   ```

   Add `--base <rev>` only if the user asked to compare against something
   specific. By default the session diffs against where the branch split from
   the default branch, so committed work on the branch is included.

4. **Report back.** The command prints the session id, and the app's URL if it
   is running. Tell the user the session id and name, and that they can open
   it by running `claude-review` (or with the printed URL), review, press
   Submit, and then run `/review-feedback` here.

Remember the session id for the rest of this conversation: `/review-feedback`
uses it to find the right review. If the command says it reused an existing
session, that is expected: the worktree already had one, and it is the one to
use.
