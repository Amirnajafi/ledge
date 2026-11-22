//! What Ledge needs from the OS, whether it has it, and a shortcut to the right System Settings pane.

use serde::Serialize;
use tauri::AppHandle;

#[derive(Serialize, Clone, Debug)]
pub struct Permission {
    pub id: &'static str,
    pub name: &'static str,
    pub why: &'static str,
    /// "granted" | "denied" | "not-asked" | "unknown"
    pub status: &'static str,
}

#[tauri::command]
pub fn permissions(app: AppHandle) -> Vec<Permission> {
    imp::list(&app)
}

/// Opens the settings pane for `id` (fixed table — never an arbitrary URL). Registers Ledge
/// in the list first where macOS needs that (Accessibility only shows apps that have asked).
/// A camera that was never asked gets macOS's own prompt (the web view can't trigger it: it just fails
/// with "Invalid constraint"); answers arrive as a "permissions" event.
#[tauri::command]
pub fn permission_open(app: AppHandle, id: String) -> Result<(), String> {
    let url = imp::pane(&id).ok_or("unknown permission")?;
    if imp::prompt(&app, &id) {
        return Ok(());
    }
    imp::register(&id);
    open_url(url)
}

fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(url).status();
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("cmd").args(["/C", "start", "", url]).status();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let status = std::process::Command::new("xdg-open").arg(url).status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("could not open settings ({s})")),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Permission;
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSString;
    use tauri::{AppHandle, Manager};

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightPostEventAccess() -> bool;
        fn CGRequestPostEventAccess() -> bool;
    }

    #[repr(C)]
    struct AEDesc {
        descriptor_type: u32,
        data_handle: *mut std::ffi::c_void,
    }

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn AECreateDesc(type_code: u32, data: *const std::ffi::c_void, size: isize, result: *mut AEDesc) -> i16;
        fn AEDisposeDesc(desc: *mut AEDesc) -> i16;
        fn AEDeterminePermissionToAutomateTarget(target: *const AEDesc, class: u32, id: u32, ask: u8) -> i32;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }

    pub fn pane(id: &str) -> Option<&'static str> {
        Some(match id {
            "camera" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Camera",
            "accessibility" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            "downloads" => "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders",
            "automation" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation",
            _ => return None,
        })
    }

    /// Shows the system prompt where macOS still can. false = nothing to prompt, open Settings instead.
    pub fn prompt(app: &AppHandle, id: &str) -> bool {
        use tauri::Emitter;
        if id != "camera" || camera_status() != "not-asked" {
            return false;
        }
        let Some(cls) = AnyClass::get(c"AVCaptureDevice") else { return false };
        let video = NSString::from_str("vide"); // AVMediaTypeVideo
        let app = app.clone();
        let done = block2::RcBlock::new(move |_granted: objc2::runtime::Bool| {
            let _ = app.emit("permissions", ());
        });
        let _: () = unsafe { msg_send![cls, requestAccessForMediaType: &*video, completionHandler: &*done] };
        true
    }

    pub fn register(id: &str) {
        if id == "accessibility" && accessibility_status() != "granted" {
            unsafe { CGRequestPostEventAccess() };
        }
    }

    pub fn camera_status() -> &'static str {
        let Some(cls) = AnyClass::get(c"AVCaptureDevice") else { return "unknown" };
        let video = NSString::from_str("vide"); // AVMediaTypeVideo
        let status: isize = unsafe { msg_send![cls, authorizationStatusForMediaType: &*video as *const NSString as *const AnyObject] };
        match status {
            0 => "not-asked",
            3 => "granted",
            1 | 2 => "denied",
            _ => "unknown",
        }
    }

    pub fn accessibility_status() -> &'static str {
        // AXIsProcessTrusted reflects a toggle flip live; the CG preflight can lag until relaunch
        if unsafe { AXIsProcessTrusted() || CGPreflightPostEventAccess() } { "granted" } else { "denied" }
    }

    /// Chrome must be running for macOS to answer; otherwise "unknown".
    fn automation_status() -> &'static str {
        const TYPE_BUNDLE_ID: u32 = u32::from_be_bytes(*b"bund");
        const ANY: u32 = u32::from_be_bytes(*b"****");
        let id = b"com.google.Chrome";
        let mut desc = AEDesc { descriptor_type: 0, data_handle: std::ptr::null_mut() };
        if unsafe { AECreateDesc(TYPE_BUNDLE_ID, id.as_ptr().cast(), id.len() as isize, &mut desc) } != 0 {
            return "unknown";
        }
        let status = unsafe { AEDeterminePermissionToAutomateTarget(&desc, ANY, ANY, 0) };
        unsafe { AEDisposeDesc(&mut desc) };
        match status {
            0 => "granted",
            -1743 => "denied",
            -1744 => "not-asked",
            _ => "unknown", // -600: Chrome not running
        }
    }

    fn downloads(app: &AppHandle) -> &'static str {
        let Ok(dir) = app.path().download_dir() else { return "unknown" };
        match std::fs::read_dir(dir) {
            Ok(_) => "granted",
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => "denied",
            Err(_) => "unknown",
        }
    }

    pub fn list(app: &AppHandle) -> Vec<Permission> {
        vec![
            Permission { id: "camera", name: "Camera", why: "Live preview in the camera panel", status: camera_status() },
            Permission { id: "accessibility", name: "Accessibility", why: "Play / pause / next media keys", status: accessibility_status() },
            Permission { id: "downloads", name: "Downloads folder", why: "Save files sent from your devices", status: downloads(app) },
            Permission { id: "automation", name: "Control Chrome", why: "Camera on/off in Google Meet", status: automation_status() },
        ]
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::Permission;
    use tauri::AppHandle;

    pub fn pane(id: &str) -> Option<&'static str> {
        match id {
            "camera" => Some("ms-settings:privacy-webcam"),
            _ => None,
        }
    }

    pub fn register(_id: &str) {}

    pub fn prompt(_app: &AppHandle, _id: &str) -> bool {
        false
    }

    pub fn list(_app: &AppHandle) -> Vec<Permission> {
        // shortcut: Windows only gates the camera; status isn't readable without WinRT, so it shows "unknown"
        vec![Permission { id: "camera", name: "Camera", why: "Live preview in the camera panel", status: "unknown" }]
    }
}

#[cfg(all(test, target_os = "macos"))]
mod live_tests {
    /// Reads real TCC state: `cargo test live_permissions -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_permissions_read_without_crashing() {
        let cam = super::imp::camera_status();
        let acc = super::imp::accessibility_status();
        println!("camera={cam} accessibility={acc}");
        assert!(["granted", "denied", "not-asked", "unknown"].contains(&cam));
        assert!(super::imp::pane("camera").is_some() && super::imp::pane("nope").is_none());
    }
}
