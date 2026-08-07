import { afterEach, describe, expect, it } from "vitest";
import { type FilterPreset, sortPresets, useFilterPresets } from "./filters";

function preset(name: string, id = name): FilterPreset {
  return { id, name, search: {} };
}

afterEach(() => {
  useFilterPresets.setState({ presets: [], activePresetId: undefined });
  localStorage.clear();
});

describe("sortPresets", () => {
  it("orders by name, case-insensitively", () => {
    const out = sortPresets([
      preset("magic"),
      preset("Isekai"),
      preset("Dungeons"),
      preset("Default Search"),
    ]);
    expect(out.map((p) => p.name)).toEqual([
      "Default Search",
      "Dungeons",
      "Isekai",
      "magic",
    ]);
  });

  it("orders embedded numbers numerically, not lexically", () => {
    const out = sortPresets([preset("Isekai 10"), preset("Isekai 2")]);
    expect(out.map((p) => p.name)).toEqual(["Isekai 2", "Isekai 10"]);
  });

  it("is stable for names differing only by case", () => {
    const out = sortPresets([
      preset("isekai", "second"),
      preset("Isekai", "first"),
    ]);
    expect(out.map((p) => p.id)).toEqual(["second", "first"]);
  });

  it("does not mutate its input", () => {
    const input = [preset("Magic"), preset("Dungeons")];
    sortPresets(input);
    expect(input.map((p) => p.name)).toEqual(["Magic", "Dungeons"]);
  });
});

describe("active preset marker", () => {
  it("records and clears the loaded preset", () => {
    useFilterPresets.getState().setActivePreset("abc");
    expect(useFilterPresets.getState().activePresetId).toBe("abc");

    useFilterPresets.getState().setActivePreset(undefined);
    expect(useFilterPresets.getState().activePresetId).toBeUndefined();
  });

  it("clears the marker when the loaded preset is deleted", () => {
    const saved = useFilterPresets.getState().savePreset("Isekai", {});
    useFilterPresets.getState().setActivePreset(saved.id);

    useFilterPresets.getState().deletePreset(saved.id);

    expect(useFilterPresets.getState().presets).toHaveLength(0);
    expect(useFilterPresets.getState().activePresetId).toBeUndefined();
  });

  it("leaves the marker alone when a different preset is deleted", () => {
    const keep = useFilterPresets.getState().savePreset("Isekai", {});
    const other = useFilterPresets.getState().savePreset("Magic", {});
    useFilterPresets.getState().setActivePreset(keep.id);

    useFilterPresets.getState().deletePreset(other.id);

    expect(useFilterPresets.getState().activePresetId).toBe(keep.id);
  });

  it("keeps the marker out of persisted storage", () => {
    const saved = useFilterPresets.getState().savePreset("Isekai", {});
    useFilterPresets.getState().setActivePreset(saved.id);

    const raw = localStorage.getItem("tsundoku.filter-presets.v1");
    expect(raw).toBeTruthy();
    expect(Object.keys(JSON.parse(raw as string).state)).toEqual(["presets"]);
  });
});

describe("updatePreset", () => {
  it("replaces the search in place, keeping id, name and position", () => {
    const first = useFilterPresets
      .getState()
      .savePreset("Isekai", { kind: ["manga"] });
    useFilterPresets.getState().savePreset("Magic", {});

    const updated = useFilterPresets
      .getState()
      .updatePreset(first.id, { kind: ["manhwa"] });

    expect(updated).toEqual({
      id: first.id,
      name: "Isekai",
      search: { kind: ["manhwa"] },
    });
    const { presets } = useFilterPresets.getState();
    expect(presets).toHaveLength(2);
    expect(presets[0]).toEqual(updated);
    expect(presets[1].name).toBe("Magic");
  });

  it("writes to the targeted id rather than matching on name", () => {
    const saved = useFilterPresets.getState().savePreset("Isekai", {});
    useFilterPresets.getState().savePreset("Magic", { kind: ["manga"] });

    useFilterPresets.getState().updatePreset(saved.id, { kind: ["manhwa"] });

    const { presets } = useFilterPresets.getState();
    expect(presets).toHaveLength(2);
    expect(presets[0].search).toEqual({ kind: ["manhwa"] });
    expect(presets[1].search).toEqual({ kind: ["manga"] });
  });

  it("returns undefined for an unknown id and changes nothing", () => {
    useFilterPresets.getState().savePreset("Isekai", {});
    const before = useFilterPresets.getState().presets;

    expect(
      useFilterPresets.getState().updatePreset("nope", {}),
    ).toBeUndefined();
    expect(useFilterPresets.getState().presets).toEqual(before);
  });
});
