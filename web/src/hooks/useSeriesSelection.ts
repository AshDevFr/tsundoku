import { useRef, useState } from "react";

/// Page-local selection over the series ids visible on the current page.
///
/// Encapsulates the Review page's anchor pattern: a plain toggle flips one id
/// and re-anchors the range; a shift+toggle selects the whole run between the
/// anchor and the clicked index (the anchor stays put, so successive
/// shift+clicks re-extend from the same origin).
export function useSeriesSelection(pageIds: number[]) {
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const anchor = useRef<number | null>(null);

  // Both the selection and the anchor index into the *current* ordering, so
  // any change to the visible id set (filters, sort, page, page size) drops
  // them — a stale anchor must not range into a reshuffled list. Adjusted
  // during render (not in an effect) so a stale selection never paints.
  const idsKey = pageIds.join(",");
  const [prevIdsKey, setPrevIdsKey] = useState(idsKey);
  if (idsKey !== prevIdsKey) {
    setPrevIdsKey(idsKey);
    setSelected(new Set());
    anchor.current = null;
  }

  const toggleAt = (index: number, shiftKey: boolean) => {
    const id = pageIds[index];
    if (id === undefined) return;
    if (shiftKey && anchor.current !== null) {
      const start = Math.min(anchor.current, index);
      const end = Math.max(anchor.current, index);
      const range = pageIds.slice(start, end + 1);
      setSelected((prev) => {
        const next = new Set(prev);
        for (const rid of range) next.add(rid);
        return next;
      });
      return;
    }
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
    anchor.current = index;
  };

  const allPageSelected =
    pageIds.length > 0 && pageIds.every((id) => selected.has(id));
  const somePageSelected = pageIds.some((id) => selected.has(id));

  const toggleAllOnPage = () => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (allPageSelected) {
        for (const id of pageIds) next.delete(id);
      } else {
        for (const id of pageIds) next.add(id);
      }
      return next;
    });
  };

  const clear = () => {
    setSelected(new Set());
    anchor.current = null;
  };

  return {
    selected,
    toggleAt,
    toggleAllOnPage,
    clear,
    allPageSelected,
    somePageSelected,
  };
}
