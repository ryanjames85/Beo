# Beo

*Beo* (Irish, "alive") — a clean, open-source Android emulator manager and
installer. No bundled adware, no gaming-emulator cruft, no IDE required.

## Why

Most "Android emulators" people find online (BlueStacks-alikes, sketchy
APK sites) bundle adware or worse. The clean alternative — Android
Studio's Device Manager — drags in an entire IDE. Beo wraps the same
official `sdkmanager` / `avdmanager` / `emulator` binaries Google ships,
with nothing else attached, behind a normal installer and a simple
dashboard.

## Stack

- **Tauri 2** (Rust backend, shells out to the official SDK CLI tools)
- **React + TypeScript** frontend
- Persistent data dir per OS ("nuke" command wipes it cleanly, devices included)
- Light / Dark / System theming (CSS custom properties, follows OS by default)

## Requirements to build

- Node.js 18+
- Rust toolchain (`rustup`)
- Platform build deps per [Tauri prerequisites](https://tauri.app/start/prerequisites/)

No system JDK is required — Beo downloads and manages its own (see below).

## Dev

```bash
npm install
npm run tauri dev   # or: npm start
```

## Build installers

```bash
npm run tauri build
```

Produces NSIS/MSI on Windows, DMG on macOS, AppImage/deb on Linux —
standard installer UX per platform, per `tauri.conf.json`.

## How it works

1. First launch checks `sdk_status` — if the cmdline-tools binary isn't
   present in Beo's persistent data dir, shows the Install screen. Setup
   also checks for a JDK and internet connectivity (a real reachability
   check against `dl.google.com`, not just "is there a network
   interface") and warns before you hit a doomed download.
2. `install_sdk` downloads Google's official cmdline-tools zip, extracts
   it, accepts licenses non-interactively, and pulls `platform-tools` +
   `emulator` — with a real progress bar (parsed from `sdkmanager`'s own
   output) and a working Cancel button.
3. Dashboard lists installed virtual devices (`avdmanager list avd`), each
   showing its **real disk usage and RAM** — the actual size of everything
   under that device's own folder (images, snapshots), not the size
   `config.ini` declares, which can understate it a lot once a snapshot
   exists (confirmed by hand: a device with a 6G declared data partition
   was genuinely using 11G on disk). A **low disk space** warning appears
   before creating a new device if free space looks tight — informational,
   not a hard cap, since device sizes vary too widely for a precise
   guarantee. **Simple mode** auto-picks a system image from a small,
   hand-verified list of known-stable API levels rather than just
   "whatever's numerically newest" — Google ships preview/beta-channel
   system images too (marked with a decimal API level like `37.0` rather
   than a plain integer), and blindly picking the newest number once
   picked one of those, which crashed on boot. **Developer mode** exposes
   the full image/ABI/device picker.
4. Launch (`emulator -avd <name>`), Stop (`adb emu kill`, not a raw process
   kill — lets the emulator clean up its own state), rotate, save/load/
   delete snapshots, or sideload an APK via a native file picker + `adb
   install`. A device card shows **"Starting…"** (a pulsing dot) until the
   guest OS actually finishes booting — not just "the process started" —
   then **"Running"** (solid dot); Rotate/Install APK/Snapshots stay
   disabled until then, since trying any of them mid-boot just fails.
   Launching a device that's already running is rejected immediately with
   a clear message rather than silently doing nothing: the emulator itself
   refuses a second instance of the same AVD a few seconds into startup
   ("running multiple emulators with the same AVD is experimental"),
   which used to surface as a false "Launching…" success with no visible
   error once the duplicate quietly died. A live debug panel also streams
   the emulator's own boot log in real time and requires boot-completion
   to hold for several seconds, not just one instantaneous check — a
   device can report "booted" once and then crash-loop — rather than
   staying silent and letting you guess whether it's actually ready.
5. Beo's own `adb`/`sdkmanager`/`avdmanager`/`emulator` calls are pinned to
   Beo's own SDK install, Beo's own dedicated `adb` server port, and Beo's
   own bundled JDK (an Eclipse Temurin JRE, downloaded and extracted into
   Beo's data dir as part of `install_sdk`, never the system Java) —
   deliberately, so a separate Android Studio (or other SDK/JDK) install on
   the same machine never gets its environment variables, `adb` server, or
   Java runtime confused with Beo's.
6. **Nuke all data** removes the entire persistent data directory (SDK,
   images) *and* the actual AVD devices under `~/.android/avd` — the two
   used to live in different places, and nuking only used to clear the
   former.
7. Settings' **About** section shows the running version and a "Check for
   updates" button that compares GitHub's latest release against it and
   opens the release page for a manual download — no auto-installer yet
   (see roadmap), but every failure mode (no releases published, a
   network error, an unexpected response) shows a real message with a
   Retry button rather than failing silently.

## Roadmap / open issues

- [ ] Frontend test suite (Rust side has one — `cargo test` in
      `src-tauri`, covering name sanitization, the profanity filter,
      progress-percentage parsing, and several real-output-fixture parser
      tests; nothing on the TypeScript side yet)
- [ ] `install_apk` doesn't target a specific device serial — fine with
      one emulator running, ambiguous with several at once (`rotate_avd`
      and everything else that talks to a running device already does)
- [ ] The hand-verified list of known-stable system-image API levels needs
      manual updates over time as new Android versions are released and
      actually confirmed to boot cleanly
- [ ] Drag-and-drop APK install (currently a file-picker button, not drag-drop)
- [ ] macOS Gatekeeper quarantine handling on downloaded binaries
- [ ] CI-built signed releases, and a real in-app auto-updater
      (`tauri-plugin-updater`) once signing is in place — today's "Check
      for updates" (Settings → About) only checks and links out to a
      manual download; see `TODO.md`

See `TODO.md` for the full PoC → alpha/beta hardening plan (CI, download
integrity, module structure, test coverage, error handling) and what's
already done.

## License

MIT — see [LICENSE](./LICENSE).

## Contributing

PRs welcome. This is a young project — the CLI-orchestration core
(`src-tauri/src/{util,jdk,sdk,avd,ide}.rs`) is the part most worth
reviewing carefully before relying on it. See `CONTRIBUTING.md` for where
things live and what to test before opening a PR.

Repo: https://github.com/ryanjames85/Beo
