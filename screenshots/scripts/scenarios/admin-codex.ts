import type { BrowserContext, Page } from "playwright";
import { captureScreenshot } from "../utils/screenshot.js";
import { waitForElement, waitForPageReady } from "../utils/wait.js";

export async function run(page: Page, _context: BrowserContext): Promise<void> {
  console.log("  🔗 Capturing admin Codex integration");

  await page.goto("/admin/codex");
  await waitForPageReady(page);
  // The status card always renders once the query resolves — whether the
  // integration is enabled (status body) or disabled (alert inside the card).
  await waitForElement(page, '[data-testid="codex-card"]');
  await captureScreenshot(page, "admin/codex", { fullPage: true });
}
