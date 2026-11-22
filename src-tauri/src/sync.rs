//! Clipboard sync between your own devices over Tailscale.
//!
//! Security model: we bind only to this machine's Tailscale IP (WireGuard-encrypted,
//! device-authenticated) and accept connections only from devices owned by the same
//! Tailscale user. No extra keys to manage.

use crate::clip;
use crate::files::{self, Transfer};
use crate::update;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};

const PORT: u16 = 58431;
const MAX_TEXT: usize = 1024 * 1024;
const REFRESH_EVERY: Duration = Duration::from_secs(20);
const BIND_RETRY: Duration = Duration::from_secs(2);
const POLL_CLIPBOARD: Duration = Duration::from_millis(400);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(900);
const IO_TIMEOUT: Duration = Duration::from_secs(4);
const FILE_IO_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_UPDATE: u64 = 300 * 1024 * 1024;
#[cfg(target_os = "macos")]
const MY_OS: &str = "macOS";
#[cfg(not(target_os = "macos"))]
const MY_OS: &str = "windows";
const TAILSCALE_PATHS: [&str; 5] = [
    r"C:\Program Files\Tailscale\tailscale.exe",
    "/usr/local/bin/tailscale",
    "/opt/homebrew/bin/tailscale",
    "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    "tailscale",
];

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Peer {
    pub name: String,
    pub ip: String,
    pub os: String,
    pub online: bool,
    /// Ledge answered on this peer.
    pub dock: bool,
    /// That peer's Ledge version, when it's new enough to report one.
    pub version: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Me {
    pub name: String,
    pub ip: String,
    pub user: i64,
}

#[derive(Serialize, Clone, Debug)]
pub struct LastClip {
    pub peer: String,
    pub incoming: bool,
    pub preview: String,
    pub at: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Snapshot {
    pub enabled: bool,
    /// Our listener is up, so other devices can reach this Ledge.
    pub listening: bool,
    pub version: &'static str,
    /// A same-OS peer running a newer Ledge we can update from.
    pub update: Option<Peer>,
    pub me: Option<String>,
    pub peers: Vec<Peer>,
    pub last: Option<LastClip>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "t", rename_all = "lowercase")]
enum Msg {
    Ping { from: String },
    Pong {
        from: String,
        #[serde(default)]
        ver: String,
    },
    /// Ask a peer for its own app bundle; it answers `Update` + `size` raw zip bytes.
    GetUpdate { from: String },
    Update { ver: String, size: u64 },
    Clip { from: String, text: String },
    /// The dock's settings as a JSON object of storage key → value.
    Settings { from: String, json: String },
    /// Followed on the wire by exactly `size` raw bytes.
    File { from: String, name: String, size: u64 },
    Ok,
    Err { msg: String },
}

#[derive(Default)]
pub struct Sync {
    enabled: AtomicBool,
    listening: AtomicBool,
    me: Mutex<Option<Me>>,
    peers: Mutex<Vec<Peer>>,
    last: Mutex<Option<LastClip>>,
    /// Change count produced by our own write of a received clip — don't echo it back.
    ignore_cc: AtomicIsize,
}

impl Sync {
    fn snapshot(&self) -> Snapshot {
        let peers = self.peers.lock().unwrap().clone();
        Snapshot {
            enabled: self.enabled.load(Ordering::SeqCst),
            listening: self.listening.load(Ordering::SeqCst),
            version: update::VERSION,
            update: peers
                .iter()
                .filter(|p| p.online && p.dock && p.os == MY_OS)
                .filter(|p| p.version.as_deref().is_some_and(|v| update::newer(v, update::VERSION)))
                .reduce(|a, b| if update::newer(b.version.as_deref().unwrap_or(""), a.version.as_deref().unwrap_or("")) { b } else { a })
                .cloned(),
            me: self.me.lock().unwrap().as_ref().map(|m| m.name.clone()),
            peers,
            last: self.last.lock().unwrap().clone(),
        }
    }

    fn my_name(&self) -> String {
        self.me.lock().unwrap().as_ref().map(|m| m.name.clone()).unwrap_or_default()
    }

    fn is_allowed(&self, ip: &IpAddr) -> bool {
        let ip = ip.to_string();
        let me = self.me.lock().unwrap();
        me.as_ref().is_some_and(|m| m.ip == ip) || self.peers.lock().unwrap().iter().any(|p| p.ip == ip)
    }

    fn set_last(&self, peer: &str, incoming: bool, text: &str) {
        let preview: String = text.chars().take(80).collect();
        let at = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        *self.last.lock().unwrap() = Some(LastClip { peer: peer.into(), incoming, preview, at });
    }

    fn mark_dock(&self, ip: &str) {
        self.peers.lock().unwrap().iter_mut().filter(|p| p.ip == ip).for_each(|p| p.dock = true);
    }
}

/// Parses `tailscale status --json`: me + online-or-not peers owned by the same user.
pub fn parse_status(json: &str) -> Option<(Me, Vec<Peer>)> {
    let v: Value = serde_json::from_str(json).ok()?;
    let ip4 = |n: &Value| -> Option<String> {
        n["TailscaleIPs"].as_array()?.iter().filter_map(|i| i.as_str()).find(|i| i.contains('.')).map(String::from)
    };
    let me_node = &v["Self"];
    let me = Me { name: me_node["HostName"].as_str()?.to_string(), ip: ip4(me_node)?, user: me_node["UserID"].as_i64()? };
    let mut peers: Vec<Peer> = v["Peer"]
        .as_object()
        .map(|m| m.values().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p["UserID"].as_i64() == Some(me.user))
        .filter_map(|p| {
            Some(Peer {
                name: p["HostName"].as_str()?.to_string(),
                ip: ip4(p)?,
                os: p["OS"].as_str().unwrap_or("").to_string(),
                online: p["Online"].as_bool().unwrap_or(false),
                dock: false,
                version: None,
            })
        })
        .collect();
    peers.sort_by(|a, b| b.online.cmp(&a.online).then(a.name.cmp(&b.name)));
    Some((me, peers))
}

/// The Tailscale app's binary only acts as a CLI when started from a terminal (TERM set) or told to;
/// launched from Finder, Ledge has no TERM and the binary tries to open the GUI instead.
fn status_cmd(bin: &str) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args(["status", "--json"]).env("TAILSCALE_BE_CLI", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashing up on every poll
    }
    cmd
}

/// Ask every tailscale CLI we can find; prefer the one whose own IP is on this machine.
/// A second node (another tailscaled, a userspace/tsnet daemon) answers with an IP we can't listen on.
fn local_status() -> Option<(Me, Vec<Peer>)> {
    let found: Vec<(Me, Vec<Peer>)> = TAILSCALE_PATHS
        .iter()
        .filter_map(|bin| {
            let out = status_cmd(bin).output().ok()?;
            out.status.success().then(|| parse_status(&String::from_utf8_lossy(&out.stdout))).flatten()
        })
        .collect();
    let local = found.iter().position(|(me, _)| TcpListener::bind((me.ip.as_str(), 0)).is_ok());
    found.into_iter().nth(local.unwrap_or(0))
}

/// Connect and write the header line; the caller may stream a body before `finish`.
fn open(ip: &str, msg: &Msg, timeout: Duration) -> std::io::Result<TcpStream> {
    let addr: SocketAddr = format!("{ip}:{PORT}").parse().map_err(std::io::Error::other)?;
    let mut s = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))?;
    let mut head = serde_json::to_vec(msg)?;
    head.push(b'\n');
    s.write_all(&head)?;
    Ok(s)
}

fn finish(s: TcpStream) -> std::io::Result<Option<Msg>> {
    s.shutdown(Shutdown::Write)?;
    let mut buf = Vec::new();
    s.take(64 * 1024).read_to_end(&mut buf)?;
    Ok(serde_json::from_slice(&buf).ok())
}

fn send(ip: &str, msg: &Msg) -> std::io::Result<Option<Msg>> {
    finish(open(ip, msg, IO_TIMEOUT)?)
}

fn refresh(sync: &Sync, app: &AppHandle) {
    let Some((me, mut peers)) = local_status() else {
        *sync.me.lock().unwrap() = None;
        sync.peers.lock().unwrap().clear();
        let _ = app.emit("sync", sync.snapshot());
        return;
    };
    let from = me.name.clone();
    thread::scope(|sc| {
        for p in peers.iter_mut().filter(|p| p.online) {
            let from = from.clone();
            sc.spawn(move || match send(&p.ip, &Msg::Ping { from }) {
                Ok(Some(Msg::Pong { ver, .. })) => {
                    p.dock = true;
                    p.version = (!ver.is_empty()).then_some(ver);
                }
                _ => p.dock = false,
            });
        }
    });
    *sync.me.lock().unwrap() = Some(me);
    *sync.peers.lock().unwrap() = peers;
    let _ = app.emit("sync", sync.snapshot());
}

fn handle(sync: &Sync, app: &AppHandle, mut s: TcpStream) -> std::io::Result<()> {
    let peer_ip = s.peer_addr()?.ip();
    if !sync.is_allowed(&peer_ip) {
        return Ok(()); // drop silently: not one of my devices
    }
    s.set_read_timeout(Some(IO_TIMEOUT))?;
    let mut reader = BufReader::new(s.try_clone()?);
    let mut head = String::new();
    (&mut reader).take((MAX_TEXT + 4096) as u64).read_line(&mut head)?;
    let reply = match serde_json::from_str::<Msg>(&head) {
        Ok(Msg::Ping { .. }) => {
            sync.mark_dock(&peer_ip.to_string());
            Msg::Pong { from: sync.my_name(), ver: update::VERSION.into() }
        }
        Ok(Msg::GetUpdate { .. }) => return serve_update(&mut s),
        Ok(Msg::Clip { from, text }) if text.len() <= MAX_TEXT => {
            sync.mark_dock(&peer_ip.to_string());
            if sync.enabled.load(Ordering::SeqCst) && clip::read_text().as_deref() != Some(text.as_str()) {
                sync.ignore_cc.store(clip::write_text(&text), Ordering::SeqCst);
                sync.set_last(&from, true, &text);
                let _ = app.emit("sync", sync.snapshot());
            }
            Msg::Ok
        }
        Ok(Msg::Settings { from, json }) if json.len() <= MAX_TEXT => {
            sync.mark_dock(&peer_ip.to_string());
            let _ = app.emit("settings-sync", serde_json::json!({ "from": from, "json": json }));
            Msg::Ok
        }
        Ok(Msg::File { from, name, size }) => {
            sync.mark_dock(&peer_ip.to_string());
            s.set_read_timeout(Some(FILE_IO_TIMEOUT))?;
            receive_file(app, &from, &name, size, &mut reader)
        }
        _ => return Ok(()),
    };
    s.write_all(&serde_json::to_vec(&reply)?)
}

/// Stream our own bundle: header line, then the zip bytes.
fn serve_update(s: &mut TcpStream) -> std::io::Result<()> {
    s.set_write_timeout(Some(FILE_IO_TIMEOUT))?;
    let zip = match update::pack() {
        Ok(z) => z,
        Err(e) => return s.write_all(&serde_json::to_vec(&Msg::Err { msg: e.to_string() })?),
    };
    let result = (|| {
        let size = std::fs::metadata(&zip)?.len();
        let mut head = serde_json::to_vec(&Msg::Update { ver: update::VERSION.into(), size })?;
        head.push(b'\n');
        s.write_all(&head)?;
        std::io::copy(&mut std::fs::File::open(&zip)?, s).map(|_| ())
    })();
    let _ = std::fs::remove_file(&zip);
    result
}

fn receive_file(app: &AppHandle, from: &str, name: &str, size: u64, body: &mut impl Read) -> Msg {
    let mut t = Transfer::new(&files::safe_name(name).unwrap_or_default(), from, true, size);
    t.emit(app);
    let result = files::downloads(app).and_then(|dir| {
        files::receive_into(&dir, name, size, body, |done| {
            t.done = done;
            t.emit(app);
        })
    });
    let reply = match &result {
        Ok(_) => Msg::Ok,
        Err(e) => Msg::Err { msg: e.to_string() },
    };
    t.finish(app, result.map(Some).map_err(|e| e.to_string()));
    reply
}

fn send_file(app: &AppHandle, ip: &str, peer: &str, from: &str, path: &Path) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut t = Transfer::new(&name, peer, false, size);
    t.emit(app);
    let result = (|| -> std::io::Result<Option<Msg>> {
        let mut src = std::fs::File::open(path)?;
        let mut s = open(ip, &Msg::File { from: from.into(), name: name.clone(), size }, FILE_IO_TIMEOUT)?;
        files::copy_with_progress(&mut (&mut src).take(size), &mut s, &mut |done| {
            t.done = done;
            t.emit(app);
        })?;
        finish(s)
    })();
    let outcome = match result {
        Ok(Some(Msg::Ok)) => Ok(None),
        Ok(Some(Msg::Err { msg })) => Err(msg),
        Ok(_) => Err(format!("{peer} did not confirm the file")),
        Err(e) => Err(e.to_string()),
    };
    t.finish(app, outcome);
}

fn serve(sync: Arc<Sync>, app: AppHandle) {
    loop {
        let ip = sync.me.lock().unwrap().as_ref().map(|m| m.ip.clone());
        let listener = ip.and_then(|ip| TcpListener::bind((ip.as_str(), PORT)).ok());
        let Some(listener) = listener else {
            sync.listening.store(false, Ordering::SeqCst);
            thread::sleep(BIND_RETRY);
            continue;
        };
        sync.listening.store(true, Ordering::SeqCst);
        let _ = app.emit("sync", sync.snapshot());
        for conn in listener.incoming().flatten() {
            let (sync, app) = (sync.clone(), app.clone());
            thread::spawn(move || {
                if let Err(e) = handle(&sync, &app, conn) {
                    eprintln!("[sync] connection error: {e}");
                }
            });
        }
    }
}

fn watch_clipboard(sync: Arc<Sync>, app: AppHandle) {
    let mut last = clip::change_count();
    loop {
        thread::sleep(POLL_CLIPBOARD);
        let cc = clip::change_count();
        if cc == last {
            continue;
        }
        last = cc;
        if cc == sync.ignore_cc.load(Ordering::SeqCst) || !sync.enabled.load(Ordering::SeqCst) {
            continue;
        }
        let Some(text) = clip::read_text().filter(|t| !t.is_empty() && t.len() <= MAX_TEXT) else { continue };
        let targets: Vec<Peer> = sync.peers.lock().unwrap().iter().filter(|p| p.dock && p.online).cloned().collect();
        if targets.is_empty() {
            continue;
        }
        let from = sync.my_name();
        let names = targets.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ");
        sync.set_last(&names, false, &text);
        let _ = app.emit("sync", sync.snapshot());
        for p in targets {
            let msg = Msg::Clip { from: from.clone(), text: text.clone() };
            thread::spawn(move || {
                if let Err(e) = send(&p.ip, &msg) {
                    eprintln!("[sync] send to {} failed: {e}", p.name);
                }
            });
        }
    }
}

pub fn start(app: &AppHandle) -> Arc<Sync> {
    let sync = Arc::new(Sync { enabled: AtomicBool::new(true), ignore_cc: AtomicIsize::new(-1), ..Default::default() });
    let (s, a) = (sync.clone(), app.clone());
    thread::spawn(move || loop {
        refresh(&s, &a);
        thread::sleep(REFRESH_EVERY);
    });
    let (s, a) = (sync.clone(), app.clone());
    thread::spawn(move || serve(s, a));
    let (s, a) = (sync.clone(), app.clone());
    thread::spawn(move || watch_clipboard(s, a));
    sync
}

#[tauri::command]
pub fn sync_status(sync: State<'_, Arc<Sync>>) -> Snapshot {
    sync.snapshot()
}

#[tauri::command]
pub fn sync_set_enabled(sync: State<'_, Arc<Sync>>, enabled: bool) -> Snapshot {
    sync.enabled.store(enabled, Ordering::SeqCst);
    sync.snapshot()
}

/// Send files to one of my devices running Ledge; progress arrives as `transfer` events.
/// Sends this dock's settings to every online dock; returns the names that took them.
#[tauri::command]
pub fn settings_push(sync: State<'_, Arc<Sync>>, json: String) -> Result<Vec<String>, String> {
    if json.len() > MAX_TEXT || !serde_json::from_str::<serde_json::Value>(&json).is_ok_and(|v| v.is_object()) {
        return Err("settings aren't a JSON object".into());
    }
    let targets: Vec<Peer> = sync.peers.lock().unwrap().iter().filter(|p| p.dock && p.online).cloned().collect();
    if targets.is_empty() {
        return Err("No other device with Ledge is online".into());
    }
    let msg = Msg::Settings { from: sync.my_name(), json };
    let took = thread::scope(|sc| {
        let jobs: Vec<_> = targets.iter().map(|p| (p, sc.spawn(|| send(&p.ip, &msg)))).collect();
        jobs.into_iter()
            .filter_map(|(p, j)| match j.join() {
                Ok(Ok(Some(Msg::Ok))) => Some(p.name.clone()),
                other => {
                    eprintln!("[sync] settings to {} failed: {other:?}", p.name);
                    None
                }
            })
            .collect::<Vec<_>>()
    });
    if took.is_empty() { Err("No device took the settings (is Ledge up to date there?)".into()) } else { Ok(took) }
}

#[tauri::command]
pub fn files_send(app: AppHandle, sync: State<'_, Arc<Sync>>, ip: String, paths: Vec<String>) -> Result<(), String> {
    let peer = sync
        .peers
        .lock()
        .unwrap()
        .iter()
        .find(|p| p.ip == ip && p.online && p.dock)
        .map(|p| p.name.clone())
        .ok_or("That device is offline or not running Ledge")?;
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    // shortcut: plain files only; folders need zipping first — add recursive send if it's missed
    if let Some(dir) = paths.iter().find(|p| !p.is_file()) {
        return Err(format!("{} is a folder or missing — zip folders first", dir.display()));
    }
    let from = sync.my_name();
    thread::spawn(move || paths.iter().for_each(|p| send_file(&app, &ip, &peer, &from, p)));
    Ok(())
}

/// Download the newer Ledge from `ip`, verify it, then quit so the swap script can relaunch us.
#[tauri::command(async)]
pub fn update_from(app: AppHandle, sync: State<'_, Arc<Sync>>, ip: String) -> Result<(), String> {
    let offer = sync.snapshot().update.filter(|p| p.ip == ip).ok_or("No newer Ledge on that device")?;
    let ver = offer.version.unwrap_or_default();
    let s = open(&ip, &Msg::GetUpdate { from: sync.my_name() }, FILE_IO_TIMEOUT).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(s);
    let mut head = String::new();
    (&mut reader).take(64 * 1024).read_line(&mut head).map_err(|e| e.to_string())?;
    let size = match serde_json::from_str::<Msg>(&head) {
        Ok(Msg::Update { ver: v, size }) if v == ver && size <= MAX_UPDATE => size,
        Ok(Msg::Err { msg }) => return Err(format!("{}: {msg}", offer.name)),
        _ => return Err(format!("{} sent an unexpected update", offer.name)),
    };
    let dir = std::env::temp_dir();
    let zip = files::receive_into(&dir, &format!("Ledge-{ver}.zip"), size, &mut reader, |_| {}).map_err(|e| e.to_string())?;
    let installed = update::install(&zip, &ver).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&zip);
    installed?;
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = r#"{
      "Self": {"HostName": "mac-a", "UserID": 7, "TailscaleIPs": ["100.1.1.1", "fd7a::1"]},
      "Peer": {
        "k1": {"HostName": "win-b", "UserID": 7, "OS": "windows", "Online": true, "TailscaleIPs": ["100.1.1.2"]},
        "k2": {"HostName": "mac-c", "UserID": 7, "OS": "macOS", "Online": false, "TailscaleIPs": ["100.1.1.3"]},
        "k3": {"HostName": "friend", "UserID": 99, "OS": "linux", "Online": true, "TailscaleIPs": ["100.1.1.9"]}
      }
    }"#;

    #[test]
    fn parses_self_and_keeps_only_my_devices() {
        let (me, peers) = parse_status(STATUS).unwrap();
        assert_eq!(me, Me { name: "mac-a".into(), ip: "100.1.1.1".into(), user: 7 });
        let names: Vec<_> = peers.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["win-b", "mac-c"]); // online first, foreign user dropped
    }

    #[test]
    fn allowlist_accepts_self_and_my_peers_only() {
        let (me, peers) = parse_status(STATUS).unwrap();
        let sync = Sync::default();
        *sync.me.lock().unwrap() = Some(me);
        *sync.peers.lock().unwrap() = peers;
        assert!(sync.is_allowed(&"100.1.1.1".parse().unwrap()));
        assert!(sync.is_allowed(&"100.1.1.2".parse().unwrap()));
        assert!(!sync.is_allowed(&"100.1.1.9".parse().unwrap()));
        assert!(!sync.is_allowed(&"192.168.1.5".parse().unwrap()));
    }

    #[test]
    fn bad_json_is_none() {
        assert!(parse_status("not json").is_none());
        assert!(parse_status("{}").is_none());
    }

    #[test]
    fn message_wire_format_is_tagged() {
        let s = serde_json::to_string(&Msg::Clip { from: "a".into(), text: "hi".into() }).unwrap();
        assert_eq!(s, r#"{"t":"clip","from":"a","text":"hi"}"#);
    }

    #[test]
    fn old_pong_without_version_still_parses() {
        let m: Msg = serde_json::from_str(r#"{"t":"pong","from":"old-mac"}"#).unwrap();
        assert!(matches!(m, Msg::Pong { ver, .. } if ver.is_empty()));
    }

    #[test]
    #[ignore] // needs the Tailscale app connected: cargo test -- --ignored
    fn app_cli_answers_without_a_terminal() {
        let mut cmd = super::status_cmd("/Applications/Tailscale.app/Contents/MacOS/Tailscale");
        cmd.env_remove("TERM"); // what a Finder-launched Ledge sees
        let out = cmd.output().expect("Tailscale app installed");
        assert!(super::parse_status(&String::from_utf8_lossy(&out.stdout)).is_some(), "{}", String::from_utf8_lossy(&out.stderr));
    }
}
