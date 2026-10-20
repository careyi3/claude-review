# claude-review

Review your working-tree changes in a browser, leave line-anchored comments, and
hand them back to Claude as a structured blob.

The CLI is Rust. The UI is a React SPA compiled into the binary.

![claude-review](./docs/claude-review.png)

## Install

Needs Node and Yarn to build the UI, plus a Rust toolchain.

```
git clone <repo> && cd claude-review
yarn --cwd web install
cargo install --path .
```

## Use

```
claude-review install-command      # once, installs /review-feedback
claude-review                      # in a repo Claude has changed
```

Comment, press **Submit review**, then run `/review-feedback` in Claude Code. It
reads `.claude/review/latest.md` and works through the comments in tag order.

Re-run `install-command` after upgrading to refresh the slash command.

Reviews are written to `.claude/review/` in the repo. To keep them out of
`git status` in every project, add this line to your global gitignore
(`~/.config/git/ignore` by default):

```
**/.claude/review/
```

## Options

```
--base <rev>       Revision to compare against          (default: HEAD)
--no-untracked     Exclude untracked files              (default: included)
--context <n>      Context lines around each hunk       (default: 3)
--max-bytes <n>    Skip diffing files larger than this  (default: 1000000)
--cwd <path>       Run as if started in <path>
--port <n>         Port to listen on                    (default: random)
--no-open          Print the URL instead of opening it
--summary          Print a human summary and exit
--json             Print the review model as JSON and exit
--web-root <path>  Serve the UI from a directory instead of the bundle
```

`install-command` takes `--project` to install into `<repo>/.claude/commands`
instead of `~/.claude/commands`.

## Keyboard

| Key | |
| --- | --- |
| `j` / `k` | Next / previous hunk |
| `n` / `p` | Next / previous file |
| `c` | Comment on the current file |
| `v` | Mark the current file viewed |
| `t` | Toggle unified and split |
| `w` | Wrap or scroll long lines |
| `b` | Show or hide the file list |
| `r` | Reload the diff |
| `⌘↵` | Submit the review |
| `esc` | Close, or clear the selection |
| `?` | The same list, in the app |

## How it works

Comments are pinned by content, not line number: each records hashes of the lines
it covers plus three either side, and every load recomputes where they are now —
`anchored`, `moved`, or `orphaned`. Claude may still be editing while a review is
open, and line numbers alone would quietly mispoint.

The diff comes from `git status --porcelain=v2 -z` plus `git cat-file blob`
against the working tree, diffed in process. Lines keep their trailing newline, so
a missing newline at end of file is a real difference.

Drafts are written to `.claude/review/.draft.json` on every change. Submit writes
`latest.md`, `latest.json` and a timestamped archive of each, then clears the
draft.

The server binds `127.0.0.1` on a random port with a per-run token. Every `/api`
route requires it.

`build.rs` runs vite into `OUT_DIR` and `rust-embed` compiles the result
into the binary, so the UI cannot go stale. Set `CLAUDE_REVIEW_SKIP_UI=1` to build
without it.

The bundle ships only the Shiki grammars `detect_language` can return, listed in
`web/src/languages.ts`, and two themes. `tests/language.rs` and `tests/bundle.rs`
fail if that drifts.

## Develop

```
yarn --cwd web install
cargo test
yarn --cwd web test
yarn --cwd web typecheck
```

UI work:

```
yarn --cwd web dev                 # Vite dev server, proxies /api
cargo run -- --port 7420 --no-open # the API it proxies to
```

## API

| Route | |
| ----- | --- |
| `GET /api/review` | Diff model plus the re-anchored draft |
| `GET /api/file` | Whole-file contents for one side |
| `POST /api/comments` | Add a comment; the server computes its anchor |
| `PATCH /api/comments/:id` | Change a comment's tag or body |
| `DELETE /api/comments/:id` | Remove a comment |
| `PUT /api/overall` | Set the overall note |
| `GET /api/preview` | The markdown submit would write |
| `POST /api/submit` | Write the review out and clear the draft |
| `PUT /api/viewed` | Tick a file off, or untick it |
| `GET /api/events` | Server-sent events when the working tree changes |
