//! Interactive dashboard server (axum). All analysis runs in Rust; the page
//! asks for results and Plotly figures through `POST /api/{op}`.

use crate::api::{self, Registry};
use anyhow::Result;
use axum::body::Body;
use axum::extract::{Extension, Path, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

pub const PLOTLY_JS: &str = include_str!("../web/vendor/plotly.min.js");
pub const COMMON_JS: &str = include_str!("../web/common.js");
pub const STYLE_CSS: &str = include_str!("../web/style.css");
const APP_HTML: &str = include_str!("../web/app.html");
const APP_JS: &str = include_str!("../web/app.js");
pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");
const ICON_ICO: &[u8] = include_bytes!("../assets/icon.ico");

pub fn app_page(token: &str) -> String {
    APP_HTML
        .replace("__STDF_TOKEN__", token)
        .replace("/*__STYLE__*/", STYLE_CSS)
        .replace("/*__COMMON__*/", COMMON_JS)
        .replace("/*__APP__*/", APP_JS)
}

async fn index(Extension(guard): Extension<Guard>) -> Html<String> {
    Html(app_page(&guard.token))
}

async fn plotly() -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "max-age=86400"),
        ],
        PLOTLY_JS,
    )
        .into_response()
}

async fn favicon() -> Response {
    ([(header::CONTENT_TYPE, "image/x-icon")], ICON_ICO).into_response()
}

async fn icon_png() -> Response {
    ([(header::CONTENT_TYPE, "image/png")], ICON_PNG).into_response()
}

async fn op(
    State(reg): State<Arc<Registry>>,
    Path(op): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    let params = body.map(|b| b.0).unwrap_or(json!({}));
    let res = tokio::task::spawn_blocking(move || api::run(&reg, &op, &params)).await;
    match res {
        Ok(Ok(v)) => Json(json!({"ok": true, "data": v})).into_response(),
        Ok(Err(e)) => (
            StatusCode::OK,
            Json(json!({"ok": false, "error": format!("{e:#}")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn export_csv(
    State(reg): State<Arc<Registry>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let params: Value = json!(q);
    match tokio::task::spawn_blocking(move || api::export_csv_op(&reg, &params)).await {
        Ok(Ok(csv)) => (
            [
                (header::CONTENT_TYPE, "text/csv"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"stdf-cypher-export.csv\"",
                ),
            ],
            csv,
        )
            .into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Largest upload the dashboard stores. The page only sends STDF files, so this is a backstop against a runaway write.
const MAX_UPLOAD: u64 = 8 << 30;

/// Receive a dropped file (raw body) and store it in the cache folder.
async fn upload(Query(q): Query<HashMap<String, String>>, body: Body) -> Response {
    let name = q
        .get("name")
        .cloned()
        .unwrap_or_else(|| "upload.stdf".into());
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let dir = crate::desktop::cache_dir().join("uploads");
    let res: Result<String> = async {
        tokio::fs::create_dir_all(&dir).await?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let sub = dir.join(stamp.to_string());
        tokio::fs::create_dir_all(&sub).await?;
        let path = sub.join(&safe);
        let mut f = tokio::fs::File::create(&path).await?;
        let mut stream = body.into_data_stream();
        let mut total = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            total += chunk.len() as u64;
            if total > MAX_UPLOAD {
                drop(f);
                let _ = tokio::fs::remove_file(&path).await;
                anyhow::bail!("larger than {} GB", MAX_UPLOAD >> 30);
            }
            f.write_all(&chunk).await?;
        }
        f.flush().await?;
        Ok(path.to_string_lossy().to_string())
    }
    .await;
    match res {
        Ok(p) => Json(json!({"ok": true, "data": {"path": p, "name": name}})).into_response(),
        Err(e) => {
            Json(json!({"ok": false, "error": format!("upload failed: {e:#}")})).into_response()
        }
    }
}

/// Dropped when the window's event stream disconnects (window closed / reloaded).
struct WindowGuard(Arc<Registry>);
impl Drop for WindowGuard {
    fn drop(&mut self) {
        self.0.session.disconnect();
    }
}

/// Long-lived event stream: tells the app the window is alive (unaffected by
/// background-tab timer throttling) and pushes files opened from a 2nd launch.
async fn events(
    State(reg): State<Arc<Registry>>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>>> {
    reg.session.connect();
    let rx = reg.session.events.subscribe();
    let guard = WindowGuard(reg);
    let stream = futures_util::stream::unfold((guard, rx), |(g, mut rx)| async move {
        let ev = tokio::select! {
            m = rx.recv() => match m {
                Ok(paths) => Event::default().event("open").data(paths),
                Err(_) => Event::default().comment("lag"),
            },
            _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => Event::default().comment("alive"),
        };
        Some((Ok(ev), (g, rx)))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Who may talk to the dashboard. The Host and Origin must name this server (so a
/// DNS-rebound name or a page from another site is refused), and the API needs the
/// per-launch token, which only the page served by this server receives.
#[derive(Clone)]
pub struct Guard {
    hosts: Vec<String>,
    token: String,
}

impl Guard {
    /// Guard for a server bound to `bound` on `port`. Loopback names always pass;
    /// a specific address (serve with --host and an address) passes as well.
    pub fn new(bound: &str, port: u16) -> Self {
        let mut hosts = vec![
            format!("127.0.0.1:{port}"),
            format!("localhost:{port}"),
            format!("[::1]:{port}"),
        ];
        if !matches!(bound, "0.0.0.0" | "::" | "127.0.0.1" | "localhost" | "::1") {
            hosts.push(if bound.contains(':') {
                format!("[{bound}]:{port}")
            } else {
                format!("{bound}:{port}")
            });
        }
        if port == 80 {
            // browsers leave the default port out of Host and Origin
            let bare: Vec<String> = hosts
                .iter()
                .filter_map(|h| h.strip_suffix(":80").map(String::from))
                .collect();
            hosts.extend(bare);
        }
        let mut raw = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw);
        Self {
            hosts: hosts.iter().map(|h| h.to_ascii_lowercase()).collect(),
            token: raw.iter().map(|b| format!("{b:02x}")).collect(),
        }
    }

    /// The per-launch token the page sends with every API request.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The Host values accepted, e.g. 127.0.0.1:8765.
    pub fn hosts(&self) -> &[String] {
        &self.hosts
    }

    /// Refuse a request whose Host or Origin is not this server, and an API request
    /// without the token: header x-stdf-token, or t= in the query (links and events).
    async fn check(&self, req: Request, next: Next) -> Response {
        let headers = req.headers();
        let host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .map(str::to_ascii_lowercase);
        if !host.is_some_and(|h| self.hosts.contains(&h)) {
            return (StatusCode::FORBIDDEN, "host not allowed").into_response();
        }
        if let Some(origin) = headers.get(header::ORIGIN) {
            let origin = origin.to_str().ok().map(str::to_ascii_lowercase);
            if !origin.is_some_and(|o| self.hosts.iter().any(|h| o == format!("http://{h}"))) {
                return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
            }
        }
        if req.uri().path().starts_with("/api/") {
            let header_token = headers.get("x-stdf-token").and_then(|v| v.to_str().ok());
            let query_token = req
                .uri()
                .query()
                .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("t=")));
            let given = header_token.or(query_token).unwrap_or("");
            if !same_token(given, &self.token) {
                return (StatusCode::UNAUTHORIZED, "token missing or wrong").into_response();
            }
        }
        next.run(req).await
    }
}

/// Compares in constant time, so the token cannot be guessed one byte at a time.
fn same_token(given: &str, want: &str) -> bool {
    given.len() == want.len()
        && given
            .bytes()
            .zip(want.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

pub fn router(reg: Arc<Registry>, guard: Guard) -> Router {
    let checked = guard.clone();
    Router::new()
        .route("/", get(index))
        .route("/favicon.ico", get(favicon))
        .route("/static/icon.png", get(icon_png))
        .route("/static/plotly.min.js", get(plotly))
        .route("/api/export.csv", get(export_csv))
        .route("/api/upload", post(upload))
        .route("/api/events", get(events))
        .route("/api/{op}", post(op))
        .with_state(reg)
        .layer(Extension(guard))
        .layer(middleware::from_fn(move |req: Request, next: Next| {
            let guard = checked.clone();
            async move { guard.check(req, next).await }
        }))
}

/// Run the server on an already-bound listener until Ctrl+C or, in app mode,
/// until the app window has been closed.
pub async fn run(
    reg: Arc<Registry>,
    listener: tokio::net::TcpListener,
    guard: Guard,
) -> Result<()> {
    let watch = reg.clone();
    axum::serve(listener, router(reg, guard))
        .with_graceful_shutdown(async move {
            let ctrl_c = tokio::signal::ctrl_c();
            let window_closed = async {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    if watch.session.should_exit() {
                        break;
                    }
                }
            };
            tokio::select! {
                _ = ctrl_c => {},
                _ = window_closed => {},
            }
        })
        .await?;
    Ok(())
}

pub async fn serve(reg: Arc<Registry>, host: &str, port: u16, open: bool) -> Result<()> {
    let listener = match tokio::net::TcpListener::bind((host, port)).await {
        Ok(l) => l,
        // another dashboard already has the port: take any free one
        Err(e) if port != 0 && e.kind() == std::io::ErrorKind::AddrInUse => {
            eprintln!("port {port} is in use, picking a free one");
            tokio::net::TcpListener::bind((host, 0)).await?
        }
        Err(e) => return Err(e.into()),
    };
    let addr = listener.local_addr()?;
    let guard = Guard::new(host, addr.port());
    let url = format!(
        "http://{}:{}/",
        if host == "0.0.0.0" { "localhost" } else { host },
        addr.port()
    );
    eprintln!("stdf-cypher dashboard: {url}  (Ctrl+C to stop)");
    if host == "0.0.0.0" {
        eprintln!("only localhost names are accepted; to open from another PC, start with --host and this PC's address");
    }
    if open {
        let _ = crate::desktop::open_default_browser(&url);
    }
    run(reg, listener, guard).await
}

/// Desktop app: free port, app window, exit when the window closes.
pub async fn app(reg: Arc<Registry>, open_window: bool) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let url = format!("http://127.0.0.1:{port}/");
    let guard = Guard::new("127.0.0.1", port);
    crate::desktop::write_instance(port, guard.token());
    eprintln!("{} running at {url}", crate::desktop::APP_NAME);
    if open_window {
        crate::desktop::open_window(&url)?;
    }
    let r = run(reg, listener, guard).await;
    crate::desktop::clear_instance();
    r
}
