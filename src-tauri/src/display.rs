//! Screens: which one is active (under the pointer) and its brightness.
//! Apple screens (built-in, Studio Display) get real backlight control through DisplayServices;
//! any other monitor is dimmed in software with its gamma table, so every screen responds.

#[cfg(target_os = "macos")]
mod imp {
    use std::collections::HashMap;
    use std::ffi::{c_char, c_void};
    use std::sync::{Mutex, OnceLock};

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
        fn CGGetDisplaysWithPoint(p: Point, max: u32, out: *mut u32, count: *mut u32) -> i32;
        fn CGMainDisplayID() -> u32;
        #[allow(clippy::too_many_arguments)]
        fn CGSetDisplayTransferByFormula(
            d: u32,
            r_min: f32,
            r_max: f32,
            r_gamma: f32,
            g_min: f32,
            g_max: f32,
            g_gamma: f32,
            b_min: f32,
            b_max: f32,
            b_gamma: f32,
        ) -> i32;
        fn CFRelease(p: *const c_void);
    }

    unsafe extern "C" {
        fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    struct Backlight {
        get: unsafe extern "C" fn(u32, *mut f32) -> i32,
        set: unsafe extern "C" fn(u32, f32) -> i32,
    }

    /// Never dim a monitor to black: you'd have nothing to slide back up with.
    const MIN_GAMMA: f32 = 0.12;

    static SOFT: Mutex<Option<HashMap<u32, f64>>> = Mutex::new(None);

    fn backlight() -> Option<&'static Backlight> {
        static B: OnceLock<Option<Backlight>> = OnceLock::new();
        B.get_or_init(|| unsafe {
            let h = dlopen(c"/System/Library/PrivateFrameworks/DisplayServices.framework/DisplayServices".as_ptr(), 2);
            if h.is_null() {
                return None;
            }
            let get = dlsym(h, c"DisplayServicesGetBrightness".as_ptr());
            let set = dlsym(h, c"DisplayServicesSetBrightness".as_ptr());
            (!get.is_null() && !set.is_null()).then(|| Backlight { get: std::mem::transmute(get), set: std::mem::transmute(set) })
        })
        .as_ref()
    }

    /// The screen under the pointer (falls back to the main screen).
    pub fn active() -> u32 {
        unsafe {
            let e = CGEventCreate(std::ptr::null());
            if !e.is_null() {
                let p = CGEventGetLocation(e);
                CFRelease(e);
                let (mut id, mut n) = (0u32, 0u32);
                if CGGetDisplaysWithPoint(p, 1, &mut id, &mut n) == 0 && n > 0 {
                    return id;
                }
            }
            CGMainDisplayID()
        }
    }

    fn hardware_level(id: u32) -> Option<f64> {
        let b = backlight()?;
        let mut v = 0f32;
        (unsafe { (b.get)(id, &mut v) } == 0).then_some(v.clamp(0.0, 1.0) as f64)
    }

    pub fn brightness(id: u32) -> f64 {
        hardware_level(id).unwrap_or_else(|| SOFT.lock().unwrap().as_ref().and_then(|m| m.get(&id).copied()).unwrap_or(1.0))
    }

    pub fn set_brightness(id: u32, v: f64) {
        let v = v.clamp(0.0, 1.0);
        if hardware_level(id).is_some() {
            if let Some(b) = backlight() {
                unsafe { (b.set)(id, v as f32) };
            }
            return;
        }
        // shortcut: gamma dimming can't go above the monitor's own brightness; real DDC/CI control
        // (IOAVService on Apple silicon) is the upgrade if that matters
        let max = MIN_GAMMA + (1.0 - MIN_GAMMA) * v as f32;
        unsafe { CGSetDisplayTransferByFormula(id, 0.0, max, 1.0, 0.0, max, 1.0, 0.0, max, 1.0) };
        SOFT.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, v);
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn active() -> u32 {
        0
    }
    pub fn brightness(_id: u32) -> f64 {
        1.0
    }
    pub fn set_brightness(_id: u32, _v: f64) {}
}

pub use imp::{active, brightness, set_brightness};
