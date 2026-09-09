import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import CreateDeviceForm from "./CreateDeviceForm";

function noop() {}

describe("CreateDeviceForm — Simple mode", () => {
  it("renders the Phone/Tablet toggle and a Create button, not the Developer picker", () => {
    render(
      <CreateDeviceForm
        mode="simple"
        category="phone"
        onCategoryChange={noop}
        newName=""
        onNewNameChange={noop}
        creating={false}
        onCreate={noop}
      />
    );
    expect(screen.getByRole("button", { name: "Phone" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Tablet" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "+ Create" })).toBeInTheDocument();
    expect(screen.queryByText(/Select system image/)).not.toBeInTheDocument();
  });

  it("shows 'Creating…' and disables the button while creating", () => {
    render(
      <CreateDeviceForm
        mode="simple"
        category="phone"
        onCategoryChange={noop}
        newName="my device"
        onNewNameChange={noop}
        creating={true}
        onCreate={noop}
      />
    );
    const button = screen.getByRole("button", { name: "Creating…" });
    expect(button).toBeDisabled();
  });

  it("shows the sanitized-name hint when the typed name needs adapting", () => {
    render(
      <CreateDeviceForm
        mode="simple"
        category="phone"
        onCategoryChange={noop}
        newName="My tablet!!"
        onNewNameChange={noop}
        creating={false}
        onCreate={noop}
      />
    );
    expect(screen.getByText(/Will be created as "My_tablet"/)).toBeInTheDocument();
  });

  it("shows the blocked-word hint instead when the sanitized name is blocked", () => {
    render(
      <CreateDeviceForm
        mode="simple"
        category="phone"
        onCategoryChange={noop}
        newName="shit device"
        onNewNameChange={noop}
        creating={false}
        onCreate={noop}
      />
    );
    expect(screen.getByText(/That name isn't allowed/)).toBeInTheDocument();
  });

  it("shows no hint when the name is already valid as typed", () => {
    render(
      <CreateDeviceForm
        mode="simple"
        category="phone"
        onCategoryChange={noop}
        newName="my_tablet"
        onNewNameChange={noop}
        creating={false}
        onCreate={noop}
      />
    );
    expect(screen.queryByText(/Will be created as/)).not.toBeInTheDocument();
    expect(screen.queryByText(/isn't allowed/)).not.toBeInTheDocument();
  });

  it("calls onCategoryChange when Tablet is clicked", () => {
    const onCategoryChange = vi.fn();
    render(
      <CreateDeviceForm
        mode="simple"
        category="phone"
        onCategoryChange={onCategoryChange}
        newName=""
        onNewNameChange={noop}
        creating={false}
        onCreate={noop}
      />
    );
    screen.getByRole("button", { name: "Tablet" }).click();
    expect(onCategoryChange).toHaveBeenCalledWith("tablet");
  });
});

describe("CreateDeviceForm — Developer mode", () => {
  const images = [
    "system-images;android-36;google_apis_playstore;x86_64",
    "system-images;android-36;google_apis_playstore;arm64-v8a",
  ];
  const profiles = [
    { id: "pixel_6", label: "Pixel 6", category: "phone" },
    { id: "pixel_tablet", label: "Pixel Tablet", category: "tablet" },
  ];

  it("renders the full image/device picker, not the Simple mode toggle", () => {
    render(
      <CreateDeviceForm
        mode="developer"
        newName=""
        onNewNameChange={noop}
        creating={false}
        onCreate={noop}
        playStore={true}
        onPlayStoreChange={noop}
        selectedImage=""
        onSelectedImageChange={noop}
        device="pixel_6"
        onDeviceChange={noop}
        images={images}
        profiles={profiles}
        abi="x86_64"
      />
    );
    expect(screen.getByText(/Select system image/)).toBeInTheDocument();
    expect(screen.getByText("Pixel 6")).toBeInTheDocument();
    expect(screen.getByText("Pixel Tablet")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Phone" })).not.toBeInTheDocument();
  });

  it("flags a non-host-ABI image in its label", () => {
    render(
      <CreateDeviceForm
        mode="developer"
        newName=""
        onNewNameChange={noop}
        creating={false}
        onCreate={noop}
        playStore={true}
        onPlayStoreChange={noop}
        selectedImage=""
        onSelectedImageChange={noop}
        device="pixel_6"
        onDeviceChange={noop}
        images={images}
        profiles={profiles}
        abi="x86_64"
      />
    );
    expect(screen.getByText(/arm64-v8a \(not x86_64/)).toBeInTheDocument();
  });
});
