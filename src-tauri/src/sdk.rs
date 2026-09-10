use crate::jdk::ensure_jdk;
use crate::util::{
    android_tool, cmdline_tools_bin, emit_progress, sdk_root, verify_sha256, AppError, SdkTask,
};
use serde::{Deserialize, Serialize};
#[cfg(any(target_os = "windows", target_os = "macos"))]
use std::process::Command;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager, State};

// Official Google-hosted command-line tools. Update version/URLs as Google
// revises them: https://developer.android.com/studio#command-line-tools-only
// — that page only ever shows a checksum for the *current* "latest" build,
// not this specific pinned one, so CMDLINE_TOOLS_SHA256 below isn't copied
// from there; it's the SHA-256 of this exact file, computed directly from a
// real download the day this was pinned (trust-on-first-use, the same as
// any lockfile hash). Update both together when repinning to a new build.
#[cfg(target_os = "windows")]
const CMDLINE_TOOLS_URL: &str =
    "https://dl.google.com/android/repository/commandlinetools-win-11076708_latest.zip";
#[cfg(target_os = "windows")]
const CMDLINE_TOOLS_SHA256: &str =
    "4d6931209eebb1bfb7c7e8b240a6a3cb3ab24479ea294f3539429574b1eec862";
#[cfg(target_os = "macos")]
const CMDLINE_TOOLS_URL: &str =
    "https://dl.google.com/android/repository/commandlinetools-mac-11076708_latest.zip";
#[cfg(target_os = "macos")]
const CMDLINE_TOOLS_SHA256: &str =
    "7bc5c72ba0275c80a8f19684fb92793b83a6b5c94d4d179fc5988930282d7e64";
#[cfg(target_os = "linux")]
const CMDLINE_TOOLS_URL: &str =
    "https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip";
#[cfg(target_os = "linux")]
const CMDLINE_TOOLS_SHA256: &str =
    "2d2d50857e4eb553af5a6dc3ad507a17adf43d115264b1afc116f95c92e5e258";

/// Returns the absolute SDK root path as a string — this is what an
/// external IDE (Android Studio, VS Code + extensions) needs to point at
/// via ANDROID_HOME / ANDROID_SDK_ROOT to reuse Beo's install instead of
/// downloading its own copy.
#[tauri::command]
pub(crate) fn sdk_path() -> String {
    sdk_root().to_string_lossy().to_string()
}

#[tauri::command]
pub(crate) fn sdk_status() -> bool {
    cmdline_tools_bin("sdkmanager").exists()
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AccelStatus {
    available: bool,
    backend: String, // "KVM", "Hypervisor.Framework", "WHPX", "HAXM", "none"
    detail: String,
}

/// Checks whether the host can hardware-accelerate the emulator.
/// This doesn't touch the emulator's own detection — it's a fast pre-flight
/// check so Beo can warn the user *before* they hit a silent slow boot.
#[tauri::command]
pub(crate) fn check_hardware_accel() -> AccelStatus {
    #[cfg(target_os = "linux")]
    {
        let kvm_ok = std::path::Path::new("/dev/kvm").exists();
        if kvm_ok {
            // Confirm it's actually usable, not just present with wrong perms.
            let readable = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/kvm")
                .is_ok();
            if readable {
                return AccelStatus {
                    available: true,
                    backend: "KVM".into(),
                    detail: "/dev/kvm present and accessible".into(),
                };
            }
            return AccelStatus {
                available: false,
                backend: "KVM".into(),
                detail: "/dev/kvm exists but isn't accessible — add your user to the 'kvm' group and re-login".into(),
            };
        }
        return AccelStatus {
            available: false,
            backend: "none".into(),
            detail: "/dev/kvm not found — check that virtualization is enabled in BIOS and the kvm kernel module is loaded".into(),
        };
    }

    #[cfg(target_os = "macos")]
    {
        // Apple Silicon and modern Intel Macs ship Hypervisor.Framework;
        // sysctl kern.hv_support reports availability.
        let out = Command::new("sysctl")
            .arg("-n")
            .arg("kern.hv_support")
            .output();
        let ok = out
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
            .unwrap_or(false);
        return AccelStatus {
            available: ok,
            backend: "Hypervisor.Framework".into(),
            detail: if ok {
                "Hypervisor.Framework available".into()
            } else {
                "Hypervisor.Framework unavailable on this Mac".into()
            },
        };
    }

    #[cfg(target_os = "windows")]
    {
        // `Get-WindowsOptionalFeature -Online` — what this used to call —
        // requires admin elevation to run at all. Beo doesn't run elevated
        // (nothing about it should need to), so that command always failed
        // here, and since Command::output() only errors when the process
        // fails to *launch* (not when it exits with an error), the failure
        // was silently read as empty output — reporting "not enabled" for
        // every non-elevated user regardless of their actual hardware,
        // confirmed hands-on: this reported off on a machine where WHPX was
        // independently confirmed operational.
        //
        // Win32_ComputerSystem.HypervisorPresent reports whether a
        // hypervisor is actually active on this boot — the real thing WHPX
        // needs — and needs no elevation at all.
        let out = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "(Get-CimInstance Win32_ComputerSystem).HypervisorPresent",
            ])
            .output();
        let enabled = out
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .eq_ignore_ascii_case("true")
            })
            .unwrap_or(false);
        return AccelStatus {
            available: enabled,
            backend: "WHPX".into(),
            detail: if enabled {
                "A hypervisor is active — WHPX should be usable".into()
            } else {
                "No hypervisor detected — turn on 'Windows Hypervisor Platform' in Windows Features (or install Intel HAXM), then restart".into()
            },
        };
    }

    #[allow(unreachable_code)]
    AccelStatus {
        available: false,
        backend: "unknown".into(),
        detail: "Could not determine acceleration support on this platform".into(),
    }
}

/// Opens Windows' "Turn Windows features on or off" dialog directly, so the
/// user doesn't have to hunt through Control Panel/Settings to find it after
/// the accel warning tells them Hypervisor Platform is off. Just opening
/// this dialog doesn't need admin rights — Windows only prompts for
/// elevation once the user actually checks a box and clicks OK inside it.
#[tauri::command]
pub(crate) fn open_windows_features() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("optionalfeatures.exe")
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Only available on Windows".into())
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct NetworkStatus {
    online: bool,
    detail: String,
}

/// Real reachability check against the same host the SDK/system-image
/// downloads use (dl.google.com), not just "is a network interface up" —
/// a machine can have a default route and still be unable to reach Google's
/// servers (captive portal, DNS failure, corporate proxy blocking it, a
/// sandboxed environment with restricted egress). A short timeout keeps
/// this from hanging the UI on a dead connection.
#[tauri::command]
pub(crate) async fn check_network() -> NetworkStatus {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return NetworkStatus {
                online: false,
                detail: format!("Couldn't set up a network check: {e}"),
            }
        }
    };
    match client
        .head("https://dl.google.com/generate_204")
        .send()
        .await
    {
        Ok(_) => NetworkStatus {
            online: true,
            detail: "Connected".into(),
        },
        Err(e) => NetworkStatus {
            online: false,
            detail: if e.is_timeout() {
                "No response from dl.google.com within 5s — check your internet connection.".into()
            } else {
                format!("Can't reach dl.google.com: {e}")
            },
        },
    }
}

/// The system-image ABI that will actually run on this host. Emulator
/// images come in multiple ABIs (x86_64, arm64-v8a, ...) and picking one
/// that doesn't match the host either fails outright or "works" without
/// hardware acceleration at a crawl — e.g. an arm64-v8a image on an x86_64
/// Windows/Linux/Intel-Mac box.
#[tauri::command]
pub(crate) fn preferred_abi() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64-v8a"
    } else {
        "x86_64"
    }
}

/// Downloads + unzips the official cmdline-tools package into the persistent
/// data dir, then accepts SDK licenses non-interactively. Emits
/// "sdk_install_progress" events throughout so the UI can show real progress
/// instead of a static spinner.
#[tauri::command]
pub(crate) async fn install_sdk(app: AppHandle) -> Result<String, AppError> {
    use futures_util::StreamExt;
    use std::io::Write;

    let root = sdk_root();
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;

    // install_sdk is also the entry point for repairing/completing an
    // existing install (e.g. adding the JDK to an install that predates
    // ensure_jdk) — it needs to be safe to call again on a machine that
    // already has cmdline-tools. Skipping straight past the download+extract
    // when sdkmanager already exists isn't just an optimization: re-running
    // the extract unconditionally leaves a stray flat copy of cmdline-tools'
    // own bin/lib/NOTICE.txt/source.properties sitting directly under
    // sdk/cmdline-tools/ forever, alongside the real sdk/cmdline-tools/latest/
    // — confirmed live, it happened for real on this machine's own install
    // during an earlier verification pass that called install_sdk twice.
    if cmdline_tools_bin("sdkmanager").exists() {
        emit_progress(
            &app,
            "downloading",
            Some(100.0),
            "Command-line tools already installed",
        );
    } else {
        // The Cancel button is shown throughout the whole install, but
        // until this, cancelling only actually did anything once
        // sdkmanager itself was running (run_sdkmanager_streaming, below,
        // is the only place that ever checked `task.cancelled`) — clicking
        // Cancel during this raw download did nothing (confirmed live: it
        // reported "Nothing running" and the download just continued).
        // Reset-then-check the same flag `cancel_sdk_task` sets here too.
        let task = app.state::<SdkTask>();
        task.cancelled.store(false, Ordering::SeqCst);

        emit_progress(&app, "downloading", Some(0.0), "Starting download…");

        let resp = reqwest::get(CMDLINE_TOOLS_URL).await?;
        let total = resp.content_length();

        let zip_path = root.join("cmdline-tools.zip");
        let mut file = std::fs::File::create(&zip_path).map_err(|e| e.to_string())?;

        let mut stream = resp.bytes_stream();
        let mut downloaded: u64 = 0;
        while let Some(chunk) = stream.next().await {
            if task.cancelled.swap(false, Ordering::SeqCst) {
                drop(file);
                let _ = std::fs::remove_file(&zip_path);
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
            emit_progress(&app, "downloading", percent, &detail);
        }
        drop(file);

        emit_progress(&app, "downloading", Some(100.0), "Verifying download…");
        verify_sha256(&zip_path, CMDLINE_TOOLS_SHA256)?;

        emit_progress(&app, "extracting", None, "Extracting command-line tools…");
        let zf = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
        let mut archive = zip::ZipArchive::new(zf).map_err(|e| e.to_string())?;
        // Google's zip extracts to "cmdline-tools/" — we nest it under "latest/"
        // per the layout sdkmanager expects.
        let extract_to = root.clone();
        archive.extract(&extract_to).map_err(|e| e.to_string())?;

        let extracted = root.join("cmdline-tools");
        let latest = extracted.join("latest");
        if !latest.exists() {
            // rename the flat "cmdline-tools/bin,lib,..." into "cmdline-tools/latest/..."
            let tmp = root.join("cmdline-tools-tmp");
            std::fs::rename(&extracted, &tmp).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&extracted).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &latest).map_err(|e| e.to_string())?;
        }
        let _ = std::fs::remove_file(&zip_path);
    }

    // Must happen before anything below tries to run sdkmanager — that's a
    // Java process, and android_tool() only points it at Beo's own JDK if
    // that JDK already exists on disk.
    ensure_jdk(&app).await?;

    // From here on it's all blocking child-process work (license prompt,
    // then two sdkmanager installs) — run it via spawn_blocking rather than
    // directly in this command's task, for the same reason download_image
    // does: blocking here directly would tie up a thread that's supposed to
    // stay free to dispatch other commands, cancel_sdk_task included.
    tauri::async_runtime::spawn_blocking(move || {
        let task = app.state::<SdkTask>();
        emit_progress(&app, "licenses", None, "Accepting SDK licenses…");
        // Accept all licenses non-interactively.
        let mut child = android_tool(cmdline_tools_bin("sdkmanager"))
            .arg("--licenses")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(&b"y\n".repeat(20));
        }
        // Registered in task.child (same as run_sdkmanager_streaming does)
        // so Cancel can actually kill this step too, not just the
        // platform-tools/emulator installs after it — previously this
        // short-lived child was invisible to cancel_sdk_task entirely.
        // Take-before-wait mirrors run_sdkmanager_streaming's own handling
        // of the cancel/finish race: if cancel_sdk_task already took (and
        // killed) the child by the time we get here, ours is `None` and
        // there's nothing left to wait on.
        *task.child.lock().map_err(|e| e.to_string())? = Some(child);
        if let Some(mut child) = task.child.lock().map_err(|e| e.to_string())?.take() {
            let _ = child.wait();
        }
        if task.cancelled.swap(false, Ordering::SeqCst) {
            return Err(AppError::Cancelled);
        }

        emit_progress(&app, "platform-tools", None, "Installing platform-tools…");
        run_sdkmanager_streaming(&app, &task, &["platform-tools"], "platform-tools")?;

        emit_progress(&app, "emulator", None, "Installing emulator…");
        run_sdkmanager_streaming(&app, &task, &["emulator"], "emulator")?;

        emit_progress(&app, "done", Some(100.0), "SDK installed");
        Ok("SDK installed".into())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Pulls a trailing "NN%" or "NN.N%" out of a progress line, e.g. from
/// sdkmanager's own `[=====     ] 42%` style output.
pub(crate) fn parse_percent(line: &str) -> Option<f64> {
    let idx = line.rfind('%')?;
    let start = line[..idx]
        .rfind(|c: char| !c.is_ascii_digit() && c != '.')
        .map(|i| i + 1)
        .unwrap_or(0);
    line[start..idx].parse::<f64>().ok()
}

enum ReaderMsg {
    Line(String),
    Eof,
}

/// After a cancelled sdkmanager install, the target package directory (e.g.
/// sdk/system-images/android-37.0/google_apis_playstore/x86_64) can be left
/// containing only sdkmanager's own `.installer` marker file — it creates
/// that upfront, before any real package content arrives. Not actual
/// partial-download bytes (those live under sdk/.temp and are already
/// cleaned up in cancel_sdk_task), just litter that would otherwise sit
/// there indefinitely looking like a leftover half-installed package.
pub(crate) fn cleanup_stale_package_dir(package_id: Option<&str>) {
    let Some(package_id) = package_id else { return };
    let mut dir = sdk_root();
    for segment in package_id.split(';') {
        dir.push(segment);
    }
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    let only_marker = entries.len() == 1 && entries[0].file_name() == ".installer";
    if only_marker {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Runs sdkmanager, streaming its stdout so real progress percentages reach
/// the UI (instead of blocking silently until the whole install finishes),
/// and registers the child in `SdkTask` so `cancel_sdk_task` can kill it.
///
/// The actual byte-reading happens on a dedicated thread rather than inline:
/// on Windows, `sdkmanager.bat` runs as `cmd.exe` running `java.exe`, and
/// after `taskkill /T` kills that tree, the pipe's read end can still be
/// left waiting on a handle that never signals EOF (observed in practice —
/// clicking Cancel killed the process but the command never returned). By
/// reading on a side thread and polling `task.cancelled` here instead of
/// blocking on the read directly, cancellation returns immediately
/// regardless of whether the pipe ever actually closes; the reader thread
/// is simply abandoned if that happens; it can't leak anything but itself.
fn run_sdkmanager_streaming(
    app: &AppHandle,
    task: &State<SdkTask>,
    args: &[&str],
    stage: &str,
) -> Result<String, AppError> {
    use std::io::Read;

    task.cancelled.store(false, Ordering::SeqCst);

    let mut child = android_tool(cmdline_tools_bin("sdkmanager"))
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    let stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    *task.child.lock().map_err(|e| e.to_string())? = Some(child);

    let (tx, rx) = std::sync::mpsc::channel::<ReaderMsg>();
    if let Some(mut out) = stdout {
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut line = String::new();
            loop {
                let n = match out.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                for &b in &buf[..n] {
                    let c = b as char;
                    if c == '\r' || c == '\n' {
                        let trimmed = line.trim().to_string();
                        if !trimmed.is_empty() {
                            let _ = tx.send(ReaderMsg::Line(trimmed));
                        }
                        line.clear();
                    } else {
                        line.push(c);
                    }
                }
            }
            let _ = tx.send(ReaderMsg::Eof);
        });
    } else {
        let _ = tx.send(ReaderMsg::Eof);
    }

    loop {
        if task.cancelled.swap(false, Ordering::SeqCst) {
            // cancel_sdk_task already killed the process tree and cleared
            // task.child — nothing left here to wait on.
            cleanup_stale_package_dir(args.first().copied());
            return Err(AppError::Cancelled);
        }
        match rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(ReaderMsg::Line(l)) => emit_progress(app, stage, parse_percent(&l), &l),
            Ok(ReaderMsg::Eof) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let mut err_text = String::new();
    if let Some(mut e) = stderr.take() {
        let _ = e.read_to_string(&mut err_text);
    }

    // Take the child out and drop the lock *before* the potentially-slow
    // wait() call — a process can hold its stdout pipe open past the point
    // where it's actually done, so EOF and exit aren't simultaneous.
    // Holding the mutex across that wait would block cancel_sdk_task's own
    // lock().take() for the same duration, defeating cancellation.
    let taken = task.child.lock().map_err(|e| e.to_string())?.take();
    let status = match taken {
        Some(mut child) => child.wait().map_err(|e| e.to_string())?,
        // Cancelled between the loop above finishing and here.
        None => {
            cleanup_stale_package_dir(args.first().copied());
            return Err(AppError::Cancelled);
        }
    };

    if !status.success() {
        return Err(AppError::Other(if err_text.trim().is_empty() {
            format!("sdkmanager exited with {status}")
        } else {
            err_text
        }));
    }
    Ok("done".into())
}

/// Kills whatever sdkmanager task is currently running, if any.
///
/// `sdkmanager` on Windows is `sdkmanager.bat` — Command::spawn() launches
/// `cmd.exe` running that script, which in turn launches the actual
/// `java.exe` doing the work. `Child::kill()` only terminates the `cmd.exe`
/// wrapper we hold a handle to; `java.exe` is orphaned and keeps running
/// (and keeps writing to the piped stdout), so the download silently
/// continues. `taskkill /T` kills the whole process tree instead — but even
/// that can leave the read side of the stdout pipe waiting forever (see
/// run_sdkmanager_streaming), so this also flips `cancelled` immediately,
/// which is what actually unblocks the stuck command.
#[tauri::command]
pub(crate) fn cancel_sdk_task(task: State<SdkTask>) -> Result<String, String> {
    task.cancelled.store(true, Ordering::SeqCst);
    let mut guard = task.child.lock().map_err(|e| e.to_string())?;
    if let Some(mut child) = guard.take() {
        #[cfg(target_os = "windows")]
        {
            let _ = Command::new("taskkill")
                .args(["/F", "/T", "/PID", &child.id().to_string()])
                .output();
        }
        let _ = child.kill();
        let _ = child.wait();
        // sdkmanager stages downloads under sdk_root/.temp before moving
        // completed packages into place — a killed download leaves partial
        // files behind here rather than in the package's final directory,
        // so clearing it is enough to undo an in-progress download.
        let _ = std::fs::remove_dir_all(sdk_root().join(".temp"));
        Ok("Cancelled".into())
    } else {
        Ok("Nothing running".into())
    }
}

fn run_sdkmanager(args: &[&str]) -> Result<String, String> {
    let out = android_tool(cmdline_tools_bin("sdkmanager"))
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Lists installable system images, e.g. "system-images;android-34;google_apis;x86_64"
/// or "system-images;android-34;google_apis_playstore;x86_64" (Play Store included).
///
/// `sdkmanager --list` prints a `|`-delimited table:
///   "  system-images;android-37.0;google_apis_playstore;x86_64 | 6 | Google Play ... | system-images\...\x86_64"
/// The previous parser took the first *whitespace-split* token, which only
/// worked because package IDs happen to contain no spaces — it wasn't
/// actually reading the table's column structure, just relying on that
/// coincidence. Splitting on the real column delimiter (`|`) instead is a
/// closer match to the actual format and won't quietly misparse if the ID
/// column's padding or a future package ID's shape changes.
/// Split out from `list_available_images` so it can be unit tested against
/// a real captured `sdkmanager --list` dump without needing a live SDK.
fn parse_available_images(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.trim_start().starts_with("system-images;"))
        .filter_map(|l| {
            let id = l.split('|').next()?.trim();
            (!id.is_empty()).then(|| id.to_string())
        })
        .collect()
}

#[tauri::command]
pub(crate) fn list_available_images() -> Result<Vec<String>, String> {
    let out = run_sdkmanager(&["--list"])?;
    Ok(parse_available_images(&out))
}

// This — and install_sdk above — must run via spawn_blocking rather than
// directly in the command handler. Both do long blocking I/O (child process
// spawn + reads that can run for minutes on a big system image); a Tauri
// command handler that blocks directly ties up the thread that's supposed
// to be dispatching *other* IPC calls too — including cancel_sdk_task — so
// on a machine with few CPU cores, Cancel could sit queued behind the very
// download it's trying to stop. spawn_blocking moves the work onto tokio's
// dedicated (much larger, elastic) blocking-task pool instead.
#[tauri::command]
pub(crate) async fn download_image(app: AppHandle, image_id: String) -> Result<String, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let task = app.state::<SdkTask>();
        emit_progress(&app, "image", None, &format!("Downloading {image_id}…"));
        let result = run_sdkmanager_streaming(&app, &task, &[&image_id], "image");
        if result.is_ok() {
            emit_progress(&app, "done", Some(100.0), "Image ready");
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_percent_reads_trailing_percentage() {
        assert_eq!(parse_percent("[=====     ] 42%"), Some(42.0));
        assert_eq!(
            parse_percent("Downloading x86_64-37.0_r06.zip... 10%"),
            Some(10.0)
        );
        assert_eq!(parse_percent("11.5%"), Some(11.5));
    }

    #[test]
    fn parse_percent_returns_none_without_a_percentage() {
        assert_eq!(parse_percent("Fetch remote repository..."), None);
        assert_eq!(parse_percent(""), None);
    }

    #[test]
    fn cleanup_stale_package_dir_ignores_missing_package_id() {
        // Just confirms this doesn't panic when there's nothing to clean up —
        // the real filesystem-dependent behavior needs a live sdk_root(),
        // which isn't something a unit test should fabricate.
        cleanup_stale_package_dir(None);
    }

    // Real `sdkmanager --list` output, captured live from this machine's
    // actual SDK install (2026-09-09) — deliberately includes the exact
    // thing that broke the old whitespace-splitting parser this replaced
    // (see the doc comment on parse_available_images/list_available_images):
    // the ID column's padding width genuinely differs between rows in real
    // output (7 spaces vs. many more here), and a non-system-image row
    // (an add-on) that must be filtered out rather than misread.
    const REAL_SDKMANAGER_LIST: &str = "Available Packages:\n  Path                                                                            | Version           | Description                                                                     \n  -------                                                                         | -------           | -------                                                                         \n  add-ons;addon-google_apis-google-15                                             | 3                 | Google APIs                                                                     \n  system-images;android-36;google_apis_playstore;x86_64   | 7       | Google Play Intel x86_64 Atom System Image | system-images\\android-36\\google_apis_playstore\\x86_64  \n  system-images;android-36;google_apis_playstore;x86_64                           | 7                 | Google Play Intel x86_64 Atom System Image\n";

    #[test]
    fn parse_available_images_matches_real_sdkmanager_output() {
        assert_eq!(
            parse_available_images(REAL_SDKMANAGER_LIST),
            vec![
                "system-images;android-36;google_apis_playstore;x86_64".to_string(),
                "system-images;android-36;google_apis_playstore;x86_64".to_string(),
            ]
        );
    }
}
