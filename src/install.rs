//! Installs the Claude Code slash commands, and tidies up after older versions.
//!
//! Every file this installs carries [`MARKER`]. A file without it is only
//! touched if it is byte-for-byte a copy some release shipped, so a command the
//! user has edited is never overwritten or deleted.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const MARKER: &str = "<!-- claude-review:managed -->";

pub struct ManagedCommand {
    pub file: &'static str,
    pub source: &'static str,
}

pub const COMMANDS: [ManagedCommand; 2] = [
    ManagedCommand {
        file: "review-feedback.md",
        source: include_str!("../commands/review-feedback.md"),
    },
    ManagedCommand {
        file: "review-start.md",
        source: include_str!("../commands/review-start.md"),
    },
];

/// Exact copies of commands shipped by releases from before [`MARKER`].
pub const LEGACY: [(&str, &str); 1] = [(
    "review-feedback.md",
    include_str!("../commands/legacy/review-feedback-0.1.0.md"),
)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Created,
    Updated,
    AlreadyCurrent,
    /// Ours once, no longer shipped, so deleted.
    Removed,
    /// Has the name of one of ours but has been edited, so left alone.
    Modified,
}

impl Outcome {
    pub fn describe(self) -> &'static str {
        match self {
            Outcome::Created => "Installed",
            Outcome::Updated => "Updated",
            Outcome::AlreadyCurrent => "Already up to date:",
            Outcome::Removed => "Removed",
            Outcome::Modified => "Left alone, because it has been edited:",
        }
    }
}

/// `~/.claude/commands`.
pub fn commands_dir() -> io::Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "could not find your home directory",
        )
    })?;

    Ok(home.join(".claude").join("commands"))
}

/// True if `content` is something this tool wrote: it has the marker, or it is
/// an untouched copy of a legacy release's command.
pub fn is_ours(file: &str, content: &str) -> bool {
    content.contains(MARKER)
        || LEGACY
            .iter()
            .any(|(name, legacy)| *name == file && *legacy == content)
}

fn shipped(file: &str) -> bool {
    COMMANDS.iter().any(|c| c.file == file)
}

/// Installs or updates every command in `dir`, and removes ones this tool
/// installed that it no longer ships. With `force`, edited copies are
/// overwritten too.
pub fn install_commands(dir: &Path, force: bool) -> io::Result<Vec<(PathBuf, Outcome)>> {
    fs::create_dir_all(dir)?;
    let mut results = Vec::new();

    for command in &COMMANDS {
        let destination = dir.join(command.file);

        let outcome = match fs::read_to_string(&destination) {
            Ok(existing) if existing == command.source => Outcome::AlreadyCurrent,
            Ok(existing) if !force && !is_ours(command.file, &existing) => Outcome::Modified,
            Ok(_) => Outcome::Updated,
            Err(_) if destination.exists() => Outcome::Updated,
            Err(_) => Outcome::Created,
        };

        if matches!(outcome, Outcome::Created | Outcome::Updated) {
            crate::fsutil::write_atomic(&destination, command.source)?;
        }

        results.push((destination, outcome));
    }

    for path in retired(dir) {
        fs::remove_file(&path)?;
        results.push((path, Outcome::Removed));
    }

    Ok(results)
}

/// Files in `dir` this tool installed but no longer ships.
fn retired(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut found: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            let Some(file) = path.file_name().and_then(|n| n.to_str()) else {
                return false;
            };
            file.ends_with(".md")
                && !shipped(file)
                && fs::read_to_string(path).is_ok_and(|content| is_ours(file, &content))
        })
        .collect();

    found.sort();
    found
}

/// Commands in `dir` that are missing or out of date, for a startup warning.
/// Edited copies are not reported: the user chose those.
pub fn stale_commands(dir: &Path) -> Vec<&'static str> {
    let mut stale: Vec<&'static str> = COMMANDS
        .iter()
        .filter(|command| match fs::read_to_string(dir.join(command.file)) {
            Ok(existing) => existing != command.source && is_ours(command.file, &existing),
            Err(_) => true,
        })
        .map(|command| command.file)
        .collect();

    if !retired(dir).is_empty() {
        stale.push("(retired commands)");
    }

    stale
}

/// Older versions could install the command into a repository's own
/// `.claude/commands`. Those copies would shadow the user-level one, so remove
/// any that are untouched. Returns a note for each one removed or kept.
pub fn remove_legacy_project_commands(repo_root: &Path) -> Vec<String> {
    let dir = repo_root.join(".claude").join("commands");
    let mut notes = Vec::new();

    for (file, legacy) in LEGACY {
        let path = dir.join(file);
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };

        if content == legacy || content.contains(MARKER) {
            if fs::remove_file(&path).is_ok() {
                notes.push(format!(
                    "Removed an old project copy of /{} at {}; the one in ~/.claude/commands is used instead.",
                    file.trim_end_matches(".md"),
                    path.display()
                ));
            }
        } else {
            notes.push(format!(
                "{} is an edited project copy of an old command and may shadow the current one; delete it if you no longer need it.",
                path.display()
            ));
        }
    }

    notes
}
