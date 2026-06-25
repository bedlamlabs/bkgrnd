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
    pub queue: Vec<PlaylistItem>,
    pub queue_index: i32,
    pub playlist_title: String,
    pub current_title: String,
}

impl PlayerState {
    pub fn new() -> Self {
        PlayerState {
            session: None,
            queue: Vec::new(),
            queue_index: -1,
            playlist_title: String::new(),
            current_title: String::new(),
        }
    }
}

pub type SharedState = Arc<Mutex<PlayerState>>;

pub async fn play(url: &str, app: AppHandle, state: SharedState) -> Result<PlayerStatus, String> {
    stop(state.clone()).await?;

    if spotify::is_spotify_url(url) {
        let result = spotify::enumerate(&app, url).await?;

        let title = result.title.clone();
        history::add_to_history(
            url,
            &title,
            &result.thumbnail,
            "spotify-playlist",
            Some(result.items.len().min(result.source_count)),
            None,
        );

        {
            let mut s = state.lock().await;
            s.queue = result.items;
            s.playlist_title = title;
        }

        play_queue_item(0, app, state.clone()).await?;
        get_status(state).await
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
        {
            let mut s = state.lock().await;
            s.session = Some(session);
            s.current_title = info.title;
        }

        // A single track has no queue to advance into, but we still must reap the
        // process when it exits — otherwise `session` stays `Some` and status
        // (tray icon + remote phone view) is stuck reporting "playing" forever.
        let state_clone = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let mut s = state_clone.lock().await;
                match s.session {
                    Some(ref mut session) => match session.child.try_wait() {
                        Ok(Some(_)) => {
                            s.session = None;
                            return;
                        }
                        Ok(None) => continue,
                        Err(_) => return,
                    },
                    None => return,
                }
            }
        });

        get_status(state).await
    }
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
            s.queue_index = index as i32;

            let item = &s.queue[index];
            (item.url.clone(), item.title.clone())
        };

        let stream_url = ytdlp::extract_stream_url(&app, &item_url).await?;
        let session = mpv::spawn_mpv(&app, &stream_url, &item_title, &item_url).await?;

        {
            let mut s = state.lock().await;
            s.session = Some(session);
            s.current_title = item_title;
        }

        // Spawn auto-advance watcher
        let state_clone = state.clone();
        let app_clone = app.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                let should_advance = {
                    let mut s = state_clone.lock().await;
                    if let Some(ref mut session) = s.session {
                        match session.child.try_wait() {
                            Ok(Some(status)) => {
                                let code = status.code().unwrap_or(-1);
                                s.session = None;
                                if code == 0 && (s.queue_index as usize) < s.queue.len() - 1 {
                                    Some((s.queue_index + 1) as usize)
                                } else {
                                    None
                                }
                            }
                            Ok(None) => continue,
                            Err(_) => return,
                        }
                    } else {
                        return;
                    }
                };

                if let Some(next_index) = should_advance {
                    let _ = play_queue_item(next_index, app_clone, state_clone).await;
                }
                return;
            }
        });

        Ok(())
    }) // end Box::pin
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

pub async fn toggle_pause(state: SharedState) -> Result<PlayerStatus, String> {
    {
        let s = state.lock().await;
        if s.session.is_none() {
            return Err("Nothing playing".to_string());
        }
    }
    mpv::pause().await?;
    get_status(state).await
}

pub async fn stop(state: SharedState) -> Result<PlayerStatus, String> {
    {
        let mut s = state.lock().await;
        if let Some(ref mut session) = s.session {
            mpv::stop_mpv(session).await;
        }
        s.session = None;
        s.queue.clear();
        s.queue_index = -1;
        s.playlist_title.clear();
        s.current_title.clear();
    }
    get_status(state).await
}

pub async fn seek_relative(seconds: f64, state: SharedState) -> Result<PlayerStatus, String> {
    {
        let s = state.lock().await;
        if s.session.is_none() {
            return Err("Nothing playing".to_string());
        }
    }
    mpv::seek(seconds).await?;
    get_status(state).await
}

pub async fn set_volume_cmd(volume: f64, state: SharedState) -> Result<PlayerStatus, String> {
    let vol = volume.clamp(0.0, 100.0);
    {
        let s = state.lock().await;
        if s.session.is_none() {
            return Err("Nothing playing".to_string());
        }
    }
    mpv::set_volume(vol).await?;
    get_status(state).await
}

pub async fn get_volume_cmd(state: SharedState) -> Result<f64, String> {
    let s = state.lock().await;
    if s.session.is_none() {
        return Ok(100.0);
    }
    drop(s);
    Ok(mpv::get_volume().await)
}

pub async fn get_status(state: SharedState) -> Result<PlayerStatus, String> {
    let s = state.lock().await;
    if s.session.is_none() {
        return Ok(PlayerStatus::empty());
    }

    let (is_paused, position, mpv_duration) = {
        drop(s);
        (
            mpv::get_paused().await,
            mpv::get_time_position().await,
            mpv::get_duration().await,
        )
    };

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
        is_paused,
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
            .or(mpv_duration.filter(|duration| *duration > 0.0)),
        position,
    })
}
