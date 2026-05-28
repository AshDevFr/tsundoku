import type { BrowserContext, Page } from "playwright";
import { captureScreenshot } from "../utils/screenshot.js";
import { waitForPageReady } from "../utils/wait.js";

export async function run(page: Page, _context: BrowserContext): Promise<void> {
  console.log("  🗺  Capturing ID maps page");

  await page.goto("/admin/id-maps");
  await waitForPageReady(page);
  await captureScreenshot(page, "admin/id-maps", { fullPage: true });
}
