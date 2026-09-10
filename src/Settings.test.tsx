import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import Settings, { isNewerVersion, type UpdateCheck } from "./Settings";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const mockedInvoke = vi.mocked(invoke);

function baseProps(overrides: Partial<React.ComponentProps<typeof Settings>> = {}) {
  return {
    onClose: vi.fn(),
    mode: "developer" as const,
    devMode: false,
    onResetApp: vi.fn(),
    githubUrl: "https://github.com/ryanjames85/Beo",
    version: "0.2.0",
    versionError: false,
    updateCheck: { status: "idle" } as UpdateCheck,
    copyLinkHint: null,
    checkForUpdates: vi.fn(),
    openReleasePage: vi.fn(),
    ...overrides,
  };
}

beforeEach(() => {
  mockedInvoke.mockReset();
  mockedInvoke.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "ide_integration_status":
        return { enabled: false, sdkPath: "C:\\Users\\ryan\\AppData\\Roaming\\beo\\sdk", shellProfile: null };
      case "data_paths":
        return { dataRoot: "C:\\Users\\ryan\\AppData\\Roaming\\beo", avdRoot: "C:\\Users\\ryan\\.android\\avd" };
      case "detect_connected_ides":
        return [];
      default:
        return null;
    }
  });
  Object.assign(navigator, { clipboard: { writeText: vi.fn() } });
});

describe("isNewerVersion", () => {
  it("recognizes a genuinely newer version", () => {
    expect(isNewerVersion("0.2.0", "0.1.0")).toBe(true);
    expect(isNewerVersion("1.0.0", "0.9.9")).toBe(true);
  });

  it("compares numerically, not as strings ('1.2.10' > '1.2.9')", () => {
    expect(isNewerVersion("1.2.10", "1.2.9")).toBe(true);
    expect(isNewerVersion("1.2.9", "1.2.10")).toBe(false);
  });

  it("returns false for the same version", () => {
    expect(isNewerVersion("0.1.0", "0.1.0")).toBe(false);
  });

  it("returns false when the candidate is older", () => {
    expect(isNewerVersion("0.1.0", "0.2.0")).toBe(false);
  });

  it("handles version strings with a different number of parts", () => {
    expect(isNewerVersion("0.2", "0.1.9")).toBe(true);
    expect(isNewerVersion("0.1.0.1", "0.1.0")).toBe(true);
  });
});

describe("Settings — Data & storage", () => {
  it("renders both real on-disk paths once loaded", async () => {
    render(<Settings {...baseProps()} />);
    expect(await screen.findByText("C:\\Users\\ryan\\AppData\\Roaming\\beo")).toBeInTheDocument();
    expect(await screen.findByText("C:\\Users\\ryan\\.android\\avd")).toBeInTheDocument();
  });

  it("shows placeholders before the paths have loaded", () => {
    mockedInvoke.mockImplementation(() => new Promise(() => {})); // never resolves
    render(<Settings {...baseProps()} />);
    const placeholders = screen.getAllByText("…");
    expect(placeholders.length).toBeGreaterThan(0);
  });
});

describe("Settings — Copy button feedback", () => {
  it("flips the app-data Copy button to 'Copied!' after clicking, then reverts", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    render(<Settings {...baseProps()} />);
    const button = await screen.findByTitle("Copy the full app data path to your clipboard");
    expect(button).toHaveTextContent("Copy");

    button.click();
    await vi.waitFor(() => expect(button).toHaveTextContent("Copied!"));
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith("C:\\Users\\ryan\\AppData\\Roaming\\beo");

    vi.advanceTimersByTime(1600);
    await vi.waitFor(() => expect(button).toHaveTextContent("Copy"));
    vi.useRealTimers();
  });
});

describe("Settings — IDE integration message placement", () => {
  it("renders the enable/disable result message inside the 'Use with another IDE' card, not detached elsewhere", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "ide_integration_status") {
        return { enabled: false, sdkPath: "C:\\sdk", shellProfile: null };
      }
      if (cmd === "enable_ide_integration") {
        return "ANDROID_HOME set. Restart any open IDE or terminal for it to take effect.";
      }
      if (cmd === "detect_connected_ides") return [];
      return null;
    });
    render(<Settings {...baseProps()} />);
    const toggleButton = await screen.findByRole("button", { name: "Enable IDE integration" });
    toggleButton.click();

    const message = await screen.findByText(/Restart any open IDE or terminal/);
    // The card is the nearest ancestor with the SDK-path label and the
    // toggle button both inside it — confirms the message lives in the
    // same visual card as the button that produced it, not orphaned in a
    // detached <p> below the whole section (the bug this test guards).
    const card = toggleButton.closest(".feature-card");
    expect(card).not.toBeNull();
    expect(card).toContainElement(message);
  });
});

describe("Settings — About / update check", () => {
  it("shows an 'up to date' message when updateCheck is up-to-date", () => {
    render(<Settings {...baseProps({ updateCheck: { status: "up-to-date" } })} />);
    expect(screen.getByText(/You're up to date/)).toBeInTheDocument();
  });

  it("shows the update-available callout with an Open release page button", () => {
    render(
      <Settings
        {...baseProps({
          updateCheck: { status: "available", version: "9.9.9", url: "https://example.com/release" },
        })}
      />
    );
    expect(screen.getByText("Update available: v9.9.9")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open release page" })).toBeInTheDocument();
  });

  it("shows a Retry button on an update-check error", () => {
    render(<Settings {...baseProps({ updateCheck: { status: "error", message: "Network down" } })} />);
    expect(screen.getByText("Network down")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });

  it("disables the Check for updates button while checking or when version is unknown", () => {
    const { rerender } = render(<Settings {...baseProps({ updateCheck: { status: "checking" } })} />);
    expect(screen.getByRole("button", { name: "Checking…" })).toBeDisabled();

    rerender(<Settings {...baseProps({ version: null })} />);
    expect(screen.getByRole("button", { name: "Check for updates" })).toBeDisabled();
  });
});

describe("Settings — Debug section visibility", () => {
  it("hides the Debug section when devMode is off", () => {
    render(<Settings {...baseProps({ devMode: false })} />);
    expect(screen.queryByText("Reset app to welcome screen")).not.toBeInTheDocument();
  });

  it("shows the Debug section when devMode is on", () => {
    render(<Settings {...baseProps({ devMode: true })} />);
    expect(screen.getByText("Reset app to welcome screen")).toBeInTheDocument();
  });
});
