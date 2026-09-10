use crate::sdk::sdk_path;
use serde::{Deserialize, Serialize};
#[cfg(not(target_os = "windows"))]
use std::path::PathBuf;
use std::process::Command;

// camelCase here specifically: this is the one struct in this file with
// multi-word fields, and the frontend (Settings.tsx's IdeStatus type) reads
// `sdkPath`/`shellProfile` — without this, serde's default (exact Rust
// field names) sends `sdk_path`/`shell_profile` instead, and those frontend
// reads silently resolve to `undefined` since JS just doesn't have the key
// under the name it's looking for. Confirmed live: the Settings page's SDK
// path field was stuck on "…" forever, and Copy would copy "undefined".
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IdeIntegrationStatus {
    enabled: bool,
    sdk_path: String,
    shell_profile: Option<String>,
}

// Windows doesn't use shell profiles the same way — ANDROID_HOME there is
// set via setx, handled separately in enable_ide_integration — so this
// whole approach (and the marker block it looks for) is only ever reached
// on macOS/Linux. Gated accordingly rather than left as dead code that
// happens not to be called on Windows: CI now compiles this file on all
// three platforms (see .github/workflows/ci.yml), and an ungated version
// showed up there as an unused-function/unused-variable warning on Windows
// specifically, since nothing on that platform ever calls it.
#[cfg(not(target_os = "windows"))]
fn shell_profile_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    // Prefer .zshrc if present (macOS default since Catalina), else .bashrc.
    let zshrc = home.join(".zshrc");
    if zshrc.exists() {
        return Some(zshrc);
    }
    Some(home.join(".bashrc"))
}

#[cfg(not(target_os = "windows"))]
const MARKER_START: &str = "# >>> beo android sdk >>>";
#[cfg(not(target_os = "windows"))]
const MARKER_END: &str = "# <<< beo android sdk <<<";

/// Reads a User-scope (HKCU) environment variable via PowerShell. This
/// needs no elevation — only *system*-scope (`setx /M`) or the deprecated
/// `HKLM\...\Environment` registry key requires admin rights, and Beo never
/// touches those.
#[cfg(target_os = "windows")]
fn get_user_env_var(name: &str) -> Option<String> {
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("[Environment]::GetEnvironmentVariable('{name}','User')"),
        ])
        .output()
        .ok()?;
    let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// Checks whether Beo's ANDROID_HOME is already set — via a shell profile
/// marker block on macOS/Linux, or the ANDROID_HOME user environment
/// variable actually pointing at Beo's own SDK on Windows (not just *some*
/// ANDROID_HOME being set — that could belong to a different SDK install
/// entirely, e.g. Android Studio's, which is exactly what enable/disable
/// need to be careful not to clobber).
#[tauri::command]
pub(crate) fn ide_integration_status() -> IdeIntegrationStatus {
    let path = sdk_path();

    #[cfg(target_os = "windows")]
    {
        let enabled = get_user_env_var("ANDROID_HOME").as_deref() == Some(path.as_str());
        IdeIntegrationStatus {
            enabled,
            sdk_path: path,
            shell_profile: None,
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let profile = shell_profile_path();
        let enabled = profile
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|contents| contents.contains(MARKER_START))
            .unwrap_or(false);
        IdeIntegrationStatus {
            enabled,
            sdk_path: path,
            shell_profile: profile.map(|p| p.to_string_lossy().to_string()),
        }
    }
}

/// Appends ANDROID_HOME / ANDROID_SDK_ROOT exports (plus platform-tools on
/// PATH) to the user's shell profile, so any IDE or terminal launched
/// afterward can find this SDK without a separate download. Idempotent —
/// re-running just confirms the block is present.
#[tauri::command]
pub(crate) fn enable_ide_integration() -> Result<String, String> {
    let path = sdk_path();

    #[cfg(target_os = "windows")]
    {
        // setx persists to the user environment for future processes/IDEs.
        Command::new("setx")
            .args(["ANDROID_HOME", &path])
            .output()
            .map_err(|e| e.to_string())?;
        Command::new("setx")
            .args(["ANDROID_SDK_ROOT", &path])
            .output()
            .map_err(|e| e.to_string())?;
        Ok("ANDROID_HOME set. Restart any open IDE or terminal for it to take effect.".into())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let profile = shell_profile_path().ok_or("Couldn't locate a shell profile to edit")?;
        let existing = std::fs::read_to_string(&profile).unwrap_or_default();
        if existing.contains(MARKER_START) {
            return Ok("Already configured.".into());
        }
        let block = format!(
            "\n{MARKER_START}\nexport ANDROID_HOME=\"{path}\"\nexport ANDROID_SDK_ROOT=\"{path}\"\nexport PATH=\"$PATH:$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator\"\n{MARKER_END}\n"
        );
        std::fs::write(&profile, existing + &block).map_err(|e| e.to_string())?;
        Ok(format!(
            "Added to {}. Restart your terminal or IDE for it to take effect.",
            profile.display()
        ))
    }
}

/// Removes the ANDROID_HOME block Beo added, if any.
#[tauri::command]
pub(crate) fn disable_ide_integration() -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        // Clearing a User-scope env var needs no elevation either — same as
        // reading it. But only clear it if it currently points at Beo's own
        // SDK: some other tool (Android Studio, in one real case found
        // while building this) may have set ANDROID_HOME to a *different*
        // SDK, and blindly clearing it would break that tool's setup for a
        // variable Beo never actually set.
        let path = sdk_path();
        let mut cleared_any = false;
        for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
            if get_user_env_var(var).as_deref() == Some(path.as_str()) {
                Command::new("powershell")
                    .args([
                        "-NoProfile",
                        "-Command",
                        &format!("[Environment]::SetEnvironmentVariable('{var}', $null, 'User')"),
                    ])
                    .output()
                    .map_err(|e| e.to_string())?;
                cleared_any = true;
            }
        }
        Ok(if cleared_any {
            "Removed. Restart any open IDE or terminal for it to take effect.".into()
        } else {
            "Nothing to remove — ANDROID_HOME wasn't pointing at Beo's SDK.".into()
        })
    }

    #[cfg(not(target_os = "windows"))]
    {
        let profile = shell_profile_path().ok_or("Couldn't locate a shell profile to edit")?;
        let existing = std::fs::read_to_string(&profile).unwrap_or_default();
        if let (Some(start), Some(end)) = (existing.find(MARKER_START), existing.find(MARKER_END)) {
            let end = end + MARKER_END.len();
            let mut new_contents = existing[..start].to_string();
            new_contents.push_str(&existing[end..]);
            std::fs::write(&profile, new_contents).map_err(|e| e.to_string())?;
            return Ok("Removed.".into());
        }
        Ok("Nothing to remove.".into())
    }
}

/// Best-effort detection of which IDEs are actually running and likely to
/// be using this SDK — adb doesn't expose "who's connected" directly, so
/// this checks host processes for known IDE executables instead. Honest
/// limitation: this shows an IDE is *running*, not that it's definitely
/// pointed at Beo's SDK specifically (if the person has another SDK
/// install too, we can't tell which one the IDE picked without reading
/// its config files).
#[tauri::command]
pub(crate) fn detect_connected_ides() -> Vec<String> {
    let mut found = Vec::new();

    #[cfg(target_os = "windows")]
    let (cmd, args): (&str, Vec<&str>) = ("tasklist", vec![]);
    #[cfg(not(target_os = "windows"))]
    let (cmd, args): (&str, Vec<&str>) = ("ps", vec!["aux"]);

    let out = Command::new(cmd).args(&args).output();
    let text = out
        .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase())
        .unwrap_or_default();

    if text.contains("studio64") || text.contains("android studio") || text.contains("studio.sh") {
        found.push("Android Studio".to_string());
    }
    if text.contains("code.exe")
        || text.contains("code helper")
        || text.contains("/code ")
        || text.contains("visual studio code")
    {
        found.push("VS Code".to_string());
    }
    // "idea64.exe" confirmed live on Windows (JetBrains Toolbox and the
    // standalone installer both use this name); JetBrains' other IDEs
    // (WebStorm, PyCharm, ...) use their own similarly-named binaries and
    // aren't covered here — this is specifically IntelliJ IDEA.
    if text.contains("idea64.exe") || text.contains("idea.exe") || text.contains("intellij idea") {
        found.push("IntelliJ IDEA".to_string());
    }
    // "zed.exe" confirmed live on Windows; macOS/Linux builds run as a bare
    // "zed" process.
    if text.contains("zed.exe") || text.contains("zed.app") || text.contains("/zed ") {
        found.push("Zed".to_string());
    }
    // Cursor and Windsurf are VS Code forks with their own process names —
    // not confirmed live on this machine (neither installed here), based
    // on their known standard binary names instead.
    if text.contains("cursor.exe") || text.contains("cursor helper") || text.contains("/cursor ") {
        found.push("Cursor".to_string());
    }
    if text.contains("windsurf.exe")
        || text.contains("windsurf helper")
        || text.contains("/windsurf ")
    {
        found.push("Windsurf".to_string());
    }
    // Same JetBrains platform as IntelliJ IDEA but separate binaries — not
    // confirmed live here either.
    if text.contains("webstorm64.exe") || text.contains("webstorm.exe") || text.contains("webstorm")
    {
        found.push("WebStorm".to_string());
    }
    if text.contains("pycharm64.exe") || text.contains("pycharm.exe") || text.contains("pycharm") {
        found.push("PyCharm".to_string());
    }
    // GUI Neovim front-ends (neovim-qt, Neovide) and a bare terminal
    // `nvim.exe` both show up under this name — not confirmed live here.
    if text.contains("nvim.exe") || text.contains("neovide.exe") || text.contains("nvim-qt.exe") {
        found.push("Neovim".to_string());
    }
    // The OG. Not confirmed live here (not installed on this machine), but
    // "notepad++.exe" has been its process name since forever.
    if text.contains("notepad++.exe") {
        found.push("Notepad++".to_string());
    }
    // Google's VS Code fork. Not confirmed live here (not installed on
    // this machine) — guessing at its process name by the same convention
    // as Cursor/Windsurf until it can be checked against a real install.
    if text.contains("antigravity.exe") || text.contains("/antigravity ") {
        found.push("Antigravity".to_string());
    }

    found
}
