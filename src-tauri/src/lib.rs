// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use once_cell::sync::OnceCell;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::path::BaseDirectory;
use tauri::Manager;

mod paths;
mod process_store;
mod rpc;
mod runner;
#[cfg(test)]
mod safety_tests;

// Global owned process store. All launched game runners are tracked here.
static PROCESS_STORE: OnceCell<Mutex<process_store::ProcessStore>> = OnceCell::new();

// Global managed Discord RPC client.
static DISCORD_CLIENT: OnceCell<tokio::sync::Mutex<Option<rpc::Client>>> = OnceCell::new();

fn get_process_store() -> &'static Mutex<process_store::ProcessStore> {
    PROCESS_STORE.get_or_init(|| Mutex::new(process_store::ProcessStore::new()))
}

fn get_discord_client() -> &'static tokio::sync::Mutex<Option<rpc::Client>> {
    DISCORD_CLIENT.get_or_init(|| tokio::sync::Mutex::new(None))
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// Create the fake-game runner under the application-owned games root.
/// Rejects paths that escape the root, invalid components, or existing targets.
#[tauri::command(rename_all = "snake_case")]
async fn create_fake_game(
    handle: tauri::AppHandle,
    path: &str,
    executable_name: &str,
    path_len: i64,
    app_id: i64,
) -> Result<String, String> {
    let _ = path_len;
    let root = paths::games_root().map_err(|e| format!("failed to locate games root: {}", e))?;
    let target = paths::resolve_confined_path(&root, app_id, path, executable_name)
        .map_err(|e| format!("path validation failed: {}", e))?;

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create game folder: {}", e))?;
    }

    let source = handle.path().resolve("data/src-win.exe", BaseDirectory::Resource)
        .map_err(|e| format!("failed to resolve packaged runner: {e}"))?;
    // Recheck after directory creation and allow only an unchanged packaged
    // runner to be reused; never overwrite unrelated existing content.
    let target = paths::resolve_confined_path(&root, app_id, path, executable_name)
        .map_err(|e| format!("path validation failed: {e}"))?;
    if target.exists() {
        paths::verify_runner(&target, &source)?;
        return Ok("Existing packaged runner verified".into());
    }

    // Do not overwrite existing files to avoid clobbering unrelated content.
    let mut source_file = std::fs::File::open(&source)
        .map_err(|e| format!("failed to open template executable: {}", e))?;
    let mut target_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|e| format!("failed to create target executable: {}", e))?;
    std::io::copy(&mut source_file, &mut target_file)
        .map_err(|e| format!("failed to copy dummy executable: {}", e))?;

    Ok(format!("Dummy executable copied to: {:?}", target))
}

/// Launch a confined runner and return its owned PID.
#[tauri::command(rename_all = "snake_case")]
async fn run_background_process(
    handle: tauri::AppHandle,
    name: &str,
    path: &str,
    executable_name: &str,
    path_len: i64,
    app_id: i64,
) -> Result<u32, String> {
    let _ = path_len;
    let root = paths::games_root().map_err(|e| format!("failed to locate games root: {}", e))?;
    let executable_path = paths::resolve_confined_path(&root, app_id, path, executable_name)
        .map_err(|e| format!("path validation failed: {}", e))?;

    if !executable_path.exists() {
        return Err(format!(
            "executable does not exist (run create_fake_game first): {:?}",
            executable_path
        ));
    }

    let template = handle.path().resolve("data/src-win.exe", BaseDirectory::Resource)
        .map_err(|e| format!("failed to resolve packaged runner: {e}"))?;
    paths::verify_runner(&executable_path, &template)?;

    let game_folder_path = executable_path
        .parent()
        .ok_or_else(|| "executable has no parent directory".to_string())?
        .to_path_buf();

    let mut store = get_process_store().lock().map_err(|_| "process tracking state is unavailable".to_string())?;
    store.running_pids(); // reap natural exits
    if store.has_running_at(&executable_path) {
        return Err(format!("process already running for {:?}", executable_path));
    }

    let child = std::process::Command::new(&executable_path)
        .args(["--title", name])
        .current_dir(&game_folder_path)
        .spawn()
        .map_err(|e| format!("failed to start process: {}", e))?;

    let pid = child.id();
    store
        .insert(child, executable_path)
        .map_err(|e| format!("failed to track process: {}", e))?;
    Ok(pid)
}

/// Stop an owned child process by PID. Rejects unowned IDs.
#[tauri::command(rename_all = "snake_case")]
async fn stop_process(process_id: u32) -> Result<(), String> {
    let mut store = get_process_store().lock().map_err(|_| "process tracking state is unavailable".to_string())?;
    match store.stop(process_id) {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!(
            "process {} is not owned by this application",
            process_id
        )),
        Err(e) => Err(e),
    }
}

/// Return currently owned live child PIDs, reaping natural exits.
#[tauri::command(rename_all = "snake_case")]
fn get_running_processes() -> Result<Vec<u32>, String> {
    let mut store = get_process_store().lock().map_err(|_| "process tracking state is unavailable".to_string())?;
    Ok(store.running_pids())
}

/// Read-only probe: returns true if a Discord IPC pipe is available.
/// Does not publish activity or perform an SDK handshake.
#[cfg(windows)]
#[tauri::command(rename_all = "snake_case")]
fn check_discord_connection() -> Result<bool, String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn WaitNamedPipeW(name: *const u16, timeout: u32) -> i32;
    }
    for i in 0..10u32 {
        let pipe_name = format!(r"\\.\pipe\discord-ipc-{}", i);
        let wide: Vec<u16> = pipe_name.encode_utf16().chain(std::iter::once(0)).collect();
        // WaitNamedPipe does not connect or send anything. A one-millisecond
        // timeout avoids the server-default wait that timeout=0 would select.
        if unsafe { WaitNamedPipeW(wide.as_ptr(), 1) } != 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(not(windows))]
#[tauri::command(rename_all = "snake_case")]
fn check_discord_connection() -> Result<bool, String> {
    // Non-Windows probe: check for Discord IPC socket files in standard locations.
    let candidates = [
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::var_os("TMPDIR").map(PathBuf::from),
        Some(std::env::temp_dir()),
    ];
    for base in candidates.into_iter().flatten() {
        for i in 0..10u32 {
            if base.join(format!("discord-ipc-{}", i)).exists() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Connect or disconnect the managed Discord RPC connection.
/// Returns after the operation succeeds or fails; no per-call listeners are leaked.
#[tauri::command(rename_all = "snake_case")]
async fn connect_to_discord_rpc_3(
    activity_json: String,
    action: String,
) -> Result<(), String> {
    match action.as_str() {
        "connect" => connect_rpc(activity_json).await,
        "disconnect" => disconnect_rpc().await,
        other => Err(format!("unsupported action: {}", other)),
    }
}

async fn connect_rpc(activity_json: String) -> Result<(), String> {
    // Serialize connection lifecycle so an older pending connect cannot revive
    // activity after a newer disconnect has completed.
    let mut guard = get_discord_client().lock().await;
    if let Some(previous) = guard.take() {
        tokio::time::timeout(std::time::Duration::from_secs(5), previous.discord.disconnect())
            .await.map_err(|_| "timed out disconnecting previous activity".to_string())?;
    }
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let activity = runner::create_activity(activity_json)?;
        let app_id = i64::try_from(activity.app_id).map_err(|_| "application ID is out of range".to_string())?;
        let client = rpc::make_client(app_id, rpc::ds::Subscriptions::ACTIVITY).await?;
        client.discord.update_activity(activity.activity).await
            .map_err(|error| format!("failed to update activity: {error}"))?;
        Ok::<rpc::Client, String>(client)
    }).await.map_err(|_| "timed out connecting activity".to_string())??;
    *guard = Some(result);
    Ok(())
}

async fn disconnect_rpc() -> Result<(), String> {
    let mut guard = get_discord_client().lock().await;
    if let Some(client) = guard.take() {
        tokio::time::timeout(std::time::Duration::from_secs(5), client.discord.disconnect())
            .await.map_err(|_| "timed out disconnecting activity".to_string())?;
    }
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
async fn disconnect_from_discord_rpc() -> Result<(), String> {
    disconnect_rpc().await
}

async fn fetch_json_with_timeout(url: &str) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("http error: {}", response.status()));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("failed to read body: {}", e))?;

    serde_json::from_str(&body).map_err(|e| format!("invalid json: {}", e))
}

#[tauri::command(rename_all = "snake_case")]
async fn fetch_gamelist_gh_mirror() -> Result<serde_json::Value, String> {
    fetch_json_with_timeout(
        "https://markterence.github.io/discord-quest-completer/detectable.json",
    )
    .await
}

#[tauri::command(rename_all = "snake_case")]
async fn fetch_gamelist_from_discord() -> Result<serde_json::Value, String> {
    fetch_json_with_timeout("https://discord.com/api/applications/detectable").await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            create_fake_game,
            run_background_process,
            stop_process,
            get_running_processes,
            connect_to_discord_rpc_3,
            disconnect_from_discord_rpc,
            check_discord_connection,
            fetch_gamelist_gh_mirror,
            fetch_gamelist_from_discord
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                if let Ok(mut store) = get_process_store().lock() { store.cleanup(); }
            }
        });
}
