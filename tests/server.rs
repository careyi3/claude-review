mod common;

use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

use claude_review::registry::{NewSession, Registry};
use claude_review::server::{start_server, RunningServer, ServerOptions};
use claude_review::statics::WebSource;
use common::{lines, Repo};
use futures_util::StreamExt;
use reqwest::{Client, Response, StatusCode};
use serde_json::{json, Value};
use tempfile::TempDir;

struct Harness {
    repo: Repo,
    registry: Registry,
    session: String,
    server: RunningServer,
    client: Client,
    _web: TempDir,
    _home: TempDir,
}

/// Routes that belong to one session, under `/api/sessions/<id>/`.
const SESSION_ROUTES: [&str; 7] = [
    "review", "file", "comments", "viewed", "overall", "preview", "submit",
];

impl Harness {
    async fn new() -> Self {
        let repo = Repo::new();
        repo.write("a.txt", lines(&["alpha", "beta"]));
        repo.commit("base");
        repo.write("a.txt", lines(&["alpha", "BETA"]));

        let web = TempDir::with_prefix("claude-review-web-").expect("temp web root");
        std::fs::write(
            web.path().join("index.html"),
            "<!doctype html><title>shell</title>",
        )
        .expect("write shell");
        std::fs::write(web.path().join("app.js"), "console.log(1)").expect("write asset");

        let home = TempDir::with_prefix("claude-review-registry-").expect("temp registry");
        let registry = Registry::new(home.path());
        let session = registry
            .register(NewSession {
                path: repo.path().to_path_buf(),
                base: Some("HEAD".into()),
                ..NewSession::default()
            })
            .expect("register session")
            .session
            .id;

        let server = start_server(ServerOptions {
            registry: registry.clone(),
            host: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            web: WebSource::Directory(web.path().to_path_buf()),
            claude_projects: None,
        })
        .await
        .expect("start server");

        Self {
            repo,
            registry,
            session,
            server,
            client: Client::new(),
            _web: web,
            _home: home,
        }
    }

    /// Rewrites `/api/review` and friends to this harness's session.
    fn url(&self, path: &str) -> String {
        let scoped = path.strip_prefix("/api/").and_then(|rest| {
            let first = rest.split(['/', '?']).next().unwrap_or("");
            SESSION_ROUTES
                .contains(&first)
                .then(|| format!("/api/sessions/{}/{rest}", self.session))
        });

        let path = scoped.unwrap_or_else(|| path.to_string());
        format!("http://127.0.0.1:{}{path}", self.server.port)
    }

    async fn get(&self, path: &str) -> Response {
        self.client
            .get(self.url(path))
            .header("x-review-token", &self.server.token)
            .send()
            .await
            .expect("send request")
    }

    async fn get_json(&self, path: &str) -> Value {
        self.get(path).await.json().await.expect("json body")
    }

    async fn send(&self, method: reqwest::Method, path: &str, body: Value) -> Response {
        self.client
            .request(method, self.url(path))
            .header("x-review-token", &self.server.token)
            .json(&body)
            .send()
            .await
            .expect("send request")
    }

    async fn review(&self) -> Value {
        self.get_json("/api/review").await
    }

    async fn draft(&self) -> Value {
        self.review().await["draft"].clone()
    }

    async fn add_comment(&self, overrides: Value) -> (StatusCode, Value) {
        let mut body = json!({
            "path": "a.txt",
            "side": "new",
            "startLine": 2,
            "endLine": 2,
            "tag": "must-fix",
            "body": "why shout?",
        });

        for (key, value) in overrides.as_object().expect("object of overrides") {
            body[key] = value.clone();
        }

        let res = self
            .send(reqwest::Method::POST, "/api/comments", body)
            .await;
        let status = res.status();

        (status, res.json().await.unwrap_or(Value::Null))
    }

    async fn close(self) {
        self.server.close().await;
    }
}

fn paths(model: &Value) -> Vec<&str> {
    model["files"]
        .as_array()
        .expect("files")
        .iter()
        .map(|f| f["path"].as_str().expect("path"))
        .collect()
}

#[tokio::test]
async fn rejects_api_requests_with_no_token() {
    let h = Harness::new().await;

    let res = h
        .client
        .get(h.url("/api/review"))
        .send()
        .await
        .expect("send");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    h.close().await;
}

#[tokio::test]
async fn rejects_api_requests_with_the_wrong_token() {
    let h = Harness::new().await;

    let res = h
        .client
        .get(h.url("/api/review"))
        .header("x-review-token", "nope")
        .send()
        .await
        .expect("send");

    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    h.close().await;
}

#[tokio::test]
async fn accepts_the_token_as_a_query_parameter() {
    let h = Harness::new().await;

    let path = format!("/api/review?token={}", h.server.token);
    let res = h.client.get(h.url(&path)).send().await.expect("send");

    assert_eq!(res.status(), StatusCode::OK);

    h.close().await;
}

#[tokio::test]
async fn requires_a_token_to_write_comments() {
    let h = Harness::new().await;

    let res = h
        .client
        .post(h.url("/api/comments"))
        .json(&json!({ "path": "a.txt", "body": "x" }))
        .send()
        .await
        .expect("send");

    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    h.close().await;
}

#[tokio::test]
async fn serves_the_review_model() {
    let h = Harness::new().await;
    let payload = h.review().await;

    assert_eq!(paths(&payload["model"]), vec!["a.txt"]);
    assert_eq!(payload["model"]["totals"]["additions"], 1);
    assert_eq!(payload["model"]["totals"]["deletions"], 1);

    h.close().await;
}

#[tokio::test]
async fn serves_an_empty_draft_alongside_the_model() {
    let h = Harness::new().await;
    let draft = h.draft().await;

    assert_eq!(draft["version"], 1);
    assert_eq!(draft["overall"], "");
    assert!(draft["comments"].as_array().expect("comments").is_empty());

    h.close().await;
}

#[tokio::test]
async fn reflects_edits_made_after_the_server_started() {
    let h = Harness::new().await;
    h.repo.write("b.txt", lines(&["added later"]));

    let payload = h.review().await;
    assert_eq!(paths(&payload["model"]), vec!["a.txt", "b.txt"]);

    h.close().await;
}

#[tokio::test]
async fn creates_a_comment_and_anchors_it_server_side() {
    let h = Harness::new().await;
    let (_, draft) = h.add_comment(json!({})).await;

    let only = &draft["comments"][0];

    assert_eq!(only["path"], "a.txt");
    assert_eq!(only["tag"], "must-fix");
    assert_eq!(only["state"], "anchored");
    assert_eq!(
        only["anchor"]["lineHashes"]
            .as_array()
            .expect("hashes")
            .len(),
        1
    );
    assert_eq!(only["resolvedStartLine"], 2);
    assert!(only["id"].as_str().is_some_and(|id| !id.is_empty()));

    h.close().await;
}

#[tokio::test]
async fn persists_comments_across_requests() {
    let h = Harness::new().await;
    h.add_comment(json!({})).await;

    let draft = h.draft().await;

    assert_eq!(draft["comments"].as_array().expect("comments").len(), 1);
    assert_eq!(draft["comments"][0]["body"], "why shout?");

    h.close().await;
}

#[tokio::test]
async fn moves_a_comment_when_the_file_is_edited_underneath_it() {
    let h = Harness::new().await;
    h.add_comment(json!({})).await;
    h.repo.write("a.txt", lines(&["inserted", "alpha", "BETA"]));

    let draft = h.draft().await;

    assert_eq!(draft["comments"][0]["state"], "moved");
    assert_eq!(draft["comments"][0]["resolvedStartLine"], 3);

    h.close().await;
}

#[tokio::test]
async fn orphans_a_comment_when_its_line_is_rewritten() {
    let h = Harness::new().await;
    h.add_comment(json!({})).await;
    h.repo
        .write("a.txt", lines(&["alpha", "something else entirely"]));

    assert_eq!(h.draft().await["comments"][0]["state"], "orphaned");

    h.close().await;
}

#[tokio::test]
async fn creates_a_file_level_comment_when_no_side_is_given() {
    let h = Harness::new().await;
    let (_, draft) = h
        .add_comment(json!({ "side": null, "startLine": null, "endLine": null }))
        .await;

    assert_eq!(draft["comments"][0]["anchor"], Value::Null);
    assert_eq!(draft["comments"][0]["state"], "anchored");

    h.close().await;
}

#[tokio::test]
async fn leaves_the_anchor_off_when_the_line_is_past_the_end_of_the_file() {
    let h = Harness::new().await;
    let (_, draft) = h
        .add_comment(json!({ "startLine": 9999, "endLine": 9999 }))
        .await;

    assert_eq!(draft["comments"][0]["anchor"], Value::Null);

    h.close().await;
}

#[tokio::test]
async fn falls_back_to_the_question_tag_for_an_unknown_tag() {
    let h = Harness::new().await;
    let (_, draft) = h.add_comment(json!({ "tag": "blocker" })).await;

    assert_eq!(draft["comments"][0]["tag"], "question");

    h.close().await;
}

#[tokio::test]
async fn rejects_a_comment_with_an_empty_body() {
    let h = Harness::new().await;
    let (status, _) = h.add_comment(json!({ "body": "   " })).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);

    h.close().await;
}

#[tokio::test]
async fn rejects_a_comment_with_no_path() {
    let h = Harness::new().await;
    let (status, _) = h.add_comment(json!({ "path": "" })).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);

    h.close().await;
}

#[tokio::test]
async fn rejects_comment_writes_that_are_not_json() {
    let h = Harness::new().await;

    let res = h
        .client
        .post(h.url("/api/comments"))
        .header("x-review-token", &h.server.token)
        .header("content-type", "application/json")
        .body("not json")
        .send()
        .await
        .expect("send");

    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    h.close().await;
}

#[tokio::test]
async fn edits_a_comment() {
    let h = Harness::new().await;
    let (_, draft) = h.add_comment(json!({})).await;

    let id = draft["comments"][0]["id"].as_str().expect("id").to_string();
    let created_at = draft["comments"][0]["createdAt"].clone();

    let res = h
        .send(
            reqwest::Method::PATCH,
            &format!("/api/comments/{id}"),
            json!({ "body": "rephrased", "tag": "nit" }),
        )
        .await;

    let updated: Value = res.json().await.expect("json");

    assert_eq!(updated["comments"][0]["body"], "rephrased");
    assert_eq!(updated["comments"][0]["tag"], "nit");
    assert_eq!(updated["comments"][0]["createdAt"], created_at);

    h.close().await;
}

#[tokio::test]
async fn refuses_to_blank_out_a_comment_body() {
    let h = Harness::new().await;
    let (_, draft) = h.add_comment(json!({})).await;
    let id = draft["comments"][0]["id"].as_str().expect("id").to_string();

    let res = h
        .send(
            reqwest::Method::PATCH,
            &format!("/api/comments/{id}"),
            json!({ "body": "  " }),
        )
        .await;

    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    h.close().await;
}

#[tokio::test]
async fn deletes_a_comment() {
    let h = Harness::new().await;
    let (_, draft) = h.add_comment(json!({})).await;
    let id = draft["comments"][0]["id"].as_str().expect("id").to_string();

    let res = h
        .send(
            reqwest::Method::DELETE,
            &format!("/api/comments/{id}"),
            json!({}),
        )
        .await;

    let after: Value = res.json().await.expect("json");
    assert!(after["comments"].as_array().expect("comments").is_empty());

    h.close().await;
}

#[tokio::test]
async fn ignores_a_delete_for_an_unknown_id() {
    let h = Harness::new().await;
    h.add_comment(json!({})).await;

    let res = h
        .send(reqwest::Method::DELETE, "/api/comments/nope", json!({}))
        .await;

    let after: Value = res.json().await.expect("json");
    assert_eq!(after["comments"].as_array().expect("comments").len(), 1);

    h.close().await;
}

#[tokio::test]
async fn stores_the_overall_note() {
    let h = Harness::new().await;

    h.send(
        reqwest::Method::PUT,
        "/api/overall",
        json!({ "overall": "solid, two nits" }),
    )
    .await;

    assert_eq!(h.draft().await["overall"], "solid, two nits");

    h.close().await;
}

#[tokio::test]
async fn keeps_the_saved_draft_out_of_the_diff_it_describes() {
    let h = Harness::new().await;
    h.add_comment(json!({})).await;

    assert_eq!(paths(&h.review().await["model"]), vec!["a.txt"]);

    h.close().await;
}

#[tokio::test]
async fn refuses_to_submit_an_empty_review() {
    let h = Harness::new().await;

    let res = h
        .send(reqwest::Method::POST, "/api/submit", json!({}))
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    h.close().await;
}

#[tokio::test]
async fn previews_without_writing_anything() {
    let h = Harness::new().await;
    h.add_comment(json!({})).await;

    let preview = h.get_json("/api/preview").await;
    assert!(preview["markdown"]
        .as_str()
        .expect("markdown")
        .contains("why shout?"));

    assert_eq!(
        h.draft().await["comments"]
            .as_array()
            .expect("comments")
            .len(),
        1
    );

    h.close().await;
}

#[tokio::test]
async fn submits_writes_the_review_and_empties_the_draft() {
    let h = Harness::new().await;
    h.add_comment(json!({})).await;

    let res = h
        .send(reqwest::Method::POST, "/api/submit", json!({}))
        .await;
    let result: Value = res.json().await.expect("json");

    assert_eq!(result["commentCount"], 1);

    let written =
        std::fs::read_to_string(result["latestMarkdownPath"].as_str().expect("latest path"))
            .expect("read latest");
    assert!(written.contains("why shout?"));

    assert!(h.draft().await["comments"]
        .as_array()
        .expect("comments")
        .is_empty());

    h.close().await;
}

#[tokio::test]
async fn returns_file_contents_for_both_sides() {
    let h = Harness::new().await;

    let old = h.get_json("/api/file?path=a.txt&side=old").await;
    let now = h.get_json("/api/file?path=a.txt&side=new").await;

    assert_eq!(old["content"], "alpha\nbeta\n");
    assert_eq!(now["content"], "alpha\nBETA\n");

    h.close().await;
}

#[tokio::test]
async fn returns_file_contents_for_a_nested_path() {
    let h = Harness::new().await;
    h.repo.write("src/deep/nested.ts", "export const x = 1;\n");

    let body = h
        .get_json("/api/file?path=src/deep/nested.ts&side=new")
        .await;

    assert_eq!(body["content"], "export const x = 1;\n");

    h.close().await;
}

#[tokio::test]
async fn serves_json_not_the_spa_shell_from_api_file() {
    let h = Harness::new().await;
    let res = h.get("/api/file?path=a.txt&side=new").await;

    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();

    assert!(
        content_type.contains("application/json"),
        "got {content_type}"
    );

    h.close().await;
}

#[tokio::test]
async fn refuses_to_read_outside_the_repository() {
    let h = Harness::new().await;

    let body = h
        .get_json("/api/file?path=../../../../etc/passwd&side=new")
        .await;

    assert_eq!(body["content"], Value::Null);

    h.close().await;
}

#[tokio::test]
async fn requires_a_path() {
    let h = Harness::new().await;

    assert_eq!(
        h.get("/api/file?side=new").await.status(),
        StatusCode::BAD_REQUEST
    );

    h.close().await;
}

#[tokio::test]
async fn marks_a_file_viewed_and_unmarks_it() {
    let h = Harness::new().await;

    let payload = h.review().await;
    let digest = payload["model"]["files"][0]["digest"].clone();

    let marked: Value = h
        .send(
            reqwest::Method::PUT,
            "/api/viewed",
            json!({ "path": "a.txt", "digest": digest }),
        )
        .await
        .json()
        .await
        .expect("json");

    assert_eq!(marked["viewed"]["a.txt"], digest);

    let cleared: Value = h
        .send(
            reqwest::Method::PUT,
            "/api/viewed",
            json!({ "path": "a.txt", "digest": null }),
        )
        .await
        .json()
        .await
        .expect("json");

    assert_eq!(cleared["viewed"], json!({}));

    h.close().await;
}

#[tokio::test]
async fn requires_a_path_to_mark_viewed() {
    let h = Harness::new().await;

    let res = h
        .send(
            reqwest::Method::PUT,
            "/api/viewed",
            json!({ "digest": "x" }),
        )
        .await;

    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    h.close().await;
}

#[tokio::test]
async fn serves_static_assets_and_falls_back_to_the_shell() {
    let h = Harness::new().await;

    let asset = h.client.get(h.url("/app.js")).send().await.expect("send");
    assert_eq!(asset.status(), StatusCode::OK);
    assert!(asset
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .contains("text/javascript"));

    let unknown = h
        .client
        .get(h.url("/some/client/route"))
        .send()
        .await
        .expect("send");
    assert!(unknown.text().await.expect("body").contains("shell"));

    h.close().await;
}

#[tokio::test]
async fn does_not_serve_files_outside_the_web_root() {
    let h = Harness::new().await;

    let res = h
        .client
        .get(h.url("/%2e%2e%2f%2e%2e%2fa.txt"))
        .send()
        .await
        .expect("send");

    assert!(res.text().await.expect("body").contains("shell"));

    h.close().await;
}

async fn read_until<S, B, E>(
    stream: &mut S,
    buffered: &mut String,
    needle: &str,
    budget: Duration,
) -> bool
where
    S: futures_util::Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
{
    if buffered.contains(needle) {
        return true;
    }

    let deadline = Instant::now() + budget;

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());

        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                buffered.push_str(&String::from_utf8_lossy(chunk.as_ref()));
                if buffered.contains(needle) {
                    return true;
                }
            }
            _ => return false,
        }
    }

    false
}

#[tokio::test(flavor = "multi_thread")]
async fn announces_working_tree_changes_over_sse() {
    let h = Harness::new().await;

    let res = h
        .client
        .get(h.url(&format!(
            "/api/events?token={}&session={}",
            h.server.token, h.session
        )))
        .send()
        .await
        .expect("send");

    assert!(res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .contains("text/event-stream"));

    let mut stream = res.bytes_stream();
    let mut buffered = String::new();

    assert!(
        read_until(
            &mut stream,
            &mut buffered,
            "event: ready",
            Duration::from_secs(3)
        )
        .await
    );

    h.repo.write("a.txt", lines(&["alpha", "CHANGED AGAIN"]));
    let saw_change = read_until(
        &mut stream,
        &mut buffered,
        "event: changed",
        Duration::from_secs(8),
    )
    .await;

    assert!(saw_change, "no change event arrived");

    h.close().await;
}

#[tokio::test]
async fn reports_health() {
    let h = Harness::new().await;
    let health = h.get_json("/api/health").await;

    assert_eq!(health["ok"], true);
    assert_eq!(health["pid"], std::process::id());

    h.close().await;
}

#[tokio::test]
async fn lists_sessions() {
    let h = Harness::new().await;
    let sessions = h.get_json("/api/sessions").await;

    let list = sessions.as_array().expect("array");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], h.session.as_str());
    assert_eq!(list[0]["branch"], "main");
    assert_eq!(list[0]["missing"], false);

    h.close().await;
}

#[tokio::test]
async fn creates_a_session_for_another_repository() {
    let h = Harness::new().await;
    let other = Repo::new();
    other.write("x.txt", "x\n");
    other.commit("base");

    let res = h
        .send(
            reqwest::Method::POST,
            "/api/sessions",
            json!({ "path": other.path(), "name": "other one" }),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let created: Value = res.json().await.expect("json");

    assert_eq!(created["reused"], false);
    assert_eq!(created["session"]["name"], "other one");
    assert_eq!(created["session"]["createdBy"], "user");
    assert_eq!(
        h.get_json("/api/sessions").await.as_array().unwrap().len(),
        2
    );

    let again = h
        .send(
            reqwest::Method::POST,
            "/api/sessions",
            json!({ "path": other.path() }),
        )
        .await;
    assert_eq!(again.json::<Value>().await.unwrap()["reused"], true);

    h.close().await;
}

#[tokio::test]
async fn refuses_to_create_a_session_for_a_missing_directory() {
    let h = Harness::new().await;

    let res = h
        .send(
            reqwest::Method::POST,
            "/api/sessions",
            json!({ "path": "/definitely/not/here" }),
        )
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    h.close().await;
}

#[tokio::test]
async fn answers_404_for_an_unknown_session() {
    let h = Harness::new().await;

    let res = h.get("/api/sessions/nope1234/review").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    h.close().await;
}

#[tokio::test]
async fn renames_a_session_and_refuses_to_mark_it_submitted() {
    let h = Harness::new().await;
    let path = format!("/api/sessions/{}", h.session);

    let renamed: Value = h
        .send(
            reqwest::Method::PATCH,
            &path,
            json!({ "name": "fresh name" }),
        )
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(renamed["name"], "fresh name");

    let res = h
        .send(
            reqwest::Method::PATCH,
            &path,
            json!({ "status": "submitted" }),
        )
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    h.close().await;
}

#[tokio::test]
async fn closing_a_session_hides_it_from_the_list() {
    let h = Harness::new().await;

    let res = h
        .send(
            reqwest::Method::DELETE,
            &format!("/api/sessions/{}", h.session),
            json!({}),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    assert!(h
        .get_json("/api/sessions")
        .await
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        h.get_json("/api/sessions?all=1")
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );

    h.close().await;
}

#[tokio::test]
async fn submitting_marks_the_session_submitted_for_claude() {
    let h = Harness::new().await;
    h.add_comment(json!({})).await;

    h.send(reqwest::Method::POST, "/api/submit", json!({}))
        .await;

    let session = h.registry.get(&h.session).expect("session");
    assert_eq!(
        session.status,
        claude_review::registry::SessionStatus::Submitted
    );
    assert_eq!(session.rounds, 1);

    let (_, markdown) = h.registry.take(&h.session).expect("take");
    assert!(markdown.contains("why shout?"));
    assert!(
        !h.repo.path().join(".claude").exists(),
        "nothing written into the repo"
    );

    h.close().await;
}

#[tokio::test]
async fn answers_410_once_the_worktree_is_gone() {
    let h = Harness::new().await;
    let doomed = Repo::new();
    doomed.write("x.txt", "x\n");
    doomed.commit("base");

    let id = h
        .registry
        .register(NewSession {
            path: doomed.path().to_path_buf(),
            ..NewSession::default()
        })
        .expect("register")
        .session
        .id;
    std::fs::remove_dir_all(doomed.path()).expect("remove worktree");

    let res = h.get(&format!("/api/sessions/{id}/review")).await;
    assert_eq!(res.status(), StatusCode::GONE);

    h.close().await;
}

#[tokio::test]
async fn suggests_worktrees_of_known_repositories() {
    let h = Harness::new().await;
    let trees = TempDir::with_prefix("claude-review-trees-").expect("temp dir");
    let feature = trees.path().join("feature");
    h.repo.git(&[
        "worktree",
        "add",
        "-b",
        "feature",
        &feature.to_string_lossy(),
    ]);

    let suggestions = h.get_json("/api/suggestions").await;
    let list = suggestions.as_array().expect("array");
    let canonical = std::fs::canonicalize(&feature).unwrap();

    assert_eq!(list.len(), 1, "the session's own worktree is not suggested");
    assert_eq!(list[0]["path"], canonical.to_string_lossy().as_ref());
    assert_eq!(list[0]["source"], "worktree");
    assert_eq!(list[0]["branch"], "feature");

    h.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn announces_registry_changes_over_sse() {
    let h = Harness::new().await;

    let res = h
        .client
        .get(h.url(&format!("/api/events?token={}", h.server.token)))
        .send()
        .await
        .expect("send");
    let mut stream = res.bytes_stream();
    let mut buffered = String::new();

    assert!(
        read_until(
            &mut stream,
            &mut buffered,
            "event: ready",
            Duration::from_secs(3)
        )
        .await
    );

    h.registry
        .mark_submitted(&h.session)
        .expect("touch registry");

    assert!(
        read_until(
            &mut stream,
            &mut buffered,
            "event: sessions",
            Duration::from_secs(8)
        )
        .await,
        "no sessions event arrived"
    );

    h.close().await;
}
