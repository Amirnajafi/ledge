//! Shared access to macOS's private MultitouchSupport framework (Magic Mouse surface, trackpads).
//! Loaded with dlopen, so a missing or changed framework only switches those features off.

use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub type Dev = *mut c_void;
pub type Callback = extern "C" fn(Dev, *const Finger, i32, f64, i32) -> i32;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

/// The framework's per-finger record (96 bytes; layout as used by MiddleClick, jitouch, SlideSense).
/// `pos` is normalized 0…1, x left→right, y bottom→top.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Finger {
    pub frame: i32,
    pub timestamp: f64,
    pub identifier: i32,
    pub state: i32,
    _foo3: i32,
    _foo4: i32,
    pub pos: Vec2,
    pub vel: Vec2,
    pub size: f32,
    _zero1: i32,
    pub angle: f32,
    pub major: f32,
    pub minor: f32,
    pub mm_pos: Vec2,
    pub mm_vel: Vec2,
    _zero2: [i32; 2],
    _unk2: f32,
}

/// The fingers of one frame (empty when the framework passes none).
///
/// # Safety
/// `p` and `n` must come straight from the framework's frame callback.
pub unsafe fn fingers<'a>(p: *const Finger, n: i32) -> &'a [Finger] {
    if p.is_null() || n <= 0 { &[] } else { unsafe { std::slice::from_raw_parts(p, n as usize) } }
}

pub struct Mt {
    create_list: unsafe extern "C" fn() -> *const c_void,
    register: unsafe extern "C" fn(Dev, Callback),
    unregister: unsafe extern "C" fn(Dev, Callback),
    start: unsafe extern "C" fn(Dev, i32),
    stop: unsafe extern "C" fn(Dev),
    family: unsafe extern "C" fn(Dev, *mut i32) -> i32,
    built_in: unsafe extern "C" fn(Dev) -> bool,
    dims: unsafe extern "C" fn(Dev, *mut i32, *mut i32) -> i32,
}

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(a: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(a: *const c_void, i: isize) -> *const c_void;
    fn CFRelease(p: *const c_void);
}

const MAGIC_MOUSE_FAMILIES: [i32; 2] = [112, 113];
const MAX_MOUSE_MM100: i32 = 11_000; // a Magic Trackpad is ~160 mm wide; the mouse's surface is far smaller
const RESCAN: Duration = Duration::from_secs(3);

pub fn mt() -> Option<&'static Mt> {
    static MT: OnceLock<Option<Mt>> = OnceLock::new();
    MT.get_or_init(|| unsafe {
        let h = dlopen(c"/System/Library/PrivateFrameworks/MultitouchSupport.framework/MultitouchSupport".as_ptr(), 2);
        if h.is_null() {
            eprintln!("[multitouch] framework not available");
            return None;
        }
        let sym = |name: &std::ffi::CStr| {
            let p = dlsym(h, name.as_ptr());
            (!p.is_null()).then_some(p)
        };
        Some(Mt {
            create_list: std::mem::transmute(sym(c"MTDeviceCreateList")?),
            register: std::mem::transmute(sym(c"MTRegisterContactFrameCallback")?),
            unregister: std::mem::transmute(sym(c"MTUnregisterContactFrameCallback")?),
            start: std::mem::transmute(sym(c"MTDeviceStart")?),
            stop: std::mem::transmute(sym(c"MTDeviceStop")?),
            family: std::mem::transmute(sym(c"MTDeviceGetFamilyID")?),
            built_in: std::mem::transmute(sym(c"MTDeviceIsBuiltIn")?),
            dims: std::mem::transmute(sym(c"MTDeviceGetSensorSurfaceDimensions")?),
        })
    })
    .as_ref()
}

pub fn is_magic_mouse(mt: &Mt, d: Dev) -> bool {
    let (mut family, mut w, mut h) = (0, 0, 0);
    unsafe {
        (mt.family)(d, &mut family);
        (mt.dims)(d, &mut w, &mut h);
        MAGIC_MOUSE_FAMILIES.contains(&family) || (!(mt.built_in)(d) && w > 0 && w.max(h) < MAX_MOUSE_MM100)
    }
}

pub fn is_trackpad(mt: &Mt, d: Dev) -> bool {
    !is_magic_mouse(mt, d)
}

/// Keeps `cb` registered on every device `want` accepts, in a background thread. Re-scans every
/// few seconds (Bluetooth devices come and go) and after sleep, which silently kills callbacks.
pub fn watch(name: &'static str, want: fn(&Mt, Dev) -> bool, cb: Callback, connected: &'static AtomicUsize) {
    let Some(mt) = mt() else { return };
    std::thread::spawn(move || {
        let mut list: *const c_void = std::ptr::null();
        let mut devs: Vec<usize> = Vec::new();
        let mut last_tick = Instant::now();
        loop {
            let slept = last_tick.elapsed() > RESCAN * 4;
            last_tick = Instant::now();
            let fresh = unsafe { (mt.create_list)() };
            let found: Vec<usize> = (0..unsafe { CFArrayGetCount(fresh) })
                .map(|i| unsafe { CFArrayGetValueAtIndex(fresh, i) } as usize)
                .filter(|d| want(mt, *d as Dev))
                .collect();
            if found.len() != devs.len() || slept {
                for d in devs.drain(..) {
                    unsafe {
                        (mt.unregister)(d as Dev, cb);
                        (mt.stop)(d as Dev);
                    }
                }
                if !list.is_null() {
                    unsafe { CFRelease(list) };
                }
                for d in &found {
                    unsafe {
                        (mt.register)(*d as Dev, cb);
                        (mt.start)(*d as Dev, 0);
                    }
                }
                eprintln!("[multitouch] {name}: listening to {} device(s)", found.len());
                list = fresh;
                devs = found;
                connected.store(devs.len(), Ordering::SeqCst);
            } else {
                unsafe { CFRelease(fresh) };
            }
            std::thread::sleep(RESCAN);
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn finger_record_is_96_bytes() {
        // the framework hands us an array of these; a wrong size scrambles every finger after the first
        assert_eq!(std::mem::size_of::<super::Finger>(), 96);
    }
}
