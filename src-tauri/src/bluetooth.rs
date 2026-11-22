//! Battery levels of connected accessories (AirPods, mice, keyboards, trackpads).
//! macOS: `pmset -g accps` already lists every accessory that reports a battery, Apple or not.

use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Battery {
    pub name: String,
    pub percent: u8,
    pub charging: bool,
}

/// Accessories with a battery, lowest charge first.
#[tauri::command]
pub fn batteries() -> Result<Vec<Battery>, String> {
    imp::read().map(|raw| parse(&raw))
}

/// pmset writes names in MacRoman ("Alex’s" arrives as byte 0xD5); keep UTF-8 when it is UTF-8.
fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes
            .iter()
            .map(|&b| match b {
                0x00..=0x7f => b as char,
                0xd2 | 0xd3 => '"',
                0xd4 | 0xd5 => '’',
                _ => '?', // shortcut: only the quote marks Apple puts in device names, full MacRoman if other names look wrong
            })
            .collect(),
    }
}

/// Lines look like ` -MX Master 4 (id=35787760)\t65%; discharging present: true`.
fn parse(raw: &[u8]) -> Vec<Battery> {
    let text = decode(raw);
    let mut seen = Vec::new();
    let mut out: Vec<Battery> = text
        .lines()
        .filter_map(|line| {
            let line = line.trim_start().strip_prefix('-')?;
            let (head, rest) = line.split_once('\t')?;
            let (name, id) = head.rsplit_once(" (id=")?;
            if name.starts_with("InternalBattery") || seen.contains(&id.to_string()) {
                return None; // the Mac's own battery; a device listed twice under two names
            }
            seen.push(id.to_string());
            let (pct, status) = rest.split_once('%')?;
            let percent = pct.trim().parse::<u8>().ok()?.min(100);
            let charging = status.contains("charging") && !status.contains("discharging") || status.contains("charged");
            Some(Battery { name: name.trim().to_string(), percent, charging })
        })
        .collect();
    out.sort_by_key(|b| b.percent);
    out
}

#[cfg(target_os = "macos")]
mod imp {
    pub fn read() -> Result<Vec<u8>, String> {
        let out = std::process::Command::new("/usr/bin/pmset").args(["-g", "accps"]).output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err("pmset failed".into());
        }
        Ok(out.stdout)
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn read() -> Result<Vec<u8>, String> {
        // shortcut: Windows exposes accessory batteries only through per-device GATT/PnP queries; empty until needed
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_pmset_accessories_lowest_first() {
        let mut raw = b"Now drawing from 'AC Power'\n -InternalBattery-0 (id=23068771)\t100%; charged; 0:00 remaining present: true\n -Alex Doe".to_vec();
        raw.extend_from_slice(b"\xd5s AirPods Pro Case (id=44372984)\t94%; discharging present: true\n");
        raw.extend_from_slice(b" -MX Master 4 (id=35787760)\t15%; \n");
        raw.extend_from_slice(b" -Magic Keyboard (id=11)\t100%; charging; present: true\n");
        raw.extend_from_slice(b" -Magic Keyboard #2 (id=11)\t100%; charged; present: true\n");
        let b = parse(&raw);
        assert_eq!(b.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["MX Master 4", "Alex Doe’s AirPods Pro Case", "Magic Keyboard"]);
        assert_eq!(b[0].percent, 15);
        assert!(!b[0].charging && !b[1].charging && b[2].charging);
    }
}
