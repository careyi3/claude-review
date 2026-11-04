mod common;

use std::path::PathBuf;

use claude_review::review::{build_review, BuildOptions};
use claude_review::types::{FileDiff, FileStatus, ReviewModel, SkipReason};
use common::{as_refs, lines, numbered, Repo};

fn options(cwd: &std::path::Path) -> BuildOptions {
    BuildOptions {
        cwd: cwd.to_path_buf(),
        ..BuildOptions::default()
    }
}

fn review(repo: &Repo) -> ReviewModel {
    build_review(&options(repo.path())).expect("build review")
}

fn by_path<'a>(model: &'a ReviewModel, path: &str) -> &'a FileDiff {
    model.file(path).unwrap_or_else(|| {
        let listed: Vec<&str> = model.files.iter().map(|f| f.path.as_str()).collect();
        panic!("no file {path} in [{}]", listed.join(", "))
    })
}

fn paths(model: &ReviewModel) -> Vec<&str> {
    model.files.iter().map(|f| f.path.as_str()).collect()
}

fn base_repo() -> Repo {
    let repo = Repo::new();
    repo.write("src/keep.ts", lines(&["export const keep = 1;"]));
    repo.write("src/edit.ts", lines(&["one", "two", "three"]));
    repo.write("src/gone.ts", lines(&["doomed"]));

    let stable = numbered(12, "stable");
    repo.write("src/move.ts", lines(&as_refs(&stable)));

    repo.commit("base");
    repo
}

#[test]
fn reports_a_clean_tree_as_no_files() {
    let repo = base_repo();
    let model = review(&repo);

    assert!(model.files.is_empty());
    assert_eq!(model.totals.files, 0);
    assert_eq!(model.totals.additions, 0);
    assert_eq!(model.totals.deletions, 0);

    let sha = model.base.sha.expect("a resolved base sha");
    assert_eq!(sha.len(), 40);
    assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn diffs_a_modified_file_and_leaves_untouched_files_out() {
    let repo = base_repo();
    repo.write("src/edit.ts", lines(&["one", "TWO", "three"]));

    let model = review(&repo);
    assert_eq!(paths(&model), vec!["src/edit.ts"]);

    let file = by_path(&model, "src/edit.ts");
    assert_eq!(file.status, FileStatus::Modified);
    assert_eq!((file.additions, file.deletions), (1, 1));
    assert_eq!(file.language.as_deref(), Some("typescript"));
    assert_eq!((file.old_line_count, file.new_line_count), (3, 3));
}

#[test]
fn includes_untracked_files_by_default_and_excludes_them_on_request() {
    let repo = base_repo();
    repo.write("src/brand-new.ts", lines(&["fresh"]));

    let included = review(&repo);
    let file = by_path(&included, "src/brand-new.ts");
    assert_eq!(file.status, FileStatus::Untracked);
    assert_eq!((file.additions, file.deletions), (1, 0));

    let excluded = build_review(&BuildOptions {
        include_untracked: false,
        ..options(repo.path())
    })
    .expect("build review");

    assert!(excluded.files.is_empty());
}

#[test]
fn handles_a_deleted_file_as_pure_deletions() {
    let repo = base_repo();
    repo.remove("src/gone.ts");

    let model = review(&repo);
    let file = by_path(&model, "src/gone.ts");

    assert_eq!(file.status, FileStatus::Deleted);
    assert_eq!((file.additions, file.deletions), (0, 1));
    assert_eq!(file.new_line_count, 0);
}

#[test]
fn detects_a_rename_and_diffs_against_the_original_path() {
    let repo = base_repo();
    repo.git(&["mv", "src/move.ts", "src/moved.ts"]);

    let mut changed = numbered(12, "stable");
    changed[0] = "CHANGED".to_string();
    repo.write("src/moved.ts", lines(&as_refs(&changed)));

    let model = review(&repo);
    let file = by_path(&model, "src/moved.ts");

    assert_eq!(file.status, FileStatus::Renamed);
    assert_eq!(file.old_path.as_deref(), Some("src/move.ts"));
    assert_eq!((file.additions, file.deletions), (1, 1));
}

#[test]
fn lists_a_binary_file_without_diffing_it() {
    let repo = base_repo();
    repo.write("assets/blob.bin", [0x00u8, 0x01, 0x02, 0x00]);

    let model = review(&repo);
    let file = by_path(&model, "assets/blob.bin");

    assert!(file.binary);
    assert_eq!(file.skipped, Some(SkipReason::Binary));
    assert!(file.hunks.is_empty());
}

#[test]
fn skips_files_over_the_size_limit() {
    let repo = base_repo();
    repo.write("src/big.ts", format!("{}\n", "x".repeat(5000)));

    let model = build_review(&BuildOptions {
        max_file_bytes: 1000,
        ..options(repo.path())
    })
    .expect("build review");

    let file = by_path(&model, "src/big.ts");
    assert_eq!(file.skipped, Some(SkipReason::TooLarge));
    assert!(file.hunks.is_empty());
    assert!(file.new_size > 1000);
}

#[test]
fn respects_the_context_setting() {
    let repo = base_repo();

    let mut changed = numbered(12, "stable");
    changed[5] = "CHANGED".to_string();
    repo.write("src/move.ts", lines(&as_refs(&changed)));

    let with_context = |n: u32| {
        let model = build_review(&BuildOptions {
            context_lines: n,
            ..options(repo.path())
        })
        .expect("build review");

        by_path(&model, "src/move.ts").hunks[0].lines.len()
    };

    assert!(with_context(1) < with_context(5));
}

#[test]
fn picks_up_staged_changes_as_well_as_unstaged_ones() {
    let repo = base_repo();
    repo.write("src/edit.ts", lines(&["one", "two", "three", "four"]));
    repo.git(&["add", "src/edit.ts"]);
    repo.write("src/keep.ts", lines(&["export const keep = 2;"]));

    let model = review(&repo);

    assert_eq!(paths(&model), vec!["src/edit.ts", "src/keep.ts"]);
    assert_eq!(model.totals.additions, 2);
}

#[test]
fn treats_every_tracked_file_as_added_in_a_repo_with_no_commits() {
    let repo = Repo::new();
    repo.write("a.txt", lines(&["hello"]));

    let model = review(&repo);

    assert!(model.base.sha.is_none());
    let file = by_path(&model, "a.txt");
    assert_eq!((file.additions, file.deletions), (1, 0));
}

#[test]
fn gives_each_file_a_digest_that_tracks_its_diff() {
    let repo = base_repo();
    repo.write("src/edit.ts", lines(&["one", "TWO", "three"]));

    let first = by_path(&review(&repo), "src/edit.ts").digest.clone();

    assert_eq!(first.len(), 16);
    assert!(first.chars().all(|c| c.is_ascii_hexdigit()));

    assert_eq!(by_path(&review(&repo), "src/edit.ts").digest, first);

    repo.write("src/edit.ts", lines(&["one", "THREE", "three"]));
    assert_ne!(by_path(&review(&repo), "src/edit.ts").digest, first);
}

#[test]
fn resolves_the_repo_root_from_a_subdirectory() {
    let repo = base_repo();
    repo.write("src/edit.ts", lines(&["one", "TWO", "three"]));

    let model = build_review(&options(&repo.path().join("src"))).expect("build review");

    assert_eq!(PathBuf::from(&model.repo_root), repo.path());
    assert_eq!(paths(&model), vec!["src/edit.ts"]);
}

#[test]
fn reports_a_missing_repository_as_an_error() {
    let outside = tempfile::TempDir::with_prefix("claude-review-bare-").expect("temp dir");

    assert!(build_review(&options(outside.path())).is_err());
}

fn against(repo: &Repo, base: &str) -> ReviewModel {
    build_review(&BuildOptions {
        base: base.to_string(),
        ..options(repo.path())
    })
    .expect("build review")
}

#[test]
fn an_older_base_includes_work_committed_since() {
    let repo = base_repo();
    repo.write("src/edit.ts", lines(&["one", "TWO", "three"]));
    repo.remove("src/gone.ts");
    repo.write("src/new.ts", lines(&["committed"]));
    repo.commit("branch work");
    repo.write("src/wip.ts", lines(&["uncommitted"]));

    let model = against(&repo, "HEAD~1");

    assert_eq!(
        paths(&model),
        vec!["src/edit.ts", "src/gone.ts", "src/new.ts", "src/wip.ts"]
    );
    assert_eq!(by_path(&model, "src/edit.ts").status, FileStatus::Modified);
    assert_eq!(by_path(&model, "src/gone.ts").status, FileStatus::Deleted);
    assert_eq!(by_path(&model, "src/new.ts").status, FileStatus::Added);
    assert_eq!(by_path(&model, "src/wip.ts").status, FileStatus::Untracked);

    assert_eq!(
        paths(&review(&repo)),
        vec!["src/wip.ts"],
        "HEAD still means uncommitted only"
    );
}

#[test]
fn an_older_base_follows_a_rename_committed_since() {
    let repo = base_repo();
    repo.git(&["mv", "src/move.ts", "src/moved.ts"]);
    repo.commit("rename");

    let model = against(&repo, "HEAD~1");
    let file = by_path(&model, "src/moved.ts");

    assert_eq!(file.status, FileStatus::Renamed);
    assert_eq!(file.old_path.as_deref(), Some("src/move.ts"));
}
