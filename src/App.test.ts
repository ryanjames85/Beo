import { describe, it, expect } from "vitest";
import { sanitizeAvdName, containsBlockedWord, isNetworkError, recommendedImage } from "./App";

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
