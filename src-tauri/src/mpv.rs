use std::path::PathBuf;
use tauri::AppHandle;
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};

fn ipc_path() -> String {
    let suffix = std::env::var("USER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| std::process::id().to_string());
    format!("/tmp/bkgrnd-mpv-ipc-{}", suffix)
}

pub struct MpvSession {
    pub child: Child,
}

fn mpv_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|e| format!("Failed to get resource dir: {}", e))?;
    Ok(resource_dir.join("mpv-bundle"))
}

pub async fn spawn_mpv(
    app: &AppHandle,
    stream_url: &str,
    _title: &str,
    _url: &str,
) -> Result<MpvSession, String> {
    let ipc_path = ipc_path();

    // Clean up stale IPC socket
    let _ = std::fs::remove_file(&ipc_path);

    let mpv_bundle_dir = mpv_dir(app)?;
    let mpv_bin = mpv_bundle_dir.join("mpv");

    if !mpv_bin.exists() {
        return Err(format!("mpv binary not found at {:?}", mpv_bin));
    }

    eprintln!(
        "[mpv] Spawning: {:?} with DYLD_LIBRARY_PATH={:?}",
        mpv_bin, mpv_bundle_dir
    );

    let mut child = Command::new(&mpv_bin)
        .env("DYLD_LIBRARY_PATH", &mpv_bundle_dir)
        .args([
            "--no-video",
            &format!("--input-ipc-server={}", ipc_path),
            "--really-quiet",
            "--terminal=no",
            stream_url,
        ])
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to spawn mpv: {}", e))?;

    // Wait for IPC socket to become available (up to 5s)
    if let Err(e) = wait_for_ipc(&ipc_path, 5000).await {
        // Check if mpv already exited
        if let Ok(Some(status)) = child.try_wait() {
            // Read stderr for diagnostics
            if let Some(stderr) = child.stderr.take() {
                use tokio::io::AsyncReadExt;
                let mut buf = String::new();
                let mut reader = stderr;
                let _ = reader.read_to_string(&mut buf).await;
                if !buf.is_empty() {
                    eprintln!("[mpv] stderr: {}", buf.trim());
                }
            }
            return Err(format!(
                "mpv exited immediately with code {:?}: {}",
                status.code(),
                e
            ));
        }
        return Err(e);
    }

    Ok(MpvSession { child })
}

async fn wait_for_ipc(path: &str, timeout_ms: u64) -> Result<(), String> {
    let start = std::time::Instant::now();
    loop {
        if std::path::Path::new(path).exists() {
            return Ok(());
        }
        if start.elapsed().as_millis() as u64 > timeout_ms {
            return Err("mpv IPC socket did not appear".to_string());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

pub async fn mpv_command(command: &[serde_json::Value]) -> Result<serde_json::Value, String> {
    let stream = UnixStream::connect(ipc_path())
        .await
        .map_err(|e| format!("mpv IPC connect failed: {}", e))?;

    let (reader, mut writer) = stream.into_split();

    let msg = serde_json::json!({ "command": command });
    let mut msg_str = serde_json::to_string(&msg).unwrap();
    msg_str.push('\n');

    writer
        .write_all(msg_str.as_bytes())
        .await
        .map_err(|e| format!("mpv IPC write failed: {}", e))?;

    let mut buf_reader = BufReader::new(reader);

    // Read response with timeout
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        let mut line = String::new();
        loop {
            line.clear();
            let n = buf_reader
                .read_line(&mut line)
                .await
                .map_err(|e| format!("mpv IPC read failed: {}", e))?;
            if n == 0 {
                return Err("mpv IPC connection closed".to_string());
            }

            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                if parsed.get("data").is_some() {
                    return Ok(parsed["data"].clone());
                }
                if parsed.get("error").and_then(|e| e.as_str()) == Some("success") {
                    return Ok(serde_json::Value::Null);
                }
            }
        }
    })
    .await;

    match result {
        Ok(inner) => inner,
        Err(_) => Err("mpv IPC timeout".to_string()),
    }
}

pub async fn pause() -> Result<(), String> {
    mpv_command(&[serde_json::json!("cycle"), serde_json::json!("pause")]).await?;
    Ok(())
}

pub async fn stop_mpv(session: &mut MpvSession) {
    let _ = mpv_command(&[serde_json::json!("quit")]).await;
    // Fallback: kill process
    let _ = session.child.start_kill();
    let _ = session.child.wait().await;
    let _ = std::fs::remove_file(ipc_path());
}

pub async fn stop_stale_mpv() {
    let _ = mpv_command(&[serde_json::json!("quit")]).await;
    let _ = std::fs::remove_file(ipc_path());
}

pub async fn get_paused() -> bool {
    match mpv_command(&[
        serde_json::json!("get_property"),
        serde_json::json!("pause"),
    ])
    .await
    {
        Ok(val) => val.as_bool().unwrap_or(false),
        Err(_) => false,
    }
}

pub async fn get_volume() -> f64 {
    match mpv_command(&[
        serde_json::json!("get_property"),
        serde_json::json!("volume"),
    ])
    .await
    {
        Ok(val) => val.as_f64().unwrap_or(100.0),
        Err(_) => 100.0,
    }
}

async fn get_number_property(name: &str) -> Option<f64> {
    match mpv_command(&[serde_json::json!("get_property"), serde_json::json!(name)]).await {
        Ok(val) => val.as_f64().filter(|value| value.is_finite()),
        Err(_) => None,
    }
}

pub async fn get_time_position() -> Option<f64> {
    get_number_property("time-pos").await
}

pub async fn get_duration() -> Option<f64> {
    get_number_property("duration").await
}

pub async fn set_volume(vol: f64) -> Result<(), String> {
    mpv_command(&[
        serde_json::json!("set_property"),
        serde_json::json!("volume"),
        serde_json::json!(vol),
    ])
    .await?;
    Ok(())
}

pub async fn seek(seconds: f64) -> Result<(), String> {
    mpv_command(&[
        serde_json::json!("seek"),
        serde_json::json!(seconds),
        serde_json::json!("relative"),
    ])
    .await?;
    Ok(())
}
