import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useSeriesSelection } from "./useSeriesSelection";

const ids = [10, 20, 30, 40, 50];

describe("useSeriesSelection", () => {
  it("toggles a single id on and off", () => {
    const { result } = renderHook(() => useSeriesSelection(ids));
    act(() => result.current.toggleAt(1, false));
    expect([...result.current.selected]).toEqual([20]);
    act(() => result.current.toggleAt(1, false));
    expect(result.current.selected.size).toBe(0);
  });

  it("shift+toggle selects the range from the anchor, which stays put", () => {
    const { result } = renderHook(() => useSeriesSelection(ids));
    act(() => result.current.toggleAt(1, false));
    act(() => result.current.toggleAt(3, true));
    expect([...result.current.selected].sort((a, b) => a - b)).toEqual([
      20, 30, 40,
    ]);
    // Successive shift+clicks re-extend from the same origin (index 1).
    act(() => result.current.toggleAt(0, true));
    expect([...result.current.selected].sort((a, b) => a - b)).toEqual([
      10, 20, 30, 40,
    ]);
  });

  it("shift+toggle with no anchor behaves like a plain toggle", () => {
    const { result } = renderHook(() => useSeriesSelection(ids));
    act(() => result.current.toggleAt(2, true));
    expect([...result.current.selected]).toEqual([30]);
  });

  it("select-all-on-page toggles between the full page and nothing", () => {
    const { result } = renderHook(() => useSeriesSelection(ids));
    expect(result.current.allPageSelected).toBe(false);
    act(() => result.current.toggleAllOnPage());
    expect([...result.current.selected].sort((a, b) => a - b)).toEqual(ids);
    expect(result.current.allPageSelected).toBe(true);
    act(() => result.current.toggleAllOnPage());
    expect(result.current.selected.size).toBe(0);
  });

  it("reports a partial page selection", () => {
    const { result } = renderHook(() => useSeriesSelection(ids));
    act(() => result.current.toggleAt(0, false));
    expect(result.current.somePageSelected).toBe(true);
    expect(result.current.allPageSelected).toBe(false);
  });

  it("drops the selection when the visible id set changes", () => {
    const { result, rerender } = renderHook(
      ({ pageIds }: { pageIds: number[] }) => useSeriesSelection(pageIds),
      { initialProps: { pageIds: ids } },
    );
    act(() => result.current.toggleAt(1, false));
    expect(result.current.selected.size).toBe(1);
    // New page / new ordering: the anchor would index into a reshuffled
    // list, so both selection and anchor are dropped.
    rerender({ pageIds: [60, 70, 80] });
    expect(result.current.selected.size).toBe(0);
    act(() => result.current.toggleAt(1, true));
    expect([...result.current.selected]).toEqual([70]);
  });

  it("clear empties the selection and drops the anchor", () => {
    const { result } = renderHook(() => useSeriesSelection(ids));
    act(() => result.current.toggleAt(1, false));
    act(() => result.current.clear());
    expect(result.current.selected.size).toBe(0);
    // A shift+toggle after clear must not range from the stale anchor.
    act(() => result.current.toggleAt(3, true));
    expect([...result.current.selected]).toEqual([40]);
  });
});
