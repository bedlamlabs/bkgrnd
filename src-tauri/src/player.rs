use serde::Serialize;
use std::future::Future;
use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::Mutex;

use crate::history;
use crate::mpv::{self, MpvSession};
use crate::spotify;
use crate::ytdlp::{self, PlaylistItem, ResolverStrategy};

const LIVE_STALL_POLL_SECONDS: u64 = 3;
const LIVE_STALL_SCORE_LIMIT: u8 = 4;
const STARTUP_READINESS_TIMEOUT_SECONDS: u64 = 12;
const STARTUP_READINESS_POLL_MILLIS: u64 = 250;

#[derive(Debug, Clone)]
struct SessionRecovery {
    source_url: String,
    strategy: ResolverStrategy,
}

#[derive(Debug, Default)]
struct PlaybackStallDetector {
    last_position: Option<f64>,
    stagnant_score: u8,
    saw_progress: bool,
}

impl PlaybackStallDetector {
    fn observe(&mut self, snapshot: mpv::StatusSnapshot) -> bool {
        if snapshot.paused {
            self.stagnant_score = 0;
            self.last_position = snapshot.position;
            return false;
        }

        let advanced = match (self.last_position, snapshot.position) {
            (Some(previous), Some(current)) => current > previous + 0.25,
            (None, Some(current)) => current > 0.25,
            _ => false,
        };
        self.last_position = snapshot.position.or(self.last_position);

        if advanced {
            self.saw_progress = true;
            self.stagnant_score = 0;
            return false;
        }

        let weight = if snapshot.buffering || snapshot.core_idle {
            2
        } else {
            1
        };
        self.stagnant_score = self.stagnant_score.saturating_add(weight);

        // Give initial startup more time than a stream that was already heard.
        let limit = if self.saw_progress {
            LIVE_STALL_SCORE_LIMIT
        } else {
            LIVE_STALL_SCORE_LIMIT + 2
        };
        self.stagnant_score >= limit
    }
}

fn recovery_strategy_order(current: ResolverStrategy) -> Vec<ResolverStrategy> {
    let mut strategies = Vec::new();
    let mut next = current.next();
    while let Some(strategy) = next {
        strategies.push(strategy);
        next = strategy.next();
    }
    strategies
}

fn recovery_install_allowed(
    expected_gen: u64,
    current_gen: u64,
    desired_paused: bool,
    has_session: bool,
) -> bool {
    expected_gen == current_gen && !desired_paused && has_session
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryResult {
    Recovered,
    Cancelled,
    Exhausted,
}

fn should_clear_after_recovery(result: RecoveryResult) -> bool {
    result == RecoveryResult::Exhausted
}

fn exit_recovery_strategy(
    clean_exit: bool,
    shuffle: bool,
    recovery: Option<&SessionRecovery>,
) -> Option<ResolverStrategy> {
    if clean_exit || shuffle {
        return None;
    }
    recovery.and_then(|context| context.strategy.next())
}

fn startup_snapshot_is_ready(snapshot: mpv::StatusSnapshot) -> bool {
    snapshot
        .position
        .map(|position| position > 0.1)
        .unwrap_or(false)
        && !snapshot.core_idle
}

async fn wait_for_startup_readiness<P, PollFuture>(
    timeout: std::time::Duration,
    poll_interval: std::time::Duration,
    mut poll: P,
) -> Result<(), String>
where
    P: FnMut() -> PollFuture,
    PollFuture: Future<Output = Result<mpv::StatusSnapshot, String>>,
{
    tokio::time::timeout(timeout, async {
        loop {
            if let Ok(snapshot) = poll().await {
                if startup_snapshot_is_ready(snapshot) {
                    return;
                }
            }
            tokio::time::sleep(poll_interval).await;
        }
    })
    .await
    .map_err(|_| "mpv did not become ready before the startup timeout".to_string())
}

async fn spawn_ready_mpv(
    app: &AppHandle,
    stream_url: &str,
    title: &str,
    source_url: &str,
    strategy: ResolverStrategy,
) -> Result<MpvSession, String> {
    let mut session = mpv::spawn_mpv(app, stream_url, title, source_url).await?;

    let force_unready = std::env::var("BKGRND_VERIFY_UNREADY_START_STRATEGY")
        .ok()
        .map(|label| label.trim() == strategy.label())
        .unwrap_or(false);
    if force_unready {
        eprintln!(
            "[player] verifier rejected startup readiness for {}",
            strategy.label()
        );
        mpv::stop_mpv(&mut session).await;
        return Err(format!(
            "{} startup rejected by playback verifier",
            strategy.label()
        ));
    }

    let readiness = wait_for_startup_readiness(
        std::time::Duration::from_secs(STARTUP_READINESS_TIMEOUT_SECONDS),
        std::time::Duration::from_millis(STARTUP_READINESS_POLL_MILLIS),
        || mpv::status_snapshot_checked(&session.ipc_path),
    )
    .await;
    if let Err(error) = readiness {
        mpv::stop_mpv(&mut session).await;
        return Err(format!(
            "{} startup readiness failed: {}",
            strategy.label(),
            error
        ));
    }

    Ok(session)
}

async fn try_startup_strategies<T, U, Resolve, ResolveFuture, Start, StartFuture>(
    strategies: impl IntoIterator<Item = ResolverStrategy>,
    mut resolve: Resolve,
    mut start: Start,
) -> Result<(ResolverStrategy, U), String>
where
    Resolve: FnMut(ResolverStrategy) -> ResolveFuture,
    ResolveFuture: Future<Output = Result<T, String>>,
    Start: FnMut(T) -> StartFuture,
    StartFuture: Future<Output = Result<U, String>>,
{
    let mut last_error = None;
    for strategy in strategies {
        let resolved = match resolve(strategy).await {
            Ok(resolved) => resolved,
            Err(error) => {
                last_error = Some(error);
                continue;
            }
        };
        match start(resolved).await {
            Ok(started) => return Ok((strategy, started)),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| "No playback strategy could start the stream".to_string()))
}

async fn resolve_and_spawn(
    app: &AppHandle,
    source_url: &str,
    preferred_title: Option<&str>,
) -> Result<(ytdlp::StreamInfo, MpvSession), String> {
    let resolve_app = app.clone();
    let resolve_url = source_url.to_string();
    let start_app = app.clone();
    let start_url = source_url.to_string();
    let preferred_title = preferred_title.map(str::to_string);

    try_startup_strategies(
        ytdlp::resolver_strategy_order(),
        move |strategy| {
            let app = resolve_app.clone();
            let url = resolve_url.clone();
            async move { ytdlp::resolve_stream_info_exact(&app, &url, strategy).await }
        },
        move |info: ytdlp::StreamInfo| {
            let app = start_app.clone();
            let url = start_url.clone();
            let title = preferred_title
                .clone()
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| info.title.clone());
            async move {
                let session =
                    spawn_ready_mpv(&app, &info.stream_url, &title, &url, info.strategy).await?;
                Ok((info, session))
            }
        },
    )
    .await
    .map(|(_, playback)| playback)
}

fn record_resolver_strategy(strategy: ResolverStrategy, source_url: &str) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let data_dir = home.join(".bkgrnd");
    if std::fs::create_dir_all(&data_dir).is_ok() {
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let marker = format!("{}\t{}\t{}\n", timestamp_ms, strategy.label(), source_url);
        let _ = std::fs::write(data_dir.join("last-resolver-strategy"), marker);
    }
}

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
    pub shuffle: bool,
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
            shuffle: false,
        }
    }
}

pub struct PlayerState {
    pub session: Option<MpvSession>,
    // User intent, updated before pause IPC is sent so an in-flight recovery
    // can never replace a just-paused stream with an unpaused session.
    pub desired_paused: bool,
    // Queue-level artwork (e.g. the Spotify playlist cover) shown instead of
    // per-track art while this queue plays.
    pub queue_artwork: Option<String>,
    pub shuffle: bool,
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
            desired_paused: false,
            queue_artwork: None,
            shuffle: false,
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

fn update_queue_item_from_recovery(item: &mut PlaylistItem, info: &ytdlp::StreamInfo) {
    item.video_id = info.video_id.clone();
    item.title = info.title.clone();
    item.channel = info.channel.clone();
    item.duration = info.duration;
    if !info.video_id.is_empty() {
        item.thumbnail = ytdlp::thumbnail_url(&info.video_id);
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
            s.queue_artwork = None;
        }

        play_queue_item(start_index, app, state.clone()).await?;
        get_status(state).await
    } else {
        let (info, session) = resolve_and_spawn(&app, url, None).await?;
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
            s.queue_artwork = None;
        }

        history::add_to_history(
            url,
            &info.title,
            &ytdlp::thumbnail_url(&video_id),
            if info.is_live { "stream" } else { "video" },
            None,
            info.duration,
        );

        install_session(
            &app,
            &state,
            session,
            info.title,
            Some(SessionRecovery {
                source_url: url.to_string(),
                strategy: info.strategy,
            }),
        )
        .await;

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

    // Resolve the first match through the resilient strategy chain before the
    // remaining queue fills in the background.
    let mut first: Option<(PlaylistItem, String, bool, ResolverStrategy)> = None;
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
                first = Some((
                    item,
                    resolved.stream_url,
                    resolved.is_live,
                    resolved.strategy,
                ));
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
    let Some((first_item, first_stream_url, _first_is_live, first_strategy)) = first else {
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
        // Queue-level cover: the Spotify playlist's own artwork.
        s.queue_artwork = if source.thumbnail.is_empty() {
            None
        } else {
            Some(source.thumbnail.clone())
        };
        s.queue_epoch
    };

    // Stream URL is already resolved; spawn mpv directly instead of going
    // through play_queue_item (which would re-resolve it).
    let session = spawn_ready_mpv(
        &app,
        &first_stream_url,
        &first_item.title,
        &first_item.url,
        first_strategy,
    )
    .await?;
    install_session(
        &app,
        &state,
        session,
        first_item.title.clone(),
        Some(SessionRecovery {
            source_url: first_item.url.clone(),
            strategy: first_strategy,
        }),
    )
    .await;

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

enum AdvanceAction {
    Index(usize),
    RandomStream,
    Recover(SessionRecovery),
}

/// Cheap non-crypto randomness for shuffle picks (no rand dependency).
fn random_index(len: usize, exclude: usize) -> usize {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    if len <= 1 {
        return 0;
    }
    let mut pick = nanos % len;
    if pick == exclude {
        pick = (pick + 1) % len;
    }
    pick
}

/// What to do when a track finishes (or dies). Shuffle semantics:
/// multi-track queues (converted playlists) shuffle within the queue and
/// keep going; a lone stream with shuffle on hops to a random saved stream.
/// Errors advance the same way successes do when shuffle is on.
fn next_action(s: &PlayerState, clean_exit: bool) -> Option<AdvanceAction> {
    let index = s.queue_index.max(0) as usize;
    if s.queue.len() > 1 {
        if s.shuffle {
            return Some(AdvanceAction::Index(random_index(s.queue.len(), index)));
        }
        if clean_exit && index < s.queue.len() - 1 {
            return Some(AdvanceAction::Index(index + 1));
        }
        return None;
    }
    if s.shuffle {
        return Some(AdvanceAction::RandomStream);
    }
    None
}

/// Shuffle hop for single streams: pick a random saved stream and play it.
async fn play_random_saved(app: AppHandle, state: SharedState) -> Result<(), String> {
    let previous_url = {
        let s = state.lock().await;
        s.queue.first().map(|i| i.url.clone()).unwrap_or_default()
    };

    let doc = crate::playlists::load_or_derive_playlists();
    let candidates: Vec<crate::playlists::PlaylistItem> = doc
        .playlists
        .into_iter()
        .flat_map(|p| p.items)
        .filter(|i| i.url != previous_url)
        .collect();
    if candidates.is_empty() {
        return Err("No other saved streams to shuffle to".to_string());
    }

    for attempt in 0..candidates.len().min(5) {
        let pick = &candidates[random_index(candidates.len(), usize::MAX) % candidates.len()];
        let video_id = ytdlp::extract_video_id(&pick.url);
        let item = PlaylistItem {
            url: pick.url.clone(),
            video_id: video_id.clone(),
            title: pick.title.clone(),
            thumbnail: if pick.thumbnail.is_empty() {
                ytdlp::thumbnail_url(&video_id)
            } else {
                pick.thumbnail.clone()
            },
            channel: pick.channel.clone(),
            duration: pick.duration,
        };

        {
            let mut s = state.lock().await;
            s.queue_epoch += 1;
            s.queue = vec![item];
            s.queue_index = 0;
            s.playlist_title = String::new();
            s.queue_artwork = None;
        }
        match play_queue_item(0, app.clone(), state.clone()).await {
            Ok(()) => return Ok(()),
            Err(err) => {
                eprintln!(
                    "[player] shuffle pick failed (attempt {}): {}",
                    attempt + 1,
                    err
                );
            }
        }
    }
    Err("Shuffle could not start any saved stream".to_string())
}

pub async fn set_shuffle(enabled: bool, state: SharedState) -> Result<PlayerStatus, String> {
    {
        let mut s = state.lock().await;
        s.shuffle = enabled;
    }
    get_status(state).await
}

pub async fn seek_absolute(seconds: f64, state: SharedState) -> Result<PlayerStatus, String> {
    let ipc_path = session_ipc_path(&state).await?;
    mpv::seek_absolute(&ipc_path, seconds).await?;
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
    recovery: Option<SessionRecovery>,
) {
    let gen = {
        let mut s = state.lock().await;
        s.session_gen += 1;
        s.session = Some(session);
        s.desired_paused = false;
        s.current_title = title;
        s.session_gen
    };

    if let Some(context) = recovery.as_ref() {
        record_resolver_strategy(context.strategy, &context.source_url);
    }

    spawn_session_watchers(app.clone(), state.clone(), gen, recovery);
}

fn spawn_session_watchers(
    app: AppHandle,
    state: SharedState,
    gen: u64,
    recovery: Option<SessionRecovery>,
) {
    if let Some(recovery) = recovery.clone() {
        let state_clone = state.clone();
        let app_clone = app.clone();
        tokio::spawn(async move {
            let mut detector = PlaybackStallDetector::default();
            let freeze_for_verification = std::env::var("BKGRND_VERIFY_FREEZE_STALL_STRATEGY")
                .ok()
                .map(|label| label.trim() == recovery.strategy.label())
                .unwrap_or(false);
            let mut frozen_position = None;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(LIVE_STALL_POLL_SECONDS)).await;

                let ipc_path = {
                    let s = state_clone.lock().await;
                    if s.session_gen != gen {
                        return;
                    }
                    match s.session.as_ref() {
                        Some(session) => session.ipc_path.clone(),
                        None => return,
                    }
                };
                let mut snapshot = mpv::status_snapshot(&ipc_path).await;
                if freeze_for_verification {
                    frozen_position = frozen_position.or(snapshot.position);
                    snapshot.position = frozen_position;
                    snapshot.buffering = true;
                }
                if detector.observe(snapshot) {
                    eprintln!(
                        "[player] playback stalled on {}; recovering",
                        recovery.strategy.label()
                    );
                    let result = Box::pin(recover_stalled_session(
                        app_clone,
                        state_clone.clone(),
                        gen,
                        recovery,
                    ))
                    .await;
                    if should_clear_after_recovery(result) {
                        clear_owned_session(&state_clone, gen).await;
                    }
                    return;
                }
            }
        });
    }

    let state_clone = state;
    let app_clone = app;
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            let action = {
                let mut s = state_clone.lock().await;
                if s.session_gen != gen {
                    return; // session replaced; a newer watcher owns it
                }
                match s.session {
                    Some(ref mut session) => match session.child.try_wait() {
                        Ok(Some(status)) => {
                            let code = status.code().unwrap_or(-1);
                            if exit_recovery_strategy(code == 0, s.shuffle, recovery.as_ref())
                                .is_some()
                            {
                                recovery.clone().map(AdvanceAction::Recover)
                            } else {
                                s.session = None;
                                if s.queue_index < 0 {
                                    None
                                } else {
                                    next_action(&s, code == 0)
                                }
                            }
                        }
                        Ok(None) => continue,
                        Err(_) => return,
                    },
                    None => return,
                }
            };

            match action {
                Some(AdvanceAction::Index(start)) => {
                    // Try successive picks until one plays; a single dead
                    // video must not end the whole queue.
                    let mut index = start;
                    let mut attempts = 0;
                    loop {
                        attempts += 1;
                        if attempts > 10 {
                            break;
                        }
                        let (in_bounds, shuffle, len, current) = {
                            let s = state_clone.lock().await;
                            (
                                index < s.queue.len(),
                                s.shuffle,
                                s.queue.len(),
                                s.queue_index,
                            )
                        };
                        if !in_bounds {
                            break;
                        }
                        match play_queue_item(index, app_clone.clone(), state_clone.clone()).await {
                            Ok(()) => break,
                            Err(err) => {
                                eprintln!(
                                    "[player] auto-advance skipping index {}: {}",
                                    index, err
                                );
                                index = if shuffle && len > 1 {
                                    random_index(len, current.max(0) as usize)
                                } else {
                                    index + 1
                                };
                            }
                        }
                    }
                }
                Some(AdvanceAction::RandomStream) => {
                    let _ = play_random_saved(app_clone.clone(), state_clone.clone()).await;
                }
                Some(AdvanceAction::Recover(recovery)) => {
                    let result = Box::pin(recover_stalled_session(
                        app_clone.clone(),
                        state_clone.clone(),
                        gen,
                        recovery,
                    ))
                    .await;
                    if should_clear_after_recovery(result) {
                        clear_owned_session(&state_clone, gen).await;
                    }
                }
                None => {}
            }
            return;
        }
    });
}

async fn clear_owned_session(state: &SharedState, expected_gen: u64) {
    let old_session = {
        let mut s = state.lock().await;
        if s.session_gen != expected_gen {
            return;
        }
        s.session_gen += 1;
        s.desired_paused = false;
        s.session.take()
    };
    if let Some(mut session) = old_session {
        mpv::stop_mpv(&mut session).await;
    }
}

async fn recover_stalled_session(
    app: AppHandle,
    state: SharedState,
    expected_gen: u64,
    recovery: SessionRecovery,
) -> RecoveryResult {
    // Re-read the actual mpv state immediately before recovery. This catches
    // pause state changed outside the normal command path and synchronizes it
    // back into user intent rather than replacing it with a playing session.
    let current_ipc = {
        let s = state.lock().await;
        if s.session_gen != expected_gen || s.session.is_none() {
            return RecoveryResult::Cancelled;
        }
        if s.desired_paused {
            return RecoveryResult::Cancelled;
        }
        s.session.as_ref().map(|session| session.ipc_path.clone())
    };
    if let Some(ipc_path) = current_ipc {
        if mpv::status_snapshot(&ipc_path).await.paused {
            let mut s = state.lock().await;
            if s.session_gen == expected_gen {
                s.desired_paused = true;
            }
            return RecoveryResult::Cancelled;
        }
    }

    for next_strategy in recovery_strategy_order(recovery.strategy) {
        let info = match ytdlp::resolve_stream_info_exact(&app, &recovery.source_url, next_strategy)
            .await
        {
            Ok(info) => info,
            Err(error) => {
                eprintln!(
                    "[player] playback recovery {} resolve failed: {}",
                    next_strategy.label(),
                    error
                );
                continue;
            }
        };
        let replacement = match spawn_ready_mpv(
            &app,
            &info.stream_url,
            &info.title,
            &recovery.source_url,
            next_strategy,
        )
        .await
        {
            Ok(session) => session,
            Err(error) => {
                eprintln!(
                    "[player] playback recovery {} mpv start failed: {}",
                    next_strategy.label(),
                    error
                );
                continue;
            }
        };

        let current_ipc = {
            let s = state.lock().await;
            if s.session_gen != expected_gen || s.session.is_none() || s.desired_paused {
                None
            } else {
                s.session.as_ref().map(|session| session.ipc_path.clone())
            }
        };
        let Some(current_ipc) = current_ipc else {
            let mut replacement = replacement;
            mpv::stop_mpv(&mut replacement).await;
            return RecoveryResult::Cancelled;
        };
        if mpv::status_snapshot(&current_ipc).await.paused {
            let mut replacement = replacement;
            mpv::stop_mpv(&mut replacement).await;
            let mut s = state.lock().await;
            if s.session_gen == expected_gen {
                s.desired_paused = true;
            }
            return RecoveryResult::Cancelled;
        }

        let mut replacement = Some(replacement);
        let installed = {
            let mut s = state.lock().await;
            if !recovery_install_allowed(
                expected_gen,
                s.session_gen,
                s.desired_paused,
                s.session.is_some(),
            ) {
                None
            } else {
                let old = s.session.take();
                s.session_gen += 1;
                s.session = replacement.take();
                let queue_index = s.queue_index;
                if queue_index >= 0 {
                    if let Some(item) = s.queue.get_mut(queue_index as usize) {
                        update_queue_item_from_recovery(item, &info);
                    }
                }
                s.current_title = info.title.clone();
                Some((s.session_gen, old))
            }
        };

        let Some((new_gen, old_session)) = installed else {
            if let Some(mut replacement) = replacement {
                mpv::stop_mpv(&mut replacement).await;
            }
            return RecoveryResult::Cancelled;
        };

        if let Some(mut old_session) = old_session {
            mpv::stop_mpv(&mut old_session).await;
        }
        record_resolver_strategy(info.strategy, &recovery.source_url);
        eprintln!("[player] playback recovered with {}", info.strategy.label());
        spawn_session_watchers(
            app,
            state,
            new_gen,
            Some(SessionRecovery {
                source_url: recovery.source_url,
                strategy: info.strategy,
            }),
        );
        return RecoveryResult::Recovered;
    }

    eprintln!("[player] playback recovery exhausted all resolver strategies");
    RecoveryResult::Exhausted
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

        let (info, session) = resolve_and_spawn(&app, &item_url, Some(&item_title)).await?;
        install_session(
            &app,
            &state,
            session,
            item_title,
            Some(SessionRecovery {
                source_url: item_url,
                strategy: info.strategy,
            }),
        )
        .await;

        Ok(())
    })
}

pub async fn play_next(app: AppHandle, state: SharedState) -> Result<PlayerStatus, String> {
    let action = {
        let s = state.lock().await;
        if s.shuffle {
            next_action(&s, true)
        } else {
            let next = s.queue_index + 1;
            if (next as usize) < s.queue.len() {
                Some(AdvanceAction::Index(next as usize))
            } else {
                None
            }
        }
    };

    match action {
        Some(AdvanceAction::Index(idx)) => {
            play_queue_item(idx, app, state.clone()).await?;
        }
        Some(AdvanceAction::RandomStream) => {
            play_random_saved(app, state.clone()).await?;
        }
        Some(AdvanceAction::Recover(_)) => {
            unreachable!("manual next never produces a recovery action")
        }
        None => {
            stop(state.clone()).await?;
        }
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
    let (ipc_path, gen, previous_intent) = {
        let mut s = state.lock().await;
        let ipc_path = s
            .session
            .as_ref()
            .map(|session| session.ipc_path.clone())
            .ok_or_else(|| "Nothing playing".to_string())?;
        let previous_intent = s.desired_paused;
        s.desired_paused = !previous_intent;
        (ipc_path, s.session_gen, previous_intent)
    };
    if let Err(error) = mpv::pause(&ipc_path).await {
        let mut s = state.lock().await;
        if s.session_gen == gen {
            s.desired_paused = previous_intent;
        }
        return Err(error);
    }
    get_status(state).await
}

pub async fn stop(state: SharedState) -> Result<PlayerStatus, String> {
    {
        let mut s = state.lock().await;
        if let Some(ref mut session) = s.session {
            mpv::stop_mpv(session).await;
        }
        s.session = None;
        s.desired_paused = false;
        s.session_gen += 1;
        s.queue_epoch += 1;
        s.queue.clear();
        s.queue_index = -1;
        s.playlist_title.clear();
        s.current_title.clear();
        s.queue_artwork = None;
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
        thumbnail: s
            .queue_artwork
            .clone()
            .or_else(|| current_item.map(|i| i.thumbnail.clone()))
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
        duration: current_item.and_then(|i| i.duration).or(snapshot.duration),
        position: snapshot.position,
        shuffle: s.shuffle,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        exit_recovery_strategy, recovery_install_allowed, recovery_strategy_order,
        should_clear_after_recovery, try_startup_strategies, update_queue_item_from_recovery,
        wait_for_startup_readiness, PlaybackStallDetector, RecoveryResult, SessionRecovery,
    };
    use crate::mpv::StatusSnapshot;
    use crate::ytdlp::ResolverStrategy;
    use std::sync::{Arc, Mutex};

    fn snapshot(position: Option<f64>, paused: bool, buffering: bool) -> StatusSnapshot {
        StatusSnapshot {
            paused,
            position,
            duration: None,
            buffering,
            core_idle: false,
        }
    }

    #[test]
    fn stall_detector_triggers_after_progress_then_buffering() {
        let mut detector = PlaybackStallDetector::default();
        assert!(!detector.observe(snapshot(Some(1.0), false, false)));
        assert!(!detector.observe(snapshot(Some(4.0), false, false)));
        assert!(!detector.observe(snapshot(Some(4.0), false, true)));
        assert!(detector.observe(snapshot(Some(4.0), false, true)));
    }

    #[test]
    fn stall_detector_never_recovers_a_user_paused_stream() {
        let mut detector = PlaybackStallDetector::default();
        assert!(!detector.observe(snapshot(Some(10.0), false, false)));
        assert!(!detector.observe(snapshot(Some(13.0), false, false)));
        for _ in 0..8 {
            assert!(!detector.observe(snapshot(Some(13.0), true, true)));
        }
    }

    #[test]
    fn stall_detector_resets_when_position_advances() {
        let mut detector = PlaybackStallDetector::default();
        assert!(!detector.observe(snapshot(Some(1.0), false, false)));
        assert!(!detector.observe(snapshot(Some(4.0), false, false)));
        assert!(!detector.observe(snapshot(Some(4.0), false, true)));
        assert!(!detector.observe(snapshot(Some(7.0), false, false)));
        assert!(!detector.observe(snapshot(Some(7.0), false, true)));
    }

    #[test]
    fn recovery_falls_through_every_remaining_strategy() {
        assert_eq!(
            recovery_strategy_order(ResolverStrategy::PotProvider),
            vec![ResolverStrategy::WebEmbedded, ResolverStrategy::Legacy]
        );
        assert_eq!(
            recovery_strategy_order(ResolverStrategy::WebEmbedded),
            vec![ResolverStrategy::Legacy]
        );
        assert!(recovery_strategy_order(ResolverStrategy::Legacy).is_empty());
    }

    #[test]
    fn recovery_install_is_rejected_after_user_pause_or_session_change() {
        assert!(recovery_install_allowed(7, 7, false, true));
        assert!(!recovery_install_allowed(7, 7, true, true));
        assert!(!recovery_install_allowed(7, 8, false, true));
        assert!(!recovery_install_allowed(7, 7, false, false));
    }

    #[test]
    fn exhausted_stall_recovery_clears_only_the_owned_dead_session() {
        assert!(should_clear_after_recovery(RecoveryResult::Exhausted));
        assert!(!should_clear_after_recovery(RecoveryResult::Recovered));
        assert!(!should_clear_after_recovery(RecoveryResult::Cancelled));
    }

    #[test]
    fn abnormal_live_exit_uses_the_next_strategy() {
        let recovery = SessionRecovery {
            source_url: "https://www.youtube.com/watch?v=Lcdi9O2XB4E".to_string(),
            strategy: ResolverStrategy::PotProvider,
        };
        assert_eq!(
            exit_recovery_strategy(false, false, Some(&recovery)),
            Some(ResolverStrategy::WebEmbedded)
        );
        assert_eq!(exit_recovery_strategy(true, false, Some(&recovery)), None);
        assert_eq!(exit_recovery_strategy(false, true, Some(&recovery)), None);

        let terminal = SessionRecovery {
            strategy: ResolverStrategy::Legacy,
            ..recovery
        };
        assert_eq!(exit_recovery_strategy(false, false, Some(&terminal)), None);
    }

    #[test]
    fn non_live_abnormal_exit_uses_next_strategy() {
        let recovery = SessionRecovery {
            source_url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string(),
            strategy: ResolverStrategy::PotProvider,
        };

        assert_eq!(
            exit_recovery_strategy(false, false, Some(&recovery)),
            Some(ResolverStrategy::WebEmbedded)
        );
        assert_eq!(exit_recovery_strategy(true, false, Some(&recovery)), None);
        assert_eq!(exit_recovery_strategy(false, true, Some(&recovery)), None);
    }

    #[test]
    fn startup_readiness_rejects_idle_session() {
        tauri::async_runtime::block_on(async {
            let idle = StatusSnapshot {
                paused: false,
                position: None,
                duration: None,
                buffering: false,
                core_idle: true,
            };

            let result = wait_for_startup_readiness(
                std::time::Duration::from_millis(10),
                std::time::Duration::from_millis(1),
                || std::future::ready(Ok(idle)),
            )
            .await;

            assert!(result.is_err());
        });
    }

    #[test]
    fn startup_readiness_rejects_non_idle_session_without_position() {
        tauri::async_runtime::block_on(async {
            let not_yet_playable = StatusSnapshot {
                paused: false,
                position: None,
                duration: None,
                buffering: false,
                core_idle: false,
            };

            let result = wait_for_startup_readiness(
                std::time::Duration::from_millis(10),
                std::time::Duration::from_millis(1),
                || std::future::ready(Ok(not_yet_playable)),
            )
            .await;

            assert!(result.is_err());
        });
    }

    #[test]
    fn startup_readiness_rejects_non_idle_session_at_zero_position() {
        tauri::async_runtime::block_on(async {
            let not_advancing = StatusSnapshot {
                paused: false,
                position: Some(0.0),
                duration: Some(60.0),
                buffering: false,
                core_idle: false,
            };

            let result = wait_for_startup_readiness(
                std::time::Duration::from_millis(10),
                std::time::Duration::from_millis(1),
                || std::future::ready(Ok(not_advancing)),
            )
            .await;

            assert!(result.is_err());
        });
    }

    #[test]
    fn recovery_refreshes_active_queue_metadata() {
        let mut item = crate::ytdlp::PlaylistItem {
            url: "https://www.youtube.com/watch?v=old".to_string(),
            video_id: "old".to_string(),
            title: "Old title".to_string(),
            thumbnail: "old-thumbnail".to_string(),
            channel: "Old channel".to_string(),
            duration: Some(1.0),
        };
        let info = crate::ytdlp::StreamInfo {
            stream_url: "https://media.example/audio".to_string(),
            title: "Recovered title".to_string(),
            is_live: true,
            video_id: "new".to_string(),
            channel: "Recovered channel".to_string(),
            duration: Some(42.0),
            strategy: ResolverStrategy::WebEmbedded,
        };

        update_queue_item_from_recovery(&mut item, &info);

        assert_eq!(item.video_id, "new");
        assert_eq!(item.title, "Recovered title");
        assert_eq!(item.channel, "Recovered channel");
        assert_eq!(item.duration, Some(42.0));
        assert!(item.thumbnail.contains("new"));
    }

    #[test]
    fn startup_fallback_continues_after_resolved_stream_cannot_start() {
        tauri::async_runtime::block_on(async {
            let resolved = Arc::new(Mutex::new(Vec::new()));
            let started = Arc::new(Mutex::new(Vec::new()));
            let resolved_log = resolved.clone();
            let started_log = started.clone();

            let (strategy, ()) = try_startup_strategies(
                crate::ytdlp::resolver_strategy_order(),
                move |strategy| {
                    resolved_log.lock().unwrap().push(strategy);
                    std::future::ready(Ok(strategy))
                },
                move |strategy| {
                    started_log.lock().unwrap().push(strategy);
                    std::future::ready(if strategy == ResolverStrategy::PotProvider {
                        Err("mpv startup failed".to_string())
                    } else {
                        Ok(())
                    })
                },
            )
            .await
            .unwrap();

            assert_eq!(strategy, ResolverStrategy::WebEmbedded);
            assert_eq!(
                *resolved.lock().unwrap(),
                vec![ResolverStrategy::PotProvider, ResolverStrategy::WebEmbedded]
            );
            assert_eq!(
                *started.lock().unwrap(),
                vec![ResolverStrategy::PotProvider, ResolverStrategy::WebEmbedded]
            );
        });
    }
}
