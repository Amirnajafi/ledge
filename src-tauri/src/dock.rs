//! Dock window placement on a screen edge + native Liquid Glass backdrop.

use serde::{Deserialize, Serialize};
use tauri::WebviewWindow;

const EDGE_MARGIN: f64 = 6.0;

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// Rect in a bottom-left-origin space (Cocoa) — `area` is the usable screen area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Stick a `w`×`h` window to `edge` of `area`. `along` (0…1) is where it sits along that edge:
/// 0 = top / left end, 0.5 = centred, 1 = bottom / right end.
pub fn frame_on_edge(area: Rect, edge: Edge, w: f64, h: f64, along: f64) -> Rect {
    let along = along.clamp(0.0, 1.0);
    let cx = area.x + (area.w - w).max(0.0) * along;
    let cy = area.y + (area.h - h).max(0.0) * (1.0 - along); // bottom-left origin: top end = highest y
    let (x, y) = match edge {
        Edge::Left => (area.x + EDGE_MARGIN, cy),
        Edge::Right => (area.x + area.w - w - EDGE_MARGIN, cy),
        Edge::Bottom => (cx, area.y + EDGE_MARGIN),
        Edge::Top => (cx, area.y + area.h - h - EDGE_MARGIN),
    };
    Rect { x: x.round(), y: y.round(), w, h }
}

#[tauri::command]
pub fn dock_place(window: WebviewWindow, edge: Edge, width: f64, height: f64, radius: f64, along: f64) -> Result<(), String> {
    imp::place(&window, edge, width, height, radius, along)
}

/// Where a dock dropped at `win` belongs: the nearest edge of `area`, and how far along it.
/// Both rects are top-left origin (window coordinates), any unit.
pub fn drop_target(area: Rect, win: Rect) -> (Edge, f64) {
    let (cx, cy) = (win.x + win.w / 2.0, win.y + win.h / 2.0);
    let gaps = [
        (cx - area.x, Edge::Left),
        (area.x + area.w - cx, Edge::Right),
        (cy - area.y, Edge::Top),
        (area.y + area.h - cy, Edge::Bottom),
    ];
    let edge = gaps.iter().min_by(|a, b| a.0.total_cmp(&b.0)).map(|g| g.1).unwrap_or(Edge::Left);
    let frac = |pos: f64, free: f64| if free <= 0.0 { 0.5 } else { (pos / free).clamp(0.0, 1.0) };
    let along = match edge {
        Edge::Left | Edge::Right => frac(win.y - area.y, area.h - win.h),
        Edge::Top | Edge::Bottom => frac(win.x - area.x, area.w - win.w),
    };
    (edge, (along * 1000.0).round() / 1000.0)
}

/// After the user drags the dock: snap target on the screen it was dropped on.
#[tauri::command]
pub fn dock_dropped(window: WebviewWindow) -> Result<(Edge, f64), String> {
    let mon = window.current_monitor().map_err(|e| e.to_string())?.ok_or("no monitor")?;
    let wa = mon.work_area();
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let area = Rect { x: wa.position.x as f64, y: wa.position.y as f64, w: wa.size.width as f64, h: wa.size.height as f64 };
    let win = Rect { x: pos.x as f64, y: pos.y as f64, w: size.width as f64, h: size.height as f64 };
    Ok(drop_target(area, win))
}

pub fn install_glass(window: &WebviewWindow) {
    imp::install_glass(window);
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{class, msg_send};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// NSGlassEffectView (macOS 26+) or NSVisualEffectView fallback, stored as a raw pointer.
    static GLASS: AtomicUsize = AtomicUsize::new(0);
    const NS_VIEW_WIDTH_SIZABLE: usize = 2;
    const NS_VIEW_HEIGHT_SIZABLE: usize = 16;
    const NS_WINDOW_BELOW: isize = -1;

    fn ns_window(window: &WebviewWindow) -> Result<usize, String> {
        window.ns_window().map(|p| p as usize).map_err(|e| e.to_string())
    }

    pub fn install_glass(window: &WebviewWindow) {
        let Ok(win) = ns_window(window) else { return };
        let _ = window.run_on_main_thread(move || unsafe {
            let win = win as *mut AnyObject;
            let content: *mut AnyObject = msg_send![win, contentView];
            let bounds: NSRect = msg_send![content, bounds];
            let glass: *mut AnyObject = match AnyClass::get(c"NSGlassEffectView") {
                Some(cls) => {
                    let v: *mut AnyObject = msg_send![cls, alloc];
                    msg_send![v, initWithFrame: bounds]
                }
                None => {
                    // pre-Tahoe fallback: classic blur
                    let v: *mut AnyObject = msg_send![class!(NSVisualEffectView), alloc];
                    let v: *mut AnyObject = msg_send![v, initWithFrame: bounds];
                    let _: () = msg_send![v, setMaterial: 13isize]; // hudWindow
                    let _: () = msg_send![v, setBlendingMode: 0isize]; // behindWindow
                    let _: () = msg_send![v, setState: 1isize]; // active
                    let _: () = msg_send![v, setWantsLayer: true];
                    v
                }
            };
            let mask = NS_VIEW_WIDTH_SIZABLE | NS_VIEW_HEIGHT_SIZABLE;
            let _: () = msg_send![glass, setAutoresizingMask: mask];
            let nil: *mut AnyObject = std::ptr::null_mut();
            let _: () = msg_send![content, addSubview: glass, positioned: NS_WINDOW_BELOW, relativeTo: nil];
            GLASS.store(glass as usize, Ordering::SeqCst);
        });
    }

    unsafe fn set_radius(radius: f64) {
        let glass = GLASS.load(Ordering::SeqCst) as *mut AnyObject;
        if glass.is_null() {
            return;
        }
        let responds: bool = unsafe { msg_send![glass, respondsToSelector: objc2::sel!(setCornerRadius:)] };
        unsafe {
            if responds {
                let _: () = msg_send![glass, setCornerRadius: radius];
            } else {
                let layer: *mut AnyObject = msg_send![glass, layer];
                if !layer.is_null() {
                    let _: () = msg_send![layer, setCornerRadius: radius];
                    let _: () = msg_send![layer, setMasksToBounds: true];
                }
            }
        }
    }

    pub fn place(window: &WebviewWindow, edge: Edge, w: f64, h: f64, radius: f64, along: f64) -> Result<(), String> {
        let win = ns_window(window)?;
        let first_show = !window.is_visible().unwrap_or(true);
        window
            .run_on_main_thread(move || unsafe {
                let win = win as *mut AnyObject;
                // first show: the menu-bar screen; afterwards: whichever screen the dock is on
                let mut screen: *mut AnyObject =
                    if first_show { std::ptr::null_mut() } else { msg_send![win, screen] };
                if screen.is_null() {
                    let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
                    let count: usize = msg_send![screens, count];
                    if count > 0 {
                        screen = msg_send![screens, objectAtIndex: 0usize];
                    }
                }
                if screen.is_null() {
                    return;
                }
                // visibleFrame excludes the menu bar and the system Dock
                let vf: NSRect = msg_send![screen, visibleFrame];
                let area = Rect { x: vf.origin.x, y: vf.origin.y, w: vf.size.width, h: vf.size.height };
                let r = frame_on_edge(area, edge, w, h, along);
                let frame = NSRect::new(NSPoint::new(r.x, r.y), NSSize::new(r.w, r.h));
                set_radius(radius);
                let _: () = msg_send![win, setFrame: frame, display: true, animate: !first_show];
            })
            .map_err(|e| e.to_string())?;
        if first_show {
            window.show().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;
    use tauri::{PhysicalPosition, PhysicalSize};

    // shortcut: no native glass off macOS yet (CSS fallback only), add Mica/Acrylic for Windows later
    pub fn install_glass(_window: &WebviewWindow) {}

    pub fn place(window: &WebviewWindow, edge: Edge, w: f64, h: f64, _radius: f64, along: f64) -> Result<(), String> {
        let mon = window
            .current_monitor()
            .map_err(|e| e.to_string())?
            .ok_or("no monitor")?;
        let s = mon.scale_factor();
        let wa = mon.work_area();
        // top-left origin here: flip Top/Bottom against the bottom-left math
        let area = Rect { x: wa.position.x as f64, y: 0.0, w: wa.size.width as f64, h: wa.size.height as f64 };
        let r = frame_on_edge(area, edge, w * s, h * s, along);
        let y = wa.position.y as f64 + (area.h - r.y - r.h);
        window.set_size(PhysicalSize::new(r.w, r.h)).map_err(|e| e.to_string())?;
        window.set_position(PhysicalPosition::new(r.x, y)).map_err(|e| e.to_string())?;
        window.show().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const AREA: Rect = Rect { x: 0.0, y: 80.0, w: 1000.0, h: 600.0 };

    #[test]
    fn left_edge_hugs_left_and_centres_vertically() {
        let r = frame_on_edge(AREA, Edge::Left, 60.0, 200.0, 0.5);
        assert_eq!(r, Rect { x: EDGE_MARGIN, y: 280.0, w: 60.0, h: 200.0 });
    }

    #[test]
    fn right_and_top_edges_respect_margin() {
        let r = frame_on_edge(AREA, Edge::Right, 60.0, 200.0, 0.5);
        assert_eq!(r.x, 1000.0 - 60.0 - EDGE_MARGIN);
        let t = frame_on_edge(AREA, Edge::Top, 300.0, 60.0, 0.5);
        assert_eq!((t.x, t.y), (350.0, 80.0 + 600.0 - 60.0 - EDGE_MARGIN));
    }

    #[test]
    fn bottom_edge_sits_on_area_floor() {
        let r = frame_on_edge(AREA, Edge::Bottom, 300.0, 60.0, 0.5);
        assert_eq!((r.x, r.y), (350.0, 80.0 + EDGE_MARGIN));
    }

    #[test]
    fn along_moves_the_dock_to_either_end() {
        // bottom-left origin: along 0 on a side edge = top of the area
        let top = frame_on_edge(AREA, Edge::Left, 60.0, 200.0, 0.0);
        assert_eq!(top.y, 80.0 + 600.0 - 200.0);
        let bottom = frame_on_edge(AREA, Edge::Left, 60.0, 200.0, 1.0);
        assert_eq!(bottom.y, 80.0);
        let right = frame_on_edge(AREA, Edge::Top, 300.0, 60.0, 1.0);
        assert_eq!(right.x, 700.0);
    }

    #[test]
    fn a_drop_snaps_to_the_nearest_edge_where_it_was_left() {
        // top-left origin screen 1000×600 at (2000, 0): a second monitor
        let screen = Rect { x: 2000.0, y: 0.0, w: 1000.0, h: 600.0 };
        let (edge, along) = drop_target(screen, Rect { x: 2010.0, y: 0.0, w: 60.0, h: 300.0 });
        assert_eq!((edge, along), (Edge::Left, 0.0)); // top-left corner, vertical dock
        let (edge, along) = drop_target(screen, Rect { x: 2350.0, y: 530.0, w: 300.0, h: 60.0 });
        assert_eq!((edge, along), (Edge::Bottom, 0.5));
        let (edge, _) = drop_target(screen, Rect { x: 2900.0, y: 200.0, w: 60.0, h: 300.0 });
        assert_eq!(edge, Edge::Right);
    }
}
