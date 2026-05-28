import type { BrowserContext, Page } from "playwright";
import { captureScreenshot } from "../utils/screenshot.js";
import { waitForElement, waitForPageReady } from "../utils/wait.js";

export async function run(page: Page, _context: BrowserContext): Promise<void> {
  console.log("  🧰 Capturing admin maintenance");

  await page.goto("/admin/maintenance");
  await waitForPageReady(page);
  // Wait for the last card so we don't catch a half-rendered list.
  await waitForElement(
    page,
    '[data-testid="maintenance-invalidate-covers-card"]',
  );
  await captureScreenshot(page, "admin/maintenance", { fullPage: true });
}
