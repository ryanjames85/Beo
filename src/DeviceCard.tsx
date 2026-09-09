export type AvdInfo = {
  name: string;
  category: string;
  // Both null when the info couldn't be read (e.g. the .avd directory or
  // config.ini couldn't be opened) — shown as "unknown" rather than 0, so
  // a read failure never reads as "this device takes no space."
  diskUsageMb: number | null;
  ramMb: number | null;
};
export type SnapshotInfo = { name: string; size: string; date: string };

// "1.2 GB" above 1024 MB, otherwise "512 MB" — real usage on a device with
// even one snapshot easily runs into multiple GB (confirmed by hand: 11 GB
// for one phone with a single boot snapshot), so GB is the common case,
// not the exception.
function formatMb(mb: number): string {
  return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${Math.round(mb)} MB`;
}

type Props = {
  avd: AvdInfo;
  running: boolean;
  // True once the guest OS has actually finished booting (a sustained
  // check on the backend, not just "the process started") — a device can
  // be `running` for a while before this flips. Rotate/Install APK/
  // Snapshots are gated on this too, not just `running`: trying any of
  // them before the guest is actually up just fails.
  booted: boolean;
  orientation: "portrait" | "landscape" | undefined;
  installingApk: boolean;
  snapshotsOpen: boolean;
  snapshots: SnapshotInfo[];
  newSnapshotName: string;
  snapshotBusy: boolean;
  onRotate: (name: string) => void;
  onInstallApk: (name: string) => void;
  onToggleSnapshots: (name: string) => void;
  onLaunch: (name: string) => void;
  onStop: (name: string) => void;
  onDelete: (name: string) => void;
  onLoadSnapshot: (name: string, snapName: string) => void;
  onDeleteSnapshot: (name: string, snapName: string) => void;
  onSaveSnapshot: (name: string) => void;
  onNewSnapshotNameChange: (name: string, value: string) => void;
};

// One device's card on the dashboard, plus its expandable snapshot panel —
// extracted out of App.tsx (which was growing a new prop/branch here for
// every device-level feature added) so this stays the one place to look
// when changing anything about how a single device is presented, without
// wading through the rest of the dashboard's layout and state wiring.
export default function DeviceCard({
  avd,
  running,
  booted,
  orientation,
  installingApk,
  snapshotsOpen,
  snapshots,
  newSnapshotName,
  snapshotBusy,
  onRotate,
  onInstallApk,
  onToggleSnapshots,
  onLaunch,
  onStop,
  onDelete,
  onLoadSnapshot,
  onDeleteSnapshot,
  onSaveSnapshot,
  onNewSnapshotNameChange,
}: Props) {
  const { name, category, diskUsageMb, ramMb } = avd;
  const ready = running && booted;
  const statusLabel = running ? (booted ? "Running" : "Starting…") : "Stopped";

  return (
    <div>
      <div className="avd-card">
        <div className="avd-info">
          <span
            className={`status-dot ${running ? (booted ? "on" : "starting") : ""}`}
            title={statusLabel}
          />
          <div>
            <p className="avd-name">
              {name}
              <span className={`device-tag ${category}`}>{category === "tablet" ? "Tablet" : "Phone"}</span>
            </p>
            <p
              className="avd-meta"
              title="Disk usage is the real size of everything under this device's folder (images, snapshots) — RAM is what the emulator allocates at launch"
            >
              {statusLabel}
              {" · "}
              {diskUsageMb !== null ? formatMb(diskUsageMb) : "size unknown"}
              {" · "}
              {ramMb !== null ? `${ramMb} MB RAM` : "RAM unknown"}
            </p>
          </div>
        </div>
        <div className="avd-actions">
          {running && (
            <button
              onClick={() => onRotate(name)}
              disabled={!ready}
              title={
                ready
                  ? `Rotate to ${orientation === "landscape" ? "portrait" : "landscape"}`
                  : "Available once the device finishes booting"
              }
            >
              ⟳ Rotate to {orientation === "landscape" ? "Portrait" : "Landscape"}
            </button>
          )}
          {running && (
            <button
              onClick={() => onInstallApk(name)}
              disabled={!ready || installingApk}
              title={ready ? "Pick an .apk file to sideload onto this device" : "Available once the device finishes booting"}
            >
              {installingApk ? "Installing…" : "Install APK"}
            </button>
          )}
          {running && (
            <button
              onClick={() => onToggleSnapshots(name)}
              disabled={!ready}
              title={
                ready
                  ? "Save or restore snapshots of this device's current state"
                  : "Available once the device finishes booting"
              }
            >
              {snapshotsOpen ? "Hide snapshots" : "Snapshots"}
            </button>
          )}
          {running ? (
            <button className="danger" onClick={() => onStop(name)} title="Shut down this running device">
              Stop
            </button>
          ) : (
            <button onClick={() => onLaunch(name)} title="Start this device in the emulator">
              Launch
            </button>
          )}
          <button
            className="danger"
            onClick={() => onDelete(name)}
            title="Permanently delete this device — can't be undone"
          >
            Delete
          </button>
        </div>
      </div>

      {running && snapshotsOpen && (
        <div className="snapshot-panel">
          {snapshots.length === 0 ? (
            <p className="hint" style={{ margin: 0 }}>
              No snapshots yet.
            </p>
          ) : (
            <ul className="feature-list">
              {snapshots.map((snap) => (
                <li key={snap.name}>
                  <span style={{ flex: 1 }}>
                    {snap.name} <span className="muted">— {snap.size}, {snap.date}</span>
                  </span>
                  <button
                    onClick={() => onLoadSnapshot(name, snap.name)}
                    disabled={snapshotBusy}
                    title="Restore the device to this snapshot's state"
                  >
                    Load
                  </button>
                  <button
                    className="danger"
                    onClick={() => onDeleteSnapshot(name, snap.name)}
                    disabled={snapshotBusy}
                    title="Permanently delete this snapshot"
                  >
                    Delete
                  </button>
                </li>
              ))}
            </ul>
          )}
          <div className="path-row" style={{ marginTop: "8px" }}>
            <input
              placeholder="New snapshot name"
              value={newSnapshotName}
              onChange={(e) => onNewSnapshotNameChange(name, e.target.value)}
            />
            <button
              className="primary"
              onClick={() => onSaveSnapshot(name)}
              disabled={snapshotBusy || !newSnapshotName.trim()}
              title="Save the device's current state as a new snapshot"
            >
              {snapshotBusy ? "Working…" : "Save"}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
