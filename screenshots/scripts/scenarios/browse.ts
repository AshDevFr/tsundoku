import type { BrowserContext, Page } from "playwright";
import { captureScreenshot } from "../utils/screenshot.js";
import { waitForImages, waitForPageReady } from "../utils/wait.js";

/// Browse page (the `/` feed). Captures the default card grid, the list
/// view variant, and one filter-applied state if filters are wired up.
export async function run(page: Page, _context: BrowserContext): Promise<void> {
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

  // Filter sidebar with the genre/tag panel visible. We don't apply
  // a specific filter — the panel itself is the interesting surface.
  await page.goto("/");
  await waitForPageReady(page);
  await captureScreenshot(page, "browse/feed-with-filters");
}
