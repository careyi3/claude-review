---
description: Work through the code review left by claude-review
argument-hint: "[session id]"
allowed-tools: Bash(claude-review sessions:*), Bash(claude-review take:*), Bash(claude-review show:*)
---
<!-- claude-review:managed -->

A human has reviewed your changes in `claude-review` and may have submitted
comments. These sessions have a submitted review waiting, as JSON:

!`claude-review sessions --status submitted --json 2>&1 || echo "CLAUDE_REVIEW_UNAVAILABLE"`

The session id the user passed, if any: `$ARGUMENTS`

If the output above says `CLAUDE_REVIEW_UNAVAILABLE` or is an error, stop and
tell the user that `claude-review` could not be run, quoting the error.

**Pick the session.** Each entry has an `id`, `name`, `path`, `branch`,
`submittedAt`, and a `relation` to your current directory: `same` (it is your
worktree), `worktree` (another worktree of the same repository), `parent` (it
is inside your directory), or absent.

1. If the user passed a session id, use it. `take` accepts a unique prefix.
2. Otherwise, if you registered a session earlier in this conversation (with
   `/review-start` or `claude-review register`), use that id. If it is not in
   the list, tell the user no review has been submitted for it yet, and stop.
3. Otherwise, if exactly one entry has `relation` `same`, use it.
4. Otherwise, if the list is empty, tell the user there is no submitted review
   waiting, and that they need to open `claude-review`, review, and press
   Submit first. Stop. Do not guess at what they might have meant.
5. Otherwise, ask the user which one to work on with the AskUserQuestion tool.
   Label each option with the session name, and describe it with its branch,
   path and when it was submitted. Put sessions with a `relation` first. If
   there are more than four, list them all in a message and ask for the id
   instead.

**Fetch the review** with `claude-review take <id>`. It prints the review and
marks it as picked up, so it will not be offered again. If you need to see it a
second time, use `claude-review show <id>`, which changes nothing.

The review's `Repository` line is the worktree its paths are relative to. If it
is not your current directory, work in that directory.

**Handle the comments in tag order.**

- **Must fix** — treat as blocking. Make the change.
- **Questions** — answer them. If answering reveals a real problem, fix it too.
  A question is not automatically a request for a change.
- **Nits** — apply them unless they conflict with something that matters more.
- **Ideas** — evaluate and say whether you agree. Do not implement an idea
  without saying you are about to; if it is a large change, ask first.

**Judgement still applies.** A comment can be wrong, or based on a misreading
of the code. If you think one is, say so plainly with your reasoning rather
than making a change you believe is incorrect. Getting this right matters more
than appearing agreeable.

**Watch for stale anchors.** A comment marked as having moved followed its
lines after an edit; one marked orphaned points at code that no longer exists,
and the quoted snippet is what it originally referred to. For orphaned
comments, work out whether the point still stands before acting, and say what
you concluded. Comments listed under "files no longer in the diff" refer to
work that was reverted or committed — check before assuming they still apply.

**Verify your work.** Run the project's tests and type checks after making the
changes. Report failures honestly rather than describing the work as done. Use
your judgement on whether the entire test suite needs to be run, just a subset,
or none if the change is small enough.

**Then report back with a disposition table** covering every comment, using the
numbers from the review:

| # | Location | Tag | Disposition | Notes |
| - | -------- | --- | ----------- | ----- |
| 1 | src/x.ts:21 | must-fix | fixed | guarded the loop |
| 2 | src/x.ts:5 | question | answered | it is only for the dashboard |
| 3 | src/y.ts | idea | declined | would couple the cache to config |

Use `fixed`, `answered`, `declined`, or `deferred`. Every comment in the review
gets a row, including ones you disagreed with. Follow the table with any
detail that does not fit in it, and end by saying whether tests pass. Mention
that the session stays open, so the user can review your changes in
`claude-review` and submit another round.
