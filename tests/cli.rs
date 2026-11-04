mod common;

use std::process::{Command, Output};

use claude_review::registry::Registry;
use common::{lines, Repo};
use serde_json::Value;
use tempfile::TempDir;

struct Cli {
    home: TempDir,
}

impl Cli {
    fn new() -> Self {
        Self {
            home: TempDir::with_prefix("claude-review-cli-").expect("temp home"),
        }
    }

    fn run(&self, cwd: &std::path::Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_claude-review"))
            .args(args)
            .current_dir(cwd)
            .env("CLAUDE_REVIEW_HOME", self.home.path())
            .output()
            .expect("run claude-review")
    }

    fn stdout(&self, cwd: &std::path::Path, args: &[&str]) -> String {
        let out = self.run(cwd, args);
        assert!(
            out.status.success(),
            "{args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

fn repo() -> Repo {
    let repo = Repo::new();
    repo.write("a.txt", lines(&["alpha"]));
    repo.commit("base");
    repo
}

#[test]
fn register_list_take_and_close() {
    let cli = Cli::new();
    let repo = repo();

    let registered: Value = serde_json::from_str(&cli.stdout(
        repo.path(),
        &["register", "--by", "claude", "--name", "task", "--json"],
    ))
    .expect("register json");
    let id = registered["session"]["id"]
        .as_str()
        .expect("id")
        .to_string();
    assert_eq!(registered["session"]["createdBy"], "claude");
    assert_eq!(registered["url"], Value::Null, "no app is running");

    let listed: Value =
        serde_json::from_str(&cli.stdout(repo.path(), &["sessions", "--json"])).expect("json");
    assert_eq!(listed[0]["id"], id.as_str());
    assert_eq!(listed[0]["relation"], "same");

    let waiting: Value = serde_json::from_str(&cli.stdout(
        repo.path(),
        &["sessions", "--status", "submitted", "--json"],
    ))
    .expect("json");
    assert!(waiting.as_array().unwrap().is_empty());

    assert!(!cli.run(repo.path(), &["take", &id]).status.success());

    let registry = Registry::new(cli.home.path());
    let reviews = registry.session_dir(&id).join("reviews");
    std::fs::create_dir_all(&reviews).unwrap();
    std::fs::write(reviews.join("latest.md"), "# the review\n").unwrap();
    registry.mark_submitted(&id).unwrap();

    let taken = cli.stdout(repo.path(), &["take", &id[..4]]);
    assert!(taken.starts_with(&format!("Session: {id} (task)")));
    assert!(taken.contains("# the review"));
    assert!(
        !cli.run(repo.path(), &["take", &id]).status.success(),
        "taken once"
    );
    assert!(cli
        .stdout(repo.path(), &["show", &id])
        .contains("# the review"));

    cli.stdout(repo.path(), &["close", &id]);
    assert!(cli
        .stdout(repo.path(), &["sessions"])
        .starts_with("No sessions"));
}

#[test]
fn register_points_at_another_directory() {
    let cli = Cli::new();
    let repo = repo();
    let elsewhere = TempDir::with_prefix("claude-review-elsewhere-").unwrap();
    let path = repo.path().to_string_lossy().into_owned();

    let out = cli.stdout(elsewhere.path(), &["register", "--path", &path]);
    assert!(out.contains(&format!("for {path}")));

    let listed: Value = serde_json::from_str(
        &cli.stdout(elsewhere.path(), &["sessions", "--json", "--near", &path]),
    )
    .unwrap();
    assert_eq!(listed[0]["relation"], "same");
}

#[test]
fn rejects_an_unknown_status_filter() {
    let cli = Cli::new();
    let repo = repo();

    let out = cli.run(repo.path(), &["sessions", "--status", "pending"]);
    assert!(!out.status.success());
}
