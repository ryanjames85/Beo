use super::lifecycle::find_serial_for_avd;
use crate::util::{adb_bin, android_tool};

/// Pulls the rotation out of `dumpsys window`'s `Display{#0 ...
/// ROTATION_n}` line and converts it to a `Surface.ROTATION_*` index
/// (0-3). `n` here is a *degree* value (0/90/180/270 — confirmed live:
/// after one `adb emu rotate` from a fresh portrait boot, `dumpsys window`
/// reported `ROTATION_270`, not `ROTATION_1`), not the enum's own integer
/// value, so this has to parse the full number and divide by 90 rather
/// than read a single digit — a single-digit read only happens to work
/// for `ROTATION_0`, and silently misreads 90/180/270 (e.g. reading just
/// the leading "2" of "270" as rotation index 2, i.e. 180°). Caught by a
/// fixture test built from that same real captured "270" line — split out
/// from `current_rotation` so it can be unit tested without a live device.
fn parse_rotation(text: &str) -> Option<u8> {
    text.lines().find_map(|line| {
        let idx = line.find("Display{#0")?;
        let after = &line[idx..];
        let tag = "ROTATION_";
        let start = after.find(tag)? + tag.len();
        let digits: String = after[start..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let degrees: u32 = digits.parse().ok()?;
        Some(((degrees / 90) % 4) as u8)
    })
}

/// Reads the display's current rotation (0-3, `Surface.ROTATION_*`) out of
/// `dumpsys window`'s `Display{#0 ... ROTATION_n}` line.
fn current_rotation(serial: &str) -> Result<u8, String> {
    let out = android_tool(adb_bin())
        .args(["-s", serial, "shell", "dumpsys", "window"])
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_rotation(&text).ok_or_else(|| "Couldn't read the current display rotation".to_string())
}

/// Rotates a running device between portrait and landscape via the
/// emulator console's own `rotate` command (reached through `adb emu`) —
/// the same mechanism Android Studio's Extended Controls panel uses. This
/// simulates a real physical rotation via the virtual sensor, unlike
/// `settings put system user_rotation`, which real Android silently
/// overrides whenever the foreground activity has a fixed orientation (as
/// the launcher and most first-run/onboarding screens do) or auto-rotate
/// is on — confirmed live: the settings-only approach never visibly
/// rotated anything, while `emu rotate` does immediately. It only rotates
/// 90° clockwise per call (relative, not absolute), so this reads the
/// current rotation first and issues however many calls are needed to
/// reach the requested orientation.
#[tauri::command]
pub(crate) fn rotate_avd(name: String, orientation: String) -> Result<String, String> {
    let serial = find_serial_for_avd(&name)?;
    let target: u8 = if orientation == "landscape" { 1 } else { 0 };
    let current = current_rotation(&serial)?;
    let steps = (target as i32 - current as i32).rem_euclid(4);
    for _ in 0..steps {
        let out = android_tool(adb_bin())
            .args(["-s", &serial, "emu", "rotate"])
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).to_string());
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    Ok(format!("Rotated to {orientation}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `dumpsys window` fragments, captured live: the first while
    // verifying the rotate_avd fix (device still portrait), the second
    // moments later after issuing `adb emu rotate` (now landscape).
    #[test]
    fn parse_rotation_reads_portrait() {
        let text = "    Display{#0 state=ON size=1080x2400 ROTATION_0}:\n      more stuff here";
        assert_eq!(parse_rotation(text), Some(0));
    }

    #[test]
    fn parse_rotation_reads_landscape_after_rotating() {
        let text = "    Display{#0 state=ON size=2400x1080 ROTATION_270}:\n      more stuff here";
        assert_eq!(parse_rotation(text), Some(3));
    }

    #[test]
    fn parse_rotation_returns_none_without_a_display_line() {
        assert_eq!(parse_rotation("nothing relevant here\njust noise"), None);
    }
}
