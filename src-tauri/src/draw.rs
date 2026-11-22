//! Pen: a transparent full-screen window to draw on, under the dock and over everything else.
//! Switching it off closes the window, which wipes the drawing.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "draw";

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct DrawStyle {
    pub color: String,
    pub width: f64,
    pub highlighter: bool,
}

static STYLE: Mutex<Option<DrawStyle>> = Mutex::new(None);

fn valid(s: &DrawStyle) -> bool {
    let hex = s.color.strip_prefix('#').unwrap_or("");
    hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) && (1.0..=48.0).contains(&s.width)
}

/// The overlay page asks for the style it should start with.
#[tauri::command]
pub fn draw_style() -> Option<DrawStyle> {
    STYLE.lock().unwrap().clone()
}

/// Turns the pen on/off; a new style while on applies live. Returns whether the pen is on.
/// Async on purpose: Tauri runs sync commands on the main thread, where building a window deadlocks on Windows.
#[tauri::command]
pub async fn draw_set(app: AppHandle, on: bool, style: DrawStyle) -> Result<bool, String> {
    if !valid(&style) {
        return Err("bad pen style".into());
    }
    *STYLE.lock().unwrap() = Some(style.clone());
    let existing = app.get_webview_window(LABEL);
    crate::diag::log(&format!("pen on={on} open={}", existing.is_some()));
    match (on, existing) {
        (false, Some(w)) => {
            if let Some(main) = app.get_webview_window("main") {
                imp::release_dock(&main); // the dock must not go down with the ink window it is tied to
            }
            w.close().map_err(|e| e.to_string())?
        }
        (false, None) => {}
        (true, Some(_)) => app.emit_to(LABEL, "draw-style", style).map_err(|e| e.to_string())?,
        (true, None) => open(&app)?,
    }
    Ok(on)
}

/// Covers the screen the dock is on.
fn open(app: &AppHandle) -> Result<(), String> {
    let main = app.get_webview_window("main").ok_or("main window missing")?;
    let monitor = main.current_monitor().map_err(|e| e.to_string())?.ok_or("no screen")?;
    let scale = monitor.scale_factor();
    let pos = monitor.position().to_logical::<f64>(scale);
    let size = monitor.size().to_logical::<f64>(scale);
    let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("draw.html".into()))
        .title("Ledge Draw")
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .accept_first_mouse(true)
        .focused(false)
        .visible(false) // shown by `below_dock` once its stacking is set, so it never lands above the dock
        .position(pos.x, pos.y)
        .inner_size(size.width, size.height)
        .build()
        .map_err(|e| e.to_string())?;
    imp::below_dock(&w, &main);
    Ok(())
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    // The dock is always-on-top (NSFloatingWindowLevel = 3); one below keeps it clickable
    // while the ink still covers every normal app window.
    const BELOW_DOCK: isize = 2;
    // canJoinAllSpaces | fullScreenAuxiliary: also over full-screen apps (slides, browsers)
    const ALL_SPACES: usize = (1 << 0) | (1 << 8);

    pub fn below_dock(w: &tauri::WebviewWindow, _dock: &tauri::WebviewWindow) {
        let Ok(ns) = w.ns_window() else { return };
        let ns = ns as usize;
        let shown = w.clone();
        // draw_set is async (off the main thread); AppKit traps on window changes from any other thread
        let _ = w.run_on_main_thread(move || {
            let ns = ns as *mut AnyObject;
            unsafe {
                let _: () = msg_send![ns, setLevel: BELOW_DOCK];
                let _: () = msg_send![ns, setCollectionBehavior: ALL_SPACES];
            }
            let _ = shown.show();
        });
    }

    pub fn release_dock(_dock: &tauri::WebviewWindow) {}
}

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWLP_HWNDPARENT, GWL_EXSTYLE, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_SHOWWINDOW, WS_EX_NOACTIVATE,
    };

    fn hwnd(w: &tauri::WebviewWindow) -> Option<HWND> {
        w.hwnd().ok().map(|h| HWND(h.0))
    }

    // Windows lifts a topmost window to the front whenever it is activated or WebView2 finishes with it,
    // so the ink layer kept burying the dock (and its off switch). An owned window always stays above
    // its owner, so while the pen is on the ink owns the dock; the ink also never takes activation.
    pub fn below_dock(w: &tauri::WebviewWindow, dock: &tauri::WebviewWindow) {
        let _ = w.set_always_on_top(true);
        let (Some(ink), Some(dock)) = (hwnd(w), hwnd(dock)) else { return };
        unsafe {
            let ex = GetWindowLongPtrW(ink, GWL_EXSTYLE);
            SetWindowLongPtrW(ink, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE.0 as isize);
            let keep = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
            let _ = SetWindowPos(ink, Some(HWND_TOPMOST), 0, 0, 0, 0, keep | SWP_SHOWWINDOW);
            SetWindowLongPtrW(dock, GWLP_HWNDPARENT, ink.0 as isize);
            let _ = SetWindowPos(dock, Some(HWND_TOPMOST), 0, 0, 0, 0, keep);
        }
    }

    pub fn release_dock(dock: &tauri::WebviewWindow) {
        if let Some(dock) = hwnd(dock) {
            unsafe { SetWindowLongPtrW(dock, GWLP_HWNDPARENT, 0) };
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    pub fn below_dock(w: &tauri::WebviewWindow, _dock: &tauri::WebviewWindow) {
        let _ = w.set_always_on_top(true);
        let _ = w.show();
    }

    pub fn release_dock(_dock: &tauri::WebviewWindow) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_is_checked_before_use() {
        let ok = DrawStyle { color: "#ff3b30".into(), width: 4.0, highlighter: false };
        assert!(valid(&ok));
        assert!(!valid(&DrawStyle { color: "red".into(), ..ok.clone() }));
        assert!(!valid(&DrawStyle { color: "#ff3b3".into(), ..ok.clone() }));
        assert!(!valid(&DrawStyle { width: 0.0, ..ok.clone() }));
        assert!(!valid(&DrawStyle { width: 500.0, ..ok }));
    }
}
