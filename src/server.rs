use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::{header, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use rand::RngCore;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use crate::clock::now_iso;
use crate::drafts::{
    add_comment, load_draft, patch_comment, remove_comment, resolve_draft, set_overall, set_viewed,
    CommentPatch, NewComment,
};
use crate::export::{render_markdown, submit_review};
use crate::git;
use crate::registry::{
    self, Creator, NewSession, Registry, Session, SessionPatch, SessionStatus, SessionSummary,
};
use crate::review::build_review;
use crate::statics::{read_static_fs, WebSource};
use crate::suggest::{claude_projects_dir, suggestions, Suggestion};
use crate::types::{CommentTag, ReviewModel, ReviewPayload, Side};
use crate::watch::{watch_repo, RepoWatcher, DEFAULT_DEBOUNCE};

pub struct ServerOptions {
    pub registry: Registry,
    pub host: IpAddr,
    pub port: u16,
    pub web: WebSource,
    /// Where to look for recent Claude Code projects. `None` turns that kind
    /// of suggestion off.
    pub claude_projects: Option<PathBuf>,
}

impl ServerOptions {
    pub fn new(registry: Registry) -> Self {
        Self {
            registry,
            host: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            web: WebSource::Embedded,
            claude_projects: claude_projects_dir(),
        }
    }
}

pub struct RunningServer {
    pub url: String,
    pub port: u16,
    pub token: String,
    shutdown: Option<oneshot::Sender<()>>,
    handle: JoinHandle<()>,
}

impl RunningServer {
    pub async fn close(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }

        if tokio::time::timeout(Duration::from_millis(250), &mut self.handle)
            .await
            .is_err()
        {
            self.handle.abort();
        }
    }
}

struct AppState {
    registry: Registry,
    token: String,
    web: WebSource,
    claude_projects: Option<PathBuf>,
    /// One per session the UI has subscribed to, created on first use.
    watchers: Mutex<HashMap<String, Arc<RepoWatcher>>>,
    /// Ticks when anything in the registry changes, including from the CLI.
    registry_watcher: RepoWatcher,
}

type Shared = Arc<AppState>;

/// What a session-scoped handler works with.
struct Context {
    session: Session,
    model: ReviewModel,
    /// The session's directory in the registry, where its draft lives.
    dir: PathBuf,
}

impl AppState {
    fn session(&self, id: &str) -> Result<Session, ApiError> {
        self.registry
            .get(id)
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, format!("no session `{id}`")))
    }

    fn context(&self, id: &str) -> Result<Context, ApiError> {
        let session = self.session(id)?;

        if !Path::new(&session.path).is_dir() {
            return Err(ApiError::new(
                StatusCode::GONE,
                format!("{} no longer exists", session.path),
            ));
        }

        let model = build_review(&session.build_options())?;
        let dir = self.registry.session_dir(&session.id);

        Ok(Context {
            session,
            model,
            dir,
        })
    }

    fn watcher(&self, session: &Session) -> Arc<RepoWatcher> {
        let mut watchers = self.watchers.lock().expect("watchers lock");
        watchers
            .entry(session.id.clone())
            .or_insert_with(|| Arc::new(watch_repo(Path::new(&session.path), DEFAULT_DEBOUNCE)))
            .clone()
    }
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
}

impl From<git::GitError> for ApiError {
    fn from(err: git::GitError) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, format!("git: {err}"))
    }
}

impl From<registry::Error> for ApiError {
    fn from(err: registry::Error) -> Self {
        let status = match err {
            registry::Error::NotFound(_) => StatusCode::NOT_FOUND,
            registry::Error::Ambiguous(..) | registry::Error::Invalid(_) => StatusCode::BAD_REQUEST,
            registry::Error::Git(_) => StatusCode::BAD_REQUEST,
            registry::Error::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::new(status, err.to_string())
    }
}

impl From<std::io::Error> for ApiError {
    fn from(err: std::io::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

async fn blocking<T, F>(f: F) -> Result<T, ApiError>
where
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(err) => Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("worker failed: {err}"),
        )),
    }
}

async fn require_token(
    State(state): State<Shared>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let from_header = request
        .headers()
        .get("x-review-token")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let presented = from_header.or_else(|| {
        request.uri().query().and_then(|query| {
            url_query_pairs(query)
                .into_iter()
                .find(|(key, _)| key == "token")
                .map(|(_, value)| value)
        })
    });

    if presented.as_deref() != Some(state.token.as_str()) {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "bad token"));
    }

    Ok(next.run(request).await)
}

fn url_query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let decode = |s: &str| {
                percent_encoding::percent_decode_str(&s.replace('+', " "))
                    .decode_utf8_lossy()
                    .into_owned()
            };
            (decode(key), decode(value))
        })
        .collect()
}

fn parse_json(body: &Bytes) -> Result<Value, ApiError> {
    serde_json::from_slice(body).map_err(|_| ApiError::bad_request("invalid JSON"))
}

fn string_field(body: &Value, key: &str) -> Option<String> {
    body.get(key).and_then(Value::as_str).map(str::to_string)
}

fn positive_field(body: &Value, key: &str) -> Option<u32> {
    let n = body.get(key)?.as_f64()?;
    if n.fract() != 0.0 || n < 1.0 {
        return None;
    }
    Some(n as u32)
}

fn side_field(body: &Value, key: &str) -> Option<Side> {
    match body.get(key).and_then(Value::as_str) {
        Some("old") => Some(Side::Old),
        Some("new") => Some(Side::New),
        _ => None,
    }
}

async fn get_review(
    State(state): State<Shared>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<ReviewPayload> {
    blocking(move || {
        let Context { model, dir, .. } = state.context(&id)?;
        let draft = resolve_draft(&model, &load_draft(&dir));

        Ok(Json(ReviewPayload { model, draft }))
    })
    .await
}

#[derive(Serialize)]
struct FileContent {
    path: String,
    side: &'static str,
    content: Option<String>,
}

async fn get_file(
    State(state): State<Shared>,
    UrlPath(id): UrlPath<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> ApiResult<FileContent> {
    let path = params.get("path").cloned().unwrap_or_default();
    if path.is_empty() {
        return Err(ApiError::bad_request("path is required"));
    }

    let side = match params.get("side").map(String::as_str) {
        Some("old") => Side::Old,
        _ => Side::New,
    };

    blocking(move || {
        let session = state.session(&id)?;
        let build = session.build_options();
        let root_string = git::repo_root(&build.cwd)?;
        let root = Path::new(&root_string);

        let content = match side {
            Side::Old => git::resolve_rev(root, &build.base)
                .and_then(|rev| git::read_blob(root, &rev, &path))
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
            Side::New => read_static_fs(root, &path)
                .map(|file| String::from_utf8_lossy(&file.body).into_owned()),
        };

        Ok(Json(FileContent {
            path,
            side: match side {
                Side::Old => "old",
                Side::New => "new",
            },
            content,
        }))
    })
    .await
}

async fn create_comment(
    State(state): State<Shared>,
    UrlPath(id): UrlPath<String>,
    body: Bytes,
) -> ApiResult<crate::types::Draft> {
    let body = parse_json(&body)?;

    let path = string_field(&body, "path").unwrap_or_default();
    let text = string_field(&body, "body").unwrap_or_default();
    if path.is_empty() || text.trim().is_empty() {
        return Err(ApiError::bad_request("path and body are required"));
    }

    let input = NewComment {
        path,
        side: side_field(&body, "side"),
        start_line: positive_field(&body, "startLine"),
        end_line: positive_field(&body, "endLine"),
        tag: CommentTag::parse_or_question(body.get("tag").and_then(Value::as_str)),
        body: text,
    };

    blocking(move || {
        let ctx = state.context(&id)?;
        Ok(Json(add_comment(&ctx.dir, &ctx.model, input)))
    })
    .await
}

async fn update_comment(
    State(state): State<Shared>,
    UrlPath((id, comment_id)): UrlPath<(String, String)>,
    body: Bytes,
) -> ApiResult<crate::types::Draft> {
    let body = parse_json(&body)?;

    let mut patch = CommentPatch::default();

    if !body.get("tag").unwrap_or(&Value::Null).is_null() {
        patch.tag = Some(CommentTag::parse_or_question(
            body.get("tag").and_then(Value::as_str),
        ));
    }

    if let Some(text) = string_field(&body, "body") {
        if text.trim().is_empty() {
            return Err(ApiError::bad_request("body cannot be empty"));
        }
        patch.body = Some(text);
    }

    blocking(move || {
        let ctx = state.context(&id)?;
        Ok(Json(patch_comment(
            &ctx.dir,
            &ctx.model,
            &comment_id,
            patch,
        )))
    })
    .await
}

async fn delete_comment(
    State(state): State<Shared>,
    UrlPath((id, comment_id)): UrlPath<(String, String)>,
) -> ApiResult<crate::types::Draft> {
    blocking(move || {
        let ctx = state.context(&id)?;
        Ok(Json(remove_comment(&ctx.dir, &ctx.model, &comment_id)))
    })
    .await
}

async fn put_viewed(
    State(state): State<Shared>,
    UrlPath(id): UrlPath<String>,
    body: Bytes,
) -> ApiResult<crate::types::Draft> {
    let body = parse_json(&body)?;

    let Some(path) = string_field(&body, "path") else {
        return Err(ApiError::bad_request("path is required"));
    };
    let digest = string_field(&body, "digest");

    blocking(move || {
        let ctx = state.context(&id)?;
        Ok(Json(set_viewed(
            &ctx.dir,
            &ctx.model,
            &path,
            digest.as_deref(),
        )))
    })
    .await
}

async fn put_overall(
    State(state): State<Shared>,
    UrlPath(id): UrlPath<String>,
    body: Bytes,
) -> ApiResult<crate::types::Draft> {
    let body = parse_json(&body)?;
    let overall = string_field(&body, "overall").unwrap_or_default();

    blocking(move || {
        let ctx = state.context(&id)?;
        Ok(Json(set_overall(&ctx.dir, &ctx.model, &overall)))
    })
    .await
}

#[derive(Serialize)]
struct Preview {
    markdown: String,
}

async fn get_preview(
    State(state): State<Shared>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Preview> {
    blocking(move || {
        let ctx = state.context(&id)?;
        let draft = resolve_draft(&ctx.model, &load_draft(&ctx.dir));

        Ok(Json(Preview {
            markdown: render_markdown(&ctx.model, &draft),
        }))
    })
    .await
}

async fn post_submit(
    State(state): State<Shared>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<crate::export::SubmitResult> {
    blocking(move || {
        let ctx = state.context(&id)?;
        let draft = resolve_draft(&ctx.model, &load_draft(&ctx.dir));

        if draft.comments.is_empty() && draft.overall.trim().is_empty() {
            return Err(ApiError::bad_request("nothing to submit"));
        }

        let result = submit_review(&ctx.dir, &ctx.model, &draft)?;
        state.registry.mark_submitted(&ctx.session.id)?;

        Ok(Json(result))
    })
    .await
}

fn tick_event(name: &'static str) -> Event {
    Event::default().event(name).data(now_iso())
}

/// Waits for the next registry or working-tree change.
async fn next_change(
    sessions: &mut watch::Receiver<u64>,
    tree: &mut Option<watch::Receiver<u64>>,
) -> Option<Event> {
    match tree {
        Some(tree) => tokio::select! {
            changed = sessions.changed() => changed.ok().map(|()| tick_event("sessions")),
            changed = tree.changed() => changed.ok().map(|()| tick_event("changed")),
        },
        None => sessions
            .changed()
            .await
            .ok()
            .map(|()| tick_event("sessions")),
    }
}

/// `sessions` whenever the registry changes; with `?session=<id>`, also
/// `changed` whenever that session's working tree does.
async fn get_events(
    State(state): State<Shared>,
    Query(params): Query<HashMap<String, String>>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let sessions = state.registry_watcher.subscribe();

    let watcher = params
        .get("session")
        .and_then(|id| state.registry.get(id))
        .filter(|session| Path::new(&session.path).is_dir())
        .map(|session| state.watcher(&session));

    let active = watcher.as_ref().is_some_and(|w| w.active());
    let tree = watcher.map(|w| w.subscribe());

    let stream = futures::stream::unfold(
        (Some(active), sessions, tree),
        |(pending_ready, mut sessions, mut tree)| async move {
            if let Some(active) = pending_ready {
                let event = Event::default().event("ready").data(active.to_string());
                return Some((Ok(event), (None, sessions, tree)));
            }

            let event = next_change(&mut sessions, &mut tree).await?;
            Some((Ok(event), (None, sessions, tree)))
        },
    );

    Sse::new(stream)
}

#[derive(Serialize)]
struct Health {
    ok: bool,
    pid: u32,
    version: &'static str,
}

async fn get_health() -> Json<Health> {
    Json(Health {
        ok: true,
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn list_sessions(
    State(state): State<Shared>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<Vec<SessionSummary>> {
    let all = params.get("all").is_some_and(|v| v == "1" || v == "true");

    blocking(move || {
        let summaries = state
            .registry
            .list()
            .into_iter()
            .filter(|s| all || s.status != SessionStatus::Closed)
            .map(|s| state.registry.summarize(s, None))
            .collect();
        Ok(Json(summaries))
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Created {
    session: SessionSummary,
    reused: bool,
    notes: Vec<String>,
}

async fn create_session(State(state): State<Shared>, body: Bytes) -> ApiResult<Created> {
    let body = parse_json(&body)?;

    let Some(path) = string_field(&body, "path").filter(|p| !p.trim().is_empty()) else {
        return Err(ApiError::bad_request("path is required"));
    };

    let input = NewSession {
        path: expand_home(path.trim()),
        name: string_field(&body, "name"),
        base: string_field(&body, "base"),
        include_untracked: body.get("includeUntracked").and_then(Value::as_bool),
        created_by: Some(Creator::User),
    };

    blocking(move || {
        if !input.path.is_dir() {
            return Err(ApiError::bad_request(format!(
                "{} is not a directory",
                input.path.display()
            )));
        }

        let registered = state.registry.register(input)?;
        Ok(Json(Created {
            session: state.registry.summarize(registered.session, None),
            reused: registered.reused,
            notes: registered.notes,
        }))
    })
    .await
}

fn expand_home(path: &str) -> PathBuf {
    match path
        .strip_prefix("~/")
        .or(if path == "~" { Some("") } else { None })
    {
        Some(rest) => dirs::home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    }
}

async fn get_session(
    State(state): State<Shared>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<SessionSummary> {
    blocking(move || {
        let session = state.session(&id)?;
        Ok(Json(state.registry.summarize(session, None)))
    })
    .await
}

async fn patch_session(
    State(state): State<Shared>,
    UrlPath(id): UrlPath<String>,
    body: Bytes,
) -> ApiResult<SessionSummary> {
    let body = parse_json(&body)?;

    let status = match body.get("status").and_then(Value::as_str) {
        None => None,
        // Submitted is only reached by submitting.
        Some(raw) => match SessionStatus::parse(raw) {
            Some(SessionStatus::Submitted) | None => {
                return Err(ApiError::bad_request("status must be open or closed"))
            }
            other => other,
        },
    };

    let patch = SessionPatch {
        name: string_field(&body, "name"),
        base: string_field(&body, "base"),
        include_untracked: body.get("includeUntracked").and_then(Value::as_bool),
        status,
    };

    blocking(move || {
        let session = state.session(&id)?;
        let updated = state.registry.update(&session.id, patch)?;
        Ok(Json(state.registry.summarize(updated, None)))
    })
    .await
}

async fn delete_session(
    State(state): State<Shared>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<SessionSummary> {
    blocking(move || {
        let session = state.session(&id)?;
        let closed = state.registry.close(&session.id)?;
        state
            .watchers
            .lock()
            .expect("watchers lock")
            .remove(&closed.id);
        Ok(Json(state.registry.summarize(closed, None)))
    })
    .await
}

async fn get_suggestions(State(state): State<Shared>) -> ApiResult<Vec<Suggestion>> {
    blocking(move || {
        Ok(Json(suggestions(
            &state.registry,
            state.claude_projects.as_deref(),
        )))
    })
    .await
}

fn bundle_headers(content_type: &'static str) -> [(header::HeaderName, &'static str); 2] {
    [
        (header::CONTENT_TYPE, content_type),
        (header::CACHE_CONTROL, "no-store"),
    ]
}

async fn spa(State(state): State<Shared>, uri: Uri) -> Response {
    let requested = uri.path();
    let target = if requested == "/" {
        "index.html"
    } else {
        requested
    };

    if let Some(file) = state.web.read(target) {
        return (bundle_headers(file.content_type), file.body).into_response();
    }

    match state.web.read("index.html") {
        Some(shell) => (bundle_headers(shell.content_type), shell.body).into_response(),
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "UI bundle is missing. Run `yarn --cwd web build`.",
        )
            .into_response(),
    }
}

fn router(state: Shared) -> Router {
    let session = Router::new()
        .route(
            "/",
            get(get_session).patch(patch_session).delete(delete_session),
        )
        .route("/review", get(get_review))
        .route("/file", get(get_file))
        .route("/comments", post(create_comment))
        .route(
            "/comments/{comment}",
            axum::routing::patch(update_comment).delete(delete_comment),
        )
        .route("/viewed", put(put_viewed))
        .route("/overall", put(put_overall))
        .route("/preview", get(get_preview))
        .route("/submit", post(post_submit));

    let api = Router::new()
        .route("/health", get(get_health))
        .route("/sessions", get(list_sessions).post(create_session))
        .route("/suggestions", get(get_suggestions))
        .route("/events", get(get_events))
        .nest("/sessions/{id}", session)
        .layer(middleware::from_fn_with_state(state.clone(), require_token));

    Router::new()
        .nest("/api", api)
        .fallback(spa)
        .with_state(state)
}

fn session_token() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub async fn start_server(opts: ServerOptions) -> anyhow::Result<RunningServer> {
    let token = session_token();

    let sessions_dir = opts.registry.sessions_dir();
    std::fs::create_dir_all(&sessions_dir)?;
    let registry_watcher = watch_repo(&sessions_dir, DEFAULT_DEBOUNCE);

    let state = Arc::new(AppState {
        registry: opts.registry,
        token: token.clone(),
        web: opts.web,
        claude_projects: opts.claude_projects,
        watchers: Mutex::new(HashMap::new()),
        registry_watcher,
    });

    let listener = TcpListener::bind(SocketAddr::new(opts.host, opts.port)).await?;
    let port = listener.local_addr()?.port();

    let app = router(state);
    let (shutdown, shutdown_rx) = oneshot::channel();

    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });

    Ok(RunningServer {
        url: format!("http://{}:{port}/?token={token}", opts.host),
        port,
        token,
        shutdown: Some(shutdown),
        handle,
    })
}
