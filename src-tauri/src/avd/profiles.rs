use crate::util::{android_tool, cmdline_tools_bin};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct DeviceProfile {
    id: String,
    label: String,
    category: String, // "phone" or "tablet"
}

/// Same "does the device profile id look like a tablet" heuristic used by
/// `lifecycle::parse_avd_list`/`create_avd` — kept in sync with it
/// deliberately, since a profile picked here is what ends up there after
/// `create_avd`.
pub(super) fn category_for_device_id(device_id: &str) -> &'static str {
    if device_id.contains("tablet") || device_id.contains("pad") {
        "tablet"
    } else {
        "phone"
    }
}

/// Lists Android Studio's own built-in hardware profiles (Pixel phones,
/// tablets, Nexus devices, Wear, TV, etc.) via `avdmanager list device`,
/// so the picker matches what Android Studio itself offers rather than
/// a hardcoded subset.
#[tauri::command]
pub(crate) fn list_device_profiles() -> Result<Vec<DeviceProfile>, String> {
    let out = android_tool(cmdline_tools_bin("avdmanager"))
        .args(["list", "device", "-c"])
        .output()
        .map_err(|e| e.to_string())?;
    let ids = String::from_utf8_lossy(&out.stdout);

    let profiles = ids
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|id| !id.contains("wear") && !id.contains("tv") && !id.contains("automotive"))
        .map(|id| {
            let label = id
                .split('_')
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            DeviceProfile {
                category: category_for_device_id(&id).to_string(),
                id,
                label,
            }
        })
        .collect();
    Ok(profiles)
}
