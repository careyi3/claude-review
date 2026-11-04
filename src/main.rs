use std::io::{Read, Write as _};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};

use claude_review::clock::now_iso;
use claude_review::install::{commands_dir, install_commands, stale_commands, Outcome};
use claude_review::open::open_browser;
use claude_review::registry::{
    resolve_base, Creator, NewSession, Place, Registered, Registry, ServerInfo, Session,
    SessionStatus, SessionSummary,
};
use claude_review::review::{build_review, BuildOptions};
use claude_review::server::{start_server, ServerOptions};
use claude_review::statics::WebSource;
use claude_review::suggest::claude_projects_dir;
use claude_review::types::ReviewModel;

const AFTER_HELP: &str = "\
Run it anywhere to open the app. Run inside a repository, it also opens (or
creates) that worktree's session. Review, comment, press Submit review, then
run /review-feedback in Claude Code.

Sessions live in ~/.claude-review, or $CLAUDE_REVIEW_HOME if set.";

#[derive(Parser)]
#[command(
    name = "claude-review",
    version,
    about = "Review working-tree changes and hand comments back to Claude",
    after_help = AFTER_HELP,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    app: AppArgs,
}

#[derive(Subcommand)]
enum Command {
    /// Create a session for a worktree, or reuse the open one it already has
    Register {
        /// Any directory inside the worktree
        #[arg(long, value_name = "path", default_value = ".")]
        path: PathBuf,

        #[arg(long, value_name = "name")]
        name: Option<String>,

        /// Revision to compare against [default: auto, the merge-base with the
        /// default branch]
        #[arg(long, value_name = "rev")]
        base: Option<String>,

        /// Who is registering it
        #[arg(long, value_name = "user|claude", default_value = "user")]
        by: String,

        #[arg(long = "no-untracked")]
        no_untracked: bool,

        #[arg(long)]
        json: bool,
    },

    /// List sessions
    Sessions {
        /// Only sessions in this state: open, submitted or closed
        #[arg(long, value_name = "status")]
        status: Option<String>,

        /// Include closed sessions
        #[arg(long)]
        all: bool,

        /// Directory to report each session's relation to [default: current]
        #[arg(long, value_name = "path")]
        near: Option<PathBuf>,

        #[arg(long)]
        json: bool,
    },

    /// Print a session's latest submitted review, changing nothing
    Show { id: String },

    /// Print a session's waiting review and mark it picked up
    Take { id: String },

    /// Close a session so it is no longer listed
    Close { id: String },

    /// Install the /review-start and /review-feedback commands for Claude Code
    #[command(name = "install-command")]
    Install {
        /// Overwrite copies you have edited
        #[arg(long)]
        force: bool,
    },
}

#[derive(Args)]
struct AppArgs {
    /// Revision to compare against, when creating this worktree's session
    /// [default: auto, the merge-base with the default branch]
    #[arg(long, value_name = "rev")]
    base: Option<String>,

    #[arg(long = "no-untracked")]
    no_untracked: bool,

    /// Context lines around each hunk, for --summary and --json
    #[arg(long, value_name = "n", default_value_t = 3)]
    context: u32,

    /// Skip diffing files larger than this, for --summary and --json
    #[arg(long = "max-bytes", value_name = "n", default_value_t = 1_000_000)]
    max_bytes: u64,

    #[arg(long, value_name = "path")]
    cwd: Option<PathBuf>,

    /// Open this session instead of the current worktree's
    #[arg(long, value_name = "id")]
    session: Option<String>,

    /// Open the app without creating a session for the current worktree
    #[arg(long = "no-session")]
    no_session: bool,

    #[arg(long, value_name = "n", default_value_t = 0)]
    port: u16,

    #[arg(long = "no-open")]
    no_open: bool,

    #[arg(long)]
    summary: bool,

    #[arg(long)]
    json: bool,

    #[arg(long = "web-root", value_name = "path")]
    web_root: Option<PathBuf>,
}

impl AppArgs {
    fn cwd(&self) -> PathBuf {
        self.cwd
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    fn build_options(&self) -> anyhow::Result<BuildOptions> {
        let cwd = self.cwd();
        let root = PathBuf::from(claude_review::git::repo_root(&cwd)?);
        let spec = self
            .base
            .as_deref()
            .unwrap_or(claude_review::registry::AUTO_BASE);
        let (base, base_label) = resolve_base(&root, spec);

        Ok(BuildOptions {
            cwd,
            base,
            base_label,
            include_untracked: !self.no_untracked,
            context_lines: self.context,
            max_file_bytes: self.max_bytes,
        })
    }
}

fn print_summary(model: &ReviewModel) {
    let short_sha = match &model.base.sha {
        Some(sha) => sha.chars().take(7).collect::<String>(),
        None => "no commits".to_string(),
    };

    if model.files.is_empty() {
        println!("No changes against {} ({short_sha}).", model.base.rev);
        return;
    }

    let named: Vec<(&_, String)> = model
        .files
        .iter()
        .map(|f| {
            let name = match &f.old_path {
                Some(old) => format!("{old} -> {}", f.path),
                None => f.path.clone(),
            };
            (f, name)
        })
        .collect();

    let width = named.iter().map(|(_, name)| name.len()).max().unwrap_or(0);

    for (file, name) in &named {
        let counts = match file.skipped {
            Some(reason) => format!("({})", reason.as_str()),
            None => format!("+{} -{}", file.additions, file.deletions),
        };
        println!("  {}  {name:width$}  {counts}", file.status.label());
    }

    let totals = &model.totals;
    let skipped = if totals.skipped > 0 {
        format!(", {} skipped", totals.skipped)
    } else {
        String::new()
    };

    println!(
        "\n{} file{} changed, +{} -{}{skipped} against {} ({short_sha}).",
        totals.files,
        if totals.files == 1 { "" } else { "s" },
        totals.additions,
        totals.deletions,
        model.base.rev,
    );
}

/// Asks a recorded server whether it is still up, and still the one recorded.
fn server_alive(info: &ServerInfo) -> bool {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), info.port);
    let timeout = Duration::from_millis(500);

    let Ok(mut stream) = TcpStream::connect_timeout(&address, timeout) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let request = format!(
        "GET /api/health HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nx-review-token: {}\r\nConnection: close\r\n\r\n",
        info.port, info.token
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }

    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);

    response.starts_with("HTTP/1.1 200") && response.contains(&format!("\"pid\":{}", info.pid))
}

fn running_server(registry: &Registry) -> Option<ServerInfo> {
    registry.server_info().filter(server_alive)
}

fn session_url(base: &str, session: Option<&str>) -> String {
    match session {
        Some(id) => format!("{base}&session={id}"),
        None => base.to_string(),
    }
}

fn print_notes(notes: &[String]) {
    for note in notes {
        eprintln!("note: {note}");
    }
}

fn warn_if_commands_stale() {
    let Ok(dir) = commands_dir() else { return };
    let stale = stale_commands(&dir);

    if !stale.is_empty() {
        eprintln!(
            "note: the Claude Code commands are missing or out of date ({}). Run `claude-review install-command`.",
            stale.join(", ")
        );
    }
}

fn describe_registered(registered: &Registered) {
    let session = &registered.session;
    println!(
        "{} session {} \"{}\" for {}",
        if registered.reused {
            "Reusing"
        } else {
            "Registered"
        },
        session.id,
        session.name,
        session.path
    );
}

fn base_description(session: &Session) -> String {
    let (base, label) = resolve_base(Path::new(&session.path), &session.base);
    let shown = label.unwrap_or(base);
    if session.base == claude_review::registry::AUTO_BASE {
        format!("auto, currently {shown}")
    } else {
        shown
    }
}

fn run_register(
    path: PathBuf,
    name: Option<String>,
    base: Option<String>,
    by: String,
    no_untracked: bool,
    json: bool,
) -> anyhow::Result<()> {
    let created_by =
        Creator::parse(&by).ok_or_else(|| anyhow::anyhow!("--by must be user or claude"))?;
    let registry = Registry::locate()?;

    let registered = registry.register(NewSession {
        path,
        name,
        base,
        include_untracked: no_untracked.then_some(false),
        created_by: Some(created_by),
    })?;

    let url =
        running_server(&registry).map(|info| session_url(&info.url, Some(&registered.session.id)));

    if json {
        let value = serde_json::json!({
            "session": registry.summarize(registered.session.clone(), None),
            "reused": registered.reused,
            "notes": registered.notes,
            "url": url,
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    describe_registered(&registered);
    println!("Base: {}", base_description(&registered.session));
    match url {
        Some(url) => println!("Open it: {url}"),
        None => println!("Run `claude-review` to open the app."),
    }
    print_notes(&registered.notes);

    Ok(())
}

fn relation_label(summary: &SessionSummary) -> &'static str {
    use claude_review::registry::Relation;
    match summary.relation {
        Some(Relation::Same) => "here",
        Some(Relation::Worktree) => "other worktree",
        Some(Relation::Parent) => "below here",
        None => "",
    }
}

fn tidy_path(path: &str) -> String {
    match dirs::home_dir().and_then(|home| {
        Path::new(path)
            .strip_prefix(&home)
            .ok()
            .map(|rest| rest.to_path_buf())
    }) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.to_string(),
    }
}

fn run_sessions(
    status: Option<String>,
    all: bool,
    near: Option<PathBuf>,
    json: bool,
) -> anyhow::Result<()> {
    let status = match status.as_deref() {
        None => None,
        Some(raw) => Some(
            SessionStatus::parse(raw)
                .ok_or_else(|| anyhow::anyhow!("--status must be open, submitted or closed"))?,
        ),
    };

    let registry = Registry::locate()?;
    let near = near.unwrap_or(std::env::current_dir()?);
    let place = Place::of(&near);

    let mut summaries: Vec<SessionSummary> = registry
        .list()
        .into_iter()
        .filter(|s| match status {
            Some(wanted) => s.status == wanted,
            None => all || s.status != SessionStatus::Closed,
        })
        .map(|s| registry.summarize(s, Some(&place)))
        .collect();

    // Sessions related to where you are come first, closest first.
    summaries.sort_by_key(|s| match s.relation {
        Some(claude_review::registry::Relation::Same) => 0,
        Some(claude_review::registry::Relation::Worktree) => 1,
        Some(claude_review::registry::Relation::Parent) => 2,
        None => 3,
    });

    if json {
        println!("{}", serde_json::to_string_pretty(&summaries)?);
        return Ok(());
    }

    if summaries.is_empty() {
        println!(
            "No sessions. Run `claude-review` in a repository, or `/review-start` in Claude Code."
        );
        return Ok(());
    }

    let name_width = summaries
        .iter()
        .map(|s| s.session.name.chars().count())
        .max()
        .unwrap_or(0);
    let branch_width = summaries
        .iter()
        .map(|s| s.branch.as_deref().unwrap_or("-").chars().count())
        .max()
        .unwrap_or(0);

    for s in &summaries {
        let mut extra = Vec::new();
        if s.missing {
            extra.push("missing".to_string());
        }
        if s.draft_comments > 0 {
            extra.push(format!("{} draft", s.draft_comments));
        }
        let relation = relation_label(s);
        if !relation.is_empty() {
            extra.push(relation.to_string());
        }

        println!(
            "{}  {:9}  {:name_width$}  {:branch_width$}  {}{}",
            s.session.id,
            s.session.status.as_str(),
            s.session.name,
            s.branch.as_deref().unwrap_or("-"),
            tidy_path(&s.session.path),
            if extra.is_empty() {
                String::new()
            } else {
                format!("  ({})", extra.join(", "))
            },
        );
    }

    Ok(())
}

fn session_header(session: &Session) -> String {
    format!(
        "Session: {} ({}), worktree {}\n",
        session.id, session.name, session.path
    )
}

fn run_show(id: &str) -> anyhow::Result<()> {
    let registry = Registry::locate()?;
    let (session, markdown) = registry.latest_review(id)?;

    let Some(markdown) = markdown else {
        anyhow::bail!(
            "session {} ({}) has no submitted review yet",
            session.id,
            session.name
        );
    };

    println!("{}\n{markdown}", session_header(&session));
    Ok(())
}

fn run_take(id: &str) -> anyhow::Result<()> {
    let registry = Registry::locate()?;
    let (session, markdown) = registry.take(id)?;

    println!("{}\n{markdown}", session_header(&session));
    Ok(())
}

fn run_close(id: &str) -> anyhow::Result<()> {
    let registry = Registry::locate()?;
    let session = registry.close(id)?;

    println!("Closed session {} \"{}\".", session.id, session.name);
    Ok(())
}

fn run_install(force: bool) -> anyhow::Result<()> {
    let dir = commands_dir()?;
    let results = install_commands(&dir, force)?;

    let mut edited = false;
    for (path, outcome) in &results {
        println!("{} {}", outcome.describe(), path.display());
        edited |= *outcome == Outcome::Modified;
    }

    if edited {
        println!("Pass --force to replace edited copies with the current version.");
    }
    println!(
        "Use /review-start to register a session, and /review-feedback after submitting a review."
    );

    Ok(())
}

async fn wait_for_shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut term = match signal(SignalKind::terminate()) {
            Ok(term) => term,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// The session the app should open on: the one asked for, else the current
/// worktree's, created if needed. `None` outside a repository.
fn target_session(registry: &Registry, args: &AppArgs) -> anyhow::Result<Option<Session>> {
    if let Some(id) = &args.session {
        return Ok(Some(registry.resolve(id)?));
    }
    if args.no_session {
        return Ok(None);
    }

    let cwd = args.cwd();
    if claude_review::git::repo_root(&cwd).is_err() {
        return Ok(None);
    }

    let registered = registry.register(NewSession {
        path: cwd,
        base: args.base.clone(),
        include_untracked: args.no_untracked.then_some(false),
        created_by: Some(Creator::User),
        ..NewSession::default()
    })?;

    describe_registered(&registered);
    print_notes(&registered.notes);

    Ok(Some(registered.session))
}

async fn run_app(args: AppArgs) -> anyhow::Result<()> {
    if args.summary || args.json {
        let model = build_review(&args.build_options()?)?;

        if args.summary {
            print_summary(&model);
        } else {
            println!("{}", serde_json::to_string_pretty(&model)?);
        }

        return Ok(());
    }

    warn_if_commands_stale();

    let registry = Registry::locate()?;
    let session = target_session(&registry, &args)?;
    let session_id = session.as_ref().map(|s| s.id.as_str());

    if let Some(info) = running_server(&registry) {
        let url = session_url(&info.url, session_id);
        println!("claude-review is already running: {url}");
        if args.web_root.is_some() || args.port != 0 {
            eprintln!("note: --web-root and --port only apply when starting the app; stop the running one first to use them.");
        }
        if !args.no_open {
            open_browser(&url);
        }
        return Ok(());
    }

    let web = match &args.web_root {
        Some(dir) => WebSource::Directory(dir.clone()),
        None => WebSource::Embedded,
    };

    let server = start_server(ServerOptions {
        registry: registry.clone(),
        host: IpAddr::V4(Ipv4Addr::LOCALHOST),
        port: args.port,
        web,
        claude_projects: claude_projects_dir(),
    })
    .await?;

    let pid = std::process::id();
    let recorded = registry.write_server_info(&ServerInfo {
        pid,
        port: server.port,
        token: server.token.clone(),
        url: server.url.clone(),
        started_at: now_iso(),
    });
    if let Err(err) = recorded {
        eprintln!("note: could not record the running app, so a second `claude-review` will start another: {err}");
    }

    let url = session_url(&server.url, session_id);
    println!("claude-review listening on {url}");
    println!("Press Ctrl+C to stop.");

    if !args.no_open {
        open_browser(&url);
    }

    wait_for_shutdown().await;
    registry.clear_server_info(pid);
    server.close().await;

    Ok(())
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Some(Command::Register {
            path,
            name,
            base,
            by,
            no_untracked,
            json,
        }) => run_register(path, name, base, by, no_untracked, json),
        Some(Command::Sessions {
            status,
            all,
            near,
            json,
        }) => run_sessions(status, all, near, json),
        Some(Command::Show { id }) => run_show(&id),
        Some(Command::Take { id }) => run_take(&id),
        Some(Command::Close { id }) => run_close(&id),
        Some(Command::Install { force }) => run_install(force),
        None => run_app(cli.app).await,
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("claude-review: {err}");
            ExitCode::FAILURE
        }
    }
}
