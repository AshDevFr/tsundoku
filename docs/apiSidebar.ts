import type { SidebarsConfig } from "@docusaurus/plugin-content-docs";
import fs from "node:fs";
import path from "node:path";

// docusaurus-plugin-openapi-docs writes a generated `sidebar.ts` next
// to the API mdx pages it produces from `web/openapi.json`. On a fresh
// checkout — or in CI before `gen-api-docs` has run — that file does
// not exist yet. Load it conditionally so the build doesn't bomb out
// at config-evaluation time; the API sidebar will simply be empty
// until generation runs. (Pattern lifted from Codex's docs project.)
const apiSidebarPath = path.join(__dirname, "docs/api/sidebar.ts");

let apiSidebarItems: SidebarsConfig[string] = [];

if (fs.existsSync(apiSidebarPath)) {
  // biome-ignore lint/security/noDangerouslySetInnerHtml: dynamic require is intentional
  apiSidebarItems = require("./docs/api/sidebar.ts").default;
}

export default apiSidebarItems;
