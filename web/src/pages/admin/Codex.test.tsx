import { describe, expect, it } from "vitest";
import { codexVersionOutdated } from "./Codex";

describe("codexVersionOutdated", () => {
  it("flags versions below 1.32.0", () => {
    expect(codexVersionOutdated("1.31.2")).toBe(true);
    expect(codexVersionOutdated("1.0.0")).toBe(true);
    expect(codexVersionOutdated("0.9.9")).toBe(true);
    expect(codexVersionOutdated("v1.31.99")).toBe(true);
  });

  it("does not flag 1.32.0 or newer", () => {
    expect(codexVersionOutdated("1.32.0")).toBe(false);
    expect(codexVersionOutdated("1.32.1")).toBe(false);
    expect(codexVersionOutdated("1.40.0")).toBe(false);
    expect(codexVersionOutdated("2.0.0")).toBe(false);
  });

  it("does not flag when the version is missing or unparseable", () => {
    expect(codexVersionOutdated(null)).toBe(false);
    expect(codexVersionOutdated(undefined)).toBe(false);
    expect(codexVersionOutdated("")).toBe(false);
    expect(codexVersionOutdated("nightly")).toBe(false);
  });
});
