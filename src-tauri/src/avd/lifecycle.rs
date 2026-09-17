use super::naming::sanitize_avd_name;
use super::profiles::category_for_device_id;
use crate::util::{adb_bin, android_tool, cmdline_tools_bin, emulator_bin, recommended_ram_mb};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Serialize, Deserialize, Clone)]
pub struct AvdLogLine {
    name: String,
    line: String,
}

/// Fired once, the moment `launch_avd`'s boot-completion poll actually
/// confirms the guest OS is up (not just that the process started) — see
/// the comment where this is emitted. Separate from `avd_log`'s plain text
/// lines so the frontend can key dashboard UI off it directly instead of
/// pattern-matching log text.
#[derive(Serialize, Deserialize, Clone)]
pub struct AvdBootedEvent {
    name: String,
}

// camelCase here specifically, matching IdeIntegrationStatus's own comment
// on the same lesson: this struct has multi-word fields, and without
// rename_all serde sends disk_usage_mb/ram_mb (snake_case) while the
// frontend reads diskUsageMb/ramMb (camelCase) — those reads would
// silently resolve to undefined otherwise.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AvdInfo {
    name: String,
    category: String, // "phone" or "tablet" — best-effort, from the device profile
    // Both `None` when parsed from avdmanager's text output alone (which
    // has neither) — populated afterward from the real files on disk, see
    // `list_avds`. Real, not estimated: config.ini's own
    // `disk.dataPartition.size` understates actual usage once snapshots
    // exist — confirmed by hand, a device with a 6G declared data
    // partition was actually using 11G on disk, mostly a boot snapshot.
    disk_usage_mb: Option<u64>,
    ram_mb: Option<u32>,
}

/// `avdmanager list avd -c` (the machine-readable form used before) only
/// gives names, with no way to tell a phone from a tablet in the listing.
/// The verbose form includes a `Device: <profile_id> (<manufacturer>)` line
/// per AVD, which is enough to recover that without needing to open each
/// AVD's `config.ini` individually.
/// Split out from `list_avds` so it can be unit tested against a real
/// captured `avdmanager list avd` dump without needing a live SDK.
fn parse_avd_list(text: &str) -> Vec<AvdInfo> {
    let mut result = Vec::new();
    let mut pending_name: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Name:") {
            // A pending name with no "Device:" line before the next "Name:"
            // happens for AVDs avdmanager couldn't fully load (e.g. its
            // device profile was removed) — still list it, just untagged.
            if let Some(name) = pending_name.take() {
                result.push(AvdInfo {
                    name,
                    category: "phone".into(),
                    disk_usage_mb: None,
                    ram_mb: None,
                });
            }
            pending_name = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("Device:") {
            if let Some(name) = pending_name.take() {
                let device_id = rest.split('(').next().unwrap_or("").trim();
                result.push(AvdInfo {
                    name,
                    category: category_for_device_id(device_id).to_string(),
                    disk_usage_mb: None,
                    ram_mb: None,
                });
            }
        }
    }
    if let Some(name) = pending_name.take() {
        result.push(AvdInfo {
            name,
            category: "phone".into(),
            disk_usage_mb: None,
            ram_mb: None,
        });
    }
    result
}

/// An AVD's registered *name* and its actual on-disk *folder* name aren't
/// guaranteed to match — confirmed by hand on a real device on this
/// machine: `avdmanager list avd` reported it as `Medium_Phone_API_36.0`,
/// but its real directory was `Medium_Phone.avd` (whatever created it —
/// an older Beo build, Android Studio, a manual `avdmanager` invocation —
/// named the folder differently than the AVD's registered name). The
/// authoritative mapping lives in `<name>.ini`'s `path=` line, which is
/// what `avdmanager`/the emulator itself actually reads — so that's
/// checked first here, with the `<name>.avd` guess only as a fallback for
/// the case (usually true right after Beo's own `create_avd`) where no
/// `.ini` exists yet or it can't be read.
fn avd_dir(name: &str) -> Option<std::path::PathBuf> {
    let home = dirs::home_dir()?;
    let avd_root = home.join(".android").join("avd");

    let ini_path = avd_root.join(format!("{name}.ini"));
    if let Ok(contents) = std::fs::read_to_string(&ini_path) {
        if let Some(declared) = contents.lines().find_map(|l| l.strip_prefix("path=")) {
            let declared = std::path::PathBuf::from(declared.trim());
            if declared.exists() {
                return Some(declared);
            }
        }
    }

    let fallback = avd_root.join(format!("{name}.avd"));
    fallback.exists().then_some(fallback)
}

/// Sums real file sizes under the AVD's directory (images, snapshots,
/// everything) — not the config's *declared* size, which understates
/// actual usage once snapshots exist (confirmed by hand: a device with a
/// 6G declared data partition was genuinely using 11G on disk, mostly one
/// boot snapshot). Best-effort: `None` if the directory can't be read at
/// all, but a file that vanishes mid-walk (e.g. the emulator deleting a
/// temp file concurrently) is just skipped rather than failing the whole
/// count.
fn avd_disk_usage_mb(name: &str) -> Option<u64> {
    fn walk(dir: &std::path::Path) -> u64 {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        entries
            .filter_map(|e| e.ok())
            .map(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path)
                } else {
                    entry.metadata().map(|m| m.len()).unwrap_or(0)
                }
            })
            .sum()
    }
    let dir = avd_dir(name)?;
    if !dir.exists() {
        return None;
    }
    Some(walk(&dir) / 1_048_576)
}

/// Reads `hw.ramSize` (MB) straight from the AVD's own `config.ini` — this
/// one *is* accurate and enforced (it's what the emulator actually
/// allocates at launch), unlike the disk-size fields in the same file.
fn avd_ram_mb(name: &str) -> Option<u32> {
    let config_path = avd_dir(name)?.join("config.ini");
    let contents = std::fs::read_to_string(&config_path).ok()?;
    let raw = contents.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "hw.ramSize").then(|| value.trim().to_string())
    })?;
    parse_size_to_mb(&raw)
}

/// Android's config.ini size fields (`hw.ramSize`, `disk.dataPartition.size`,
/// `sdcard.size`, ...) are a plain number of MB, or a number with a K/M/G
/// suffix — confirmed by hand: two real devices on this machine had
/// `hw.ramSize = 2048` and `hw.ramSize = 1536M` respectively, and a naive
/// `parse::<u32>()` silently failed (returning `None`, read as "unknown")
/// on the second one purely because of the trailing unit letter.
fn parse_size_to_mb(raw: &str) -> Option<u32> {
    let raw = raw.trim();
    let last = raw.chars().last()?;
    let (digits, mb_multiplier): (&str, f64) = if last.eq_ignore_ascii_case(&'g') {
        (&raw[..raw.len() - 1], 1024.0)
    } else if last.eq_ignore_ascii_case(&'m') {
        (&raw[..raw.len() - 1], 1.0)
    } else if last.eq_ignore_ascii_case(&'k') {
        (&raw[..raw.len() - 1], 1.0 / 1024.0)
    } else {
        (raw, 1.0)
    };
    let value: f64 = digits.trim().parse().ok()?;
    Some((value * mb_multiplier).round() as u32)
}

#[tauri::command]
pub(crate) fn list_avds() -> Result<Vec<AvdInfo>, String> {
    let out = android_tool(cmdline_tools_bin("avdmanager"))
        .args(["list", "avd"])
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(parse_avd_list(&text)
        .into_iter()
        .map(|mut avd| {
            avd.disk_usage_mb = avd_disk_usage_mb(&avd.name);
            avd.ram_mb = avd_ram_mb(&avd.name);
            avd
        })
        .collect())
}

#[tauri::command]
pub(crate) fn create_avd(
    name: String,
    image_id: String,
    device: String,
    ram_mb: Option<u32>,
    disk_gb: Option<u32>,
) -> Result<String, String> {
    let name = sanitize_avd_name(&name)?;
    let mut child = android_tool(cmdline_tools_bin("avdmanager"))
        .args(["create", "avd", "-n", &name, "-k", &image_id, "-d", &device])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        // avdmanager prompts "Do you wish to create a custom hardware profile [no]"
        let _ = stdin.write_all(b"no\n");
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    // avdmanager always writes `hw.initialOrientation = portrait`, even for
    // tablet profiles whose own config (hw.lcd.width > hw.lcd.height) is
    // clearly landscape-shaped — confirmed by hand against a freshly created
    // pixel_tablet AVD's config.ini. Android Studio's own AVD creation flow
    // patches this after the fact for tablets; avdmanager's CLI has no flag
    // for it, so do the same patch here rather than ship every tablet
    // starting rotated wrong.
    if category_for_device_id(&device) == "tablet" {
        let _ = set_avd_config_value(&name, "hw.initialOrientation", "landscape");
    }
    // Several device profiles (this machine's pixel_tablet included — confirmed
    // by hand in its config.ini: `hw.keyboard = no`) ship with the emulated
    // hardware keyboard disabled by default, so the physical keyboard doesn't
    // reach the guest at all until it's turned on by hand every time via
    // Extended Controls > Settings > "Enable keyboard input". Force it on for
    // every device Beo creates instead — there's no real downside to having it
    // on (the on-screen keyboard still works fine alongside it), and it's the
    // one thing standing between "type normally" and "click a tiny toggle
    // first, every session."
    let _ = set_avd_config_value(&name, "hw.keyboard", "yes");
    // avdmanager's own per-profile default (1536 MB for pixel/pixel_6) is
    // tight for real-world use — confirmed live: a throwaway device on
    // that default genuinely ran out of memory playing a single YouTube
    // tab in Chrome (lowmemorykiller actively reaping processes, Chrome's
    // main process itself killed moments later), while a device created
    // outside Beo with 2048 MB handled the same workload fine all night.
    // No caller-specified value (Simple mode has no slider at all;
    // Developer mode before the slider is touched) falls back to
    // `recommended_ram_mb()` rather than a single flat number for every
    // host — Simple mode users never see a slider, so the one chance to
    // give them more than the bare-minimum floor is picking a smarter
    // automatic default based on what their machine can actually spare.
    let ram_mb = ram_mb.unwrap_or_else(recommended_ram_mb);
    let _ = set_avd_config_value(&name, "hw.ramSize", &ram_mb.to_string());
    // avdmanager's own default (6G) leaves almost no real headroom —
    // confirmed live: a freshly booted device, before the user had
    // installed or updated anything themselves, already showed /data at
    // 84% full (4.9G of 6G used, ~1G free) purely from the Google Play
    // image's own bundled apps and services. That little free space is
    // exactly what "can't even update the apps on the image" looks like —
    // Play Store updates need real staging room. Chosen generously (16G,
    // leaving ~11G of real headroom on top of that same baseline
    // footprint) rather than just barely enough: this is a dynamically
    // growing virtual disk, not pre-allocated, so a bigger ceiling costs
    // no real host disk upfront — actual usage still only ever reflects
    // what's genuinely stored, same as the 6G default already did.
    let disk_gb = disk_gb.unwrap_or(16);
    let _ = set_avd_config_value(&name, "disk.dataPartition.size", &format!("{disk_gb}G"));
    Ok(format!("Created {name}"))
}

/// Best-effort: rewrites (or appends) a `key = value` line in a freshly
/// created AVD's `config.ini`. Errors are swallowed by the caller — a
/// missed patch just means that one device keeps avdmanager's own default
/// for that key, which is the same behavior as before this existed, not a
/// regression.
fn set_avd_config_value(name: &str, key: &str, value: &str) -> Result<(), String> {
    let config_path = avd_dir(name)
        .ok_or("Couldn't locate the AVD's directory")?
        .join("config.ini");
    let contents = std::fs::read_to_string(&config_path).map_err(|e| e.to_string())?;
    // Matched on the exact key before '=', not a prefix — config.ini has
    // sibling keys that share a prefix (e.g. `hw.keyboard.charmap`,
    // `hw.keyboard.lid` alongside `hw.keyboard`), and a naive starts_with
    // would clobber those too.
    let mut found = false;
    let mut new_lines: Vec<String> = contents
        .lines()
        .map(|line| {
            if line.split('=').next().map(str::trim) == Some(key) {
                found = true;
                format!("{key} = {value}")
            } else {
                line.to_string()
            }
        })
        .collect();
    if !found {
        new_lines.push(format!("{key} = {value}"));
    }
    std::fs::write(&config_path, new_lines.join("\n") + "\n").map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn delete_avd(name: String) -> Result<String, String> {
    let out = android_tool(cmdline_tools_bin("avdmanager"))
        .args(["delete", "avd", "-n", &name])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    Ok(format!("Deleted {name}"))
}

/// Launches detached so it survives if the manager window is closed.
/// Default `-accel auto` lets the emulator use hardware virtualization
/// (KVM/HVF/WHPX) whenever available, falling back to software CPU
/// emulation otherwise — that part is left alone and unaffected here.
///
/// `-gpu` is pinned to `host` (real GPU passthrough via gfxstream) rather
/// than left as `auto`. This isn't the original choice — `auto` previously
/// hung forever at the boot logo on this exact host (WHPX genuinely
/// operational, `adb` reporting the device "offline" indefinitely, no
/// error), which is why this was pinned to `swiftshader_indirect`
/// (software rendering) for a long stretch of this project instead: it
/// worked reliably, just slower, and with audible video-playback audio
/// glitching under load — software rendering competes with audio for the
/// same CPU cycles, worse than real GPU rendering does. Re-tested by hand
/// (not just reasoned about) after that glitching was reported: explicit
/// `-gpu host` boots this same host fast and reliably (real Radeon GPU via
/// gfxstream, confirmed in the emulator's own log), unlike `auto`'s
/// heuristic, which is what actually hung before — `host` was never
/// re-tried on its own until now. If a hang like the original one ever
/// recurs on some future host/driver combination, `swiftshader_indirect`
/// is the known-safe fallback to pin back to.
/// Clipboard sharing (copy/paste between host and device) is on by default
/// — matching Android Studio's emulator — and only disabled if requested.
///
/// The emulator binary can fail almost instantly on a bad setup — most
/// notably, resolving the wrong SDK root from an ANDROID_SDK_ROOT/
/// ANDROID_HOME set by some other installed SDK (see android_tool above)
/// — and a bare spawn() gives no indication of that: the call still
/// "succeeds" because the process did start, it just immediately exited.
/// So this gives it a moment, then checks whether it's still alive and
/// reports the real error if not, instead of reporting success for a
/// process that's already gone.
#[tauri::command]
pub(crate) fn launch_avd(
    app: AppHandle,
    name: String,
    headless: bool,
    share_clipboard: bool,
) -> Result<String, String> {
    // Confirmed live: without this, launching an AVD that's already running
    // reported *success* ("Launching ...") immediately — the real emulator
    // binary doesn't reject a duplicate until several seconds into its own
    // startup checks, well past the point this function had already
    // returned Ok. It then exits with "FATAL | Running multiple emulators
    // with the same AVD is an experimental feature," which the user never
    // saw: only one real process actually survived, and nothing told them
    // the second click did nothing. Checking first and failing immediately
    // beats a false "Launching..." followed by silent failure.
    if find_serial_for_avd(&name).is_ok() {
        return Err(format!("\"{name}\" is already running."));
    }

    let mut args = vec![
        "-avd".to_string(),
        name.clone(),
        "-gpu".to_string(),
        "host".to_string(),
        // Reverted (2026-09-14): this build defaults to VirtioSndCard = on
        // (a modern paravirtualized virtio-snd device, replacing the older
        // emulated Intel HDA card — see emulator/lib/advancedFeatures.ini).
        // An earlier pass here forced `-feature -VirtioSndCard` (legacy
        // Intel HDA) as an experiment to fix reported popping — but that
        // was only ever confirmed to still boot and load audio modules,
        // *never* confirmed to actually produce audible sound by a real
        // human ear. Investigated live after a *total silence* report on a
        // different device: `dumpsys media.audio_flinger` showed a real,
        // varying, non-silent signal power history at the AudioFlinger
        // mixer/HAL boundary (proving Android's own audio stack was doing
        // everything right, all the way to the virtual sound device), with
        // zero underruns — meaning the break was downstream, inside QEMU's
        // audio backend itself, specifically under the untested Intel HDA
        // path. The original virtio-snd default was reported audible
        // (if glitchy) before this experiment; there is no equivalent
        // confirmation the Intel HDA path is audible at all on this host.
        // Reverting to virtio-snd (silence is a far worse UX than glitchy
        // audio) — `-gpu host` is kept since that part *was* independently
        // confirmed to still boot reliably and use the real GPU.
    ];
    if headless {
        args.push("-no-window".to_string());
    }
    if !share_clipboard {
        // Confirmed live against the currently installed emulator
        // (37.1.11.0): `-no-clipboard-sharing` is gone entirely — not
        // renamed, not moved to `-feature` (checked the full feature list
        // in emulator/lib/advancedFeatures.ini, no clipboard entry at all).
        // Passing it now hard-fails the launch with "unknown option."
        // Rather than either crash the launch or silently pretend the
        // toggle worked, tell the user via the debug log that this build
        // can't honor it and proceed with clipboard sharing on (its
        // current unconditional default) — a much smaller regression than
        // failing to launch at all.
        let _ = app.emit(
            "avd_log",
            AvdLogLine {
                name: name.clone(),
                line: "Clipboard sharing can't be disabled on this emulator version — launching with it on.".into(),
            },
        );
    }
    let mut child = android_tool(emulator_bin())
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    // Drain both pipes on side threads that keep running for the life of
    // the process — if launch succeeds and the emulator keeps writing to
    // stdout/stderr, nothing is left blocked waiting on a full pipe buffer.
    // Every line is also emitted live as an "avd_log" event: a boot that
    // hangs (not crashes) produces no error for the 1.5s check below to
    // catch, so without this there was no way to see *why* it was stuck —
    // only that it was. The debug panel now shows the emulator's own
    // ongoing boot log in real time.
    let captured = Arc::new(Mutex::new(String::new()));
    let streams: [Option<Box<dyn std::io::Read + Send>>; 2] = [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    ];
    for stream in streams.into_iter().flatten() {
        let captured = captured.clone();
        let app = app.clone();
        let name = name.clone();
        std::thread::spawn(move || {
            use std::io::Read;
            let mut stream = stream;
            let mut buf = [0u8; 4096];
            let mut line = String::new();
            loop {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&buf[..n]);
                        if let Ok(mut c) = captured.lock() {
                            c.push_str(&text);
                            // Keep only the tail — this is for diagnosing an
                            // early crash, not for holding a full session log.
                            let len = c.len();
                            if len > 8192 {
                                let cut = len - 8192;
                                c.drain(..cut);
                            }
                        }
                        for ch in text.chars() {
                            if ch == '\n' || ch == '\r' {
                                let trimmed = line.trim();
                                // These two repeat forever, often dozens of
                                // times a second, whenever the host has any
                                // extra network adapter the emulator's netsim
                                // can't resolve cleanly (VirtualBox host-only
                                // adapters, VPN clients, Hyper-V switches,
                                // WSL) — confirmed by hand: the device had
                                // *already* fully booted (sys.boot_completed=1,
                                // launcher in foreground) while this kept
                                // scrolling forever, making a genuinely
                                // working device look permanently stuck.
                                // Purely cosmetic noise, never a sign of an
                                // actual problem, so it's dropped here rather
                                // than drowning out lines that do matter.
                                let is_log_noise = trimmed
                                    .starts_with("INFO         | IPv4 server found")
                                    || trimmed.starts_with("INFO         | Ignore IPv6 address");
                                if !trimmed.is_empty() && !is_log_noise {
                                    let _ = app.emit(
                                        "avd_log",
                                        AvdLogLine {
                                            name: name.clone(),
                                            line: trimmed.to_string(),
                                        },
                                    );
                                }
                                line.clear();
                            } else {
                                line.push(ch);
                            }
                        }
                    }
                }
            }
        });
    }

    // Absence of errors isn't the same as confirmation the device is
    // actually usable — confirmed by hand twice now: (1) a device can be
    // fully booted and responsive while its log keeps scrolling with
    // unrelated background noise forever, reading as "still stuck" with
    // nothing to say otherwise; (2) sys.boot_completed can flip to 1, then
    // flip back as system_server crashes and Zygote restarts from scratch
    // — an unstable system image (see VERIFIED_API_LEVELS on the frontend)
    // can crash-loop this way indefinitely while every individual snapshot
    // in time claims "booted". So this doesn't declare success on the
    // first sys.boot_completed=1 — it requires it to hold for three
    // consecutive 5s checks (~15s) before announcing readiness, and if a
    // device that already reported booted once stops reporting that,
    // that's the crash-loop signature and gets its own clear warning
    // instead of silence.
    {
        let app = app.clone();
        let name = name.clone();
        std::thread::spawn(move || {
            let mut consecutive_ok = 0;
            let mut ever_booted = false;
            let mut warned = false;
            for _ in 0..180 {
                std::thread::sleep(std::time::Duration::from_secs(5));
                let Ok(serial) = find_serial_for_avd(&name) else {
                    continue;
                };
                let out = android_tool(adb_bin())
                    .args(["-s", &serial, "shell", "getprop", "sys.boot_completed"])
                    .output();
                let is_booted = out
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
                    .unwrap_or(false);

                if is_booted {
                    consecutive_ok += 1;
                    ever_booted = true;
                    if consecutive_ok >= 3 {
                        let _ = app.emit(
                            "avd_log",
                            AvdLogLine {
                                name: name.clone(),
                                line: "✓ Boot completed — device is ready to use".into(),
                            },
                        );
                        // A separate structured event from the log line
                        // above: previously this confirmation only ever
                        // reached the (devMode-only, easy to miss) debug
                        // panel — the dashboard card itself flipped
                        // straight from "Stopped" to "Running" the instant
                        // Launch was clicked, with no visible distinction
                        // between "the process started" and "the guest OS
                        // actually finished booting," even though rotating
                        // or installing an APK during that window is
                        // liable to just fail. The frontend now shows
                        // "Starting…" until this arrives.
                        let _ = app.emit("avd_booted", AvdBootedEvent { name: name.clone() });
                        break;
                    }
                } else {
                    if ever_booted && !warned {
                        let _ = app.emit(
                            "avd_log",
                            AvdLogLine {
                                name: name.clone(),
                                line: "⚠ Device restarted right after boot completed — this usually means the system image is unstable on this host. If it keeps happening, recreate the device with a different image (Developer mode lets you pick one).".into(),
                            },
                        );
                        warned = true;
                    }
                    consecutive_ok = 0;
                }
            }
        });
    }

    std::thread::sleep(std::time::Duration::from_millis(1500));
    match child.try_wait() {
        Ok(Some(status)) if !status.success() => {
            let text = captured.lock().map(|c| c.clone()).unwrap_or_default();
            // Prefer the FATAL line (the actual terminal reason) if there is
            // one; otherwise every ERROR line together usually reads better
            // than just the last one, which is often a trailing footnote
            // ("Directories are searched in the order...") rather than the
            // specific problem ("Unknown AVD name...").
            let fatal = text.lines().find(|l| l.contains("FATAL")).map(str::trim);
            let errors: Vec<&str> = text
                .lines()
                .filter(|l| l.contains("ERROR"))
                .map(str::trim)
                .collect();
            let detail = fatal
                .map(str::to_string)
                .or_else(|| (!errors.is_empty()).then(|| errors.join(" ")))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| text.trim().to_string());
            Err(if detail.is_empty() {
                format!("Emulator exited immediately (status {status})")
            } else {
                detail
            })
        }
        _ => Ok(format!("Launching {name}")),
    }
}

/// Multiple emulators can be running at once, each on its own adb serial
/// (emulator-5554, emulator-5556, ...), and adb doesn't map "serial" to
/// "AVD name" directly — this asks each running emulator serial which AVD
/// it is via `emu avd name`. Shared by everything that needs to enumerate
/// or target *specific* running devices, rather than "whichever one adb
/// defaults to."
fn running_avd_serials() -> Result<Vec<(String, String)>, String> {
    let out = android_tool(adb_bin())
        .arg("devices")
        .output()
        .map_err(|e| e.to_string())?;
    let devices_text = String::from_utf8_lossy(&out.stdout).to_string();

    let mut result = Vec::new();
    for line in devices_text.lines() {
        let serial = match line.split_whitespace().next() {
            Some(s) if s.starts_with("emulator-") => s,
            _ => continue,
        };
        let avd_out = android_tool(adb_bin())
            .args(["-s", serial, "emu", "avd", "name"])
            .output();
        let Ok(avd_out) = avd_out else { continue };
        let running_name = String::from_utf8_lossy(&avd_out.stdout);
        let running_name = running_name.lines().next().unwrap_or("").trim();
        if !running_name.is_empty() {
            result.push((running_name.to_string(), serial.to_string()));
        }
    }
    Ok(result)
}

pub(super) fn find_serial_for_avd(name: &str) -> Result<String, String> {
    running_avd_serials()?
        .into_iter()
        .find(|(running_name, _)| running_name == name)
        .map(|(_, serial)| serial)
        .ok_or_else(|| format!("Couldn't find a running emulator for \"{name}\""))
}

/// Ground truth for "which AVDs are actually running right now" — an
/// emulator launched by Beo runs detached (see `launch_avd`) specifically
/// so it survives the manager window closing, which means the frontend's
/// own `running` state (only ever updated by its own Launch/Stop clicks
/// during the current session) goes stale the moment Beo is closed and
/// reopened while a device is still up: confirmed live, a device launched
/// in one session showed as "Stopped" with a "Launch" button on the very
/// next app start, even though it was genuinely still running. The
/// frontend calls this on every refresh to reconcile against reality
/// instead of only trusting its own click history.
#[tauri::command]
pub(crate) fn list_running_avds() -> Result<Vec<String>, String> {
    Ok(running_avd_serials()?
        .into_iter()
        .map(|(name, _)| name)
        .collect())
}

/// Stops a running device gracefully via `adb emu kill` rather than killing
/// the emulator/qemu processes directly — `emu kill` lets the emulator
/// clean up its own state (snapshot, lock files) on the way down.
#[tauri::command]
pub(crate) fn stop_avd(name: String) -> Result<String, String> {
    let serial = find_serial_for_avd(&name)?;
    let kill_out = android_tool(adb_bin())
        .args(["-s", &serial, "emu", "kill"])
        .output()
        .map_err(|e| e.to_string())?;
    if !kill_out.status.success() {
        return Err(String::from_utf8_lossy(&kill_out.stderr).to_string());
    }
    Ok(format!("Stopped {name}"))
}

/// Targets a specific device via `-s <serial>`, same as `rotate_avd`/the
/// snapshot commands — without this, `adb install` falls back to whatever
/// device it defaults to (ambiguous, or an outright error, the moment more
/// than one emulator is running at once), silently installing on the wrong
/// device rather than the one the user clicked "Install APK" on.
#[tauri::command]
pub(crate) fn install_apk(name: String, apk_path: String) -> Result<String, String> {
    let serial = find_serial_for_avd(&name)?;
    let out = android_tool(adb_bin())
        .args(["-s", &serial, "install", "-r", &apk_path])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Whether `toggle_avd_mute` has muted this device — used by the frontend
/// to show the right button label on load, since the mute state itself
/// lives in a sidecar file (see `toggle_avd_mute`), not in-memory, and so
/// needs to be queryable independently of the toggle action itself.
#[tauri::command]
pub(crate) fn is_avd_muted(name: String) -> bool {
    avd_dir(&name)
        .map(|dir| dir.join(".beo_muted_volume").exists())
        .unwrap_or(false)
}

/// Parses `adb shell cmd media_session volume --get`'s real output to
/// recover the current stream volume — used to remember what to *restore*
/// on unmute. Falls back to a reasonable mid-range default if the format
/// ever changes, since failing to restore a plausible volume is a much
/// smaller problem than failing to mute would be (muting itself always
/// presses down a fixed, safe count regardless of this value — see
/// `toggle_avd_mute`). Split out so this parsing can be unit tested against
/// real captured output without needing a live device.
///
/// Confirmed live this needed a real regression test, not just eyeballing
/// raw adb output: the real output line is prefixed with `"[V] "` (e.g.
/// `"[V] volume is 11 in range [0..15]"`), which an earlier version of
/// this parser — anchored on `strip_prefix("volume is ")`, requiring an
/// exact match at the start of the line — never actually matched. It was
/// silently falling back to the default on every single real call; nothing
/// caught it until a test using real captured output was added.
fn parse_media_session_volume(stdout: &[u8]) -> u32 {
    String::from_utf8_lossy(stdout)
        .lines()
        .find_map(|l| l.split_once("volume is ").map(|(_, rest)| rest))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(5)
}

/// Presses a hardware volume key `count` times via separate, individual
/// `adb shell input keyevent` invocations. Confirmed live this has to be
/// separate calls, not a single `input keyevent KEY KEY KEY...` batch —
/// that batched form silently drops most of the presses (Android's volume
/// UI debounces rapid synthetic key events arriving faster than it can
/// process them), which looked at first like the whole approach was
/// broken until per-press round-trips (each with its own real adb-shell
/// latency) were confirmed to land reliably.
fn press_volume_key(serial: &str, keycode: &str, count: u32) -> Result<(), String> {
    for _ in 0..count {
        let out = android_tool(adb_bin())
            .args(["-s", serial, "shell", "input", "keyevent", keycode])
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).to_string());
        }
    }
    Ok(())
}

/// Mutes or restores a running device's Android media volume —
/// requested after a real audio investigation made it clear there's no
/// quick way to silence a running AVD without digging through Windows'
/// own per-app volume mixer each time. Lowering `STREAM_MUSIC` (index 3)
/// to 0 has the same practical effect on what reaches the host as turning
/// the phone's own volume down, and works identically regardless of which
/// Windows sound device is default (Bluetooth, wired, HDMI, ...) — a
/// host-level (Windows Core Audio) mute would need a new
/// platform-specific dependency for no real benefit here.
///
/// Confirmed live that `adb shell media volume` (the first approach here)
/// doesn't exist as a shell command on this Android build at all
/// ("inaccessible or not found"), and its apparent modern replacement,
/// `adb shell cmd media_session volume --set`, silently no-ops — `--get`
/// reads the real stream volume correctly, but `--set` reports success
/// while leaving the actual volume completely unchanged (confirmed via
/// `dumpsys audio` immediately after). What does reliably work, confirmed
/// the same way: synthetic `KEYCODE_VOLUME_UP`/`KEYCODE_VOLUME_DOWN` key
/// presses, which is what a real volume rocker sends and so goes through
/// the same path `AudioService` has always handled.
///
/// The pre-mute volume is stored in a small sidecar file inside the AVD's
/// own directory (`.beo_muted_volume`) rather than in-memory Tauri state,
/// so unmuting still restores the right volume even if Beo itself was
/// restarted in between — and `is_avd_muted` above just checks whether
/// that file exists, giving the frontend a simple, restart-safe signal.
#[tauri::command]
pub(crate) fn toggle_avd_mute(name: String) -> Result<bool, String> {
    let serial = find_serial_for_avd(&name)?;
    let sidecar = avd_dir(&name)
        .ok_or_else(|| "couldn't resolve this AVD's directory".to_string())?
        .join(".beo_muted_volume");

    if sidecar.exists() {
        let prev: u32 = std::fs::read_to_string(&sidecar)
            .map_err(|e| e.to_string())?
            .trim()
            .parse()
            .unwrap_or(5);
        press_volume_key(&serial, "KEYCODE_VOLUME_UP", prev)?;
        std::fs::remove_file(&sidecar).map_err(|e| e.to_string())?;
        Ok(false)
    } else {
        let get_out = android_tool(adb_bin())
            .args([
                "-s",
                &serial,
                "shell",
                "cmd",
                "media_session",
                "volume",
                "--stream",
                "3",
                "--get",
            ])
            .output()
            .map_err(|e| e.to_string())?;
        if !get_out.status.success() {
            return Err(String::from_utf8_lossy(&get_out.stderr).to_string());
        }
        let current = parse_media_session_volume(&get_out.stdout);

        // Always press enough times to reach 0 regardless of `current` —
        // STREAM_MUSIC's max index has never been observed above 15 on
        // this build, so 15 presses guarantees real silence even if the
        // parse above fell back to its default (which, unlike here, isn't
        // safe to rely on for *muting*: understating the real volume would
        // leave it audible, defeating the whole point of this button).
        press_volume_key(&serial, "KEYCODE_VOLUME_DOWN", 15)?;
        // Only persist "muted" once the device is actually confirmed
        // silenced — writing this first (the original approach) left the
        // sidecar claiming a mute that might have only partially applied
        // if a press failed partway through, with the frontend never
        // finding out since the command call above would have already
        // returned an error before reaching here.
        std::fs::write(&sidecar, current.to_string()).map_err(|e| e.to_string())?;
        Ok(true)
    }
}

/// Installs and launches Beo's own bundled diagnostics app (see
/// `diagnostics-app/` at the repo root) — audio/GPU/network/storage checks
/// with a controlled, repeatable signal for each, built specifically
/// because comparing behavior across `-gpu`/`-feature` emulator flag
/// experiments by ear/eye alone wasn't reliable. Unlike `install_apk`,
/// there's no file picker: the APK is bundled with Beo itself
/// (`tauri.conf.json`'s `bundle.resources`), so this resolves its own path
/// via Tauri's resource resolver rather than taking one as an argument.
#[tauri::command]
pub(crate) fn install_diagnostics_apk(app: AppHandle, name: String) -> Result<String, String> {
    let serial = find_serial_for_avd(&name)?;
    let apk_path = app
        .path()
        .resolve("resources/beo-diagnostics.apk", BaseDirectory::Resource)
        .map_err(|e| e.to_string())?;
    let apk_path = apk_path.to_string_lossy().to_string();

    let install_out = android_tool(adb_bin())
        .args(["-s", &serial, "install", "-r", &apk_path])
        .output()
        .map_err(|e| e.to_string())?;
    if !install_out.status.success() {
        return Err(String::from_utf8_lossy(&install_out.stderr).to_string());
    }

    let start_out = android_tool(adb_bin())
        .args([
            "-s",
            &serial,
            "shell",
            "am",
            "start",
            "-n",
            "org.beo.diagnostics/.MainActivity",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !start_out.status.success() {
        return Err(String::from_utf8_lossy(&start_out.stderr).to_string());
    }
    Ok("Installed and launched Beo Diagnostics".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `avdmanager list avd` output, captured live from this machine's
    // actual SDK install (2026-09-09) — including a broken entry (its
    // device profile no longer exists in a newer SDK release) with no
    // "Device:" line, which is exactly the shape that broke the original
    // whitespace-splitting parser this replaced.
    const REAL_AVD_LIST: &str = "Available Android Virtual Devices:\n    Name: Medium_Phone_API_36.0\n  Device: medium_phone (Generic)\n    Path: C:\\Users\\ryan\\.android\\avd\\Medium_Phone.avd\n  Target: Google Play (Google Inc.)\n          Based on: Android API 36 Tag/ABI: google_apis_playstore/x86_64\n    Skin: 1080x2400\n  Sdcard: 512M\n---------\n    Name: tes3\n  Device: pixel_tablet (Google)\n    Path: C:\\Users\\ryan\\.android\\avd\\tes3.avd\n  Target: Google Play (Google Inc.)\n          Based on: Android API 36 Tag/ABI: google_apis_playstore/x86_64\n  Sdcard: 512 MB\n\nThe following Android Virtual Devices could not be loaded:\n    Name: Pixel_10_Pro\n    Path: C:\\Users\\ryan\\.android\\avd\\Pixel_10_Pro.avd\n   Error: Google pixel_10_pro no longer exists as a device\n";

    #[test]
    fn parse_avd_list_matches_real_avdmanager_output() {
        assert_eq!(
            parse_avd_list(REAL_AVD_LIST),
            vec![
                AvdInfo {
                    name: "Medium_Phone_API_36.0".into(),
                    category: "phone".into(),
                    disk_usage_mb: None,
                    ram_mb: None,
                },
                AvdInfo {
                    name: "tes3".into(),
                    category: "tablet".into(),
                    disk_usage_mb: None,
                    ram_mb: None,
                },
                AvdInfo {
                    name: "Pixel_10_Pro".into(),
                    category: "phone".into(),
                    disk_usage_mb: None,
                    ram_mb: None,
                },
            ]
        );
    }

    // Two real `hw.ramSize` values captured from actual devices on this
    // machine — one bare, one with a unit suffix. The suffixed one is
    // exactly what the original naive `parse::<u32>()` silently failed on.
    #[test]
    fn parse_size_to_mb_handles_bare_number() {
        assert_eq!(parse_size_to_mb("2048"), Some(2048));
    }

    #[test]
    fn parse_size_to_mb_handles_m_suffix() {
        assert_eq!(parse_size_to_mb("1536M"), Some(1536));
    }

    #[test]
    fn parse_size_to_mb_handles_g_suffix() {
        assert_eq!(parse_size_to_mb("6G"), Some(6144));
    }

    #[test]
    fn parse_size_to_mb_is_case_insensitive_on_suffix() {
        assert_eq!(parse_size_to_mb("512m"), Some(512));
        assert_eq!(parse_size_to_mb("2g"), Some(2048));
    }

    #[test]
    fn parse_size_to_mb_rejects_garbage() {
        assert_eq!(parse_size_to_mb(""), None);
        assert_eq!(parse_size_to_mb("not a number"), None);
    }

    // Real `adb shell cmd media_session volume --get` output, captured live
    // from an actual running device this session — mixed in among other
    // `[V]`-prefixed verbose logging, not on its own line by itself.
    const REAL_VOLUME_GET_OUTPUT: &[u8] = b"[V] will control stream=3 (STREAM_MUSIC)\n[V] will get volume\n[V] Connecting to AudioService\n[V] volume is 11 in range [0..15]\n";

    #[test]
    fn parse_media_session_volume_matches_real_output() {
        assert_eq!(parse_media_session_volume(REAL_VOLUME_GET_OUTPUT), 11);
    }

    #[test]
    fn parse_media_session_volume_handles_zero() {
        assert_eq!(
            parse_media_session_volume(b"[V] volume is 0 in range [0..15]\n"),
            0
        );
    }

    // Falls back to a mid-range default rather than 0 — confirmed live this
    // matters: the fallback here is only ever used to decide what to
    // *restore* on unmute (muting itself always uses a fixed safe press
    // count regardless), and restoring to silence would be a worse outcome
    // than restoring to a plausible non-zero guess.
    #[test]
    fn parse_media_session_volume_falls_back_to_default_on_unexpected_format() {
        assert_eq!(parse_media_session_volume(b""), 5);
        assert_eq!(
            parse_media_session_volume(b"not the expected output at all"),
            5
        );
    }
}
