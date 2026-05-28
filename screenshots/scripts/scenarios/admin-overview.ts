import type { BrowserContext, Page } from "playwright";
import { captureScreenshot } from "../utils/screenshot.js";
import { waitForPageReady } from "../utils/wait.js";

export async function run(page: Page, _context: BrowserContext): Promise<void> {
  console.log("  🛠  Capturing admin overview");

  await page.goto("/admin");
  await waitForPageReady(page);
  await captureScreenshot(page, "admin/overview", { fullPage: true });
}
