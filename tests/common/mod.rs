#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

pub struct Repo {
    pub dir: PathBuf,
    /// Stands in for the session's directory in the registry.
    pub store: PathBuf,
    _temp: TempDir,
    _store: TempDir,
}

impl Repo {
    pub fn new() -> Self {
        let temp = TempDir::with_prefix("claude-review-").expect("temp dir");

        let dir = fs::canonicalize(temp.path()).expect("canonical temp dir");

        let store_temp = TempDir::with_prefix("claude-review-store-").expect("temp store");
        let store = fs::canonicalize(store_temp.path()).expect("canonical store dir");

        let repo = Self {
            dir,
            store,
            _temp: temp,
            _store: store_temp,
        };
        repo.git(&["init", "-b", "main"]);
        repo
    }

    pub fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .output()
            .expect("run git");

        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );

        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    pub fn write(&self, path: &str, content: impl AsRef<[u8]>) {
        let abs = self.dir.join(path);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(abs, content).expect("write fixture file");
    }

    pub fn read(&self, path: &str) -> String {
        fs::read_to_string(self.dir.join(path)).expect("read fixture file")
    }

    pub fn remove(&self, path: &str) {
        fs::remove_file(self.dir.join(path)).expect("remove fixture file");
    }

    pub fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-m", message, "--no-gpg-sign"]);
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    pub fn store(&self) -> &Path {
        &self.store
    }

    pub fn list(&self, dir: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.dir.join(dir))
            .expect("read dir")
            .map(|entry| {
                entry
                    .expect("dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }
}

pub fn lines(ls: &[&str]) -> String {
    let mut out = ls.join("\n");
    out.push('\n');
    out
}

pub fn numbered(n: usize, prefix: &str) -> Vec<String> {
    (1..=n).map(|i| format!("{prefix} {i}")).collect()
}

pub fn as_refs(items: &[String]) -> Vec<&str> {
    items.iter().map(String::as_str).collect()
}
