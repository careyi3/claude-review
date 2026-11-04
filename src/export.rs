use std::path::Path;

use serde::Serialize;

use crate::drafts::{empty_draft, save_draft};
use crate::fsutil::write_atomic;
use crate::sides::{read_sides, side_lines};
use crate::types::{
    AnchorState, Comment, CommentTag, Draft, ReviewBase, ReviewModel, Side, Totals, COMMENT_TAGS,
};

fn fence_for(code: &str) -> String {
    let mut longest = 0;
    let mut run = 0;

    for ch in code.chars() {
        if ch == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }

    "`".repeat((longest + 1).max(3))
}

fn code_block(lines: &[String], language: Option<&str>) -> String {
    let body = lines.join("\n");
    let fence = fence_for(&body);

    format!("{fence}{}\n{body}\n{fence}", language.unwrap_or(""))
}

fn timestamp_slug(iso: &str) -> String {
    let trimmed = match iso.strip_suffix('Z') {
        Some(head) => match head.rsplit_once('.') {
            Some((before, frac))
                if !frac.is_empty() && frac.chars().all(|c| c.is_ascii_digit()) =>
            {
                format!("{before}Z")
            }
            _ => iso.to_string(),
        },
        None => iso.to_string(),
    };

    trimmed.replace(':', "-")
}

fn quote_for(model: &ReviewModel, comment: &Comment) -> Vec<String> {
    let root = Path::new(&model.repo_root);
    let Some(anchor) = &comment.anchor else {
        return Vec::new();
    };

    let Some(file) = model.file(&comment.path) else {
        return anchor.line_texts.clone();
    };
    if comment.state == Some(AnchorState::Orphaned) {
        return anchor.line_texts.clone();
    }

    let start = comment.resolved_start_line.unwrap_or(anchor.start_line);
    let end = comment.resolved_end_line.unwrap_or(anchor.end_line);

    let sides = read_sides(root, model.base.sha.as_deref(), file);
    let lines = side_lines(&sides, anchor.side);

    let from = (start.saturating_sub(1) as usize).min(lines.len());
    let to = (end as usize).min(lines.len());

    let slice: &[String] = if from < to { &lines[from..to] } else { &[] };

    if slice.is_empty() {
        anchor.line_texts.clone()
    } else {
        slice.to_vec()
    }
}

fn short_id(id: &str) -> String {
    id.replace('-', "").chars().take(8).collect()
}

fn location_of(comment: &Comment) -> String {
    let Some(anchor) = &comment.anchor else {
        return format!("{} — whole file", comment.path);
    };

    let start = comment.resolved_start_line.unwrap_or(anchor.start_line);
    let end = comment.resolved_end_line.unwrap_or(anchor.end_line);

    let range = if start == end {
        start.to_string()
    } else {
        format!("{start}–{end}")
    };

    let side = match anchor.side {
        Side::Old => "old",
        Side::New => "new",
    };

    format!("{}:{range} ({side} side)", comment.path)
}

fn state_note(comment: &Comment) -> Option<&'static str> {
    match comment.state {
        Some(AnchorState::Moved) => {
            Some("the file changed after this was written; the comment followed its lines")
        }
        Some(AnchorState::Orphaned) => Some(
            "the lines this was written against are gone; the code below is what it originally pointed at",
        ),
        _ => None,
    }
}

fn plural(n: u32) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

pub fn render_markdown(model: &ReviewModel, draft: &Draft) -> String {
    let short_sha = match &model.base.sha {
        Some(sha) => sha.chars().take(7).collect::<String>(),
        None => "no commits".to_string(),
    };

    let counts: Vec<(CommentTag, Vec<&Comment>)> = COMMENT_TAGS
        .into_iter()
        .map(|tag| {
            let comments: Vec<&Comment> = draft.comments.iter().filter(|c| c.tag == tag).collect();
            (tag, comments)
        })
        .filter(|(_, comments)| !comments.is_empty())
        .collect();

    let mut out: Vec<String> = Vec::new();

    out.push(format!("# Code review — {}", draft.updated_at));
    out.push(String::new());
    out.push(format!("- Repository: `{}`", model.repo_root));
    out.push(format!(
        "- Reviewing: working tree against {} ({short_sha})",
        model.base.rev
    ));
    out.push(format!(
        "- Diff: {} file{} changed, +{} −{}",
        model.totals.files,
        plural(model.totals.files),
        model.totals.additions,
        model.totals.deletions,
    ));

    let summary = counts
        .iter()
        .map(|(tag, comments)| format!("{} {}", comments.len(), tag.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    out.push(format!(
        "- Comments: {}",
        if summary.is_empty() {
            "none"
        } else {
            summary.as_str()
        }
    ));
    out.push(String::new());

    out.push("## Overall".to_string());
    out.push(String::new());
    let overall = draft.overall.trim();
    out.push(if overall.is_empty() {
        "_No overall note._".to_string()
    } else {
        overall.to_string()
    });
    out.push(String::new());

    let mut index = 0;

    for (tag, comments) in &counts {
        out.push(format!("## {} ({})", tag.heading(), comments.len()));
        out.push(String::new());

        for comment in comments {
            index += 1;
            let file = model.file(&comment.path);

            out.push(format!("### {index}. {}", location_of(comment)));
            out.push(String::new());
            out.push(format!("`id {}`", short_id(&comment.id)));
            if let Some(note) = state_note(comment) {
                out.push(format!("\n> Note: {note}"));
            }
            out.push(String::new());

            let quote = quote_for(model, comment);
            if quote.is_empty() {
            } else if quote.iter().all(|l| l.trim().is_empty()) {
                out.push(
                    if quote.len() == 1 {
                        "_The anchored line is blank._"
                    } else {
                        "_The anchored lines are blank._"
                    }
                    .to_string(),
                );
                out.push(String::new());
            } else {
                out.push(code_block(&quote, file.and_then(|f| f.language.as_deref())));
                out.push(String::new());
            }

            out.push(comment.body.trim().to_string());
            out.push(String::new());
        }
    }

    let stale: Vec<&Comment> = draft
        .comments
        .iter()
        .filter(|c| model.file(&c.path).is_none())
        .collect();

    if !stale.is_empty() {
        out.push("## Comments on files no longer in the diff".to_string());
        out.push(String::new());
        out.push(
            "_These files were reverted, committed, or renamed after the comment was written._"
                .to_string(),
        );
        out.push(String::new());
        for comment in stale {
            out.push(format!(
                "- `{}` ({}): {}",
                comment.path,
                comment.tag.as_str(),
                comment.body.trim()
            ));
        }
        out.push(String::new());
    }

    out.join("\n")
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitResult {
    pub markdown_path: String,
    pub json_path: String,
    pub latest_markdown_path: String,
    pub comment_count: u32,
    pub markdown: String,
}

#[derive(Serialize)]
struct ExportedComment<'a> {
    #[serde(flatten)]
    comment: &'a Comment,
    location: String,
    quote: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportedReview<'a> {
    version: u32,
    generated_at: &'a str,
    base: &'a ReviewBase,
    repo_root: &'a str,
    totals: &'a Totals,
    overall: &'a str,
    comments: Vec<ExportedComment<'a>>,
}

pub const REVIEWS_DIR: &str = "reviews";
pub const LATEST_MARKDOWN: &str = "latest.md";

/// Writes the review into `<dir>/reviews/`, where `dir` is the session's
/// directory, and clears the session's draft.
pub fn submit_review(
    dir: &Path,
    model: &ReviewModel,
    draft: &Draft,
) -> std::io::Result<SubmitResult> {
    let reviews = dir.join(REVIEWS_DIR);
    std::fs::create_dir_all(&reviews)?;

    let markdown = render_markdown(model, draft);
    let slug = timestamp_slug(&draft.updated_at);

    let markdown_path = reviews.join(format!("review-{slug}.md"));
    let json_path = reviews.join(format!("review-{slug}.json"));
    let latest_markdown_path = reviews.join(LATEST_MARKDOWN);
    let latest_json_path = reviews.join("latest.json");

    let exported = ExportedReview {
        version: 1,
        generated_at: &draft.updated_at,
        base: &model.base,
        repo_root: &model.repo_root,
        totals: &model.totals,
        overall: &draft.overall,
        comments: draft
            .comments
            .iter()
            .map(|comment| ExportedComment {
                comment,
                location: location_of(comment),
                quote: quote_for(model, comment),
            })
            .collect(),
    };

    let json = serde_json::to_string_pretty(&exported)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;

    write_atomic(&markdown_path, &markdown)?;
    write_atomic(&json_path, &json)?;
    write_atomic(&latest_markdown_path, &markdown)?;
    write_atomic(&latest_json_path, &json)?;

    save_draft(dir, &empty_draft());

    Ok(SubmitResult {
        markdown_path: markdown_path.to_string_lossy().into_owned(),
        json_path: json_path.to_string_lossy().into_owned(),
        latest_markdown_path: latest_markdown_path.to_string_lossy().into_owned(),
        comment_count: draft.comments.len() as u32,
        markdown,
    })
}
