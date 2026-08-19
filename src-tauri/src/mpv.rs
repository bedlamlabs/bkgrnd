use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::AppHandle;
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

// mpv's JSON IPC transport differs by platform: a Unix domain socket under /tmp
// on macOS/Linux, a named pipe on Windows. `IpcStream` + `connect_ipc` hide the
// difference; everything else reads/writes generically via tokio::io::split.
#[cfg(unix)]
type IpcStream = tokio::net::UnixStream;
#[cfg(windows)]
type IpcStream = tokio::net::windows::named_pipe::NamedPipeClient;

async fn connect_ipc(path: &str) -> std::io::Result<IpcStream> {
    #[cfg(unix)]
    {
        tokio::net::UnixStream::connect(path).await
    }
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ClientOptions;
        // ERROR_PIPE_BUSY (231): server exists but is momentarily busy — retry.
        loop {
            match ClientOptions::new().open(path) {
                Ok(client) => return Ok(client),
                Err(e) if e.raw_os_error() == Some(231) => {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

static IPC_COUNTER: AtomicU64 = AtomicU64::new(0);

// Each mpv session gets its own socket/pipe so a crashed/stale mpv (or a second
// app instance) can never intercept commands meant for the current session.
fn new_ipc_path() -> String {
    let id = std::process::id();
    let n = IPC_COUNTER.fetch_add(1, Ordering::Relaxed);
    #[cfg(unix)]
    {
        format!("/tmp/bkgrnd-mpv-ipc-{id}-{n}")
    }
    #[cfg(windows)]
    {
        format!(r"\\.\pipe\bkgrnd-mpv-ipc-{id}-{n}")
    }
}

pub struct MpvSession {
    pub child: Child,
    pub ipc_path: String,
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
    let ipc_path = new_ipc_path();
    // Unix sockets are files to clean up; Windows named pipes are not.
    #[cfg(unix)]
    let _ = std::fs::remove_file(&ipc_path);

    let mpv_bundle_dir = mpv_dir(app)?;
    #[cfg(unix)]
    let mpv_bin = mpv_bundle_dir.join("mpv");
    #[cfg(windows)]
    let mpv_bin = mpv_bundle_dir.join("mpv.exe");

    if !mpv_bin.exists() {
        return Err(format!("mpv binary not found at {:?}", mpv_bin));
    }

    eprintln!("[mpv] Spawning: {:?}", mpv_bin);

    let mut command = Command::new(&mpv_bin);
    // macOS: mpv finds its dylibs via DYLD_LIBRARY_PATH. Windows: the DLLs sit
    // next to mpv.exe in the same dir, so no env is needed.
    #[cfg(target_os = "macos")]
    command.env("DYLD_LIBRARY_PATH", &mpv_bundle_dir);
    if std::env::var("BKGRND_VERIFY_MUTE_AUDIO").as_deref() == Ok("1") {
        command.arg("--ao=null");
    }
    let mut child = command
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

    Ok(MpvSession { child, ipc_path })
}

async fn wait_for_ipc(path: &str, timeout_ms: u64) -> Result<(), String> {
    let start = std::time::Instant::now();
    loop {
        // Unix: the socket appears as a file. Windows: probe by connecting (the
        // named pipe isn't a filesystem path).
        #[cfg(unix)]
        let ready = std::path::Path::new(path).exists();
        #[cfg(windows)]
        let ready = connect_ipc(path).await.is_ok();
        if ready {
            return Ok(());
        }
        if start.elapsed().as_millis() as u64 > timeout_ms {
            return Err("mpv IPC socket did not appear".to_string());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

pub async fn mpv_command(
    ipc_path: &str,
    command: &[serde_json::Value],
) -> Result<serde_json::Value, String> {
    let stream = connect_ipc(ipc_path)
        .await
        .map_err(|e| format!("mpv IPC connect failed: {}", e))?;

    let (reader, mut writer) = tokio::io::split(stream);

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

#[derive(Debug, Default, Clone, Copy)]
pub struct StatusSnapshot {
    pub paused: bool,
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub buffering: bool,
    pub core_idle: bool,
}

/// Read playback state over ONE IPC connection. Status is polled
/// every 1-2s by both the popover and the WOPR sync loop; three separate
/// connects per poll (each with its own 3s timeout) made a wedged mpv stall
/// callers for ~9s.
pub async fn status_snapshot(ipc_path: &str) -> StatusSnapshot {
    let Ok(stream) = connect_ipc(ipc_path).await else {
        return StatusSnapshot::default();
    };
    let (reader, mut writer) = tokio::io::split(stream);

    let properties = [
        "pause",
        "time-pos",
        "duration",
        "paused-for-cache",
        "core-idle",
    ];
    let mut request = String::new();
    for (index, property) in properties.iter().enumerate() {
        let msg = serde_json::json!({
            "command": ["get_property", property],
            "request_id": index + 1,
        });
        request.push_str(&serde_json::to_string(&msg).unwrap());
        request.push('\n');
    }
    if writer.write_all(request.as_bytes()).await.is_err() {
        return StatusSnapshot::default();
    }

    let mut answers: [Option<serde_json::Value>; 5] = [None, None, None, None, None];
    let mut buf_reader = BufReader::new(reader);
    let read_result = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        let mut line = String::new();
        let mut received = 0usize;
        while received < properties.len() {
            line.clear();
            match buf_reader.read_line(&mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let Ok(parsed) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
                continue;
            };
            let Some(request_id) = parsed.get("request_id").and_then(|v| v.as_u64()) else {
                continue; // unsolicited event, ignore
            };
            let index = (request_id as usize).wrapping_sub(1);
            if index < answers.len() && answers[index].is_none() {
                answers[index] = Some(
                    parsed
                        .get("data")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                );
                received += 1;
            }
        }
    })
    .await;
    let _ = read_result;

    let number = |value: &Option<serde_json::Value>| {
        value
            .as_ref()
            .and_then(|v| v.as_f64())
            .filter(|v| v.is_finite())
    };

    StatusSnapshot {
        paused: answers[0]
            .as_ref()
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        position: number(&answers[1]),
        duration: number(&answers[2]).filter(|duration| *duration > 0.0),
        buffering: answers[3]
            .as_ref()
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        core_idle: answers[4]
            .as_ref()
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }
}

pub async fn pause(ipc_path: &str) -> Result<(), String> {
    mpv_command(
        ipc_path,
        &[serde_json::json!("cycle"), serde_json::json!("pause")],
    )
    .await?;
    Ok(())
}

pub async fn stop_mpv(session: &mut MpvSession) {
    let _ = mpv_command(&session.ipc_path, &[serde_json::json!("quit")]).await;
    // Fallback: kill process
    let _ = session.child.start_kill();
    let _ = session.child.wait().await;
    let _ = std::fs::remove_file(&session.ipc_path);
}

/// Startup cleanup: quit any mpv left over from a previous run (crash, force
/// quit) and remove its orphaned sockets. Sockets are namespaced per PID, so
/// anything on disk that isn't ours is stale.
pub async fn stop_stale_mpv() {
    // Only Unix leaves socket files behind; Windows named pipes vanish with the
    // process, so there's nothing to sweep.
    #[cfg(unix)]
    {
        const IPC_PREFIX: &str = "/tmp/bkgrnd-mpv-ipc-";
        let Ok(entries) = std::fs::read_dir("/tmp") else {
            return;
        };
        let own_prefix = format!("{}{}-", IPC_PREFIX, std::process::id());
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(path_str) = path.to_str() else {
                continue;
            };
            if !path_str.starts_with(IPC_PREFIX) || path_str.starts_with(&own_prefix) {
                continue;
            }
            let _ = mpv_command(path_str, &[serde_json::json!("quit")]).await;
            let _ = std::fs::remove_file(&path);
        }
    }
}

pub async fn get_volume(ipc_path: &str) -> f64 {
    match mpv_command(
        ipc_path,
        &[
            serde_json::json!("get_property"),
            serde_json::json!("volume"),
        ],
    )
    .await
    {
        Ok(val) => val.as_f64().unwrap_or(100.0),
        Err(_) => 100.0,
    }
}

pub async fn set_volume(ipc_path: &str, vol: f64) -> Result<(), String> {
    mpv_command(
        ipc_path,
        &[
            serde_json::json!("set_property"),
            serde_json::json!("volume"),
            serde_json::json!(vol),
        ],
    )
    .await?;
    Ok(())
}

pub async fn seek(ipc_path: &str, seconds: f64) -> Result<(), String> {
    mpv_command(
        ipc_path,
        &[
            serde_json::json!("seek"),
            serde_json::json!(seconds),
            serde_json::json!("relative"),
        ],
    )
    .await?;
    Ok(())
}

pub async fn seek_absolute(ipc_path: &str, seconds: f64) -> Result<(), String> {
    mpv_command(
        ipc_path,
        &[
            serde_json::json!("seek"),
            serde_json::json!(seconds),
            serde_json::json!("absolute"),
        ],
    )
    .await?;
    Ok(())
}
