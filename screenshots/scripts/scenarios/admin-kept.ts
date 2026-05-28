import type { BrowserContext, Page } from "playwright";
import { captureScreenshot } from "../utils/screenshot.js";
import { waitForPageReady } from "../utils/wait.js";

export async function run(page: Page, _context: BrowserContext): Promise<void> {
  console.log("  📌 Capturing kept page");

  await page.goto("/admin/kept");
  await waitForPageReady(page);
  await captureScreenshot(page, "admin/kept", { fullPage: true });
}
