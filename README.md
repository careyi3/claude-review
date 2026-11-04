# claude-review

[![CI](https://github.com/careyi3/claude-review/actions/workflows/ci.yml/badge.svg)](https://github.com/careyi3/claude-review/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/careyi3/claude-review)](https://github.com/careyi3/claude-review/releases/latest)
[![crates.io](https://img.shields.io/crates/v/claude-review-cli)](https://crates.io/crates/claude-review-cli)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)

Review your working-tree changes in a browser, leave line-anchored comments, and
hand them back to Claude as a structured blob.

The CLI is Rust. The UI is a React SPA compiled into the binary.

![claude-review](./docs/claude-review.png)

## Install

Homebrew, on macOS or Linux:

```
brew install careyi3/tap/claude-review
```

Or with the shell installer, which drops a prebuilt binary into `~/.cargo/bin`:

```
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/careyi3/claude-review/releases/latest/download/claude-review-cli-installer.sh | sh
```

Or from crates.io, if you have a Rust toolchain. The crate is
`claude-review-cli` because `claude-review` was taken; the command is still
`claude-review`. It ships the built UI, so this needs no Node:

```
cargo install claude-review-cli
```

From a checkout, which needs Node and Yarn as well as Rust:

```
git clone https://github.com/careyi3/claude-review && cd claude-review
yarn --cwd web install
cargo install --path .
```

## Use

```
claude-review install-command      # once, and after upgrading
```

That installs two Claude Code commands into `~/.claude/commands`:
`/review-start` and `/review-feedback`.

A **session** is one worktree to review, with its own drafts and submitted
reviews. Sessions live in `~/.claude-review`, not in your repository, so the app
and Claude can each be started from anywhere.

1. **Create a session**, in whichever way suits:
   - In Claude Code, run `/review-start`. Claude registers the worktree it is
     working in, names the session after the task, and remembers its id.
   - Run `claude-review` inside a repository. It opens that worktree's session,
     creating it if needed.
   - In the app, press **New session** and pick a path. It suggests directories
     Claude Code has recently worked in, and other worktrees of repositories you
     already have sessions for.
2. **Review.** Run `claude-review` from anywhere to open the app. If it is
   already running, it just opens your browser to it. Pick the session, comment,
   and press **Submit review**.
3. **Hand it back.** Run `/review-feedback` in Claude Code. It uses the session
   it registered in that conversation if there is one, else the one for its
   current worktree, else asks you which. Pass an id to choose:
   `/review-feedback 4f2a`.
4. **Go again.** The session stays open. Claude's changes show up live in the
   app; review them and submit another round. Close the session when you are
   done.

By default a session compares against where its branch split from the default
branch (`main (merge-base)`), so work Claude has already committed is still in
the diff. On the default branch itself that is just your uncommitted changes.
Switch a session to `HEAD`, or any revision, from the header.

### Sessions from the command line

```
claude-review register [--path <dir>] [--name <name>] [--base <rev>] [--by claude]
claude-review sessions [--status open|submitted|closed] [--all] [--json]
claude-review show <id>        # print the latest submitted review
claude-review take <id>        # print the waiting review and mark it picked up
claude-review close <id>
```

Ids can be shortened to any unique prefix. `register` reuses the worktree's
open session if it has one. `sessions --json` reports each session's `relation`
to the current directory (`same`, `worktree` or `parent`), which is how
`/review-feedback` finds the right one.

### Upgrading from 0.1

Reviews used to live in each repository's `.claude/review/`. That directory is
no longer used. When you first create a session for a repository, an
unsubmitted draft there is imported, and the old directory is left for you to
delete.

`install-command` replaces the old `/review-feedback` and removes copies it no
longer ships. It only touches files it wrote: a command you have edited is left
alone, with a warning, unless you pass `--force`. Old project-level copies in a
repository's `.claude/commands/` are removed when you register a session for
that repository. The app warns at startup if the installed commands are out of
date.

## Options

```
claude-review [options]

--session <id>     Open this session instead of the current worktree's
--no-session       Open the app without creating a session for this worktree
--base <rev>       Compare against this, when creating the session (default: auto)
--no-untracked     Exclude untracked files, when creating the session
--cwd <path>       Run as if started in <path>
--port <n>         Port to listen on                    (default: random)
--no-open          Print the URL instead of opening it
--summary          Print a human summary and exit
--json             Print the review model as JSON and exit
--context <n>      Context lines, for --summary/--json  (default: 3)
--max-bytes <n>    Skip larger files, for --summary/--json (default: 1000000)
--web-root <path>  Serve the UI from a directory instead of the bundle
```

`--port` and `--web-root` only apply when starting the app, not when attaching
to one that is running. Set `CLAUDE_REVIEW_HOME` to keep sessions somewhere
other than `~/.claude-review`.

## Keyboard

| Key | |
| --- | --- |
| `s` | Switch session |
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
against the working tree, diffed in process. When the base is older than `HEAD`,
such as a merge-base, the file list also comes from `git diff --name-status`
against it, because `git status` cannot see committed work. Lines keep their
trailing newline, so a missing newline at end of file is a real difference.

Each session is a directory under `~/.claude-review/sessions/<id>/`:

```
session.json       name, worktree path, base, status, who created it
draft.json         written on every change
reviews/           latest.md, latest.json, and a timestamped copy of each
```

Submit writes the review, clears the draft and marks the session `submitted`.
`take` marks it `open` again. Sessions are matched to worktrees by their git top
level and to each other by git's common directory, so separate worktrees of one
repository get separate sessions, including worktrees inside the main checkout.

One app serves every session. It records its port, token and pid in
`~/.claude-review/server.json`, and a second `claude-review` checks that file and
opens the browser to the running app instead of starting another. The app
watches the registry, so a session Claude registers from the CLI appears in the
UI straight away.

The server binds `127.0.0.1` on a random port with a per-run token. Every `/api`
route requires it.

`build.rs` runs vite into `OUT_DIR` and `rust-embed` compiles the result
into the binary, so the UI cannot go stale. Set `CLAUDE_REVIEW_SKIP_UI=1` to build
without it. Without `web/node_modules` it falls back to a prebuilt `web/dist`,
which is how the published crate builds without Node.

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

To try a development build alongside an installed copy, give it its own home
so the two keep separate sessions and do not attach to each other:

```
export CLAUDE_REVIEW_HOME=/tmp/claude-review-dev
cargo run -- --cwd ~/code/some-repo
```

UI work:

```
yarn --cwd web dev                 # Vite dev server, proxies /api
cargo run -- --port 7420 --no-open # the API it proxies to
```

## API

| Route | |
| ----- | --- |
| `GET /api/health` | Liveness, and the app's pid |
| `GET /api/sessions` | Every open session; `?all=1` includes closed ones |
| `POST /api/sessions` | Create a session for a path, or reuse its open one |
| `GET /api/suggestions` | Paths a new session could point at |
| `GET /api/events` | Server-sent events: `sessions` on registry changes; with `?session=<id>`, `changed` on working-tree changes |

Under `/api/sessions/:id`:

| Route | |
| ----- | --- |
| `GET`, `PATCH`, `DELETE /` | Read, rename or rebase, or close the session |
| `GET /review` | Diff model plus the re-anchored draft |
| `GET /file` | Whole-file contents for one side |
| `POST /comments` | Add a comment; the server computes its anchor |
| `PATCH /comments/:comment` | Change a comment's tag or body |
| `DELETE /comments/:comment` | Remove a comment |
| `PUT /overall` | Set the overall note |
| `GET /preview` | The markdown submit would write |
| `POST /submit` | Write the review out, clear the draft, mark the session submitted |
| `PUT /viewed` | Tick a file off, or untick it |
