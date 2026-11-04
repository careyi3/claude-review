use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use sha1::{Digest, Sha1};

use crate::clock::now_iso;
use crate::diff::{build_diff, count_lines, is_binary};
use crate::git::{self, StatusEntry};
use crate::language::detect_language;
use crate::types::{FileDiff, FileStatus, Hunk, ReviewBase, ReviewModel, SkipReason, Totals};

const EXCLUDED_PREFIX: &str = ".claude/review/";

#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub cwd: PathBuf,
    pub base: String,
    /// How to describe `base` to people, when it is a resolved SHA standing in
    /// for something friendlier such as a merge-base.
    pub base_label: Option<String>,
    pub include_untracked: bool,
    pub context_lines: u32,
    pub max_file_bytes: u64,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            cwd: env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            base: "HEAD".to_string(),
            base_label: None,
            include_untracked: true,
            context_lines: 3,
            max_file_bytes: 1_000_000,
        }
    }
}

struct WorkingContent {
    buf: Option<Vec<u8>>,
    skip: Option<SkipReason>,
}

fn read_working(root: &Path, path: &str) -> WorkingContent {
    let abs = root.join(path);

    let Ok(meta) = fs::symlink_metadata(&abs) else {
        return WorkingContent {
            buf: None,
            skip: None,
        };
    };

    if meta.file_type().is_symlink() {
        let target = fs::read_link(&abs)
            .map(|t| t.to_string_lossy().into_owned().into_bytes())
            .unwrap_or_default();
        return WorkingContent {
            buf: Some(target),
            skip: None,
        };
    }

    if meta.is_dir() {
        return WorkingContent {
            buf: None,
            skip: Some(SkipReason::Submodule),
        };
    }

    if !meta.is_file() {
        return WorkingContent {
            buf: None,
            skip: Some(SkipReason::Binary),
        };
    }

    match fs::read(&abs) {
        Ok(buf) => WorkingContent {
            buf: Some(buf),
            skip: None,
        },
        Err(_) => WorkingContent {
            buf: None,
            skip: None,
        },
    }
}

fn digest_of(path: &str, status: FileStatus, hunks: &[Hunk]) -> String {
    let mut hash = Sha1::new();
    hash.update(format!("{path}\0{}", status.as_str()).as_bytes());

    for hunk in hunks {
        hash.update(format!("\0@{},{}", hunk.old_start, hunk.new_start).as_bytes());
        for line in &hunk.lines {
            hash.update(b"\0");
            hash.update(line.line_type.as_str().as_bytes());
            hash.update(line.content.as_bytes());
        }
    }

    hex::encode(hash.finalize())[..16].to_string()
}

fn classify(entry: &StatusEntry, old_exists: bool, new_exists: bool) -> FileStatus {
    if entry.unmerged {
        return FileStatus::Unmerged;
    }
    if entry.untracked {
        return FileStatus::Untracked;
    }
    if entry
        .orig_path
        .as_ref()
        .is_some_and(|orig| *orig != entry.path)
    {
        return FileStatus::Renamed;
    }
    match (old_exists, new_exists) {
        (false, true) => FileStatus::Added,
        (true, false) => FileStatus::Deleted,
        _ => FileStatus::Modified,
    }
}

fn build_file(
    root: &Path,
    rev: Option<&str>,
    entry: &StatusEntry,
    opts: &BuildOptions,
) -> FileDiff {
    let base_path = entry.orig_path.as_deref().unwrap_or(&entry.path);
    let is_submodule = entry.sub.starts_with('S');

    let old_buf = match rev {
        Some(rev) if !entry.untracked && !is_submodule => git::read_blob(root, rev, base_path),
        _ => None,
    };

    let working = if is_submodule {
        WorkingContent {
            buf: None,
            skip: Some(SkipReason::Submodule),
        }
    } else {
        read_working(root, &entry.path)
    };

    let status = classify(entry, old_buf.is_some(), working.buf.is_some());

    let mut skipped = working.skip;
    if skipped.is_none() && entry.unmerged {
        skipped = Some(SkipReason::Unmerged);
    }

    let old_size = old_buf.as_ref().map_or(0, |b| b.len() as u64);
    let new_size = working.buf.as_ref().map_or(0, |b| b.len() as u64);
    let binary =
        old_buf.as_deref().is_some_and(is_binary) || working.buf.as_deref().is_some_and(is_binary);

    if skipped.is_none() && binary {
        skipped = Some(SkipReason::Binary);
    }
    if skipped.is_none() && (old_size > opts.max_file_bytes || new_size > opts.max_file_bytes) {
        skipped = Some(SkipReason::TooLarge);
    }

    let old_path = entry
        .orig_path
        .as_ref()
        .filter(|orig| **orig != entry.path)
        .cloned();

    let mut file = FileDiff {
        path: entry.path.clone(),
        old_path,
        status,
        similarity: entry.similarity,
        language: detect_language(&entry.path).map(str::to_string),
        binary,
        skipped,
        additions: 0,
        deletions: 0,
        old_line_count: 0,
        new_line_count: 0,
        old_size,
        new_size,
        digest: digest_of(&entry.path, status, &[]),
        hunks: Vec::new(),
    };

    if skipped.is_some() {
        return file;
    }

    let old_text = old_buf
        .as_deref()
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default();
    let new_text = working
        .buf
        .as_deref()
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default();

    let built = build_diff(&old_text, &new_text, opts.context_lines);

    file.additions = built.additions;
    file.deletions = built.deletions;
    file.old_line_count = count_lines(&old_text);
    file.new_line_count = count_lines(&new_text);
    file.digest = digest_of(&entry.path, status, &built.hunks);
    file.hunks = built.hunks;

    file
}

/// What differs from `base`. Against HEAD that is just `git status`. Against an
/// older commit, such as a merge-base, it also includes work committed since,
/// which `git status` cannot see.
fn changed_entries(
    root: &Path,
    base: Option<&str>,
    include_untracked: bool,
) -> git::Result<Vec<StatusEntry>> {
    let status = git::status(root, include_untracked)?;

    let head = git::resolve_rev(root, "HEAD");
    let Some(base) = base.filter(|sha| Some(*sha) != head.as_deref()) else {
        return Ok(status);
    };

    let unmerged: std::collections::HashSet<&str> = status
        .iter()
        .filter(|e| e.unmerged)
        .map(|e| e.path.as_str())
        .collect();

    let mut entries: Vec<StatusEntry> = git::diff_entries(root, base)?
        .into_iter()
        .map(|mut entry| {
            entry.unmerged |= unmerged.contains(entry.path.as_str());
            entry
        })
        .collect();

    entries.extend(status.iter().filter(|e| e.untracked).cloned());
    Ok(entries)
}

pub fn build_review(opts: &BuildOptions) -> git::Result<ReviewModel> {
    let root_string = git::repo_root(&opts.cwd)?;
    let root = Path::new(&root_string);

    let sha = git::resolve_rev(root, &opts.base);
    let entries = changed_entries(root, sha.as_deref(), opts.include_untracked)?;

    let mut files: Vec<FileDiff> = entries
        .iter()
        .filter(|entry| !entry.path.starts_with(EXCLUDED_PREFIX))
        .map(|entry| build_file(root, sha.as_deref(), entry, opts))
        .collect();

    files.sort_by(|a, b| {
        a.path
            .to_lowercase()
            .cmp(&b.path.to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });

    let totals = Totals {
        files: files.len() as u32,
        additions: files.iter().map(|f| f.additions).sum(),
        deletions: files.iter().map(|f| f.deletions).sum(),
        skipped: files.iter().filter(|f| f.skipped.is_some()).count() as u32,
    };

    Ok(ReviewModel {
        version: 1,
        repo_root: root_string,
        base: ReviewBase {
            rev: opts.base_label.clone().unwrap_or_else(|| opts.base.clone()),
            sha,
        },
        generated_at: now_iso(),
        include_untracked: opts.include_untracked,
        context_lines: opts.context_lines,
        files,
        totals,
    })
}
