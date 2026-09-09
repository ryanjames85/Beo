import { describe, it, expect } from "vitest";
import { isNewerVersion } from "./Settings";

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
