import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import DeviceCard, { formatMb, type AvdInfo } from "./DeviceCard";

describe("formatMb", () => {
  it("shows MB below 1024", () => {
    expect(formatMb(512)).toBe("512 MB");
    expect(formatMb(1023)).toBe("1023 MB");
  });

  it("shows GB (one decimal place) at or above 1024", () => {
    expect(formatMb(1024)).toBe("1.0 GB");
    // Real value confirmed live on an actual device this session.
    expect(formatMb(11191)).toBe("10.9 GB");
  });
});

function baseAvd(overrides: Partial<AvdInfo> = {}): AvdInfo {
  return { name: "test_device", category: "phone", diskUsageMb: 1024, ramMb: 2048, ...overrides };
}

function noop() {}

const baseProps = {
  avd: baseAvd(),
  running: false,
  booted: false,
  orientation: undefined as "portrait" | "landscape" | undefined,
  installingApk: false,
  snapshotsOpen: false,
  snapshots: [],
  newSnapshotName: "",
  snapshotBusy: false,
  onRotate: noop,
  onInstallApk: noop,
  onToggleSnapshots: noop,
  onLaunch: noop,
  onStop: noop,
  onDelete: noop,
  onLoadSnapshot: noop,
  onDeleteSnapshot: noop,
  onSaveSnapshot: noop,
  onNewSnapshotNameChange: noop,
};

describe("DeviceCard status", () => {
  it("shows Stopped and a Launch button when not running", () => {
    render(<DeviceCard {...baseProps} />);
    expect(screen.getByText(/Stopped/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Launch" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Stop" })).not.toBeInTheDocument();
  });

  it("shows 'Starting…' (not 'Running') once launched but before boot completes", () => {
    render(<DeviceCard {...baseProps} running={true} booted={false} />);
    expect(screen.getByText(/Starting…/)).toBeInTheDocument();
    expect(screen.queryByText(/^Running/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Stop" })).toBeInTheDocument();
  });

  it("shows Running only once actually booted", () => {
    render(<DeviceCard {...baseProps} running={true} booted={true} />);
    expect(screen.getByText(/Running/)).toBeInTheDocument();
  });

  it("disables Rotate/Install APK/Snapshots while Starting, but not while Running", () => {
    const { rerender } = render(<DeviceCard {...baseProps} running={true} booted={false} />);
    expect(screen.getByRole("button", { name: /Rotate/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Install APK" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Snapshots" })).toBeDisabled();

    rerender(<DeviceCard {...baseProps} running={true} booted={true} />);
    expect(screen.getByRole("button", { name: /Rotate/ })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Install APK" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Snapshots" })).toBeEnabled();
  });

  it("shows real disk usage and RAM, or 'unknown' when unavailable", () => {
    const { rerender } = render(
      <DeviceCard {...baseProps} avd={baseAvd({ diskUsageMb: 6591, ramMb: 1536 })} />
    );
    expect(screen.getByText(/6\.4 GB/)).toBeInTheDocument();
    expect(screen.getByText(/1536 MB RAM/)).toBeInTheDocument();

    rerender(<DeviceCard {...baseProps} avd={baseAvd({ diskUsageMb: null, ramMb: null })} />);
    expect(screen.getByText(/size unknown/)).toBeInTheDocument();
    expect(screen.getByText(/RAM unknown/)).toBeInTheDocument();
  });

  it("calls onLaunch with the device name when Launch is clicked", () => {
    const onLaunch = vi.fn();
    render(<DeviceCard {...baseProps} avd={baseAvd({ name: "my_device" })} onLaunch={onLaunch} />);
    screen.getByRole("button", { name: "Launch" }).click();
    expect(onLaunch).toHaveBeenCalledWith("my_device");
  });
});
