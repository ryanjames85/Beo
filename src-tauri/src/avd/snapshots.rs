use super::lifecycle::find_serial_for_avd;
use super::naming::sanitize_avd_name;
use crate::util::{adb_bin, android_tool};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SnapshotInfo {
    name: String,
    size: String,
    date: String,
}

/// Lists snapshots via `adb emu avd snapshot list`, which prints a
/// fixed-width text table (confirmed by hand: "ID   TAG   VM SIZE   DATE
/// VM CLOCK", terminated by "OK") rather than anything structured — this
/// parses that table by position rather than assuming exact column widths,
/// since sdkmanager/emulator table output isn't a stable contract across
/// versions (see list_available_images for the same concern elsewhere).
/// Split out from `list_snapshots` so it can be unit tested against a real
/// captured `emu avd snapshot list` dump without needing a live device.
fn parse_snapshot_list(text: &str) -> Vec<SnapshotInfo> {
    text.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // Real rows look like: "--  mysnap  1.4M  2026-09-04  21:34:45  00:10:07.753"
            // — at least ID, TAG, SIZE, DATE, TIME. Header/footer/blank
            // lines won't have that shape (header's ID column literally
            // reads "ID", which parts[0] == "--" filters out).
            if parts.len() < 5 || parts[0] != "--" {
                return None;
            }
            Some(SnapshotInfo {
                name: parts[1].to_string(),
                size: parts[2].to_string(),
                date: format!("{} {}", parts[3], parts[4]),
            })
        })
        .collect()
}

#[tauri::command]
pub(crate) fn list_snapshots(name: String) -> Result<Vec<SnapshotInfo>, String> {
    let serial = find_serial_for_avd(&name)?;
    let out = android_tool(adb_bin())
        .args(["-s", &serial, "emu", "avd", "snapshot", "list"])
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(parse_snapshot_list(&text))
}

#[tauri::command]
pub(crate) fn save_snapshot(name: String, snapshot_name: String) -> Result<String, String> {
    let serial = find_serial_for_avd(&name)?;
    let safe = sanitize_avd_name(&snapshot_name)?;
    let out = android_tool(adb_bin())
        .args(["-s", &serial, "emu", "avd", "snapshot", "save", &safe])
        .output()
        .map_err(|e| e.to_string())?;
    if !String::from_utf8_lossy(&out.stdout).contains("OK") {
        return Err(String::from_utf8_lossy(&out.stdout).to_string());
    }
    Ok(format!("Saved snapshot \"{safe}\""))
}

#[tauri::command]
pub(crate) fn load_snapshot(name: String, snapshot_name: String) -> Result<String, String> {
    let serial = find_serial_for_avd(&name)?;
    let out = android_tool(adb_bin())
        .args([
            "-s",
            &serial,
            "emu",
            "avd",
            "snapshot",
            "load",
            &snapshot_name,
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !String::from_utf8_lossy(&out.stdout).contains("OK") {
        return Err(String::from_utf8_lossy(&out.stdout).to_string());
    }
    Ok(format!("Loaded snapshot \"{snapshot_name}\""))
}

#[tauri::command]
pub(crate) fn delete_snapshot(name: String, snapshot_name: String) -> Result<String, String> {
    let serial = find_serial_for_avd(&name)?;
    let out = android_tool(adb_bin())
        .args([
            "-s",
            &serial,
            "emu",
            "avd",
            "snapshot",
            "delete",
            &snapshot_name,
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !String::from_utf8_lossy(&out.stdout).contains("OK") {
        return Err(String::from_utf8_lossy(&out.stdout).to_string());
    }
    Ok(format!("Deleted snapshot \"{snapshot_name}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `adb emu avd snapshot list` output, captured live: first with
    // only the emulator's own auto-created `default_boot` snapshot, then
    // again after saving one through Beo (`fixture_test`) — confirming the
    // parser handles more than one real row, not just a single-row sample.
    const REAL_SNAPSHOT_LIST_EMPTY: &str = "List of snapshots present on all disks:\nID        TAG                 VM SIZE                DATE       VM CLOCK\n--        default_boot            75M 2026-09-09 11:41:45   00:10:07.753\nOK\n";
    const REAL_SNAPSHOT_LIST_WITH_SAVE: &str = "List of snapshots present on all disks:\nID        TAG                 VM SIZE                DATE       VM CLOCK\n--        default_boot            75M 2026-09-09 11:41:45   00:10:07.753\n--        fixture_test            75M 2026-09-09 16:55:38   00:10:45.114\nOK\n";

    #[test]
    fn parse_snapshot_list_reads_the_auto_created_boot_snapshot() {
        assert_eq!(
            parse_snapshot_list(REAL_SNAPSHOT_LIST_EMPTY),
            vec![SnapshotInfo {
                name: "default_boot".into(),
                size: "75M".into(),
                date: "2026-09-09 11:41:45".into(),
            }]
        );
    }

    #[test]
    fn parse_snapshot_list_reads_multiple_rows_after_a_save() {
        assert_eq!(
            parse_snapshot_list(REAL_SNAPSHOT_LIST_WITH_SAVE),
            vec![
                SnapshotInfo {
                    name: "default_boot".into(),
                    size: "75M".into(),
                    date: "2026-09-09 11:41:45".into(),
                },
                SnapshotInfo {
                    name: "fixture_test".into(),
                    size: "75M".into(),
                    date: "2026-09-09 16:55:38".into(),
                },
            ]
        );
    }
}
