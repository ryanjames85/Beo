import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

// Without this, each render() in a component test file stays mounted into
// the same jsdom document for every subsequent test in that file — later
// tests' queries (getByRole, etc.) then match multiple stale copies of the
// UI and fail with a "found multiple elements" error that has nothing to
// do with the thing actually being tested.
afterEach(() => {
  cleanup();
});
