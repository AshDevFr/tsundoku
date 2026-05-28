import type { BrowserContext, Page } from "playwright";
import { captureScreenshot } from "../utils/screenshot.js";
import { waitForPageReady } from "../utils/wait.js";

export async function run(page: Page, _context: BrowserContext): Promise<void> {
  console.log("  🧠 Capturing providers list");

  await page.goto("/admin/providers");
  await waitForPageReady(page);
  await captureScreenshot(page, "admin/providers-list", { fullPage: true });

  const detailLink = await page.$('a[href^="/admin/providers/"]:not([href$="/providers"])');
  if (!detailLink) {
    console.log("    ⚠️  No provider rows found; skipping provider-detail capture");
    return;
  }
  await detailLink.click();
  await page.waitForURL(/\/admin\/providers\/[^/]+$/, { timeout: 10000 }).catch(() => {});
  await waitForPageReady(page);
  await captureScreenshot(page, "admin/provider-detail", { fullPage: true });
}
