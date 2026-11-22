//! "Is any camera in use?" via CoreMediaIO — the same signal behind macOS's green camera dot.

#[cfg(target_os = "macos")]
pub fn in_use() -> bool {
    use std::ffi::c_void;

    #[repr(C)]
    struct Addr {
        selector: u32,
        scope: u32,
        element: u32,
    }

    #[link(name = "CoreMediaIO", kind = "framework")]
    unsafe extern "C" {
        fn CMIOObjectGetPropertyDataSize(id: u32, a: *const Addr, qs: u32, qd: *const c_void, out: *mut u32) -> i32;
        fn CMIOObjectGetPropertyData(
            id: u32,
            a: *const Addr,
            qs: u32,
            qd: *const c_void,
            size: u32,
            used: *mut u32,
            out: *mut c_void,
        ) -> i32;
    }

    const fn fcc(s: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*s)
    }
    const SYSTEM: u32 = 1;
    let devices = Addr { selector: fcc(b"dev#"), scope: fcc(b"glob"), element: 0 };
    let running = Addr { selector: fcc(b"gone"), scope: fcc(b"glob"), element: 0 };

    let mut size = 0u32;
    if unsafe { CMIOObjectGetPropertyDataSize(SYSTEM, &devices, 0, std::ptr::null(), &mut size) } != 0 {
        return false;
    }
    let mut ids = vec![0u32; size as usize / 4];
    let mut used = 0u32;
    let ok = unsafe { CMIOObjectGetPropertyData(SYSTEM, &devices, 0, std::ptr::null(), size, &mut used, ids.as_mut_ptr() as *mut c_void) };
    if ok != 0 {
        return false;
    }
    ids.truncate(used as usize / 4);
    ids.iter().any(|id| {
        let mut on = 0u32;
        let ok = unsafe { CMIOObjectGetPropertyData(*id, &running, 0, std::ptr::null(), 4, &mut used, &mut on as *mut u32 as *mut c_void) };
        ok == 0 && on != 0
    })
}

#[cfg(not(target_os = "macos"))]
pub fn in_use() -> bool {
    false // shortcut: Windows has no public equivalent; registry CapabilityAccessManager if wanted
}
