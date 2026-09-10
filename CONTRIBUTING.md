# Contributing to Beo

Thanks for considering it — this is a young project and there's a lot of
useful first-issue material in the [README roadmap](./README.md#roadmap--open-issues).

## Setup

```bash
npm install
npm run tauri dev
```

You'll need Rust (`rustup`) and the platform build deps listed in
[Tauri's prerequisites](https://tauri.app/start/prerequisites/).

## Before opening a PR

- `cargo fmt --manifest-path src-tauri/Cargo.toml` and
  `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` should
  both pass clean — CI enforces this.
- `npm run build` should typecheck without errors.
- `npm run test:e2e:jdk` runs a real, isolated end-to-end check of the JDK
  bootstrap (`scripts/e2e-jdk-bootstrap.mjs`) — builds a standalone debug
  bundle, launches it against a disposable `BEO_DATA_DIR` (never your real
  install), and drives it over CDP to confirm `install_sdk` actually
  downloads and wires up Beo's own JDK. It does a real ~230MB network
  download, so it's not meant to run on every save — run it before a
  release or after touching `ensure_jdk`/`android_tool`/`check_java`.
  Windows-only for now (relies on WebView2's remote-debugging support).
- If you touch anything under `src-tauri/src/` (the CLI-orchestration
  core), explain in the PR description what you tested it against — which
  OS, which SDK tool versions. These modules shell out to Google's
  binaries directly, so behavior changes here are worth extra scrutiny.

## Where things live

The Rust backend is split by domain (each has its own `#[cfg(test)]`
module — run all of them with `cargo test --manifest-path src-tauri/Cargo.toml`):

- `src-tauri/src/lib.rs` — thin entry point: `mod` declarations and the
  `invoke_handler!` wiring. Nothing else lives here on purpose.
- `src-tauri/src/util.rs` — shared low-level plumbing every other module
  depends on: `data_root()`/`sdk_root()`, `android_tool()` (pins every
  spawned SDK process to Beo's own SDK root, `adb` port, and JDK),
  `verify_sha256()`, the `SdkTask` cancellation state, `nuke_all`, and
  `check_disk_space` (best-effort free-space check, shelling out to
  PowerShell's `System.IO.DriveInfo` on Windows / `df -Pk` elsewhere —
  no cross-platform disk-space API in std, and this avoids a new
  dependency for one check).
- `src-tauri/src/jdk.rs` — JDK bootstrap. `ensure_jdk()` downloads and
  extracts Beo's own JRE (Eclipse Temurin) the first time `install_sdk`
  runs — there is no dependency on a system-installed JDK anywhere here.
- `src-tauri/src/sdk.rs` — SDK install/download/cancel, plus the
  pre-flight checks shown on the setup screen (`check_hardware_accel`,
  `check_network`, `preferred_abi`). Both this and `ensure_jdk`'s
  downloads, plus the license-acceptance step, check `SdkTask.cancelled`
  so the Cancel button actually works throughout the whole install, not
  just once `sdkmanager` itself is running.
- `src-tauri/src/avd/` — the AVD lifecycle, split by responsibility:
  `naming.rs` (name sanitizer/profanity filter), `profiles.rs` (device
  profile listing), `rotation.rs` (rotate + rotation parsing),
  `snapshots.rs` (save/load/delete/list), and `lifecycle.rs` (the core:
  create/launch/stop/delete, disk/RAM lookups, `find_serial_for_avd`).
  `mod.rs` just declares the submodules and re-exports — glob re-exports
  specifically, since `#[tauri::command]` leaves a hidden helper item next
  to each command function that a named re-export would miss. `avd_dir()`
  (in `lifecycle.rs`) is the one place that resolves an AVD's real on-disk
  folder — reads it from `<name>.ini`'s `path=` line rather than assuming
  `<name>.avd`, since those can genuinely differ (confirmed on a real
  device on this machine). Real per-device disk usage (walking the
  folder) and RAM (`config.ini`'s `hw.ramSize`, which can carry a K/M/G
  suffix — `parse_size_to_mb` handles that) are both surfaced through
  `list_avds`. `launch_avd` rejects a duplicate launch immediately
  (checked via `find_serial_for_avd`) rather than spawning a second
  emulator process that the emulator itself would reject a few seconds
  into startup — confirmed live, that used to report false success. Its
  boot-completion poll emits both `avd_log` (a text line, for the debug
  panel) and `avd_booted` (a structured `{name}` event) — the frontend
  uses the latter to flip a device's dashboard status from "Starting…" to
  "Running," rather than parsing log text.
- `src-tauri/src/ide.rs` — `ANDROID_HOME` integration for external IDEs
  and the "which IDEs are running" detection.
- `src/App.tsx` — the setup screen + dashboard, all app state, and the
  Simple-mode image-selection logic (`recommendedImage`,
  `VERIFIED_API_LEVELS`). Exports `sanitizeAvdName`/`containsBlockedWord`
  (mirrors the Rust-side validation) for `CreateDeviceForm.tsx` to reuse.
- `src/DeviceCard.tsx` — one device's card on the dashboard (status,
  actions, expandable snapshot panel). Takes the device and every
  callback as props — no state of its own.
- `src/CreateDeviceForm.tsx` — the "New device"/"Create a device"
  section, both Simple and Developer variants.
- `src/Settings.tsx` — theme, clipboard, IDE integration, the About
  section (version, license, "Check for updates" against GitHub's latest
  release), and the debug-only "reset app" flow.
- `src/style.css` — CSS custom properties for theming (light/dark), the
  rest is fairly minimal.
- `src-tauri/capabilities/default.json` — Tauri v2's permission grants
  (`dialog:default` for the APK file picker, `shell:allow-open` for
  opening GitHub links from Settings, etc.). If you add a use of
  `listen()`/`emit()`/a plugin API and it silently does nothing, check
  here first — a missing permission fails quietly rather than with an
  obvious error.
- `scripts/e2e-jdk-bootstrap.mjs` — the one real end-to-end test, see
  above. `BEO_DATA_DIR` (an env var `data_root()` in `util.rs` checks
  first) is what makes it possible to run this in full isolation from a
  real install, including a real one already running dev/side by side.

## Good first issues

Check the README roadmap. Multi-emulator serial targeting for
`install_apk` (`rotate_avd` and the snapshot commands already target a
specific serial) is standalone and doesn't require deep familiarity with
the rest of the codebase. There's a Vitest/Testing Library suite on the
frontend (`src/*.test.ts(x)`) alongside the Rust unit tests and one real
e2e test — new pure-logic functions or components should generally come
with a test in the same style.

## Reporting bugs

Include your OS, the output of `sdkmanager --version` if the SDK is
already installed, and whatever error text Beo showed you. Turn on the
**Debug** toggle in the dashboard header before reproducing — it shows a
live log of every backend call, including the emulator's own boot output,
with a Copy button, which is usually the fastest way to actually diagnose
an emulator problem.

If the emulator fails to launch outright, that's often a hardware-
acceleration issue (KVM/HAXM/WHPX) — check the dashboard's own warning
banner first. If it launches but boots into a repeating animation/crash
loop, that's a different problem (usually a bad system image, not
acceleration) — pull the actual guest log with `adb logcat`, not just
Beo's own debug panel, since a crash loop can still report
`sys.boot_completed=1` in between crashes and look "fine" from the host
side alone.

## Code of conduct

Be respectful, assume good faith, keep discussion focused on the code.
