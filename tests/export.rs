mod common;

use std::collections::BTreeMap;

use claude_review::anchor::create_anchor;
use claude_review::drafts::{load_draft, resolve_draft, save_draft};
use claude_review::export::{render_markdown, submit_review, SubmitResult};
use claude_review::review::{build_review, BuildOptions};
use claude_review::types::{Comment, CommentTag, Draft, ReviewModel, Side, COMMENT_TAGS};
use common::{lines, Repo};

const BEFORE: [&str; 3] = ["function go() {", "  return 1;", "}"];
const AFTER: [&str; 3] = ["function go() {", "  return 2;", "}"];

fn owned(ls: &[&str]) -> Vec<String> {
    ls.iter().map(|s| s.to_string()).collect()
}

fn comment() -> Comment {
    Comment {
        id: "id-1".to_string(),
        path: "src/go.ts".to_string(),
        anchor: Some(create_anchor(Side::New, 2, 2, &owned(&AFTER))),
        tag: CommentTag::MustFix,
        body: "off by one".to_string(),
        created_at: "2026-01-01T00:00:00.000Z".to_string(),
        updated_at: "2026-01-01T00:00:00.000Z".to_string(),
        state: None,
        resolved_start_line: None,
        resolved_end_line: None,
    }
}

fn draft_of(comments: Vec<Comment>, overall: &str) -> Draft {
    Draft {
        version: 1,
        overall: overall.to_string(),
        comments,
        viewed: BTreeMap::new(),
        updated_at: "2026-02-03T04:05:06.000Z".to_string(),
    }
}

struct Fixture {
    repo: Repo,
    model: ReviewModel,
}

impl Fixture {
    fn new() -> Self {
        let repo = Repo::new();
        repo.write("src/go.ts", lines(&BEFORE));
        repo.commit("base");
        repo.write("src/go.ts", lines(&AFTER));

        let mut fixture = Self {
            repo,
            model: ReviewModel {
                version: 1,
                repo_root: String::new(),
                base: claude_review::types::ReviewBase {
                    rev: "HEAD".into(),
                    sha: None,
                },
                generated_at: String::new(),
                include_untracked: true,
                context_lines: 3,
                files: Vec::new(),
                totals: claude_review::types::Totals {
                    files: 0,
                    additions: 0,
                    deletions: 0,
                    skipped: 0,
                },
            },
        };
        fixture.rebuild();
        fixture
    }

    fn rebuild(&mut self) {
        self.model = build_review(&BuildOptions {
            cwd: self.repo.path().to_path_buf(),
            ..BuildOptions::default()
        })
        .expect("build review");
    }

    fn render(&self, draft: Draft) -> String {
        let resolved = resolve_draft(&self.model, &draft);
        render_markdown(&self.model, &resolved)
    }

    fn submit(&self) -> SubmitResult {
        let draft = resolve_draft(&self.model, &load_draft(self.repo.store()));
        submit_review(self.repo.store(), &self.model, &draft).expect("submit")
    }
}

#[test]
fn summarises_the_diff_and_the_tag_counts_in_the_header() {
    let fixture = Fixture::new();
    let nit = Comment {
        id: "id-2".into(),
        tag: CommentTag::Nit,
        body: "naming".into(),
        ..comment()
    };

    let md = fixture.render(draft_of(vec![comment(), nit], ""));

    assert!(md.contains("# Code review — 2026-02-03T04:05:06.000Z"));
    assert!(md.contains("1 file changed, +1 \u{2212}1"));
    assert!(md.contains("1 must-fix, 1 nit"));
}

#[test]
fn includes_the_overall_note_or_says_there_is_none() {
    let fixture = Fixture::new();

    assert!(fixture
        .render(draft_of(vec![comment()], "looks fine"))
        .contains("looks fine"));
    assert!(fixture
        .render(draft_of(vec![comment()], ""))
        .contains("_No overall note._"));
}

#[test]
fn orders_sections_by_severity_and_numbers_comments_across_them() {
    let fixture = Fixture::new();

    let md = fixture.render(draft_of(
        vec![
            Comment {
                id: "a".into(),
                tag: CommentTag::Idea,
                body: "idea body".into(),
                ..comment()
            },
            Comment {
                id: "b".into(),
                tag: CommentTag::MustFix,
                body: "must body".into(),
                ..comment()
            },
            Comment {
                id: "c".into(),
                tag: CommentTag::Question,
                body: "question body".into(),
                ..comment()
            },
        ],
        "",
    ));

    let at = |needle: &str| {
        md.find(needle)
            .unwrap_or_else(|| panic!("missing {needle}"))
    };

    assert!(at("## Must fix") < at("## Questions"));
    assert!(at("## Questions") < at("## Ideas"));
    assert!(md.contains("### 1. src/go.ts:2 (new side)"));
    assert!(md.contains("### 3. src/go.ts:2 (new side)"));
}

#[test]
fn quotes_the_current_text_of_the_anchored_lines() {
    let fixture = Fixture::new();
    let md = fixture.render(draft_of(vec![comment()], ""));

    assert!(md.contains("```typescript\n  return 2;\n```"));
}

#[test]
fn renders_a_line_range_rather_than_a_single_line_number() {
    let fixture = Fixture::new();
    let spanning = Comment {
        anchor: Some(create_anchor(Side::New, 1, 3, &owned(&AFTER))),
        ..comment()
    };

    let md = fixture.render(draft_of(vec![spanning], ""));

    assert!(md.contains("### 1. src/go.ts:1\u{2013}3 (new side)"));
}

#[test]
fn labels_a_file_level_comment() {
    let fixture = Fixture::new();
    let whole_file = Comment {
        anchor: None,
        ..comment()
    };

    assert!(fixture
        .render(draft_of(vec![whole_file], ""))
        .contains("src/go.ts — whole file"));
}

#[test]
fn flags_a_moved_comment_and_still_quotes_the_right_lines() {
    let mut fixture = Fixture::new();
    fixture.repo.write(
        "src/go.ts",
        lines(&["// header", AFTER[0], AFTER[1], AFTER[2]]),
    );
    fixture.rebuild();

    let md = fixture.render(draft_of(vec![comment()], ""));

    assert!(md.contains("the comment followed its lines"));
    assert!(md.contains("  return 2;"));
}

#[test]
fn falls_back_to_the_recorded_text_for_an_orphaned_comment() {
    let mut fixture = Fixture::new();
    fixture.repo.write(
        "src/go.ts",
        lines(&["function go() {", "  return 99;", "}"]),
    );
    fixture.rebuild();

    let md = fixture.render(draft_of(vec![comment()], ""));

    assert!(md.contains("the lines this was written against are gone"));
    assert!(md.contains("  return 2;"));
    assert!(!md.contains("return 99"));
}

#[test]
fn says_so_rather_than_emitting_an_empty_fence_for_a_blank_line() {
    let mut fixture = Fixture::new();
    let blank = ["function go() {", "", "}"];
    fixture.repo.write("src/go.ts", lines(&blank));
    fixture.rebuild();

    let on_blank = Comment {
        anchor: Some(create_anchor(Side::New, 2, 2, &owned(&blank))),
        ..comment()
    };
    let md = fixture.render(draft_of(vec![on_blank], ""));

    assert!(md.contains("_The anchored line is blank._"));
    assert!(!md.contains("```typescript\n\n```"));
}

#[test]
fn shortens_the_comment_id() {
    let fixture = Fixture::new();
    let uuid = Comment {
        id: "afed34c4-e52b-4fc4-9ec6-163ac37ebeae".into(),
        ..comment()
    };

    let md = fixture.render(draft_of(vec![uuid], ""));

    assert!(md.contains("`id afed34c4`"));
    assert!(!md.contains("163ac37ebeae"));
}

#[test]
fn lists_comments_whose_file_left_the_diff() {
    let fixture = Fixture::new();
    let elsewhere = Comment {
        path: "deleted/elsewhere.ts".into(),
        anchor: None,
        ..comment()
    };

    let md = fixture.render(draft_of(vec![elsewhere], ""));

    assert!(md.contains("## Comments on files no longer in the diff"));
    assert!(md.contains("`deleted/elsewhere.ts`"));
}

#[test]
fn lengthens_the_fence_when_the_quoted_code_contains_backticks() {
    let mut fixture = Fixture::new();
    let with_ticks = ["const s = `a ``` b`;"];
    fixture.repo.write("src/go.ts", lines(&with_ticks));
    fixture.rebuild();

    let on_ticks = Comment {
        anchor: Some(create_anchor(Side::New, 1, 1, &owned(&with_ticks))),
        ..comment()
    };
    let md = fixture.render(draft_of(vec![on_ticks], ""));

    assert!(md.contains("````typescript"));
}

#[test]
fn names_every_tag_it_can_emit() {
    let fixture = Fixture::new();

    let comments = COMMENT_TAGS
        .into_iter()
        .enumerate()
        .map(|(i, tag)| Comment {
            id: format!("t{i}"),
            tag,
            ..comment()
        })
        .collect();

    let md = fixture.render(draft_of(comments, ""));

    for heading in ["## Must fix", "## Questions", "## Nits", "## Ideas"] {
        assert!(md.contains(heading), "missing {heading}");
    }
}

fn reviews_dir(fixture: &Fixture) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(fixture.repo.store().join("reviews"))
        .expect("read reviews dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn submitting_fixture() -> Fixture {
    let fixture = Fixture::new();
    save_draft(
        fixture.repo.store(),
        &draft_of(vec![comment()], "overall thoughts"),
    );
    fixture
}

#[test]
fn writes_a_timestamped_pair_and_the_latest_pair() {
    let fixture = submitting_fixture();
    let result = fixture.submit();

    let files = reviews_dir(&fixture);

    assert!(files.iter().any(|f| f == "latest.md"));
    assert!(files.iter().any(|f| f == "latest.json"));
    assert!(files
        .iter()
        .any(|f| f.starts_with("review-") && f.ends_with(".md")));
    assert!(files
        .iter()
        .any(|f| f.starts_with("review-") && f.ends_with(".json")));
    assert!(std::path::Path::new(&result.latest_markdown_path).exists());
    assert!(!fixture.repo.path().join(".claude").exists());
}

#[test]
fn leaves_no_temp_files_behind() {
    let fixture = submitting_fixture();
    fixture.submit();

    assert!(!reviews_dir(&fixture).iter().any(|f| f.ends_with(".tmp")));
}

#[test]
fn writes_identical_content_to_the_archive_and_to_latest() {
    let fixture = submitting_fixture();
    let result = fixture.submit();

    let latest = std::fs::read_to_string(&result.latest_markdown_path).expect("latest");
    let archived = std::fs::read_to_string(&result.markdown_path).expect("archive");

    assert_eq!(latest, archived);
}

#[test]
fn clears_the_draft_so_the_next_review_starts_empty() {
    let fixture = submitting_fixture();
    fixture.submit();

    let after = load_draft(fixture.repo.store());

    assert!(after.comments.is_empty());
    assert_eq!(after.overall, "");
}

#[test]
fn keeps_the_full_draft_in_the_archived_json() {
    let fixture = submitting_fixture();
    let result = fixture.submit();

    let archived: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&result.json_path).expect("json"))
            .expect("parse archive");

    assert_eq!(archived["overall"], "overall thoughts");
    assert_eq!(archived["comments"].as_array().expect("comments").len(), 1);
    assert_eq!(archived["comments"][0]["tag"], "must-fix");
    assert_eq!(
        archived["comments"][0]["location"],
        "src/go.ts:2 (new side)"
    );
    assert_eq!(
        archived["comments"][0]["quote"],
        serde_json::json!(["  return 2;"])
    );
}

#[test]
fn reports_how_many_comments_it_wrote() {
    let fixture = submitting_fixture();
    assert_eq!(fixture.submit().comment_count, 1);
}

#[test]
fn keeps_its_own_output_out_of_the_next_review() {
    let mut fixture = submitting_fixture();
    fixture.submit();
    fixture.rebuild();

    let paths: Vec<&str> = fixture
        .model
        .files
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(paths, vec!["src/go.ts"]);
}
