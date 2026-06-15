use anyhow::Context;
use axum::{
    body::Body,
    extract::{Json, Query, State},
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
use tokio::sync::Mutex;
use tracing::{error, info};
use tower_http::services::ServeDir;

#[derive(Clone)]
struct AppState {
    data_dir: PathBuf,
    bearer_token: Option<String>,
    playlists_lock: Arc<Mutex<()>>,
    http: reqwest::Client,
    stream_cache: Arc<Mutex<HashMap<String, CachedStreamUrl>>>,
    stream_failures: Arc<Mutex<HashMap<String, CachedStreamFailure>>>,
    stream_resolves: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    local_status: Arc<Mutex<Option<LocalStatusRecord>>>,
    local_commands: Arc<Mutex<VecDeque<LocalPlaybackCommand>>>,
    next_local_command_id: Arc<Mutex<u64>>,
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
    if !auth_ok(&headers, &state.bearer_token, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let _guard = state.playlists_lock.lock().await;
    let path = playlists_path(&state.data_dir);
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
    if !auth_ok(&headers, &state.bearer_token, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let _guard = state.playlists_lock.lock().await;
    let path = playlists_path(&state.data_dir);
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
    if !auth_ok(&headers, &state.bearer_token, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    // Validate YAML
    let raw = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UTF-8").into_response(),
    };
    if serde_yaml::from_str::<PlaylistDoc>(raw).is_err() {
        return (StatusCode::BAD_REQUEST, "Invalid playlist YAML").into_response();
    }

    let _guard = state.playlists_lock.lock().await;
    let path = playlists_path(&state.data_dir);
    if let Err(e) = tokio::fs::create_dir_all(&state.data_dir).await {
        error!("failed to create data dir: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if let Err(e) = tokio::fs::write(&path, raw).await {
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
    if !auth_ok(&headers, &state.bearer_token, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let doc: PlaylistDoc = match serde_json::from_slice(&body) {
        Ok(d) => d,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid playlist JSON").into_response(),
    };
    let raw = serde_yaml::to_string(&doc).unwrap_or_default();

    let _guard = state.playlists_lock.lock().await;
    let path = playlists_path(&state.data_dir);
    if let Err(e) = tokio::fs::create_dir_all(&state.data_dir).await {
        error!("failed to create data dir: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if let Err(e) = tokio::fs::write(&path, raw).await {
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
    if !auth_ok(&headers, &state.bearer_token, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let path = history_path(&state.data_dir);
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
    if !auth_ok(&headers, &state.bearer_token, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    // Validate JSON array (history entries are client-defined; keep it permissive).
    if serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .is_none()
    {
        return (StatusCode::BAD_REQUEST, "Invalid history JSON").into_response();
    }

    if let Err(e) = tokio::fs::create_dir_all(&state.data_dir).await {
        error!("failed to create data dir: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let path = history_path(&state.data_dir);
    if let Err(e) = tokio::fs::write(&path, &body).await {
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
    if !auth_ok(&headers, &state.bearer_token, q.token.as_deref()) {
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
    if !auth_ok(&headers, &state.bearer_token, q.token.as_deref()) {
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
    if !auth_ok(&headers, &state.bearer_token, q.token.as_deref()) {
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
    commands.push_back(command.clone());
    while commands.len() > 50 {
        commands.pop_front();
    }

    Json(command).into_response()
}

async fn get_next_local_command(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if !auth_ok(&headers, &state.bearer_token, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let mut commands = state.local_commands.lock().await;
    if let Some(command) = commands.pop_front() {
        Json(Some(command)).into_response()
    } else {
        Json(Option::<LocalPlaybackCommand>::None).into_response()
    }
}

#[derive(Debug, Deserialize)]
struct StreamQuery {
    url: String,
    token: Option<String>,
    proxy: Option<bool>,
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

async fn resolve_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ResolveQuery>,
) -> impl IntoResponse {
    if !auth_ok(&headers, &state.bearer_token, q.token.as_deref()) {
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
    if !auth_ok(&headers, &state.bearer_token, q.token.as_deref()) {
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
    if !auth_ok(&headers, &state.bearer_token, q.token.as_deref()) {
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

// Stable-ish endpoint: client asks for /stream?url=... and we resolve a fresh direct media URL and proxy bytes.
// For a production-grade design we'd support resume/range + caching; keep minimal for personal use.
async fn stream_audio(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<StreamQuery>,
) -> impl IntoResponse {
    if !auth_ok(&headers, &state.bearer_token, q.token.as_deref()) {
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
    let mut req = state.http.get(direct_url);
    if let Some(range) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        req = req.header(header::RANGE, range);
    }

    let upstream = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            error!("upstream fetch failed: {e}");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    if !(upstream.status().is_success() || upstream.status().as_u16() == 206) {
        return StatusCode::BAD_GATEWAY.into_response();
    }

    // NOTE: no range support here; keep it simple.
    let content_type = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/mpeg")
        .to_string();

    let status = upstream.status();
    let content_length = upstream.headers().get(header::CONTENT_LENGTH).cloned();
    let content_range = upstream.headers().get(header::CONTENT_RANGE).cloned();

    let stream = upstream.bytes_stream();
    let body = Body::from_stream(stream);
    let mut resp = Response::builder().status(status).body(body).unwrap();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type).unwrap_or(HeaderValue::from_static("audio/mpeg")),
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
    let resolved = match resolve_direct_url(url).await {
        Ok(url) => Ok(url),
        Err(e) if e.terminal => {
            let mut failures = state.stream_failures.lock().await;
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
            let fallback = resolve_via_piped(&state.http, url).await.map_err(|fallback_error| {
                error!("resolve_via_piped failed: {fallback_error:#}");
                primary_error
            })?;
            resolved_source = "piped".to_string();
            Ok(fallback)
        }
    }?;

    info!(
        "resolved stream url via {} in {}ms",
        resolved_source,
        start.elapsed().as_millis()
    );

    let ttl = cache_ttl_for_stream_url(&resolved);
    let mut cache = state.stream_cache.lock().await;
    cache.insert(
        url.to_string(),
        CachedStreamUrl {
            url: resolved.clone(),
            expires_at: Instant::now() + ttl,
        },
    );
    drop(cache);

    let mut resolves = state.stream_resolves.lock().await;
    resolves.remove(url);

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

async fn resolve_direct_url(url: &str) -> Result<String, StreamResolveError> {
    // Prefer iOS-friendly audio first (m4a/mp4a), then generic bestaudio, then best.
    // WebM/Opus can work in some players but is flaky in iOS Safari.
    let format = "bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]/bestaudio/best";

    let cookies = std::env::var("WOPR_YTDLP_COOKIES").ok();
    let js_runtimes = std::env::var("WOPR_YTDLP_JS_RUNTIMES").ok();

    let mut cmd = ytdlp_command();
    cmd.args(["-f", format, "--get-url", "--no-playlist"]);

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

    let out = cmd.output().await.map_err(|e| StreamResolveError {
        user_message: "Could not start YouTube stream resolver.".to_string(),
        technical_message: e.to_string(),
        terminal: false,
    })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(classify_ytdlp_error(&stderr));
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let direct = stdout.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    if !direct.trim().is_empty() {
        return Ok(direct.trim().to_string());
    }

    Err(StreamResolveError {
        user_message: "YouTube did not return a playable stream URL.".to_string(),
        technical_message: "empty yt-dlp output".to_string(),
        terminal: false,
    })
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResult {
    title: String,
    url: String,
    video_id: String,
    thumbnail: String,
    channel: String,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: String,
    token: Option<String>,
}

async fn search(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<SearchQuery>) -> impl IntoResponse {
    if !auth_ok(&headers, &state.bearer_token, q.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let query = q.q.trim();
    if query.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing q").into_response();
    }

    let output = match ytdlp_command()
        .args(["--flat-playlist", "--dump-json", &format!("ytsearch10:{} music", query)])
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };

    if !output.status.success() {
        return StatusCode::BAD_GATEWAY.into_response();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut results: Vec<SearchResult> = Vec::new();

    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        if results.len() >= 10 {
            break;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let Some(id) = v.get("id").and_then(|x| x.as_str()) else { continue };
        let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("Unknown").to_string();
        let channel = v.get("channel").and_then(|x| x.as_str())
            .or_else(|| v.get("uploader").and_then(|x| x.as_str()))
            .unwrap_or("")
            .to_string();
        let url = format!("https://www.youtube.com/watch?v={}", id);
        let thumbnail = format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", id);
        results.push(SearchResult {
            title,
            url: url.clone(),
            video_id: id.to_string(),
            thumbnail,
            channel,
        });
    }

    match serde_json::to_vec(&results) {
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
    let bearer_token = std::env::var("WOPR_BEARER_TOKEN").ok();

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
        local_status: Arc::new(Mutex::new(None)),
        local_commands: Arc::new(Mutex::new(VecDeque::new())),
        next_local_command_id: Arc::new(Mutex::new(1)),
    };

    let web_dir = find_web_dir();
    info!("serving web ui from {}", web_dir.display());

    let app = Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/playlists", get(get_playlists).put(put_playlists))
        .route("/api/v1/playlists.json", get(get_playlists_json).put(put_playlists_json))
        .route("/api/v1/history.json", get(get_history_json).put(put_history_json))
        .route("/api/v1/local/status", get(get_local_status).put(put_local_status))
        .route("/api/v1/local/commands", post(post_local_command))
        .route("/api/v1/local/commands/next", get(get_next_local_command))
        .route("/api/v1/search", get(search))
        .route("/api/v1/resolve", get(resolve_stream))
        .route("/api/v1/prewarm", get(prewarm_stream))
        .route("/api/v1/stream", get(stream_audio))
        .route("/api/v1/thumbnail", get(thumbnail_image))
        // Serve the stopgap web app from the same origin to avoid CORS hassles.
        .fallback_service(ServeDir::new(web_dir).append_index_html_on_directories(true))
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
