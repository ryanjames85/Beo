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
- [ ] macOS/Linux installers — deferred, revisit once there's a way to
      verify them by hand. **User has a Mac and a Linux VM available for
      this testing** — when picking this back up, add `macos-latest` and
      `ubuntu-22.04` legs back to `release.yml`'s `build` job (dmg/appimage/
      deb targets already declared in `tauri.conf.json`, just need release
      workflow coverage) and verify install/launch/uninstall by hand on
      each, same discipline as the Windows NSIS verification above.

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
