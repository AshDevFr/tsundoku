import type { BrowserContext, Page } from "playwright";
import { captureScreenshot } from "../utils/screenshot.js";
import { waitForElement, waitForPageReady } from "../utils/wait.js";

export async function run(page: Page, _context: BrowserContext): Promise<void> {
  console.log("  ⬇️  Capturing admin catalog export");

  await page.goto("/admin/export");
  await waitForPageReady(page);
  // The fields card always renders once the page mounts (the genre/tag
  // queries only populate filter options, not this card).
  await waitForElement(page, '[data-testid="export-fields-card"]');
  await captureScreenshot(page, "admin/export", { fullPage: true });
}
