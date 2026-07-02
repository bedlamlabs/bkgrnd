use serde::Serialize;
use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::Mutex;

use crate::history;
use crate::mpv::{self, MpvSession};
use crate::spotify;
use crate::ytdlp::{self, PlaylistItem};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerStatus {
    pub is_playing: bool,
    pub is_paused: bool,
    pub title: String,
    pub mode: Option<String>,
    pub thumbnail: String,
    pub video_id: String,
    pub source_url: String,
    pub queue_position: usize,
    pub queue_length: usize,
    pub playlist_title: String,
    pub channel: String,
    pub duration: Option<f64>,
    pub position: Option<f64>,
}

impl PlayerStatus {
    pub fn empty() -> Self {
        PlayerStatus {
            is_playing: false,
            is_paused: false,
            title: String::new(),
            mode: None,
            thumbnail: String::new(),
            video_id: String::new(),
            source_url: String::new(),
            queue_position: 0,
            queue_length: 0,
            playlist_title: String::new(),
            channel: String::new(),
            duration: None,
            position: None,
        }
    }
}

pub struct PlayerState {
    pub session: Option<MpvSession>,
    // Bumped every time `session` is installed or torn down by an owner
    // action. Exit-watcher tasks capture the value at spawn and exit when it
    // changes, so a watcher from a previous session can never reap (or
    // auto-advance on) a session it doesn't own.
    pub session_gen: u64,
    // Bumped every time the queue is replaced (new play, stop). Background
    // queue-fill tasks (Spotify conversion) capture it and abort when the
    // user has moved on.
    pub queue_epoch: u64,
    pub queue: Vec<PlaylistItem>,
    pub queue_index: i32,
    pub playlist_title: String,
    pub current_title: String,
}

impl PlayerState {
    pub fn new() -> Self {
        PlayerState {
            session: None,
            session_gen: 0,
            queue_epoch: 0,
            queue: Vec::new(),
            queue_index: -1,
            playlist_title: String::new(),
            current_title: String::new(),
        }
    }
}

pub type SharedState = Arc<Mutex<PlayerState>>;

fn queue_item_from_search(track: &spotify::TrackMeta, result: ytdlp::SearchResult) -> PlaylistItem {
    PlaylistItem {
        url: result.url,
        video_id: result.video_id,
        title: track.display_title(),
        thumbnail: result.thumbnail,
        channel: track.artist(),
        duration: result.duration,
    }
}

pub async fn play(url: &str, app: AppHandle, state: SharedState) -> Result<PlayerStatus, String> {
    stop(state.clone()).await?;

    if spotify::is_spotify_url(url) {
        play_spotify(url, app, state).await
    } else if ytdlp::is_playlist_url(url) {
        let result = ytdlp::enumerate_playlist(&app, url).await?;

        if result.items.is_empty() {
            return Err("Playlist is empty or could not be enumerated".to_string());
        }

        let mut start_index = 0;
        let video_id = ytdlp::extract_video_id(url);
        if !video_id.is_empty() {
            if let Some(idx) = result.items.iter().position(|i| i.video_id == video_id) {
                start_index = idx;
            }
        }

        let title = if result.title.is_empty() {
            result.items[0].title.clone()
        } else {
            result.title.clone()
        };
        history::add_to_history(
            url,
            &title,
            &result.items[0].thumbnail,
            "playlist",
            Some(result.items.len()),
            None,
        );

        {
            let mut s = state.lock().await;
            s.queue_epoch += 1;
            s.queue = result.items;
            s.playlist_title = result.title;
        }

        play_queue_item(start_index, app, state.clone()).await?;
        get_status(state).await
    } else {
        let info = ytdlp::resolve_stream_info(&app, url).await?;
        let video_id = if info.video_id.is_empty() {
            ytdlp::extract_video_id(url)
        } else {
            info.video_id.clone()
        };

        {
            let mut s = state.lock().await;
            s.queue_epoch += 1;
            s.queue = vec![PlaylistItem {
                url: url.to_string(),
                video_id: video_id.clone(),
                title: info.title.clone(),
                thumbnail: ytdlp::thumbnail_url(&video_id),
                channel: info.channel.clone(),
                duration: info.duration,
            }];
            s.queue_index = 0;
            s.playlist_title = String::new();
        }

        history::add_to_history(
            url,
            &info.title,
            &ytdlp::thumbnail_url(&video_id),
            if info.is_live { "stream" } else { "video" },
            None,
            info.duration,
        );

        let session = mpv::spawn_mpv(&app, &info.stream_url, &info.title, url).await?;
        install_session(&app, &state, session, info.title).await;

        get_status(state).await
    }
}

// Spotify conversion: fetch track metadata (fast), find the first YouTube
// match and start playing immediately, then match the remaining tracks in
// the background and append them to the queue. Individual match failures are
// skipped instead of failing the conversion.
async fn play_spotify(
    url: &str,
    app: AppHandle,
    state: SharedState,
) -> Result<PlayerStatus, String> {
    let source = spotify::fetch_source(url).await?;
    let limit = spotify::track_limit();
    let source_count = source.tracks.len();
    let title = if source_count > limit {
        format!("{} (first {} tracks)", source.title, limit)
    } else {
        source.title.clone()
    };

    let mut tracks = source.tracks.into_iter().take(limit);

    // First track: search + stream-resolve in ONE yt-dlp invocation so audio
    // starts as fast as possible; the queue fills behind it.
    let mut first: Option<(PlaylistItem, String)> = None;
    for track in tracks.by_ref() {
        match ytdlp::search_resolve_first(&app, &track.search_query()).await {
            Ok(Some(resolved)) => {
                let item = PlaylistItem {
                    url: resolved.url,
                    video_id: resolved.video_id.clone(),
                    title: track.display_title(),
                    thumbnail: ytdlp::thumbnail_url(&resolved.video_id),
                    channel: track.artist(),
                    duration: resolved.duration,
                };
                first = Some((item, resolved.stream_url));
                break;
            }
            Ok(None) => continue,
            Err(err) => {
                eprintln!(
                    "[spotify] match failed for {:?}: {} (skipping)",
                    track.display_title(),
                    err
                );
                continue;
            }
        }
    }
    let Some((first_item, first_stream_url)) = first else {
        return Err("Could not find YouTube matches for this Spotify URL.".to_string());
    };

    let history_thumbnail = if source.thumbnail.is_empty() {
        first_item.thumbnail.clone()
    } else {
        source.thumbnail.clone()
    };
    history::add_to_history(
        url,
        &title,
        &history_thumbnail,
        "spotify-playlist",
        Some(source_count.min(limit)),
        None,
    );

    let epoch = {
        let mut s = state.lock().await;
        s.queue_epoch += 1;
        s.queue = vec![first_item.clone()];
        s.queue_index = 0;
        s.playlist_title = title;
        s.queue_epoch
    };

    // Stream URL is already resolved; spawn mpv directly instead of going
    // through play_queue_item (which would re-resolve it).
    let session = mpv::spawn_mpv(&app, &first_stream_url, &first_item.title, &first_item.url).await?;
    install_session(&app, &state, session, first_item.title.clone()).await;

    let remaining: Vec<spotify::TrackMeta> = tracks.collect();
    if !remaining.is_empty() {
        let app = app.clone();
        let state_bg = state.clone();
        tokio::spawn(async move {
            for track in remaining {
                {
                    let s = state_bg.lock().await;
                    if s.queue_epoch != epoch {
                        return; // user started something else
                    }
                }
                match ytdlp::search_first_music(&app, &track.search_query()).await {
                    Ok(Some(result)) => {
                        let item = queue_item_from_search(&track, result);
                        let mut s = state_bg.lock().await;
                        if s.queue_epoch != epoch {
                            return;
                        }
                        s.queue.push(item);
                    }
                    Ok(None) => {}
                    Err(err) => {
                        eprintln!(
                            "[spotify] match failed for {:?}: {} (skipping)",
                            track.display_title(),
                            err
                        );
                    }
                }
            }
            eprintln!("[spotify] queue fill complete");
        });
    }

    get_status(state).await
}

/// Install a freshly spawned mpv session and start its exit watcher. The
/// watcher owns exactly this session (via session_gen) and handles both
/// reaping and queue auto-advance.
async fn install_session(
    app: &AppHandle,
    state: &SharedState,
    session: MpvSession,
    title: String,
) {
    let gen = {
        let mut s = state.lock().await;
        s.session_gen += 1;
        s.session = Some(session);
        s.current_title = title;
        s.session_gen
    };

    let state_clone = state.clone();
    let app_clone = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            let advance_from = {
                let mut s = state_clone.lock().await;
                if s.session_gen != gen {
                    return; // session replaced; a newer watcher owns it
                }
                match s.session {
                    Some(ref mut session) => match session.child.try_wait() {
                        Ok(Some(status)) => {
                            let code = status.code().unwrap_or(-1);
                            s.session = None;
                            if code == 0 && s.queue_index >= 0 {
                                Some((s.queue_index + 1) as usize)
                            } else {
                                None
                            }
                        }
                        Ok(None) => continue,
                        Err(_) => return,
                    },
                    None => return,
                }
            };

            if let Some(start) = advance_from {
                // Try successive queue entries until one plays; a single dead
                // video must not end the whole queue.
                let mut index = start;
                loop {
                    let in_bounds = {
                        let s = state_clone.lock().await;
                        index < s.queue.len()
                    };
                    if !in_bounds {
                        break;
                    }
                    match play_queue_item(index, app_clone.clone(), state_clone.clone()).await {
                        Ok(()) => break,
                        Err(err) => {
                            eprintln!("[player] auto-advance skipping index {}: {}", index, err);
                            index += 1;
                        }
                    }
                }
            }
            return;
        }
    });
}

fn play_queue_item(
    index: usize,
    app: AppHandle,
    state: SharedState,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> {
    Box::pin(async move {
        let (item_url, item_title) = {
            let mut s = state.lock().await;
            if index >= s.queue.len() {
                return Err("Queue index out of bounds".to_string());
            }

            if let Some(ref mut session) = s.session {
                mpv::stop_mpv(session).await;
            }
            s.session = None;
            s.session_gen += 1; // detach any watcher still bound to the old session
            s.queue_index = index as i32;

            let item = &s.queue[index];
            (item.url.clone(), item.title.clone())
        };

        let stream_url = ytdlp::extract_stream_url(&app, &item_url).await?;
        let session = mpv::spawn_mpv(&app, &stream_url, &item_title, &item_url).await?;
        install_session(&app, &state, session, item_title).await;

        Ok(())
    })
}

pub async fn play_next(app: AppHandle, state: SharedState) -> Result<PlayerStatus, String> {
    let next_index = {
        let s = state.lock().await;
        let next = s.queue_index + 1;
        if (next as usize) < s.queue.len() {
            Some(next as usize)
        } else {
            None
        }
    };

    if let Some(idx) = next_index {
        play_queue_item(idx, app, state.clone()).await?;
    } else {
        stop(state.clone()).await?;
    }
    get_status(state).await
}

pub async fn play_prev(app: AppHandle, state: SharedState) -> Result<PlayerStatus, String> {
    let prev_index = {
        let s = state.lock().await;
        if s.queue_index > 0 {
            Some((s.queue_index - 1) as usize)
        } else if !s.queue.is_empty() {
            Some(0)
        } else {
            None
        }
    };

    if let Some(idx) = prev_index {
        play_queue_item(idx, app, state.clone()).await?;
    }
    get_status(state).await
}

async fn session_ipc_path(state: &SharedState) -> Result<String, String> {
    let s = state.lock().await;
    s.session
        .as_ref()
        .map(|session| session.ipc_path.clone())
        .ok_or_else(|| "Nothing playing".to_string())
}

pub async fn toggle_pause(state: SharedState) -> Result<PlayerStatus, String> {
    let ipc_path = session_ipc_path(&state).await?;
    mpv::pause(&ipc_path).await?;
    get_status(state).await
}

pub async fn stop(state: SharedState) -> Result<PlayerStatus, String> {
    {
        let mut s = state.lock().await;
        if let Some(ref mut session) = s.session {
            mpv::stop_mpv(session).await;
        }
        s.session = None;
        s.session_gen += 1;
        s.queue_epoch += 1;
        s.queue.clear();
        s.queue_index = -1;
        s.playlist_title.clear();
        s.current_title.clear();
    }
    get_status(state).await
}

pub async fn seek_relative(seconds: f64, state: SharedState) -> Result<PlayerStatus, String> {
    let ipc_path = session_ipc_path(&state).await?;
    mpv::seek(&ipc_path, seconds).await?;
    get_status(state).await
}

pub async fn set_volume_cmd(volume: f64, state: SharedState) -> Result<PlayerStatus, String> {
    let vol = volume.clamp(0.0, 100.0);
    let ipc_path = session_ipc_path(&state).await?;
    mpv::set_volume(&ipc_path, vol).await?;
    get_status(state).await
}

pub async fn get_volume_cmd(state: SharedState) -> Result<f64, String> {
    let Ok(ipc_path) = session_ipc_path(&state).await else {
        return Ok(100.0);
    };
    Ok(mpv::get_volume(&ipc_path).await)
}

pub async fn get_status(state: SharedState) -> Result<PlayerStatus, String> {
    let ipc_path = {
        let s = state.lock().await;
        match s.session {
            Some(ref session) => session.ipc_path.clone(),
            None => return Ok(PlayerStatus::empty()),
        }
    };

    // One IPC round-trip for pause/position/duration (not three).
    let snapshot = mpv::status_snapshot(&ipc_path).await;

    let s = state.lock().await;
    if s.session.is_none() {
        return Ok(PlayerStatus::empty());
    }

    let current_item = if s.queue_index >= 0 && (s.queue_index as usize) < s.queue.len() {
        Some(&s.queue[s.queue_index as usize])
    } else {
        None
    };

    Ok(PlayerStatus {
        is_playing: true,
        is_paused: snapshot.paused,
        title: s.current_title.clone(),
        mode: Some("ytdlp".to_string()),
        thumbnail: current_item
            .map(|i| i.thumbnail.clone())
            .unwrap_or_default(),
        video_id: current_item.map(|i| i.video_id.clone()).unwrap_or_default(),
        source_url: current_item.map(|i| i.url.clone()).unwrap_or_default(),
        queue_position: if s.queue_index >= 0 {
            s.queue_index as usize
        } else {
            0
        },
        queue_length: s.queue.len(),
        playlist_title: s.playlist_title.clone(),
        channel: current_item.map(|i| i.channel.clone()).unwrap_or_default(),
        duration: current_item
            .and_then(|i| i.duration)
            .or(snapshot.duration),
        position: snapshot.position,
    })
}
