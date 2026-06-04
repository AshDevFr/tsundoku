import type { BrowserContext, Page } from "playwright";
import { captureScreenshot } from "../utils/screenshot.js";
import { waitForElement, waitForPageReady } from "../utils/wait.js";

/// Stand-in for the operator's real client URL in the committed docs
/// screenshot. The screenshots backend reads the `[download]` block from the
/// local-only `tsundoku.screenshots.local.toml` overlay, so the captured frame
/// would otherwise ship the operator's actual seedbox host.
const PLACEHOLDER_BASE_URL = "https://seedbox.example.com/rutorrent";

export async function run(page: Page, _context: BrowserContext): Promise<void> {
  console.log("  📤 Capturing admin download client");

  await page.goto("/admin/download");
  await waitForPageReady(page);
  // The page renders one of two terminal states once status loads: the
  // connection card (enabled) or the disabled alert. Wait for either so we
  // don't catch the loading spinner.
  await waitForElement(
    page,
    '[data-testid="download-card"], [data-testid="download-disabled"]',
  );

  // When the client is configured, the card prints the real `base_url`. Redact
  // it before capturing so the committed image never ships a real host. Fail
  // closed: if the card is up but we can't find the row, abort rather than
  // capture an unmasked frame.
  if (await page.$('[data-testid="download-card"]')) {
    const redacted = await page.evaluate((placeholder) => {
      const card = document.querySelector('[data-testid="download-card"]');
      if (!card) return false;
      // ConfigRow renders a label <Text> ("base_url") followed by its value
      // <Text>; match the leaf label cell and overwrite its sibling.
      const label = Array.from(card.querySelectorAll("*")).find(
        (el) => el.childElementCount === 0 && el.textContent?.trim() === "base_url",
      );
      const value = label?.nextElementSibling;
      if (!value) return false;
      value.textContent = placeholder;
      return true;
    }, PLACEHOLDER_BASE_URL);

    if (!redacted) {
      throw new Error(
        "download base_url row not found — refusing to capture a frame that may leak the real client URL",
      );
    }
  }

  await captureScreenshot(page, "admin/download", { fullPage: true });
}
