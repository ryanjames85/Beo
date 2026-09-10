import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import App, {
  sanitizeAvdName,
  containsBlockedWord,
  isNetworkError,
  recommendedImage,
  explainInstallApkError,
  initialOrientationForLaunch,
} from "./App";

describe("sanitizeAvdName", () => {
  it("replaces disallowed characters with underscores", () => {
    expect(sanitizeAvdName("My tablet")).toBe("My_tablet");
    expect(sanitizeAvdName("weird name!! @@ ##")).toBe("weird_name");
  });

  it("collapses runs and trims leading/trailing underscores only", () => {
    // '.', '-', '_' are all allowed characters, so only leading/trailing
    // *underscores* get trimmed — a leading/trailing '.' is left as-is.
    expect(sanitizeAvdName("  __leading")).toBe("leading");
    expect(sanitizeAvdName("trailing__  ")).toBe("trailing");
    expect(sanitizeAvdName("a   b")).toBe("a_b");
  });

  it("allows dots, dashes, and underscores", () => {
    expect(sanitizeAvdName("my.device-1_test")).toBe("my.device-1_test");
  });

  it("returns an empty string when nothing valid remains", () => {
    expect(sanitizeAvdName("!!! @@@ ###")).toBe("");
    expect(sanitizeAvdName("   ")).toBe("");
  });

  it("truncates to 60 characters", () => {
    expect(sanitizeAvdName("a".repeat(100)).length).toBe(60);
  });
});

describe("containsBlockedWord", () => {
  it("matches whole tokens only, not substrings", () => {
    expect(containsBlockedWord("fuck_device")).toBe(true);
    expect(containsBlockedWord("my_shit_phone")).toBe(true);
    // contains "ass" as a substring, not a token
    expect(containsBlockedWord("classic_assistant")).toBe(false);
    expect(containsBlockedWord("my_tablet")).toBe(false);
  });

  it("is case-insensitive", () => {
    expect(containsBlockedWord("FUCK")).toBe(true);
    expect(containsBlockedWord("FuCk_device")).toBe(true);
  });
});

describe("isNetworkError", () => {
  it("recognizes common transient-network error phrases", () => {
    expect(isNetworkError("Error: connection was aborted")).toBe(true);
    expect(isNetworkError("request timed out after 30s")).toBe(true);
    expect(isNetworkError("dial tcp: no route to host")).toBe(true);
  });

  it("does not flag unrelated errors as network errors", () => {
    expect(isNetworkError("Package path is not valid")).toBe(false);
    expect(isNetworkError("Device name needs at least one letter or number.")).toBe(false);
  });
});

describe("recommendedImage", () => {
  // Real system-image IDs, in the exact shape sdkmanager --list produces
  // (captured live from this project's own SDK install) — including the
  // decimal-versioned preview builds (37.0, 36.1) that this function's
  // exclusion logic exists specifically to avoid picking.
  const REAL_IMAGES = [
    "system-images;android-28;google_apis_playstore;x86_64",
    "system-images;android-34;google_apis_playstore;x86_64",
    "system-images;android-35;google_apis_playstore;x86_64",
    "system-images;android-35;google_apis_playstore_tablet;x86_64",
    "system-images;android-36;google_apis_playstore;x86_64",
    "system-images;android-36.1;google_apis_playstore;x86_64",
    "system-images;android-37.0;google_apis_playstore;x86_64",
    "system-images;android-28;google_apis_playstore;arm64-v8a",
    "system-images;android-36;google_apis_playstore;arm64-v8a",
  ];

  it("picks the highest verified level (36) over lower verified levels", () => {
    expect(recommendedImage(REAL_IMAGES, "x86_64")).toBe(
      "system-images;android-36;google_apis_playstore;x86_64"
    );
  });

  it("prefers the host ABI when both are available", () => {
    expect(recommendedImage(REAL_IMAGES, "arm64-v8a")).toBe(
      "system-images;android-36;google_apis_playstore;arm64-v8a"
    );
  });

  it("falls back to the next verified level when the top one is unavailable", () => {
    const withoutThirtySix = REAL_IMAGES.filter((img) => !img.includes("android-36;"));
    expect(recommendedImage(withoutThirtySix, "x86_64")).toBe(
      "system-images;android-35;google_apis_playstore;x86_64"
    );
  });

  it("falls back to the highest non-preview level when no verified level exists, excluding decimal-versioned preview builds", () => {
    // This is the exact real incident this function was built to prevent:
    // with only pre-release images available, a naive "pick the highest
    // number" would choose android-37.0 (a preview build that crashed on
    // boot) over the real android-28 GA release.
    const onlyOldAndPreview = [
      "system-images;android-28;google_apis_playstore;x86_64",
      "system-images;android-36.1;google_apis_playstore;x86_64",
      "system-images;android-37.0;google_apis_playstore;x86_64",
    ];
    expect(recommendedImage(onlyOldAndPreview, "x86_64")).toBe(
      "system-images;android-28;google_apis_playstore;x86_64"
    );
  });

  it("returns undefined when no Play Store image is available at all", () => {
    expect(recommendedImage(["system-images;android-34;google_apis;x86_64"], "x86_64")).toBeUndefined();
  });
});

describe("explainInstallApkError", () => {
  it("translates an ABI mismatch into plain English", () => {
    const msg = explainInstallApkError(
      "adb: failed to install app.apk: Failure [INSTALL_FAILED_NO_MATCHING_ABIS: Failed to extract native libraries]",
      "Medium_Phone_API_36.0"
    );
    expect(msg).toContain("architecture");
    expect(msg).toContain("Medium_Phone_API_36.0");
  });

  it("translates an SDK-version mismatch into plain English", () => {
    const msg = explainInstallApkError("Failure [INSTALL_FAILED_OLDER_SDK]", "tes3");
    expect(msg).toContain("newer Android version");
  });

  it("translates a signature/version conflict into plain English", () => {
    expect(explainInstallApkError("Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE]", "tes3")).toContain(
      "Uninstall the existing app"
    );
    expect(explainInstallApkError("Failure [INSTALL_FAILED_VERSION_DOWNGRADE]", "tes3")).toContain(
      "Uninstall the existing app"
    );
  });

  it("translates an invalid/corrupt APK into plain English", () => {
    expect(explainInstallApkError("Failure [INSTALL_FAILED_INVALID_APK]", "tes3")).toContain("valid APK");
    expect(explainInstallApkError("cmd: INSTALL_PARSE_FAILED_NOT_APK", "tes3")).toContain("valid APK");
  });

  it("translates insufficient storage into plain English", () => {
    expect(explainInstallApkError("Failure [INSTALL_FAILED_INSUFFICIENT_STORAGE]", "tes3")).toContain(
      "out of storage"
    );
  });

  it("falls back to the raw message for an unrecognized failure", () => {
    const msg = explainInstallApkError("some never-before-seen adb error", "tes3");
    expect(msg).toContain("tes3");
    expect(msg).toContain("some never-before-seen adb error");
  });
});

describe("initialOrientationForLaunch", () => {
  const avds = [
    { name: "phone1", category: "phone", diskUsageMb: 100, ramMb: 2048 },
    { name: "tablet1", category: "tablet", diskUsageMb: 100, ramMb: 2048 },
  ];

  it("defaults to portrait for a phone", () => {
    expect(initialOrientationForLaunch(avds, "phone1")).toBe("portrait");
  });

  it("defaults to landscape for a tablet — create_avd patches hw.initialOrientation for tablets", () => {
    // This is the exact real bug it guards: assuming portrait regardless
    // of category made the Rotate button's first click on a fresh tablet
    // a silent no-op (it already matched the device's real starting
    // rotation) while still claiming to have rotated it.
    expect(initialOrientationForLaunch(avds, "tablet1")).toBe("landscape");
  });

  it("defaults to portrait for an unknown device name", () => {
    expect(initialOrientationForLaunch(avds, "does_not_exist")).toBe("portrait");
  });
});

// --- Component tests ---
//
// App.tsx owns ~28 Tauri commands' worth of state, and mocking that entire
// surface just to render it was explicitly judged poor ROI (see TODO.md /
// project memory). This mocks only the minimal set of commands the mount
// path actually calls to reach a stable dashboard render with one device,
// then targets two specific pieces of real reconciliation logic that were
// previously only ever verified by hand: doStop's failure-path re-sync
// against ground truth, and the log auto-dismiss timer.

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

const mockedInvoke = vi.mocked(invoke);

const ONE_DEVICE = [{ name: "dev1", category: "phone", diskUsageMb: 100, ramMb: 2048 }];

function mockBaseCommands(overrides: Record<string, unknown> = {}) {
  const defaults: Record<string, unknown> = {
    sdk_status: true,
    check_network: { online: true, detail: "" },
    check_java: { available: true, detail: "openjdk 21" },
    list_avds: ONE_DEVICE,
    list_available_images: [],
    list_device_profiles: [],
    check_hardware_accel: { available: true, backend: "WHPX", detail: "ok" },
    preferred_abi: "x86_64",
    list_running_avds: ["dev1"],
    check_disk_space: { availableMb: 20000 },
    data_paths: { dataRoot: "C:\\data", avdRoot: "C:\\avd" },
    ...overrides,
  };
  mockedInvoke.mockImplementation(async (cmd: string) => {
    if (cmd in defaults) return defaults[cmd];
    return null;
  });
}

beforeEach(() => {
  mockedInvoke.mockReset();
  localStorage.clear();
});

describe("App — dashboard reaches a stable render", () => {
  it("shows the device from list_avds as Running once mounted", async () => {
    mockBaseCommands();
    render(<App />);
    expect(await screen.findByText("dev1")).toBeInTheDocument();
    expect(await screen.findByText(/Running/)).toBeInTheDocument();
  });
});

describe("App — doStop reconciles against ground truth on failure", () => {
  it("flips a device back to Stopped when stop_avd fails but the device isn't actually running anymore", async () => {
    mockBaseCommands();
    render(<App />);
    const stopButton = await screen.findByRole("button", { name: "Stop" });

    // The device already crashed on its own — stop_avd fails because
    // find_serial_for_avd can't find it, and the very next
    // list_running_avds call (the reconciliation query in doStop's catch
    // block) correctly reports it's gone.
    mockedInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
      const name = (args as { name?: string } | undefined)?.name;
      if (cmd === "stop_avd") throw new Error(`Couldn't find a running emulator for "${name}"`);
      if (cmd === "list_running_avds") return [];
      return null;
    });

    stopButton.click();

    // Without the reconciliation fix, this device would stay stuck showing
    // "Running" with a Stop button that fails the same way forever.
    await waitFor(() => expect(screen.getByRole("button", { name: "Launch" })).toBeInTheDocument());
    expect(screen.queryByRole("button", { name: "Stop" })).not.toBeInTheDocument();
  });
});

describe("App — log auto-dismiss", () => {
  it("clears a no-action-needed log message on its own after a few seconds", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mockBaseCommands();
    render(<App />);
    const stopButton = await screen.findByRole("button", { name: "Stop" });

    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "stop_avd") return "Stopped dev1";
      if (cmd === "list_running_avds") return [];
      return null;
    });
    stopButton.click();

    // doStop's success path doesn't call setLog, so trigger a message via
    // the Nuke confirmation path being cancelled isn't available headlessly
    // — simplest reliable no-action-needed message is the create-form's
    // client-side name validation, which never touches the network.
    const createButton = await screen.findByRole("button", { name: "+ Create" });
    createButton.click();
    await vi.waitFor(() =>
      expect(screen.getByText(/Enter a device name with at least one letter or number/)).toBeInTheDocument()
    );

    vi.advanceTimersByTime(4200);
    await vi.waitFor(() =>
      expect(
        screen.queryByText(/Enter a device name with at least one letter or number/)
      ).not.toBeInTheDocument()
    );
    vi.useRealTimers();
  });
});
