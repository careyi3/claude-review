mod common;

use std::collections::BTreeMap;

use claude_review::anchor::create_anchor;
use claude_review::drafts::{
    draft_path, empty_draft, load_draft, normalize_draft, resolve_draft, save_draft, set_viewed,
};
use claude_review::review::{build_review, BuildOptions};
use claude_review::types::{AnchorState, Comment, CommentTag, Draft, ReviewModel, Side};
use common::{lines, Repo};
use serde_json::json;

const FILE_LINES: [&str; 4] = ["alpha", "beta", "gamma", "delta"];

fn owned(ls: &[&str]) -> Vec<String> {
    ls.iter().map(|s| s.to_string()).collect()
}

fn comment() -> Comment {
    Comment {
        id: "c1".to_string(),
        path: "a.txt".to_string(),
        anchor: Some(create_anchor(Side::New, 3, 3, &owned(&FILE_LINES))),
        tag: CommentTag::MustFix,
        body: "this looks wrong".to_string(),
        created_at: "2026-01-01T00:00:00.000Z".to_string(),
        updated_at: "2026-01-01T00:00:00.000Z".to_string(),
        state: None,
        resolved_start_line: None,
        resolved_end_line: None,
    }
}

fn draft_with(comments: Vec<Comment>, overall: &str) -> Draft {
    Draft {
        version: 1,
        overall: overall.to_string(),
        comments,
        viewed: BTreeMap::new(),
        updated_at: "2026-01-01T00:00:00.000Z".to_string(),
    }
}

fn committed_repo() -> Repo {
    let repo = Repo::new();
    repo.write("a.txt", lines(&FILE_LINES));
    repo.commit("base");
    repo
}

fn review(repo: &Repo) -> ReviewModel {
    build_review(&BuildOptions {
        cwd: repo.path().to_path_buf(),
        ..BuildOptions::default()
    })
    .expect("build review")
}

#[test]
fn returns_an_empty_draft_for_junk_input() {
    for junk in [json!(null), json!("nonsense"), json!(7), json!([])] {
        let draft = normalize_draft(&junk);
        assert_eq!(draft.version, 1);
        assert_eq!(draft.overall, "");
        assert!(draft.comments.is_empty());
    }
}

#[test]
fn drops_comments_with_no_body_or_no_path() {
    let blank_body = json!({ "path": "a.txt", "body": "   " });
    let no_path = json!({ "path": "", "body": "real" });
    let good = json!({ "path": "a.txt", "body": "real" });

    let draft = normalize_draft(&json!({ "comments": [blank_body, no_path, good] }));

    assert_eq!(draft.comments.len(), 1);
}

#[test]
fn falls_back_to_the_question_tag_for_an_unknown_tag() {
    let draft = normalize_draft(&json!({
        "comments": [{ "path": "a.txt", "body": "hm", "tag": "wat" }]
    }));

    assert_eq!(draft.comments[0].tag, CommentTag::Question);
}

#[test]
fn discards_a_malformed_anchor_rather_than_the_whole_comment() {
    let draft = normalize_draft(&json!({
        "comments": [{
            "path": "a.txt",
            "body": "hm",
            "anchor": { "side": "new", "startLine": 0 }
        }]
    }));

    assert_eq!(draft.comments.len(), 1);
    assert!(draft.comments[0].anchor.is_none());
}

#[test]
fn rejects_an_anchor_whose_end_precedes_its_start() {
    let mut anchor = create_anchor(Side::New, 3, 3, &owned(&FILE_LINES));
    anchor.start_line = 5;
    anchor.end_line = 2;

    let draft = normalize_draft(&json!({
        "comments": [{ "path": "a.txt", "body": "hm", "anchor": anchor }]
    }));

    assert!(draft.comments[0].anchor.is_none());
}

#[test]
fn keeps_the_overall_note() {
    let draft = normalize_draft(&json!({ "overall": "looks good overall" }));
    assert_eq!(draft.overall, "looks good overall");
}

#[test]
fn never_trusts_a_stored_anchor_state() {
    let draft = normalize_draft(&json!({
        "comments": [{
            "path": "a.txt",
            "body": "hm",
            "state": "anchored",
            "resolvedStartLine": 42
        }]
    }));

    assert!(draft.comments[0].state.is_none());
    assert!(draft.comments[0].resolved_start_line.is_none());
}

#[test]
fn empty_draft_is_a_valid_draft() {
    let draft = normalize_draft(&serde_json::to_value(empty_draft()).unwrap());

    assert_eq!(draft.version, 1);
    assert!(draft.comments.is_empty());
    assert_eq!(draft.overall, "");
}

#[test]
fn returns_an_empty_draft_when_none_has_been_saved() {
    let repo = committed_repo();
    let draft = load_draft(repo.store());

    assert!(draft.comments.is_empty());
    assert_eq!(draft.overall, "");
}

#[test]
fn round_trips_a_saved_draft() {
    let repo = committed_repo();
    save_draft(repo.store(), &draft_with(vec![comment()], "overall note"));

    let loaded = load_draft(repo.store());

    assert_eq!(loaded.overall, "overall note");
    assert_eq!(loaded.comments[0].id, "c1");
    assert_eq!(loaded.comments[0].tag, CommentTag::MustFix);
    assert_eq!(loaded.comments[0].body, "this looks wrong");
}

#[test]
fn writes_into_the_session_directory_and_leaves_no_temp_files_behind() {
    let repo = committed_repo();
    save_draft(repo.store(), &draft_with(vec![comment()], ""));

    assert!(draft_path(repo.store()).exists());
    let names: Vec<String> = std::fs::read_dir(repo.store())
        .expect("read store")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["draft.json"]);
    assert!(review(&repo).files.is_empty());
}

#[test]
fn survives_a_corrupt_draft_file() {
    let repo = committed_repo();
    save_draft(repo.store(), &draft_with(vec![comment()], ""));
    std::fs::write(draft_path(repo.store()), "{ not json").expect("corrupt draft");

    assert!(load_draft(repo.store()).comments.is_empty());
}

#[test]
fn keeps_a_legacy_in_repo_review_directory_out_of_the_review() {
    let repo = committed_repo();
    repo.write(".claude/review/.draft.json", "{}");
    repo.write(".claude/review/latest.md", "# old");

    assert!(review(&repo).files.is_empty());
}

fn resolved_state(repo: &Repo, comments: Vec<Comment>) -> Comment {
    let model = review(repo);
    let resolved = resolve_draft(&model, &draft_with(comments, ""));

    resolved.comments.into_iter().next().expect("one comment")
}

#[test]
fn marks_a_comment_anchored_when_its_line_is_untouched() {
    let repo = committed_repo();
    repo.write("a.txt", lines(&["alpha", "beta", "gamma", "DELTA"]));

    let resolved = resolved_state(&repo, vec![comment()]);

    assert_eq!(resolved.state, Some(AnchorState::Anchored));
    assert_eq!(resolved.resolved_start_line, Some(3));
}

#[test]
fn moves_a_comment_when_lines_are_inserted_above_it() {
    let repo = committed_repo();
    repo.write(
        "a.txt",
        lines(&["header", "alpha", "beta", "gamma", "delta"]),
    );

    let resolved = resolved_state(&repo, vec![comment()]);

    assert_eq!(resolved.state, Some(AnchorState::Moved));
    assert_eq!(resolved.resolved_start_line, Some(4));
}

#[test]
fn orphans_a_comment_whose_line_was_deleted() {
    let repo = committed_repo();
    repo.write("a.txt", lines(&["alpha", "beta", "delta"]));

    assert_eq!(
        resolved_state(&repo, vec![comment()]).state,
        Some(AnchorState::Orphaned)
    );
}

#[test]
fn leaves_file_level_comments_alone() {
    let repo = committed_repo();
    repo.write("a.txt", lines(&["totally", "different"]));

    let file_level = Comment {
        anchor: None,
        ..comment()
    };
    let resolved = resolved_state(&repo, vec![file_level]);

    assert_eq!(resolved.state, Some(AnchorState::Anchored));
    assert!(resolved.resolved_start_line.is_none());
}

#[test]
fn orphans_a_comment_on_a_file_that_is_no_longer_in_the_diff() {
    let repo = committed_repo();

    assert_eq!(
        resolved_state(&repo, vec![comment()]).state,
        Some(AnchorState::Orphaned)
    );
}

#[test]
fn resolves_comments_against_the_old_side_too() {
    let repo = committed_repo();
    repo.write(
        "a.txt",
        lines(&["alpha", "beta", "gamma", "delta", "epsilon"]),
    );

    let on_old = Comment {
        anchor: Some(create_anchor(Side::Old, 2, 2, &owned(&FILE_LINES))),
        ..comment()
    };
    let resolved = resolved_state(&repo, vec![on_old]);

    assert_eq!(resolved.state, Some(AnchorState::Anchored));
    assert_eq!(resolved.resolved_start_line, Some(2));
}

fn edited_repo() -> Repo {
    let repo = committed_repo();
    repo.write("a.txt", lines(&["alpha", "BETA", "gamma", "delta"]));
    repo
}

#[test]
fn records_the_digest_a_file_was_viewed_at() {
    let repo = edited_repo();
    let model = review(&repo);
    let digest = model.files[0].digest.clone();

    let draft = set_viewed(repo.store(), &model, "a.txt", Some(&digest));

    assert_eq!(draft.viewed.get("a.txt"), Some(&digest));
}

#[test]
fn clears_the_mark_when_given_none() {
    let repo = edited_repo();
    let model = review(&repo);
    let digest = model.files[0].digest.clone();

    set_viewed(repo.store(), &model, "a.txt", Some(&digest));
    let cleared = set_viewed(repo.store(), &model, "a.txt", None);

    assert!(cleared.viewed.is_empty());
}

#[test]
fn keeps_the_stale_digest_so_the_ui_can_tell_the_file_moved_on() {
    let repo = edited_repo();

    let before = review(&repo);
    let stale = before.files[0].digest.clone();
    set_viewed(repo.store(), &before, "a.txt", Some(&stale));

    repo.write("a.txt", lines(&["alpha", "GAMMA", "gamma", "delta"]));
    let after = review(&repo);

    assert_eq!(load_draft(repo.store()).viewed.get("a.txt"), Some(&stale));
    assert_ne!(stale, after.files[0].digest);
}

#[test]
fn discards_non_string_entries_from_a_hand_edited_draft() {
    let draft = normalize_draft(&json!({ "viewed": { "a.txt": 5, "b.txt": "ok" } }));

    assert_eq!(draft.viewed.len(), 1);
    assert_eq!(draft.viewed.get("b.txt"), Some(&"ok".to_string()));
}
