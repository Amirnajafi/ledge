//! Camera on/off in a Google Meet call running in Chrome.
//! macOS gives no app a way to switch the webcam off for other apps, so we click Meet's own camera
//! button with a small script run in the call tab (Chrome: View → Developer → Allow JavaScript from
//! Apple Events). No keystrokes, so extensions that also use ⌘E can't steal it, and no tab switching.
//! Windows: the same button is pressed through UI Automation (the accessibility API).

/// A Meet *call* URL: meet.google.com/abc-defg-hij (not the Meet landing page).
pub fn is_call_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://meet.google.com/") else { return false };
    let code = rest.split(['?', '#', '/']).next().unwrap_or("");
    let parts: Vec<&str> = code.split('-').collect();
    parts.len() == 3
        && [3, 4, 3].iter().zip(&parts).all(|(n, p)| p.len() == *n && p.chars().all(|c| c.is_ascii_lowercase()))
}

/// Async so the lookup (it can retry for a second) never blocks the dock's UI thread.
#[tauri::command]
pub async fn meet_camera() -> Result<String, String> {
    imp::toggle()
}

/// Chrome/Edge window titles follow the active tab: a Meet call reads "Meet - abc-defg-hij - Google Chrome".
pub fn is_meet_title(title: &str) -> bool {
    title.starts_with("Meet - ") || title.starts_with("Meet – ")
}

/// Meet's camera button is named "Turn off camera (ctrl + e)" / "Turn on camera …".
/// Some(true) = the camera is on now (pressing turns it off), Some(false) = it is off, None = another button.
pub fn camera_button(name: &str) -> Option<bool> {
    let n = name.to_lowercase();
    if n.starts_with("turn off camera") {
        Some(true)
    } else if n.starts_with("turn on camera") {
        Some(false)
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::process::Command;

    // Guarded by `is running` so a click never launches Chrome. Inside `tell Chrome`, `tab` means
    // Chrome's tab class (it prints as the word "tab"), so the separator is spelled `character id 9`.
    pub(super) const LIST_TABS: &str = r#"
if application "Google Chrome" is not running then return ""
set sep to character id 9
tell application "Google Chrome"
  set out to ""
  repeat with w from 1 to count windows
    set ws to window w
    repeat with t from 1 to count tabs of ws
      set u to URL of tab t of ws
      if u starts with "https://meet.google.com/" then set out to out & w & sep & t & sep & u & linefeed
    end repeat
  end repeat
  return out
end tell"#;

    const RUN_IN_TAB: &str = r#"on run argv
tell application "Google Chrome" to return execute tab ((item 2 of argv) as integer) of window ((item 1 of argv) as integer) javascript (item 3 of argv)
end run"#;

    /// Meet marks its mic and camera buttons with `data-is-muted`; the camera one says "camera" in its label.
    pub(super) const TOGGLE_JS: &str = r#"(() => {
  const b = [...document.querySelectorAll('[data-is-muted]')].find((e) => /camera|video/i.test(e.getAttribute('aria-label') || ''));
  if (!b) return 'no-button';
  const wasOff = b.getAttribute('data-is-muted') === 'true';
  b.click();
  return wasOff ? 'on' : 'off';
})()"#;

    fn osascript(script: &str, args: &[&str]) -> Result<String, String> {
        let out = Command::new("osascript").arg("-e").arg(script).args(args).output().map_err(|e| e.to_string())?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        Err(if err.contains("-1743") {
            "Allow Ledge to control Google Chrome: System Settings → Privacy & Security → Automation".into()
        } else if err.contains("JavaScript") {
            "In Chrome, turn on View → Developer → Allow JavaScript from Apple Events (one time)".into()
        } else {
            format!("Chrome didn't respond: {}", err.trim())
        })
    }

    pub fn toggle() -> Result<String, String> {
        let tabs = osascript(LIST_TABS, &[])?;
        let call = tabs
            .lines()
            .map(|l| l.split('\t').collect::<Vec<_>>())
            .find(|f| f.len() == 3 && super::is_call_url(f[2]))
            .ok_or("No Google Meet call open in Chrome")?;
        match osascript(RUN_IN_TAB, &[call[0], call[1], TOGGLE_JS])?.as_str() {
            "on" => Ok("Camera on in Google Meet".into()),
            "off" => Ok("Camera off in Google Meet".into()),
            _ => Err("Couldn't find Meet's camera button. Are you in the call (not the waiting room)?".into()),
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::{camera_button, is_meet_title};
    use std::time::Duration;
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationInvokePattern, TreeScope_Descendants, UIA_ButtonControlTypeId,
        UIA_ControlTypePropertyId, UIA_InvokePatternId,
    };
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetClassNameW, GetWindowTextW, IsWindowVisible};

    /// Chrome only builds a page's accessibility tree once someone asks, so the first look can come back empty.
    const TRIES: usize = 4;
    const RETRY: Duration = Duration::from_millis(350);

    unsafe extern "system" fn collect(h: HWND, out: LPARAM) -> BOOL {
        let out = unsafe { &mut *(out.0 as *mut Vec<HWND>) };
        let (mut class, mut title) = ([0u16; 64], [0u16; 512]);
        let (nc, nt) = unsafe { (GetClassNameW(h, &mut class).max(0) as usize, GetWindowTextW(h, &mut title).max(0) as usize) };
        let (class, title) = (String::from_utf16_lossy(&class[..nc]), String::from_utf16_lossy(&title[..nt]));
        if unsafe { IsWindowVisible(h) }.as_bool() && class == "Chrome_WidgetWin_1" && is_meet_title(&title) {
            out.push(h);
        }
        BOOL(1)
    }

    fn meet_windows() -> Vec<HWND> {
        let mut found: Vec<HWND> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(collect), LPARAM(&mut found as *mut Vec<HWND> as isize));
        }
        found
    }

    fn err(e: windows::core::Error) -> String {
        e.message().to_string()
    }

    pub fn toggle() -> Result<String, String> {
        let windows = meet_windows();
        if windows.is_empty() {
            return Err("No Google Meet call found. Is the call the open tab in its Chrome window?".into());
        }
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).map_err(err)?;
            let buttons = uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_ButtonControlTypeId.0)).map_err(err)?;
            for attempt in 0..TRIES {
                if attempt > 0 {
                    std::thread::sleep(RETRY);
                }
                for &w in &windows {
                    let Ok(root) = uia.ElementFromHandle(w) else { continue };
                    let Ok(all) = root.FindAll(TreeScope_Descendants, &buttons) else { continue };
                    for i in 0..all.Length().unwrap_or(0) {
                        let Ok(b) = all.GetElement(i) else { continue };
                        let Some(was_on) = b.CurrentName().ok().and_then(|n| camera_button(&n.to_string())) else { continue };
                        let press: IUIAutomationInvokePattern = b.GetCurrentPatternAs(UIA_InvokePatternId).map_err(err)?;
                        press.Invoke().map_err(err)?;
                        return Ok(if was_on { "Camera off in Google Meet" } else { "Camera on in Google Meet" }.into());
                    }
                }
            }
        }
        Err("Couldn't find Meet's camera button. Are you in the call (not the waiting room)?".into())
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    pub fn toggle() -> Result<String, String> {
        Err("not implemented on this OS yet".into())
    }
}

#[cfg(test)]
mod tests {
    use super::is_call_url;

    #[cfg(target_os = "macos")]
    #[test]
    fn tab_list_uses_a_real_tab_separator() {
        let out = std::process::Command::new("osascript")
            .args(["-e", "set sep to character id 9", "-e", r#"tell application "Google Chrome" to return "1" & sep & "2""#])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), "1\t2");
        assert!(!super::imp::LIST_TABS.contains("& tab &"), "bare `tab` inside tell Chrome is the tab class");
    }

    #[test]
    fn windows_finds_the_call_window_and_camera_button() {
        use super::{camera_button, is_meet_title};
        assert!(is_meet_title("Meet - abc-defg-hij - Google Chrome"));
        assert!(is_meet_title("Meet – Weekly sync - Google Chrome"));
        assert!(!is_meet_title("Google Meet - Google Chrome")); // landing page
        assert!(!is_meet_title("Inbox - Gmail - Google Chrome"));
        assert_eq!(camera_button("Turn off camera (ctrl + e)"), Some(true));
        assert_eq!(camera_button("Turn on camera (ctrl + e)"), Some(false));
        assert_eq!(camera_button("Video settings"), None);
        assert_eq!(camera_button("Turn off microphone (ctrl + d)"), None);
    }

    #[test]
    fn only_real_meet_calls_match() {
        assert!(is_call_url("https://meet.google.com/abc-defg-hij"));
        assert!(is_call_url("https://meet.google.com/abc-defg-hij?authuser=1"));
        assert!(!is_call_url("https://meet.google.com/"));
        assert!(!is_call_url("https://meet.google.com/landing"));
        assert!(!is_call_url("https://meet.google.com/new"));
        assert!(!is_call_url("https://evil.com/?https://meet.google.com/abc-defg-hij"));
        assert!(!is_call_url("https://meet.google.com/ABC-defg-hij"));
    }
}
