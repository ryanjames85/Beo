import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { open } from "@tauri-apps/plugin-shell";
import { applyTheme, getStoredTheme, type Theme } from "./theme";

type IdeStatus = { enabled: boolean; sdkPath: string; shellProfile: string | null };

const GITHUB_REPO = "ryanjames85/Beo";
const GITHUB_URL = `https://github.com/${GITHUB_REPO}`;

type UpdateCheck =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "up-to-date" }
  | { status: "available"; version: string; url: string }
  | { status: "error"; message: string };

// Numeric semver-ish comparison ("1.2.10" > "1.2.9", unlike a plain string
// compare) — good enough for this app's own version scheme without pulling
// in a full semver library for one comparison.
function isNewerVersion(latest: string, current: string): boolean {
  const toParts = (v: string) => v.split(".").map((p) => parseInt(p, 10) || 0);
  const a = toParts(latest);
  const b = toParts(current);
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const diff = (a[i] ?? 0) - (b[i] ?? 0);
    if (diff !== 0) return diff > 0;
  }
  return false;
}

export default function Settings({
  onClose,
  mode,
  devMode,
  onResetApp,
}: {
  onClose: () => void;
  mode: "simple" | "developer";
  devMode: boolean;
  onResetApp: () => void;
}) {
  const [status, setStatus] = useState<IdeStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [connectedIdes, setConnectedIdes] = useState<string[]>([]);
  const [clipboard, setClipboard] = useState(() => localStorage.getItem("beo-clipboard") !== "off");
  const [theme, setTheme] = useState<Theme>(() => getStoredTheme());
  const [version, setVersion] = useState<string | null>(null);
  const [versionError, setVersionError] = useState(false);
  const [updateCheck, setUpdateCheck] = useState<UpdateCheck>({ status: "idle" });
  const [copyLinkHint, setCopyLinkHint] = useState<string | null>(null);

  useEffect(() => {
    invoke<IdeStatus>("ide_integration_status").then(setStatus);
    refreshConnections();
    const interval = setInterval(refreshConnections, 5000);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    // No .catch() here previously meant a rejected getVersion() (unlikely,
    // but possible — an IPC hiccup, a stripped permission) left `version`
    // null forever with the "Check for updates" button silently disabled
    // and no way to tell why. Now it's an explicit, visible state instead.
    getVersion()
      .then(setVersion)
      .catch(() => setVersionError(true));
  }, []);

  // No auto-updater wired up (that needs a signing keypair — deliberately
  // deferred) — this only checks GitHub's latest release against the
  // running version and, if newer, opens the release page for a manual
  // download. Every failure mode (network down, repo/releases not public
  // yet, a malformed response) surfaces as a real message instead of
  // silently doing nothing, since a "Check for updates" button that can
  // fail invisibly is worse than no button at all.
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
      await open(url);
    } catch {
      // The shell plugin call itself failed (not just "browser didn't
      // launch") — leaving this silent would mean clicking the button
      // visibly does nothing with no explanation. Give the user the raw
      // link to copy instead of a dead end.
      setCopyLinkHint(url);
    }
  }

  async function refreshConnections() {
    try {
      setConnectedIdes(await invoke<string[]>("detect_connected_ides"));
    } catch {
      setConnectedIdes([]);
    }
  }

  async function toggleIde() {
    if (!status) return;
    setBusy(true);
    setMsg("");
    try {
      const result = status.enabled
        ? await invoke<string>("disable_ide_integration")
        : await invoke<string>("enable_ide_integration");
      setMsg(result);
      setStatus(await invoke<IdeStatus>("ide_integration_status"));
    } catch (e) {
      setMsg(`Failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  function copyPath() {
    if (status) navigator.clipboard.writeText(status.sdkPath);
  }

  function toggleClipboard() {
    const next = !clipboard;
    setClipboard(next);
    localStorage.setItem("beo-clipboard", next ? "on" : "off");
  }

  function changeTheme(next: Theme) {
    setTheme(next);
    applyTheme(next);
  }

  return (
    <div className="dashboard">
      <header>
        <div className="brand">
          <button className="back-btn" onClick={onClose} title="Back to the dashboard">
            ← Back
          </button>
        </div>
      </header>

      {mode === "simple" && (
        <p className="hint" style={{ marginBottom: "1rem" }}>
          You're in simple mode — this page is for people who also write
          Android apps. Switch to Developer mode on the dashboard for the
          full picker.
        </p>
      )}

      <section>
        <p className="section-label">Appearance</p>
        <div className="mode-toggle small" style={{ marginBottom: "1.25rem" }}>
          <button
            className={theme === "light" ? "mode-btn active" : "mode-btn"}
            onClick={() => changeTheme("light")}
            title="Always use the light theme"
          >
            Light
          </button>
          <button
            className={theme === "dark" ? "mode-btn active" : "mode-btn"}
            onClick={() => changeTheme("dark")}
            title="Always use the dark theme"
          >
            Dark
          </button>
          <button
            className={theme === "system" ? "mode-btn active" : "mode-btn"}
            onClick={() => changeTheme("system")}
            title="Match your OS's light/dark setting"
          >
            System
          </button>
        </div>
      </section>

      <section>
        <p className="section-label">Copy and paste</p>
        <div className="feature-card" style={{ textAlign: "left" }}>
          <p className="feature-intro">
            Share the clipboard between your computer and the device — same as
            Android Studio's emulator. Applies the next time you launch a device.
          </p>
          <label className="checkbox-row">
            <input type="checkbox" checked={clipboard} onChange={toggleClipboard} />
            Share clipboard with host
          </label>
        </div>
      </section>

      <section>
        <p className="section-label">Use with another IDE</p>
        <div className="feature-card" style={{ textAlign: "left" }}>
          <p className="feature-intro">
            Beo's SDK is a standalone install — point Android Studio, VS Code,
            or any other tool at it instead of downloading a separate copy.
            No extension needed: IDEs find it via <code>ANDROID_HOME</code>.
          </p>

          <p className="section-label" style={{ marginTop: "12px" }}>SDK path</p>
          <div className="path-row">
            <code className="path-value">{status?.sdkPath ?? "…"}</code>
            <button onClick={copyPath} title="Copy the full SDK path to your clipboard">
              Copy
            </button>
          </div>

          <p className="feature-intro" style={{ marginTop: "12px" }}>
            {status?.enabled
              ? "ANDROID_HOME is set in your shell profile. Any IDE launched from a terminal will pick it up."
              : "Set ANDROID_HOME so IDEs and terminals can find this SDK automatically."}
          </p>
          <button
            className="primary"
            disabled={busy || !status}
            onClick={toggleIde}
            title={
              status?.enabled
                ? "Removes ANDROID_HOME from your shell profile"
                : "Sets ANDROID_HOME so other tools can find this SDK"
            }
          >
            {busy ? "Working…" : status?.enabled ? "Disable IDE integration" : "Enable IDE integration"}
          </button>

          <p className="section-label" style={{ marginTop: "16px" }}>Currently connected</p>
          {connectedIdes.length > 0 ? (
            <ul className="feature-list">
              {connectedIdes.map((ide) => (
                <li key={ide}>
                  <span className="status-dot on" style={{ marginRight: "2px" }} />
                  {ide} is running
                </li>
              ))}
            </ul>
          ) : (
            <p className="hint" style={{ margin: 0 }}>
              No IDE detected running right now.
            </p>
          )}
          <p className="hint" style={{ marginTop: "6px" }}>
            This shows whether Android Studio or VS Code is open — not proof
            either one is pointed at this specific SDK, since adb doesn't
            report that directly.
          </p>
        </div>
      </section>

      {msg && <p className="hint">{msg}</p>}

      {devMode && (
        <section>
          <p className="section-label">Debug</p>
          <div className="feature-card" style={{ textAlign: "left" }}>
            <p className="feature-intro">
              Wipes the SDK, images, devices, and every saved setting, then
              reloads to the welcome screen — for retesting the full
              install → configure → download flow from a clean slate instead
              of doing it by hand each time.
            </p>
            <button
              className="danger"
              onClick={onResetApp}
              title="Deletes everything and reloads to the very first launch screen"
            >
              Reset app to welcome screen
            </button>
          </div>
        </section>
      )}

      <section>
        <p className="section-label">About</p>
        <div className="feature-card" style={{ textAlign: "left" }}>
          <p className="feature-intro">
            <strong>Beo</strong> {version ? `v${version}` : ""} — a clean, open-source Android
            emulator manager. MIT licensed, no adware, no telemetry.
          </p>
          {versionError && (
            <p className="hint blocked-hint" style={{ margin: "0 0 8px" }}>
              Couldn't determine the running version, so update checks are unavailable this
              session.
            </p>
          )}
          <div style={{ display: "flex", gap: "6px", flexWrap: "wrap" }}>
            <button onClick={() => openReleasePage(GITHUB_URL)} title="View source, issues, and releases on GitHub">
              View on GitHub
            </button>
            <button
              onClick={checkForUpdates}
              disabled={updateCheck.status === "checking" || !version}
              title={
                version
                  ? "Checks GitHub's latest release against this version"
                  : "Unavailable until the running version can be determined"
              }
            >
              {updateCheck.status === "checking" ? "Checking…" : "Check for updates"}
            </button>
          </div>

          {copyLinkHint && (
            <p className="hint" style={{ marginTop: "8px" }}>
              Couldn't open a browser automatically — copy this link instead:{" "}
              <code className="path-value" style={{ display: "inline", padding: "2px 6px" }}>
                {copyLinkHint}
              </code>
            </p>
          )}

          {updateCheck.status === "up-to-date" && (
            <p className="hint" style={{ marginTop: "8px" }}>
              You're up to date (v{version}).
            </p>
          )}
          {updateCheck.status === "available" && (
            <div className="accel-warning" style={{ marginTop: "8px" }}>
              <p className="accel-title">Update available: v{updateCheck.version}</p>
              <p className="accel-detail">
                You're on v{version}. Beo doesn't auto-install updates yet — download the new
                version from the release page.
              </p>
              <button
                className="primary"
                style={{ marginTop: "6px" }}
                onClick={() => openReleasePage(updateCheck.status === "available" ? updateCheck.url : GITHUB_URL)}
              >
                Open release page
              </button>
            </div>
          )}
          {updateCheck.status === "error" && (
            <div className="accel-warning" style={{ marginTop: "8px" }}>
              <p className="accel-title">Couldn't check for updates</p>
              <p className="accel-detail">{updateCheck.message}</p>
              <button style={{ marginTop: "6px" }} onClick={checkForUpdates}>
                Retry
              </button>
            </div>
          )}
        </div>
      </section>
    </div>
  );
}
