//! Play/pause/next/prev by posting the same system media-key events the keyboard sends.
//! Works with anything that owns "Now Playing" (Music, Spotify, browsers, …).

use serde::Deserialize;

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "lowercase")]
pub enum MediaKey {
    Play,
    Next,
    Prev,
}

#[tauri::command]
pub fn media_key(key: MediaKey) -> Result<(), String> {
    imp::press(key)
}

#[cfg(target_os = "macos")]
pub mod imp {
    use super::MediaKey;
    use objc2::encode::{Encode, Encoding, RefEncode};
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_foundation::NSPoint;
    use std::ffi::c_void;

    #[repr(transparent)]
    #[derive(Clone, Copy)]
    struct CGEventRef(*mut c_void);
    unsafe impl Encode for CGEventRef {
        const ENCODING: Encoding = Encoding::Pointer(&Encoding::Struct("__CGEvent", &[]));
    }
    unsafe impl RefEncode for CGEventRef {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventPost(tap: u32, event: CGEventRef);
        fn CGPreflightPostEventAccess() -> bool;
        fn CGRequestPostEventAccess() -> bool;
    }

    const NS_EVENT_TYPE_SYSTEM_DEFINED: usize = 14;
    const AUX_CONTROL_BUTTONS: i16 = 8;
    const KEY_DOWN: isize = 0xa;
    const KEY_UP: isize = 0xb;
    const HID_EVENT_TAP: u32 = 0;

    fn code(k: MediaKey) -> isize {
        // NX_KEYTYPE_PLAY / NEXT / PREVIOUS from IOKit/hidsystem/ev_keymap.h
        match k {
            MediaKey::Play => 16,
            MediaKey::Next => 17,
            MediaKey::Prev => 18,
        }
    }

    fn post(code: isize, state: isize) {
        unsafe {
            let flags: usize = (state as usize) << 8;
            let data1: isize = (code << 16) | (state << 8);
            let nil: *mut AnyObject = std::ptr::null_mut();
            let ev: *mut AnyObject = msg_send![class!(NSEvent),
                otherEventWithType: NS_EVENT_TYPE_SYSTEM_DEFINED,
                location: NSPoint::new(0.0, 0.0),
                modifierFlags: flags,
                timestamp: 0.0f64,
                windowNumber: 0isize,
                context: nil,
                subtype: AUX_CONTROL_BUTTONS,
                data1: data1,
                data2: -1isize];
            if ev.is_null() {
                return;
            }
            let cg: CGEventRef = msg_send![ev, CGEvent];
            CGEventPost(HID_EVENT_TAP, cg);
        }
    }

    fn ensure_access() -> Result<(), String> {
        if !unsafe { CGPreflightPostEventAccess() } {
            unsafe { CGRequestPostEventAccess() };
            return Err("Allow Ledge in System Settings → Privacy & Security → Accessibility, then try again".into());
        }
        Ok(())
    }

    pub fn press(k: MediaKey) -> Result<(), String> {
        ensure_access()?;
        post(code(k), KEY_DOWN);
        post(code(k), KEY_UP);
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use super::MediaKey;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn keybd_event(vk: u8, scan: u8, flags: u32, extra: usize);
    }
    const EXTENDED: u32 = 0x1;
    const KEY_UP: u32 = 0x2;

    pub fn press(k: MediaKey) -> Result<(), String> {
        let vk = match k {
            MediaKey::Play => 0xB3, // VK_MEDIA_PLAY_PAUSE
            MediaKey::Next => 0xB0, // VK_MEDIA_NEXT_TRACK
            MediaKey::Prev => 0xB1, // VK_MEDIA_PREV_TRACK
        };
        unsafe {
            keybd_event(vk, 0, EXTENDED, 0);
            keybd_event(vk, 0, EXTENDED | KEY_UP, 0);
        }
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    pub fn press(_k: super::MediaKey) -> Result<(), String> {
        Err("not implemented on this OS yet".into())
    }
}
