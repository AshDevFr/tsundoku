import { Page } from "playwright";
import { mkdir } from "fs/promises";
import { existsSync } from "fs";
import path from "path";
import { config } from "../../playwright.config.js";
import { waitForToastsToDisappear } from "./wait.js";

export interface ScreenshotOptions {
  fullPage?: boolean;
  clip?: { x: number; y: number; width: number; height: number };
  timeout?: number;
}

const capturedScreenshots: string[] = [];

async function ensureDir(dirPath: string): Promise<void> {
  const resolvedPath = path.resolve(dirPath);
  if (!existsSync(resolvedPath)) {
    await mkdir(resolvedPath, { recursive: true });
  }
}

/// Capture a screenshot with consistent naming. `name` may include a
/// subdirectory (e.g. "admin/overview"); the matching dir is created on
/// demand. Toasts are dismissed first so notifications never leak into
/// the frame.
export async function captureScreenshot(
  page: Page,
  name: string,
  options: ScreenshotOptions = {},
): Promise<string> {
  await waitForToastsToDisappear(page);

  const filename = `${name}.png`;
  const filepath = path.join(config.outputDir, filename);

  const dir = path.dirname(filepath);
  await ensureDir(dir);

  await page.screenshot({
    path: filepath,
    fullPage: options.fullPage ?? false,
    clip: options.clip,
    timeout: options.timeout ?? 30000,
  });

  capturedScreenshots.push(filename);
  console.log(`  📸 Captured: ${filename}`);

  return filepath;
}

export function getCapturedScreenshots(): string[] {
  return [...capturedScreenshots];
}

export function printScreenshotSummary(): void {
  console.log("\n" + "=".repeat(50));
  console.log("Screenshot Summary");
  console.log("=".repeat(50));
  console.log(`Total: ${capturedScreenshots.length} screenshots captured\n`);

  for (const screenshot of capturedScreenshots) {
    console.log(`  ✓ ${screenshot}`);
  }

  console.log("\n" + "=".repeat(50));
  console.log(`Output directory: ${path.resolve(config.outputDir)}`);
  console.log("=".repeat(50) + "\n");
}
