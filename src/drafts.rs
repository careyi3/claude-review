use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use uuid::Uuid;

use crate::anchor::{create_anchor, hash_lines, reanchor};
use crate::clock::now_iso;
use crate::fsutil::write_atomic;
use crate::sides::{read_sides, side_lines};
use crate::types::{AnchorState, Comment, CommentAnchor, CommentTag, Draft, ReviewModel, Side};

pub const DRAFT_FILE: &str = "draft.json";

/// Where versions before sessions kept their draft, inside the repository.
pub const LEGACY_DRAFT_FILE: &str = ".draft.json";

/// `dir` is the session's own directory in the registry, not the repository.
pub fn draft_path(dir: &Path) -> PathBuf {
    dir.join(DRAFT_FILE)
}

pub fn legacy_review_dir(repo_root: &Path) -> PathBuf {
    repo_root.join(".claude").join("review")
}

pub fn empty_draft() -> Draft {
    Draft {
        version: 1,
        overall: String::new(),
        comments: Vec::new(),
        viewed: BTreeMap::new(),
        updated_at: now_iso(),
    }
}

fn as_string(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn as_string_or(value: Option<&Value>, fallback: &str) -> String {
    match value.and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => fallback.to_string(),
    }
}

fn as_line(value: Option<&Value>) -> Option<u32> {
    let n = value?.as_f64()?;
    if n.fract() != 0.0 || n < 1.0 {
        return None;
    }
    Some(n as u32)
}

fn as_hashes(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn as_viewed(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn normalize_anchor(value: Option<&Value>) -> Option<CommentAnchor> {
    let raw = value?.as_object()?;

    let side = if raw.get("side").and_then(Value::as_str) == Some("old") {
        Side::Old
    } else {
        Side::New
    };

    let start_line = as_line(raw.get("startLine"))?;
    let end_line = as_line(raw.get("endLine"))?;
    let line_hashes = as_hashes(raw.get("lineHashes"));

    if end_line < start_line || line_hashes.is_empty() {
        return None;
    }

    Some(CommentAnchor {
        side,
        start_line,
        end_line,
        line_hashes,
        line_texts: as_hashes(raw.get("lineTexts")),
        before_hashes: as_hashes(raw.get("beforeHashes")),
        after_hashes: as_hashes(raw.get("afterHashes")),
    })
}

fn normalize_comment(value: &Value) -> Option<Comment> {
    let raw = value.as_object()?;

    let path = as_string(raw.get("path"));
    let body = as_string(raw.get("body"));
    if path.is_empty() || body.trim().is_empty() {
        return None;
    }

    let now = now_iso();
    let id = match raw.get("id").and_then(Value::as_str) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => format!("c_{}", &Uuid::new_v4().simple().to_string()[..8]),
    };

    Some(Comment {
        id,
        path,
        anchor: normalize_anchor(raw.get("anchor")),
        tag: CommentTag::parse_or_question(raw.get("tag").and_then(Value::as_str)),
        body,
        created_at: as_string_or(raw.get("createdAt"), &now),
        updated_at: as_string_or(raw.get("updatedAt"), &now),
        state: None,
        resolved_start_line: None,
        resolved_end_line: None,
    })
}

pub fn normalize_draft(value: &Value) -> Draft {
    let empty = serde_json::Map::new();
    let raw = value.as_object().unwrap_or(&empty);

    let comments = raw
        .get("comments")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(normalize_comment).collect())
        .unwrap_or_default();

    Draft {
        version: 1,
        overall: as_string(raw.get("overall")),
        comments,
        viewed: as_viewed(raw.get("viewed")),
        updated_at: now_iso(),
    }
}

fn normalize(draft: &Draft) -> Draft {
    match serde_json::to_value(draft) {
        Ok(value) => normalize_draft(&value),
        Err(_) => empty_draft(),
    }
}

pub fn load_draft(dir: &Path) -> Draft {
    load_draft_file(&draft_path(dir))
}

pub fn load_draft_file(path: &Path) -> Draft {
    let Ok(text) = fs::read_to_string(path) else {
        return empty_draft();
    };

    match serde_json::from_str::<Value>(&text) {
        Ok(value) => normalize_draft(&value),
        Err(_) => empty_draft(),
    }
}

pub fn save_draft(dir: &Path, draft: &Draft) -> Draft {
    let normalized = normalize(draft);

    if let Ok(json) = serde_json::to_string_pretty(&normalized) {
        let _ = write_atomic(&draft_path(dir), &json);
    }

    normalized
}

pub fn resolve_draft(model: &ReviewModel, draft: &Draft) -> Draft {
    let root = Path::new(&model.repo_root);
    let mut cache: HashMap<(Side, String), Vec<String>> = HashMap::new();

    let mut resolved = draft.clone();

    for comment in &mut resolved.comments {
        let Some(anchor) = &comment.anchor else {
            comment.state = Some(AnchorState::Anchored);
            continue;
        };

        let side = anchor.side;
        let path = comment.path.clone();

        let hashes = cache
            .entry((side, path.clone()))
            .or_insert_with(|| match model.file(&path) {
                Some(file) => {
                    let sides = read_sides(root, model.base.sha.as_deref(), file);
                    hash_lines(side_lines(&sides, side))
                }
                None => Vec::new(),
            });

        let result = reanchor(anchor, hashes);

        comment.state = Some(result.state);
        comment.resolved_start_line = Some(result.start_line);
        comment.resolved_end_line = Some(result.end_line);
    }

    resolved
}

#[derive(Debug, Clone)]
pub struct NewComment {
    pub path: String,
    pub side: Option<Side>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub tag: CommentTag,
    pub body: String,
}

pub fn add_comment(dir: &Path, model: &ReviewModel, input: NewComment) -> Draft {
    let root = Path::new(&model.repo_root);
    let mut anchor = None;

    if let (Some(file), Some(side), Some(start), Some(end)) = (
        model.file(&input.path),
        input.side,
        input.start_line,
        input.end_line,
    ) {
        let sides = read_sides(root, model.base.sha.as_deref(), file);
        let lines = side_lines(&sides, side);

        if start >= 1 && end >= start && end as usize <= lines.len() {
            anchor = Some(create_anchor(side, start, end, lines));
        }
    }

    let now = now_iso();
    let comment = Comment {
        id: Uuid::new_v4().to_string(),
        path: input.path,
        anchor,
        tag: input.tag,
        body: input.body,
        created_at: now.clone(),
        updated_at: now,
        state: None,
        resolved_start_line: None,
        resolved_end_line: None,
    };

    let mut draft = load_draft(dir);
    draft.comments.push(comment);

    let saved = save_draft(dir, &draft);
    resolve_draft(model, &saved)
}

#[derive(Debug, Clone, Default)]
pub struct CommentPatch {
    pub tag: Option<CommentTag>,
    pub body: Option<String>,
}

pub fn patch_comment(dir: &Path, model: &ReviewModel, id: &str, patch: CommentPatch) -> Draft {
    let mut draft = load_draft(dir);

    for comment in &mut draft.comments {
        if comment.id != id {
            continue;
        }
        if let Some(tag) = patch.tag {
            comment.tag = tag;
        }
        if let Some(body) = &patch.body {
            comment.body = body.clone();
        }
        comment.updated_at = now_iso();
    }

    let saved = save_draft(dir, &draft);
    resolve_draft(model, &saved)
}

pub fn remove_comment(dir: &Path, model: &ReviewModel, id: &str) -> Draft {
    let mut draft = load_draft(dir);
    draft.comments.retain(|c| c.id != id);

    let saved = save_draft(dir, &draft);
    resolve_draft(model, &saved)
}

pub fn set_viewed(dir: &Path, model: &ReviewModel, path: &str, digest: Option<&str>) -> Draft {
    let mut draft = load_draft(dir);

    match digest {
        Some(digest) => {
            draft.viewed.insert(path.to_string(), digest.to_string());
        }
        None => {
            draft.viewed.remove(path);
        }
    }

    let saved = save_draft(dir, &draft);
    resolve_draft(model, &saved)
}

pub fn set_overall(dir: &Path, model: &ReviewModel, overall: &str) -> Draft {
    let mut draft = load_draft(dir);
    draft.overall = overall.to_string();

    let saved = save_draft(dir, &draft);
    resolve_draft(model, &saved)
}
