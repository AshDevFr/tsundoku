import type { BrowserContext, Page } from "playwright";
import { listGenres } from "../utils/api.js";
import { captureScreenshot } from "../utils/screenshot.js";
import { waitForImages, waitForPageReady } from "../utils/wait.js";

/// Browse page (the `/` feed). Captures the default card grid, the list
/// view variant, the full-width "wide" layout, and the filter sidebar.
export async function run(page: Page, context: BrowserContext): Promise<void> {
  console.log("  🏠 Capturing browse page");

  await page.goto("/");
  await waitForPageReady(page);
  await waitForImages(page).catch(() => {
    console.log("    (some covers may not have loaded)");
  });
  await captureScreenshot(page, "browse/feed-cards");

  // List variant via the SegmentedControl. Falls back to a no-op if the
  // toggle has been removed.
  const listToggle = await page.$('[data-testid="feed-view-toggle"] label:has-text("List")');
  if (listToggle) {
    await listToggle.click();
    await page.waitForTimeout(500);
    await waitForPageReady(page);
    await captureScreenshot(page, "browse/feed-list");
    // Toggle back so subsequent scenarios start from the default view.
    const cardToggle = await page.$('[data-testid="feed-view-toggle"] label:has-text("Cards")');
    if (cardToggle) {
      await cardToggle.click();
      await page.waitForTimeout(300);
    }
  } else {
    console.log("    ⚠️  Feed view toggle not found, skipping list-variant capture");
  }

  // Wide layout: drops the centered max-width container for a fixed-width
  // sidebar + fluid card grid that packs more columns. The toggle only
  // renders at ≥lg viewports, so this no-ops on a narrow capture viewport.
  // Persisted in localStorage, so we toggle it back off before moving on.
  const wideToggle = await page.$('[data-testid="feed-wide-toggle"]');
  if (wideToggle) {
    await wideToggle.click();
    await page.waitForTimeout(500);
    await waitForPageReady(page);
    await waitForImages(page).catch(() => {});
    await captureScreenshot(page, "browse/feed-wide");
    // Toggle back so the remaining captures use the default centered width.
    await wideToggle.click();
    await page.waitForTimeout(300);
  } else {
    console.log("    ⚠️  Wide toggle not found (narrow viewport?), skipping wide capture");
  }

  // Filter sidebar in an *active* state. Apply a real genre filter so the
  // shot shows a selected chip + narrowed grid + the enabled "Clear filters"
  // button — otherwise it's indistinguishable from the default feed-cards
  // shot (the sidebar is always rendered). Genre filters are URL state, so we
  // navigate straight to the filtered view rather than driving the combobox
  // (whose hidden form input makes click targeting brittle). Pick the most
  // common genre so the resulting grid stays full.
  const genres = await listGenres(context.request);
  const topGenre = genres.sort((a, b) => b.seriesCount - a.seriesCount)[0]?.name;
  if (topGenre) {
    await page.goto(`/?genres=${encodeURIComponent(topGenre)}`);
    console.log(`    Applied genre filter: ${topGenre}`);
  } else {
    console.log("    ⚠️  No genres available; capturing default panel");
    await page.goto("/");
  }
  await waitForPageReady(page);
  await waitForImages(page).catch(() => {});
  await captureScreenshot(page, "browse/feed-with-filters");
}
