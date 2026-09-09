import { sanitizeAvdName, containsBlockedWord } from "./App";

export type Category = "phone" | "tablet";
export type DeviceProfile = { id: string; label: string; category: string };

type SimpleProps = {
  mode: "simple";
  category: Category;
  onCategoryChange: (c: Category) => void;
  newName: string;
  onNewNameChange: (v: string) => void;
  creating: boolean;
  onCreate: () => void;
};

type DeveloperProps = {
  mode: "developer";
  newName: string;
  onNewNameChange: (v: string) => void;
  creating: boolean;
  onCreate: () => void;
  playStore: boolean;
  onPlayStoreChange: (v: boolean) => void;
  selectedImage: string;
  onSelectedImageChange: (v: string) => void;
  device: string;
  onDeviceChange: (v: string) => void;
  images: string[];
  profiles: DeviceProfile[];
  abi: string;
};

type Props = SimpleProps | DeveloperProps;

// The name-validation hint shown under the name field in both modes —
// pulled out once since Simple and Developer duplicated it exactly.
function NameHint({ newName }: { newName: string }) {
  const trimmed = newName.trim();
  if (trimmed === "") return null;
  const safe = sanitizeAvdName(newName);
  if (containsBlockedWord(safe)) {
    return (
      <p className="hint blocked-hint" style={{ margin: 0 }}>
        That name isn't allowed — please choose something else.
      </p>
    );
  }
  if (safe !== trimmed) {
    return (
      <p className="hint" style={{ margin: 0 }}>
        Will be created as "{safe || "…"}" (device names only allow letters, numbers, "." "_" "-")
      </p>
    );
  }
  return null;
}

// The "New device" / "Create a device" section on the dashboard — extracted
// out of App.tsx, which carried two full form variants (Simple vs.
// Developer mode) inline. Simple mode just wants a name and a category;
// Developer mode exposes the full image/ABI/profile picker straight from
// the SDK's own listings.
export default function CreateDeviceForm(props: Props) {
  if (props.mode === "simple") {
    const { category, onCategoryChange, newName, onNewNameChange, creating, onCreate } = props;
    return (
      <section>
        <p className="section-label">New device</p>
        <div className="create-card">
          <div className="mode-toggle small">
            <button
              className={category === "phone" ? "mode-btn active" : "mode-btn"}
              onClick={() => onCategoryChange("phone")}
              title="Phone-sized screen and profile"
            >
              Phone
            </button>
            <button
              className={category === "tablet" ? "mode-btn active" : "mode-btn"}
              onClick={() => onCategoryChange("tablet")}
              title="Tablet-sized screen and profile"
            >
              Tablet
            </button>
          </div>
          <input
            placeholder="Name it anything, e.g. 'My tablet'"
            value={newName}
            onChange={(e) => onNewNameChange(e.target.value)}
            title="Any name — it'll be adapted to what Android's tooling allows"
          />
          <NameHint newName={newName} />
          <p className="hint" style={{ margin: 0 }}>
            Includes the Play Store, ready to sign in and install apps.
          </p>
          <button
            className="primary create-btn"
            disabled={creating}
            onClick={onCreate}
            title="Downloads the system image (if needed) and creates the device"
          >
            {creating ? "Creating…" : "+ Create"}
          </button>
        </div>
      </section>
    );
  }

  const {
    newName,
    onNewNameChange,
    creating,
    onCreate,
    playStore,
    onPlayStoreChange,
    selectedImage,
    onSelectedImageChange,
    device,
    onDeviceChange,
    images,
    profiles,
    abi,
  } = props;

  return (
    <section>
      <p className="section-label">Create a device</p>
      <div className="create-card">
        <input placeholder="Device name" value={newName} onChange={(e) => onNewNameChange(e.target.value)} />
        <NameHint newName={newName} />
        <label className="checkbox-row">
          <input type="checkbox" checked={playStore} onChange={(e) => onPlayStoreChange(e.target.checked)} />
          Include Play Store (needed for apps that require it installed)
        </label>
        <select
          value={selectedImage}
          onChange={(e) => onSelectedImageChange(e.target.value)}
          title="The Android system image this device will run — matches your host's CPU architecture where possible"
        >
          <option value="">Select system image…</option>
          {images
            .filter((img) =>
              playStore ? img.includes("google_apis_playstore") : img.includes("google_apis") && !img.includes("playstore")
            )
            // Host-ABI images first — anything else either won't launch
            // or runs unaccelerated (e.g. arm64-v8a on an x86_64 host).
            .sort((a, b) => Number(b.endsWith(`;${abi}`)) - Number(a.endsWith(`;${abi}`)))
            .map((img) => (
              <option key={img} value={img}>
                {img}
                {!img.endsWith(`;${abi}`) ? ` (not ${abi} — this host's arch)` : ""}
              </option>
            ))}
        </select>
        <select
          value={device}
          onChange={(e) => onDeviceChange(e.target.value)}
          title="Hardware profile — screen size, resolution, and RAM"
        >
          <optgroup label="Phones">
            {profiles
              .filter((p) => p.category === "phone")
              .map((p) => (
                <option key={p.id} value={p.id}>
                  {p.label}
                </option>
              ))}
          </optgroup>
          <optgroup label="Tablets">
            {profiles
              .filter((p) => p.category === "tablet")
              .map((p) => (
                <option key={p.id} value={p.id}>
                  {p.label}
                </option>
              ))}
          </optgroup>
        </select>
        <button
          className="primary create-btn"
          disabled={creating}
          onClick={onCreate}
          title="Downloads the system image (if needed) and creates the device"
        >
          {creating ? "Creating…" : "+ Create"}
        </button>
      </div>
    </section>
  );
}
