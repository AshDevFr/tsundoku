import { useMediaQuery } from "@mantine/hooks";

/// True below Mantine's `sm` breakpoint (48em / 768px). Use this for the
/// coarse mobile/desktop branches that live in a component *prop* (e.g. a
/// Modal's `fullScreen`), where the CSS-only `hiddenFrom`/`visibleFrom` helpers
/// can't reach. Defaults to `false` before the media query resolves (SSR/first
/// paint and the jsdom test environment, where matchMedia reports no match).
export function useIsMobile(): boolean {
  return useMediaQuery("(max-width: 48em)") ?? false;
}
