//! A small log file (`ledge.log` in the app's log folder) so problems on another machine can be read
//! afterwards: UI errors arrive through `log_line`, Rust side calls `diag::log`.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

const MAX_BYTES: u64 = 512 * 1024;

static PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn init(app: &AppHandle) {
    if let Ok(dir) = app.path().app_log_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = PATH.set(dir.join("ledge.log"));
    }
    log(&format!("start {} on {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS));
}

pub fn log(msg: &str) {
    eprintln!("[ledge] {msg}");
    let Some(path) = PATH.get() else { return };
    // keep one old file instead of growing forever
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(path, path.with_extension("old.log"));
    }
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{secs} {msg}");
    }
}

#[tauri::command]
pub fn log_line(msg: String) {
    log(&format!("ui: {}", msg.chars().take(2000).collect::<String>()));
}
