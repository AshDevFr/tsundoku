import { defineConfig } from "playwright/test";

const BASE_URL = process.env.BASE_URL || "http://localhost:8080";
const VIEWPORT_WIDTH = parseInt(process.env.VIEWPORT_WIDTH || "1440", 10);
const VIEWPORT_HEIGHT = parseInt(process.env.VIEWPORT_HEIGHT || "900", 10);
const COLOR_SCHEME: "light" | "dark" =
  process.env.COLOR_SCHEME === "light" ? "light" : "dark";

export default defineConfig({
  testDir: "./scripts",
  timeout: 60000,
  expect: {
    timeout: 10000,
  },
  use: {
    baseURL: BASE_URL,
    viewport: { width: VIEWPORT_WIDTH, height: VIEWPORT_HEIGHT },
    colorScheme: COLOR_SCHEME,
    screenshot: "off",
    video: "off",
    trace: "off",
    headless: true,
  },
  projects: [
    {
      name: "chromium",
      use: {
        browserName: "chromium",
      },
    },
  ],
});

export const config = {
  baseUrl: BASE_URL,
  viewport: { width: VIEWPORT_WIDTH, height: VIEWPORT_HEIGHT },
  colorScheme: COLOR_SCHEME,
  outputDir: "./output",
  admin: {
    // Must match `auth.admin_token` in config/tsundoku.screenshots.toml (set
    // via TSUNDOKU_AUTH__ADMIN_TOKEN in the compose service).
    token: process.env.ADMIN_TOKEN || "screenshots-admin-token",
  },
  // Optional one-shot poll on startup. Surfaces real releases on the
  // browse / series-detail / kept pages so the screenshots are not empty.
  // Disable with POLL_ON_START=false for a quick smoke run.
  pollOnStart: process.env.POLL_ON_START !== "false",
  pollSources: (process.env.POLL_SOURCES || "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean),
  pollWaitMaxSeconds: parseInt(
    process.env.POLL_WAIT_MAX_SECONDS || "300",
    10,
  ),
};
