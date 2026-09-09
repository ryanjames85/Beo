use crate::util::{emit_progress, verify_sha256, AppError, SdkTask};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};

pub(crate) fn jdk_root() -> PathBuf {
    crate::util::data_root().join("jdk")
}

/// `sdkmanager`/`avdmanager` need a JDK to run at all. Rather than depend
/// on whatever Java happens (or doesn't happen) to be on the system —
/// version and all — Beo downloads and manages its own, the same way it
/// does for the Android SDK itself. This finds it: the downloaded JRE
/// archive's top-level directory name embeds its exact version
/// (`jdk-21.0.5+11-jre`), so rather than hardcode that (and have to update
/// it every time JRE_URL is repinned), this just looks for the one
/// directory `jdk_root()` actually contains.
pub(crate) fn jdk_extracted_dir_at(root: &std::path::Path) -> Option<PathBuf> {
    std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.is_dir())
}

/// The JRE's own home directory — where `bin/`, `lib/`, etc. live. Confirmed
/// by hand: on Windows/Linux this is the extracted directory itself; macOS
/// JDK/JRE builds nest an extra `Contents/Home` (the standard macOS bundle
/// layout), even in the plain tar.gz distribution.
pub(crate) fn jdk_home_dir_at(root: &std::path::Path) -> Option<PathBuf> {
    let base = jdk_extracted_dir_at(root)?;
    #[cfg(target_os = "macos")]
    return Some(base.join("Contents").join("Home"));
    #[cfg(not(target_os = "macos"))]
    Some(base)
}

pub(crate) fn jdk_home_dir() -> Option<PathBuf> {
    jdk_home_dir_at(&jdk_root())
}

pub(crate) fn java_bin_at(root: &std::path::Path) -> Option<PathBuf> {
    let home = jdk_home_dir_at(root)?;
    #[cfg(target_os = "windows")]
    let bin = home.join("bin").join("java.exe");
    #[cfg(not(target_os = "windows"))]
    let bin = home.join("bin").join("java");
    bin.exists().then_some(bin)
}

pub(crate) fn java_bin() -> Option<PathBuf> {
    java_bin_at(&jdk_root())
}

// Eclipse Temurin's redistributable JRE builds — GPLv2 with Classpath
// Exception, freely bundleable. A JRE (not a full JDK) is enough since
// sdkmanager/avdmanager only need to *run* on Java, not compile anything.
// Verified by hand before wiring this in: downloaded, extracted, and ran
// `sdkmanager.bat --version` against this exact build with JAVA_HOME
// pointed at it. Update the version/URLs *and* JRE_SHA256 together to
// repin — see https://github.com/adoptium/temurin21-binaries/releases (each
// asset there has a `.sha256.txt` sidecar; the hash below is copied
// straight from it, cross-checked once against a real downloaded file's
// own computed hash rather than trusted blind).
#[cfg(target_os = "windows")]
const JRE_URL: &str = "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.5%2B11/OpenJDK21U-jre_x64_windows_hotspot_21.0.5_11.zip";
#[cfg(target_os = "windows")]
const JRE_SHA256: &str = "1749b36cfac273cee11802bf3e90caada5062de6a3fef1a3814c0568b25fd654";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const JRE_URL: &str = "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.5%2B11/OpenJDK21U-jre_x64_mac_hotspot_21.0.5_11.tar.gz";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const JRE_SHA256: &str = "0e0dcb571f7bf7786c111fe066932066d9eab080c9f86d8178da3e564324ee81";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const JRE_URL: &str = "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.5%2B11/OpenJDK21U-jre_aarch64_mac_hotspot_21.0.5_11.tar.gz";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const JRE_SHA256: &str = "12249a1c5386957c93fc372260c483ae921b1ec6248a5136725eabd0abc07f93";
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const JRE_URL: &str = "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.5%2B11/OpenJDK21U-jre_x64_linux_hotspot_21.0.5_11.tar.gz";
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const JRE_SHA256: &str = "553dda64b3b1c3c16f8afe402377ffebe64fb4a1721a46ed426a91fd18185e62";
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const JRE_URL: &str = "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.5%2B11/OpenJDK21U-jre_aarch64_linux_hotspot_21.0.5_11.tar.gz";
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const JRE_SHA256: &str = "e4d02c33aeaf8e1148c1c505e129a709c5bc1889e855d4fb4f001b1780db42b4";

/// Downloads and extracts Beo's own JDK, if it isn't already present.
/// Shared by `install_sdk` (which needs Java to exist before it can even
/// run `sdkmanager --licenses`) and reachable on its own if the JDK ever
/// needs reinstalling without redoing the whole SDK.
pub(crate) async fn ensure_jdk(app: &AppHandle) -> Result<(), AppError> {
    if java_bin().is_some() {
        return Ok(());
    }
    let root = jdk_root();
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;

    // Same gap as install_sdk's cmdline-tools download had: the Cancel
    // button is visible throughout the whole install, but this raw
    // download never checked `task.cancelled` — clicking Cancel while the
    // JDK was downloading did nothing.
    let task = app.state::<SdkTask>();
    task.cancelled.store(false, Ordering::SeqCst);

    emit_progress(app, "jdk", Some(0.0), "Starting JDK download…");
    let resp = reqwest::get(JRE_URL).await?;
    let total = resp.content_length();

    let is_zip = JRE_URL.ends_with(".zip");
    let archive_path = root.join(if is_zip { "jre.zip" } else { "jre.tar.gz" });
    let mut file = std::fs::File::create(&archive_path).map_err(|e| e.to_string())?;

    use futures_util::StreamExt;
    use std::io::Write;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    while let Some(chunk) = stream.next().await {
        if task.cancelled.swap(false, Ordering::SeqCst) {
            drop(file);
            let _ = std::fs::remove_file(&archive_path);
            return Err(AppError::Cancelled);
        }
        let chunk = chunk?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        let percent = total.map(|t| (downloaded as f64 / t as f64) * 100.0);
        let mb = downloaded as f64 / 1_048_576.0;
        let detail = match total {
            Some(t) => format!("{:.1} MB / {:.1} MB", mb, t as f64 / 1_048_576.0),
            None => format!("{:.1} MB downloaded", mb),
        };
        emit_progress(app, "jdk", percent, &detail);
    }
    drop(file);

    emit_progress(app, "jdk", Some(100.0), "Verifying download…");
    verify_sha256(&archive_path, JRE_SHA256)?;

    emit_progress(app, "extracting_jdk", None, "Extracting JDK…");
    if is_zip {
        let f = std::fs::File::open(&archive_path).map_err(|e| e.to_string())?;
        let mut archive = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
        archive.extract(&root).map_err(|e| e.to_string())?;
    } else {
        let f = std::fs::File::open(&archive_path).map_err(|e| e.to_string())?;
        let gz = flate2::read::GzDecoder::new(f);
        let mut archive = tar::Archive::new(gz);
        archive.unpack(&root).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_file(&archive_path);

    if java_bin().is_none() {
        return Err("JDK extracted but the java binary wasn't found where expected".into());
    }
    Ok(())
}

#[derive(Serialize, Deserialize, Clone)]
pub struct JavaStatus {
    available: bool,
    detail: String,
}

/// Reports on *Beo's own* bundled JDK, not whatever may or may not be on
/// the system — Beo downloads and manages its own (see `ensure_jdk`)
/// specifically so it never depends on, or conflicts with, a system-wide
/// Java install. Before the SDK is installed there's no JDK yet, which is
/// expected and not an error: `install_sdk` fetches one automatically.
#[tauri::command]
pub(crate) fn check_java() -> JavaStatus {
    match java_bin() {
        Some(bin) => match Command::new(&bin).arg("-version").output() {
            Ok(out) => {
                // Every JDK, oddly, prints `-version` output to stderr, not stdout.
                let text = String::from_utf8_lossy(&out.stderr);
                let first_line = text.lines().next().unwrap_or("").trim();
                JavaStatus {
                    available: true,
                    detail: if first_line.is_empty() {
                        "Beo's own JDK is installed".into()
                    } else {
                        first_line.to_string()
                    },
                }
            }
            Err(e) => JavaStatus {
                available: false,
                detail: format!(
                    "Beo's JDK is present but couldn't be run ({e}) — try reinstalling the SDK."
                ),
            },
        },
        None => JavaStatus {
            available: false,
            detail:
                "No JDK yet — Beo downloads its own automatically as part of installing the SDK."
                    .into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::ScratchDir;

    #[cfg(target_os = "windows")]
    const JAVA_BIN_NAME: &str = "java.exe";
    #[cfg(not(target_os = "windows"))]
    const JAVA_BIN_NAME: &str = "java";

    #[test]
    fn jdk_extracted_dir_returns_none_when_root_is_empty() {
        let scratch = ScratchDir::new("jdk_empty");
        assert_eq!(jdk_extracted_dir_at(&scratch.0), None);
    }

    #[test]
    fn jdk_extracted_dir_returns_none_when_root_is_missing() {
        let missing = std::env::temp_dir().join("beo_test_definitely_does_not_exist_xyz");
        assert_eq!(jdk_extracted_dir_at(&missing), None);
    }

    #[test]
    fn jdk_extracted_dir_finds_the_versioned_subdirectory() {
        let scratch = ScratchDir::new("jdk_versioned");
        let versioned = scratch.0.join("jdk-21.0.5+11-jre");
        std::fs::create_dir_all(&versioned).unwrap();
        // A loose file alongside it (e.g. a leftover archive) must not be
        // mistaken for the extracted JDK directory.
        std::fs::write(scratch.0.join("jre.zip"), b"not a real archive").unwrap();
        assert_eq!(jdk_extracted_dir_at(&scratch.0), Some(versioned));
    }

    #[test]
    fn jdk_home_dir_matches_platform_bundle_layout() {
        let scratch = ScratchDir::new("jdk_home");
        let versioned = scratch.0.join("jdk-21.0.5+11-jre");
        std::fs::create_dir_all(&versioned).unwrap();

        let home = jdk_home_dir_at(&scratch.0).unwrap();
        #[cfg(target_os = "macos")]
        assert_eq!(home, versioned.join("Contents").join("Home"));
        #[cfg(not(target_os = "macos"))]
        assert_eq!(home, versioned);
    }

    #[test]
    fn java_bin_is_none_when_jdk_not_extracted_yet() {
        let scratch = ScratchDir::new("java_bin_missing");
        assert_eq!(java_bin_at(&scratch.0), None);
    }

    #[test]
    fn java_bin_is_none_when_extracted_but_binary_absent() {
        // A partially-extracted or corrupt JDK shouldn't be reported as usable.
        let scratch = ScratchDir::new("java_bin_partial");
        std::fs::create_dir_all(scratch.0.join("jdk-21.0.5+11-jre").join("bin")).unwrap();
        assert_eq!(java_bin_at(&scratch.0), None);
    }

    #[test]
    fn java_bin_resolves_once_the_binary_exists() {
        let scratch = ScratchDir::new("java_bin_present");
        #[cfg(target_os = "macos")]
        let bin_dir = scratch
            .0
            .join("jdk-21.0.5+11-jre")
            .join("Contents")
            .join("Home")
            .join("bin");
        #[cfg(not(target_os = "macos"))]
        let bin_dir = scratch.0.join("jdk-21.0.5+11-jre").join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let expected = bin_dir.join(JAVA_BIN_NAME);
        std::fs::write(&expected, b"#!/bin/sh\necho fake java").unwrap();
        assert_eq!(java_bin_at(&scratch.0), Some(expected));
    }
}
