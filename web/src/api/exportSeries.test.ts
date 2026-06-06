import { describe, expect, it } from "vitest";
import { buildExportUrl } from "./exportSeries";

describe("buildExportUrl", () => {
  it("sets the format and joins selected fields", () => {
    const url = buildExportUrl({
      format: "csv",
      fields: ["canonicalTitle", "year"],
      includeReleases: false,
      filters: {},
    });
    const sp = new URL(url, "http://x").searchParams;
    expect(sp.get("format")).toBe("csv");
    expect(sp.get("fields")).toBe("canonicalTitle,year");
  });

  it("omits empty fields, includeReleases, and absent filters", () => {
    const url = buildExportUrl({
      format: "json",
      fields: [],
      includeReleases: false,
      filters: {},
    });
    const sp = new URL(url, "http://x").searchParams;
    expect(sp.get("format")).toBe("json");
    expect(sp.get("fields")).toBeNull();
    expect(sp.get("includeReleases")).toBeNull();
    expect(sp.get("kind")).toBeNull();
    expect(sp.get("hasReleases")).toBeNull();
  });

  it("serializes filters and includeReleases", () => {
    const url = buildExportUrl({
      format: "markdown",
      fields: ["canonicalTitle"],
      includeReleases: true,
      filters: {
        kind: "manga",
        status: "ongoing",
        metadataSource: "auto",
        hasReleases: false,
        codexStatus: ["missing", "behind"],
        genres: ["Action", "Drama"],
        tags: ["isekai"],
      },
    });
    const sp = new URL(url, "http://x").searchParams;
    expect(sp.get("includeReleases")).toBe("true");
    expect(sp.get("kind")).toBe("manga");
    expect(sp.get("status")).toBe("ongoing");
    expect(sp.get("metadataSource")).toBe("auto");
    // `false` must be sent explicitly (it's a meaningful "orphans only" filter),
    // distinct from an absent filter.
    expect(sp.get("hasReleases")).toBe("false");
    expect(sp.get("codexStatus")).toBe("missing,behind");
    expect(sp.get("genres")).toBe("Action,Drama");
    expect(sp.get("tags")).toBe("isekai");
  });

  it("sends hasReleases=true when the orphan filter is positive", () => {
    const sp = new URL(
      buildExportUrl({
        format: "json",
        fields: [],
        includeReleases: false,
        filters: { hasReleases: true },
      }),
      "http://x",
    ).searchParams;
    expect(sp.get("hasReleases")).toBe("true");
  });
});
