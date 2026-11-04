mod common;

use std::fs;
use std::path::Path;

use claude_review::drafts::{load_draft, save_draft};
use claude_review::registry::{
    relation_to, resolve_base, Creator, Error, NewSession, Place, Registry, Relation, SessionPatch,
    SessionStatus, AUTO_BASE,
};
use common::{lines, Repo};
use tempfile::TempDir;

struct Fixture {
    repo: Repo,
    registry: Registry,
    trees: TempDir,
    _home: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let repo = Repo::new();
        repo.write("a.txt", lines(&["alpha"]));
        repo.commit("base");

        let home = TempDir::with_prefix("claude-review-registry-").expect("temp registry");
        Self {
            repo,
            registry: Registry::new(home.path()),
            trees: TempDir::with_prefix("claude-review-trees-").expect("temp worktrees"),
            _home: home,
        }
    }

    fn register(&self, path: &Path) -> claude_review::registry::Registered {
        self.registry
            .register(NewSession {
                path: path.to_path_buf(),
                ..NewSession::default()
            })
            .expect("register")
    }

    fn submit(&self, id: &str, markdown: &str) {
        let reviews = self.registry.session_dir(id).join("reviews");
        fs::create_dir_all(&reviews).expect("mkdir reviews");
        fs::write(reviews.join("latest.md"), markdown).expect("write review");
        self.registry.mark_submitted(id).expect("mark submitted");
    }

    fn worktree(&self, name: &str) -> std::path::PathBuf {
        let path = self.trees.path().join(name);
        self.repo
            .git(&["worktree", "add", "-b", name, &path.to_string_lossy()]);
        fs::canonicalize(path).expect("canonical worktree")
    }
}

#[test]
fn registers_a_session_with_sensible_defaults() {
    let f = Fixture::new();
    let registered = f.register(f.repo.path());
    let session = &registered.session;

    assert!(!registered.reused);
    assert_eq!(session.path, f.repo.path().to_string_lossy());
    assert_eq!(session.base, AUTO_BASE);
    assert_eq!(session.status, SessionStatus::Open);
    assert_eq!(session.created_by, Creator::User);
    assert!(session.include_untracked);
    assert!(
        session.name.ends_with("· main"),
        "name was {}",
        session.name
    );
    assert_eq!(session.id.len(), 8);
    assert!(f
        .registry
        .session_dir(&session.id)
        .join("session.json")
        .exists());
}

#[test]
fn registering_from_a_subdirectory_uses_the_top_level() {
    let f = Fixture::new();
    f.repo.write("src/deep/file.txt", "x\n");

    let session = f.register(&f.repo.path().join("src/deep")).session;

    assert_eq!(session.path, f.repo.path().to_string_lossy());
}

#[test]
fn reuses_the_open_session_for_the_same_worktree() {
    let f = Fixture::new();
    let first = f.register(f.repo.path()).session;

    let again = f
        .registry
        .register(NewSession {
            path: f.repo.path().to_path_buf(),
            name: Some("renamed".into()),
            created_by: Some(Creator::Claude),
            ..NewSession::default()
        })
        .expect("register again");

    assert!(again.reused);
    assert_eq!(again.session.id, first.id);
    assert_eq!(again.session.name, "renamed");
    assert_eq!(f.registry.list().len(), 1);
}

#[test]
fn a_closed_session_is_not_reused() {
    let f = Fixture::new();
    let first = f.register(f.repo.path()).session;
    f.registry.close(&first.id).expect("close");

    let second = f.register(f.repo.path());

    assert!(!second.reused);
    assert_ne!(second.session.id, first.id);
}

#[test]
fn refuses_a_directory_outside_git() {
    let f = Fixture::new();
    let outside = TempDir::with_prefix("claude-review-plain-").expect("temp dir");

    let err = f
        .registry
        .register(NewSession {
            path: outside.path().to_path_buf(),
            ..NewSession::default()
        })
        .expect_err("not a repository");

    assert!(matches!(err, Error::Git(_)));
}

#[test]
fn refuses_a_base_that_does_not_resolve() {
    let f = Fixture::new();

    let err = f
        .registry
        .register(NewSession {
            path: f.repo.path().to_path_buf(),
            base: Some("no-such-branch".into()),
            ..NewSession::default()
        })
        .expect_err("bad base");

    assert!(matches!(err, Error::Invalid(_)));
}

#[test]
fn each_worktree_gets_its_own_session_sharing_the_common_dir() {
    let f = Fixture::new();
    let main = f.register(f.repo.path()).session;
    let other = f.register(&f.worktree("feature")).session;

    assert_ne!(main.id, other.id);
    assert_eq!(main.common_dir, other.common_dir);
    assert!(other.name.ends_with("· feature"));
}

#[test]
fn resolves_an_id_prefix_and_rejects_unknown_ids() {
    let f = Fixture::new();
    let session = f.register(f.repo.path()).session;

    assert_eq!(
        f.registry.resolve(&session.id[..4]).expect("prefix").id,
        session.id
    );
    assert!(matches!(
        f.registry.resolve("zzzzzzzz"),
        Err(Error::NotFound(_))
    ));
    assert!(f.registry.get("../escape").is_none());
}

#[test]
fn take_hands_over_the_review_once() {
    let f = Fixture::new();
    let session = f.register(f.repo.path()).session;

    assert!(
        f.registry.take(&session.id).is_err(),
        "nothing submitted yet"
    );

    f.submit(&session.id, "# review one");
    let submitted = f.registry.get(&session.id).expect("session");
    assert_eq!(submitted.status, SessionStatus::Submitted);
    assert_eq!(submitted.rounds, 1);

    let (taken, markdown) = f.registry.take(&session.id).expect("take");
    assert_eq!(markdown, "# review one");
    assert_eq!(taken.status, SessionStatus::Open);
    assert!(taken.taken_at.is_some());

    assert!(f.registry.take(&session.id).is_err(), "already taken");
    let (_, again) = f.registry.latest_review(&session.id).expect("show");
    assert_eq!(again.as_deref(), Some("# review one"));
}

#[test]
fn updates_name_and_base() {
    let f = Fixture::new();
    let session = f.register(f.repo.path()).session;

    let updated = f
        .registry
        .update(
            &session.id,
            SessionPatch {
                name: Some("  new name  ".into()),
                base: Some("HEAD".into()),
                ..SessionPatch::default()
            },
        )
        .expect("update");

    assert_eq!(updated.name, "new name");
    assert_eq!(updated.base, "HEAD");
}

#[test]
fn lists_most_recently_touched_first() {
    let f = Fixture::new();
    let first = f.register(f.repo.path()).session;
    let second = f.register(&f.worktree("feature")).session;

    std::thread::sleep(std::time::Duration::from_millis(5));
    f.registry.mark_submitted(&first.id).expect("touch first");

    let ids: Vec<String> = f.registry.list().into_iter().map(|s| s.id).collect();
    assert_eq!(ids, vec![first.id, second.id]);
}

#[test]
fn relates_sessions_to_a_directory() {
    let f = Fixture::new();
    let main = f.register(f.repo.path()).session;
    let feature_path = f.worktree("feature");
    let feature = f.register(&feature_path).session;

    let in_main = Place::of(f.repo.path());
    assert_eq!(relation_to(&main, &in_main), Some(Relation::Same));
    assert_eq!(relation_to(&feature, &in_main), Some(Relation::Worktree));

    let above = Place::of(f.repo.path().parent().expect("parent"));
    assert_eq!(relation_to(&main, &above), Some(Relation::Parent));

    let unrelated = TempDir::with_prefix("claude-review-plain-").expect("temp dir");
    assert_eq!(relation_to(&main, &Place::of(unrelated.path())), None);
}

#[test]
fn a_worktree_nested_inside_the_main_checkout_is_not_the_main_session() {
    let f = Fixture::new();
    let main = f.register(f.repo.path()).session;

    let nested = f.repo.path().join(".claude/worktrees/task");
    f.repo
        .git(&["worktree", "add", "-b", "task", &nested.to_string_lossy()]);

    let place = Place::of(&nested);
    assert_eq!(relation_to(&main, &place), Some(Relation::Worktree));

    let task = f.register(&nested).session;
    assert_eq!(relation_to(&task, &place), Some(Relation::Same));
}

#[test]
fn auto_base_is_head_on_the_default_branch() {
    let f = Fixture::new();

    assert_eq!(
        resolve_base(f.repo.path(), AUTO_BASE),
        ("HEAD".into(), None)
    );
}

#[test]
fn auto_base_is_the_merge_base_on_a_feature_branch() {
    let f = Fixture::new();
    let split = f.repo.git(&["rev-parse", "HEAD"]).trim().to_string();

    f.repo.git(&["checkout", "-b", "feature"]);
    f.repo.write("b.txt", "committed on the branch\n");
    f.repo.commit("feature work");

    let (rev, label) = resolve_base(f.repo.path(), AUTO_BASE);
    assert_eq!(rev, split);
    assert_eq!(label.as_deref(), Some("main (merge-base)"));

    let model =
        claude_review::review::build_review(&f.register(f.repo.path()).session.build_options())
            .expect("build");
    let paths: Vec<&str> = model.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["b.txt"], "committed branch work is in the diff");
    assert_eq!(model.base.rev, "main (merge-base)");
}

#[test]
fn an_explicit_base_is_used_as_given() {
    let f = Fixture::new();
    assert_eq!(
        resolve_base(f.repo.path(), "HEAD~0"),
        ("HEAD~0".into(), None)
    );
}

#[test]
fn imports_an_unsubmitted_draft_left_in_the_repository() {
    let f = Fixture::new();
    let legacy = f.repo.path().join(".claude/review");
    save_draft(&legacy, &{
        let mut draft = claude_review::drafts::empty_draft();
        draft.overall = "carried over".into();
        draft
    });
    // save_draft writes draft.json; older versions called it .draft.json.
    fs::rename(legacy.join("draft.json"), legacy.join(".draft.json")).expect("rename");

    let registered = f.register(f.repo.path());

    let imported = load_draft(&f.registry.session_dir(&registered.session.id));
    assert_eq!(imported.overall, "carried over");
    assert!(legacy.join(".draft.json.imported").exists());
    assert!(!legacy.join(".draft.json").exists());
    assert_eq!(registered.notes.len(), 2);
}

#[test]
fn says_nothing_about_legacy_files_when_there_are_none() {
    let f = Fixture::new();
    assert!(f.register(f.repo.path()).notes.is_empty());
}

#[test]
fn summaries_flag_a_missing_worktree() {
    let f = Fixture::new();
    let path = f.worktree("doomed");
    let session = f.register(&path).session;

    f.repo
        .git(&["worktree", "remove", "--force", &path.to_string_lossy()]);

    let summary = f.registry.summarize(session, None);
    assert!(summary.missing);
    assert_eq!(summary.branch, None);
}

#[test]
fn records_and_clears_the_running_server() {
    let f = Fixture::new();
    let info = claude_review::registry::ServerInfo {
        pid: 42,
        port: 1234,
        token: "t".into(),
        url: "http://127.0.0.1:1234/?token=t".into(),
        started_at: "now".into(),
    };

    f.registry.write_server_info(&info).expect("write");
    assert_eq!(f.registry.server_info(), Some(info));

    f.registry.clear_server_info(7);
    assert!(
        f.registry.server_info().is_some(),
        "another process's record is kept"
    );

    f.registry.clear_server_info(42);
    assert!(f.registry.server_info().is_none());
}
