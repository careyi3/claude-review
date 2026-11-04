---
description: Work through the code review left by claude-review
---

A human has reviewed your changes using `claude-review` and left comments.
Here is their review:

!`cat .claude/review/latest.md 2>/dev/null || echo "NO_REVIEW_FOUND"`

If the review above says `NO_REVIEW_FOUND`, stop and tell the user there is no
submitted review at `.claude/review/latest.md`, and that they need to run
`claude-review` and press Submit first. Do not guess at what they might have
meant.

Otherwise, work through it as follows.

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
changes. Report failures honestly rather than describing the work as done. Use your judgement on if the entire test suite needs to be run or just a subset or even none if the change is small enough.

**Then report back with a disposition table** covering every comment, using the
numbers from the review:

| # | Location | Tag | Disposition | Notes |
| - | -------- | --- | ----------- | ----- |
| 1 | src/x.ts:21 | must-fix | fixed | guarded the loop |
| 2 | src/x.ts:5 | question | answered | it is only for the dashboard |
| 3 | src/y.ts | idea | declined | would couple the cache to config |

Use `fixed`, `answered`, `declined`, or `deferred`. Every comment in the review
gets a row, including ones you disagreed with. Follow the table with any
detail that does not fit in it, and end by saying whether tests pass.
