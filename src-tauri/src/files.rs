//! File transfer between my devices over the Tailscale link opened by `sync`.
//! Wire: one JSON header line `{"t":"file",...,"size":N}\n`, then N raw bytes, then a JSON reply.

use serde::Serialize;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

const CHUNK: usize = 256 * 1024;
const PROGRESS_EVERY: Duration = Duration::from_millis(200);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Serialize, Clone, Debug)]
pub struct Transfer {
    pub id: u64,
    pub name: String,
    pub peer: String,
    pub incoming: bool,
    pub done: u64,
    pub total: u64,
    /// "active" | "done" | "failed"
    pub state: &'static str,
    pub error: Option<String>,
    pub path: Option<String>,
}

impl Transfer {
    pub fn new(name: &str, peer: &str, incoming: bool, total: u64) -> Self {
        Transfer {
            id: NEXT_ID.fetch_add(1, Ordering::SeqCst),
            name: name.into(),
            peer: peer.into(),
            incoming,
            done: 0,
            total,
            state: "active",
            error: None,
            path: None,
        }
    }

    pub fn emit(&self, app: &AppHandle) {
        let _ = app.emit("transfer", self.clone());
    }

    pub fn finish(mut self, app: &AppHandle, result: Result<Option<PathBuf>, String>) {
        match result {
            Ok(path) => {
                self.state = "done";
                self.done = self.total;
                self.path = path.map(|p| p.to_string_lossy().into_owned());
            }
            Err(e) => {
                self.state = "failed";
                self.error = Some(e);
            }
        }
        self.emit(app);
    }
}

/// Reduce a sender-supplied name to a plain file name: no directories, no traversal, no control chars.
pub fn safe_name(name: &str) -> Option<String> {
    let base = name.rsplit(['/', '\\']).next()?;
    let clean: String = base.chars().filter(|c| !c.is_control() && *c != ':').collect();
    let clean = clean.trim().trim_start_matches('.').to_string();
    (!clean.is_empty() && clean.len() <= 255).then_some(clean)
}

/// `dir/name`, or `dir/stem (2).ext` … if taken.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let p = Path::new(name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let ext = p.extension().and_then(|s| s.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|c| !c.exists())
        .expect("unbounded range always finds a free name")
}

/// Copy exactly `size` bytes from `src` into `dir` (via a hidden partial file), returning the final path.
pub fn receive_into(dir: &Path, name: &str, size: u64, src: &mut impl Read, mut tick: impl FnMut(u64)) -> io::Result<PathBuf> {
    let name = safe_name(name).ok_or_else(|| io::Error::other("bad file name"))?;
    let partial = dir.join(format!(".{name}.{}.ledge-part", NEXT_ID.fetch_add(1, Ordering::SeqCst)));
    let result = (|| {
        let mut out = File::create_new(&partial)?;
        let copied = copy_with_progress(&mut src.take(size), &mut out, &mut tick)?;
        if copied != size {
            return Err(io::Error::other(format!("connection dropped at {copied} of {size} bytes")));
        }
        out.sync_all()?;
        let dest = unique_path(dir, &name);
        fs::rename(&partial, &dest)?;
        Ok(dest)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

pub fn copy_with_progress(src: &mut impl Read, dst: &mut impl Write, tick: &mut impl FnMut(u64)) -> io::Result<u64> {
    let mut buf = vec![0u8; CHUNK];
    let (mut total, mut last) = (0u64, Instant::now());
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n])?;
        total += n as u64;
        if last.elapsed() >= PROGRESS_EVERY {
            tick(total);
            last = Instant::now();
        }
    }
    Ok(total)
}

pub fn downloads(app: &AppHandle) -> io::Result<PathBuf> {
    app.path().download_dir().map_err(io::Error::other)
}

/// Show a received file in Finder — only paths inside Downloads.
#[tauri::command]
pub fn files_reveal(app: AppHandle, path: String) -> Result<(), String> {
    let dl = downloads(&app).map_err(|e| e.to_string())?.canonicalize().map_err(|e| e.to_string())?;
    let p = Path::new(&path).canonicalize().map_err(|_| "file no longer exists".to_string())?;
    if !p.starts_with(&dl) {
        return Err("can only reveal files in Downloads".into());
    }
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg("-R").arg(&p).status();
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).status();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let status = std::process::Command::new("xdg-open").arg(&dl).status();
    status.map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_name_strips_paths_and_traversal() {
        assert_eq!(safe_name("report.pdf").as_deref(), Some("report.pdf"));
        assert_eq!(safe_name("../../etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(safe_name("C:\\Users\\x\\a.txt").as_deref(), Some("a.txt"));
        assert_eq!(safe_name(".bashrc").as_deref(), Some("bashrc"));
        assert_eq!(safe_name("..").as_deref(), None);
        assert_eq!(safe_name("a\u{0}b\nc.txt").as_deref(), Some("abc.txt"));
        assert_eq!(safe_name("").as_deref(), None);
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ledge-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn receive_writes_file_and_never_overwrites() {
        let dir = tmpdir("recv");
        let a = receive_into(&dir, "x/../note.txt", 5, &mut &b"hello"[..], |_| {}).unwrap();
        let b = receive_into(&dir, "note.txt", 3, &mut &b"bye"[..], |_| {}).unwrap();
        assert_eq!(a, dir.join("note.txt"));
        assert_eq!(b, dir.join("note (2).txt"));
        assert_eq!(fs::read_to_string(&a).unwrap(), "hello");
        assert_eq!(fs::read_to_string(&b).unwrap(), "bye");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn short_body_fails_and_leaves_nothing_behind() {
        let dir = tmpdir("short");
        assert!(receive_into(&dir, "big.bin", 10, &mut &b"abc"[..], |_| {}).is_err());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        fs::remove_dir_all(&dir).unwrap();
    }
}
