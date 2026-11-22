//! Magic Mouse taps → clicks. macOS has no tap-to-click for the Magic Mouse, so we read its touch
//! surface through the private MultitouchSupport framework and post clicks (needs Accessibility).

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct MouseSettings {
    /// One-finger tap = left click (two quick taps = double-click).
    pub tap: bool,
    /// Two-finger tap = right-click.
    pub two_finger_right: bool,
    /// One-finger tap on the right half = right-click (wins over `tap` there).
    pub right_side_right: bool,
    /// Three-finger tap = middle click.
    pub three_finger_middle: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Click {
    Left,
    Right,
    Middle,
}

const TAP_MAX_SECS: f64 = 0.22;
const TAP_MAX_MOVE: f32 = 0.05; // normalized surface units (0..1)
const GROUP_SECS: f64 = 0.08; // fingers lifting this close together count as one multi-finger tap
const RIGHT_SIDE: f32 = 0.55;

/// One finger's touch, from landing to lifting.
#[derive(Clone, Copy, Debug)]
pub struct Contact {
    pub secs: f64,
    pub moved: f32,
    pub x: f32,
    /// A physical button went down while it touched: that was a real click, not a tap.
    pub pressed: bool,
}

pub fn is_tap(c: &Contact) -> bool {
    !c.pressed && c.secs <= TAP_MAX_SECS && c.moved <= TAP_MAX_MOVE
}

/// What a group of tapping fingers should click, if anything.
pub fn classify(taps: &[Contact], s: &MouseSettings) -> Option<Click> {
    match taps {
        [one] if s.right_side_right && one.x > RIGHT_SIDE => Some(Click::Right),
        [_] if s.tap => Some(Click::Left),
        [_, _] if s.two_finger_right => Some(Click::Right),
        [_, _, _] if s.three_finger_middle => Some(Click::Middle),
        _ => None,
    }
}

#[tauri::command]
pub fn mouse_configure(settings: MouseSettings) -> usize {
    imp::configure(settings);
    imp::connected()
}

#[tauri::command]
pub fn mouse_status() -> usize {
    imp::connected()
}

pub fn start() {
    imp::start();
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use crate::multitouch::{self, Dev, Finger};
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> *mut c_void;
        fn CGEventGetLocation(e: *mut c_void) -> Point;
        fn CGEventCreateMouseEvent(source: *const c_void, kind: u32, at: Point, button: u32) -> *mut c_void;
        fn CGEventSetIntegerValueField(e: *mut c_void, field: u32, value: i64);
        fn CGEventPost(tap: u32, e: *mut c_void);
        fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
        fn CFRelease(p: *const c_void);
    }

    const HID_STATE: i32 = 1; // kCGEventSourceStateHIDSystemState
    const CLICK_STATE: u32 = 1; // kCGMouseEventClickState

    struct Live {
        start: f64,
        x0: f32,
        y0: f32,
        x: f32,
        moved: f32,
        pressed: bool,
    }

    #[derive(Default)]
    struct Tracker {
        live: HashMap<i32, Live>,
        pending: Vec<(f64, super::Contact)>, // (lifted at, contact)
        last_click: Option<(Instant, Click, i64)>,
    }

    static SETTINGS: Mutex<MouseSettings> =
        Mutex::new(MouseSettings { tap: false, two_finger_right: false, right_side_right: false, three_finger_middle: false });
    static TRACKER: OnceLock<Mutex<Tracker>> = OnceLock::new();
    static CONNECTED: AtomicUsize = AtomicUsize::new(0);

    pub fn configure(s: MouseSettings) {
        *SETTINGS.lock().unwrap() = s;
    }

    pub fn connected() -> usize {
        CONNECTED.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn start() {
        multitouch::watch("magic mouse", multitouch::is_magic_mouse, on_frame, &CONNECTED);
    }

    fn button_down() -> bool {
        unsafe { CGEventSourceButtonState(HID_STATE, 0) || CGEventSourceButtonState(HID_STATE, 1) }
    }

    extern "C" fn on_frame(_dev: Dev, fingers: *const Finger, n: i32, now: f64, _frame: i32) -> i32 {
        let settings = *SETTINGS.lock().unwrap();
        if settings == MouseSettings::default() {
            return 0;
        }
        let fingers = unsafe { multitouch::fingers(fingers, n) };
        let tracker = TRACKER.get_or_init(Default::default);
        let mut t = tracker.lock().unwrap();
        let pressed = button_down();

        let touching: Vec<&Finger> = fingers.iter().filter(|f| matches!(f.state, 3..=5)).collect();
        for f in &touching {
            let live = t.live.entry(f.identifier).or_insert(Live { start: now, x0: f.pos.x, y0: f.pos.y, x: f.pos.x, moved: 0.0, pressed: false });
            live.x = f.pos.x;
            live.moved = live.moved.max(((f.pos.x - live.x0).powi(2) + (f.pos.y - live.y0).powi(2)).sqrt());
            live.pressed |= pressed;
        }
        if pressed {
            t.pending.clear(); // a real click is happening; nothing lifted around it is a tap
        }
        let ended: Vec<i32> = t.live.keys().copied().filter(|id| !touching.iter().any(|f| f.identifier == *id)).collect();
        for id in ended {
            let Some(l) = t.live.remove(&id) else { continue };
            let c = super::Contact { secs: now - l.start, moved: l.moved, x: l.x, pressed: l.pressed };
            if super::is_tap(&c) {
                t.pending.push((now, c));
            }
        }
        // Flush once no other short touch could still join this tap, or the group window passed.
        let joinable = t.live.values().any(|l| now - l.start <= TAP_MAX_SECS && !l.pressed);
        if let Some(&(first, _)) = t.pending.first() {
            if !joinable || now - first > GROUP_SECS {
                let taps: Vec<super::Contact> = t.pending.drain(..).map(|(_, c)| c).collect();
                if let Some(click) = super::classify(&taps, &settings) {
                    post(&mut t, click);
                }
            }
        }
        0
    }

    fn double_click_secs() -> f64 {
        use objc2::msg_send;
        use objc2::runtime::AnyClass;
        AnyClass::get(c"NSEvent").map(|c| unsafe { msg_send![c, doubleClickInterval] }).unwrap_or(0.5)
    }

    fn post(t: &mut Tracker, click: Click) {
        let (down, up, button) = match click {
            Click::Left => (1, 2, 0),     // kCGEventLeftMouseDown/Up, kCGMouseButtonLeft
            Click::Right => (3, 4, 1),    // kCGEventRightMouseDown/Up
            Click::Middle => (25, 26, 2), // kCGEventOtherMouseDown/Up, center button
        };
        let now = Instant::now();
        let count = match t.last_click {
            Some((at, c, n)) if c == click && now.duration_since(at).as_secs_f64() < double_click_secs() => (n % 3) + 1,
            _ => 1,
        };
        t.last_click = Some((now, click, count));
        unsafe {
            let probe = CGEventCreate(std::ptr::null());
            if probe.is_null() {
                return;
            }
            let at = CGEventGetLocation(probe);
            CFRelease(probe);
            for kind in [down, up] {
                let e = CGEventCreateMouseEvent(std::ptr::null(), kind, at, button);
                if e.is_null() {
                    continue;
                }
                CGEventSetIntegerValueField(e, CLICK_STATE, count);
                CGEventPost(0, e); // kCGHIDEventTap
                CFRelease(e);
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::MouseSettings;
    // shortcut: Magic Mouse taps are macOS-only (Windows needs a Magic Mouse driver anyway)
    pub fn configure(_s: MouseSettings) {}
    pub fn connected() -> usize {
        0
    }
    pub fn start() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap(x: f32) -> Contact {
        Contact { secs: 0.1, moved: 0.01, x, pressed: false }
    }
    const ALL: MouseSettings = MouseSettings { tap: true, two_finger_right: true, right_side_right: false, three_finger_middle: true };

    #[test]
    fn taps_map_to_clicks() {
        assert_eq!(classify(&[tap(0.3)], &ALL), Some(Click::Left));
        assert_eq!(classify(&[tap(0.3), tap(0.6)], &ALL), Some(Click::Right));
        assert_eq!(classify(&[tap(0.2), tap(0.5), tap(0.8)], &ALL), Some(Click::Middle));
        assert_eq!(classify(&[tap(0.9)], &MouseSettings { right_side_right: true, ..ALL }), Some(Click::Right));
        assert_eq!(classify(&[tap(0.3)], &MouseSettings { right_side_right: true, ..ALL }), Some(Click::Left));
        assert_eq!(classify(&[tap(0.3)], &MouseSettings::default()), None);
        assert_eq!(classify(&[tap(0.1); 4], &ALL), None);
    }

    #[test]
    fn rests_scrolls_and_real_clicks_are_not_taps() {
        assert!(is_tap(&tap(0.5)));
        assert!(!is_tap(&Contact { secs: 0.6, ..tap(0.5) })); // resting finger
        assert!(!is_tap(&Contact { moved: 0.2, ..tap(0.5) })); // scroll flick
        assert!(!is_tap(&Contact { pressed: true, ..tap(0.5) })); // physical click
    }
}
