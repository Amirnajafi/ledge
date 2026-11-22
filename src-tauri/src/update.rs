//! Self-update, two ways:
//! - GitHub: at launch and twice a day each machine checks the latest release and installs it.
//! - Tailscale: a device running a newer Ledge serves its own app bundle; older devices download it
//!   through `sync`, verify it, swap it in and relaunch (handy for builds not yet released).

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tauri::AppHandle;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const BUNDLE_ID: &str = "com.mac.ledge";

/// `a` is a newer version than `b` (dotted numbers; anything unparsable is never newer).
pub fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| v.split('.').map(|p| p.parse::<u64>().ok()).collect::<Option<Vec<_>>>();
    matches!((parse(a), parse(b)), (Some(x), Some(y)) if x > y)
}

/// The running `.app` (None in `tauri dev`, which runs a bare binary).
pub fn bundle_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let app = exe.parent()?.parent()?.parent()?.to_path_buf(); // X.app/Contents/MacOS/bin
    (app.extension()? == "app").then_some(app)
}

fn run(cmd: &mut Command) -> io::Result<String> {
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(io::Error::other(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Zip the running bundle for a peer. Caller deletes the returned file.
pub fn pack() -> io::Result<PathBuf> {
    let app = bundle_path().ok_or_else(|| io::Error::other("this Ledge isn't an installed app"))?;
    let zip = std::env::temp_dir().join(format!("ledge-{VERSION}-{}.zip", std::process::id()));
    let _ = std::fs::remove_file(&zip);
    run(Command::new("ditto").args(["-c", "-k", "--keepParent"]).arg(&app).arg(&zip))?;
    Ok(zip)
}

fn plist(app: &Path, key: &str) -> io::Result<String> {
    run(Command::new("plutil").args(["-extract", key, "raw"]).arg(app.join("Contents/Info.plist")))
}

/// Unpack next to the current bundle, check it really is a newer Ledge, then hand off to a
/// detached shell that swaps the bundles after we quit and relaunches.
pub fn install(zip: &Path, expect: &str) -> io::Result<()> {
    let current = bundle_path().ok_or_else(|| io::Error::other("this Ledge isn't an installed app"))?;
    let parent = current.parent().ok_or_else(|| io::Error::other("bad app path"))?;
    let stage = parent.join(format!(".ledge-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage)?;
    let checked = (|| {
        run(Command::new("ditto").args(["-x", "-k"]).arg(zip).arg(&stage))?;
        let new = std::fs::read_dir(&stage)?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .ok_or_else(|| io::Error::other("update has no app inside"))?;
        if plist(&new, "CFBundleIdentifier")? != BUNDLE_ID {
            return Err(io::Error::other("update is not Ledge"));
        }
        let ver = plist(&new, "CFBundleShortVersionString")?;
        if ver != expect || !newer(&ver, VERSION) {
            return Err(io::Error::other(format!("update version {ver} doesn't match {expect}")));
        }
        Ok(new)
    })();
    let new = match checked {
        Ok(n) => n,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(e);
        }
    };
    // Paths go in as $1..$3, never spliced into the script. Old bundle comes back if the swap fails.
    let script = r#"sleep 1
old="$1"; new="$2"; stage="$3"
rm -rf "$old.previous"
if mv "$old" "$old.previous" && mv "$new" "$old"; then rm -rf "$old.previous"; else [ -d "$old" ] || mv "$old.previous" "$old"; fi
rm -rf "$stage"
open "$old""#;
    Command::new("sh").arg("-c").arg(script).arg("ledge-update").arg(&current).arg(&new).arg(&stage).spawn()?;
    Ok(())
}

// ---------- GitHub releases ----------

pub const REPO: &str = "Amirnajafi/ledge";
/// Release assets are named `Ledge-<version><suffix>`; scripts/release.sh uploads them.
pub const ASSET_SUFFIX: &str = if cfg!(target_os = "macos") { "-macos-arm64.zip" } else { "-windows-x64.exe" };
const FIRST_CHECK: Duration = Duration::from_secs(30); // let the network come up after login
const EVERY: Duration = Duration::from_secs(12 * 3600);
const MAX_DOWNLOAD: u64 = 200 * 1024 * 1024;

#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}
#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// curl ships with macOS and Windows 10+, so no HTTP client crate is needed.
fn curl() -> Command {
    let mut c = Command::new(if cfg!(windows) { "curl.exe" } else { "/usr/bin/curl" });
    c.args(["-fsSL", "--proto", "=https", "--max-time", "600"]);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut c, 0x0800_0000); // no console window
    c
}

/// From the releases/latest JSON: (version, download url) of this platform's build when it is newer.
fn pick(json: &str) -> io::Result<Option<(String, String)>> {
    let r: Release = serde_json::from_str(json).map_err(io::Error::other)?;
    let ver = r.tag_name.trim_start_matches('v').to_string();
    if !newer(&ver, VERSION) {
        return Ok(None);
    }
    let want = format!("Ledge-{ver}{ASSET_SUFFIX}");
    let prefix = format!("https://github.com/{REPO}/releases/download/");
    Ok(r.assets
        .into_iter()
        .find(|a| a.name == want && a.size <= MAX_DOWNLOAD && a.browser_download_url.starts_with(&prefix))
        .map(|a| (ver, a.browser_download_url)))
}

/// Download and install a newer release, then quit so the new build takes over. Ok(false) = up to date.
fn update_from_github(app: &AppHandle) -> io::Result<bool> {
    let json = run(curl().arg(format!("https://api.github.com/repos/{REPO}/releases/latest")))?;
    let Some((ver, url)) = pick(&json)? else { return Ok(false) };
    crate::diag::log(&format!("update: downloading {ver} from GitHub"));
    #[cfg(target_os = "macos")]
    {
        let zip = std::env::temp_dir().join(format!("Ledge-{ver}-{}.zip", std::process::id()));
        let got = run(curl().arg("-o").arg(&zip).arg(&url)).and_then(|_| install(&zip, &ver));
        let _ = std::fs::remove_file(&zip);
        got?;
    }
    #[cfg(windows)]
    {
        let exe = std::env::current_exe()?;
        let new = exe.with_extension("exe.new");
        run(curl().arg("-o").arg(&new).arg(&url))?;
        install_exe(&exe, &new)?;
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = (ver, url);
    crate::diag::log(&format!("update: installed {ver}, restarting"));
    app.exit(0);
    Ok(true)
}

/// Windows lets a running exe be renamed: move it aside, put the new one in its place, start it.
#[cfg(windows)]
fn install_exe(exe: &Path, new: &Path) -> io::Result<()> {
    let head = std::fs::read(new).map(|b| b.get(..2) == Some(b"MZ"));
    if !head.unwrap_or(false) {
        let _ = std::fs::remove_file(new);
        return Err(io::Error::other("update is not a Windows program"));
    }
    let old = exe.with_extension("exe.old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old)?;
    if let Err(e) = std::fs::rename(new, exe) {
        let _ = std::fs::rename(&old, exe);
        return Err(e);
    }
    // the new copy waits for this one to exit (it would otherwise find the sync port taken)
    Command::new(exe).env(AFTER_UPDATE, "1").spawn()?;
    Ok(())
}

pub const AFTER_UPDATE: &str = "LEDGE_AFTER_UPDATE";

/// Background loop: check at launch, then every 12 h. Dev builds (`tauri dev`) never update.
pub fn start(app: &AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("exe.old")); // left by the last update
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        loop {
            match update_from_github(&app) {
                Ok(true) => return,
                Ok(false) => {}
                Err(e) => crate::diag::log(&format!("update: GitHub check failed: {e}")),
            }
            std::thread::sleep(EVERY);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{newer, pick, ASSET_SUFFIX, REPO};

    #[test]
    fn github_release_picks_this_platforms_newer_build() {
        let json = |tag: &str, name: &str, host: &str| {
            format!(r#"{{"tag_name":"{tag}","assets":[{{"name":"Ledge-9.0.0-other.zip","size":1,"browser_download_url":"x"}},
              {{"name":"{name}","size":5,"browser_download_url":"https://{host}/{REPO}/releases/download/{tag}/{name}"}}]}}"#)
        };
        let ours = format!("Ledge-9.0.0{ASSET_SUFFIX}");
        let (ver, url) = pick(&json("v9.0.0", &ours, "github.com")).unwrap().unwrap();
        assert_eq!(ver, "9.0.0");
        assert!(url.ends_with(&ours));
        assert_eq!(pick(&json("v0.0.1", "Ledge-0.0.1-x", "github.com")).unwrap(), None); // older
        assert_eq!(pick(&json("v9.0.0", &ours, "evil.example")).unwrap(), None); // not this repo's downloads
        assert!(pick("not json").is_err());
    }

    #[test]
    fn version_order() {
        assert!(newer("0.1.1", "0.1.0"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("1.0.0", "0.99.99"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.1.1"));
        assert!(!newer("", "0.1.0"));
        assert!(!newer("0.2.x", "0.1.0"));
    }
}
