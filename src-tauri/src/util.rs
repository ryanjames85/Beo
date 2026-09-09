use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use tauri::AppHandle;

/// Structured error for the two commands whose failure *kind* the frontend
/// actually needs to distinguish — `install_sdk` and `download_image`, both
/// long-running, cancellable, network-dependent operations with a Cancel
/// button and a progress bar. Everything else in this codebase stays
/// `Result<T, String>`: converting all ~28 commands to this would be a lot
/// of mechanical churn for cases where the frontend only ever displays the
/// error text verbatim and never branches on what kind it is.
///
/// Before this, the frontend told "cancelled" and "network failure" apart
/// by substring-matching the error *text* (`raw.includes("Cancelled")`,
/// a list of network-error phrases) — fragile against wording changes in
/// Rust's own error formatting or the underlying tools. `#[serde(tag =
/// "kind", content = "message")]` means a rejected `invoke()` promise for
/// these two commands resolves to `{kind: "Cancelled"}` or `{kind:
/// "Network", message: "..."}` in JS, so the frontend can match on
/// `error.kind` directly instead.
#[derive(Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum AppError {
    Cancelled,
    Network(String),
    Other(String),
}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError::Other(s)
    }
}

impl From<&str> for AppError {
    fn from(s: &str) -> Self {
        AppError::Other(s.to_string())
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        AppError::Network(e.to_string())
    }
}

/// Holds the currently-running sdkmanager child (if any) so the UI's
/// "Cancel" button can kill it, plus a flag the reader loop polls so
/// cancellation doesn't depend on the killed process's stdout pipe actually
/// closing — on Windows a killed process can leave that pipe held open
/// (see run_sdkmanager_streaming), which would otherwise hang the command
/// forever regardless of how many times the process was killed.
pub struct SdkTask {
    pub(crate) child: Mutex<Option<std::process::Child>>,
    pub(crate) cancelled: AtomicBool,
}

/// Persistent data root — survives app restarts, wiped on "nuke".
/// Windows: %APPDATA%/beo | macOS: ~/Library/Application Support/beo | Linux: ~/.local/share/beo
///
/// `BEO_DATA_DIR` overrides this when set — an isolation escape hatch for
/// the e2e test harness (`scripts/e2e-jdk-bootstrap.mjs`) so it can drive a
/// real, disposable instance without touching (or racing) the real
/// per-user install. Not documented as a user-facing setting.
pub(crate) fn data_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("BEO_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs::data_dir()
        .expect("no data dir on this platform")
        .join("beo")
}

pub(crate) fn sdk_root() -> PathBuf {
    data_root().join("sdk")
}

pub(crate) fn cmdline_tools_bin(exe: &str) -> PathBuf {
    let base = sdk_root().join("cmdline-tools").join("latest").join("bin");
    #[cfg(target_os = "windows")]
    return base.join(format!("{exe}.bat"));
    #[cfg(not(target_os = "windows"))]
    return base.join(exe);
}

pub(crate) fn emulator_bin() -> PathBuf {
    let base = sdk_root().join("emulator");
    #[cfg(target_os = "windows")]
    return base.join("emulator.exe");
    #[cfg(not(target_os = "windows"))]
    return base.join("emulator");
}

pub(crate) fn adb_bin() -> PathBuf {
    let base = sdk_root().join("platform-tools");
    #[cfg(target_os = "windows")]
    return base.join("adb.exe");
    #[cfg(not(target_os = "windows"))]
    return base.join("adb");
}

/// adb's default server port. Whichever SDK's `adb.exe` happens to start
/// the server first "wins" it — every other `adb.exe` on the machine
/// (Android Studio's, a bare platform-tools install, Beo's) becomes a
/// client of that *same* server regardless of which binary launched it,
/// since discovery is purely by port number. Mixing clients and a server
/// from different SDK releases like that risks protocol-version mismatches
/// and the server being silently killed/restarted out from under whichever
/// tool didn't start it. Beo runs its own server on a different port
/// instead (below), so it's never sharing one with anything else on the
/// machine.
const BEO_ADB_SERVER_PORT: &str = "5039";

/// Every spawn of an Android SDK tool (sdkmanager, avdmanager, emulator,
/// adb) must go through this instead of a bare `Command::new` — those
/// tools don't all resolve "which SDK am I part of" the same way. Scripts
/// like avdmanager.bat mostly infer it from their own file location, but
/// the compiled `emulator` binary resolves it primarily from the
/// ANDROID_SDK_ROOT/ANDROID_HOME environment variables. On a machine that
/// also has Android Studio installed — extremely common — those variables
/// are already set to *that* SDK, not Beo's. The emulator then can't find
/// the system image Beo just downloaded and crashes on startup with
/// `FATAL | Cannot find AVD system path`, silently, since launch_avd only
/// spawns and returns. Setting these explicitly on every child process
/// makes Beo's tools self-contained regardless of what else is installed.
/// ANDROID_ADB_SERVER_PORT gives Beo's whole toolchain (adb *and* the
/// emulator, which registers itself with whatever adb server it's told
/// about) a private adb server, per BEO_ADB_SERVER_PORT above. JAVA_HOME
/// (plus that JDK's bin/ prepended to PATH, since some launcher scripts
/// invoke `java` directly rather than trusting JAVA_HOME) points
/// sdkmanager.bat/avdmanager.bat at Beo's own bundled JDK instead of
/// whatever Java is or isn't already on the system — confirmed by hand
/// that sdkmanager.bat actually honors JAVA_HOME over any system java.
pub(crate) fn android_tool(path: PathBuf) -> Command {
    let root = sdk_root();
    let mut cmd = Command::new(path);
    cmd.env("ANDROID_SDK_ROOT", &root)
        .env("ANDROID_HOME", &root)
        .env("ANDROID_ADB_SERVER_PORT", BEO_ADB_SERVER_PORT);
    if let Some(java_home) = crate::jdk::jdk_home_dir() {
        cmd.env("JAVA_HOME", &java_home);
        let bin_dir = java_home.join("bin");
        let existing = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![bin_dir];
        dirs.extend(std::env::split_paths(&existing));
        if let Ok(joined) = std::env::join_paths(dirs) {
            cmd.env("PATH", joined);
        }
    }
    cmd
}

/// Hashes a downloaded file and compares it against a pinned expected
/// SHA-256, deleting the file and failing loudly on any mismatch rather
/// than extracting a corrupt or tampered archive. HTTPS guarantees the
/// bytes weren't altered in transit, not that they're actually the file
/// this URL was pinned against in the first place — this closes that gap.
pub(crate) fn verify_sha256(path: &std::path::Path, expected_hex: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = format!("{:x}", hasher.finalize());

    if !actual.eq_ignore_ascii_case(expected_hex) {
        let _ = std::fs::remove_file(path);
        return Err(format!(
            "Checksum mismatch — expected {expected_hex}, got {actual}. The download may be corrupt or tampered with; deleted it, please retry."
        ));
    }
    Ok(())
}

#[derive(Serialize, Deserialize, Clone)]
pub struct InstallProgress {
    pub(crate) stage: String,
    /// 0-100, or None for indeterminate stages (extracting, licenses, sdkmanager calls).
    pub(crate) percent: Option<f64>,
    pub(crate) detail: String,
}

pub(crate) fn emit_progress(app: &AppHandle, stage: &str, percent: Option<f64>, detail: &str) {
    use tauri::Emitter;
    let _ = app.emit(
        "sdk_install_progress",
        InstallProgress {
            stage: stage.into(),
            percent,
            detail: detail.into(),
        },
    );
}

/// Wipes the entire persistent SDK/system-image data dir *and* the actual
/// AVD devices — the "nuke" button.
///
/// avdmanager doesn't create devices under Beo's own data_root(); it uses
/// the standard Android tooling location, `~/.android/avd` (unless
/// ANDROID_AVD_HOME overrides it, which Beo doesn't set). So wiping only
/// data_root() leaves every created device behind on disk even though the
/// confirmation dialog promises "all devices" are deleted too.
#[tauri::command]
pub(crate) fn nuke_all() -> Result<String, String> {
    let root = data_root();
    if root.exists() {
        std::fs::remove_dir_all(&root).map_err(|e| e.to_string())?;
    }
    if let Some(home) = dirs::home_dir() {
        let avd_dir = home.join(".android").join("avd");
        if avd_dir.exists() {
            std::fs::remove_dir_all(&avd_dir).map_err(|e| e.to_string())?;
        }
    }
    Ok("Wiped all AVD Manager data".into())
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiskSpaceStatus {
    available_mb: Option<u64>,
}

/// Free disk space (best-effort) on the volume where AVDs actually get
/// created — `~/.android/avd` — falling back to the home directory itself
/// if that doesn't exist yet (a fresh install, before any device has ever
/// been created). Backs a soft warning shown before creating a new device,
/// not a hard gate: device sizes vary too widely for a precise "will this
/// fit" check — confirmed by hand, a device with a 6G declared data
/// partition was genuinely using 11G once a boot snapshot existed. `None`
/// if it can't be determined at all rather than guessing.
#[tauri::command]
pub(crate) fn check_disk_space() -> DiskSpaceStatus {
    let Some(home) = dirs::home_dir() else {
        return DiskSpaceStatus { available_mb: None };
    };
    let avd_dir = home.join(".android").join("avd");
    let check_path = if avd_dir.exists() { avd_dir } else { home };
    DiskSpaceStatus {
        available_mb: free_space_mb(&check_path),
    }
}

#[cfg(target_os = "windows")]
fn free_space_mb(path: &std::path::Path) -> Option<u64> {
    // Escaped for embedding in a PowerShell single-quoted string literal.
    let escaped = path.to_string_lossy().replace('\'', "''");
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "(New-Object System.IO.DriveInfo((Split-Path -Path '{escaped}' -Qualifier))).AvailableFreeSpace"
            ),
        ])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u64>()
        .ok()
        .map(|bytes| bytes / 1_048_576)
}

#[cfg(not(target_os = "windows"))]
fn free_space_mb(path: &std::path::Path) -> Option<u64> {
    // POSIX `df -Pk` output is one stable-width header line plus one data
    // line: "Filesystem 1024-blocks Used Available Capacity Mounted-on".
    // `-P` avoids some platforms' habit of wrapping long device names onto
    // their own line, which would otherwise shift the column split.
    let out = Command::new("df")
        .args(["-Pk", &path.to_string_lossy()])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let last_line = text.lines().last()?;
    let available_kb: u64 = last_line.split_whitespace().nth(3)?.parse().ok()?;
    Some(available_kb / 1024)
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::PathBuf;

    /// A unique, self-cleaning scratch directory per test — avoids touching
    /// the real `%APPDATA%/beo` and avoids inter-test races, since each test
    /// gets its own directory rather than sharing one via a mutated env var.
    pub(crate) struct ScratchDir(pub(crate) PathBuf);
    impl ScratchDir {
        pub(crate) fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "beo_test_{label}_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            ScratchDir(dir)
        }
    }
    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::ScratchDir;
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn data_root_honors_beo_data_dir_override() {
        // Serialized via a lock so this doesn't race the other tests in this
        // binary over the shared process-wide env var — none of the others
        // read BEO_DATA_DIR, but Rust's test harness runs tests in parallel
        // by default, so mutating process env at all needs care.
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap();

        let scratch = ScratchDir::new("data_root_override");
        std::env::set_var("BEO_DATA_DIR", &scratch.0);
        let result = data_root();
        std::env::remove_var("BEO_DATA_DIR");
        assert_eq!(result, scratch.0);
    }

    #[test]
    fn verify_sha256_accepts_matching_hash() {
        let scratch = ScratchDir::new("sha256_ok");
        let path = scratch.0.join("file.bin");
        std::fs::write(&path, b"hello beo").unwrap();
        // sha256("hello beo")
        let expected = "dacbcd62a75291103fff5b9d9ac68899c759bb64745beb757bc24b45588513fc";
        assert!(verify_sha256(&path, expected).is_ok());
        // Success must not delete the verified file.
        assert!(path.exists());
    }

    #[test]
    fn verify_sha256_rejects_mismatch_and_deletes_file() {
        let scratch = ScratchDir::new("sha256_bad");
        let path = scratch.0.join("file.bin");
        std::fs::write(&path, b"corrupted or tampered content").unwrap();
        let wrong = "0000000000000000000000000000000000000000000000000000000000000000";
        let result = verify_sha256(&path, wrong);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Checksum mismatch"));
        // A failed download shouldn't be left on disk looking installable.
        assert!(!path.exists());
    }
}
