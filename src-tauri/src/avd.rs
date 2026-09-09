use crate::util::{adb_bin, android_tool, cmdline_tools_bin, emulator_bin};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

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

#[derive(Serialize, Deserialize, Clone)]
pub struct DeviceProfile {
    id: String,
    label: String,
    category: String, // "phone" or "tablet"
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

/// Same "does the device profile id look like a tablet" heuristic as
/// `list_device_profiles` — kept in sync with it deliberately, since a
/// profile picked there is what ends up here after `create_avd`.
fn category_for_device_id(device_id: &str) -> &'static str {
    if device_id.contains("tablet") || device_id.contains("pad") {
        "tablet"
    } else {
        "phone"
    }
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

/// avdmanager only accepts `[A-Za-z0-9._-]` in AVD names — anything else
/// (spaces, emoji, punctuation) either gets rejected outright or produces a
/// device whose on-disk name silently diverges from what was typed. Beo's
/// "name it anything" UX promises free-text input, so this maps that input
/// into a valid name instead of forwarding it as-is to the CLI.
pub(crate) fn sanitize_avd_name(raw: &str) -> Result<String, String> {
    let mut out = String::with_capacity(raw.len());
    let mut last_was_underscore = false;
    for c in raw.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
            out.push(c);
            last_was_underscore = c == '_';
        } else if !last_was_underscore {
            out.push('_');
            last_was_underscore = true;
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    let truncated: String = trimmed.chars().take(60).collect();
    if truncated.is_empty() {
        return Err("Device name needs at least one letter or number.".into());
    }
    if contains_blocked_word(&truncated) {
        return Err("That name isn't allowed — please choose something else.".into());
    }
    Ok(truncated)
}

// Deliberately small and word-boundary-matched rather than a substring scan
// — a substring check would block innocent names for containing a bad
// word as a fragment (e.g. "classic" contains "ass"), which is the classic
// failure mode of naive profanity filters. Checked against whole tokens of
// the *sanitized* name (already split on any non [A-Za-z0-9._-] character),
// so "bad_word" is checked as ["bad", "word"], not as one long string.
const BLOCKED_WORDS: &[&str] = &[
    "fuck", "shit", "bitch", "asshole", "cunt", "nigger", "nigga", "faggot", "retard", "whore",
    "slut", "dick", "piss", "cock", "pussy", "bastard",
];

pub(crate) fn contains_blocked_word(sanitized_name: &str) -> bool {
    sanitized_name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| BLOCKED_WORDS.contains(&token.to_ascii_lowercase().as_str()))
}

#[tauri::command]
pub(crate) fn create_avd(name: String, image_id: String, device: String) -> Result<String, String> {
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
/// `-gpu` is deliberately pinned to `swiftshader_indirect` (software
/// rendering) rather than left as `auto`. `auto` picks `gfxstream` (host
/// GPU passthrough) when it looks available, but on at least some
/// virtualized/sandboxed hosts — confirmed by hand here: WHPX genuinely
/// operational, `-gpu auto` still hangs forever at the boot logo with
/// `adb` reporting the device "offline" indefinitely, no error, nothing in
/// the UI to explain it. The same AVD boots fully with
/// `-gpu swiftshader_indirect` instead — slower rendering, but it actually
/// works, which beats an indefinite silent hang. This is also Android's
/// own recommended setting for CI/headless/virtualized environments.
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
        "swiftshader_indirect".to_string(),
    ];
    if headless {
        args.push("-no-window".to_string());
    }
    if !share_clipboard {
        args.push("-no-clipboard-sharing".to_string());
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

fn find_serial_for_avd(name: &str) -> Result<String, String> {
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

#[tauri::command]
pub(crate) fn install_apk(apk_path: String) -> Result<String, String> {
    let out = android_tool(adb_bin())
        .args(["install", "-r", &apk_path])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_avd_name_replaces_disallowed_chars() {
        assert_eq!(sanitize_avd_name("My tablet").unwrap(), "My_tablet");
        assert_eq!(
            sanitize_avd_name("weird name!! @@ ##").unwrap(),
            "weird_name"
        );
    }

    #[test]
    fn sanitize_avd_name_collapses_runs_and_trims_edges() {
        // '.', '-', '_' are all allowed characters, so only leading/trailing
        // *underscores* get trimmed — a leading/trailing '.' is left as-is,
        // since it's just as valid mid-name as at the edges.
        assert_eq!(sanitize_avd_name("  __leading").unwrap(), "leading");
        assert_eq!(sanitize_avd_name("trailing__  ").unwrap(), "trailing");
        assert_eq!(sanitize_avd_name("a   b").unwrap(), "a_b");
    }

    #[test]
    fn sanitize_avd_name_allows_dots_dashes_underscores() {
        assert_eq!(
            sanitize_avd_name("my.device-1_test").unwrap(),
            "my.device-1_test"
        );
    }

    #[test]
    fn sanitize_avd_name_rejects_empty_result() {
        assert!(sanitize_avd_name("!!! @@@ ###").is_err());
        assert!(sanitize_avd_name("   ").is_err());
    }

    #[test]
    fn sanitize_avd_name_truncates_to_60_chars() {
        let long = "a".repeat(100);
        assert_eq!(sanitize_avd_name(&long).unwrap().len(), 60);
    }

    #[test]
    fn sanitize_avd_name_blocks_profanity_after_sanitizing() {
        // "shit device" sanitizes to "shit_device" before the blocklist
        // check runs — this confirms the check happens on the sanitized
        // form, not the raw input.
        assert!(sanitize_avd_name("fuck_this").is_err());
        assert!(sanitize_avd_name("shit device").is_err());
    }

    #[test]
    fn contains_blocked_word_matches_whole_tokens_only() {
        assert!(contains_blocked_word("fuck_device"));
        assert!(contains_blocked_word("my_shit_phone"));
        assert!(!contains_blocked_word("classic_assistant")); // contains "ass" as a substring, not a token
        assert!(!contains_blocked_word("my_tablet"));
    }

    #[test]
    fn contains_blocked_word_is_case_insensitive() {
        assert!(contains_blocked_word("FUCK"));
        assert!(contains_blocked_word("FuCk_device"));
    }

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
}
