import type { BrowserContext, Page } from "playwright";
import { captureScreenshot } from "../utils/screenshot.js";
import { waitForImages, waitForPageReady } from "../utils/wait.js";

/// Click into the first series card on the browse page and capture the
/// detail view. If no series exist (poll didn't surface anything) this
/// degrades to a console warning.
export async function run(page: Page, _context: BrowserContext): Promise<void> {
  console.log("  📚 Capturing series detail");

  await page.goto("/");
  await waitForPageReady(page);
  await waitForImages(page).catch(() => {});

  const firstCard = await page.$('[data-testid^="series-card-"]');
  if (!firstCard) {
    console.log("    ⚠️  No series cards on the feed; skipping series-detail capture");
    return;
  }

  await firstCard.click();
  await page.waitForURL(/\/series\/[^/]+$/, { timeout: 10000 }).catch(() => {});
  await waitForPageReady(page);
  await waitForImages(page).catch(() => {});

  await captureScreenshot(page, "series/detail", { fullPage: true });
}
