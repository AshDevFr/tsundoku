import { Page } from "playwright";

const DEFAULT_TIMEOUT = 30000;

/// Wait for the page to settle before taking a screenshot. Network idle
/// is best-effort: TanStack Query keeps polling in the background, so we
/// don't fail the run if it never goes idle.
export async function waitForPageReady(
  page: Page,
  _timeout: number = DEFAULT_TIMEOUT,
): Promise<void> {
  try {
    await page.waitForLoadState("domcontentloaded", { timeout: 10000 });
  } catch {
    console.log("    (DOM load timeout, continuing...)");
  }

  try {
    await page.waitForLoadState("networkidle", { timeout: 5000 });
  } catch {
    // ignore — polling queries can keep this busy forever
  }

  await waitForNoLoadingIndicators(page, 5000);

  // Small delay for any final renders / chart animations.
  await page.waitForTimeout(500);
}

export async function waitForNoLoadingIndicators(
  page: Page,
  timeout: number = DEFAULT_TIMEOUT,
): Promise<void> {
  const loadingSelectors = [
    '[data-loading="true"]',
    ".mantine-Loader-root",
    ".mantine-LoadingOverlay-root",
    '[aria-busy="true"]',
    ".loading",
    ".spinner",
    ".mantine-Skeleton-root:not([data-visible='false'])",
  ];

  const startTime = Date.now();

  while (Date.now() - startTime < timeout) {
    let hasLoadingIndicator = false;

    for (const selector of loadingSelectors) {
      const element = await page.$(selector);
      if (element) {
        const isVisible = await element.isVisible();
        if (isVisible) {
          hasLoadingIndicator = true;
          break;
        }
      }
    }

    if (!hasLoadingIndicator) {
      return;
    }

    await page.waitForTimeout(100);
  }
}

export async function waitForElement(
  page: Page,
  selector: string,
  timeout: number = DEFAULT_TIMEOUT,
): Promise<void> {
  await page.waitForSelector(selector, {
    state: "visible",
    timeout,
  });
}

export async function waitForImages(
  page: Page,
  timeout: number = DEFAULT_TIMEOUT,
): Promise<void> {
  await page.waitForFunction(
    () => {
      const images = document.querySelectorAll("img");
      return Array.from(images).every(
        (img) => img.complete && img.naturalHeight > 0,
      );
    },
    { timeout },
  );
}

export async function waitForUrl(
  page: Page,
  pattern: string | RegExp,
  timeout: number = DEFAULT_TIMEOUT,
): Promise<void> {
  await page.waitForURL(pattern, { timeout });
}

/// Dismiss Mantine notifications before screenshotting so the toast
/// container doesn't leak into otherwise-static frames.
export async function waitForToastsToDisappear(
  page: Page,
  timeout: number = 10000,
): Promise<void> {
  const toastSelectors = [
    ".mantine-Notifications-root .mantine-Notification-root",
    '[data-mantine-notification="true"]',
    ".mantine-Notification-root",
  ];

  const startTime = Date.now();
  await page.waitForTimeout(300);

  while (Date.now() - startTime < timeout) {
    let hasVisibleToast = false;

    for (const selector of toastSelectors) {
      const elements = await page.$$(selector);
      for (const element of elements) {
        const isVisible = await element.isVisible();
        if (isVisible) {
          hasVisibleToast = true;
          break;
        }
      }
      if (hasVisibleToast) break;
    }

    if (!hasVisibleToast) {
      return;
    }

    await page.waitForTimeout(200);
  }
}
