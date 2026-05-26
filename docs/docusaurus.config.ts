import type * as Preset from "@docusaurus/preset-classic";
import type { Config } from "@docusaurus/types";
import { themes as prismThemes } from "prism-react-renderer";

// Read version from package.json so the navbar badge tracks the site's own
// pin (separate from the binary's Cargo version).
import packageJson from "./package.json";
const appVersion = packageJson.version;

// This runs in Node.js — don't use client-side code here (browser APIs, JSX).

const config: Config = {
  title: "tsundoku",
  tagline: "Manga discovery sidecar for Codex",
  favicon: "img/tsundoku-logo.svg",

  // Future flags, see https://docusaurus.io/docs/api/docusaurus-config#future
  future: {
    v4: true, // Smooths the upgrade to Docusaurus v4.
  },

  // Production URL. Cloudflare Pages will serve at this host once Phase 3
  // wires up the custom domain. Adjust if the operator picks a different
  // host.
  url: "https://tsundoku.4sh.dev",
  baseUrl: "/",

  // GitHub Pages config would live here if we used GitHub Pages. We don't
  // (Cloudflare Pages), so these are unused but harmless.
  organizationName: "skewb1k",
  projectName: "tsundoku",

  onBrokenLinks: "throw",

  // Even when we don't ship translations, the i18n block sets useful HTML
  // metadata (`<html lang="en">`).
  i18n: {
    defaultLocale: "en",
    locales: ["en"],
  },

  presets: [
    [
      "classic",
      {
        docs: {
          sidebarPath: "./sidebars.ts",
        },
        // No blog. We surface release notes via the GitHub releases page.
        blog: false,
        theme: {
          customCss: ["./src/css/custom.css"],
        },
      } satisfies Preset.Options,
    ],
  ],

  // Phase 3 adds `docusaurus-plugin-openapi-docs` here for the auto-
  // generated API reference. The plugin is omitted in Phase 1 so the
  // scaffold can build without any OpenAPI spec dependency.
  plugins: [],

  themes: [
    [
      require.resolve("@easyops-cn/docusaurus-search-local"),
      /** @type {import("@easyops-cn/docusaurus-search-local").PluginOptions} */
      ({
        // Long-term cache friendliness.
        hashed: true,
        language: ["en"],
        // Phase 3 will add /docs/api/* and we'll want to exclude those
        // (matches Codex's pattern). For now we only have docs/ content.
        docsDir: ["docs"],
        docsRouteBasePath: ["docs"],
      }),
    ],
  ],

  themeConfig: {
    image: "img/tsundoku-social-card.svg",
    colorMode: {
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: "tsundoku",
      logo: {
        alt: "tsundoku logo",
        src: "img/tsundoku-logo.svg",
      },
      items: [
        {
          type: "html",
          position: "left",
          value: `<span class="badge badge--primary" style="margin-left: 4px; font-size: 0.7rem; vertical-align: middle;">v${appVersion}</span>`,
        },
        {
          type: "docSidebar",
          sidebarId: "tutorialSidebar",
          position: "left",
          label: "Docs",
        },
        {
          href: "https://github.com/skewb1k/tsundoku",
          label: "GitHub",
          position: "right",
        },
      ],
    },
    footer: {
      style: "dark",
      links: [
        {
          title: "Documentation",
          items: [
            { label: "Introduction", to: "/docs/" },
          ],
        },
        {
          title: "Project",
          items: [
            {
              label: "GitHub",
              href: "https://github.com/skewb1k/tsundoku",
            },
            {
              label: "Issues",
              href: "https://github.com/skewb1k/tsundoku/issues",
            },
          ],
        },
        {
          title: "Related",
          items: [
            {
              label: "Codex",
              href: "https://codex.4sh.dev",
            },
          ],
        },
      ],
      copyright: `Copyright © ${new Date().getFullYear()} tsundoku. Built with Docusaurus.`,
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
      additionalLanguages: ["rust", "toml", "bash"],
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
