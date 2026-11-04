//! The session registry: one directory per review target under
//! `~/.claude-review/sessions/`, shared by the app, the CLI and Claude.
//!
//! Everything here is plain files, so the CLI can register or pick up a
//! review whether or not the app is running.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::clock::now_iso;
use crate::drafts::{
    legacy_review_dir, load_draft, load_draft_file, save_draft, LEGACY_DRAFT_FILE,
};
use crate::export::{LATEST_MARKDOWN, REVIEWS_DIR};
use crate::fsutil::write_atomic;
use crate::git;
use crate::review::BuildOptions;

/// Overrides `~/.claude-review`. Tests and local development builds use it to
/// keep their sessions apart from an installed copy's.
pub const HOME_ENV: &str = "CLAUDE_REVIEW_HOME";

/// The base spec meaning "where this branch split from the default branch".
pub const AUTO_BASE: &str = "auto";

const SESSION_FILE: &str = "session.json";
const SERVER_FILE: &str = "server.json";

#[derive(Debug)]
pub enum Error {
    NotFound(String),
    Ambiguous(String, Vec<String>),
    Invalid(String),
    Io(io::Error),
    Git(git::GitError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound(id) => write!(f, "no session matches `{id}`"),
            Error::Ambiguous(id, matches) => write!(
                f,
                "`{id}` matches more than one session: {}",
                matches.join(", ")
            ),
            Error::Invalid(message) => write!(f, "{message}"),
            Error::Io(err) => write!(f, "{err}"),
            Error::Git(err) => write!(f, "git: {err}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::Io(err)
    }
}

impl From<git::GitError> for Error {
    fn from(err: git::GitError) -> Self {
        Error::Git(err)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Creator {
    User,
    Claude,
}

impl Creator {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Creator::User),
            "claude" => Some(Creator::Claude),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    /// Being reviewed, or waiting for the next round.
    Open,
    /// A review has been submitted and Claude has not picked it up yet.
    Submitted,
    /// Finished with. Hidden from lists unless asked for.
    Closed,
}

impl SessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionStatus::Open => "open",
            SessionStatus::Submitted => "submitted",
            SessionStatus::Closed => "closed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(SessionStatus::Open),
            "submitted" => Some(SessionStatus::Submitted),
            "closed" => Some(SessionStatus::Closed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub name: String,
    /// The worktree's top level.
    pub path: String,
    /// Shared by every worktree of one repository.
    pub common_dir: String,
    /// `auto`, or any revision git understands.
    pub base: String,
    pub include_untracked: bool,
    pub created_by: Creator,
    pub status: SessionStatus,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub submitted_at: Option<String>,
    #[serde(default)]
    pub taken_at: Option<String>,
    /// How many reviews have been submitted.
    #[serde(default)]
    pub rounds: u32,
}

impl Session {
    pub fn build_options(&self) -> BuildOptions {
        let root = Path::new(&self.path);
        let (base, label) = resolve_base(root, &self.base);

        BuildOptions {
            cwd: root.to_path_buf(),
            base,
            base_label: label,
            include_untracked: self.include_untracked,
            ..BuildOptions::default()
        }
    }
}

/// Turns a base spec into a revision to diff against, plus a label when that
/// revision is a bare SHA.
pub fn resolve_base(root: &Path, spec: &str) -> (String, Option<String>) {
    if spec != AUTO_BASE {
        return (spec.to_string(), None);
    }

    let head = git::resolve_rev(root, "HEAD");

    let found = git::default_branch(root)
        .and_then(|branch| git::merge_base(root, "HEAD", &branch).map(|sha| (sha, branch)));

    match found {
        // On the default branch itself, or nothing has been committed on top
        // of it: only uncommitted work differs, so say HEAD.
        Some((sha, _)) if Some(&sha) == head.as_ref() => ("HEAD".to_string(), None),
        Some((sha, branch)) => (sha, Some(format!("{branch} (merge-base)"))),
        None => ("HEAD".to_string(), None),
    }
}

/// How a session's path relates to some other directory, usually Claude's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Relation {
    /// The directory is the session's worktree, or inside it.
    Same,
    /// Another worktree of the same repository.
    Worktree,
    /// The session's worktree is inside the directory.
    Parent,
}

/// What is known about the directory a session is being compared with.
#[derive(Debug, Clone, Default)]
pub struct Place {
    pub dir: PathBuf,
    /// The git top level containing `dir`, if it is in a repository.
    pub root: Option<String>,
    pub common_dir: Option<String>,
}

impl Place {
    pub fn of(dir: &Path) -> Self {
        let dir = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        let root = git::repo_root(&dir).ok().map(|r| {
            fs::canonicalize(&r)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(r)
        });
        let common_dir = root.as_ref().and_then(|_| git::common_dir(&dir).ok());

        Self {
            dir,
            root,
            common_dir,
        }
    }
}

/// Compares top levels rather than path prefixes: a worktree can live inside
/// the main checkout (Claude Code puts them in `.claude/worktrees/`), and that
/// must not count as the main checkout's session.
pub fn relation_to(session: &Session, place: &Place) -> Option<Relation> {
    if place.root.as_deref() == Some(session.path.as_str()) {
        return Some(Relation::Same);
    }
    if place.common_dir.as_deref() == Some(session.common_dir.as_str()) {
        return Some(Relation::Worktree);
    }
    if Path::new(&session.path).starts_with(&place.dir) {
        return Some(Relation::Parent);
    }
    None
}

/// A session plus what is cheap to work out about it now.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    #[serde(flatten)]
    pub session: Session,
    pub branch: Option<String>,
    /// The worktree has gone, usually because it was removed.
    pub missing: bool,
    pub draft_comments: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<Relation>,
}

#[derive(Debug, Clone, Default)]
pub struct NewSession {
    pub path: PathBuf,
    pub name: Option<String>,
    pub base: Option<String>,
    pub include_untracked: Option<bool>,
    pub created_by: Option<Creator>,
}

#[derive(Debug, Clone)]
pub struct Registered {
    pub session: Session,
    /// An open session for this worktree already existed and was reused.
    pub reused: bool,
    /// Things worth telling the user, such as an imported draft.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SessionPatch {
    pub name: Option<String>,
    pub base: Option<String>,
    pub include_untracked: Option<bool>,
    pub status: Option<SessionStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub pid: u32,
    pub port: u16,
    pub token: String,
    pub url: String,
    pub started_at: String,
}

#[derive(Debug, Clone)]
pub struct Registry {
    root: PathBuf,
}

fn new_id() -> String {
    Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn default_name(root: &Path) -> String {
    let dir = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());

    match git::current_branch(root) {
        Some(branch) => format!("{dir} · {branch}"),
        None => dir,
    }
}

fn clean_name(name: Option<String>) -> Option<String> {
    name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty())
}

fn clean_base(base: Option<String>) -> Option<String> {
    base.map(|b| b.trim().to_string()).filter(|b| !b.is_empty())
}

impl Registry {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `$CLAUDE_REVIEW_HOME`, or `~/.claude-review`.
    pub fn locate() -> io::Result<Self> {
        if let Some(dir) = std::env::var_os(HOME_ENV).filter(|v| !v.is_empty()) {
            return Ok(Self::new(PathBuf::from(dir)));
        }

        let home = dirs::home_dir().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "could not find your home directory",
            )
        })?;

        Ok(Self::new(home.join(".claude-review")))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    pub fn session_dir(&self, id: &str) -> PathBuf {
        self.sessions_dir().join(id)
    }

    pub fn latest_review_path(&self, id: &str) -> PathBuf {
        self.session_dir(id).join(REVIEWS_DIR).join(LATEST_MARKDOWN)
    }

    fn read(&self, id: &str) -> Option<Session> {
        let text = fs::read_to_string(self.session_dir(id).join(SESSION_FILE)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, session: &Session) -> io::Result<()> {
        let json = serde_json::to_string_pretty(session)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        write_atomic(&self.session_dir(&session.id).join(SESSION_FILE), &json)
    }

    /// Every session, most recently touched first.
    pub fn list(&self) -> Vec<Session> {
        let Ok(entries) = fs::read_dir(self.sessions_dir()) else {
            return Vec::new();
        };

        let mut sessions: Vec<Session> = entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| self.read(&entry.file_name().to_string_lossy()))
            .collect();

        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
        sessions
    }

    pub fn get(&self, id: &str) -> Option<Session> {
        if id.is_empty() || id.contains(['/', '\\', '.']) {
            return None;
        }
        self.read(id)
    }

    /// Finds a session by its id or an unambiguous prefix of it.
    pub fn resolve(&self, id: &str) -> Result<Session> {
        let id = id.trim();
        if let Some(session) = self.get(id) {
            return Ok(session);
        }
        if id.is_empty() {
            return Err(Error::NotFound(id.to_string()));
        }

        let matches: Vec<Session> = self
            .list()
            .into_iter()
            .filter(|s| s.id.starts_with(id))
            .collect();

        match matches.len() {
            0 => Err(Error::NotFound(id.to_string())),
            1 => Ok(matches.into_iter().next().expect("one match")),
            _ => Err(Error::Ambiguous(
                id.to_string(),
                matches.into_iter().map(|s| s.id).collect(),
            )),
        }
    }

    pub fn summarize(&self, session: Session, near: Option<&Place>) -> SessionSummary {
        let root = Path::new(&session.path);
        let missing = !root.is_dir();

        let relation = near.and_then(|place| relation_to(&session, place));

        SessionSummary {
            branch: if missing {
                None
            } else {
                git::current_branch(root)
            },
            missing,
            draft_comments: load_draft(&self.session_dir(&session.id)).comments.len() as u32,
            relation,
            session,
        }
    }

    /// Creates a session for the worktree containing `input.path`, or reuses
    /// the open one that already covers it.
    pub fn register(&self, input: NewSession) -> Result<Registered> {
        let root = PathBuf::from(git::repo_root(&input.path)?);
        let root = fs::canonicalize(&root).unwrap_or(root);
        let path = root.to_string_lossy().into_owned();

        let name = clean_name(input.name);
        let base = clean_base(input.base);

        if let Some(base) = &base {
            if base != AUTO_BASE && git::resolve_rev(&root, base).is_none() {
                return Err(Error::Invalid(format!(
                    "`{base}` is not a revision in {path}"
                )));
            }
        }

        let mut notes = Vec::new();
        notes.extend(crate::install::remove_legacy_project_commands(&root));

        let existing = self
            .list()
            .into_iter()
            .find(|s| s.path == path && s.status != SessionStatus::Closed);

        if let Some(mut session) = existing {
            let mut changed = false;
            if let Some(name) = name {
                if session.name != name {
                    session.name = name;
                    changed = true;
                }
            }
            if let Some(base) = base {
                if session.base != base {
                    session.base = base;
                    changed = true;
                }
            }
            if let Some(untracked) = input.include_untracked {
                if session.include_untracked != untracked {
                    session.include_untracked = untracked;
                    changed = true;
                }
            }
            if changed {
                session.updated_at = now_iso();
                self.save(&session)?;
            }

            return Ok(Registered {
                session,
                reused: true,
                notes,
            });
        }

        let now = now_iso();
        let session = Session {
            id: self.unused_id(),
            name: name.unwrap_or_else(|| default_name(&root)),
            common_dir: git::common_dir(&root)?,
            path,
            base: base.unwrap_or_else(|| AUTO_BASE.to_string()),
            include_untracked: input.include_untracked.unwrap_or(true),
            created_by: input.created_by.unwrap_or(Creator::User),
            status: SessionStatus::Open,
            created_at: now.clone(),
            updated_at: now,
            submitted_at: None,
            taken_at: None,
            rounds: 0,
        };

        self.save(&session)?;
        notes.extend(self.import_legacy_draft(&session, &root));

        Ok(Registered {
            session,
            reused: false,
            notes,
        })
    }

    fn unused_id(&self) -> String {
        loop {
            let id = new_id();
            if !self.session_dir(&id).exists() {
                return id;
            }
        }
    }

    /// Brings over an unsubmitted draft left in the repository by a version
    /// from before sessions, so nothing in progress is lost.
    fn import_legacy_draft(&self, session: &Session, root: &Path) -> Vec<String> {
        let legacy_dir = legacy_review_dir(root);
        if !legacy_dir.is_dir() {
            return Vec::new();
        }

        let mut notes = Vec::new();
        let legacy_draft = legacy_dir.join(LEGACY_DRAFT_FILE);
        let draft = load_draft_file(&legacy_draft);

        if !draft.comments.is_empty() || !draft.overall.trim().is_empty() {
            save_draft(&self.session_dir(&session.id), &draft);

            let mut imported = legacy_draft.clone().into_os_string();
            imported.push(".imported");
            let _ = fs::rename(&legacy_draft, &imported);

            notes.push(format!(
                "Imported the unsubmitted draft from {} ({} comment{}).",
                legacy_draft.display(),
                draft.comments.len(),
                if draft.comments.len() == 1 { "" } else { "s" },
            ));
        }

        notes.push(format!(
            "{} is from an older claude-review and is no longer used; delete it when you no longer need the old reviews.",
            legacy_dir.display()
        ));

        notes
    }

    pub fn update(&self, id: &str, patch: SessionPatch) -> Result<Session> {
        let mut session = self.resolve(id)?;

        if let Some(name) = clean_name(patch.name) {
            session.name = name;
        }
        if let Some(base) = clean_base(patch.base) {
            if base != AUTO_BASE && git::resolve_rev(Path::new(&session.path), &base).is_none() {
                return Err(Error::Invalid(format!(
                    "`{base}` is not a revision in {}",
                    session.path
                )));
            }
            session.base = base;
        }
        if let Some(untracked) = patch.include_untracked {
            session.include_untracked = untracked;
        }
        if let Some(status) = patch.status {
            session.status = status;
        }

        session.updated_at = now_iso();
        self.save(&session)?;
        Ok(session)
    }

    /// Records a submitted review against the session.
    pub fn mark_submitted(&self, id: &str) -> Result<Session> {
        let mut session = self.resolve(id)?;
        let now = now_iso();

        session.status = SessionStatus::Submitted;
        session.submitted_at = Some(now.clone());
        session.updated_at = now;
        session.rounds += 1;

        self.save(&session)?;
        Ok(session)
    }

    /// The latest submitted review's markdown, if there is one.
    pub fn latest_review(&self, id: &str) -> Result<(Session, Option<String>)> {
        let session = self.resolve(id)?;
        let markdown = fs::read_to_string(self.latest_review_path(&session.id)).ok();
        Ok((session, markdown))
    }

    /// Hands the waiting review to Claude and marks it picked up.
    pub fn take(&self, id: &str) -> Result<(Session, String)> {
        let (mut session, markdown) = self.latest_review(id)?;

        let Some(markdown) = markdown.filter(|_| session.status == SessionStatus::Submitted) else {
            return Err(Error::Invalid(format!(
                "session {} ({}) has no submitted review waiting",
                session.id, session.name
            )));
        };

        let now = now_iso();
        session.status = SessionStatus::Open;
        session.taken_at = Some(now.clone());
        session.updated_at = now;
        self.save(&session)?;

        Ok((session, markdown))
    }

    pub fn close(&self, id: &str) -> Result<Session> {
        self.update(
            id,
            SessionPatch {
                status: Some(SessionStatus::Closed),
                ..SessionPatch::default()
            },
        )
    }

    pub fn server_info(&self) -> Option<ServerInfo> {
        let text = fs::read_to_string(self.root.join(SERVER_FILE)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn write_server_info(&self, info: &ServerInfo) -> io::Result<()> {
        let json = serde_json::to_string_pretty(info)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        write_atomic(&self.root.join(SERVER_FILE), &json)
    }

    /// Removes `server.json`, but only if it still describes this process.
    pub fn clear_server_info(&self, pid: u32) {
        if self.server_info().is_some_and(|info| info.pid == pid) {
            let _ = fs::remove_file(self.root.join(SERVER_FILE));
        }
    }
}
