import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { applyTheme, getStoredTheme, type Theme } from "./theme";

type IdeStatus = { enabled: boolean; sdkPath: string; shellProfile: string | null };

export type UpdateCheck =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "up-to-date" }
  | { status: "available"; version: string; url: string }
  | { status: "error"; message: string };

// Numeric semver-ish comparison ("1.2.10" > "1.2.9", unlike a plain string
// compare) — good enough for this app's own version scheme without pulling
// in a full semver library for one comparison.
export function isNewerVersion(latest: string, current: string): boolean {
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
  githubUrl,
  version,
  versionError,
  updateCheck,
  copyLinkHint,
  checkForUpdates,
  openReleasePage,
}: {
  onClose: () => void;
  mode: "simple" | "developer";
  devMode: boolean;
  onResetApp: () => void;
  githubUrl: string;
  version: string | null;
  versionError: boolean;
  updateCheck: UpdateCheck;
  copyLinkHint: string | null;
  checkForUpdates: () => void;
  openReleasePage: (url: string) => void;
}) {
  const [status, setStatus] = useState<IdeStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [connectedIdes, setConnectedIdes] = useState<string[]>([]);
  const [clipboard, setClipboard] = useState(() => localStorage.getItem("beo-clipboard") !== "off");
  const [theme, setTheme] = useState<Theme>(() => getStoredTheme());

  useEffect(() => {
    invoke<IdeStatus>("ide_integration_status").then(setStatus);
    refreshConnections();
    const interval = setInterval(refreshConnections, 5000);
    return () => clearInterval(interval);
  }, []);

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
            <strong>Beo</strong> {version ? `v${version}` : ""} — a clean, source-available Android
            emulator manager. PolyForm Shield 1.0.0 licensed, no adware, no telemetry.
          </p>
          {versionError && (
            <p className="hint blocked-hint" style={{ margin: "0 0 8px" }}>
              Couldn't determine the running version, so update checks are unavailable this
              session.
            </p>
          )}
          <div style={{ display: "flex", gap: "6px", flexWrap: "wrap" }}>
            <button onClick={() => openReleasePage(githubUrl)} title="View source, issues, and releases on GitHub">
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
                onClick={() => openReleasePage(updateCheck.status === "available" ? updateCheck.url : githubUrl)}
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
