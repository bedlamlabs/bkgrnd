mod config;
mod history;
mod mpv;
mod player;
mod playlists;
mod spotify;
mod wopr_sync;
mod ytdlp;

use player::{PlayerStatus, SharedState};
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WebviewWindow,
};
use tauri_plugin_positioner::{Position, WindowExt};
use tokio::sync::Mutex;

#[tauri::command]
async fn quit_app(app: AppHandle, state: tauri::State<'_, SharedState>) -> Result<(), String> {
    let _ = player::stop(state.inner().clone()).await;
    app.exit(0);
    Ok(())
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

#[tauri::command]
fn resize_window(height: f64, app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let current_size = window.outer_size().unwrap_or_default();
        let scale = window.scale_factor().unwrap_or(1.0);
        let logical_width = current_size.width as f64 / scale;
        let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize {
            width: logical_width,
            height,
        }));
    }
}

#[tauri::command]
fn set_tray_state(state: String, app: AppHandle) {
    if let Some(tray) = app.tray_by_id("main") {
        let icon_bytes: &[u8] = match state.as_str() {
            "playing" => include_bytes!("../icons/icon-playing.png"),
            "paused" => include_bytes!("../icons/icon-paused.png"),
            _ => include_bytes!("../icons/icon.png"),
        };

        if let Ok(icon) = tauri::image::Image::from_bytes(icon_bytes) {
            let _ = tray.set_icon(Some(icon));
        }

        let tooltip = match state.as_str() {
            "playing" => "bkgrnd — Streaming",
            "paused" => "bkgrnd — Paused",
            _ => "bkgrnd",
        };
        let _ = tray.set_tooltip(Some(tooltip));
    }
}

#[tauri::command]
async fn search(query: String, app: AppHandle) -> Result<Vec<ytdlp::SearchResult>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("Search query is empty".to_string());
    }
    ytdlp::search_music(&app, trimmed).await
}

#[tauri::command]
async fn play(
    url: String,
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<PlayerStatus, String> {
    let re = regex::Regex::new(
        r"^(https?://(www\.)?(youtube\.com|youtu\.be|music\.youtube\.com|open\.spotify\.com)/|spotify:(playlist|album|track):)",
    )
    .unwrap();
    if !re.is_match(&url) {
        return Err("Not a YouTube or Spotify URL".to_string());
    }
    player::play(&url, app, state.inner().clone()).await
}

#[tauri::command]
async fn toggle_pause(state: tauri::State<'_, SharedState>) -> Result<PlayerStatus, String> {
    player::toggle_pause(state.inner().clone()).await
}

#[tauri::command]
async fn stop(state: tauri::State<'_, SharedState>) -> Result<PlayerStatus, String> {
    player::stop(state.inner().clone()).await
}

#[tauri::command]
async fn get_status(state: tauri::State<'_, SharedState>) -> Result<PlayerStatus, String> {
    player::get_status(state.inner().clone()).await
}

#[tauri::command]
fn get_history() -> Vec<history::HistoryEntry> {
    history::load_history()
}

#[tauri::command]
fn get_playlists() -> playlists::PlaylistDoc {
    playlists::load_or_derive_playlists()
}

#[tauri::command]
fn save_playlist_item(
    url: String,
    title: String,
    thumbnail: String,
    channel: String,
    duration: Option<f64>,
) -> playlists::PlaylistDoc {
    playlists::save_item(playlists::PlaylistItem {
        url,
        title,
        channel,
        thumbnail,
        duration,
        added_at: String::new(),
    })
}

#[tauri::command]
fn remove_playlist_item(url: String) -> playlists::PlaylistDoc {
    let doc = playlists::remove_item(&url);
    history::remove_from_history(&url);
    doc
}

#[tauri::command]
async fn play_next(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<PlayerStatus, String> {
    player::play_next(app, state.inner().clone()).await
}

#[tauri::command]
async fn play_prev(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<PlayerStatus, String> {
    player::play_prev(app, state.inner().clone()).await
}

#[tauri::command]
async fn seek_relative(
    seconds: f64,
    state: tauri::State<'_, SharedState>,
) -> Result<PlayerStatus, String> {
    player::seek_relative(seconds, state.inner().clone()).await
}

#[tauri::command]
async fn set_volume(
    volume: f64,
    state: tauri::State<'_, SharedState>,
) -> Result<PlayerStatus, String> {
    player::set_volume_cmd(volume, state.inner().clone()).await
}

#[tauri::command]
async fn get_volume(state: tauri::State<'_, SharedState>) -> Result<f64, String> {
    player::get_volume_cmd(state.inner().clone()).await
}

fn toggle_window(window: &WebviewWindow) {
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        let _ = window.move_window(Position::TrayBottomCenter);
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .manage(Arc::new(Mutex::new(player::PlayerState::new())) as SharedState)
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let separator = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit bkgrnd", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&separator, &quit_item])?;
            let menu_state = app.state::<SharedState>().inner().clone();

            let icon = tauri::include_image!("icons/icon.png");

            let _tray = TrayIconBuilder::with_id("main")
                .icon(icon)
                .icon_as_template(false)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("bkgrnd")
                .on_menu_event(move |app, event| {
                    if event.id.as_ref() == "quit" {
                        let app_handle = app.clone();
                        let state = menu_state.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = player::stop(state).await;
                            app_handle.exit(0);
                        });
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);

                    match event {
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } => {
                            if let Some(window) = tray.app_handle().get_webview_window("main") {
                                toggle_window(&window);
                            }
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();

                let window_clone = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(false) = event {
                        let _ = window_clone.hide();
                    }
                });
            }

            // Best-effort playlist sync and WOPR remote control (no UI impact; safe to fail)
            let app_handle = app.handle().clone();
            let shared_state = app.state::<SharedState>().inner().clone();
            tauri::async_runtime::spawn(async {
                mpv::stop_stale_mpv().await;
            });
            tauri::async_runtime::spawn(async {
                wopr_sync::sync_loop(app_handle, shared_state).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            quit_app,
            hide_window,
            resize_window,
            set_tray_state,
            search,
            play,
            toggle_pause,
            stop,
            get_status,
            get_history,
            get_playlists,
            save_playlist_item,
            remove_playlist_item,
            play_next,
            play_prev,
            seek_relative,
            set_volume,
            get_volume,
        ])
        .run(tauri::generate_context!())
        .expect("error while running bkgrnd");
}
