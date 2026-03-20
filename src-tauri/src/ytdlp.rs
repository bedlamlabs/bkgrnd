use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;

pub fn extract_video_id(url_str: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url_str) {
        let host = parsed.host_str().unwrap_or("");

        if host.contains("youtube.com") {
            // youtube.com/watch?v=ID
            for (key, val) in parsed.query_pairs() {
                if key == "v" && val.len() == 11 {
                    return val.to_string();
                }
            }
            let path = parsed.path();
            // /embed/ID or /v/ID or /shorts/ID
            let patterns = ["/embed/", "/v/", "/shorts/"];
            for pat in patterns {
                if let Some(rest) = path.strip_prefix(pat) {
                    let id = rest.split('/').next().unwrap_or("");
                    if id.len() == 11 {
                        return id.to_string();
                    }
                }
            }
        }

        if host == "youtu.be" {
            let id = parsed.path().trim_start_matches('/').split('/').next().unwrap_or("");
            if id.len() == 11 {
                return id.to_string();
            }
        }
    }

    // Fallback regex
    let re = regex::Regex::new(r"(?:v=|/|youtu\.be/)([a-zA-Z0-9_-]{11})").unwrap();
    if let Some(caps) = re.captures(url_str) {
        return caps[1].to_string();
    }

    String::new()
}

pub fn thumbnail_url(video_id: &str) -> String {
    if video_id.is_empty() {
        return String::new();
    }
    format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", video_id)
}

pub fn is_playlist_url(url_str: &str) -> bool {
    if let Ok(parsed) = url::Url::parse(url_str) {
        let has_list = parsed.query_pairs().any(|(k, _)| k == "list");
        let is_playlist_path = parsed.path() == "/playlist";
        return has_list || is_playlist_path;
    }
    false
}

pub async fn extract_stream_url(app: &AppHandle, url: &str) -> Result<String, String> {
    let formats = ["bestaudio", "best"];
    for fmt in formats {
        eprintln!("[ytdlp] Trying format {} for {}", fmt, url);
        let output = app
            .shell()
            .sidecar("yt-dlp")
            .map_err(|e| {
                eprintln!("[ytdlp] Sidecar creation failed: {}", e);
                format!("Failed to create yt-dlp sidecar: {}", e)
            })?
            .args(["-f", fmt, "--get-url", "--no-playlist", url])
            .output()
            .await
            .map_err(|e| {
                eprintln!("[ytdlp] Spawn failed: {}", e);
                format!("yt-dlp spawn failed: {}", e)
            })?;

        if output.status.success() {
            let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !result.is_empty() {
                eprintln!("[ytdlp] Got stream URL ({} chars)", result.len());
                return Ok(result);
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("[ytdlp] Format {} failed: {}", fmt, stderr.trim());
        }
    }
    Err("yt-dlp could not extract any stream URL".to_string())
}

#[derive(Debug)]
pub struct VideoInfo {
    pub title: String,
    pub is_live: bool,
    pub video_id: String,
}

pub async fn get_video_info(app: &AppHandle, url: &str) -> VideoInfo {
    let output = app
        .shell()
        .sidecar("yt-dlp")
        .ok()
        .map(|cmd| cmd.args(["--dump-json", "--no-playlist", "--skip-download", url]));

    if let Some(cmd) = output {
        if let Ok(output) = cmd.output().await {
            if output.status.success() {
                let json_str = String::from_utf8_lossy(&output.stdout);
                if let Ok(info) = serde_json::from_str::<serde_json::Value>(json_str.trim()) {
                    return VideoInfo {
                        title: info["title"].as_str().unwrap_or("Unknown").to_string(),
                        is_live: info["is_live"].as_bool().unwrap_or(false),
                        video_id: info["id"]
                            .as_str()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| extract_video_id(url)),
                    };
                }
            }
        }
    }

    VideoInfo {
        title: "Unknown".to_string(),
        is_live: false,
        video_id: extract_video_id(url),
    }
}

#[derive(Debug, Clone)]
pub struct PlaylistItem {
    pub url: String,
    pub video_id: String,
    pub title: String,
    pub thumbnail: String,
}

pub struct PlaylistResult {
    pub items: Vec<PlaylistItem>,
    pub title: String,
}

pub async fn enumerate_playlist(app: &AppHandle, url: &str) -> Result<PlaylistResult, String> {
    let output = app
        .shell()
        .sidecar("yt-dlp")
        .map_err(|e| format!("Failed to create yt-dlp sidecar: {}", e))?
        .args(["--flat-playlist", "--dump-json", url])
        .output()
        .await
        .map_err(|e| format!("yt-dlp enumerate failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("yt-dlp failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut items = Vec::new();
    let mut pl_title = String::new();

    for line in stdout.lines().filter(|l| !l.is_empty()) {
        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
            let id = match entry["id"].as_str() {
                Some(id) => id,
                None => continue,
            };

            if pl_title.is_empty() {
                if let Some(t) = entry["playlist_title"].as_str() {
                    pl_title = t.to_string();
                }
            }

            items.push(PlaylistItem {
                url: format!("https://www.youtube.com/watch?v={}", id),
                video_id: id.to_string(),
                title: entry["title"].as_str().unwrap_or("Unknown").to_string(),
                thumbnail: thumbnail_url(id),
            });
        }
    }

    Ok(PlaylistResult {
        items,
        title: pl_title,
    })
}
