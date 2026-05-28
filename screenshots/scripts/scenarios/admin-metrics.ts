import type { BrowserContext, Page } from "playwright";
import { captureScreenshot } from "../utils/screenshot.js";
import { waitForPageReady } from "../utils/wait.js";

export async function run(page: Page, _context: BrowserContext): Promise<void> {
  console.log("  📈 Capturing metrics dashboard");

  await page.goto("/admin/metrics");
  await waitForPageReady(page);
  // Give the chart libs a beat to finish their initial animation.
  await page.waitForTimeout(800);
  await captureScreenshot(page, "admin/metrics", { fullPage: true });
}
