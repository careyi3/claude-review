//! Places a new session could point at, for the UI's "New session" dialog:
//! directories Claude Code has recently worked in, and the worktrees of
//! repositories that already have sessions.

use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;
use serde_json::Value;

use crate::git;
use crate::registry::{Registry, SessionStatus};

const MAX_PROJECTS: usize = 20;
const MAX_LINES: usize = 200;
const MAX_SUGGESTIONS: usize = 30;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub path: String,
    /// `claude` for a recent Claude Code project, `worktree` for a worktree of
    /// a repository that already has a session.
    pub source: &'static str,
    pub branch: Option<String>,
}

/// `$CLAUDE_CONFIG_DIR/projects`, or `~/.claude/projects`.
pub fn claude_projects_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => dirs::home_dir()?.join(".claude"),
    };
    Some(base.join("projects"))
}

fn modified(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn newest(dir: &Path, extension: Option<&str>) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut found: Vec<(SystemTime, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| match extension {
            Some(ext) => p.extension().is_some_and(|e| e == ext),
            None => p.is_dir(),
        })
        .map(|p| (modified(&p), p))
        .collect();

    found.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    found.into_iter().map(|(_, p)| p).collect()
}

/// The working directory a Claude Code transcript was recorded in.
fn transcript_cwd(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;

    BufReader::new(file)
        .lines()
        .take(MAX_LINES)
        .map_while(Result::ok)
        .find_map(|line| {
            let value: Value = serde_json::from_str(&line).ok()?;
            value.get("cwd")?.as_str().map(str::to_string)
        })
}

/// Directories Claude Code has recently been run in, newest first.
pub fn recent_claude_dirs(projects: &Path) -> Vec<String> {
    newest(projects, None)
        .into_iter()
        .take(MAX_PROJECTS)
        .filter_map(|project| {
            let transcript = newest(&project, Some("jsonl")).into_iter().next()?;
            transcript_cwd(&transcript)
        })
        .collect()
}

fn top_level(dir: &Path) -> Option<String> {
    let root = git::repo_root(dir).ok()?;
    Some(
        fs::canonicalize(&root)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(root),
    )
}

pub fn suggestions(registry: &Registry, projects: Option<&Path>) -> Vec<Suggestion> {
    let sessions: Vec<_> = registry
        .list()
        .into_iter()
        .filter(|s| s.status != SessionStatus::Closed)
        .collect();

    let mut seen: HashSet<String> = sessions.iter().map(|s| s.path.clone()).collect();
    let mut out = Vec::new();

    let mut push = |path: String, source: &'static str, out: &mut Vec<Suggestion>| {
        if seen.insert(path.clone()) {
            out.push(Suggestion {
                branch: git::current_branch(Path::new(&path)),
                path,
                source,
            });
        }
    };

    let mut repos: Vec<String> = sessions.iter().map(|s| s.path.clone()).collect();

    if let Some(projects) = projects {
        for dir in recent_claude_dirs(projects) {
            if let Some(root) = top_level(Path::new(&dir)) {
                repos.push(root.clone());
                push(root, "claude", &mut out);
            }
        }
    }

    let mut common_seen = HashSet::new();
    for repo in repos {
        let Ok(common) = git::common_dir(Path::new(&repo)) else {
            continue;
        };
        if !common_seen.insert(common) {
            continue;
        }
        for worktree in git::worktrees(Path::new(&repo)) {
            let path = fs::canonicalize(&worktree)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(worktree);
            if Path::new(&path).is_dir() {
                push(path, "worktree", &mut out);
            }
        }
    }

    out.truncate(MAX_SUGGESTIONS);
    out
}
