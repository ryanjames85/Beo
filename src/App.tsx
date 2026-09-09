import { useEffect, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { getVersion } from "@tauri-apps/api/app";
import Settings, { isNewerVersion, type UpdateCheck } from "./Settings";
import DeviceCard, { type AvdInfo, type SnapshotInfo } from "./DeviceCard";
import CreateDeviceForm, { type Category, type DeviceProfile } from "./CreateDeviceForm";

type View = "checking" | "setup" | "dashboard" | "settings";
type Mode = "simple" | "developer";
type InstallProgress = { stage: string; percent: number | null; detail: string };

const STAGE_LABELS: Record<string, string> = {
  downloading: "Downloading command-line tools",
  extracting: "Extracting",
  jdk: "Downloading Java runtime",
  extracting_jdk: "Extracting Java runtime",
  licenses: "Accepting licenses",
  "platform-tools": "Installing platform-tools",
  emulator: "Installing emulator",
  image: "Downloading system image",
  done: "Done",
};

type DebugEntry = { time: string; message: string };
type NetworkStatus = { online: boolean; detail: string };
type DiskSpaceStatus = { availableMb: number | null };

// A fresh device (before any snapshot) typically needs a few GB; the same
// device after one snapshot can be much more — confirmed by hand, 11 GB for
// one phone with a single boot snapshot. This threshold is deliberately
// conservative and informational, not a hard "will it fit" guarantee.
const LOW_DISK_SPACE_MB = 8192;
type FailedAction = { kind: "install" } | { kind: "create"; name: string; imageId: string; device: string };

const GITHUB_REPO = "ryanjames85/Beo";
const GITHUB_URL = `https://github.com/${GITHUB_REPO}`;
const UPDATE_CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000;

// sdkmanager/reqwest error text varies by platform and failure mode, but
// these substrings show up across the common transient-network cases
// (dropped connection, DNS failure, timeout) — used to show a plain-language
// summary above the raw error rather than leading with a Java stack trace.
const NETWORK_ERROR_PATTERNS = [
  "connection was aborted",
  "connection reset",
  "connection refused",
  "could not resolve",
  "name or service not known",
  "temporary failure in name resolution",
  "timed out",
  "network is unreachable",
  "no route to host",
];

export function isNetworkError(raw: string): boolean {
  const lower = raw.toLowerCase();
  return NETWORK_ERROR_PATTERNS.some((p) => lower.includes(p));
}

// `install_sdk` and `download_image` — the two long-running, cancellable,
// network-dependent commands with a Cancel button and a progress bar —
// reject with this structured shape instead of a plain string (see
// `AppError` in src-tauri/src/util.rs), so `fail()` below can match on
// `kind` for these two instead of substring-matching error text. Every
// other command still rejects with a plain string, handled by the
// isNetworkError()/"Cancelled" text fallback in the same function.
type StructuredAppError = { kind: "Cancelled" } | { kind: "Network" | "Other"; message: string };

function isStructuredAppError(e: unknown): e is StructuredAppError {
  return typeof e === "object" && e !== null && "kind" in e;
}

// Mirrors sanitize_avd_name in src-tauri/src/lib.rs — avdmanager only
// accepts [A-Za-z0-9._-] in AVD names, so free-text input like "My tablet"
// gets mapped to a safe name before it's sent to the backend. Shown to the
// user so they know what name will actually be used; the backend re-applies
// this itself as the real guard.
export function sanitizeAvdName(raw: string): string {
  let out = "";
  let lastWasUnderscore = false;
  for (const c of raw.trim()) {
    if (/[A-Za-z0-9._-]/.test(c)) {
      out += c;
      lastWasUnderscore = c === "_";
    } else if (!lastWasUnderscore) {
      out += "_";
      lastWasUnderscore = true;
    }
  }
  return out.replace(/^_+|_+$/g, "").slice(0, 60);
}

// Mirrors BLOCKED_WORDS / contains_blocked_word in src-tauri/src/lib.rs.
// Word-boundary matched against tokens of the *sanitized* name, not a raw
// substring scan — a substring check would block innocent names for
// containing a bad word as a fragment (e.g. "classic" contains "ass").
const BLOCKED_WORDS = new Set([
  "fuck", "shit", "bitch", "asshole", "cunt", "nigger", "nigga", "faggot",
  "retard", "whore", "slut", "dick", "piss", "cock", "pussy", "bastard",
]);

export function containsBlockedWord(sanitizedName: string): boolean {
  return sanitizedName
    .split(/[^A-Za-z0-9]+/)
    .some((token) => BLOCKED_WORDS.has(token.toLowerCase()));
}

// Small UX touch: the device name stands out from the surrounding status
// text instead of blending into it.
function highlightName(name: string) {
  return <strong className="log-name">{name}</strong>;
}

// Ordered, hand-verified fallback chain for Simple mode's auto-picked
// image — highest-numbered "looks stable" isn't the same as "known to
// actually work". android-36 google_apis_playstore was booted and watched
// for 50+ seconds of real logcat with zero crashes (see project notes);
// 35 and 34 are older GA releases with a long track record elsewhere.
// Kept short and explicit on purpose — this is a curated allowlist, not an
// attempt to auto-detect "stability" from the image list, which is exactly
// the kind of guess that picked the broken android-37.0 preview build
// before. Update this list by hand once a newer level has actually been
// verified to boot cleanly, not just because it's now the newest one Google
// publishes.
const VERIFIED_API_LEVELS = [36, 35, 34];

// Simple mode: skip the picker, just take a known-good, host-ABI Play
// Store image and a profile matching the chosen category. Tries each
// level in VERIFIED_API_LEVELS in order and uses the first one actually
// available for this SDK snapshot + host ABI; only if *none* of those are
// available does it fall back to "highest remaining stable level" so the
// app doesn't just stop working after an SDK update removes an old level
// — but that fallback path is exactly the guess that picked the broken
// preview build before, so it also excludes decimal-versioned preview/beta
// levels (36.1, 37.0, 37.2-beta1) the same way, via the same reasoning:
// Google marks preview/beta channels with a decimal version rather than a
// plain integer (34, 35, 36) or an extension suffix (34-ext10).
export function recommendedImage(images: string[], abi: string): string | undefined {
  const playstore = images.filter((img) => img.includes("google_apis_playstore"));
  const matchingAbi = (pool: string[]) => {
    const withAbi = pool.filter((img) => img.endsWith(`;${abi}`));
    return withAbi.length > 0 ? withAbi : pool;
  };

  for (const level of VERIFIED_API_LEVELS) {
    const candidates = matchingAbi(playstore.filter((img) => img.includes(`android-${level};`)));
    if (candidates.length > 0) return candidates[0];
  }

  // No verified level available for this SDK snapshot — fall back to the
  // newest stable (non-preview) level on offer, same exclusion rule as
  // above, rather than refusing to create a device at all.
  const withLevel = playstore
    .filter((img) => !/android-\d+\.\d+/.test(img))
    .map((img) => ({ img, level: parseInt(img.match(/android-(\d+)/)?.[1] ?? "", 10) }))
    .filter((c) => !Number.isNaN(c.level));
  const pool = matchingAbi(withLevel.map((c) => c.img));
  const byLevel = withLevel.filter((c) => pool.includes(c.img));
  byLevel.sort((a, b) => b.level - a.level);
  return byLevel[0]?.img;
}

// Beo — a clean, open-source Android emulator manager.
// Two modes: "simple" (just use it like a tablet — install apps, Play Store,
// one-tap device) and "developer" (full image/ABI/profile picker + IDE
// integration so VS Code and Android Studio can share the same SDK install).
export default function App() {
  const [view, setView] = useState<View>("checking");
  const [mode, setMode] = useState<Mode>(() => {
    return (localStorage.getItem("beo-mode") as Mode) || "simple";
  });
  const [installing, setInstalling] = useState(false);
  const [creating, setCreating] = useState(false);
  const [avds, setAvds] = useState<AvdInfo[]>([]);
  const [running, setRunning] = useState<Set<string>>(new Set());
  // A device in `running` but not yet in `booted` is "Starting…" rather
  // than "Running" — previously the card flipped straight from Stopped to
  // Running the instant Launch was clicked, with no visible distinction
  // between "the process started" and "the guest OS actually finished
  // booting," even though rotating or installing an APK during that
  // window is liable to just fail.
  const [booted, setBooted] = useState<Set<string>>(new Set());
  const [orientation, setOrientation] = useState<Record<string, "portrait" | "landscape">>({});
  const [images, setImages] = useState<string[]>([]);
  const [profiles, setProfiles] = useState<DeviceProfile[]>([]);
  const [category, setCategory] = useState<Category>("phone");
  const [newName, setNewName] = useState("");
  const [selectedImage, setSelectedImage] = useState("");
  const [device, setDevice] = useState("pixel_6");
  const [playStore, setPlayStore] = useState(true);
  const [log, setLog] = useState<ReactNode>("");
  const [progress, setProgress] = useState<InstallProgress | null>(null);
  const [accel, setAccel] = useState<{ available: boolean; backend: string; detail: string } | null>(null);
  const [devMode, setDevMode] = useState(() => localStorage.getItem("beo-devmode") === "on");
  const [debugLog, setDebugLog] = useState<DebugEntry[]>([]);
  const [abi, setAbi] = useState<string>("x86_64");
  const [network, setNetwork] = useState<NetworkStatus | null>(null);
  const [diskSpaceMb, setDiskSpaceMb] = useState<number | null>(null);
  const [java, setJava] = useState<{ available: boolean; detail: string } | null>(null);
  const [recheckingAccel, setRecheckingAccel] = useState(false);
  const [installingApk, setInstallingApk] = useState<string | null>(null);
  const [snapshotsOpen, setSnapshotsOpen] = useState<Record<string, boolean>>({});
  const [snapshots, setSnapshots] = useState<Record<string, SnapshotInfo[]>>({});
  const [newSnapshotName, setNewSnapshotName] = useState<Record<string, string>>({});
  const [snapshotBusy, setSnapshotBusy] = useState<string | null>(null);
  const [lastFailed, setLastFailed] = useState<FailedAction | null>(null);
  const [version, setVersion] = useState<string | null>(null);
  const [versionError, setVersionError] = useState(false);
  const [updateCheck, setUpdateCheck] = useState<UpdateCheck>({ status: "idle" });
  const [copyLinkHint, setCopyLinkHint] = useState<string | null>(null);

  function toggleDevMode() {
    const next = !devMode;
    setDevMode(next);
    localStorage.setItem("beo-devmode", next ? "on" : "off");
  }

  function logDebug(message: string) {
    const time = new Date().toLocaleTimeString();
    setDebugLog((prev) => [...prev, { time, message }].slice(-200));
  }

  function copyDebugLog() {
    const text = debugLog.map((e) => `[${e.time}] ${e.message}`).join("\n");
    navigator.clipboard.writeText(text);
  }

  function fail(context: string, e: unknown, retry?: FailedAction) {
    if (isStructuredAppError(e)) {
      if (e.kind === "Cancelled") {
        setLog("Cancelled.");
        logDebug(`${context} — cancelled by user`);
        if (retry) setLastFailed(retry);
        return;
      }
      const summary =
        e.kind === "Network"
          ? "Network error — the connection to Google's servers dropped. Check your internet connection and retry."
          : `Failed: ${context}: ${e.message}`;
      setLog(summary);
      logDebug(`ERROR — ${context}: ${e.message}`);
      if (retry) setLastFailed(retry);
      return;
    }

    // Fallback for every other command, which still rejects with a plain
    // string rather than the structured shape above.
    const raw = String(e);
    if (raw.includes("Cancelled")) {
      setLog("Cancelled.");
      logDebug(`${context} — cancelled by user`);
      if (retry) setLastFailed(retry);
      return;
    }
    const summary = isNetworkError(raw)
      ? "Network error — the connection to Google's servers dropped. Check your internet connection and retry."
      : `Failed: ${context}: ${raw}`;
    setLog(summary);
    logDebug(`ERROR — ${context}: ${raw}`);
    if (retry) setLastFailed(retry);
  }

  async function openWindowsFeatures() {
    logDebug("Opening Windows Features dialog");
    try {
      await invoke("open_windows_features");
    } catch (e) {
      logDebug(`Couldn't open Windows Features: ${e}`);
    }
  }

  async function recheckAccel() {
    setRecheckingAccel(true);
    try {
      setAccel(await invoke("check_hardware_accel"));
      logDebug("Rechecked hardware acceleration status");
    } catch (e) {
      logDebug(`Recheck failed: ${e}`);
    } finally {
      setRecheckingAccel(false);
    }
  }

  async function doCancel() {
    logDebug("Cancel requested");
    try {
      await invoke("cancel_sdk_task");
    } catch (e) {
      logDebug(`Cancel failed: ${e}`);
    }
  }

  async function checkNetwork() {
    try {
      setNetwork(await invoke<NetworkStatus>("check_network"));
    } catch {
      // Best-effort — a failed check just means no banner, not a crash.
    }
  }

  async function checkDiskSpace() {
    try {
      const status = await invoke<DiskSpaceStatus>("check_disk_space");
      setDiskSpaceMb(status.availableMb);
    } catch {
      // Best-effort — a failed check just means no banner, not a crash.
      setDiskSpaceMb(null);
    }
  }

  async function checkJava() {
    try {
      setJava(await invoke("check_java"));
    } catch {
      // Best-effort — a failed check just means no banner, not a crash.
    }
  }

  // No auto-updater wired up (that needs a signing keypair — deliberately
  // deferred) — this only checks GitHub's latest release against the
  // running version and, if newer, opens the release page for a manual
  // download. Every failure mode (network down, repo/releases not public
  // yet, a malformed response) surfaces as a real message instead of
  // silently doing nothing. Lives here (not in Settings) so the daily
  // background check below can run for the whole app lifetime, not just
  // while the Settings screen happens to be mounted.
  async function checkForUpdates() {
    if (!version) return;
    setUpdateCheck({ status: "checking" });
    let res: Response;
    try {
      res = await fetch(`https://api.github.com/repos/${GITHUB_REPO}/releases/latest`);
    } catch {
      // fetch() itself only throws for network-layer failures (offline, DNS,
      // TLS, CORS) — genuinely "couldn't reach GitHub at all."
      setUpdateCheck({
        status: "error",
        message: "Couldn't reach GitHub to check for updates — check your internet connection and try again.",
      });
      return;
    }

    if (res.status === 404) {
      // GitHub returns 404 both when the repo has no releases yet *and*
      // when the repo/owner name is simply wrong — can't tell which from
      // the status code alone, so the message doesn't overclaim either way.
      setUpdateCheck({
        status: "error",
        message:
          "No release found — either this repository has no published releases yet, or the repository name is wrong.",
      });
      return;
    }
    if (res.status === 403 || res.status === 429) {
      setUpdateCheck({
        status: "error",
        message: "GitHub is rate-limiting update checks from this network right now — try again in a few minutes.",
      });
      return;
    }
    if (!res.ok) {
      setUpdateCheck({ status: "error", message: `GitHub returned an unexpected response (${res.status}).` });
      return;
    }

    let data: unknown;
    try {
      data = await res.json();
    } catch {
      setUpdateCheck({ status: "error", message: "GitHub's response wasn't valid JSON — try again later." });
      return;
    }
    if (typeof data !== "object" || data === null) {
      setUpdateCheck({ status: "error", message: "GitHub's response wasn't in the expected format." });
      return;
    }

    const record = data as Record<string, unknown>;
    const latest = String(record.tag_name ?? "").replace(/^v/, "");
    const url = typeof record.html_url === "string" ? record.html_url : GITHUB_URL;
    if (!latest) {
      setUpdateCheck({ status: "error", message: "GitHub's response didn't include a version tag." });
      return;
    }

    if (isNewerVersion(latest, version)) {
      setUpdateCheck({ status: "available", version: latest, url });
    } else {
      setUpdateCheck({ status: "up-to-date" });
    }
  }

  async function openReleasePage(url: string) {
    setCopyLinkHint(null);
    try {
      await openUrl(url);
    } catch {
      // The shell plugin call itself failed (not just "browser didn't
      // launch") — leaving this silent would mean clicking the button
      // visibly does nothing with no explanation. Give the user the raw
      // link to copy instead of a dead end.
      setCopyLinkHint(url);
    }
  }

  async function doRetry() {
    if (!lastFailed) return;
    setLog("");
    if (lastFailed.kind === "install") {
      await doInstallSdk();
    } else {
      await createWithImage(lastFailed.name, lastFailed.imageId, lastFailed.device);
    }
  }

  useEffect(() => {
    invoke<boolean>("sdk_status").then((ok) => setView(ok ? "dashboard" : "setup"));
    checkNetwork();
    checkJava();
    // No .catch() here previously meant a rejected getVersion() (unlikely,
    // but possible — an IPC hiccup, a stripped permission) left `version`
    // null forever with the "Check for updates" button silently disabled
    // and no way to tell why. Now it's an explicit, visible state instead.
    getVersion()
      .then(setVersion)
      .catch(() => setVersionError(true));
  }, []);

  // A daily background check so a new release surfaces without the user
  // having to remember to open Settings and click the button — same
  // check, same error handling, just triggered by a stale timestamp
  // instead of a click. Runs once `version` is known (checkForUpdates is a
  // no-op without it) rather than on every render.
  useEffect(() => {
    if (!version) return;
    const last = Number(localStorage.getItem("beo-last-update-check") ?? 0);
    if (Date.now() - last < UPDATE_CHECK_INTERVAL_MS) return;
    localStorage.setItem("beo-last-update-check", String(Date.now()));
    checkForUpdates();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [version]);

  // A stuck/looping boot doesn't error — try_wait() in launch_avd only
  // catches an almost-instant crash, not a boot that just never finishes.
  // Without this there was no way to see *why* a device was stuck at the
  // boot logo, only that it was. This surfaces the emulator's own ongoing
  // log into the debug panel live, for the whole app lifetime (not just
  // while a specific launch call is pending), since boot can take a while
  // and the launch_avd call itself already returned by then.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ name: string; line: string }>("avd_log", (event) => {
      logDebug(`[${event.payload.name}] ${event.payload.line}`);
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, []);

  // Structured counterpart to the avd_log line above — fired once the same
  // sustained boot-completion check actually passes, so the dashboard card
  // can flip from "Starting…" to "Running" without parsing log text.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ name: string }>("avd_booted", (event) => {
      setBooted((prev) => new Set(prev).add(event.payload.name));
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, []);

  useEffect(() => {
    if (view === "dashboard") refresh();
  }, [view]);

  function switchMode(next: Mode) {
    setMode(next);
    localStorage.setItem("beo-mode", next);
  }

  async function refresh() {
    logDebug("Refreshing devices, images, profiles, accel status…");
    try {
      setAvds(await invoke<AvdInfo[]>("list_avds"));
      setImages(await invoke<string[]>("list_available_images"));
      setProfiles(await invoke<DeviceProfile[]>("list_device_profiles"));
      setAccel(await invoke("check_hardware_accel"));
      setAbi(await invoke<string>("preferred_abi"));
      // Ground truth, not just this session's own Launch/Stop clicks — a
      // device launched in an earlier Beo session is still running (it's
      // detached on purpose) even though this session never clicked
      // Launch for it, so relying only on local click history would show
      // it as "Stopped" right after reopening Beo.
      const runningNames = await invoke<string[]>("list_running_avds");
      setRunning(new Set(runningNames));
      // A device reconciled as already-running this way (as opposed to
      // just clicked via doLaunch in this session) has necessarily been up
      // for a while already — the boot-completion event that flips this on
      // normally only fires from the specific launch_avd call's own
      // background thread, which doesn't exist anymore once that process
      // exits, so an already-running device would otherwise show "Starting…"
      // forever after every reopen.
      setBooted(new Set(runningNames));
      checkNetwork();
      checkDiskSpace();
    } catch (e) {
      fail("Refresh failed", e);
    }
  }

  async function doInstallSdk() {
    setInstalling(true);
    setLog("");
    logDebug("install_sdk invoked");
    setProgress({ stage: "downloading", percent: 0, detail: "Starting download…" });
    // listen() itself can reject (e.g. a missing capability grant) — it must
    // be inside the try, or a rejection here skips `finally` entirely and
    // leaves `installing` stuck true forever with no error ever shown.
    let unlisten: (() => void) | undefined;
    try {
      unlisten = await listen<InstallProgress>("sdk_install_progress", (event) => {
        setProgress(event.payload);
      });
      await invoke("install_sdk");
      logDebug("install_sdk completed");
      setLog("");
      setLastFailed(null);
      setView("dashboard");
    } catch (e) {
      fail("install_sdk", e, { kind: "install" });
    } finally {
      unlisten?.();
      setInstalling(false);
      setProgress(null);
    }
  }

  function recommendedProfile(): string {
    const inCategory = profiles.filter((p) => p.category === category);
    const pixel = inCategory.find((p) => p.id.includes("pixel"));
    return (pixel ?? inCategory[0])?.id ?? (category === "tablet" ? "pixel_tablet" : "pixel_6");
  }

  // A system image only shows up in list_available_images because it's
  // *downloadable*, not because it's already installed — sdkmanager has to
  // pull it down before avdmanager can point an AVD at it, or creation
  // fails with "Package path is not valid".
  async function createWithImage(name: string, imageId: string, deviceProfile: string) {
    setCreating(true);
    logDebug(`Creating "${name}" with image ${imageId}, device ${deviceProfile}`);
    setProgress({ stage: "image", percent: null, detail: `Downloading ${imageId}…` });
    // See the comment in doInstallSdk — listen() has to be inside the try,
    // or a rejection here (e.g. a missing capability grant) skips `finally`
    // and leaves `creating` stuck true forever with no error ever shown.
    let unlisten: (() => void) | undefined;
    try {
      unlisten = await listen<InstallProgress>("sdk_install_progress", (event) => {
        setProgress(event.payload);
      });
      setLog(<>Downloading system image for {highlightName(name)}…</>);
      await invoke("download_image", { imageId });
      logDebug(`Image ${imageId} downloaded`);
      setLog(<>Setting up {highlightName(name)}…</>);
      await invoke("create_avd", { name, imageId, device: deviceProfile });
      logDebug(`Device "${name}" created`);
      setLog("");
      setLastFailed(null);
      setNewName("");
      refresh();
    } catch (e) {
      fail(`Create "${name}"`, e, { kind: "create", name, imageId, device: deviceProfile });
    } finally {
      unlisten?.();
      setCreating(false);
      setProgress(null);
    }
  }

  async function doCreateSimple() {
    const image = recommendedImage(images, abi);
    const safeName = sanitizeAvdName(newName);
    if (!safeName || containsBlockedWord(safeName) || !image) {
      setLog(
        !safeName
          ? "Enter a device name with at least one letter or number."
          : containsBlockedWord(safeName)
          ? "That name isn't allowed — please choose something else."
          : "No Play Store image available yet — try again in a moment."
      );
      return;
    }
    await createWithImage(safeName, image, recommendedProfile());
  }

  async function doCreate() {
    const safeName = sanitizeAvdName(newName);
    if (!safeName || containsBlockedWord(safeName) || !selectedImage) {
      if (!safeName) setLog("Enter a device name with at least one letter or number.");
      else if (containsBlockedWord(safeName)) setLog("That name isn't allowed — please choose something else.");
      return;
    }
    await createWithImage(safeName, selectedImage, device);
  }

  async function doLaunch(name: string) {
    logDebug(`Launching "${name}"`);
    setRunning((prev) => new Set(prev).add(name));
    setBooted((prev) => {
      const next = new Set(prev);
      next.delete(name);
      return next;
    });
    // Tablets actually boot landscape (create_avd patches hw.initialOrientation
    // for tablet profiles) — assuming portrait here regardless of category
    // would make the Rotate button's first click a silent no-op (it already
    // matches the device's real starting rotation) while still claiming to
    // have rotated it.
    const isTablet = avds.find((a) => a.name === name)?.category === "tablet";
    setOrientation((prev) => ({ ...prev, [name]: isTablet ? "landscape" : "portrait" }));
    const shareClipboard = localStorage.getItem("beo-clipboard") !== "off";
    try {
      await invoke("launch_avd", { name, headless: false, shareClipboard });
    } catch (e) {
      fail(`Launch "${name}"`, e);
      setRunning((prev) => {
        const next = new Set(prev);
        next.delete(name);
        return next;
      });
    }
  }

  async function doStop(name: string) {
    logDebug(`Stopping "${name}"`);
    try {
      await invoke("stop_avd", { name });
      setRunning((prev) => {
        const next = new Set(prev);
        next.delete(name);
        return next;
      });
      setBooted((prev) => {
        const next = new Set(prev);
        next.delete(name);
        return next;
      });
    } catch (e) {
      fail(`Stop "${name}"`, e);
      // The most common way this fails is the device having already
      // crashed or exited on its own — find_serial_for_avd on the backend
      // can no longer find it running. Without this, `running` stayed
      // stale (still showing "Running" with a Stop button that would just
      // fail the same way forever) until the next full refresh. Reconcile
      // against ground truth instead of only trusting the local click
      // history, same fix as the earlier "stale after reopening the app"
      // bug — this is the same class of gap, just triggered by a failed
      // Stop instead of a restart.
      try {
        const runningNames = await invoke<string[]>("list_running_avds");
        setRunning(new Set(runningNames));
        setBooted(new Set(runningNames));
      } catch {
        // Best-effort reconciliation — if this also fails, the original
        // error above has already been shown, so just leave state as-is.
      }
    }
  }

  async function doInstallApk(name: string) {
    let path: string | null;
    try {
      path = await open({
        multiple: false,
        filters: [{ name: "Android app", extensions: ["apk"] }],
      });
    } catch (e) {
      // Previously unhandled: a dialog-plugin failure here (e.g. a
      // permission or platform-specific picker error) would reject with
      // nothing catching it — no error shown, no visible effect, just
      // silence.
      fail(`Open file picker for "${name}"`, e);
      return;
    }
    if (!path) return;
    logDebug(`Installing ${path} on "${name}"`);
    setInstallingApk(name);
    try {
      const result = await invoke<string>("install_apk", { apkPath: path });
      logDebug(`Install result for "${name}": ${result || "OK"}`);
      setLog(`Installed on ${name}.`);
    } catch (e) {
      fail(`Install APK on "${name}"`, e);
    } finally {
      setInstallingApk(null);
    }
  }

  async function refreshSnapshots(name: string) {
    try {
      const list = await invoke<SnapshotInfo[]>("list_snapshots", { name });
      setSnapshots((prev) => ({ ...prev, [name]: list }));
    } catch (e) {
      fail(`List snapshots for "${name}"`, e);
    }
  }

  function toggleSnapshots(name: string) {
    const opening = !snapshotsOpen[name];
    setSnapshotsOpen((prev) => ({ ...prev, [name]: opening }));
    if (opening) refreshSnapshots(name);
  }

  async function doSaveSnapshot(name: string) {
    const snapName = (newSnapshotName[name] ?? "").trim();
    if (!snapName) return;
    setSnapshotBusy(name);
    try {
      await invoke("save_snapshot", { name, snapshotName: snapName });
      logDebug(`Saved snapshot "${snapName}" for "${name}"`);
      setNewSnapshotName((prev) => ({ ...prev, [name]: "" }));
      await refreshSnapshots(name);
    } catch (e) {
      fail(`Save snapshot for "${name}"`, e);
    } finally {
      setSnapshotBusy(null);
    }
  }

  async function doLoadSnapshot(name: string, snapName: string) {
    setSnapshotBusy(name);
    try {
      await invoke("load_snapshot", { name, snapshotName: snapName });
      logDebug(`Loaded snapshot "${snapName}" for "${name}"`);
    } catch (e) {
      fail(`Load snapshot for "${name}"`, e);
    } finally {
      setSnapshotBusy(null);
    }
  }

  async function doDeleteSnapshot(name: string, snapName: string) {
    if (!confirm(`Delete snapshot "${snapName}"? This can't be undone.`)) return;
    setSnapshotBusy(name);
    try {
      await invoke("delete_snapshot", { name, snapshotName: snapName });
      logDebug(`Deleted snapshot "${snapName}" for "${name}"`);
      await refreshSnapshots(name);
    } catch (e) {
      fail(`Delete snapshot for "${name}"`, e);
    } finally {
      setSnapshotBusy(null);
    }
  }

  async function doRotate(name: string) {
    const next = orientation[name] === "landscape" ? "portrait" : "landscape";
    try {
      await invoke("rotate_avd", { name, orientation: next });
      setOrientation((prev) => ({ ...prev, [name]: next }));
    } catch (e) {
      fail("Rotate", e);
    }
  }

  async function doDelete(name: string) {
    if (!confirm(`Delete "${name}"? This can't be undone.`)) return;
    try {
      await invoke("delete_avd", { name });
      logDebug(`Deleted "${name}"`);
      refresh();
    } catch (e) {
      fail(`Delete "${name}"`, e);
    }
  }

  async function doNuke() {
    if (!confirm("This deletes the SDK, all system images, and all devices. Continue?")) return;
    try {
      await invoke("nuke_all");
      logDebug("nuke_all completed");
      setView("setup");
    } catch (e) {
      fail("Nuke", e);
    }
  }

  // Debug-only: a full reset back to a true first-launch state, for
  // retesting the whole welcome → install → configure → download flow
  // without manually nuking data and clearing storage by hand each time.
  // Unlike doNuke, this also wipes saved preferences (theme, mode, etc.)
  // and reloads the page — nuke_all alone only clears device/SDK data and
  // keeps your settings, which is the right default for a real user but
  // not for testing first-run behavior.
  async function doResetApp() {
    if (
      !confirm(
        "This resets Beo completely — SDK, images, devices, and all saved settings — back to the welcome screen. Continue?"
      )
    )
      return;
    try {
      await invoke("nuke_all");
      localStorage.clear();
      window.location.reload();
    } catch (e) {
      fail("Reset app", e);
    }
  }

  if (view === "checking") {
    return <div className="center">Checking for existing installation…</div>;
  }

  if (view === "settings") {
    return (
      <Settings
        onClose={() => setView("dashboard")}
        mode={mode}
        devMode={devMode}
        onResetApp={doResetApp}
        githubUrl={GITHUB_URL}
        version={version}
        versionError={versionError}
        updateCheck={updateCheck}
        copyLinkHint={copyLinkHint}
        checkForUpdates={checkForUpdates}
        openReleasePage={openReleasePage}
      />
    );
  }

  if (view === "setup") {
    return (
      <div className="center setup">
        <div className="logo-mark">B</div>
        <h1>Beo</h1>
        <p className="tagline">A clean, open-source Android emulator manager</p>

        <div className="feature-card">
          <p className="feature-intro">
            This installs Google's official command-line tools — nothing bundled,
            nothing else running.
          </p>
          <ul className="feature-list">
            <li><span className="check">✓</span> platform-tools + emulator</li>
            <li><span className="check">✓</span> No adware, no telemetry</li>
            <li><span className="check">✓</span> Wipe everything anytime</li>
          </ul>
        </div>

        {network && !network.online && (
          <div className="accel-warning">
            <p className="accel-title">No internet connection</p>
            <p className="accel-detail">{network.detail}</p>
            <p className="accel-detail">Beo needs internet access to download the SDK.</p>
          </div>
        )}

        <button
          className="primary"
          disabled={installing}
          onClick={doInstallSdk}
          title="Downloads Google's official command-line tools, platform-tools, emulator, and its own Java runtime"
        >
          {installing ? "Installing…" : "Install"}
        </button>
        <p className="hint">~230 MB download (includes Beo's own bundled Java runtime)</p>

        {progress && (
          <div className="install-progress">
            <div className="progress-bar">
              <div
                className={progress.percent === null ? "progress-fill indeterminate" : "progress-fill"}
                style={progress.percent !== null ? { width: `${progress.percent}%` } : undefined}
              />
            </div>
            <div className="progress-row">
              <div>
                <p className="progress-label">
                  {STAGE_LABELS[progress.stage] ?? progress.stage}
                  {progress.percent !== null && ` — ${Math.round(progress.percent)}%`}
                </p>
                <p className="hint" style={{ margin: 0 }}>{progress.detail}</p>
              </div>
              <button className="danger" onClick={doCancel} title="Stop the current download">
                Cancel
              </button>
            </div>
          </div>
        )}
        {log && (
          <div className="log-block">
            <pre className="log">{log}</pre>
            {lastFailed && (
              <button onClick={doRetry} title="Try the same operation again with the same settings">
                Retry
              </button>
            )}
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="dashboard">
      <header>
        <div className="brand">
          <div className="logo-mark small">B</div>
          <h1>Beo</h1>
        </div>
        <div className="header-actions">
          <button
            className={devMode ? "dev-toggle active" : "dev-toggle"}
            onClick={toggleDevMode}
            title="Show a debug log of every backend call and error"
          >
            Debug
          </button>
          <button
            onClick={() => setView("settings")}
            title={updateCheck.status === "available" ? `Update available: v${updateCheck.version}` : "App settings and preferences"}
            style={{ position: "relative" }}
          >
            Settings
            {updateCheck.status === "available" && (
              <span
                className="status-dot on"
                style={{ position: "absolute", top: 2, right: 2 }}
              />
            )}
          </button>
          <button
            className="danger"
            onClick={doNuke}
            title="Permanently delete the SDK, all system images, and all devices"
          >
            Nuke all data
          </button>
        </div>
      </header>

      <div className="mode-toggle">
        <button
          className={mode === "simple" ? "mode-btn active" : "mode-btn"}
          onClick={() => switchMode("simple")}
          title="Just name it and go — auto-picks the newest Play Store image for your device"
        >
          Simple — use it like a tablet
        </button>
        <button
          className={mode === "developer" ? "mode-btn active" : "mode-btn"}
          onClick={() => switchMode("developer")}
          title="Full control — pick the exact system image, ABI, and device profile yourself"
        >
          Developer
        </button>
      </div>

      {accel && !accel.available && (
        <div className="accel-warning">
          <p className="accel-title">Hardware acceleration unavailable ({accel.backend})</p>
          <p className="accel-detail">{accel.detail}</p>
          <p className="accel-detail">Devices will still launch but may run very slowly.</p>
          {accel.backend === "WHPX" && (
            <>
              <p className="accel-detail">
                After turning it on, restart your computer — Windows doesn't finish
                enabling it until then, so re-checking here right away will still
                show it as off.
              </p>
              <div style={{ display: "flex", gap: "6px", marginTop: "8px" }}>
                <button
                  onClick={openWindowsFeatures}
                  title="Opens Windows' 'Turn Windows features on or off' dialog"
                >
                  Open Windows Features
                </button>
                <button onClick={recheckAccel} title="Re-check hardware acceleration status right now">
                  {recheckingAccel ? "Checking…" : "Recheck"}
                </button>
              </div>
            </>
          )}
        </div>
      )}

      {network && !network.online && (
        <div className="accel-warning">
          <p className="accel-title">No internet connection</p>
          <p className="accel-detail">{network.detail}</p>
          <p className="accel-detail">Downloading images or the SDK will fail until this is back.</p>
        </div>
      )}

      {java && !java.available && (
        <div className="accel-warning">
          <p className="accel-title">No Java runtime found</p>
          <p className="accel-detail">{java.detail}</p>
        </div>
      )}

      <section>
        <p className="section-label">Your devices</p>
        {avds.length === 0 && <p className="muted">No devices yet — create one below.</p>}
        <div className="avd-cards">
          {avds.map((avd) => (
            <DeviceCard
              key={avd.name}
              avd={avd}
              running={running.has(avd.name)}
              booted={booted.has(avd.name)}
              orientation={orientation[avd.name]}
              installingApk={installingApk === avd.name}
              snapshotsOpen={!!snapshotsOpen[avd.name]}
              snapshots={snapshots[avd.name] ?? []}
              newSnapshotName={newSnapshotName[avd.name] ?? ""}
              snapshotBusy={snapshotBusy === avd.name}
              onRotate={doRotate}
              onInstallApk={doInstallApk}
              onToggleSnapshots={toggleSnapshots}
              onLaunch={doLaunch}
              onStop={doStop}
              onDelete={doDelete}
              onLoadSnapshot={doLoadSnapshot}
              onDeleteSnapshot={doDeleteSnapshot}
              onSaveSnapshot={doSaveSnapshot}
              onNewSnapshotNameChange={(name, value) =>
                setNewSnapshotName((prev) => ({ ...prev, [name]: value }))
              }
            />
          ))}
        </div>
      </section>

      {diskSpaceMb !== null && diskSpaceMb < LOW_DISK_SPACE_MB && (
        <div className="accel-warning">
          <p className="accel-title">Running low on disk space</p>
          <p className="accel-detail">
            About {(diskSpaceMb / 1024).toFixed(1)} GB free. A new device (plus its system image)
            typically needs several GB, more if you save snapshots — this is a heads-up, not a
            hard limit.
          </p>
        </div>
      )}

      {mode === "simple" ? (
        <CreateDeviceForm
          mode="simple"
          category={category}
          onCategoryChange={setCategory}
          newName={newName}
          onNewNameChange={setNewName}
          creating={creating}
          onCreate={doCreateSimple}
        />
      ) : (
        <CreateDeviceForm
          mode="developer"
          newName={newName}
          onNewNameChange={setNewName}
          creating={creating}
          onCreate={doCreate}
          playStore={playStore}
          onPlayStoreChange={setPlayStore}
          selectedImage={selectedImage}
          onSelectedImageChange={setSelectedImage}
          device={device}
          onDeviceChange={setDevice}
          images={images}
          profiles={profiles}
          abi={abi}
        />
      )}

      {progress && (
        <div className="install-progress">
          <div className="progress-bar">
            <div
              className={progress.percent === null ? "progress-fill indeterminate" : "progress-fill"}
              style={progress.percent !== null ? { width: `${progress.percent}%` } : undefined}
            />
          </div>
          <div className="progress-row">
            <div>
              <p className="progress-label">
                {STAGE_LABELS[progress.stage] ?? progress.stage}
                {progress.percent !== null && ` — ${Math.round(progress.percent)}%`}
              </p>
              <p className="hint" style={{ margin: 0 }}>{progress.detail}</p>
            </div>
            <button className="danger" onClick={doCancel}>Cancel</button>
          </div>
        </div>
      )}
      {log && (
        <div className="log-block">
          <pre className="log">{log}</pre>
          {lastFailed && <button onClick={doRetry}>Retry</button>}
        </div>
      )}

      {devMode && (
        <section className="debug-panel">
          <div className="debug-header">
            <p className="section-label" style={{ margin: 0 }}>Debug log</p>
            <button onClick={copyDebugLog} title="Copy the full debug log to your clipboard">
              Copy
            </button>
          </div>
          <pre className="log debug-log">
            {debugLog.length === 0
              ? "No events yet."
              : debugLog.map((e) => `[${e.time}] ${e.message}`).join("\n")}
          </pre>
        </section>
      )}
    </div>
  );
}
