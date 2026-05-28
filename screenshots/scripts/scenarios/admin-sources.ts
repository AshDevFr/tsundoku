import type { BrowserContext, Page } from "playwright";
import { captureScreenshot } from "../utils/screenshot.js";
import { waitForPageReady } from "../utils/wait.js";

/// Sources list and, when at least one source exists, the per-source
/// detail page.
export async function run(page: Page, _context: BrowserContext): Promise<void> {
  console.log("  🔌 Capturing sources list");

  await page.goto("/admin/sources");
  await waitForPageReady(page);
  await captureScreenshot(page, "admin/sources-list", { fullPage: true });

  // Drill into the first source card. The cards are anchors to
  // /admin/sources/<name>; we use the URL change to detect success.
  const detailLink = await page.$('a[href^="/admin/sources/"]:not([href$="/sources"])');
  if (!detailLink) {
    console.log("    ⚠️  No source rows found; skipping source-detail capture");
    return;
  }
  await detailLink.click();
  await page.waitForURL(/\/admin\/sources\/[^/]+$/, { timeout: 10000 }).catch(() => {});
  await waitForPageReady(page);
  await captureScreenshot(page, "admin/source-detail", { fullPage: true });
}
