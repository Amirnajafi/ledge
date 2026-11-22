//! Mic / speaker: list devices, switch default device, mute — CoreAudio on macOS, Core Audio (WASAPI) on Windows.

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Input,
    Output,
}

#[derive(Serialize, Clone, Debug)]
pub struct Device {
    pub id: u32,
    pub name: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Side {
    pub devices: Vec<Device>,
    pub default: u32,
    pub muted: bool,
    /// Some app is using the default device right now (mic capturing / audio playing).
    pub in_use: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct AudioState {
    pub input: Side,
    pub output: Side,
    /// Any camera is streaming to any app (including Ledge's own preview).
    pub camera_in_use: bool,
}

#[tauri::command]
pub fn audio_state() -> Result<AudioState, String> {
    Ok(AudioState { input: imp::side(Kind::Input)?, output: imp::side(Kind::Output)?, camera_in_use: crate::camera::in_use() })
}

#[tauri::command]
pub fn audio_set_default(kind: Kind, id: u32) -> Result<(), String> {
    imp::set_default(kind, id)
}

#[tauri::command]
pub fn audio_set_mute(kind: Kind, muted: bool) -> Result<bool, String> {
    imp::set_mute(kind, muted)
}

/// Default output's volume 0…1 (muted reads as 0; None when it has no volume control).
pub fn output_level() -> Option<f32> {
    imp::output_level()
}

/// Sets the default output's volume; raising it from muted unmutes first.
pub fn set_output_level(v: f32) {
    imp::set_output_level(v)
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::sync::Mutex;

    #[repr(C)]
    struct Addr {
        selector: u32,
        scope: u32,
        element: u32,
    }

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectHasProperty(id: u32, a: *const Addr) -> u8;
        fn AudioObjectIsPropertySettable(id: u32, a: *const Addr, out: *mut u8) -> i32;
        fn AudioObjectGetPropertyDataSize(id: u32, a: *const Addr, qs: u32, qd: *const c_void, out: *mut u32) -> i32;
        fn AudioObjectGetPropertyData(id: u32, a: *const Addr, qs: u32, qd: *const c_void, sz: *mut u32, out: *mut c_void) -> i32;
        fn AudioObjectSetPropertyData(id: u32, a: *const Addr, qs: u32, qd: *const c_void, sz: u32, data: *const c_void) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringGetCString(s: *const c_void, buf: *mut u8, len: isize, enc: u32) -> u8;
        fn CFRelease(p: *const c_void);
    }

    const fn fcc(s: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*s)
    }
    const SYSTEM: u32 = 1;
    const GLOBAL: u32 = fcc(b"glob");
    const DEVICES: u32 = fcc(b"dev#");
    const DEFAULT_IN: u32 = fcc(b"dIn ");
    const DEFAULT_OUT: u32 = fcc(b"dOut");
    const NAME: u32 = fcc(b"lnam");
    const STREAMS: u32 = fcc(b"stm#");
    const MUTE: u32 = fcc(b"mute");
    const VOLUME: u32 = fcc(b"volm");
    const RUNNING_SOMEWHERE: u32 = fcc(b"gone");
    const UTF8: u32 = 0x0800_0100;

    /// Volume saved before a volume-0 "mute" on devices with no mute control.
    static SAVED_VOLUME: Mutex<Option<HashMap<u32, f32>>> = Mutex::new(None);

    fn scope(k: Kind) -> u32 {
        match k {
            Kind::Input => fcc(b"inpt"),
            Kind::Output => fcc(b"outp"),
        }
    }

    fn addr(selector: u32, scope: u32, element: u32) -> Addr {
        Addr { selector, scope, element }
    }

    fn get<T: Copy + Default>(id: u32, a: &Addr) -> Option<T> {
        let mut v = T::default();
        let mut sz = size_of::<T>() as u32;
        let st = unsafe { AudioObjectGetPropertyData(id, a, 0, std::ptr::null(), &mut sz, &mut v as *mut T as *mut c_void) };
        (st == 0).then_some(v)
    }

    fn set<T: Copy>(id: u32, a: &Addr, v: T) -> bool {
        unsafe { AudioObjectSetPropertyData(id, a, 0, std::ptr::null(), size_of::<T>() as u32, &v as *const T as *const c_void) == 0 }
    }

    fn settable(id: u32, a: &Addr) -> bool {
        let mut s = 0u8;
        unsafe { AudioObjectHasProperty(id, a) != 0 && AudioObjectIsPropertySettable(id, a, &mut s) == 0 && s != 0 }
    }

    fn device_ids() -> Vec<u32> {
        let a = addr(DEVICES, GLOBAL, 0);
        let mut sz = 0u32;
        if unsafe { AudioObjectGetPropertyDataSize(SYSTEM, &a, 0, std::ptr::null(), &mut sz) } != 0 {
            return vec![];
        }
        let mut ids = vec![0u32; sz as usize / 4];
        let ok = unsafe { AudioObjectGetPropertyData(SYSTEM, &a, 0, std::ptr::null(), &mut sz, ids.as_mut_ptr() as *mut c_void) };
        if ok != 0 {
            return vec![];
        }
        ids.truncate(sz as usize / 4);
        ids
    }

    fn has_streams(id: u32, k: Kind) -> bool {
        let mut sz = 0u32;
        let a = addr(STREAMS, scope(k), 0);
        unsafe { AudioObjectGetPropertyDataSize(id, &a, 0, std::ptr::null(), &mut sz) == 0 && sz > 0 }
    }

    fn name(id: u32) -> String {
        let Some(cf) = get::<usize>(id, &addr(NAME, GLOBAL, 0)).filter(|p| *p != 0) else {
            return format!("Device {id}");
        };
        let mut buf = [0u8; 256];
        let ok = unsafe { CFStringGetCString(cf as *const c_void, buf.as_mut_ptr(), buf.len() as isize, UTF8) };
        unsafe { CFRelease(cf as *const c_void) };
        if ok == 0 {
            return format!("Device {id}");
        }
        let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    }

    fn default_device(k: Kind) -> u32 {
        let sel = if matches!(k, Kind::Input) { DEFAULT_IN } else { DEFAULT_OUT };
        get::<u32>(SYSTEM, &addr(sel, GLOBAL, 0)).unwrap_or(0)
    }

    /// Main element first, then the first two channels (many USB mics only expose per-channel controls).
    fn elements(id: u32, sel: u32, k: Kind) -> Vec<u32> {
        if settable(id, &addr(sel, scope(k), 0)) {
            return vec![0];
        }
        [1, 2].into_iter().filter(|e| settable(id, &addr(sel, scope(k), *e))).collect()
    }

    fn volume(id: u32, k: Kind) -> Option<f32> {
        elements(id, VOLUME, k).first().and_then(|e| get::<f32>(id, &addr(VOLUME, scope(k), *e)))
    }

    /// Muted = mute switch on OR volume at 0 (both are silent to every app).
    fn is_muted(id: u32, k: Kind) -> bool {
        if let Some(e) = elements(id, MUTE, k).first() {
            return get::<u32>(id, &addr(MUTE, scope(k), *e)).unwrap_or(0) != 0 || volume(id, k) == Some(0.0);
        }
        let saved = SAVED_VOLUME.lock().unwrap();
        if saved.as_ref().is_some_and(|m| m.contains_key(&id)) {
            return true;
        }
        volume(id, k) == Some(0.0)
    }

    fn set_volume(id: u32, k: Kind, v: f32) {
        elements(id, VOLUME, k).iter().for_each(|e| {
            set(id, &addr(VOLUME, scope(k), *e), v);
        });
    }

    pub fn side(k: Kind) -> Result<Side, String> {
        let devices = device_ids()
            .into_iter()
            .filter(|id| has_streams(*id, k))
            .map(|id| Device { id, name: name(id) })
            .collect();
        let default = default_device(k);
        let in_use = default != 0 && get::<u32>(default, &addr(RUNNING_SOMEWHERE, GLOBAL, 0)).unwrap_or(0) != 0;
        Ok(Side { devices, default, muted: default != 0 && is_muted(default, k), in_use })
    }

    pub fn set_default(k: Kind, id: u32) -> Result<(), String> {
        if !has_streams(id, k) {
            return Err("device does not support this direction".into());
        }
        let sel = if matches!(k, Kind::Input) { DEFAULT_IN } else { DEFAULT_OUT };
        set(SYSTEM, &addr(sel, GLOBAL, 0), id).then_some(()).ok_or_else(|| "CoreAudio refused device switch".into())
    }

    pub fn set_mute(k: Kind, muted: bool) -> Result<bool, String> {
        let id = default_device(k);
        if id == 0 {
            return Err("no default device".into());
        }
        let sc = scope(k);
        let mute_els = elements(id, MUTE, k);
        if !mute_els.is_empty() {
            let ok = mute_els.iter().all(|e| set(id, &addr(MUTE, sc, *e), muted as u32));
            if !ok {
                return Err("mute failed".into());
            }
            // Unmute must be audible: a volume left at 0 (by us earlier or another app) is still silent.
            if !muted && volume(id, k) == Some(0.0) {
                let v = SAVED_VOLUME.lock().unwrap().get_or_insert_with(HashMap::new).remove(&id).unwrap_or(0.75);
                set_volume(id, k, v);
            }
            return Ok(is_muted(id, k));
        }
        // No mute control: fall back to volume 0 and remember the old level.
        let vol_els = elements(id, VOLUME, k);
        if vol_els.is_empty() {
            return Err("device has no mute or volume control".into());
        }
        let mut guard = SAVED_VOLUME.lock().unwrap();
        let saved = guard.get_or_insert_with(HashMap::new);
        if muted {
            let cur = get::<f32>(id, &addr(VOLUME, sc, vol_els[0])).unwrap_or(1.0);
            saved.entry(id).or_insert(if cur > 0.0 { cur } else { 0.75 });
            set_volume(id, k, 0.0);
        } else {
            set_volume(id, k, saved.remove(&id).unwrap_or(0.75));
        }
        drop(guard);
        Ok(is_muted(id, k))
    }

    pub fn output_level() -> Option<f32> {
        let id = default_device(Kind::Output);
        if is_muted(id, Kind::Output) {
            return Some(0.0);
        }
        volume(id, Kind::Output)
    }

    pub fn set_output_level(v: f32) {
        let id = default_device(Kind::Output);
        if v > 0.0 && is_muted(id, Kind::Output) {
            let _ = set_mute(Kind::Output, false);
        }
        set_volume(id, Kind::Output, v);
    }
}

#[cfg(windows)]
mod imp {
    //! Core Audio: IMMDeviceEnumerator for devices, IAudioEndpointVolume for mute/volume,
    //! IAudioMeterInformation for "something is playing", IPolicyConfig to switch the default.
    use super::*;
    use windows::core::{interface, Interface, IUnknown, IUnknown_Vtbl, GUID, HRESULT, PCWSTR};
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
    use windows::Win32::Media::Audio::{
        eCapture, eCommunications, eConsole, eMultimedia, eRender, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
        DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};

    /// Undocumented but stable since Windows 7: what the Sound settings use to change the default device.
    /// Only the 11th method is called; the ten before it just hold their vtable slots.
    #[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    unsafe trait IPolicyConfig: IUnknown {
        fn get_mix_format(&self) -> HRESULT;
        fn get_device_format(&self) -> HRESULT;
        fn reset_device_format(&self) -> HRESULT;
        fn set_device_format(&self) -> HRESULT;
        fn get_processing_period(&self) -> HRESULT;
        fn set_processing_period(&self) -> HRESULT;
        fn get_share_mode(&self) -> HRESULT;
        fn set_share_mode(&self) -> HRESULT;
        fn get_property_value(&self) -> HRESULT;
        fn set_property_value(&self) -> HRESULT;
        fn set_default_endpoint(&self, id: PCWSTR, role: ERole) -> HRESULT;
    }
    const CLSID_POLICY_CONFIG: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
    /// Peak level above which the default output counts as playing.
    const PLAYING_PEAK: f32 = 0.0001;

    type R<T> = Result<T, String>;
    fn err(e: windows::core::Error) -> String {
        e.message().to_string()
    }

    fn flow(k: Kind) -> EDataFlow {
        match k {
            Kind::Input => eCapture,
            Kind::Output => eRender,
        }
    }

    fn enumerator() -> R<IMMDeviceEnumerator> {
        unsafe {
            // already-initialised threads (the UI thread is STA) answer RPC_E_CHANGED_MODE: COM still works
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(err)
        }
    }

    fn id_of(d: &IMMDevice) -> R<String> {
        unsafe {
            let p = d.GetId().map_err(err)?;
            let s = p.to_string().map_err(|e| e.to_string());
            CoTaskMemFree(Some(p.0 as *const _));
            s
        }
    }

    fn name_of(d: &IMMDevice) -> R<String> {
        unsafe {
            let store = d.OpenPropertyStore(STGM_READ).map_err(err)?;
            let v = store.GetValue(&PKEY_Device_FriendlyName).map_err(err)?;
            let p = PropVariantToStringAlloc(&v).map_err(err)?;
            let s = p.to_string().map_err(|e| e.to_string());
            CoTaskMemFree(Some(p.0 as *const _));
            s
        }
    }

    /// Active devices of one kind, in the enumerator's order. Our ids are 1-based positions (0 = none).
    fn devices(e: &IMMDeviceEnumerator, k: Kind) -> R<Vec<IMMDevice>> {
        unsafe {
            let c = e.EnumAudioEndpoints(flow(k), DEVICE_STATE_ACTIVE).map_err(err)?;
            (0..c.GetCount().map_err(err)?).map(|i| c.Item(i).map_err(err)).collect()
        }
    }

    fn default_device(e: &IMMDeviceEnumerator, k: Kind) -> Option<IMMDevice> {
        unsafe { e.GetDefaultAudioEndpoint(flow(k), eConsole).ok() }
    }

    fn volume(k: Kind) -> R<IAudioEndpointVolume> {
        let d = default_device(&enumerator()?, k).ok_or("no default device")?;
        unsafe { d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).map_err(err) }
    }

    pub fn side(k: Kind) -> Result<Side, String> {
        let e = enumerator()?;
        let list = devices(&e, k)?;
        let def = default_device(&e, k);
        let def_id = def.as_ref().and_then(|d| id_of(d).ok());
        let mut default = 0;
        let mut out = Vec::with_capacity(list.len());
        for (i, d) in list.iter().enumerate() {
            let id = i as u32 + 1;
            if def_id.is_some() && id_of(d).ok() == def_id {
                default = id;
            }
            out.push(Device { id, name: name_of(d).unwrap_or_else(|_| format!("Device {id}")) });
        }
        let muted = volume(k).ok().and_then(|v| unsafe { v.GetMute() }.ok()).is_some_and(|b| b.as_bool());
        // shortcut: "mic in use" needs IAudioSessionManager2 session states; only output playback is detected
        let in_use = matches!(k, Kind::Output)
            && def
                .and_then(|d| unsafe { d.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None) }.ok())
                .and_then(|m| unsafe { m.GetPeakValue() }.ok())
                .is_some_and(|p| p > PLAYING_PEAK);
        Ok(Side { devices: out, default, muted, in_use })
    }

    pub fn set_default(k: Kind, id: u32) -> Result<(), String> {
        let list = devices(&enumerator()?, k)?;
        let d = list.get((id as usize).wrapping_sub(1)).ok_or("that device is gone")?;
        let wide: Vec<u16> = id_of(d)?.encode_utf16().chain([0]).collect();
        unsafe {
            let policy: IUnknown = CoCreateInstance(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL).map_err(err)?;
            let policy: IPolicyConfig = policy.cast().map_err(err)?;
            for role in [eConsole, eMultimedia, eCommunications] {
                policy.set_default_endpoint(PCWSTR(wide.as_ptr()), role).ok().map_err(err)?;
            }
        }
        Ok(())
    }

    pub fn set_mute(k: Kind, muted: bool) -> Result<bool, String> {
        unsafe { volume(k)?.SetMute(muted, std::ptr::null()).map_err(err)? };
        Ok(muted)
    }

    pub fn output_level() -> Option<f32> {
        let v = volume(Kind::Output).ok()?;
        unsafe {
            if v.GetMute().ok()?.as_bool() {
                return Some(0.0);
            }
            v.GetMasterVolumeLevelScalar().ok()
        }
    }

    pub fn set_output_level(level: f32) {
        let Ok(v) = volume(Kind::Output) else { return };
        unsafe {
            if level > 0.0 && v.GetMute().is_ok_and(|b| b.as_bool()) {
                let _ = v.SetMute(false, std::ptr::null());
            }
            let _ = v.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null());
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    use super::*;
    pub fn side(_k: Kind) -> Result<Side, String> {
        Ok(Side { devices: vec![], default: 0, muted: false, in_use: false })
    }
    pub fn set_default(_k: Kind, _id: u32) -> Result<(), String> {
        Err("not implemented on this OS yet".into())
    }
    pub fn set_mute(_k: Kind, _m: bool) -> Result<bool, String> {
        Err("not implemented on this OS yet".into())
    }
    pub fn output_level() -> Option<f32> {
        None
    }
    pub fn set_output_level(_v: f32) {}
}

#[cfg(all(test, target_os = "macos"))]
mod live_tests {
    use super::*;

    /// Touches real hardware: `cargo test live_ -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_mic_mute_round_trip() {
        let before = audio_state().unwrap();
        let names: Vec<_> = before.input.devices.iter().map(|d| d.name.as_str()).collect();
        println!("inputs: {names:?} default={} muted={} mic_in_use={} audio_playing={} camera_in_use={}",
            before.input.default, before.input.muted, before.input.in_use, before.output.in_use, before.camera_in_use);
        assert!(!before.input.devices.is_empty());
        let flipped = audio_set_mute(Kind::Input, !before.input.muted).unwrap();
        assert_eq!(flipped, !before.input.muted);
        let restored = audio_set_mute(Kind::Input, before.input.muted).unwrap();
        assert_eq!(restored, before.input.muted);
    }
}

