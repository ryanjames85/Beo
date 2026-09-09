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
