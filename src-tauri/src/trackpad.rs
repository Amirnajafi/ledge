//! Trackpad edges and corners. Slide along an edge to drive volume, brightness or track skips;
//! double-tap a corner to fire an action (the dock maps corners to its shortcut actions).
//! Gesture model adapted from SlideSense (MIT, github.com/shashvat1965/slidesense), extended to
//! four configurable edges + four corners, and only claiming a slide that runs *along* its edge.

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Slide {
    #[default]
    Off,
    Volume,
    Brightness,
    Tracks,
}

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct TrackpadSettings {
    pub left: Slide,
    pub right: Slide,
    pub top: Slide,
    pub bottom: Slide,
    /// Corners with a double-tap action: top-left, top-right, bottom-left, bottom-right.
    pub corners: [bool; 4],
}

impl TrackpadSettings {
    fn any(&self) -> bool {
        self.corners.iter().any(|c| *c) || [self.left, self.right, self.top, self.bottom].iter().any(|s| *s != Slide::Off)
    }
    fn slide(&self, side: Side) -> Slide {
        match side {
            Side::Left => self.left,
            Side::Right => self.right,
            Side::Top => self.top,
            Side::Bottom => self.bottom,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    fn vertical(self) -> bool {
        matches!(self, Side::Left | Side::Right)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// A slide took over the finger: freeze the cursor.
    Begin,
    Level(Slide, f64),
    /// The level crossed a 1/16 step: haptic tick.
    Tick,
    Track { next: bool },
    End,
    Corner(usize),
}

const BAND: f64 = 0.15;
const CORNER: f64 = 0.16;
const CLAIM: f64 = 0.012; // travel before deciding "slide along the edge" vs "normal cursor move"
const SPAN: f64 = 0.75; // travel for a full 0 → 1 sweep
const FINE_STEPS: f64 = 64.0;
const TICK_STEPS: f64 = 16.0;
const TRACK_SWIPE: f64 = 0.12;
const TAP_SECS: f64 = 0.4;
const TAP_TRAVEL: f64 = 0.03;
const DOUBLE_TAP_SECS: f64 = 0.45;

#[derive(Clone, Copy, Debug)]
enum Kind {
    Slide { side: Side, action: Slide, start: f64 },
    Corner(usize),
    Ignored,
}

#[derive(Clone, Copy, Debug)]
struct Gesture {
    kind: Kind,
    x0: f64,
    y0: f64,
    t0: f64,
    x: f64,
    y: f64,
    seen: f64,
    moved: bool,
    level: f64,
    step: i64,
    consumed: bool,
    lost: u32,
}

#[derive(Default)]
pub struct Engine {
    gesture: Option<Gesture>,
    last_corner_tap: Option<(usize, f64)>,
}

fn corner_at(x: f64, y: f64) -> Option<usize> {
    let col = if x <= CORNER { 0 } else if x >= 1.0 - CORNER { 1 } else { return None };
    let row = if y >= 1.0 - CORNER { 0 } else if y <= CORNER { 1 } else { return None };
    Some(row * 2 + col) // 0 TL, 1 TR, 2 BL, 3 BR
}

/// Closest enabled edge band under the finger.
fn side_at(x: f64, y: f64, s: &TrackpadSettings) -> Option<Side> {
    [(Side::Left, x), (Side::Right, 1.0 - x), (Side::Bottom, y), (Side::Top, 1.0 - y)]
        .into_iter()
        .filter(|(side, d)| *d <= BAND && s.slide(*side) != Slide::Off)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(side, _)| side)
}

impl Engine {
    /// Feed one frame of touching fingers (normalized x, y). `level` reads the current volume/brightness.
    pub fn frame(&mut self, touches: &[(f64, f64)], now: f64, s: &TrackpadSettings, level: impl Fn(Slide) -> f64) -> Vec<Effect> {
        let mut out = Vec::new();
        let Some(mut g) = self.gesture.take() else {
            if let [(x, y)] = touches {
                self.gesture = Some(self.begin(*x, *y, now, s, &level));
            }
            return out;
        };
        match touches {
            [] => {
                g.lost += 1;
                // a tap ends the moment the finger lifts; a slide survives a few dropped frames
                if !g.moved || g.lost > 5 || now - g.seen > 0.12 {
                    self.finish(&g, now, &mut out);
                } else {
                    self.gesture = Some(g);
                }
            }
            [(x, y)] => {
                (g.x, g.y, g.seen, g.lost) = (*x, *y, now, 0);
                self.update(&mut g, s, &level, &mut out);
                self.gesture = Some(g);
            }
            _ => {
                // more fingers = a system gesture (scroll, swipe); let it through
                if g.moved && matches!(g.kind, Kind::Slide { .. }) {
                    out.push(Effect::End);
                }
            }
        }
        out
    }

    fn begin(&self, x: f64, y: f64, now: f64, s: &TrackpadSettings, level: &impl Fn(Slide) -> f64) -> Gesture {
        let kind = match (corner_at(x, y).filter(|c| s.corners[*c]), side_at(x, y, s)) {
            (Some(c), _) => Kind::Corner(c),
            (None, Some(side)) => slide_kind(side, s, level),
            (None, None) => Kind::Ignored,
        };
        let start = if let Kind::Slide { start, .. } = kind { start } else { 0.0 };
        Gesture {
            kind,
            x0: x,
            y0: y,
            t0: now,
            x,
            y,
            seen: now,
            moved: false,
            level: start,
            step: (start * TICK_STEPS).round() as i64,
            consumed: false,
            lost: 0,
        }
    }

    fn update(&self, g: &mut Gesture, s: &TrackpadSettings, level: &impl Fn(Slide) -> f64, out: &mut Vec<Effect>) {
        let (dx, dy) = (g.x - g.x0, g.y - g.y0);
        if let Kind::Corner(c) = g.kind {
            if dx.abs().max(dy.abs()) <= CLAIM {
                return;
            }
            // Sliding out of a corner drives the edge it slides along (if that edge has an action).
            let side = match (dy.abs() > dx.abs(), c) {
                (true, 0 | 2) => Side::Left,
                (true, _) => Side::Right,
                (false, 0 | 1) => Side::Top,
                (false, _) => Side::Bottom,
            };
            g.kind = if s.slide(side) == Slide::Off { Kind::Ignored } else { slide_kind(side, s, level) };
            if let Kind::Slide { start, .. } = g.kind {
                (g.level, g.step) = (start, (start * TICK_STEPS).round() as i64);
            }
        }
        let Kind::Slide { side, action, start } = g.kind else { return };
        let (along, across) = if side.vertical() { (dy, dx) } else { (dx, dy) };
        if !g.moved {
            if along.abs().max(across.abs()) <= CLAIM {
                return;
            }
            if across.abs() >= along.abs() {
                g.kind = Kind::Ignored; // moving into the pad: an ordinary cursor move
                return;
            }
            g.moved = true;
            out.push(Effect::Begin);
        }
        match action {
            Slide::Volume | Slide::Brightness => {
                let lvl = ((start + along / SPAN).clamp(0.0, 1.0) * FINE_STEPS).round() / FINE_STEPS;
                if lvl != g.level {
                    g.level = lvl;
                    out.push(Effect::Level(action, lvl));
                }
                let step = (lvl * TICK_STEPS).round() as i64;
                if step != g.step {
                    g.step = step;
                    out.push(Effect::Tick);
                }
            }
            Slide::Tracks if !g.consumed && along.abs() > TRACK_SWIPE => {
                g.consumed = true;
                out.push(Effect::Tick);
                out.push(Effect::Track { next: along > 0.0 }); // right / up = next
            }
            _ => {}
        }
    }

    fn finish(&mut self, g: &Gesture, now: f64, out: &mut Vec<Effect>) {
        match g.kind {
            Kind::Slide { .. } if g.moved => out.push(Effect::End),
            Kind::Corner(c) => {
                let tap = now - g.t0 < TAP_SECS && (g.x - g.x0).hypot(g.y - g.y0) < TAP_TRAVEL;
                if !tap {
                    return;
                }
                match self.last_corner_tap {
                    Some((prev, at)) if prev == c && now - at < DOUBLE_TAP_SECS => {
                        self.last_corner_tap = None;
                        out.push(Effect::Corner(c));
                    }
                    _ => self.last_corner_tap = Some((c, now)),
                }
            }
            _ => {}
        }
    }
}

fn slide_kind(side: Side, s: &TrackpadSettings, level: &impl Fn(Slide) -> f64) -> Kind {
    let action = s.slide(side);
    let start = if matches!(action, Slide::Volume | Slide::Brightness) { level(action) } else { 0.0 };
    Kind::Slide { side, action, start }
}

#[tauri::command]
pub fn trackpad_configure(settings: TrackpadSettings) -> usize {
    imp::configure(settings);
    imp::connected()
}

pub fn start(app: &tauri::AppHandle) {
    imp::start(app);
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use crate::multitouch::{self, Dev, Finger};
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, OnceLock};
    use tauri::{AppHandle, Emitter};

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
        fn CGWarpMouseCursorPosition(p: Point) -> i32;
        fn CFRelease(p: *const c_void);
    }

    static SETTINGS: Mutex<TrackpadSettings> = Mutex::new(TrackpadSettings {
        left: Slide::Off,
        right: Slide::Off,
        top: Slide::Off,
        bottom: Slide::Off,
        corners: [false; 4],
    });
    static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();
    static ANCHOR: Mutex<Option<(f64, f64)>> = Mutex::new(None);
    static APP: OnceLock<AppHandle> = OnceLock::new();
    static CONNECTED: AtomicUsize = AtomicUsize::new(0);

    pub fn configure(s: TrackpadSettings) {
        *SETTINGS.lock().unwrap() = s;
    }

    pub fn connected() -> usize {
        CONNECTED.load(Ordering::SeqCst)
    }

    pub fn start(app: &AppHandle) {
        let _ = APP.set(app.clone());
        multitouch::watch("trackpad", multitouch::is_trackpad, on_frame, &CONNECTED);
    }

    fn level(s: Slide) -> f64 {
        match s {
            Slide::Volume => crate::audio::output_level().unwrap_or(0.5) as f64,
            Slide::Brightness => crate::display::brightness(crate::display::active()),
            _ => 0.0,
        }
    }

    fn cursor() -> Option<(f64, f64)> {
        unsafe {
            let e = CGEventCreate(std::ptr::null());
            if e.is_null() {
                return None;
            }
            let p = CGEventGetLocation(e);
            CFRelease(e);
            Some((p.x, p.y))
        }
    }

    fn on_main(f: impl FnOnce() + Send + 'static) {
        if let Some(app) = APP.get() {
            let _ = app.run_on_main_thread(f);
        }
    }

    fn haptic() {
        on_main(|| unsafe {
            let Some(cls) = AnyClass::get(c"NSHapticFeedbackManager") else { return };
            let performer: *mut AnyObject = msg_send![cls, defaultPerformer];
            if !performer.is_null() {
                // NSHapticFeedbackPatternAlignment, NSHapticFeedbackPerformanceTimeNow
                let _: () = msg_send![performer, performFeedbackPattern: 1isize, performanceTime: 1usize];
            }
        });
    }

    fn apply(e: Effect) {
        match e {
            // No cursor hiding: a background app's hide count only bites once Ledge is active,
            // so an unmatched hide made the pointer vanish after clicking the dock. Pinning is enough.
            Effect::Begin => *ANCHOR.lock().unwrap() = cursor(),
            Effect::End => *ANCHOR.lock().unwrap() = None,
            Effect::Level(Slide::Volume, v) => crate::audio::set_output_level(v as f32),
            // the pointer is pinned during a slide, so "active" stays the screen the slide began on
            Effect::Level(Slide::Brightness, v) => crate::display::set_brightness(crate::display::active(), v),
            Effect::Level(..) => {}
            Effect::Tick => haptic(),
            Effect::Track { next } => {
                let _ = crate::media::imp::press(if next { crate::media::MediaKey::Next } else { crate::media::MediaKey::Prev });
            }
            Effect::Corner(c) => {
                if let Some(app) = APP.get() {
                    let _ = app.emit("trackpad-corner", c);
                }
            }
        }
    }

    extern "C" fn on_frame(_dev: Dev, fingers: *const Finger, n: i32, now: f64, _frame: i32) -> i32 {
        let settings = *SETTINGS.lock().unwrap();
        if !settings.any() {
            return 0;
        }
        let touches: Vec<(f64, f64)> = unsafe { multitouch::fingers(fingers, n) }
            .iter()
            .filter(|f| matches!(f.state, 3 | 4)) // on the glass
            .map(|f| (f.pos.x as f64, f.pos.y as f64))
            .collect();
        let effects = ENGINE.get_or_init(Default::default).lock().unwrap().frame(&touches, now, &settings, level);
        effects.into_iter().for_each(apply);
        // the system still moves the pointer with the finger; pin it while a slide owns the finger
        if let Some((x, y)) = *ANCHOR.lock().unwrap() {
            unsafe { CGWarpMouseCursorPosition(Point { x, y }) };
        }
        0
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::TrackpadSettings;
    // shortcut: trackpad edges are macOS-only (Windows precision touchpads need a different API)
    pub fn configure(_s: TrackpadSettings) {}
    pub fn connected() -> usize {
        0
    }
    pub fn start(_app: &tauri::AppHandle) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: TrackpadSettings =
        TrackpadSettings { left: Slide::Brightness, right: Slide::Volume, top: Slide::Off, bottom: Slide::Tracks, corners: [false, true, false, false] };

    /// Drags one finger along `path`, one frame per point, then lifts it.
    fn drag(e: &mut Engine, path: &[(f64, f64)], t0: f64) -> Vec<Effect> {
        let mut out = Vec::new();
        for (i, p) in path.iter().enumerate() {
            out.extend(e.frame(&[*p], t0 + i as f64 * 0.01, &S, |_| 0.5));
        }
        out.extend(e.frame(&[], t0 + path.len() as f64 * 0.01, &S, |_| 0.5));
        out.extend(e.frame(&[], t0 + 1.0, &S, |_| 0.5)); // past the dropped-frame grace
        out
    }

    #[test]
    fn sliding_up_the_right_edge_raises_volume() {
        let mut e = Engine::default();
        let fx = drag(&mut e, &[(0.95, 0.4), (0.95, 0.45), (0.95, 0.55), (0.95, 0.7)], 0.0);
        assert_eq!(fx.first(), Some(&Effect::Begin));
        assert!(fx.iter().any(|f| matches!(f, Effect::Level(Slide::Volume, v) if *v > 0.8)));
        assert!(fx.contains(&Effect::Tick));
        assert_eq!(fx.last(), Some(&Effect::End));
    }

    #[test]
    fn moving_inward_from_an_edge_stays_a_cursor_move() {
        let mut e = Engine::default();
        let fx = drag(&mut e, &[(0.05, 0.5), (0.1, 0.5), (0.3, 0.52), (0.6, 0.55)], 0.0);
        assert!(fx.is_empty(), "{fx:?}");
    }

    #[test]
    fn bottom_swipe_skips_once() {
        let mut e = Engine::default();
        let fx = drag(&mut e, &[(0.4, 0.05), (0.45, 0.05), (0.6, 0.05), (0.8, 0.05)], 0.0);
        assert_eq!(fx.iter().filter(|f| matches!(f, Effect::Track { next: true })).count(), 1);
    }

    #[test]
    fn double_tap_in_an_enabled_corner_fires_it() {
        let mut e = Engine::default();
        let tap = [(0.95, 0.95)];
        assert!(drag(&mut e, &tap, 0.0).is_empty());
        // second tap well inside the double-tap window (drag's lift frames end at t0 + 1.0, so start right after)
        let mut e = Engine::default();
        let mut fx = e.frame(&tap, 0.0, &S, |_| 0.5);
        fx.extend(e.frame(&[], 0.05, &S, |_| 0.5));
        fx.extend(e.frame(&tap, 0.2, &S, |_| 0.5));
        fx.extend(e.frame(&[], 0.25, &S, |_| 0.5));
        assert_eq!(fx, vec![Effect::Corner(1)]);
        // a disabled corner (top-left) does nothing
        let mut e = Engine::default();
        let left = [(0.05, 0.95)];
        let mut fx = e.frame(&left, 0.0, &S, |_| 0.5);
        fx.extend(e.frame(&[], 0.05, &S, |_| 0.5));
        fx.extend(e.frame(&left, 0.2, &S, |_| 0.5));
        fx.extend(e.frame(&[], 0.25, &S, |_| 0.5));
        assert!(!fx.iter().any(|f| matches!(f, Effect::Corner(_))));
    }

    #[test]
    fn two_fingers_hand_the_gesture_back_to_macos() {
        let mut e = Engine::default();
        let mut fx = e.frame(&[(0.95, 0.4)], 0.0, &S, |_| 0.5);
        fx.extend(e.frame(&[(0.95, 0.5)], 0.01, &S, |_| 0.5));
        fx.extend(e.frame(&[(0.95, 0.55), (0.5, 0.5)], 0.02, &S, |_| 0.5));
        assert_eq!(fx.last(), Some(&Effect::End));
        assert!(e.frame(&[(0.95, 0.7), (0.5, 0.6)], 0.03, &S, |_| 0.5).is_empty());
    }
}
