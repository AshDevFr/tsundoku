import { chromium, type Browser, type BrowserContext, type Page } from "playwright";
import { config } from "../playwright.config.js";
import { seedAdminToken } from "./utils/auth.js";
import {
  getStats,
  listSources,
  triggerPolls,
  waitForPollSoak,
} from "./utils/api.js";
import { printScreenshotSummary } from "./utils/screenshot.js";

interface ScenarioModule {
  run: (page: Page, context: BrowserContext) => Promise<void>;
  name: string;
}

async function main(): Promise<void> {
  console.log("\n" + "=".repeat(50));
  console.log("tsundoku Screenshot Automation");
  console.log("=".repeat(50));
  console.log(`Base URL:   ${config.baseUrl}`);
  console.log(`Viewport:   ${config.viewport.width}x${config.viewport.height}`);
  console.log(`Scheme:     ${config.colorScheme}`);
  console.log(`Output:     ${config.outputDir}`);
  console.log(`Poll start: ${config.pollOnStart}`);
  console.log("=".repeat(50) + "\n");

  let browser: Browser | null = null;
  let context: BrowserContext | null = null;

  try {
    console.log("🚀 Launching browser...");
    browser = await chromium.launch({ headless: true });

    context = await browser.newContext({
      viewport: config.viewport,
      colorScheme: config.colorScheme,
      baseURL: config.baseUrl,
    });

    // Seed the admin token into localStorage so the AuthGate doesn't
    // block any /admin/* navigation we do later.
    await seedAdminToken(context);

    const page = await context.newPage();

    console.log("⏳ Waiting for backend to be ready...");
    await waitForBackend(page);
    console.log("✓ Backend is ready\n");

    // Kick the polls off before doing anything else so the DB has time
    // to populate (resolver + cover fetches + MangaBaka enrichment)
    // while we wait. Scenarios only start after the soak window
    // elapses.
    if (config.pollOnStart) {
      const sources = await resolvePollSources(context);
      if (sources.length === 0) {
        console.log("  ⚠️  No sources configured/enabled; skipping poll.");
      } else {
        await triggerPolls(context.request, sources);
        console.log(
          `  ⏱  Soaking for ${config.pollWaitMinSeconds}s (max ${config.pollWaitMaxSeconds}s)...`,
        );
        await waitForPollSoak(context.request);
      }
    }

    const stats = await getStats(context.request);
    if (stats) {
      console.log(
        `\n📊 DB now has ${stats.series} series / ${stats.totalReleases} releases ` +
          `(active provider: ${stats.activeProvider})\n`,
      );
    }

    const scenarios = await loadScenarios();

    if (scenarios.length === 0) {
      console.log("\n⚠️  No scenarios found under ./scenarios/");
    }

    for (const scenario of scenarios) {
      console.log(`\n📷 Running scenario: ${scenario.name}`);
      console.log("-".repeat(40));
      try {
        await scenario.run(page, context);
        console.log(`✓ ${scenario.name} completed`);
      } catch (error) {
        console.error(`✗ ${scenario.name} failed:`, error);
        // Continue with the rest — partial output is more useful than none.
      }
    }

    printScreenshotSummary();
    console.log("✅ Screenshot capture complete!\n");
  } catch (error) {
    console.error("❌ Screenshot capture failed:", error);
    process.exit(1);
  } finally {
    if (context) {
      await context.close();
    }
    if (browser) {
      await browser.close();
    }
  }
}

async function loadScenarios(): Promise<ScenarioModule[]> {
  const scenarios: ScenarioModule[] = [];

  const entries: Array<{ name: string; path: string }> = [
    { name: "Browse", path: "./scenarios/browse.js" },
    { name: "Series Detail", path: "./scenarios/series-detail.js" },
    { name: "Admin Overview", path: "./scenarios/admin-overview.js" },
    { name: "Admin Review", path: "./scenarios/admin-review.js" },
    { name: "Admin Kept", path: "./scenarios/admin-kept.js" },
    { name: "Admin Sources", path: "./scenarios/admin-sources.js" },
    { name: "Admin Providers", path: "./scenarios/admin-providers.js" },
    { name: "Admin Metrics", path: "./scenarios/admin-metrics.js" },
    { name: "Admin ID Maps", path: "./scenarios/admin-id-maps.js" },
    { name: "Admin Maintenance", path: "./scenarios/admin-maintenance.js" },
  ];

  for (const entry of entries) {
    try {
      const mod = await import(entry.path);
      scenarios.push({ name: entry.name, run: mod.run });
    } catch (err) {
      console.log(`⚠️  Scenario "${entry.name}" not found, skipping (${(err as Error).message})`);
    }
  }

  return scenarios;
}

/// Resolve which sources to poll: explicit POLL_SOURCES wins, otherwise
/// every enabled source the API reports.
async function resolvePollSources(context: BrowserContext): Promise<string[]> {
  if (config.pollSources.length > 0) {
    return config.pollSources;
  }
  const sources = await listSources(context.request);
  return sources.filter((s) => s.config.enabled).map((s) => s.name);
}

async function waitForBackend(
  page: Page,
  maxRetries: number = 30,
  initialDelay: number = 1000,
): Promise<void> {
  let delay = initialDelay;

  for (let i = 0; i < maxRetries; i++) {
    try {
      const response = await page.goto("/api/v1/health", {
        timeout: 10000,
        waitUntil: "domcontentloaded",
      });
      if (response && response.ok()) {
        return;
      }
    } catch {
      // retry
    }

    console.log(`  Waiting for backend... (attempt ${i + 1}/${maxRetries})`);
    await page.waitForTimeout(delay);
    delay = Math.min(delay * 1.2, 5000);
  }

  throw new Error("Backend did not become available");
}

main();
