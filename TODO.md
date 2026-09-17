# Beo: PoC → alpha/beta hardening

Tracking the path from proof-of-concept to alpha/beta. Nothing here changes
app features — it's the safety net around the ones that already exist.
Ordered cheapest/highest-leverage first; each phase is independently
shippable.

## Phase 0 — CI actually exercises the code ✅ done (2026-09-09)
- [x] `cargo test` runs in CI (currently only `fmt`/`clippy` do — the 19 unit
      tests only run when someone remembers to run them locally)
- [x] CI compiles on a `[ubuntu, windows, macos]` matrix, not just ubuntu —
      most Windows-only `#[cfg]` code is currently never compiled by PR checks
- [x] Fix whatever `fmt`/`clippy` surface once they run on Windows/macOS too
      — found and fixed: `shell_profile_path`/`MARKER_START`/`MARKER_END`
      were unconditionally-defined but only used on macOS/Linux, so Windows
      compiles flagged them dead (now `#[cfg(not(target_os = "windows"))]`-
      gated); 3 `needless_return` lints in the Windows-only IDE-integration
      branches; and `cargo fmt --check` itself had 18 pre-existing diffs
      that were platform-agnostic (would have failed on ubuntu too) — ran
      `cargo fmt` to clear them. Verified clean on Windows (this machine):
      `cargo check`, `clippy -D warnings`, `fmt --check` all pass, 19/19
      tests pass. Not yet verified on actual macOS/Linux CI runners — that
      only happens once this lands and a PR/push triggers the workflow.

## Phase 1 — Integrity of what Beo downloads ✅ done (2026-09-09)
- [x] SHA-256 verification for the cmdline-tools zip download — hash
      computed directly from the actual pinned build (11076708; Google's
      download page only publishes a checksum for the current "latest"
      build, which is a newer one, so this is trust-on-first-use like a
      lockfile hash, not scraped from an unversioned page)
- [x] SHA-256 verification for the JDK (Temurin JRE) download — Adoptium
      publishes a `.sha256.txt` sidecar per release asset; cross-checked
      one (Windows) against an independently computed hash of the actual
      downloaded file before trusting the rest
- [x] Fail the install cleanly on a checksum mismatch instead of extracting
      — new `verify_sha256()` in `lib.rs`, wired into both `ensure_jdk` and
      `install_sdk` right after download completes, before extraction;
      deletes the bad file on mismatch. 2 new unit tests
      (`verify_sha256_accepts_matching_hash`,
      `verify_sha256_rejects_mismatch_and_deletes_file`) prove both paths
      directly. Verified live: real e2e install still succeeds
      (`npm run test:e2e:jdk`) with checksum verification now in the path.
      21/21 tests pass, clippy/fmt clean.

## Phase 2 — Backend module split ✅ done (2026-09-09)
- [x] Split `lib.rs` (2219 lines) into `util.rs` (shared plumbing:
      `data_root`/`android_tool`/`verify_sha256`/`SdkTask`/`nuke_all`),
      `jdk.rs` (JDK bootstrap), `sdk.rs` (SDK install/download/cancel +
      pre-flight checks), `avd.rs` (AVD lifecycle, 783 lines — the
      largest, but a genuinely single-responsibility domain), and
      `ide.rs` (IDE integration). `lib.rs` is now 53 lines: just `mod`
      declarations and the `invoke_handler!` wiring. No command
      signatures changed, so the frontend needed zero changes. Also
      dropped `PortForward` — a struct defined once, referenced nowhere
      (Rust nor frontend), found dead while mapping the split.
- [x] Move tests alongside the code they test — each module has its own
      `#[cfg(test)]` (jdk.rs: JDK path helpers; sdk.rs: parse_percent/
      cleanup; avd.rs: name sanitization/profanity filter; util.rs:
      data_root override + the two checksum tests from Phase 1). 21/21
      still pass, same count as before the split.
- Verified: `cargo check`/`clippy -D warnings`/`fmt --check` all clean
  after the split (one unused-import warning surfaced and was fixed —
  `ide.rs`'s `PathBuf` import needed the same `cfg(not(windows))` gate as
  the function using it). Live end-to-end: rebuilt the standalone bundle
  and re-ran `npm run test:e2e:jdk` (still passes — real install, real
  JDK download, through the split modules) plus a direct CDP spot-check
  of `list_avds`/`list_device_profiles`/`list_running_avds`/
  `ide_integration_status`/`sdk_status`/`preferred_abi`/
  `detect_connected_ides` against the real install, all returning
  correct real data through the new module boundaries.

## Phase 3 — Frontend structure ✅ done (2026-09-09)
- [x] Extract the AVD card into `DeviceCard.tsx` — the per-device card,
      actions, and expandable snapshot panel, taking the device + all
      callbacks as props (state stays owned by `App.tsx`, prop-drilled
      down, per the plan — no state-management rework)
- [x] Extract the create-device form into `CreateDeviceForm.tsx` — both
      Simple and Developer variants as a discriminated union on `mode`,
      sharing one `NameHint` component for the validation text that used
      to be duplicated between them
- `App.tsx` 1045 → 853 lines. `sanitizeAvdName`/`containsBlockedWord`
  exported from `App.tsx` and imported into `CreateDeviceForm.tsx` rather
  than duplicated, so the "mirrors the Rust backend" logic still has one
  source of truth. Verified: `tsc --noEmit` and `npm run build` both
  clean; live-checked via CDP against the real install — dashboard device
  cards, Simple mode's create form, and Developer mode's full image/ABI/
  profile picker (with real `sdkmanager`-listed images and
  `avdmanager`-listed device profiles) all render correctly through the
  new components, no visual or behavioral change.

## Phase 4 — Fixture-based parser tests ✅ done (2026-09-09)
- [x] Pin real captured output for `avdmanager list avd`, `sdkmanager --list`,
      `dumpsys window`'s rotation line, and `emu avd snapshot list` as test
      fixtures. Each parser was split out of its command function into a
      pure `parse_*` function first (`parse_avd_list`, `parse_available_images`,
      `parse_rotation`, `parse_snapshot_list`), then tested against real
      output captured live from this machine — the `avdmanager list avd`
      fixture includes a genuinely broken entry (device profile removed in
      a newer SDK release) with no `Device:` line, and the `sdkmanager
      --list` fixture includes two rows for the same image with different
      column padding widths, both real, both exactly the kind of case that
      broke earlier parsers in this project.
- **Found a real bug writing the rotation fixture, before it ever shipped
  wrong:** `dumpsys window` reports rotation in *degrees*
  (`ROTATION_0/90/180/270` — confirmed live: one `adb emu rotate` from
  portrait produced `ROTATION_270`, not `ROTATION_1`), but `parse_rotation`
  read only the first digit after `ROTATION_` and used it directly as the
  0-3 step index. That's correct by coincidence for `ROTATION_0` (the only
  case ever manually verified so far) but silently wrong for 90/180/270
  (e.g. "270" → reads "2" → treated as 180°). Fixed to parse the full
  number and divide by 90. Verified two ways: the fixture test itself
  (fails on the old code, passes on the fix), and live through the actual
  compiled `rotate_avd` IPC call — rotated a real device landscape then
  back to portrait (the exact sequence the bug broke, since the second
  call's "current" is genuinely 270° not 0°) and confirmed via
  `dumpsys window` it landed correctly on `ROTATION_0`. This is also a
  gap in earlier work worth noting: the original rotate_avd fix a few
  passes back was "verified live" only via manual `adb` calls simulating
  the mechanism, never through the actual compiled Rust code on a second
  rotation — so it looked confirmed without actually having been exercised.
- 28/28 tests pass (7 new: 1 avd list, 3 rotation, 2 snapshot list, 1
  available-images).

## Phase 5 — Structured errors ✅ done (2026-09-09)
- [x] Replace `Result<T, String>` with a serializable `AppError` enum —
      scoped deliberately to the two commands whose failure *kind* the
      frontend actually branches on (`install_sdk`, `download_image`,
      plus the shared `run_sdkmanager_streaming`/`ensure_jdk` they call
      through). The other ~26 commands stay `Result<T, String>` — the
      frontend only ever displays their errors verbatim, so converting
      them would be churn with no behavior to fix. `AppError` lives in
      `util.rs`: `Cancelled` (unit), `Network(String)`, `Other(String)`,
      with `#[serde(tag = "kind", content = "message")]` so a rejected
      `invoke()` resolves to `{kind: "Cancelled"}` or `{kind: "Network",
      message: "..."}` in JS. `From<String>`/`From<&str>`/`From<reqwest::Error>`
      mean every existing `?`/`.map_err(|e| e.to_string())?` site kept
      working unchanged — only the two explicit `"Cancelled".into()` sites
      needed to become `AppError::Cancelled` (that `.into()` would
      otherwise have silently produced `Other("Cancelled")`, defeating
      the whole point).
- [x] Frontend matches on `error.kind` instead of substring-matching error
      text — `fail()` in `App.tsx` now checks for the structured shape
      first (`isStructuredAppError`) and only falls back to the old
      `raw.includes("Cancelled")`/`isNetworkError()` text-matching for
      the other commands that still reject with plain strings.
- Verified live, not just compiled: triggered a real cancellation through
  the actual `install_sdk` → `cancel_sdk_task` flow (waited for the JDK
  download to finish as a proxy for "past the initial cmdline-tools zip
  download, into the cancellable sdkmanager phase," then cancelled) and
  confirmed the real rejected promise value was exactly
  `{"kind":"Cancelled"}` — proving the serde shape, Tauri's IPC delivery,
  and the frontend's handling all actually connect end to end. 28/28
  tests, clippy, fmt, and `tsc --noEmit` all clean.

---

**All 5 phases of the PoC → alpha/beta hardening roadmap are now done.**
Remaining, deferred by design (see below): code signing/notarization and
an auto-updater, both flagged as needing a deliberate decision (cost,
process) rather than being silently scoped into this pass.

## Deferred (needs a decision first, not just code)
- Code signing / notarization — real cost + process change, discuss before scoping
- **Real in-app auto-updater** (`tauri-plugin-updater`, signed artifacts,
  download-and-install) — still deferred; needs the signing-key decision
  above first. What shipped instead (2026-09-09) is a lighter, unsigned
  version: an **About section** in Settings (app version via
  `getVersion()`, MIT license, "View on GitHub") plus a **"Check for
  updates" button** that compares GitHub's latest release
  (`api.github.com/repos/ryanjames85/Beo/releases/latest`) against the
  running version and opens the release page (via the new
  `shell:allow-open` capability) for a manual download — no signing keys,
  no CI changes, no auto-download. Handles every real failure mode
  explicitly rather than failing silently: no releases published yet
  (404 — this repo's actual current state, verified live against the
  real GitHub API), an unexpected HTTP status, a malformed response
  missing a version tag, and a network failure, each with its own message
  and (except the malformed-response case, which is store-side) a Retry
  button. Verified live end-to-end against the real API, including the
  real 404 path.

Full rationale and file-level detail: see the review in conversation, or ask
to regenerate this from the plan.

---

# Second batch: frontend tests, daily update check, Vite upgrade, avd.rs split

First commit landed and is pushed to GitHub (`ryanjames85/Beo`). This batch
follows a second honest review: zero frontend test coverage, `npm audit`
flagging Vite/esbuild (dev-only, but unpatched), and `avd.rs` having grown
to 1086 lines. No code signing being pursued — updates stay check-and-link
only, but should also run automatically once a day, not just on demand.

## Phase 1 — Vite v5 → v8 upgrade ✅ done (2026-09-09)
- [x] Bumped `vite` to `^8.2.2`, `@vitejs/plugin-react` to `^6.1.1`
- [x] `npm audit` clean afterward (was 2 moderate/high, both dev-only)
- [x] Verified live: `npm run build`, standalone `vite` dev server, and a
      full real `tauri dev` session (compiled, launched, real `invoke()`
      calls against the real backend, correct device data rendered) —
      not just "it installed."

## Phase 2 — Frontend test coverage (Vitest + Testing Library) ✅ done (2026-09-09)
- [x] Added vitest 5 + @testing-library/react 16 + jsdom — note: jsdom
      **30.x** has an engine range (`^24.15.0`) narrower than this
      machine's actual Node (24.14.0) and warns/risks breaking; pinned
      jsdom to **29.1.1** instead (`>=24.0.0`, actually satisfied). CI's
      `setup-node` bumped from 20→22 for the same reason (jsdom 29 wants
      `^22.13.0` within the 22 line).
- [x] `test`/`test:watch` scripts; `vite.config.ts` gained a `test` block
      (jsdom env, `src/test/setup.ts` for jest-dom matchers + RTL's
      `afterEach(cleanup)` — the latter isn't automatic and its absence
      caused every component test file's later tests to see prior tests'
      still-mounted DOM and fail with "found multiple elements," caught
      immediately on first run).
- [x] 35 tests total: pure-logic tests for `sanitizeAvdName`,
      `containsBlockedWord`, `recommendedImage`, `isNetworkError`
      (App.tsx — all now exported for testing), `isNewerVersion`
      (Settings.tsx), `formatMb` (DeviceCard.tsx); component tests for
      `DeviceCard` (Stopped/Starting/Running, action-button gating, real
      disk/RAM display) and `CreateDeviceForm` (Simple vs. Developer,
      `NameHint` validation states).
- [x] Verified the tests actually catch regressions, not just pass:
      temporarily disabled `recommendedImage`'s preview-build exclusion
      regex — the exact fixture test built from the real `android-37.0`
      incident failed immediately, correctly, then passed again once
      reverted.
- [x] Wired `npm test` into `ci.yml` alongside `cargo test`, same matrix.

## Phase 3 — Daily automatic update check ✅ done (2026-09-09)
- [x] Moved update-check state/logic (`GITHUB_REPO`/`GITHUB_URL`,
      `checkForUpdates`, `openReleasePage`, `version`/`versionError`/
      `updateCheck`/`copyLinkHint`) from `Settings.tsx` up to `App.tsx`;
      `Settings` is now a pure props consumer (`isNewerVersion` and the
      `UpdateCheck` type stay exported from `Settings.tsx` since they're
      pure/type-only and `Settings.test.ts` already depends on that path).
- [x] `beo-last-update-check` timestamp in localStorage; a `useEffect`
      keyed on `version` fires `checkForUpdates()` once on mount if the
      stored timestamp is missing or >24h old, then records a fresh one —
      same check function, same error handling as the manual button.
- [x] Quiet dashboard indicator: a small `.status-dot` badge on the header
      Settings button (title also updates to name the version) when
      `updateCheck.status === "available"` — no popup/banner; the manual
      "Check for updates" button in Settings is unchanged.
- [x] Verified live via CDP against a real `tauri dev` session: injected a
      fetch mock via `Page.addScriptToEvaluateOnNewDocument` (so it's in
      place before the app's own mount effect runs — an initial attempt
      that mocked `fetch` *after* reload lost the race with the real
      effect and gave a false negative) with a cleared timestamp, reloaded,
      and confirmed the background check ran unprompted, the timestamp was
      recorded, the badge appeared on the Settings button, and opening
      Settings showed the full "Update available: v99.0.0" detail sourced
      from the lifted App-level state.

## Phase 4 — Split `avd.rs` (1086 lines) into a directory module ✅ done (2026-09-09)
- [x] `avd/{mod,naming,profiles,rotation,snapshots,lifecycle}.rs` —
      mechanical, no behavior/signature changes, tests moved with their code
      (33/33 unchanged). `naming.rs` (sanitize/blocklist), `profiles.rs`
      (device profiles + `category_for_device_id`), `rotation.rs`
      (rotate_avd + its parser), `snapshots.rs` (CRUD + parser),
      `lifecycle.rs` (the core: AvdInfo, list/create/delete/launch/stop_avd,
      disk/RAM lookups, `find_serial_for_avd`). `avd/mod.rs` declares the
      submodules and re-exports.
- [x] **Non-obvious compile gotcha:** `#[tauri::command]` leaves a hidden
      `__cmd__<name>` helper item next to each command function in its
      *defining* module, and `generate_handler!` in `lib.rs` looks that up
      at the same path given for the command itself (`avd::list_avds`
      implies `avd::__cmd__list_avds` must also exist). A named re-export
      (`pub(crate) use lifecycle::{list_avds, ...}`) only re-exports the
      function, not its hidden companion, and fails with `E0433` for every
      single command — fixed by using glob re-exports
      (`pub(crate) use lifecycle::*;` etc.) in `avd/mod.rs` instead. Worth
      remembering for any future module split involving `#[tauri::command]`
      functions: always glob-import, never name individual commands.
- [x] Cross-submodule internals (`sanitize_avd_name`, `category_for_device_id`,
      `find_serial_for_avd`) made `pub(super)` — visible throughout `avd`,
      not the whole crate, since nothing outside `avd` needs them.
- [x] Verified: `cargo check`/`cargo test` (33/33, same tests, now living
      next to the code they test) clean; `cargo clippy -D warnings` and
      `cargo fmt --check` both clean. Live smoke-tested the full lifecycle
      through the real compiled binary via CDP against a throwaway AVD:
      `create_avd` → `launch_avd` → (wait for real boot) → `rotate_avd`
      (landscape then back to portrait) → `save_snapshot` →
      `list_snapshots` → `delete_snapshot` → `stop_avd` → `delete_avd`, all
      succeeded identically to pre-split behavior.
- **Found, unrelated to this split:** `launch_avd`'s `-no-clipboard-sharing`
  flag is no longer accepted by the current emulator binary (37.1.11.0,
  auto-updated by `sdkmanager` since it was last checked) —
  `unknown option: -no-clipboard-sharing / please use -help for a list of
  valid options`. Confirmed via `emulator -help-all` that the flag is gone
  from this build. Only affects launching with clipboard sharing turned
  *off* (the Settings toggle default is on, so most users won't hit it).
  Not fixed as part of this phase (out of scope — a pre-existing behavior
  bug, not a refactor regression) — needs its own pass to find the current
  equivalent flag (if any) or handle its absence gracefully.

Full rationale and file-level detail: see `C:\Users\ryan\.claude\plans\lucky-tumbling-candy.md`
or the plan approved in conversation.

---

# Third batch: first real installer release (Windows, signed via SignPath)

License changed to PolyForm Shield 1.0.0 (from MIT) — `LICENSE`,
`package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, and the
in-app About text (`src/Settings.tsx`) all updated to match. This batch turns
that into a real, distributable installer for the first time — everything
before this was debug bundles built by hand for e2e testing.

Decisions made with the user: **Windows only** for this first release
(matches this machine, and the precedent set by the user's other project,
`development/balla`, which also ships Windows-only); **code signing via
SignPath.io's free OSS program**, the same approach already proven on
`balla`'s CI, wired in but gated so it's a no-op until SignPath approval
lands.

## Done (2026-09-09)
- [x] **Fixed the `-no-clipboard-sharing` bug** (found during the Phase 4
      avd.rs-split smoke test above). Confirmed via `emulator -help-all` and
      the emulator's own `advancedFeatures.ini` (every feature flag it
      knows about) that clipboard sharing has no command-line control at
      all anymore in this build (37.1.11.0) — not renamed, not moved to
      `-feature`, just gone. `launch_avd` (`avd/lifecycle.rs`) no longer
      passes the dead flag; instead it emits an `avd_log` line explaining
      that this emulator version can't disable clipboard sharing, and
      proceeds with it on (the same as leaving the toggle unset) rather
      than failing the whole launch. Verified live via CDP: launching with
      `shareClipboard: false` used to fail with `unknown option:
      -no-clipboard-sharing`; now it launches, boots, and the device shows
      up on `adb devices` as fully running, same as any normal launch.
- [x] Installer/package metadata updated to match the license change:
      `tauri.conf.json`'s `bundle.longDescription`, `package.json`'s and
      `Cargo.toml`'s `description` fields all changed from "open source" to
      "source-available ... PolyForm Shield 1.0.0."
- [x] **Restructured `.github/workflows/release.yml`** from a single
      3-platform `tauri-action` step into Windows-only `build` → `sign` →
      `release` jobs: `build` runs a real `tauri build` and uploads the
      `.msi`/setup `.exe` as workflow artifacts; `sign` downloads them and
      runs the same `Install-Module SignPath` + `Submit-SigningRequest`
      PowerShell pattern already proven on `balla`'s GitLab CI (own
      `ProjectSlug: "beo"`); `release` creates a draft GitHub release via
      `gh release create` (no extra third-party action) with both files
      attached. **Real GitHub Actions constraint hit and fixed:** `secrets`
      can't be referenced in a job-level `if:` (only in `env:`) — so `sign`
      always runs, and the individual signing step gates on `env.*` instead
      (checked via a job-level `env:` block sourced from secrets), meaning
      unconfigured `SIGNPATH_API_TOKEN`/`SIGNPATH_ORG_ID` make the job a
      clean no-op (installers pass through unsigned) rather than skipping
      the job entirely or failing it.
- [x] Version bumped 0.1.0 → 0.2.0 across `package.json`, `Cargo.toml`, and
      `tauri.conf.json` (regenerated `Cargo.lock`/`package-lock.json`) — this
      release includes everything since the initial commit (Vite upgrade,
      frontend tests, daily update check, avd.rs split, the license
      change), more than a patch's worth of change.
- [x] **Real local release build and full NSIS install/launch/uninstall
      cycle, verified by hand, not just configured:**
      - `npm run tauri build` (real release profile, not `--debug`)
        succeeded, producing `Beo_0.2.0_x64-setup.exe` and
        `Beo_0.2.0_x64_en-US.msi`.
      - **Git Bash gotcha hit while testing:** running the NSIS installer
        with `/S` (silent) from the Bash tool opened a full GUI installer
        window instead of installing silently — Git Bash's MSYS layer
        mangles single-letter `/X`-style flags into path arguments before
        they ever reach the Windows exe. Switched to `Start-Process
        -ArgumentList "/S"` in PowerShell (native argument passing, no
        conversion), which worked correctly. Worth remembering for any
        future Windows-native-exe flag testing on this machine.
      - Silently installed via PowerShell (`/S`), confirmed it landed at
        `%LOCALAPPDATA%\Beo\beo.exe` (per-user, no admin needed) with a
        real Start Menu shortcut created. Launched `beo.exe` directly,
        confirmed a real "Beo" window opened and stayed responsive for
        10+ seconds (not an instant-crash false success, the exact failure
        mode this project has hit before with `launch_avd` itself).
        Uninstalled via the generated `uninstall.exe /S`, confirmed both
        the install directory and the Start Menu shortcut were fully
        removed afterward.
- **Found, not fixed — documented instead:** the MSI (`msiexec /qn`) fails
  with **Error 1925 — insufficient privileges** when installed
  non-elevated. The WiX-based MSI bundle defaults to a per-machine
  (all-users) install scope, which requires admin elevation; the NSIS
  installer defaults to per-user and needs none, which is why it's the
  installer recommended first in the release notes. This is standard
  MSI/WiX behavior, not a Beo bug — confirmed via a verbose `/L*v` install
  log showing the exact WiX error, and confirmed the failed attempt left no
  partial install behind (`C:\Program Files\Beo` never created, clean
  rollback). Not fixed in this pass since it needs a decision (elevate to
  test properly, or set `installPerMachine: false` in the WiX config to
  make the MSI per-user too) — deferred to a future pass if the MSI turns
  out to matter enough to a user to justify either.

## Still to do

**Release gate, stated explicitly by the user (2026-09-10): tag and
release only once the build has been tested and confirmed working on
both Windows and Linux.** Not before. macOS testing is separately
deferred ("down the road") and isn't part of this gate — don't wait on it
before the Windows+Linux release, and don't push a tag or suggest one is
due until the user says both platforms are confirmed.

- [ ] Push the `v0.2.0` tag to trigger the real `release.yml` run on
      GitHub's own Windows runner — this is a git push, so per this
      project's standing rule the user does this themselves, not Claude.
      Exact commands: `git tag v0.2.0` then `git push origin v0.2.0` (after
      committing everything above). Once it runs, confirm the draft release
      appears with both files attached, and download+run the actual GitHub
      Actions-built artifact (not just the local one) to close the loop —
      first real confirmation the workflow itself works end to end, not
      just the local build.
- [ ] Apply for SignPath's free OSS signing program
      (https://about.signpath.io/product/open-source) for this repo
      specifically (own project, separate from `balla`'s) — external,
      user-driven. Once approved, add `SIGNPATH_API_TOKEN`/
      `SIGNPATH_ORG_ID` as repo secrets; the workflow already knows what
      to do with them the moment they exist, no further code changes
      needed.
- [x] **Linux CI leg wired up (2026-09-10)** — new `build-linux` job in
      `release.yml` (`ubuntu-22.04`, own system deps step mirroring
      `ci.yml`'s, bash-based artifact collection since PowerShell doesn't
      apply there), producing `Beo-linux.AppImage`/`Beo-linux.deb` and
      feeding them into the `release` job alongside the Windows
      installers. Skips the `sign` job entirely — SignPath signs Windows
      PE binaries specifically, appimage/deb don't need or support that.
      Release notes updated to cover the Linux install steps too. YAML
      syntax validated locally (`npx js-yaml`) but not run for real —
      GitHub Actions itself needs an actual runner, so *that specific
      workflow file* is only confirmed once a tag triggers it.
      **That's separate from testing the Linux build itself, which
      doesn't need a tag or CI at all** — same as how the Windows NSIS
      cycle was verified by just running `npm run tauri build` locally
      and installing the result by hand. Next real step: build locally on
      the Linux VM (`npm run tauri build`, produces the `.AppImage`/`.deb`
      under `src-tauri/target/release/bundle/`) and verify
      install/launch/uninstall there — that can happen any time, well
      before deciding to push a tag.
- [ ] macOS installer — deferred, "down the road" per the user
      (2026-09-10, sequencing decided: Linux first, Mac later). When
      picking it up: add a `macos-latest` leg the same way (the `dmg`
      target is already declared in `tauri.conf.json`), verify
      install/launch/uninstall by hand on the Mac, and address Gatekeeper
      quarantine handling on the downloaded binary.

---

# Fourth batch: small UX polish, transparency, and IDE detection

## Done (2026-09-10)
- [x] **`detect_connected_ides` expanded** from just Android Studio/VS Code
      to also check for IntelliJ IDEA and Zed (both confirmed live against
      real running processes on this machine — `idea64.exe`, `Zed.exe`),
      plus Cursor, Windsurf, WebStorm, PyCharm, Neovim, Notepad++, and
      Google Antigravity by their documented/standard process names (none
      of those seven installed here, so unconfirmed live — flagged as such
      in code comments).
- [x] **New "Data & storage" section in Settings** — new `data_paths`
      command (`util.rs`) surfaces both real on-disk locations Beo writes
      to: its own app data dir (SDK/images/JDK) and the separate standard
      `~/.android/avd` location `avdmanager` actually uses for devices —
      called out explicitly as two different places, for transparency.
      Both have Copy buttons.
- [x] **Copy buttons now give feedback.** Every "Copy" button in the app
      (SDK path, the two new data paths, the debug log) previously wrote
      to the clipboard with zero visible confirmation — clicking looked
      like it did nothing. All four now flip to "Copied!" for 1.5s.
- [x] **`install_apk` now targets a specific device serial** (via
      `find_serial_for_avd`, the same helper `rotate_avd`/the snapshot
      commands already use) instead of calling bare `adb install` and
      trusting whatever device adb defaults to — the one remaining
      command that didn't disambiguate between multiple running emulators.
      Verified live: installing onto a named-but-not-running device now
      correctly fails with "Couldn't find a running emulator for ..."
      instead of silently trying (and possibly succeeding against the
      wrong device, or erroring confusingly) via adb's own default.
- [x] **Success messages in the dashboard's log block now auto-dismiss**
      after 4 seconds (`"Created X"`, `"Installed on Y"`, etc.) instead of
      sitting there until the next action overwrites them. An error paired
      with a Retry button (`lastFailed` set) still persists — only the
      no-action-needed case fades on its own.
- All four verified live via CDP against a real standalone release build
  (`npm run tauri build`, not `--debug`, to avoid the running dev
  session's locked `target/debug/beo.exe`) — real paths rendering
  correctly in Data & storage, both the SDK-path and debug-log Copy
  buttons flipping to "Copied!" and reverting, a validation message
  appearing then disappearing after ~4s, and the `install_apk` error path
  confirmed against a real (not-running) device name.
- **Process note:** killed the user's real active `tauri dev` session by
  accident mid-verification — `taskkill /IM beo.exe` matched by image name
  and killed both a test instance and the user's own session, since two
  processes shared that name. User confirmed no harm (it was disposable),
  but the lesson stands: target a specific PID when there's any chance
  another same-named process is running, never kill by image name on this
  machine without checking first. Also: the standalone test instance
  wrote `beo-devmode: on` to `localStorage`, which is a shared WebView2
  profile — the user's own next real launch may show the Debug panel on
  by default as a result (harmless, one click to turn back off, just
  worth knowing why it's on).
- 33/33 Rust tests, clippy, fmt, `tsc --noEmit`, 35/35 frontend tests,
  and `npm run build` all clean throughout.

---

# Fifth batch: sideload hand-holding, an AVD/sideload e2e test, Settings component tests

## Done (2026-09-10)
- [x] **Sideload UX hand-holding.** The success message after installing an
      APK just said "Installed on X." with no next step — now says
      "Installed on X. Open the app drawer on the device to launch it."
      Failure messages were raw `adb install` output (`INSTALL_FAILED_*`
      codes) — added `explainInstallApkError()` (`App.tsx`, exported,
      unit-tested) translating the common real-world cases (ABI mismatch,
      SDK-version mismatch, signature/version conflict, invalid/corrupt
      APK, insufficient storage) into plain English with a concrete next
      step, falling back to the raw message for anything not covered —
      same "explain every failure mode, don't just show raw tool output"
      convention already used for the update-check errors. 6 new tests.
- [x] **New local-only e2e test**: `scripts/e2e-avd-sideload.mjs`
      (`npm run test:e2e:avd`), covering the AVD lifecycle + sideloading
      end to end through the real compiled app over CDP — create, launch,
      wait for full boot, install a real APK (pulled live off the device
      itself, so no bundled test fixture needed), confirm success, then
      confirm installing onto a *not-running* device name fails cleanly
      instead of silently falling through to adb's default target.
      Deliberately **not** wired into CI (same call as the existing
      `test:e2e:jdk`) — needs a real emulator boot (1-2 min even with
      hardware acceleration) and doesn't isolate `BEO_DATA_DIR` like the
      JDK e2e test does, running against the real installed SDK instead so
      repeat local runs don't re-download a ~1GB system image every time.
- [x] **Real bug caught by writing this e2e test, before it ever shipped
      wrong**: the first run failed with adb's own "device is still
      booting" error during `install_apk`, even though `rotate_avd` had
      *just* succeeded moments earlier — proving a successful rotate
      (window manager responding) is not sufficient proof the device is
      ready for package installs; `PackageManagerService` can still be
      initializing after window manager already responds. Fixed by having
      the e2e script poll `sys.boot_completed` for 3 consecutive reads
      (matching `launch_avd`'s own internal boot-completion threshold)
      before attempting the install. Also hardened the poll itself after a
      second run hit a transient `adb shell` exit-255 hiccup (normal
      adb behavior right after boot) that crashed the whole script — now
      caught and treated as "not ready yet" rather than fatal. Third run
      passed clean end to end.
- [x] **New `Settings.test.tsx` component test suite** (Settings.tsx had
      zero component-level coverage before this — only pure-function tests
      for `isNewerVersion`, now merged into this file per the same
      one-file-per-component convention `DeviceCard.test.tsx` already
      uses). Covers: Data & storage paths rendering (and their unloaded
      "…" placeholder state), Copy-button "Copied!" feedback with fake
      timers, the IDE-integration result message actually rendering inside
      the "Use with another IDE" card (a regression guard for the exact
      bug just fixed in the fourth batch), the About section's four
      update-check states, and Debug-section visibility gating on
      `devMode`. First component test file in this project to mock
      `@tauri-apps/api/core`'s `invoke` — sets the pattern for testing any
      future component that calls Tauri commands directly rather than only
      taking props (`DeviceCard`/`CreateDeviceForm` are pure-props, so
      never needed this).
- 51/51 frontend tests (10 net new), `tsc --noEmit`, `npm run build` all
  clean. The e2e script itself was run for real three times while fixing
  it (not just written and trusted) — see the bug it caught, above.

---

# Sixth batch: pin platform-tools and emulator versions

Root-causes the class of bug behind the `-no-clipboard-sharing` removal
(fourth batch): `install_sdk` installed platform-tools and emulator via
plain `sdkmanager install platform-tools emulator`, which has no way to
request a specific historical version of either package (unlike
build-tools, they carry no version suffix) — every install silently got
whatever Google considered "latest" that day, and calling `install_sdk`
again later (its own repair path) could silently upgrade an
already-working install to a newer, untested build with zero warning.

## Done (2026-09-10)
- [x] Pinned both packages the same way `cmdline-tools` already is: an
      explicit URL + self-computed SHA-256 per platform, downloaded
      directly and extracted without going through `sdkmanager`'s package
      resolution at all. Sourced from Google's own repository manifest
      (`https://dl.google.com/android/repository/repository2-3.xml`, the
      same file `sdkmanager` itself reads) — **platform-tools r37.0.1**
      and **emulator 37.1.11 (build 15917651, the "stable" channel build
      as of this pinning)** across Windows/macOS(x64+arm64)/Linux(x64), 7
      URL/hash pairs total. Every one of the 7 files was actually
      downloaded (not just its checksum copied from the manifest) and its
      SHA-256 computed independently — the manifest's own SHA-1 for each
      was cross-checked and matched all 7, confirming genuine, unaltered
      downloads before trusting them as the pinned values.
- [x] Noted honestly in a code comment: the pinned "stable" emulator build
      is the *same* build already installed on this machine that's
      missing `-no-clipboard-sharing` — pinning doesn't undo that
      upstream removal, it stops any *further* silent drift. Re-pinning
      to a newer build later is a deliberate version bump in `sdk.rs`,
      not something that happens automatically.
- [x] Extracted a shared `download_and_verify()` helper (used by all three
      pinned downloads now — cmdline-tools, platform-tools, emulator) to
      avoid tripling the download/progress/cancellation loop; bundled its
      per-download config into a `PinnedDownload` struct after clippy
      flagged the unbundled version for having too many arguments.
- [x] **Same "skip if already installed" guard now applies to
      platform-tools and emulator, not just cmdline-tools** — this is the
      actual fix for the repair-path drift: `install_sdk` checks
      `adb_bin()`/`emulator_bin()` existence before each download, so
      calling it again (its own documented repair/complete-an-install use
      case) no longer touches either package if already present.
- [x] **Verified live, fully, not just compiled:** built a debug bundle,
      ran a real fresh `install_sdk()` against an isolated `BEO_DATA_DIR`
      — cmdline-tools, JDK, platform-tools, and emulator all downloaded
      via the new pinned path in 108s, `verify_sha256` passed for all of
      them, extracted to the exact directory layout `adb_bin()`/
      `emulator_bin()` expect, zero leftover zip files. Called
      `install_sdk()` again on the same isolated dir immediately after:
      returned in 0.7s (vs. 108s), confirming the skip-if-installed guard
      actually prevents the redundant re-download/upgrade this whole
      batch exists to stop. Then did a full functional pass — downloaded
      a real system image, created and launched a real device with the
      newly-pinned emulator binary, confirmed it boots and responds
      (`rotate_avd` round-trip succeeded) — proving the pinned binary
      isn't just present on disk but actually works. Cleaned up the
      isolated dir, throwaway AVD, and downloaded verification zips
      afterward.
- 33/33 Rust tests, clippy, fmt all clean throughout. No new tests added —
  this is download/wiring logic already covered by `verify_sha256`'s
  existing accept/reject unit tests; the live e2e pass above is what
  actually proves this behavior, the same way the original cmdline-tools
  pinning was verified.

---

# Seventh batch: App.tsx test coverage

`App.tsx` (the biggest, most stateful file — all refresh/reconcile/
booted-state orchestration lives there) had zero automated coverage
before this, only manual CDP verification each time it was touched. A
full render-test mocking its entire ~28-command `invoke` surface was
already deliberately deferred as poor ROI (see the "second pass" test
coverage work) — this batch takes a narrower approach instead.

## Done (2026-09-10)
- [x] **Extracted `initialOrientationForLaunch(avds, name)`** — the
      tablet-vs-phone default-rotation logic (`doLaunch` previously had
      this inline) is now its own pure, exported, unit-tested function.
      This is the exact real bug from an earlier pass (assuming portrait
      regardless of category made a tablet's first Rotate click a silent
      no-op) — now has a regression test guarding it specifically, not
      just a comment.
- [x] **New `App.test.tsx`**, merging the previous pure-only `App.test.ts`
      (per the same one-file-per-component convention as `DeviceCard`/
      `Settings`) plus two new component-level tests that mock only the
      minimal command set needed to reach a stable one-device dashboard
      render (~10 commands: `sdk_status`, `check_network`, `check_java`,
      `list_avds`, `list_available_images`, `list_device_profiles`,
      `check_hardware_accel`, `preferred_abi`, `list_running_avds`,
      `check_disk_space`, `data_paths`) — not the full ~28-command
      surface, scoped specifically to the two behaviors being tested:
  - `doStop`'s failure-path reconciliation: `stop_avd` rejects (device
    already crashed), the test confirms the very next `list_running_avds`
    re-sync call actually flips the device back to "Stopped" instead of
    leaving it stuck showing "Running" with a Stop button that would fail
    the same way forever.
  - The log auto-dismiss timer: a no-action-needed message (the
    create-form's client-side name validation, chosen because it needs no
    backend call) clears itself after ~4s via fake timers.
- [x] **Verified both new component tests actually catch a regression,
      not just pass** — temporarily disabled `doStop`'s reconciliation
      re-sync (commented out the `setRunning`/`setBooted` calls in the
      catch block), confirmed the exact new test failed with the device
      stuck on "Running"/"Stop" and the raw error in the log, then
      reverted and confirmed 57/57 pass again.
- 57/57 frontend tests (6 net new: 3 for `initialOrientationForLaunch`, 3
  component-level), `tsc --noEmit`, `npm run build` all clean. No Rust
  changes this pass.

---

# Eighth batch: switch back to real GPU rendering, fixing audio glitches

User reported glitchy audio playing YouTube in a tablet's browser.
Investigated live rather than guessing:
- Host: hypervisor active, host CPU 4-9%, emulator process ~20% of one
  core — not overloaded.
- Confirmed the bundled `qemu-system-x86_64.exe` only contains the
  **DirectSound** audio backend string (no WASAPI) — the older, more
  glitch-prone Windows audio API, already the only option this build
  offers regardless of any `-audio` flag Beo could pass.
- The real lever available: `-gpu swiftshader_indirect` (software
  rendering, CPU-based) had been the pinned default since early in this
  project specifically because `-gpu auto` once hung forever at the boot
  logo on this exact host. Software rendering competes with audio for the
  same CPU cycles during video playback — a known contributor to exactly
  this kind of glitching.

## Done (2026-09-10)
- [x] **Re-tested `-gpu host` by hand, not just reasoned about** — a
      throwaway device launched with `-gpu host` booted fast (~10-30s) and
      stayed stable for a 3-minute observation window, using the real
      discrete GPU (confirmed in the emulator's own log: "Graphics
      backend: gfxstream" against the actual Radeon RX 7600 XT) via
      gfxstream, not the software path. Audio modules (WINMM, dsound,
      MMDevApi, AUDIOSES) loaded normally. `-gpu auto`'s old hang was never
      specifically re-tested as plain `-gpu host` before now — `auto`'s
      heuristic may have picked something else entirely, or the host/driver
      state has simply changed since the original finding.
- [x] **Switched `launch_avd`'s pinned `-gpu` value from
      `swiftshader_indirect` to `host`** (`avd/lifecycle.rs`). Doc comment
      rewritten to carry the full history honestly: the original hang,
      why swiftshader was chosen, why it's being moved off now, and that
      `swiftshader_indirect` is the known-safe fallback if a hang like the
      original one ever recurs on some future host/driver combination —
      this is a real, live regression risk being knowingly accepted to fix
      a confirmed real UX problem (glitchy audio), not a risk-free change.
- [x] 33/33 Rust tests, clippy, fmt all clean.
- **Not fully re-verified through Beo's own compiled UI this pass** — the
  user's real `tauri dev` session was active throughout, and WebView2
  reuses the same browser process for a given app identifier, so a second
  Tauri instance launched alongside it doesn't get its own debuggable
  browser process (no way to attach CDP without touching their session).
  Verified instead via direct `emulator.exe` invocation with the exact
  same flags `launch_avd` now produces (identical `-avd`/`-gpu host`
  arguments) — a real, equivalent test of the actual emulator behavior,
  just not driven through Beo's own IPC layer this specific time. **Ask
  the user to relaunch their real tablet and confirm the audio glitching
  is actually gone** — that's the real acceptance test this fix exists
  for, and hasn't been confirmed yet.

---

# Ninth batch: Beo Diagnostics — a sideloadable test app

While debugging the audio glitching above, verification kept hitting a
wall: no controlled, repeatable way to generate a test signal inside the
guest, or compare rendering/network/storage behavior across
`-gpu`/`-feature` experiments except by subjective impression. User asked
for a small on-demand test utility — a real installable APK, not adb
one-liners — covering a broader health check (audio, GPU, network,
storage), agreed via plan mode before writing any code.

## Done (2026-09-10)
- [x] **New `diagnostics-app/` project** — a real Android app (Kotlin,
      plain Views, no Compose/Hilt/Room/DI — deliberately far simpler than
      the user's other Android project since this is a lightweight dev
      tool). One Activity, four independent checks, each with a Run/Stop
      button and live status:
  - **Audio** — synthesizes a 440Hz sine tone directly via `AudioTrack`
    (no bundled asset needed) and loops it with an elapsed-time counter —
    a controlled, repeatable signal instead of YouTube's variable content.
  - **GPU/rendering** — an animated `View.onDraw` bouncing box with a live
    FPS readout via `Choreographer.postFrameCallback`.
  - **Network** — HEAD request to Android's own official
    connectivity-check endpoint, reporting latency.
  - **Storage** — write → read-back → delete a temp file, reports free
    space via `StatFs`.
- [x] **This is explicitly NOT part of Beo's own build.** Beo's bundled
      SDK deliberately has no build-tools (kept minimal) — this app is
      built once, separately, using the machine's *other*, already-present
      Android Studio SDK (`C:\Users\ryan\AppData\Local\Android\Sdk`,
      build-tools 36.0.0), via `./gradlew assembleDebug`. Gradle wrapper
      files reused directly from the user's other Android project
      (`development/balla/apps/android`) — same bootstrap code, no need to
      reinvent it, and it let the build use an already-cached Gradle
      8.9 distribution with zero fresh downloads. **Built successfully on
      the first real attempt** — `BUILD SUCCESSFUL in 31s`.
- [x] Compiled APK copied to `resources/beo-diagnostics.apk` (7.3MB) and
      committed as a binary — see `diagnostics-app/README.md` for how to
      rebuild it later (only when this app's own code changes, not on
      every Beo build). Added `diagnostics-app/.gitignore` so Gradle's own
      `build/`/`.gradle/`/`.kotlin/` caches never get committed alongside
      it.
- [x] **Beo integration**: `tauri.conf.json`'s `bundle.resources` now
      ships the APK inside every installer. New Rust command
      `install_diagnostics_apk(name)` in `avd/lifecycle.rs` (right next to
      `install_apk`) resolves the bundled resource's real path via
      Tauri's `PathResolver`/`BaseDirectory::Resource`, reuses the exact
      same `find_serial_for_avd` + `adb install -r` logic `install_apk`
      already has, then also runs `adb shell am start` so it's
      install-and-open in one click — no file picker needed, unlike
      sideloading an arbitrary APK, since this one's bundled with Beo
      itself. New "Diagnostics" button on `DeviceCard.tsx`, gated on
      `ready` the same way Rotate/Install APK/Snapshots already are.
- [x] **Verified live, thoroughly, on a real throwaway device**: installed
      and launched for real (`adb install` → `adb shell am start`,
      confirmed via `dumpsys activity activities` that it was genuinely
      the foreground activity, no crash in logcat). Then actually drove
      the UI via `uiautomator dump` + coordinate-based `input tap` on all
      four buttons and read back the real results: **440Hz tone playing
      with a live elapsed-time counter, 60fps GPU animation (confirming
      `-gpu host` is delivering full smooth rendering), a real network
      check succeeding in 1336ms, and a real free-space report (3981MB)
      from a genuine write/read/delete cycle.** All four checks work
      exactly as designed, first try.
- 33/33 Rust tests, clippy, fmt, `tsc --noEmit`, 57/57 frontend tests,
  `npm run build` all clean.
- **Not yet verified through Beo's own compiled UI end-to-end** — same
  WebView2 shared-browser-profile obstacle as the audio-fix batch above:
  the user's real `tauri dev` session was active throughout, and a second
  instance can't get its own debuggable browser process regardless of
  which binary launches it (dev vs. release — WebView2's profile is keyed
  by the app identifier, not the exe path). The Rust command's ADB logic
  is the same proven pattern `install_apk` already uses; the one genuinely
  new, unverified piece is Tauri's resource-path resolution actually
  finding the bundled APK at runtime. **Ask the user to click the new
  "Diagnostics" button on a real device in their own already-running
  session** (it already picked up this code via its file-watcher) — that
  closes the loop on the one untested part, and is the real end-to-end
  acceptance test regardless.

## Bug found and fixed on first real use: content hidden behind system bars
User installed the app for real on their own tablet (`tes3`) and reported
"app is not scrollable." Investigated live rather than guessing — dumped
the real UI hierarchy via `uiautomator`, which showed `scrollable="false"`
on the root `ScrollView` even though the bottom "Storage" section's button
was rendered at y=1413-1509 on a 1600px-tall screen with the navigation
bar occupying y=1536-1600. Root cause: `compileSdk`/`targetSdk = 36`
enables Android 15+'s **enforced edge-to-edge** — the system draws the
status bar (top 48px) and navigation bar (bottom 64px) *over* app content
by default now, not beside it. The `ScrollView` correctly measured "all
content fits in 1600px" and reported nothing to scroll, while in reality
the bottom of the Storage section was rendered directly behind the opaque
navigation bar — invisible and unreachable. Looked exactly like "the app
doesn't scroll" because there was genuinely nothing left to scroll to; the
content was just hidden behind system chrome.
- [x] Fixed via `ViewCompat.setOnApplyWindowInsetsListener` padding the
      scroll root by the real system-bar insets — not the deprecated
      `setDecorFitsSystemWindows(true)` opt-out, since Google's own
      direction is that opting out of edge-to-edge won't keep being
      honored in future Android versions.
- [x] **Verified live on the user's actual real tablet**, not a
      throwaway: rebuilt, reinstalled over the running app, re-dumped the
      UI — `ScrollView` now correctly reports `scrollable="true"`.
      Performed a real `adb shell input swipe` gesture and confirmed the
      Storage button moved from a clipped 11px sliver at the very bottom
      edge to its full, normal height, fully clear of the navigation bar.
- Updated `resources/beo-diagnostics.apk` with the fix; no Rust/frontend
  changes needed for this one (contained entirely inside the Android app).

## Tenth batch — a real, repeatable audio-glitch test case (2026-09-14)
User asked to actually spin up a device and test the `-gpu host` +
`-feature -VirtioSndCard` audio changes against real YouTube playback,
since neither Claude nor an automated script can literally listen for the
reported popping — the ask was for an objective, repeatable signal
alongside listening, not a replacement for it.

New script: `scripts/test-audio-youtube.mjs` (`npm run test:audio:youtube`,
local-only, not wired into CI — same reasoning as the other e2e scripts).
Drives the real compiled Beo binary over CDP exactly like
`e2e-avd-sideload.mjs`, creates a throwaway device via Beo's real
`create_avd`/`launch_avd` (so it exercises whatever `-gpu`/`-feature` flags
are currently pinned in `lifecycle.rs`), opens a live YouTube stream in
Chrome, clicks through onboarding, confirms real audio playback via
`dumpsys audio`, then captures 45s of logcat and flags any
underrun/xrun/glitch lines.

Getting this to actually run cleanly (not just compile) surfaced a chain
of real bugs, each confirmed live before moving to the next:
- [x] The debug binary was silently a *dev-mode* build pointing at
      `http://localhost:1420` (Vite dev server) instead of the bundled
      frontend — a plain `cargo build`/`cargo check` run elsewhere during
      this session had overwritten `target/debug/beo.exe`, which the
      script's `--skip-build` flag then reused. Only `npm run tauri build
      -- --debug` produces the self-contained binary this script needs.
- [x] `adb devices` reported nothing even with the emulator genuinely
      running — Beo runs adb/emulator against a *private* adb server
      (`ANDROID_ADB_SERVER_PORT=5039`, see `android_tool()` in `util.rs`)
      so it never collides with a system adb install. Every adb call this
      script makes now sets the same env var.
- [x] A `list_running_avds`-based boot check can report "booted" slightly
      before adb has actually attached the emulator's serial (same race
      already documented in `e2e-avd-sideload.mjs`) — added the same
      `sys.boot_completed`-polling confirmation step.
- [x] The cookie-consent dialog's scroll gesture started *below* the
      dialog card, in the dimmed background page, so "Accept all" was
      never scrolled into view — fixed the swipe to act within the card's
      actual bounds.
- [x] A hardcoded specific video ID is a single point of failure — the one
      this was first pinned to went from "live" to permanently unavailable
      between sessions. Switched to a channel's `/live` URL
      (`youtube.com/@LofiGirl/live`), which YouTube always resolves to
      whatever that channel currently has live.
- [x] Chrome's autoplay policy mutes video that starts playing without a
      user gesture — the stream was genuinely playing, just silently, with
      a "TAP TO UNMUTE" control. That control isn't reliably
      text-labeled (one run showed a text banner, another only a bare
      icon with zero accessible text), so switched to a fixed-coordinate
      tap that works for both variants.
- [x] A single fixed sleep after that unmute tap caused a false negative —
      the tap had worked, but `dumpsys audio` hadn't registered the new
      focus yet, so the script gave up on an already-fixed stream and
      hopped to a still-muted related video instead. Now polls for a few
      seconds before concluding the tap didn't do anything.
- [x] `adb logcat -d` after 45s of real playback exceeds Node's default
      1MB `execFileSync` buffer (`ENOBUFS`) — raised `maxBuffer` to 64MB.
- [x] Related-video fallback (used when the primary stream isn't live) now
      matches on the `"<title> by <channel> ... N views|watching"` pattern
      unique to real related-video rows, with a real-height check —
      earlier attempts matched the *primary* video's own standalone view
      count, or degenerate zero-height off-screen placeholder rows
      `uiautomator` returns for content not yet scrolled into view.
- [x] **Actually run successfully end-to-end at least once** (2026-09-14):
      real device, real Chrome/YouTube playback, confirmed real
      `AudioManager` focus, 45s of real logcat captured, no explicit
      underrun/xrun/glitch lines found in that run. As the script itself
      says on every run: this confirms the log shows no explicit
      underruns — it cannot confirm the audio actually sounds clean. The
      user still needs to listen for themselves to close that loop.

## Eleventh batch — a second, non-browser audio test path, sharing infrastructure with the first (2026-09-14)
User asked to extend the audio test into an actual suite: play a real audio
file (not synthesized, not through the browser) so the YouTube/Chrome path
and a plain native path can both be tested — a clean result in Chrome alone
doesn't prove the underlying emulator audio pipeline is fine (Chrome has its
own decode/playback path), and a glitchy result in Chrome alone doesn't
prove the emulator is at fault either. Testing both tells you which side of
that line a problem is on.

- [x] **Shared scaffolding extracted** into `scripts/lib/audio-test-common.mjs`
      — build/launch/CDP connect, AVD create+boot (with all three
      previously-fixed races folded in), adb helpers, `uiautomator`
      dump/tap, logcat glitch analysis, cleanup. `test-audio-youtube.mjs`
      was refactored onto this shared module and re-verified still passes
      end-to-end after the refactor — same real device, real Chrome
      playback, real focus confirmation, clean logcat.
- [x] **`diagnostics-app` gained a "Play Music File" section** — native
      `MediaPlayer` playing a bundled synthetic WAV
      (`res/raw/sample_song.wav`, generated by new
      `scripts/generate-sample-song.mjs`: a short melody with two-partial
      "notes" and per-note fade envelopes, closer to real music than the
      app's existing single-frequency test tone, deliberately synthetic
      rather than a real downloaded track to avoid any licensing question).
      Loops via `MediaPlayer.isLooping` so one tap sustains playback for a
      full capture window.
- [x] **Real bug found and fixed while wiring up detection**: the new
      music-file button's real, active playback initially showed *zero*
      audio focus signal — not a script bug, but a real gap in the app:
      `MediaPlayer.create()` alone never requests `AudioFocus` and, without
      an explicit `AudioAttributes`, reports `usage=USAGE_UNKNOWN` in
      `dumpsys audio` — confirmed live via the full dump, which showed a
      genuine `state:started` `AudioPlaybackConfiguration` entry (real
      playback, undeniably) while the legacy "Audio Focus stack" section
      was completely empty. Fixed two ways: (1) `diagnostics-app` now sets
      proper `AudioAttributes` (`USAGE_MEDIA`/`CONTENT_TYPE_MUSIC`) and
      formally requests `AudioFocus` via `AudioFocusRequest`, making it a
      well-behaved media app like the thing it's meant to approximate; (2)
      `hasAudioFocus()` (shared helper, used by both test scripts) now
      checks for a `state:started` `AudioPlaybackConfiguration` instead of
      the old `"USAGE_MEDIA"` + `"gain: GAIN"` substring pair, since that
      pair was confirmed live to miss genuine playback that never populates
      the legacy focus-stack section — this is a more robust signal
      regardless of an app's own focus-request hygiene. Re-verified the
      YouTube script still passes with the new check.
- [x] New `scripts/test-audio-file.mjs` (`npm run test:audio:file`):
      creates a throwaway device, installs+launches Beo Diagnostics via the
      real `install_diagnostics_apk` command, taps "Play Music File" via
      `uiautomator` (native views are reliably tappable by text — unlike
      the WebView/dialog cases in the YouTube script that needed coordinate
      fallbacks), confirms real playback, captures 45s of logcat.
      **Actually run successfully end-to-end**: real native `MediaPlayer`
      playback confirmed, no underrun/xrun/glitch lines found. Noted one
      benign, expected pattern in the raw log
      (`AudioTrack: pauseAndWait: timeout expired... still pausing`,
      recurring at each ~6s loop boundary) — an artifact of
      `MediaPlayer.isLooping`'s stop/restart cycle, not a glitch signature;
      visible in the script's own printed log dump for anyone who wants to
      look closer.
- Both scripts still carry the same standing caveat on every run: this
  confirms what the log does or doesn't show — it cannot confirm the audio
  actually sounds clean. The user still needs to listen for themselves.

## Deferred — a video-playback test suite (2026-09-14)
User asked whether other "internal function" test suites are needed (video,
etc.) alongside the new audio suite. Decision: **not proactively** — the
audio suite exists because of a real, reported bug (the popping), not
because audio was tested "for completeness." Building a video-glitch suite
speculatively risks the same multi-hour flaky-UI debugging this audio suite
needed, for a problem nobody has actually reported. The diagnostics app's
existing GPU FPS counter already gives a reasonable proxy for rendering
health (frame drops/stutter) that overlaps with a lot of what a dedicated
video test would check.
- **If real video issues show up later** (choppy playback, A/V sync drift,
  tearing), build a video test the same way this one was built: reactively,
  against a specific reported symptom, reusing
  `scripts/lib/audio-test-common.mjs`'s scaffolding (build/launch/CDP, AVD
  create+boot, adb helpers) with new checks for that symptom specifically
  (e.g. dropped/decoded frame counts via `dumpsys media.player` or
  `SurfaceFlinger`, not a generic "does video play" check).

## Real bug found and fixed live: stale quick-boot snapshots masked the eighth batch's audio fix (2026-09-14)
User reported total silence (not glitching — nothing at all) playing
YouTube on a "Medium Phone" device, over a Bluetooth speaker. Investigated
live rather than guessing, ruling out layers in order:
- **Guest-side audio was genuinely fine**: `dumpsys audio` on the real
  running device showed a real `AudioTrack` with `state:started`,
  `usage=USAGE_MEDIA`, `mutedState:none`, routed to `speaker`, stream
  volume 7/15 — Android itself believed it was playing normally.
- **Host mixer was fine**: Windows' per-app volume mixer showed the
  `qemu-system-x86_64.exe` entry unmuted at a normal level.
- **Not Bluetooth-specific**: asked the user to switch the default output
  device to wired/onboard speakers — still silent, ruling out the
  DirectSound/Bluetooth-A2DP routing theory this would otherwise have
  pointed at.
- **Not Chrome's autoplay-mute policy**: confirmed the in-page video
  itself showed no mute indicator, ruling out the "TAP TO UNMUTE" issue
  this session's audio-test-suite work had separately found and worked
  around.
- **Root cause, confirmed by file timestamp**: the device's automatic
  quick-boot snapshot (`<avd>.avd/snapshots/default_boot/`, distinct from
  the named snapshots in Beo's own snapshot panel) had a `ram.img` dated
  **2026-09-09** — a full day *before* the eighth batch's `-gpu host` /
  `-feature -VirtioSndCard` change (2026-09-10). Every launch since then
  had been *resuming* a boot image captured under the old
  `VirtioSndCard`-enabled hardware config, while QEMU was actually
  presenting the new legacy Intel HDA device underneath it — a real
  mismatch between what Android's audio framework thought it was talking
  to and what hardware was actually there, with no error surfaced anywhere
  (the framework layer looked completely normal because it never
  re-probed real hardware after resuming from the stale snapshot).
- [x] Cleared the stale `snapshots/default_boot/` directory for both
      affected AVDs (`Medium_Phone.avd`, and `tes3.avd` — its snapshot was
      dated the same day as the fix, close enough to not trust the
      ordering) so their next launch does a full cold boot. Not yet
      confirmed by the user whether this actually restored audio — that's
      the pending real acceptance test.
- [ ] **Deferred fix, approved by user (2026-09-14) but not yet built**:
      Beo should automatically invalidate a device's quick-boot snapshot
      whenever the launch flags (`-gpu`/`-feature`) it was saved under
      change, so this can't silently recur for future flag changes.
      Approach discussed: stamp `config.ini` (or a Beo-owned sidecar file)
      with a hash of the current `-gpu`/`-feature` args on launch; before
      trusting an existing quick-boot snapshot, compare against what's
      stored, and delete `snapshots/default_boot/` first if they differ so
      the emulator cold-boots instead of silently resuming stale hardware
      state. Not urgent — this was a one-off manual cleanup — but real
      enough to build properly once other higher-priority work is clear.

## Correction: the stale-snapshot theory wasn't the (whole) story — reverted the untested Intel HDA switch (2026-09-14)
User cleared the snapshot per the above, cold-booted, and **still got
total silence**. Rightly called out that this hadn't actually been
verified — I'd been asking the user to test rather than digging further
myself. Went back in and checked `dumpsys media.audio_flinger` on the live
device directly:
- **Real, non-silent signal power history** at the AudioFlinger mixer/HAL
  boundary — power fluctuating between roughly -15 dB and -30 dB during
  actual YouTube playback (vs. -63 dB silence beforehand), frames written
  climbing continuously, zero underruns reported by the mixer. This proves
  Android's own audio stack (mixing, HAL, write path) was doing everything
  correctly, handing real audio data to the virtual sound device — the
  break was downstream, inside QEMU's audio backend itself.
- That points at the eighth batch's `-feature -VirtioSndCard` experiment
  (forcing legacy Intel HDA over the default virtio-snd device), which its
  own doc comment already admitted was "not yet confirmed to actually fix
  the popping, only confirmed to still boot and load audio modules
  normally" — it was **never confirmed audible by an actual human ear, on
  any device**. The original virtio-snd default *was* reported audible
  (glitchy, but audible) before that experiment.
- [x] **Reverted** `-feature -VirtioSndCard` in `launch_avd` — back to the
      default virtio-snd device. Kept `-gpu host`, since that part *was*
      independently confirmed (real GPU use, fast reliable boot, no hang).
      `cargo check` clean. Not yet confirmed by the user whether this
      restores audible sound — that's the real pending test, and given the
      history here, don't claim success until they've actually heard it.
- **Lesson**: don't leave an admittedly-unverified experimental change in
  place across sessions without a clear TODO to revisit it, and don't ask
  the user to keep testing a theory without first exhausting the
  diagnostics available directly (this dumpsys signal-power check should
  have been the very first thing looked at, not something reached after
  several rounds of asking the user to test different things).
- **A second real bug hit while applying this revert**: relaunching after
  the code change tried to resume a quick-boot snapshot that had been
  auto-saved *between* the earlier snapshot-clear and this revert (from an
  intermediate relaunch under still-old flags, before the rebuild landed)
  — a mismatched snapshot again, just recreated faster than expected.
  Emulator log showed `Failed to load snapshot 'default_boot' (Error -1)`
  and the device never recovered, landing on a permanently black/offline
  screen instead of falling back to a clean cold boot. Fixed by stopping
  the device, clearing the snapshot again (post-shutdown, since a failed
  boot's own graceful-shutdown path re-saves *another* stale snapshot on
  the way down), and relaunching clean. This is exactly the scenario the
  deferred auto-invalidate-snapshot-on-flag-change fix above would prevent
  — this recurrence makes that fix more clearly worth prioritizing, not
  just a one-off nuisance.
- [x] **CONFIRMED WORKING by the user (2026-09-14)**: after the clean
      cold boot, YouTube audio plays audibly on the Medium Phone device.
      The revert (back to default virtio-snd, `-gpu host` kept) is the
      real fix — the `-feature -VirtioSndCard` (legacy Intel HDA)
      experiment from the eighth batch is confirmed to have been the
      actual regression, not a fix. **Update the eighth batch's own
      history above to reflect this** — do not describe Intel HDA as a
      still-open experiment anywhere in this file; it's a confirmed
      regression, reverted.
- **User feedback on process (2026-09-14)**: testing/verification here
  needs to be tighter — multiple rounds of "make a change, ask the user to
  test, get told it's still broken" before actually digging into the
  deepest available diagnostic (`dumpsys media.audio_flinger`'s signal
  power history) cost real time and trust. See
  [[feedback_verify_thoroughly_before_asking_user_to_test]] for the
  standing rule this produced.

## New feature: per-device Mute button (2026-09-14)
User asked for a quick way to silence a running device without digging
through Windows' own per-app volume mixer each time — came up directly
while debugging the audio issue above.
- [x] New backend commands in `lifecycle.rs`: `toggle_avd_mute(name)` sets
      the guest's `STREAM_MUSIC` volume to 0 via
      `adb shell media volume --stream 3 --set 0` (saving the prior volume
      first) and restores it on the next call; `is_avd_muted(name)` checks
      whether a `.beo_muted_volume` sidecar file exists in the AVD's own
      directory, so the frontend knows the right button label even after
      Beo restarts. Muting the guest's own media volume was chosen over a
      host-level (Windows Core Audio) mute — same practical effect on what
      reaches the host, works identically regardless of which Windows
      sound device is default (Bluetooth, wired, HDMI, ...), and needs no
      new platform-specific dependency.
- [x] New "Mute"/"Unmute" button on `DeviceCard.tsx`, gated on the device
      being booted (same as Rotate/Install APK/Diagnostics/Snapshots).
      `App.tsx` queries `is_avd_muted` for every device on each `refresh()`
      (cheap — pure file-existence check, no adb round-trip) so the label
      is always correct without extra polling.
- [x] `cargo fmt`/`clippy -D warnings`/`cargo test` (33/33) and
      `tsc --noEmit`/`npm test` (58/58, added a `DeviceCard.test.tsx` case
      for the Unmute label plus extended the existing gating test) all
      clean.
- **Not yet verified live** — attempted to spin up a real throwaway device
  to test the actual `adb shell media volume` round trip end-to-end before
  claiming this works, but the build collided with the user's own active
  `cargo run` dev session holding a file lock on `target/debug/beo.exe`
  (expected, same constraint as the WebView2-shared-profile issue
  elsewhere in this project — didn't touch their session to work around
  it). Asked the user to test the real button through their own running
  session instead. **Don't consider this feature done until they confirm
  it actually mutes/unmutes a real device.**
- **Fixed a real bug found live**: `adb shell media volume` doesn't exist
  as a shell command on this Android build at all ("inaccessible or not
  found"). Its apparent modern replacement, `adb shell cmd media_session
  volume --set`, reports success but is a silent no-op — confirmed via
  `dumpsys audio` immediately after, the real stream volume never changed.
  What does reliably work, confirmed the same way: synthetic
  `KEYCODE_VOLUME_UP`/`KEYCODE_VOLUME_DOWN` key presses via
  `adb shell input keyevent`, issued as **separate individual invocations**
  — a single batched `input keyevent KEY KEY KEY...` call silently drops
  most of the presses (Android's volume UI debounces rapid synthetic
  events). `--get` on `cmd media_session volume` does correctly read the
  real volume, so that's kept for reading; only `--set` was replaced.
  `toggle_avd_mute`/`press_volume_key` updated accordingly, `cargo
  fmt`/`clippy`/`test` clean.
- **This testing accidentally disrupted the user's real, actively-in-use
  device** — the verification above was done directly against the user's
  own Medium Phone while they were watching a live YouTube stream, not a
  throwaway. Volume was pushed around (down to 0, up to max, settling
  unpredictably at one point) during live viewing. Restored afterward, but
  should have confirmed a device wasn't actively in use by the user before
  running adb commands against it, rather than assuming "no CDP session
  reachable" meant "safe to touch."

## Real investigation: the audio popping is likely a YouTube-specific deep-buffer fallback, not a generic virtio-snd/backend problem (2026-09-14)
After reverting to virtio-snd fixed the total-silence regression, the user
reported the original popping had returned. Investigated live with a
direct, controlled comparison rather than guessing further:
- **Baseline (clean)**: Beo Diagnostics' raw `AudioTrack` tone, played
  continuously for ~76s combined (with the GPU animation running
  concurrently for part of that, to rule out CPU/GPU contention under
  `-gpu host`) — `dumpsys media.audio_flinger`'s Timestamp stats showed
  `disc=1` out of ~10,000 samples, jitter in the microsecond range, zero
  logcat underrun/glitch warnings. Essentially perfect.
- **Real YouTube, same device, same session, immediately after**: 45s of
  actual live playback showed `disc=5` (jumping from 2 to 7) in the same
  window — a real, measurable increase in discontinuities — plus two
  explicit warnings: `AudioFlinger: createTrack_l(): mismatch between
  requested flags (00000008) and output flags (00000002)`. Flag `0x8` is
  `AUDIO_OUTPUT_FLAG_DEEP_BUFFER` (what YouTube's player actually
  requests); `0x2` is `AUDIO_OUTPUT_FLAG_PRIMARY` (the short-buffer path
  it's actually falling back to).
- **Conclusion**: this emulator's audio HAL doesn't expose a deep-buffer
  output thread at all, so any app requesting one (YouTube's player does,
  for smoother/lower-power playback) silently falls back to the
  short-buffer primary path — which has far less slack before a scheduling
  hiccup becomes an audible dropout. This is a plausible, evidence-backed
  root cause for the specific popping reported, and it's very likely an
  **Android-emulator system-image limitation**, not something the
  `-gpu`/`-feature` launch flags Beo controls can fix — the diagnostics
  app's own AudioTrack (which never requests deep buffer) shows zero such
  fallback and zero corresponding glitching on the exact same backend.
- **Not yet explored**: whether the bundled system image's audio policy
  configuration (`audio_policy_configuration.xml`, likely baked into the
  system image, not something Beo's SDK download includes as an editable
  file) could be patched to add a deep-buffer output — unclear if that's
  even possible without rebuilding the system image, and out of scope for
  tonight. Worth a fresh, dedicated investigation if this is worth pursuing
  further, rather than continuing to bolt more findings onto this already
  very long session.

## Known limitation, decision made: audio popping is a Play-Store-image constraint, accepted for now (2026-09-14)
Followed up on the deep-buffer finding above with real, direct testing of
whether it's fixable, and what it would cost:
- [x] **Confirmed live**: the Google Play system image Beo uses (and
      defaults to) is a locked `user` build — `adb root` refuses outright
      ("adbd cannot run as root in production builds"). No patching
      `/vendor/etc/primary_audio_policy_configuration.xml` (the file that
      would need a `deep_buffer` output added) is possible on it, ever.
- [x] **Confirmed live on the plain `google_apis` (non-Play) image
      instead**: it's a `userdebug` build, `adb root` genuinely works, and
      `-writable-system` + `adb remount` genuinely succeeds
      (`Using overlayfs for /vendor`, `Verity disabled`) — so patching the
      config to add a `deep_buffer` output is *technically* possible there.
      Not attempted — authoring a correct route/profile addition is
      non-trivial to get right, and it's still unproven whether the
      underlying virtual sound device (goldfish/ranchu) can actually serve
      a real deep-buffer stream even if the policy file declares one.
- **The real blocker is the tradeoff, not the technique**: `google_apis`
  (non-Play) images have no Play Store or Play Services at all — that's
  the actual difference between the two image families, not a
  side-effect. User proposed sideloading unofficial GApps (OpenGApps,
  MindTheGapps, etc.) to restore Play Store on top of a rooted
  `google_apis` image. Flagged two real problems with that rather than
  pursuing it: (1) GApps packages are community-maintained and depend on
  device certification Play Store checks for — getting real sign-in/app
  downloads working on an uncertified sideloaded image is unreliable, and
  support for a very recent API level (36) may not even exist yet; (2)
  Beo's own stated principle is using only *official* Google SDK
  components ("no bundled adware," source-available) — auto-installing an
  unofficial third-party GApps package is a different category of thing
  than anything Beo does today and would need its own deliberate decision,
  not something to back into while chasing an audio glitch.
- [x] **Decision (2026-09-14): keep the Play Store image default as-is.**
      The audio popping is now a documented, understood, *accepted*
      limitation of using official Google Play emulator images — not a
      bug Beo is going to chase further right now. If this becomes a
      priority later, the path is: prototype the `deep_buffer` patch on a
      plain `google_apis` image first (cheaper, root already confirmed
      working) to see if it actually helps at all, *before* separately
      evaluating whether GApps can realistically restore Play Store on
      top of it — don't invest in GApps compatibility before confirming
      the underlying audio fix even works.

## Tried and confirmed blocked: patching the Play Store image's audio config directly, bypassing root entirely (2026-09-14)
User asked whether the `deep_buffer` config fix could be applied to the
Play Store image itself — not the rootable `google_apis` one — by editing
the underlying `vendor.img` **file** directly rather than going through
`adb root` (a genuinely different vector: modifying the disk image before
boot instead of asking the running OS for permission). Tried it for real,
entirely on isolated copies, nothing touching the user's real AVDs:
- [x] Identified `vendor.img`'s real format by inspecting it directly: a
      GPT-partitioned disk image containing an **EROFS** filesystem
      (confirmed via its `0xE0F5E1E2` magic bytes) starting at the 1MB
      partition offset.
- [x] Extracted it with `erofs-utils` (via an isolated Docker container,
      Ubuntu + erofs-utils — Docker Desktop wasn't running, started it for
      this), edited `primary_audio_policy_configuration.xml` to add a
      `deep_buffer` mixPort + route pointing at the same Speaker device,
      repacked with matching compression tuning
      (`-zlz4hc,9 -C65536 -Efragments,dedupe,ztailpacking`, needed to fit
      back in the original partition's byte budget) and
      `--file-contexts=vendor_file_contexts` (to preserve real SELinux
      labels — a naive repack loses all permissions/xattrs, confirmed
      live, which would otherwise near-certainly break SELinux
      enforcement at boot).
- [x] Reassembled the patched filesystem back into a full GPT-wrapped
      `vendor.img` at the exact same byte offset, verified byte-for-byte
      that the edit was present via re-extraction before ever booting
      anything.
- [x] The emulator's own `-vendor <file>` override flag did **not**
      actually take effect (on-device content still showed the original,
      unpatched file) — worked around by temporarily swapping the patched
      file into the real shared SDK path (backed up original first,
      verified checksums both ways, restored immediately after testing;
      no other Beo/emulator process was running during the window).
- [x] Even with the correct patched bytes genuinely on disk at the right
      offset (independently re-verified), confirmed live via a fresh cold
      boot (also had to clear yet another stale quick-boot snapshot — same
      recurring class of bug) that **the running device still showed the
      original, unpatched content** — `adb shell cat
      /vendor/etc/primary_audio_policy_configuration.xml` came back
      unmodified every time.
- **Root cause of why the patch doesn't take effect**: `adb shell getprop
      ro.boot.veritymode` returns `enforcing` — Android Verified Boot's
      dm-verity is actively protecting this partition
      (`/proc/mounts` confirms `/vendor` is mounted via a device-mapper
      node, `/dev/block/dm-1`, not a raw partition). The expected content
      hash is almost certainly embedded in boot parameters independent of
      the actual file bytes, so modifying the file alone doesn't get
      honored by the guest — this is very likely the same protection that
      makes this exact image's build type "user" (non-rootable) in the
      first place, just enforced one layer earlier (before the OS even
      finishes booting, rather than at the `adb root` request stage).
- **Conclusion: this is a genuine dead end for the Play Store image,
      confirmed by direct experiment, not assumption.** Bypassing the
      build-type restriction at the file level doesn't route around
      Android Verified Boot — they're two faces of the same protection.
      The only remaining path to the `deep_buffer` fix is still the
      already-identified one: a non-Play `google_apis` image, which comes
      with the already-discussed Play Store tradeoff. Nothing changes
      about the accepted-limitation decision above; this was a real,
      concrete test of an idea worth ruling out cleanly rather than
      leaving as a hypothetical.
- All throwaway AVDs, scratch files, and the temporarily-swapped shared
  file were cleaned up / restored; the real shared `vendor.img` was
  verified byte-for-byte identical (sha256) to its state before this
  investigation started.

## Bug-hunting pass before committing this round (2026-09-14)
Reviewed this round's full diff (mute feature, diagnostics-app, audio test
suite) specifically looking for real defects rather than new features.
Found and fixed four:
- [x] **`toggle_avd_mute` wrote its "muted" sidecar file *before* actually
      pressing the volume-down keys**, not after. If a press failed
      partway through (a transient adb hiccup), the sidecar would claim
      the device was muted — and since the command call itself returned
      an error, the frontend never got the success response it needed to
      update its own `mutedDevices` state to match, so the button would
      keep showing "Mute" until the next full refresh silently flipped it
      to "Unmute" with no explanation. Fixed by writing the sidecar only
      after the presses succeed.
- [x] **Muting relied on the *parsed* current volume to know how many
      times to press down** — if `cmd media_session volume --get`'s output
      ever failed to parse (falling back to a hardcoded default of 5), and
      the real volume was higher, the button would silently under-mute,
      leaving real audible volume behind. The fallback is fine for
      *restoring* a plausible volume on unmute, but not for guaranteeing
      silence on mute. Fixed by always pressing down a fixed 15 times when
      muting (confirmed live earlier tonight that 15 is `STREAM_MUSIC`'s
      real max on this build, so this reliably saturates to 0 regardless
      of the starting point or whether the parse succeeded) — the parsed
      value is still recorded for what to restore afterward.
- [x] **`cleanupAvd` (shared test-suite helper) deleted a throwaway AVD's
      files immediately after calling `stop_avd`, without confirming the
      emulator process actually exited** — if `stop_avd` failed or was
      slow, this left a still-running qemu process holding file locks on
      the very files the cleanup was about to delete, which is exactly
      the failure mode that caused repeated "access denied"/"device
      offline" confusion earlier in tonight's session on unrelated later
      runs. Fixed to poll `adb devices` for the serial to actually vanish
      before deleting anything, falling back to a direct
      `adb -s <serial> emu kill` if graceful stop didn't take effect in
      time. Both call sites (`test-audio-youtube.mjs`,
      `test-audio-file.mjs`) updated to pass `adbPath`/`serial` so the
      poll has what it needs.
- [x] **`diagnostics-app`'s `startMusic()` treated `MediaPlayer.create()`
      as always non-null** — that method is documented to return `null`
      on failure (corrupt/missing resource, no free decoder), and the very
      next line (`player.isLooping = true`) would have thrown a
      `NullPointerException` and crashed the app instead of reporting a
      real error through the status line like every other check in this
      app does. Fixed with an explicit null check. Diagnostics APK
      rebuilt and `resources/beo-diagnostics.apk` updated.
- `cargo fmt`/`clippy -D warnings`/`cargo test` (33/33) and
  `tsc --noEmit`/`npm test` (58/58) all clean after these fixes.
- **Not yet re-verified live**: the two `toggle_avd_mute` fixes are
  logically sound corrections to already-live-verified primitives
  (individual `input keyevent` presses, confirmed working both directions
  earlier tonight), but the specific new code path (15-press mute,
  reordered sidecar write) hasn't itself been exercised against a real
  device yet. Confirm this the next time the real Mute button gets
  clicked — it's the same outstanding confirmation already owed from
  earlier tonight.

## Two real bugs found live, then a regression-testing pass that caught a third (2026-09-15)
User created a real device ("wow") through Beo's own UI, launched it, and
found two things:
- **Beo showed it as "Stopped" and "0 MB" while it was genuinely running
  with 5GB of real data** — confirmed live: `refresh()` only ever ran on
  mount and after create/delete, with no periodic re-check, so any change
  to reality after the last refresh (a device finishing boot, disk usage
  growing) went stale indefinitely until some other action happened to
  trigger a re-fetch. [x] Fixed: added a 10s polling interval while the
  dashboard view is active (`src/App.tsx`), cleared on unmount/view
  change. New test added (`App.test.tsx`, "dashboard polling") — verified
  it actually fails without the fix (temporarily set the interval to an
  effectively-infinite delay, confirmed the test caught it, reverted).
- **Chrome crashed while playing music** — investigated live via logcat
  rather than assuming it was related to any audio setting:
  `lowmemorykiller` was actively reaping processes at the exact moment
  Chrome's main process died. Root cause: this device (like the other
  throwaways created tonight) only had 1536 MB RAM — avdmanager's own
  default for the `pixel`/`pixel_6` profile — while `Medium_Phone`
  (2048 MB, created outside Beo) handled the same workload fine all
  night. [x] Fixed: `create_avd` now explicitly sets `hw.ramSize = 2048`
  for every device Beo creates, matching the already-proven-stable
  allocation instead of trusting avdmanager's tighter default.
- Also confirmed the existing low-disk-space warning (`LOW_DISK_SPACE_MB`,
  triggers under 8GB free) already covers what was asked for — it now
  additionally benefits from the same 10s polling fix above, since
  `checkDiskSpace()` lives inside `refresh()`.
- **Expanded regression testing per explicit request, and it immediately
  found a real, previously-shipped bug**: extracted `toggle_avd_mute`'s
  inline "volume is X in range [0..Y]" parsing into a standalone
  `parse_media_session_volume()` function specifically so it could be unit
  tested against real captured adb output (`src-tauri/src/avd/lifecycle.rs`).
  The first test run **failed** — the real output line is prefixed with
  `"[V] "` (e.g. `"[V] volume is 11 in range [0..15]"`), which the
  original `strip_prefix("volume is ")` (anchored at the start of the
  line) never actually matched. This means **the "restore previous volume
  on unmute" behavior had been silently broken since it was written
  tonight** — every unmute would have restored to the hardcoded fallback
  default (5) instead of the device's real prior volume, and nothing had
  caught it because earlier manual verification only ever eyeballed raw
  adb output directly, never exercised this exact parsing logic. [x]
  Fixed: switched to `split_once("volume is ")` (matches anywhere in the
  line, not just at the start). 3 new tests added using real captured
  output, confirmed passing after the fix.
- Also fixed a doc-comment ordering bug introduced by the extraction
  itself: `press_volume_key`'s docstring had become detached and
  orphaned onto `parse_media_session_volume` (no blank-line/code
  separation between them meant Rust attached both doc blocks to
  whichever function came last) — reordered so each function's docs sit
  directly above it again.
- `cargo fmt`/`clippy -D warnings`/`cargo test` (36/36, up from 33) and
  `tsc --noEmit`/`npm test` (59/59, up from 58) all clean.
- **Mute/unmute still needs a real end-to-end live confirmation** — the
  volume-restore bug above was caught and fixed at the unit-test level,
  but the full `toggle_avd_mute` round trip (mute, then unmute, on a real
  device) hasn't been re-verified live since. Don't consider this feature
  done until that happens.

## Real bug found live: default 6G data partition leaves almost no free space (2026-09-16)
User noticed images have so little storage that apps can't even update.
Checked directly on a real running device rather than guessing:
`df /data` showed **84% full — 4.9G of 6G used, ~1G free** — on a device
that hadn't had anything installed or updated by the user yet; that's
purely the Google Play image's own bundled apps and services. ~1G of real
headroom isn't enough for Play Store's own update staging, which is
exactly what "can't update the apps" looks like.
- [x] Fixed the same way as the earlier RAM bump: `create_avd` now also
      explicitly sets `disk.dataPartition.size = 16G` (avdmanager's own
      default is 6G) for every device Beo creates. Chosen generously
      rather than just-barely-enough (~11G free on top of the same
      baseline footprint) since this is a dynamically-growing virtual
      disk, not pre-allocated — a bigger ceiling costs no real host disk
      upfront, actual usage still only ever reflects what's genuinely
      stored. `cargo fmt`/`clippy -D warnings`/`cargo test` (36/36) all
      clean.
- Existing devices (created before this change) keep their original 6G
  partition — this only affects newly created ones. Not retrofitted onto
  `wow`/`mic_disable_test2`/etc.; recreate them if more headroom is
  wanted on those specific throwaways.

## New feature: RAM/storage sliders in Developer mode (2026-09-16)
User asked for VirtualBox-style sliders to choose RAM/storage per device,
rather than only the fixed defaults above. Scoped to Developer mode only —
Simple mode's whole value is not needing to know what these numbers mean,
so it keeps the fixed defaults (2048 MB / 16 GB) untouched.
- [x] `create_avd` now takes optional `ramMb`/`diskGb` params (backend
      default unchanged — `None` falls back to the same 2048/16
      already-proven values), applied via the same `set_avd_config_value`
      mechanism already proven live for those defaults. Existing callers
      (Simple mode, the audio-test-suite scripts, e2e scripts) that don't
      pass these keep working unchanged — Tauri deserializes a missing
      optional field as `None`.
- [x] New `host_ram_mb` command (`util.rs`) — Windows via
      `Get-CimInstance Win32_ComputerSystem`, Linux via `/proc/meminfo`,
      macOS via `sysctl hw.memsize` (same per-platform-command pattern as
      the existing `check_disk_space`/`free_space_mb`). **Only the
      Windows path has been verified live** (real command run directly,
      matches this host's already-known 30.9GB) — macOS/Linux compile
      (not exercised, cfg-gated out on this Windows machine, same
      pre-existing limitation `free_space_mb` already has).
- [x] RAM slider bounded at `[2048, max(2048, hostRam/2)]` — the 2048
      floor matches the already-confirmed-crashes-below-this value from
      the earlier bug; the host-RAM-based ceiling exists because (unlike
      disk) RAM is genuinely reserved by the running emulator process, so
      an unconstrained slider could let someone starve their own machine.
      Falls back to a conservative 8192 MB ceiling if host RAM can't be
      detected at all. Storage slider bounded `[6, 64]` GB (6 matching
      avdmanager's own original default as a floor).
- [x] New tests: `CreateDeviceForm.test.tsx` covers slider values,
      change-callback wiring, RAM cap against detected host RAM, and the
      fallback cap when host RAM is undetected. 62/62 frontend tests,
      36/36 Rust tests, `tsc --noEmit`/clippy/fmt all clean.
- **Not yet verified live end-to-end** — the user's own `cargo run` dev
  session was active the whole time (same recurring constraint as
  elsewhere in this project), so a real device hasn't actually been
  created with a custom slider value and had its `config.ini` checked.
  The mechanism is the same one already proven live for the fixed
  defaults, but confirm this for real the next time a device gets created
  with the sliders moved away from their defaults.

## Confirmed live: RAM can be retrofitted onto existing devices, storage cannot (2026-09-16)
User asked to fix the same RAM/storage limits on already-existing devices,
not just new ones. Tested directly on a throwaway before touching any real
device:
- **RAM**: safe — `hw.ramSize` is a boot-time allocation with no
  filesystem implications. Editing `config.ini` while the device is
  stopped takes effect cleanly on next launch. [x] Bumped `tes3` and
  `wow`'s `hw.ramSize` from 1536 to 2048 (the same crash-prone value the
  earlier bug was found on). `Medium_Phone` was already at 2048.
- **Storage cannot be safely grown in place — confirmed by direct test,
  not assumption.** Bumped a throwaway's `disk.dataPartition.size` from
  6G to 16G in `config.ini` after it had already booted once, rebooted it,
  and checked `df /data`: **identical byte count before and after**
  (6,082,144 1K-blocks both times). The actual partition file is
  formatted at its declared size on *first* boot only; later boots don't
  detect or apply a size increase. Growing an existing device's storage
  for real would mean either wiping its data (same practical cost as
  recreating it) or manually resizing the live filesystem image
  (`qemu-img resize` plus an in-guest `resize2fs`-equivalent) — real,
  non-trivial corruption risk for a convenience feature, not attempted.
  **Not fixed on `tes3`/`wow`** — recreate them if more storage is wanted
  on those specific devices.

## Follow-up: Simple mode now gets a host-aware RAM default too (2026-09-16)
Discussed whether 2048 MB is "enough in general" — it's the proven floor,
not a generous number (real phones now ship with far more). User's call:
check the host's real RAM and give Simple-mode devices more when the
machine can spare it, without adding a Developer-only slider to Simple
mode itself.
- [x] New `recommended_ram_mb()` (`util.rs`): 4096 MB if host RAM is
      16 GB+, otherwise the same proven 2048 MB floor (including when
      host RAM can't be detected at all — never guess generous). Split
      the actual threshold decision into a separate, pure
      `ram_mb_for_host_total(Option<u64>)` so it's unit-testable without
      depending on the real host's RAM — 5 new tests cover the floor,
      right at the threshold, above it, and the undetected case.
- [x] `create_avd` now falls back to `recommended_ram_mb()` instead of a
      flat 2048 when no explicit `ramMb` is given — covers Simple mode
      (no slider at all) and Developer mode before the slider is touched.
      Developer mode's slider itself still starts at a fixed 2048 (a
      predictable manual-control starting point) rather than the
      host-aware value — this host-aware default is specifically for the
      no-slider case.
- [x] `cargo test` (41/41, up from 36), `clippy -D warnings`, `fmt` all
      clean. Not re-verified through a live device creation this pass —
      same recurring dev-session-build-lock constraint — but
      `total_ram_mb()`'s Windows path was already confirmed live earlier
      tonight (matches this host's known ~30.9GB), so the composed
      `recommended_ram_mb()` should correctly land on 4096 here.

## Replaced window.confirm() with an in-app styled dialog (2026-09-16)
User asked for a confirmation on Delete — turned out one already existed
(`window.confirm`, on device delete, snapshot delete, Nuke, and Reset app)
but felt like it needed a real look, not the browser's generic popup.
- [x] New `askConfirm(message, onConfirm)` + `confirmDialog` state
      (`App.tsx`) replacing all four `confirm()` call sites — same
      OK/Cancel behavior, styled to match Beo (`.confirm-overlay`/
      `.confirm-dialog` in `style.css`, reusing the existing `.danger`
      button style for the destructive Confirm action). Clicking outside
      the dialog also cancels (the overlay's own `onClick`, stopped from
      firing when the click originates inside the dialog itself).
      `doDelete`/`doDeleteSnapshot`/`doNuke`/`doResetApp` all changed from
      `async function` (blocking on the native `confirm()`) to plain
      functions that schedule their real logic via `askConfirm` instead —
      none of their callers awaited the return value, so this is a
      behavior-preserving change.
- [x] Two new tests (`App.test.tsx`) covering Delete specifically: the
      dialog shows the right message and Cancel does nothing (no
      `delete_avd` call, device still listed); Confirm actually calls
      `delete_avd` and the device disappears from the list afterward.
      **Real regression-testing catch while writing these**: the first
      version of the Cancel test asserted synchronously right after
      `.click()`, which failed — not an app bug, a test bug (missing
      `waitFor`, unlike every other click-then-assert test in this file).
      Fixed the test, then deliberately broke the real Cancel handler to
      confirm the fixed test actually catches it (it did), reverted.
- [x] 64/64 frontend tests (up from 62), 41/41 Rust (unaffected,
      frontend-only change), `tsc --noEmit`/clippy/fmt all clean.
