use std::fmt;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct GitError {
    pub message: String,
    pub args: Vec<String>,
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for GitError {}

pub type Result<T> = std::result::Result<T, GitError>;

fn run(cwd: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let fail = |message: String| GitError {
        message,
        args: args.iter().map(|a| a.to_string()).collect(),
    };

    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| fail(format!("could not run git: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(fail(if stderr.is_empty() {
            format!("git {} failed", args.join(" "))
        } else {
            stderr
        }));
    }

    Ok(output.stdout)
}

fn run_text(cwd: &Path, args: &[&str]) -> Result<String> {
    run(cwd, args).map(|out| String::from_utf8_lossy(&out).into_owned())
}

pub fn repo_root(cwd: &Path) -> Result<String> {
    Ok(run_text(cwd, &["rev-parse", "--show-toplevel"])?
        .trim()
        .to_string())
}

pub fn resolve_rev(root: &Path, rev: &str) -> Option<String> {
    let spec = format!("{rev}^{{commit}}");
    run_text(root, &["rev-parse", "--verify", &spec])
        .ok()
        .map(|s| s.trim().to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: String,
    pub orig_path: Option<String>,
    pub xy: String,
    pub sub: String,
    pub untracked: bool,
    pub unmerged: bool,
    pub similarity: Option<u32>,
}

fn split_fields(s: &str, n: usize) -> Vec<&str> {
    let mut out = Vec::with_capacity(n + 1);
    let mut start = 0;

    for _ in 0..n {
        let Some(offset) = s[start..].find(' ') else {
            return out;
        };
        let at = start + offset;
        out.push(&s[start..at]);
        start = at + 1;
    }

    out.push(&s[start..]);
    out
}

fn leading_int(s: &str) -> Option<u32> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

pub fn parse_porcelain_v2(raw: &str) -> Vec<StatusEntry> {
    let tokens: Vec<&str> = raw.split('\0').collect();
    let mut out = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        let t = tokens[i];
        if t.is_empty() {
            i += 1;
            continue;
        }

        match t.as_bytes()[0] {
            b'1' => {
                let f = split_fields(t, 8);
                if f.len() >= 9 {
                    out.push(StatusEntry {
                        path: f[8].to_string(),
                        orig_path: None,
                        xy: f[1].to_string(),
                        sub: f[2].to_string(),
                        untracked: false,
                        unmerged: false,
                        similarity: None,
                    });
                }
            }
            b'2' => {
                let f = split_fields(t, 9);
                if f.len() >= 10 {
                    let similarity = f[8].get(1..).and_then(leading_int);
                    let path = f[9].to_string();
                    let (xy, sub) = (f[1].to_string(), f[2].to_string());

                    i += 1;
                    let orig_path = tokens.get(i).map(|s| s.to_string());

                    out.push(StatusEntry {
                        path,
                        orig_path,
                        xy,
                        sub,
                        untracked: false,
                        unmerged: false,
                        similarity,
                    });
                }
            }
            b'u' => {
                let f = split_fields(t, 10);
                if f.len() >= 11 {
                    out.push(StatusEntry {
                        path: f[10].to_string(),
                        orig_path: None,
                        xy: f[1].to_string(),
                        sub: f[2].to_string(),
                        untracked: false,
                        unmerged: true,
                        similarity: None,
                    });
                }
            }
            b'?' => out.push(StatusEntry {
                path: t[2..].to_string(),
                orig_path: None,
                xy: "??".to_string(),
                sub: "N...".to_string(),
                untracked: true,
                unmerged: false,
                similarity: None,
            }),
            _ => {}
        }

        i += 1;
    }

    out
}

pub fn status(root: &Path, include_untracked: bool) -> Result<Vec<StatusEntry>> {
    let untracked = if include_untracked {
        "--untracked-files=all"
    } else {
        "--untracked-files=no"
    };

    let raw = run_text(
        root,
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--find-renames",
            untracked,
        ],
    )?;

    Ok(parse_porcelain_v2(&raw))
}

pub fn read_blob(root: &Path, rev: &str, path: &str) -> Option<Vec<u8>> {
    let spec = format!("{rev}:{path}");
    run(root, &["cat-file", "blob", &spec]).ok()
}

fn absolute_from(root: &Path, raw: &str) -> String {
    let path = Path::new(raw);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };

    std::fs::canonicalize(&joined)
        .unwrap_or(joined)
        .to_string_lossy()
        .into_owned()
}

/// The repository's shared `.git` directory. Every worktree of one repository
/// reports the same value, which is how sessions recognise each other's
/// worktrees.
pub fn common_dir(root: &Path) -> Result<String> {
    let raw = run_text(root, &["rev-parse", "--git-common-dir"])?;
    Ok(absolute_from(root, raw.trim()))
}

pub fn current_branch(root: &Path) -> Option<String> {
    run_text(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The branch work is usually merged into: origin's HEAD if it is set, else a
/// local `main` or `master`. Prefers the local branch when one exists.
pub fn default_branch(root: &Path) -> Option<String> {
    let from_origin = run_text(
        root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .ok()
    .map(|s| s.trim().to_string())
    .and_then(|s| s.strip_prefix("origin/").map(str::to_string));

    let mut candidates: Vec<String> = Vec::new();
    if let Some(name) = from_origin {
        candidates.push(name.clone());
        candidates.push(format!("origin/{name}"));
    }
    for name in ["main", "master", "origin/main", "origin/master"] {
        candidates.push(name.to_string());
    }

    candidates
        .into_iter()
        .find(|name| resolve_rev(root, name).is_some())
}

pub fn merge_base(root: &Path, a: &str, b: &str) -> Option<String> {
    run_text(root, &["merge-base", a, b])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Paths of every worktree attached to the repository at `root`.
pub fn worktrees(root: &Path) -> Vec<String> {
    let Ok(raw) = run_text(root, &["worktree", "list", "--porcelain"]) else {
        return Vec::new();
    };

    raw.lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(str::to_string)
        .collect()
}

/// Parses `git diff --name-status -z` output into status entries.
pub fn parse_name_status(raw: &str) -> Vec<StatusEntry> {
    let tokens: Vec<&str> = raw.split('\0').collect();
    let mut out = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        let code = tokens[i];
        if code.is_empty() {
            i += 1;
            continue;
        }

        let kind = code.as_bytes()[0];
        let entry = |path: &str, orig: Option<&str>, similarity: Option<u32>| StatusEntry {
            path: path.to_string(),
            orig_path: orig.map(str::to_string),
            xy: code.to_string(),
            sub: "N...".to_string(),
            untracked: false,
            unmerged: kind == b'U',
            similarity,
        };

        match kind {
            b'R' | b'C' => {
                let (Some(old), Some(new)) = (tokens.get(i + 1), tokens.get(i + 2)) else {
                    break;
                };
                if kind == b'R' {
                    out.push(entry(new, Some(old), leading_int(&code[1..])));
                } else {
                    out.push(entry(new, None, None));
                }
                i += 3;
            }
            _ => {
                let Some(path) = tokens.get(i + 1) else { break };
                out.push(entry(path, None, None));
                i += 2;
            }
        }
    }

    out
}

/// Every path that differs between `rev` and the working tree, committed or
/// not. Untracked files are not included; `status` has those.
pub fn diff_entries(root: &Path, rev: &str) -> Result<Vec<StatusEntry>> {
    let raw = run_text(
        root,
        &["diff", "--name-status", "-z", "--find-renames", rev, "--"],
    )?;
    Ok(parse_name_status(&raw))
}
