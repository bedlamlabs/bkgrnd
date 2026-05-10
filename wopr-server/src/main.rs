use anyhow::Context;
use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, HeaderMap, HeaderValue, Response, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use tracing::{error, info};

#[derive(Clone)]
struct AppState {
    data_dir: PathBuf,
    bearer_token: Option<String>,
    playlists_lock: Arc<Mutex<()>>,
    http: reqwest::Client,
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

fn playlists_path(data_dir: &PathBuf) -> PathBuf {
    data_dir.join("playlists.yaml")
}

fn auth_ok(headers: &HeaderMap, token: &Option<String>) -> bool {
    let Some(expected) = token else {
        return true;
    };
    let Some(auth) = headers.get(header::AUTHORIZATION) else {
        return false;
    };
    let Ok(s) = auth.to_str() else {
        return false;
    };
    s == format!("Bearer {}", expected)
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn get_playlists(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !auth_ok(&headers, &state.bearer_token) {
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

async fn get_playlists_json(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !auth_ok(&headers, &state.bearer_token) {
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
    body: Bytes,
) -> impl IntoResponse {
    if !auth_ok(&headers, &state.bearer_token) {
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
    body: Bytes,
) -> impl IntoResponse {
    if !auth_ok(&headers, &state.bearer_token) {
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

#[derive(Debug, Deserialize)]
struct StreamQuery {
    url: String,
}

// Stable-ish endpoint: client asks for /stream?url=... and we resolve a fresh direct media URL and proxy bytes.
// For a production-grade design we'd support resume/range + caching; keep minimal for personal use.
async fn stream_audio(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<StreamQuery>,
) -> impl IntoResponse {
    if !auth_ok(&headers, &state.bearer_token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    // Call yt-dlp to resolve direct URL. We expect yt-dlp installed on WOPR.
    let direct_url = match resolve_direct_url(&q.url).await {
        Ok(u) => u,
        Err(e) => {
            error!("resolve_direct_url failed: {e:#}");
            return (StatusCode::BAD_REQUEST, "Could not resolve stream").into_response();
        }
    };

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

async fn resolve_direct_url(url: &str) -> anyhow::Result<String> {
    // Prefer bestaudio; yt-dlp prints one URL with --get-url.
    let out = tokio::process::Command::new("yt-dlp")
        .args(["-f", "bestaudio", "--get-url", "--no-playlist", url])
        .output()
        .await
        .context("failed to run yt-dlp")?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("yt-dlp failed: {}", err.trim());
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let direct = stdout.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    if direct.is_empty() {
        anyhow::bail!("yt-dlp returned empty url");
    }
    Ok(direct.trim().to_string())
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
}

async fn search(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<SearchQuery>) -> impl IntoResponse {
    if !auth_ok(&headers, &state.bearer_token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let query = q.q.trim();
    if query.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing q").into_response();
    }

    let output = match tokio::process::Command::new("yt-dlp")
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
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("failed to build reqwest client")?;

    let state = AppState {
        data_dir,
        bearer_token,
        playlists_lock: Arc::new(Mutex::new(())),
        http,
    };

    let app = Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/playlists", get(get_playlists).put(put_playlists))
        .route("/api/v1/playlists.json", get(get_playlists_json).put(put_playlists_json))
        .route("/api/v1/search", get(search))
        .route("/api/v1/stream", get(stream_audio))
        .with_state(state);

    let addr: SocketAddr = std::env::var("WOPR_BIND")
        .unwrap_or_else(|_| "0.0.0.0:808".to_string())
        .parse()
        .context("invalid WOPR_BIND")?;

    info!("wopr server listening on {addr}");
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app)
        .await
        .context("server error")?;

    Ok(())
}
