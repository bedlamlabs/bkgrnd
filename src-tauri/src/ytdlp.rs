use std::path::PathBuf;
use std::sync::OnceLock;
use tauri::AppHandle;
use tokio::process::Command;

use crate::config;

// Prefer a native yt-dlp install over the bundled sidecar: the sidecar is a
// PyInstaller onefile binary that spends ~7s self-extracting on EVERY
// invocation, while a native install starts in ~0.3s. The sidecar remains the
// zero-setup fallback.
fn ytdlp_binary() -> &'static PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let cfg = config::load_config();
        if let Some(bin) = config::setting("BKGRND_YTDLP_BIN", cfg.ytdlp_bin.as_deref()) {
            let path = PathBuf::from(&bin);
            if path.is_file() {
                return path;
            }
            eprintln!("[ytdlp] configured ytdlp_bin {:?} not found; falling back", bin);
        }

        let mut candidates: Vec<PathBuf> = vec![
            PathBuf::from("/opt/homebrew/bin/yt-dlp"),
            PathBuf::from("/usr/local/bin/yt-dlp"),
        ];
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".local/bin/yt-dlp"));
        }
        for candidate in candidates {
            if candidate.is_file() {
                eprintln!("[ytdlp] using native binary {:?}", candidate);
                return candidate;
            }
        }

        // Bundled sidecar sits next to the app executable (Contents/MacOS on
        // macOS; the install dir on Windows). Tauri names it per-platform.
        #[cfg(windows)]
        let sidecar_name = "yt-dlp.exe";
        #[cfg(not(windows))]
        let sidecar_name = "yt-dlp";
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let sidecar = dir.join(sidecar_name);
                if sidecar.is_file() {
                    eprintln!("[ytdlp] using bundled sidecar (slow startup; install a native yt-dlp for faster resolves)");
                    return sidecar;
                }
            }
        }
        PathBuf::from(sidecar_name)
    })
}

fn ytdlp_command(args: &[&str]) -> Command {
    let mut cmd = Command::new(ytdlp_binary());
    cmd.args(args);
    cmd
}

fn ytdlp_command_owned(args: &[String]) -> Command {
    let mut cmd = Command::new(ytdlp_binary());
    cmd.args(args);
    cmd
}

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
            let id = parsed
                .path()
                .trim_start_matches('/')
                .split('/')
                .next()
                .unwrap_or("");
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

// The legacy resolver still pins a single fast client before trying yt-dlp's
// default client set. It is deliberately last: current YouTube GVS sessions
// can resolve successfully and then lose authorization for later live HLS
// segments.
const FAST_PLAYER_CLIENT: &str = "youtube:player_client=android_vr";
const POT_PLAYER_CLIENT: &str = "youtube:player_client=mweb";
const EMBEDDED_PLAYER_CLIENT: &str = "youtube:player_client=web_embedded";
const POT_PROVIDER_ARGS: &str = "youtubepot-bgutilhttp:base_url=http://127.0.0.1:4416";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolverStrategy {
    PotProvider,
    WebEmbedded,
    Legacy,
}

impl ResolverStrategy {
    pub fn label(self) -> &'static str {
        match self {
            Self::PotProvider => "pot-provider",
            Self::WebEmbedded => "web-embedded",
            Self::Legacy => "legacy",
        }
    }

    pub fn next(self) -> Option<Self> {
        let order = resolver_strategy_order();
        order
            .iter()
            .position(|candidate| *candidate == self)
            .and_then(|index| order.get(index + 1).copied())
    }
}

pub fn resolver_strategy_order() -> [ResolverStrategy; 3] {
    [
        ResolverStrategy::PotProvider,
        ResolverStrategy::WebEmbedded,
        ResolverStrategy::Legacy,
    ]
}

fn pot_plugin_dir() -> PathBuf {
    std::env::var("BKGRND_YTDLP_PLUGIN_DIR")
        .ok()
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".bkgrnd/yt-dlp-plugins")))
        .unwrap_or_else(|| PathBuf::from(".bkgrnd/yt-dlp-plugins"))
}

fn strategy_argument_sets(strategy: ResolverStrategy) -> Vec<Vec<String>> {
    match strategy {
        ResolverStrategy::PotProvider => {
            let plugin_dir = pot_plugin_dir();
            vec![vec![
                "--plugin-dirs".to_string(),
                plugin_dir.to_string_lossy().into_owned(),
                "--extractor-args".to_string(),
                POT_PROVIDER_ARGS.to_string(),
                "--extractor-args".to_string(),
                POT_PLAYER_CLIENT.to_string(),
            ]]
        }
        ResolverStrategy::WebEmbedded => vec![vec![
            "--extractor-args".to_string(),
            EMBEDDED_PLAYER_CLIENT.to_string(),
        ]],
        ResolverStrategy::Legacy => vec![
            vec![
                "--extractor-args".to_string(),
                FAST_PLAYER_CLIENT.to_string(),
            ],
            Vec::new(),
        ],
    }
}

fn js_runtime_arg() -> Option<String> {
    std::env::var("BKGRND_YTDLP_JS_RUNTIMES")
        .ok()
        .or_else(|| std::env::var("WOPR_YTDLP_JS_RUNTIMES").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let deno = PathBuf::from("/opt/homebrew/bin/deno");
            deno.is_file()
                .then(|| format!("deno:{}", deno.to_string_lossy()))
        })
}

#[derive(Debug)]
pub struct StreamInfo {
    pub stream_url: String,
    pub title: String,
    pub is_live: bool,
    pub video_id: String,
    pub channel: String,
    pub duration: Option<f64>,
    pub strategy: ResolverStrategy,
}

pub async fn resolve_stream_info_exact(
    app: &AppHandle,
    url: &str,
    strategy: ResolverStrategy,
) -> Result<StreamInfo, String> {
    if std::env::var("BKGRND_VERIFY_FAIL_STRATEGIES")
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|label| label == strategy.label())
        })
        .unwrap_or(false)
    {
        eprintln!(
            "[ytdlp] verification mode forced {} failure",
            strategy.label()
        );
        return Err(format!(
            "Verification mode forced {} failure",
            strategy.label()
        ));
    }
    resolve_stream_info_with_strategies(app, url, &[strategy]).await
}

async fn resolve_stream_info_with_strategies(
    _app: &AppHandle,
    url: &str,
    strategies: &[ResolverStrategy],
) -> Result<StreamInfo, String> {
    let format = "bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]/bestaudio/best";
    let js = js_runtime_arg();
    eprintln!("[ytdlp] Resolving metadata + stream for {}", url);
    let mut last_error = None;

    for strategy in strategies.iter().copied() {
        let argument_sets = strategy_argument_sets(strategy);
        if argument_sets.is_empty() {
            eprintln!(
                "[ytdlp] {} unavailable; provider is not installed",
                strategy.label()
            );
            continue;
        }
        for strategy_args in argument_sets {
            let mut args = vec![
                "-f".to_string(),
                format.to_string(),
                "--no-playlist".to_string(),
                "--print".to_string(),
                "%(title)s".to_string(),
                "--print".to_string(),
                "%(is_live)s".to_string(),
                "--print".to_string(),
                "%(id)s".to_string(),
                "--print".to_string(),
                "%(channel,uploader)s".to_string(),
                "--print".to_string(),
                "%(duration)s".to_string(),
                "--get-url".to_string(),
            ];
            args.extend(strategy_args);
            if let Some(value) = js.as_deref() {
                args.push("--js-runtimes".to_string());
                args.push(value.to_string());
            }
            args.push(url.to_string());

            eprintln!("[ytdlp] trying {}", strategy.label());
            let output = ytdlp_command_owned(&args).output().await.map_err(|e| {
                eprintln!("[ytdlp] Spawn failed: {}", e);
                format!("yt-dlp spawn failed: {}", e)
            })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                eprintln!("[ytdlp] {} failed: {}", strategy.label(), stderr.trim());
                last_error = Some(user_facing_ytdlp_error(&stderr));
                continue;
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            match parse_stream_info_output(&stdout, url, strategy) {
                Ok(info) => return Ok(info),
                Err(error) => {
                    eprintln!("[ytdlp] {} output invalid: {}", strategy.label(), error);
                    last_error = Some(error);
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "YouTube did not return a playable stream URL.".to_string()))
}

fn parse_stream_info_output(
    stdout: &str,
    url: &str,
    strategy: ResolverStrategy,
) -> Result<StreamInfo, String> {
    let mut lines = stdout.lines().filter(|line| !line.trim().is_empty());
    let title = lines.next().unwrap_or("Unknown").trim().to_string();
    let is_live = lines
        .next()
        .map(|value| value.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let video_id = lines
        .next()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| extract_video_id(url));
    let channel = lines
        .next()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("none"))
        .unwrap_or_default();
    let duration = lines.next().and_then(parse_duration);
    let stream_url = lines
        .find(|line| line.starts_with("http://") || line.starts_with("https://"))
        .map(str::trim)
        .unwrap_or("")
        .to_string();

    if stream_url.is_empty() {
        return Err(format!(
            "{} returned no playable stream URL",
            strategy.label()
        ));
    }

    Ok(StreamInfo {
        stream_url,
        title,
        is_live,
        video_id,
        channel,
        duration,
        strategy,
    })
}

fn parse_duration(value: &str) -> Option<f64> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed.eq_ignore_ascii_case("na")
        || trimmed.eq_ignore_ascii_case("n/a")
    {
        return None;
    }
    trimmed
        .parse::<f64>()
        .ok()
        .filter(|duration| *duration > 0.0)
}

/// Search YouTube for the first Spotify match, then resolve that concrete
/// video through the same POT -> embedded -> legacy strategy chain used by
/// direct playback.
#[derive(Debug)]
pub struct ResolvedTrack {
    pub stream_url: String,
    pub url: String,
    pub video_id: String,
    pub duration: Option<f64>,
    pub is_live: bool,
    pub strategy: ResolverStrategy,
}

pub async fn search_resolve_first(
    app: &AppHandle,
    query: &str,
) -> Result<Option<ResolvedTrack>, String> {
    let Some(result) = search_first_music(app, query).await? else {
        return Ok(None);
    };
    let info = resolve_stream_info_with_strategies(app, &result.url, &resolver_strategy_order())
        .await?;
    Ok(Some(ResolvedTrack {
        stream_url: info.stream_url,
        url: result.url,
        video_id: info.video_id,
        duration: info.duration.or(result.duration),
        is_live: info.is_live,
        strategy: info.strategy,
    }))
}

fn user_facing_ytdlp_error(stderr: &str) -> String {
    let lower = stderr.to_lowercase();
    if lower.contains("live stream recording is not available") {
        return "This live stream recording is not available.".to_string();
    }
    if lower.contains("private video") {
        return "This video is private and cannot be played.".to_string();
    }
    if lower.contains("video unavailable") || lower.contains("this video is unavailable") {
        return "This video is unavailable on YouTube.".to_string();
    }
    if lower.contains("members-only") || lower.contains("join this channel") {
        return "This video is members-only and cannot be played.".to_string();
    }
    if lower.contains("sign in to confirm your age") || lower.contains("age-restricted") {
        return "This video is age-restricted and requires YouTube sign-in.".to_string();
    }

    stderr
        .lines()
        .find_map(|line| line.trim().strip_prefix("ERROR:").map(str::trim))
        .filter(|message| !message.is_empty())
        .map(|message| message.to_string())
        .unwrap_or_else(|| "YouTube did not return a playable stream.".to_string())
}

#[derive(Debug, Clone)]
pub struct PlaylistItem {
    pub url: String,
    pub video_id: String,
    pub title: String,
    pub thumbnail: String,
    pub channel: String,
    pub duration: Option<f64>,
}

pub struct PlaylistResult {
    pub items: Vec<PlaylistItem>,
    pub title: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub video_id: String,
    pub thumbnail: String,
    pub duration: Option<f64>,
    pub channel: String,
    pub track_count: Option<u64>,
}

pub async fn search_music(_app: &AppHandle, query: &str) -> Result<Vec<SearchResult>, String> {
    let lower = query.to_lowercase();
    let is_playlist_search = lower.contains("playlist");

    let search_arg = if is_playlist_search {
        let clean: Vec<&str> = lower
            .split_whitespace()
            .filter(|w| *w != "playlist")
            .collect();
        let mut search_url = url::Url::parse("https://www.youtube.com/results").unwrap();
        search_url
            .query_pairs_mut()
            .append_pair("search_query", &clean.join(" "))
            .append_pair("sp", "EgIQAw==");
        search_url.to_string()
    } else {
        format!("ytsearch10:{} music", query)
    };

    let output = ytdlp_command(&["--flat-playlist", "--dump-json", &search_arg])
        .output()
        .await
        .map_err(|e| format!("yt-dlp search failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("yt-dlp search failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut results = Vec::new();

    for line in stdout.lines().filter(|l| !l.is_empty()) {
        if results.len() >= 10 {
            break;
        }
        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
            let id = match entry["id"].as_str() {
                Some(id) => id,
                None => continue,
            };

            if is_playlist_search {
                let n_entries = entry["playlist_count"]
                    .as_u64()
                    .or_else(|| entry["n_entries"].as_u64());
                let thumb = entry["thumbnails"]
                    .as_array()
                    .and_then(|t| t.first())
                    .and_then(|t| t["url"].as_str())
                    .unwrap_or("")
                    .to_string();

                results.push(SearchResult {
                    title: entry["title"]
                        .as_str()
                        .unwrap_or("Unknown Playlist")
                        .to_string(),
                    url: format!("https://www.youtube.com/playlist?list={}", id),
                    video_id: String::new(),
                    thumbnail: thumb,
                    duration: None,
                    channel: entry["channel"]
                        .as_str()
                        .or_else(|| entry["uploader"].as_str())
                        .unwrap_or("")
                        .to_string(),
                    track_count: n_entries,
                });
            } else {
                results.push(SearchResult {
                    title: entry["title"].as_str().unwrap_or("Unknown").to_string(),
                    url: format!("https://www.youtube.com/watch?v={}", id),
                    video_id: id.to_string(),
                    thumbnail: thumbnail_url(id),
                    duration: entry["duration"].as_f64(),
                    channel: entry["channel"].as_str().unwrap_or("").to_string(),
                    track_count: None,
                });
            }
        }
    }

    Ok(results)
}

pub async fn search_first_music(
    _app: &AppHandle,
    query: &str,
) -> Result<Option<SearchResult>, String> {
    let search_arg = format!("ytsearch1:{} music", query);

    let output = ytdlp_command(&["--flat-playlist", "--dump-json", &search_arg])
        .output()
        .await
        .map_err(|e| format!("yt-dlp search failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("yt-dlp search failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(line) = stdout.lines().find(|l| !l.trim().is_empty()) else {
        return Ok(None);
    };

    let entry = serde_json::from_str::<serde_json::Value>(line)
        .map_err(|e| format!("yt-dlp search returned invalid JSON: {}", e))?;
    let Some(id) = entry["id"].as_str() else {
        return Ok(None);
    };

    Ok(Some(SearchResult {
        title: entry["title"].as_str().unwrap_or("Unknown").to_string(),
        url: format!("https://www.youtube.com/watch?v={}", id),
        video_id: id.to_string(),
        thumbnail: thumbnail_url(id),
        duration: entry["duration"].as_f64(),
        channel: entry["channel"].as_str().unwrap_or("").to_string(),
        track_count: None,
    }))
}

pub async fn enumerate_playlist(_app: &AppHandle, url: &str) -> Result<PlaylistResult, String> {
    let output = ytdlp_command(&["--flat-playlist", "--dump-json", url])
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
                channel: entry["channel"]
                    .as_str()
                    .or_else(|| entry["uploader"].as_str())
                    .unwrap_or("")
                    .to_string(),
                duration: entry["duration"].as_f64(),
            });
        }
    }

    Ok(PlaylistResult {
        items,
        title: pl_title,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        parse_stream_info_output, resolver_strategy_order, strategy_argument_sets,
        ResolverStrategy, POT_PLAYER_CLIENT, POT_PROVIDER_ARGS,
    };

    #[test]
    fn resolver_strategy_order_prefers_pot_then_embedded_then_legacy() {
        assert_eq!(
            resolver_strategy_order(),
            [
                ResolverStrategy::PotProvider,
                ResolverStrategy::WebEmbedded,
                ResolverStrategy::Legacy,
            ]
        );
    }

    #[test]
    fn resolver_strategy_next_is_bounded() {
        assert_eq!(
            ResolverStrategy::PotProvider.next(),
            Some(ResolverStrategy::WebEmbedded)
        );
        assert_eq!(
            ResolverStrategy::WebEmbedded.next(),
            Some(ResolverStrategy::Legacy)
        );
        assert_eq!(ResolverStrategy::Legacy.next(), None);
    }

    #[test]
    fn pot_strategy_always_produces_provider_arguments() {
        let attempts = strategy_argument_sets(ResolverStrategy::PotProvider);
        assert_eq!(attempts.len(), 1);
        assert!(attempts[0].iter().any(|arg| arg == "--plugin-dirs"));
        assert!(attempts[0].iter().any(|arg| arg == POT_PROVIDER_ARGS));
        assert!(attempts[0].iter().any(|arg| arg == POT_PLAYER_CLIENT));
    }

    #[test]
    fn legacy_strategy_keeps_both_compatible_attempts() {
        let attempts = strategy_argument_sets(ResolverStrategy::Legacy);
        assert_eq!(attempts.len(), 2);
        assert!(attempts[0]
            .iter()
            .any(|arg| arg.contains("youtube:player_client=android_vr")));
        assert!(attempts[1].is_empty());
    }

    #[test]
    fn missing_stream_url_reports_the_strategy_that_failed() {
        let error = parse_stream_info_output(
            "Example title\ntrue\nvideo-id\nchannel\nnone\n",
            "https://www.youtube.com/watch?v=video-id",
            ResolverStrategy::WebEmbedded,
        )
        .expect_err("metadata without a URL must fail");
        assert!(error.contains("web-embedded"));
        assert!(error.contains("no playable stream URL"));
    }
}
