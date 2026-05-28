import type * as Preset from "@docusaurus/preset-classic";
import type { Config } from "@docusaurus/types";
import type * as OpenApiPlugin from "docusaurus-plugin-openapi-docs";
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
  organizationName: "AshDevFr",
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
          // Required by docusaurus-theme-openapi-docs: API pages opt
          // into the theme's ApiItem renderer (the OpenAPI-aware page
          // shell). Non-API markdown pages use the default theme
          // component automatically.
          docItemComponent: "@theme/ApiItem",
        },
        // No blog. We surface release notes via the GitHub releases page.
        blog: false,
        theme: {
          customCss: ["./src/css/custom.css"],
        },
      } satisfies Preset.Options,
    ],
  ],

  plugins: [
    [
      "docusaurus-plugin-openapi-docs",
      {
        id: "api",
        docsPluginId: "classic",
        config: {
          tsundoku: {
            // The spec lives inside `docs/` so Cloudflare Pages can be
            // configured with "Root directory: docs/" and only check
            // out the docs subtree (rather than the whole monorepo).
            // `make openapi` copies `web/openapi.json` here as part of
            // its workflow.
            specPath: "api/openapi.json",
            outputDir: "docs/api",
            sidebarOptions: {
              groupPathsBy: "tag",
              categoryLinkSource: "tag",
            },
            showSchemas: true,
          } satisfies OpenApiPlugin.Options,
        },
      },
    ],
  ],

  themes: [
    "docusaurus-theme-openapi-docs",
    [
      require.resolve("@easyops-cn/docusaurus-search-local"),
      /** @type {import("@easyops-cn/docusaurus-search-local").PluginOptions} */
      ({
        // Long-term cache friendliness.
        hashed: true,
        language: ["en"],
        docsDir: ["docs"],
        docsRouteBasePath: ["docs"],
        // API reference pages are dense and not useful in free-text
        // search. Exclude them so search results stay readable —
        // matches Codex's pattern.
        ignoreFiles: [/docs\/api\/.*/],
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
          type: "docSidebar",
          sidebarId: "apiSidebar",
          position: "left",
          label: "API",
        },
        {
          href: "https://github.com/AshDevFr/tsundoku",
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
              href: "https://github.com/AshDevFr/tsundoku",
            },
            {
              label: "Issues",
              href: "https://github.com/AshDevFr/tsundoku/issues",
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
