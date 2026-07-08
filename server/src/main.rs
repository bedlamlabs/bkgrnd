mod auth;
mod spotify;

use anyhow::Context;
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Json, Query, State},
    http::{header, HeaderMap, HeaderValue, Response, StatusCode},
    response::{IntoResponse, Redirect},
    routing::{get, post},
    Router,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, Notify, Semaphore};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tracing::{error, info, warn};
use tower_http::services::ServeDir;

const YTDLP_RESOLVE_TIMEOUT: Duration = Duration::from_secs(60);
const YTDLP_SEARCH_TIMEOUT: Duration = Duration::from_secs(30);
// yt-dlp forks a python + JS-runtime process per invocation; cap global
// concurrency so a burst of clients can't exhaust the host.
const YTDLP_MAX_CONCURRENCY: usize = 4;
const SEARCH_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
// Long-poll hold for /local/commands/next: sub-second command pickup without
// the Mac hammering the endpoint every 2s.
const COMMAND_LONG_POLL: Duration = Duration::from_secs(25);
const SPOTIFY_MATCH_CONCURRENCY: usize = 3;

#[derive(Clone)]
struct AppState {
    data_dir: PathBuf,
    bearer_token: Option<String>,
    playlists_lock: Arc<Mutex<()>>,
    http: reqwest::Client,
    stream_cache: Arc<Mutex<HashMap<String, CachedStreamUrl>>>,
    stream_failures: Arc<Mutex<HashMap<String, CachedStreamFailure>>>,
    stream_resolves: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    search_cache: Arc<Mutex<HashMap<String, (Instant, Vec<SearchResult>)>>>,
    ytdlp_sem: Arc<Semaphore>,
    local_status: Arc<Mutex<Option<LocalStatusRecord>>>,
    local_commands: Arc<Mutex<VecDeque<(Instant, LocalPlaybackCommand)>>>,
    next_local_command_id: Arc<Mutex<u64>>,
    command_notify: Arc<Notify>,
    // PWA auth: HMAC key for signed session cookies + the persistent user store.
    session_secret: Arc<Vec<u8>>,
    users: Arc<Mutex<Vec<auth::User>>>,
    registration_open: bool,
}

/// Who is making a request. The bearer token (iOS app + Mac menubar) reads and
/// writes the shared/global library; a signed-in PWA user gets their own.
enum Principal {
    Bearer,
    User(String),
}

impl AppState {
    /// Identify the caller, or None if unauthorized. Bearer token (header or
    /// `?token=`) wins; otherwise a valid signed PWA session cookie.
    fn principal(&self, headers: &HeaderMap, token_qs: Option<&str>) -> Option<Principal> {
        if auth_ok(headers, &self.bearer_token, token_qs) {
            return Some(Principal::Bearer);
        }
        let cookie = headers.get(header::COOKIE).and_then(|v| v.to_str().ok())?;
        let tok = auth::session_from_cookie_header(cookie)?;
        auth::verify_session(&self.session_secret, &tok).map(Principal::User)
    }

    /// True if a request may access the API (the common case that doesn't care
    /// who the caller is). Cast stream signatures are checked separately.
    fn authorized(&self, headers: &HeaderMap, token_qs: Option<&str>) -> bool {
        self.principal(headers, token_qs).is_some()
    }

    /// Path to the caller's playlists file. A user's file is seeded from the
    /// current global library on first access, then diverges independently.
    async fn playlists_path_for(&self, principal: &Principal) -> PathBuf {
        match principal {
            Principal::Bearer => playlists_path(&self.data_dir),
            Principal::User(user) => {
                let path = self.data_dir.join("users").join(user).join("playlists.yaml");
                self.seed_if_absent(&path, &playlists_path(&self.data_dir), None)
                    .await;
                path
            }
        }
    }

    /// Path to the caller's history file (seeded like playlists, default `[]`).
    async fn history_path_for(&self, principal: &Principal) -> PathBuf {
        match principal {
            Principal::Bearer => history_path(&self.data_dir),
            Principal::User(user) => {
                let path = self.data_dir.join("users").join(user).join("history.json");
                self.seed_if_absent(&path, &history_path(&self.data_dir), Some(b"[]"))
                    .await;
                path
            }
        }
    }

    /// Copy `global` into `path` on first access; if the global file is missing,
    /// write `fallback` (so a new user starts from the current shared library,
    /// or an empty default).
    async fn seed_if_absent(&self, path: &std::path::Path, global: &std::path::Path, fallback: Option<&[u8]>) {
        if tokio::fs::metadata(path).await.is_ok() {
            return;
        }
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        match tokio::fs::read(global).await {
            Ok(bytes) => {
                let _ = write_atomic(path, &bytes).await;
            }
            Err(_) => {
                if let Some(fallback) = fallback {
                    let _ = write_atomic(path, fallback).await;
                }
            }
        }
    }
}

#[derive(Clone)]
struct CachedStreamUrl {
    url: String,
    expires_at: Instant,
}

#[derive(Clone)]
struct CachedStreamFailure {
    user_message: String,
    technical_message: String,
    expires_at: Instant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistDoc {
    version: u32,
    updated_at: String,
    playlists: Vec<Playlist>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Playlist {
    id: String,
    name: String,
    items: Vec<PlaylistItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistItem {
    url: String,
    title: String,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    thumbnail: String,
    #[serde(default)]
    added_at: String,
    // Menubar app writes these; clients use them for duration/LIVE chips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    duration: Option<f64>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    entry_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalPlayerStatus {
    #[serde(default)]
    is_playing: bool,
    #[serde(default)]
    is_paused: bool,
    #[serde(default)]
    title: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    thumbnail: String,
    #[serde(default)]
    video_id: String,
    #[serde(default)]
    source_url: String,
    #[serde(default)]
    queue_position: usize,
    #[serde(default)]
    queue_length: usize,
    #[serde(default)]
    playlist_title: String,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    position: Option<f64>,
}

#[derive(Debug, Clone)]
struct LocalStatusRecord {
    status: LocalPlayerStatus,
    received_at: Instant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalStatusResponse {
    online: bool,
    received_ago_ms: Option<u128>,
    status: Option<LocalPlayerStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalCommandRequest {
    action: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    thumbnail: String,
    #[serde(default)]
    source_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalPlaybackCommand {
    id: u64,
    action: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    thumbnail: String,
    #[serde(default)]
    source_url: String,
}

fn playlists_path(data_dir: &PathBuf) -> PathBuf {
    data_dir.join("playlists.yaml")
}

/// Write via temp-file + rename so a crash mid-write can never leave a torn
/// file behind. (A truncated playlists.yaml once wedged sync fleet-wide: the
/// Mac treated the unparseable remote as unreachable and never overwrote it.)
async fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await
}

fn history_path(data_dir: &PathBuf) -> PathBuf {
    data_dir.join("history.json")
}

fn find_web_dir() -> PathBuf {
    // Prefer explicit override for deployments.
    if let Ok(v) = std::env::var("WOPR_WEB_DIR") {
        if !v.trim().is_empty() {
            return PathBuf::from(v);
        }
    }

    // Try common relative locations so systemd WorkingDirectory does not matter.
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("web"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("web"));
            candidates.push(exe_dir.join("../web"));
            candidates.push(exe_dir.join("../../web"));
            candidates.push(exe_dir.join("../../../web"));
        }
    }

    for p in candidates {
        if p.join("index.html").is_file() {
            return p;
        }
    }

    // Fall back to the original relative path.
    PathBuf::from("web")
}

fn auth_ok(headers: &HeaderMap, token: &Option<String>, token_qs: Option<&str>) -> bool {
    let Some(expected) = token else {
        return true;
    };

    // Stopgap: allow query param token for browser/PWA media elements that can't set Authorization headers.
    if let Some(q) = token_qs {
        if q == expected {
            return true;
        }
    }

    let Some(auth) = headers.get(header::AUTHORIZATION) else {
        return false;
    };
    let Ok(s) = auth.to_str() else {
        return false;
    };
    s == format!("Bearer {}", expected)
}

fn now_unix_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string()
}

#[derive(Debug, Deserialize)]
struct CredsRequest {
    username: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct MeResponse {
    username: String,
}

#[derive(Debug, Serialize)]
struct CastSigResponse {
    castsig: String,
}

fn cookie_response(status: StatusCode, set_cookie: String) -> Response<Body> {
    let mut resp = status.into_response();
    if let Ok(val) = HeaderValue::from_str(&set_cookie) {
        resp.headers_mut().append(header::SET_COOKIE, val);
    }
    resp
}

/// Login/register success: set the session cookie (for the PWA) AND return the
/// token in the body. Native clients (Android app) can't easily juggle cookies,
/// so they read the token here and send it back as `Cookie: bkgrnd_session=…`.
fn session_response(username: &str, token: &str) -> Response<Body> {
    let mut resp =
        Json(serde_json::json!({ "username": username, "token": token })).into_response();
    if let Ok(val) = HeaderValue::from_str(&auth::session_set_cookie(token)) {
        resp.headers_mut().append(header::SET_COOKIE, val);
    }
    resp
}

/// Create a user (open unless WOPR_REGISTRATION_OPEN=0) and log them in.
async fn pwa_register(State(state): State<AppState>, Json(req): Json<CredsRequest>) -> Response<Body> {
    if !state.registration_open {
        return (StatusCode::FORBIDDEN, "Registration is closed").into_response();
    }
    let Some(username) = auth::normalize_username(&req.username) else {
        return (
            StatusCode::BAD_REQUEST,
            "Invalid username (1-32 chars: a-z 0-9 _ . -)",
        )
            .into_response();
    };
    if req.password.len() < 6 {
        return (StatusCode::BAD_REQUEST, "Password must be at least 6 characters").into_response();
    }

    let mut users = state.users.lock().await;
    if users.iter().any(|u| u.username == username) {
        return (StatusCode::CONFLICT, "Username already taken").into_response();
    }
    let password_hash = match auth::hash_password(&req.password) {
        Ok(h) => h,
        Err(e) => {
            error!("password hash failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    users.push(auth::User {
        username: username.clone(),
        password_hash,
        created_at: now_unix_string(),
    });
    if let Err(e) = auth::save_users(&state.data_dir, &users) {
        error!("failed to persist users.json: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    drop(users);

    let token = auth::mint_session(&state.session_secret, &username, auth::SESSION_TTL_SECS);
    session_response(&username, &token)
}

async fn pwa_login(State(state): State<AppState>, Json(req): Json<CredsRequest>) -> Response<Body> {
    let Some(username) = auth::normalize_username(&req.username) else {
        return (StatusCode::UNAUTHORIZED, "Invalid username or password").into_response();
    };
    let users = state.users.lock().await;
    let ok = users
        .iter()
        .find(|u| u.username == username)
        .map(|u| auth::verify_password(&req.password, &u.password_hash))
        .unwrap_or(false);
    drop(users);
    if !ok {
        return (StatusCode::UNAUTHORIZED, "Invalid username or password").into_response();
    }
    let token = auth::mint_session(&state.session_secret, &username, auth::SESSION_TTL_SECS);
    session_response(&username, &token)
}

async fn pwa_logout() -> Response<Body> {
    cookie_response(StatusCode::NO_CONTENT, auth::session_clear_cookie())
}

/// Cheap "am I logged in?" check for the PWA on load.
async fn pwa_me(State(state): State<AppState>, headers: HeaderMap) -> Response<Body> {
    if let Some(cookie) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        if let Some(tok) = auth::session_from_cookie_header(cookie) {
            if let Some(username) = auth::verify_session(&state.session_secret, &tok) {
                return Json(MeResponse { username }).into_response();
            }
        }
    }
    StatusCode::UNAUTHORIZED.into_response()
}

/// Hand an authenticated browser a short-lived signature it can put in a Cast
/// stream URL (Chromecast fetches the URL itself and can't send the cookie).
async fn pwa_cast_sig(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> Response<Body> {
    if !state.authorized(&headers, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let castsig = auth::mint_cast_sig(&state.session_secret, auth::CAST_TTL_SECS);
    Json(CastSigResponse { castsig }).into_response()
}

fn ytdlp_command() -> tokio::process::Command {
    let bin = std::env::var("WOPR_YTDLP_BIN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "yt-dlp".to_string());
    tokio::process::Command::new(bin)
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

#[derive(Debug, Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

async fn get_playlists(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    let Some(principal) = state.principal(&headers, q.token.as_deref()) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    let _guard = state.playlists_lock.lock().await;
    let path = state.playlists_path_for(&principal).await;
    let raw = match tokio::fs::read_to_string(&path).await {
        Ok(v) => v,
        Err(_) => {
            // Empty doc if missing.
            let doc = PlaylistDoc {
                version: 1,
                updated_at: "1970-01-01T00:00:00.000Z".to_string(),
                playlists: vec![],
            };
            serde_yaml::to_string(&doc).unwrap_or_default()
        }
    };

    let mut resp = Response::new(Body::from(raw));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/yaml; charset=utf-8"),
    );
    resp
}

async fn get_playlists_json(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    let Some(principal) = state.principal(&headers, q.token.as_deref()) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    let _guard = state.playlists_lock.lock().await;
    let path = state.playlists_path_for(&principal).await;
    let doc: PlaylistDoc = match tokio::fs::read_to_string(&path).await {
        Ok(raw) => serde_yaml::from_str(&raw).unwrap_or(PlaylistDoc {
            version: 1,
            updated_at: "1970-01-01T00:00:00.000Z".to_string(),
            playlists: vec![],
        }),
        Err(_) => PlaylistDoc {
            version: 1,
            updated_at: "1970-01-01T00:00:00.000Z".to_string(),
            playlists: vec![],
        },
    };

    match serde_json::to_vec(&doc) {
        Ok(bytes) => {
            let mut resp = Response::new(Body::from(bytes));
            resp.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json; charset=utf-8"),
            );
            resp
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn put_playlists(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    body: Bytes,
) -> impl IntoResponse {
    let Some(principal) = state.principal(&headers, q.token.as_deref()) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    // Validate YAML
    let raw = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UTF-8").into_response(),
    };
    if let Err(e) = serde_yaml::from_str::<PlaylistDoc>(raw) {
        return (StatusCode::BAD_REQUEST, format!("Invalid playlist YAML: {e}")).into_response();
    }

    let _guard = state.playlists_lock.lock().await;
    let path = state.playlists_path_for(&principal).await;
    if let Some(parent) = path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            error!("failed to create data dir: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    if let Err(e) = write_atomic(&path, raw.as_bytes()).await {
        error!("failed to write playlists: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn put_playlists_json(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    body: Bytes,
) -> impl IntoResponse {
    let Some(principal) = state.principal(&headers, q.token.as_deref()) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let doc: PlaylistDoc = match serde_json::from_slice(&body) {
        Ok(d) => d,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid playlist JSON").into_response(),
    };
    let raw = serde_yaml::to_string(&doc).unwrap_or_default();

    let _guard = state.playlists_lock.lock().await;
    let path = state.playlists_path_for(&principal).await;
    if let Some(parent) = path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            error!("failed to create data dir: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    if let Err(e) = write_atomic(&path, raw.as_bytes()).await {
        error!("failed to write playlists: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn get_history_json(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    let Some(principal) = state.principal(&headers, q.token.as_deref()) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    let path = state.history_path_for(&principal).await;
    let raw = match tokio::fs::read_to_string(&path).await {
        Ok(v) => v,
        Err(_) => "[]".to_string(),
    };

    let mut resp = Response::new(Body::from(raw));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    resp
}

async fn put_history_json(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    body: Bytes,
) -> impl IntoResponse {
    let Some(principal) = state.principal(&headers, q.token.as_deref()) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    // Validate JSON array (history entries are client-defined; keep it permissive).
    if serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .is_none()
    {
        return (StatusCode::BAD_REQUEST, "Invalid history JSON").into_response();
    }

    let path = state.history_path_for(&principal).await;
    if let Some(parent) = path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            error!("failed to create data dir: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    if let Err(e) = write_atomic(&path, &body).await {
        error!("failed to write history: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    StatusCode::NO_CONTENT.into_response()
}

async fn put_local_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(status): Json<LocalPlayerStatus>,
) -> impl IntoResponse {
    if !state.authorized(&headers, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let mut local_status = state.local_status.lock().await;
    *local_status = Some(LocalStatusRecord {
        status,
        received_at: Instant::now(),
    });
    StatusCode::NO_CONTENT.into_response()
}

async fn get_local_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if !state.authorized(&headers, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let local_status = state.local_status.lock().await;
    let Some(record) = local_status.as_ref() else {
        return Json(LocalStatusResponse {
            online: false,
            received_ago_ms: None,
            status: None,
        })
        .into_response();
    };

    let age = record.received_at.elapsed();
    Json(LocalStatusResponse {
        online: age <= Duration::from_secs(15),
        received_ago_ms: Some(age.as_millis()),
        status: Some(record.status.clone()),
    })
    .into_response()
}

async fn post_local_command(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(req): Json<LocalCommandRequest>,
) -> impl IntoResponse {
    if !state.authorized(&headers, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let action = req.action.trim();
    let valid = matches!(action, "play" | "pause_toggle" | "stop");
    if !valid {
        return (StatusCode::BAD_REQUEST, "Unsupported local command").into_response();
    }
    if action == "play" && req.url.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing url").into_response();
    }

    let mut next_id = state.next_local_command_id.lock().await;
    let command = LocalPlaybackCommand {
        id: *next_id,
        action: action.to_string(),
        url: req.url,
        title: req.title,
        thumbnail: req.thumbnail,
        source_url: req.source_url,
    };
    *next_id += 1;

    let mut commands = state.local_commands.lock().await;
    commands.push_back((Instant::now(), command.clone()));
    while commands.len() > 50 {
        commands.pop_front();
    }
    drop(commands);
    state.command_notify.notify_waiters();

    Json(command).into_response()
}

async fn get_next_local_command(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if !state.authorized(&headers, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    // Drop commands that have been waiting too long: if the Mac was offline when
    // the phone enqueued taps, we don't want them to replay minutes later.
    const COMMAND_TTL: Duration = Duration::from_secs(30);

    let pop_fresh = |commands: &mut VecDeque<(Instant, LocalPlaybackCommand)>| {
        let now = Instant::now();
        while let Some((enqueued_at, command)) = commands.pop_front() {
            if now.duration_since(enqueued_at) <= COMMAND_TTL {
                return Some(command);
            }
        }
        None
    };

    // Long-poll: hold the request open until a command arrives (or the hold
    // window lapses) so phone taps reach the Mac in sub-second time.
    let deadline = Instant::now() + COMMAND_LONG_POLL;
    loop {
        {
            let mut commands = state.local_commands.lock().await;
            if let Some(command) = pop_fresh(&mut commands) {
                return Json(Some(command)).into_response();
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Json(Option::<LocalPlaybackCommand>::None).into_response();
        }
        let _ = timeout(remaining, state.command_notify.notified()).await;
    }
}

#[derive(Debug, Deserialize)]
struct StreamQuery {
    url: String,
    token: Option<String>,
    proxy: Option<bool>,
    // Short-lived signature so Cast/Chromecast devices (which can't send the
    // session cookie) can fetch the proxied stream.
    castsig: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResolveQuery {
    url: String,
    token: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResolveResponse {
    stream_url: String,
    cached: bool,
    resolve_ms: u128,
    source: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrewarmResponse {
    cached: bool,
}

#[derive(Debug, Deserialize)]
struct ThumbnailQuery {
    src: String,
    token: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MetaResponse {
    title: String,
    channel: String,
    thumbnail: String,
}

/// Video metadata (title/channel/thumbnail) via YouTube oEmbed. Lets clients
/// show a real name for a pasted URL instead of the raw link. Server-side so it
/// works for the PWA too (YouTube's oEmbed has no CORS headers for browsers).
async fn video_meta(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ResolveQuery>,
) -> impl IntoResponse {
    if !state.authorized(&headers, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let resp = state
        .http
        .get("https://www.youtube.com/oembed")
        .query(&[("url", q.url.as_str()), ("format", "json")])
        .send()
        .await;
    if let Ok(resp) = resp {
        if resp.status().is_success() {
            if let Ok(v) = resp.json::<serde_json::Value>().await {
                let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                return axum::Json(MetaResponse {
                    title: s("title"),
                    channel: s("author_name"),
                    thumbnail: s("thumbnail_url"),
                })
                .into_response();
            }
        }
    }
    StatusCode::NOT_FOUND.into_response()
}

async fn resolve_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ResolveQuery>,
) -> impl IntoResponse {
    if !state.authorized(&headers, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let started_at = Instant::now();
    match resolve_stream_url(&state, &q.url).await {
        Ok(resolved) => axum::Json(ResolveResponse {
            stream_url: resolved.url,
            cached: resolved.cached,
            resolve_ms: started_at.elapsed().as_millis(),
            source: resolved.source,
        }).into_response(),
        Err(e) => {
            error!("resolve_stream failed: {e}");
            (StatusCode::BAD_REQUEST, e.user_message).into_response()
        }
    }
}

async fn prewarm_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ResolveQuery>,
) -> impl IntoResponse {
    if !state.authorized(&headers, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let cached = {
        let cache = state.stream_cache.lock().await;
        cache
            .get(&q.url)
            .map(|cached| cached.expires_at > Instant::now())
            .unwrap_or(false)
    };
    if cached {
        return axum::Json(PrewarmResponse { cached: true }).into_response();
    }

    match resolve_stream_url(&state, &q.url).await {
        Ok(_) => axum::Json(PrewarmResponse { cached: true }).into_response(),
        Err(e) => {
            if !e.terminal {
                error!("prewarm_stream failed: {e}");
            }
            (StatusCode::BAD_REQUEST, e.user_message).into_response()
        }
    }
}

async fn thumbnail_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ThumbnailQuery>,
) -> impl IntoResponse {
    if !state.authorized(&headers, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let fallback = || {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 180"><defs><linearGradient id="g" x1="0" x2="1" y1="0" y2="1"><stop stop-color="#262b38"/><stop offset="1" stop-color="#101218"/></linearGradient><radialGradient id="r" cx=".28" cy=".22" r=".45"><stop stop-color="#ee6d74" stop-opacity=".45"/><stop offset="1" stop-color="#ee6d74" stop-opacity="0"/></radialGradient></defs><rect width="320" height="180" fill="url(#g)"/><rect width="320" height="180" fill="url(#r)"/><text x="160" y="112" text-anchor="middle" font-family="system-ui, -apple-system, sans-serif" font-size="72" font-weight="900" fill="white">b</text></svg>"##;
        let mut resp = Response::new(Body::from(svg));
        resp.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("image/svg+xml; charset=utf-8"),
        );
        resp
    };

    let Ok(url) = url::Url::parse(q.src.trim()) else {
        return fallback();
    };
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if url.scheme() != "https" || !(host == "i.ytimg.com" || host.ends_with(".ytimg.com")) {
        return fallback();
    }

    let upstream = match state.http.get(url).send().await {
        Ok(resp) if resp.status().is_success() => resp,
        _ => return fallback(),
    };
    let content_type = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.starts_with("image/"))
        .unwrap_or("image/jpeg")
        .to_string();
    let bytes = match upstream.bytes().await {
        Ok(bytes) => bytes,
        Err(_) => return fallback(),
    };

    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type).unwrap_or(HeaderValue::from_static("image/jpeg")),
    );
    resp
}

/// Parse an HTTP Range header of the form `bytes=START-[END]` (first range only),
/// returning the start offset and optional end. Suffix ranges (`bytes=-N`) and malformed
/// values fall back to `(0, None)`.
fn parse_range_start_end(range: Option<&str>) -> (u64, Option<u64>) {
    let Some(spec) = range.and_then(|r| r.trim().strip_prefix("bytes=")) else {
        return (0, None);
    };
    let first = spec.split(',').next().unwrap_or("").trim();
    let mut parts = first.splitn(2, '-');
    let start = parts.next().unwrap_or("").trim().parse::<u64>().ok();
    let end = parts
        .next()
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .and_then(|e| e.parse::<u64>().ok());
    match start {
        Some(s) => (s, end),
        None => (0, None),
    }
}

// Stable-ish endpoint: client asks for /stream?url=... and we resolve a fresh direct media URL and proxy bytes.
// For a production-grade design we'd support resume/range + caching; keep minimal for personal use.
async fn stream_audio(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<StreamQuery>,
) -> impl IntoResponse {
    let cast_ok = q
        .castsig
        .as_deref()
        .map(|sig| auth::verify_cast_sig(&state.session_secret, sig))
        .unwrap_or(false);
    if !cast_ok && !state.authorized(&headers, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    // Resolve once per source URL for a short window. iOS Safari may issue multiple
    // Range/reconnect requests; rerunning yt-dlp for each one causes slow starts/stalls.
    let direct_url = match resolve_stream_url(&state, &q.url).await {
        Ok(u) => u.url,
        Err(e) => {
            error!("resolve_stream_url failed: {e}");
            if e.terminal {
                return (StatusCode::BAD_REQUEST, e.user_message).into_response();
            }
            return (StatusCode::BAD_REQUEST, e.user_message).into_response();
        }
    };

    if q.proxy != Some(true) {
        return Redirect::temporary(&direct_url).into_response();
    }

    // Proxy stream bytes to client (support Range for iOS scrubbing/reconnects).
    //
    // YouTube throttles open-ended progressive downloads to ~playback speed but serves
    // bounded byte-ranges at full speed. iOS issues an open-ended `Range: bytes=0-` to
    // stream, which—forwarded verbatim—gets throttled to ~real-time and stalls playback.
    // So for progressive audio we always request a bounded chunk upstream (capping any
    // open-ended client range); iOS pulls later chunks via follow-up Range requests.
    // HLS manifests (live streams) are tiny text playlists and are proxied verbatim.
    let is_hls = direct_url.contains("/manifest/")
        || direct_url.contains("hls_playlist")
        || direct_url.contains(".m3u8");
    if is_hls {
        let mut req = state.http.get(direct_url.as_str());
        if let Some(range) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
            req = req.header(header::RANGE, range);
        }
        let upstream = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                error!("upstream fetch failed: {e}");
                evict_stream_cache(&state, &q.url).await;
                return StatusCode::BAD_GATEWAY.into_response();
            }
        };
        if !(upstream.status().is_success() || upstream.status().as_u16() == 206) {
            warn!(
                "upstream returned {} for cached stream url; evicting cache entry",
                upstream.status()
            );
            evict_stream_cache(&state, &q.url).await;
            return StatusCode::BAD_GATEWAY.into_response();
        }
        let content_type = upstream
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/vnd.apple.mpegurl")
            .to_string();
        let status = upstream.status();
        let mut resp = Response::builder()
            .status(status)
            .body(Body::from_stream(upstream.bytes_stream()))
            .unwrap();
        resp.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(&content_type)
                .unwrap_or(HeaderValue::from_static("application/vnd.apple.mpegurl")),
        );
        return resp;
    }

    // Progressive audio. Two YouTube realities collide here:
    //   1. Large or open-ended ranges get throttled to ~playback speed
    //      (measured: full-file range 31KB/s vs 8MB range 3.7MB/s).
    //   2. AVPlayer treats a response shorter than the EXACT explicit range
    //      it requested as fatal (CoreMedia -12939 "content range mismatch").
    // So we advertise exactly the client's requested range, but fetch it
    // upstream in bounded chunks and relay them sequentially.
    const PROXY_CHUNK: u64 = 8 * 1024 * 1024;
    let (start, client_end) =
        parse_range_start_end(headers.get(header::RANGE).and_then(|v| v.to_str().ok()));

    let first_end = match client_end {
        Some(e) => e.min(start + PROXY_CHUNK - 1),
        None => start + PROXY_CHUNK - 1,
    };
    let first = match state
        .http
        .get(direct_url.as_str())
        .header(header::RANGE, format!("bytes={}-{}", start, first_end))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() || r.status().as_u16() == 206 => r,
        Ok(r) => {
            warn!(
                "upstream returned {} for cached stream url; evicting cache entry",
                r.status()
            );
            evict_stream_cache(&state, &q.url).await;
            return StatusCode::BAD_GATEWAY.into_response();
        }
        Err(e) => {
            error!("upstream fetch failed: {e}");
            evict_stream_cache(&state, &q.url).await;
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let content_type = first
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/mpeg")
        .to_string();
    let total = first
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_content_range_total);

    match client_end {
        // Explicit range wider than one chunk: relay it chunk by chunk while
        // advertising the exact requested range.
        Some(cend) if cend > first_end => {
            let span = cend - start + 1;
            let total_label = total.map(|t| t.to_string()).unwrap_or_else(|| "*".to_string());
            let content_range = format!("bytes {}-{}/{}", start, cend, total_label);

            let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(8);
            let http = state.http.clone();
            let upstream_url = direct_url.clone();
            let cache = state.stream_cache.clone();
            let source_key = q.url.clone();
            tokio::spawn(async move {
                use futures_util::StreamExt;
                let mut current = Some(first);
                let mut next_pos = first_end + 1;
                'relay: while let Some(resp) = current.take() {
                    let mut stream = resp.bytes_stream();
                    while let Some(item) = stream.next().await {
                        match item {
                            Ok(bytes) => {
                                if tx.send(Ok(bytes)).await.is_err() {
                                    break 'relay; // client went away
                                }
                            }
                            Err(e) => {
                                let _ = tx
                                    .send(Err(std::io::Error::new(std::io::ErrorKind::Other, e)))
                                    .await;
                                break 'relay;
                            }
                        }
                    }
                    if next_pos > cend {
                        break;
                    }
                    let chunk_end = (next_pos + PROXY_CHUNK - 1).min(cend);
                    match http
                        .get(upstream_url.as_str())
                        .header(header::RANGE, format!("bytes={}-{}", next_pos, chunk_end))
                        .send()
                        .await
                    {
                        Ok(r) if r.status().is_success() || r.status().as_u16() == 206 => {
                            current = Some(r);
                            next_pos = chunk_end + 1;
                        }
                        _ => {
                            cache.lock().await.remove(&source_key);
                            let _ = tx
                                .send(Err(std::io::Error::new(
                                    std::io::ErrorKind::Other,
                                    "upstream chunk failed",
                                )))
                                .await;
                            break;
                        }
                    }
                }
            });

            let body = Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx));
            let mut resp = Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .body(body)
                .unwrap();
            resp.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(&content_type)
                    .unwrap_or(HeaderValue::from_static("audio/mpeg")),
            );
            resp.headers_mut()
                .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
            if let Ok(v) = HeaderValue::from_str(&span.to_string()) {
                resp.headers_mut().insert(header::CONTENT_LENGTH, v);
            }
            if let Ok(v) = HeaderValue::from_str(&content_range) {
                resp.headers_mut().insert(header::CONTENT_RANGE, v);
            }
            resp
        }
        // Small explicit range or open-ended: forward the first chunk's
        // response as-is (open-ended clients pull follow-up ranges themselves).
        _ => {
            let status = first.status();
            let content_length = first.headers().get(header::CONTENT_LENGTH).cloned();
            let content_range = first.headers().get(header::CONTENT_RANGE).cloned();
            let mut resp = Response::builder()
                .status(status)
                .body(Body::from_stream(first.bytes_stream()))
                .unwrap();
            resp.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(&content_type)
                    .unwrap_or(HeaderValue::from_static("audio/mpeg")),
            );
            resp.headers_mut()
                .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
            if let Some(v) = content_length {
                resp.headers_mut().insert(header::CONTENT_LENGTH, v);
            }
            if let Some(v) = content_range {
                resp.headers_mut().insert(header::CONTENT_RANGE, v);
            }
            resp
        }
    }
}

/// Extract the total size from a Content-Range header ("bytes 0-99/1234").
fn parse_content_range_total(value: &str) -> Option<u64> {
    value.rsplit('/').next()?.trim().parse::<u64>().ok()
}

async fn evict_stream_cache(state: &AppState, url: &str) {
    state.stream_cache.lock().await.remove(url);
}

#[derive(Debug)]
struct StreamResolveError {
    user_message: String,
    technical_message: String,
    terminal: bool,
}

#[derive(Debug)]
struct ResolvedStream {
    url: String,
    cached: bool,
    source: String,
}

impl std::fmt::Display for StreamResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.user_message, self.technical_message)
    }
}

async fn resolve_stream_url(state: &AppState, url: &str) -> Result<ResolvedStream, StreamResolveError> {
    let now = Instant::now();
    {
        let cache = state.stream_cache.lock().await;
        if let Some(cached) = cache.get(url) {
            if cached.expires_at > now {
                return Ok(ResolvedStream {
                    url: cached.url.clone(),
                    cached: true,
                    source: "cache".to_string(),
                });
            }
        }
    }
    {
        let failures = state.stream_failures.lock().await;
        if let Some(cached) = failures.get(url) {
            if cached.expires_at > now {
                return Err(StreamResolveError {
                    user_message: cached.user_message.clone(),
                    technical_message: cached.technical_message.clone(),
                    terminal: true,
                });
            }
        }
    }

    let resolve_lock = {
        let mut resolves = state.stream_resolves.lock().await;
        resolves
            .entry(url.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = resolve_lock.lock().await;

    {
        let cache = state.stream_cache.lock().await;
        if let Some(cached) = cache.get(url) {
            if cached.expires_at > Instant::now() {
                return Ok(ResolvedStream {
                    url: cached.url.clone(),
                    cached: true,
                    source: "cache".to_string(),
                });
            }
        }
    }
    {
        let failures = state.stream_failures.lock().await;
        if let Some(cached) = failures.get(url) {
            if cached.expires_at > Instant::now() {
                return Err(StreamResolveError {
                    user_message: cached.user_message.clone(),
                    technical_message: cached.technical_message.clone(),
                    terminal: true,
                });
            }
        }
    }

    let start = Instant::now();
    let mut resolved_source = "yt-dlp".to_string();
    let resolve_result: Result<String, StreamResolveError> = match resolve_direct_url(state, url).await {
        Ok(url) => Ok(url),
        Err(e) if e.terminal => {
            let mut failures = state.stream_failures.lock().await;
            let now = Instant::now();
            failures.retain(|_, entry| entry.expires_at > now);
            failures.insert(
                url.to_string(),
                CachedStreamFailure {
                    user_message: e.user_message.clone(),
                    technical_message: e.technical_message.clone(),
                    expires_at: Instant::now() + Duration::from_secs(60 * 60),
                },
            );
            Err(e)
        }
        Err(primary_error) => {
            error!("resolve_direct_url failed: {primary_error}");
            match resolve_via_piped(&state.http, url).await {
                Ok(fallback) => {
                    resolved_source = "piped".to_string();
                    Ok(fallback)
                }
                Err(fallback_error) => {
                    error!("resolve_via_piped failed: {fallback_error:#}");
                    Err(primary_error)
                }
            }
        }
    };

    // Always release the per-URL resolve slot, including on the error paths, so
    // `stream_resolves` cannot grow unbounded with one Arc<Mutex> per failed URL.
    {
        let mut resolves = state.stream_resolves.lock().await;
        resolves.remove(url);
    }

    let resolved = resolve_result?;

    info!(
        "resolved stream url via {} in {}ms",
        resolved_source,
        start.elapsed().as_millis()
    );

    let ttl = cache_ttl_for_stream_url(&resolved);
    let mut cache = state.stream_cache.lock().await;
    // Opportunistic sweep so long-lived containers don't accumulate expired
    // entries forever.
    let now = Instant::now();
    cache.retain(|_, entry| entry.expires_at > now);
    cache.insert(
        url.to_string(),
        CachedStreamUrl {
            url: resolved.clone(),
            expires_at: Instant::now() + ttl,
        },
    );
    drop(cache);

    Ok(ResolvedStream {
        url: resolved,
        cached: false,
        source: resolved_source,
    })
}

fn cache_ttl_for_stream_url(stream_url: &str) -> Duration {
    let default_ttl = Duration::from_secs(55 * 60);
    let Ok(parsed) = url::Url::parse(stream_url) else {
        return default_ttl;
    };
    let Some(expire) = parsed
        .query_pairs()
        .find(|(key, _)| key == "expire")
        .and_then(|(_, value)| value.parse::<u64>().ok())
    else {
        return default_ttl;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    if expire <= now + 300 {
        return Duration::from_secs(5 * 60);
    }
    Duration::from_secs((expire - now - 300).min(90 * 60))
}

// Pinning a single fast player client avoids yt-dlp querying several clients
// (its default), which is markedly faster per resolve. Falls back to default
// extraction when the fast client fails. Mirrors the menubar app's resolver.
const FAST_PLAYER_CLIENT: &str = "youtube:player_client=android_vr";

async fn resolve_direct_url(state: &AppState, url: &str) -> Result<String, StreamResolveError> {
    // Prefer iOS-friendly audio first (m4a/mp4a), then generic bestaudio, then best.
    // WebM/Opus can work in some players but is flaky in iOS Safari.
    let format = "bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]/bestaudio/best";

    let cookies = std::env::var("WOPR_YTDLP_COOKIES").ok();
    let js_runtimes = std::env::var("WOPR_YTDLP_JS_RUNTIMES").ok();

    let _permit = state.ytdlp_sem.acquire().await;

    let mut last_error: Option<StreamResolveError> = None;
    for fast in [true, false] {
        let mut cmd = ytdlp_command();
        cmd.args(["-f", format, "--get-url", "--no-playlist"]);
        if fast {
            cmd.args(["--extractor-args", FAST_PLAYER_CLIENT]);
        }

        // Optional: YouTube increasingly requires a JS runtime for extraction.
        // Example: WOPR_YTDLP_JS_RUNTIMES="deno:/Users/dev/local/bin/deno"
        if let Some(v) = js_runtimes.as_deref() {
            let v = v.trim();
            if !v.is_empty() {
                cmd.args(["--js-runtimes", v]);
            }
        }

        // Optional: YouTube increasingly requires cookies to avoid bot checks.
        if let Some(path) = cookies.as_deref() {
            let path = path.trim();
            if !path.is_empty() && std::path::Path::new(path).is_file() {
                cmd.args(["--cookies", path]);
            }
        }

        cmd.arg(url);

        let out = match timeout(YTDLP_RESOLVE_TIMEOUT, cmd.output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                return Err(StreamResolveError {
                    user_message: "Could not start YouTube stream resolver.".to_string(),
                    technical_message: e.to_string(),
                    terminal: false,
                });
            }
            Err(_) => {
                return Err(StreamResolveError {
                    user_message: "YouTube stream resolver timed out.".to_string(),
                    technical_message: format!(
                        "yt-dlp exceeded {}s",
                        YTDLP_RESOLVE_TIMEOUT.as_secs()
                    ),
                    terminal: false,
                });
            }
        };

        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let direct = stdout.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            if !direct.trim().is_empty() {
                return Ok(direct.trim().to_string());
            }
            last_error = Some(StreamResolveError {
                user_message: "YouTube did not return a playable stream URL.".to_string(),
                technical_message: "empty yt-dlp output".to_string(),
                terminal: false,
            });
            continue;
        }

        let stderr = String::from_utf8_lossy(&out.stderr);
        let err = classify_ytdlp_error(&stderr);
        if err.terminal {
            // Private/unavailable/age-restricted fail identically on every
            // player client; don't burn a second resolve.
            return Err(err);
        }
        if fast {
            warn!("fast player client failed, falling back to default extraction");
        }
        last_error = Some(err);
    }

    Err(last_error.unwrap_or_else(|| StreamResolveError {
        user_message: "YouTube did not return a playable stream URL.".to_string(),
        technical_message: "empty yt-dlp output".to_string(),
        terminal: false,
    }))
}

fn classify_ytdlp_error(stderr: &str) -> StreamResolveError {
    let lower = stderr.to_lowercase();
    let known = [
        (
            "live stream recording is not available",
            "This live stream recording is not available.",
        ),
        ("private video", "This video is private and cannot be played."),
        ("video unavailable", "This video is unavailable on YouTube."),
        (
            "this video is unavailable",
            "This video is unavailable on YouTube.",
        ),
        (
            "members-only",
            "This video is members-only and cannot be played.",
        ),
        (
            "join this channel",
            "This video is members-only and cannot be played.",
        ),
        (
            "sign in to confirm your age",
            "This video is age-restricted and requires YouTube sign-in.",
        ),
        (
            "age-restricted",
            "This video is age-restricted and requires YouTube sign-in.",
        ),
    ];

    for (needle, message) in known {
        if lower.contains(needle) {
            return StreamResolveError {
                user_message: message.to_string(),
                technical_message: stderr.trim().to_string(),
                terminal: true,
            };
        }
    }

    let user_message = stderr
        .lines()
        .find_map(|line| line.trim().strip_prefix("ERROR:").map(str::trim))
        .filter(|message| !message.is_empty())
        .unwrap_or("Could not resolve stream.");

    StreamResolveError {
        user_message: user_message.to_string(),
        technical_message: stderr.trim().to_string(),
        terminal: false,
    }
}

fn extract_youtube_id(url: &str) -> Option<String> {
    // Keep it minimal: support watch?v=, youtu.be/, shorts/.
    // If it isn't YouTube, Piped won't help anyway.
    let Ok(u) = url::Url::parse(url) else { return None };
    let host = u.host_str()?.to_lowercase();
    if host == "youtu.be" {
        let id = u.path().trim_start_matches('/').split('/').next().unwrap_or("");
        if id.len() == 11 { return Some(id.to_string()) }
        return None;
    }
    if host.ends_with("youtube.com") {
        if u.path().starts_with("/shorts/") {
            let parts: Vec<&str> = u.path().split('/').collect();
            if parts.len() >= 3 && parts[2].len() == 11 {
                return Some(parts[2].to_string());
            }
        }
        for (k, v) in u.query_pairs() {
            if k == "v" && v.len() == 11 {
                return Some(v.to_string());
            }
        }
    }
    None
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PipedAudioStream {
    url: String,
    #[serde(default)]
    bitrate: u64,
    #[serde(default)]
    mime_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PipedStreamsResponse {
    #[serde(default)]
    audio_streams: Vec<PipedAudioStream>,
}

async fn resolve_via_piped(http: &reqwest::Client, url: &str) -> anyhow::Result<String> {
    let base = std::env::var("WOPR_PIPED_API_BASE").ok().unwrap_or_default();
    if base.trim().is_empty() {
        anyhow::bail!("WOPR_PIPED_API_BASE not set");
    }
    let vid = extract_youtube_id(url).context("could not extract youtube id")?;
    let endpoint = format!("{}/streams/{}", base.trim_end_matches('/'), vid);

    let resp = http
        .get(endpoint)
        .send()
        .await
        .context("piped request failed")?;
    if !resp.status().is_success() {
        anyhow::bail!("piped returned {}", resp.status());
    }
    let body: PipedStreamsResponse = resp.json().await.context("invalid piped json")?;
    if body.audio_streams.is_empty() {
        anyhow::bail!("piped returned no audio streams");
    }
    let best = body
        .audio_streams
        .into_iter()
        .filter(|s| !s.url.trim().is_empty())
        .max_by_key(|s| {
            let mime = s.mime_type.to_ascii_lowercase();
            let ios_score = if mime.contains("mp4") || mime.contains("m4a") || mime.contains("mp4a") {
                1_000_000_000
            } else {
                0
            };
            ios_score + s.bitrate
        })
        .context("piped returned no usable audio streams")?;
    if best.url.trim().is_empty() {
        anyhow::bail!("piped stream url empty");
    }
    Ok(best.url)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResult {
    title: String,
    url: String,
    video_id: String,
    thumbnail: String,
    channel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: String,
    token: Option<String>,
}

fn parse_search_line(line: &str) -> Option<SearchResult> {
    let v = serde_json::from_str::<serde_json::Value>(line).ok()?;
    let id = v.get("id").and_then(|x| x.as_str())?;
    let title = v
        .get("title")
        .and_then(|x| x.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let channel = v
        .get("channel")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("uploader").and_then(|x| x.as_str()))
        .unwrap_or("")
        .to_string();
    Some(SearchResult {
        title,
        url: format!("https://www.youtube.com/watch?v={}", id),
        video_id: id.to_string(),
        thumbnail: format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", id),
        channel,
        duration: v.get("duration").and_then(|x| x.as_f64()),
    })
}

async fn run_ytsearch(state: &AppState, search_arg: &str, limit: usize) -> Result<Vec<SearchResult>, StatusCode> {
    let _permit = state.ytdlp_sem.acquire().await;

    // The fast pinned player client speeds search up the same way it speeds
    // resolves up; fall back to default extraction if it returns nothing.
    for fast in [true, false] {
        let mut cmd = ytdlp_command();
        if fast {
            cmd.args(["--extractor-args", FAST_PLAYER_CLIENT]);
        }
        cmd.args(["--flat-playlist", "--dump-json", search_arg]);
        let output = match timeout(YTDLP_SEARCH_TIMEOUT, cmd.output()).await {
            Ok(Ok(o)) => o,
            Ok(Err(_)) => return Err(StatusCode::BAD_GATEWAY),
            Err(_) => return Err(StatusCode::GATEWAY_TIMEOUT),
        };
        if !output.status.success() {
            if fast {
                continue;
            }
            return Err(StatusCode::BAD_GATEWAY);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let results: Vec<SearchResult> = stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(parse_search_line)
            .take(limit)
            .collect();
        if !results.is_empty() || !fast {
            return Ok(results);
        }
    }
    Ok(Vec::new())
}

async fn search(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<SearchQuery>) -> impl IntoResponse {
    if !state.authorized(&headers, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let query = q.q.trim().to_lowercase();
    if query.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing q").into_response();
    }

    {
        let mut cache = state.search_cache.lock().await;
        let now = Instant::now();
        cache.retain(|_, (cached_at, _)| now.duration_since(*cached_at) < SEARCH_CACHE_TTL);
        if let Some((_, results)) = cache.get(&query) {
            return Json(results.clone()).into_response();
        }
    }

    let results = match run_ytsearch(&state, &format!("ytsearch10:{} music", query), 10).await {
        Ok(results) => results,
        Err(StatusCode::GATEWAY_TIMEOUT) => {
            return (StatusCode::GATEWAY_TIMEOUT, "Search timed out.").into_response()
        }
        Err(code) => return code.into_response(),
    };

    if !results.is_empty() {
        let mut cache = state.search_cache.lock().await;
        cache.insert(query, (Instant::now(), results.clone()));
    }

    Json(results).into_response()
}

#[derive(Debug, Deserialize)]
struct SpotifyQueueQuery {
    url: String,
    token: Option<String>,
    #[serde(default)]
    max_tracks: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpotifyQueueItem {
    url: String,
    video_id: String,
    title: String,
    thumbnail: String,
    channel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpotifyQueueResponse {
    title: String,
    thumbnail: String,
    source_count: usize,
    matched_count: usize,
    items: Vec<SpotifyQueueItem>,
}

fn spotify_track_limit(requested: Option<usize>) -> usize {
    let default = std::env::var("WOPR_SPOTIFY_MAX_TRACKS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(75);
    requested.filter(|v| *v > 0).map_or(default, |v| v.min(default))
}

// Convert a Spotify playlist/album/track into a YouTube-backed queue. Track
// searches run with bounded parallelism; individual misses are skipped rather
// than failing the whole conversion.
async fn spotify_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<SpotifyQueueQuery>,
) -> impl IntoResponse {
    if !state.authorized(&headers, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !spotify::is_spotify_url(&q.url) {
        return (StatusCode::BAD_REQUEST, "Not a Spotify URL").into_response();
    }

    let source = match spotify::fetch_source(&state.http, &q.url).await {
        Ok(source) => source,
        Err(message) => {
            error!("spotify fetch failed: {message}");
            return (StatusCode::BAD_GATEWAY, message).into_response();
        }
    };

    let limit = spotify_track_limit(q.max_tracks);
    let source_count = source.tracks.len();
    let tracks: Vec<spotify::TrackMeta> = source.tracks.into_iter().take(limit).collect();

    let mut join_set: JoinSet<(usize, Option<SpotifyQueueItem>)> = JoinSet::new();
    let match_sem = Arc::new(Semaphore::new(SPOTIFY_MATCH_CONCURRENCY));
    for (index, track) in tracks.into_iter().enumerate() {
        let state = state.clone();
        let match_sem = match_sem.clone();
        join_set.spawn(async move {
            let _slot = match_sem.acquire().await;
            let query = format!("ytsearch1:{}", track.search_query());
            let item = match run_ytsearch(&state, &query, 1).await {
                Ok(results) => results.into_iter().next().map(|r| SpotifyQueueItem {
                    url: r.url,
                    video_id: r.video_id,
                    title: track.display_title(),
                    thumbnail: r.thumbnail,
                    channel: track.artist(),
                    duration: r.duration,
                }),
                Err(_) => {
                    warn!("spotify match search failed for {:?}; skipping", track.display_title());
                    None
                }
            };
            (index, item)
        });
    }

    let mut matched: Vec<(usize, SpotifyQueueItem)> = Vec::new();
    while let Some(result) = join_set.join_next().await {
        if let Ok((index, Some(item))) = result {
            matched.push((index, item));
        }
    }
    matched.sort_by_key(|(index, _)| *index);
    let items: Vec<SpotifyQueueItem> = matched.into_iter().map(|(_, item)| item).collect();

    if items.is_empty() {
        return (
            StatusCode::BAD_GATEWAY,
            "Could not find YouTube matches for this Spotify URL.",
        )
            .into_response();
    }

    let title = if source_count > limit {
        format!("{} (first {} tracks)", source.title, limit)
    } else {
        source.title
    };

    Json(SpotifyQueueResponse {
        title,
        thumbnail: source.thumbnail,
        source_count,
        matched_count: items.len(),
        items,
    })
    .into_response()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let data_dir = std::env::var("WOPR_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./data"));
    let bearer_token = std::env::var("WOPR_BEARER_TOKEN")
        .ok()
        .filter(|t| !t.trim().is_empty());
    let allow_no_auth = std::env::var("WOPR_ALLOW_NO_AUTH")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if bearer_token.is_none() && !allow_no_auth {
        anyhow::bail!(
            "WOPR_BEARER_TOKEN is not set: refusing to start an unauthenticated server that can \
             control local playback and run yt-dlp. Set WOPR_BEARER_TOKEN, or set \
             WOPR_ALLOW_NO_AUTH=1 to deliberately run open (e.g. bound to a trusted private network)."
        );
    }
    if bearer_token.is_none() {
        tracing::warn!(
            "WOPR_ALLOW_NO_AUTH is enabled: running with NO authentication; all endpoints are public."
        );
    }

    // PWA session-cookie signing key. Falls back to the bearer token so a
    // dedicated secret is optional; either way it's stable across restarts.
    let session_secret: Vec<u8> = std::env::var("WOPR_SESSION_SECRET")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.into_bytes())
        .or_else(|| bearer_token.clone().map(|t| t.into_bytes()))
        .unwrap_or_else(|| b"bkgrnd-insecure-dev-session-secret".to_vec());
    let registration_open = std::env::var("WOPR_REGISTRATION_OPEN")
        .map(|v| !(v == "0" || v.eq_ignore_ascii_case("false")))
        .unwrap_or(true);
    let users = auth::load_users(&data_dir);
    info!(
        "pwa auth: {} user(s) loaded, registration {}",
        users.len(),
        if registration_open { "OPEN" } else { "closed" }
    );

    let http = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .context("failed to build reqwest client")?;

    let state = AppState {
        data_dir,
        bearer_token,
        playlists_lock: Arc::new(Mutex::new(())),
        http,
        stream_cache: Arc::new(Mutex::new(HashMap::new())),
        stream_failures: Arc::new(Mutex::new(HashMap::new())),
        stream_resolves: Arc::new(Mutex::new(HashMap::new())),
        search_cache: Arc::new(Mutex::new(HashMap::new())),
        ytdlp_sem: Arc::new(Semaphore::new(YTDLP_MAX_CONCURRENCY)),
        local_status: Arc::new(Mutex::new(None)),
        local_commands: Arc::new(Mutex::new(VecDeque::new())),
        next_local_command_id: Arc::new(Mutex::new(1)),
        command_notify: Arc::new(Notify::new()),
        session_secret: Arc::new(session_secret),
        users: Arc::new(Mutex::new(users)),
        registration_open,
    };

    let web_dir = find_web_dir();
    info!("serving web ui from {}", web_dir.display());

    let app = Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/pwa/register", post(pwa_register))
        .route("/api/v1/pwa/login", post(pwa_login))
        .route("/api/v1/pwa/logout", post(pwa_logout))
        .route("/api/v1/pwa/me", get(pwa_me))
        .route("/api/v1/pwa/cast-sig", get(pwa_cast_sig))
        .route("/api/v1/playlists", get(get_playlists).put(put_playlists))
        .route("/api/v1/playlists.json", get(get_playlists_json).put(put_playlists_json))
        .route("/api/v1/history.json", get(get_history_json).put(put_history_json))
        .route("/api/v1/local/status", get(get_local_status).put(put_local_status))
        .route("/api/v1/local/commands", post(post_local_command))
        .route("/api/v1/local/commands/next", get(get_next_local_command))
        .route("/api/v1/search", get(search))
        .route("/api/v1/spotify/queue", get(spotify_queue))
        .route("/api/v1/resolve", get(resolve_stream))
        .route("/api/v1/meta", get(video_meta))
        .route("/api/v1/prewarm", get(prewarm_stream))
        .route("/api/v1/stream", get(stream_audio))
        .route("/api/v1/thumbnail", get(thumbnail_image))
        // Serve the stopgap web app from the same origin to avoid CORS hassles.
        .fallback_service(ServeDir::new(web_dir).append_index_html_on_directories(true))
        // History/playlist PUTs are client-supplied JSON/YAML; cap them so a
        // buggy or hostile client can't fill the disk.
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(state);

    let addr: SocketAddr = std::env::var("WOPR_BIND")
        .unwrap_or_else(|_| "127.0.0.1:18081".to_string())
        .parse()
        .context("invalid WOPR_BIND")?;

    info!("wopr server listening on {addr}");
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app)
        .await
        .context("server error")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_range_headers() {
        assert_eq!(parse_range_start_end(None), (0, None));
        assert_eq!(parse_range_start_end(Some("bytes=0-")), (0, None));
        assert_eq!(parse_range_start_end(Some("bytes=100-200")), (100, Some(200)));
        assert_eq!(parse_range_start_end(Some("bytes=100-")), (100, None));
        // Suffix and malformed ranges fall back to the start of the file.
        assert_eq!(parse_range_start_end(Some("bytes=-500")), (0, None));
        assert_eq!(parse_range_start_end(Some("garbage")), (0, None));
        // Only the first range of a multi-range request is honored.
        assert_eq!(parse_range_start_end(Some("bytes=5-9,20-30")), (5, Some(9)));
    }

    #[test]
    fn extracts_youtube_ids() {
        assert_eq!(
            extract_youtube_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
        assert_eq!(
            extract_youtube_id("https://youtu.be/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
        assert_eq!(
            extract_youtube_id("https://www.youtube.com/shorts/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
        assert_eq!(extract_youtube_id("https://example.com/watch?v=dQw4w9WgXcQ"), None);
        assert_eq!(extract_youtube_id("not a url"), None);
    }

    #[test]
    fn classifies_ytdlp_errors() {
        let terminal = classify_ytdlp_error("ERROR: Private video. Sign in.");
        assert!(terminal.terminal);
        assert_eq!(terminal.user_message, "This video is private and cannot be played.");

        let transient = classify_ytdlp_error("ERROR: HTTP Error 429: Too Many Requests");
        assert!(!transient.terminal);
        assert_eq!(transient.user_message, "HTTP Error 429: Too Many Requests");

        let unknown = classify_ytdlp_error("something exploded");
        assert!(!unknown.terminal);
        assert_eq!(unknown.user_message, "Could not resolve stream.");
    }

    #[test]
    fn stream_url_ttl_honors_expire_param() {
        // No expire param -> default 55 minutes.
        assert_eq!(
            cache_ttl_for_stream_url("https://example.com/audio"),
            Duration::from_secs(55 * 60)
        );

        // Already-expired URL -> short 5 minute TTL.
        let past = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 100;
        assert_eq!(
            cache_ttl_for_stream_url(&format!("https://example.com/audio?expire={past}")),
            Duration::from_secs(5 * 60)
        );

        // Far-future expiry is clamped to 90 minutes.
        let future = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 10 * 60 * 60;
        assert_eq!(
            cache_ttl_for_stream_url(&format!("https://example.com/audio?expire={future}")),
            Duration::from_secs(90 * 60)
        );
    }

    #[test]
    fn parses_content_range_totals() {
        assert_eq!(parse_content_range_total("bytes 0-99/1234"), Some(1234));
        assert_eq!(parse_content_range_total("bytes 0-99/*"), None);
        assert_eq!(parse_content_range_total("garbage"), None);
    }

    #[test]
    fn parses_search_output_lines() {
        let line = r#"{"id":"dQw4w9WgXcQ","title":"Song","channel":"Artist","duration":213.0}"#;
        let result = parse_search_line(line).unwrap();
        assert_eq!(result.video_id, "dQw4w9WgXcQ");
        assert_eq!(result.url, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
        assert_eq!(result.channel, "Artist");
        assert_eq!(result.duration, Some(213.0));

        assert!(parse_search_line("not json").is_none());
        assert!(parse_search_line(r#"{"title":"no id"}"#).is_none());
    }

    #[test]
    fn spotify_limit_respects_env_default_and_request() {
        // Request below default is honored; zero/None falls back to default.
        assert_eq!(spotify_track_limit(Some(10)), 10);
        assert_eq!(spotify_track_limit(Some(0)), 75);
        assert_eq!(spotify_track_limit(None), 75);
        // Requests can't exceed the server-side ceiling.
        assert_eq!(spotify_track_limit(Some(10_000)), 75);
    }

    #[test]
    fn auth_accepts_header_or_query_token() {
        let token = Some("secret".to_string());
        let mut headers = HeaderMap::new();

        assert!(!auth_ok(&headers, &token, None));
        assert!(auth_ok(&headers, &token, Some("secret")));
        assert!(!auth_ok(&headers, &token, Some("wrong")));

        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer secret"));
        assert!(auth_ok(&headers, &token, None));

        // No configured token -> open.
        assert!(auth_ok(&HeaderMap::new(), &None, None));
    }
}
