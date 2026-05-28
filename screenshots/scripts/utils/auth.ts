import type { BrowserContext } from "playwright";
import { config } from "../../playwright.config.js";

/// Inject the admin token into localStorage before any page script runs.
/// `AdminAuthGate` checks the zustand-persisted store under
/// `tsundoku.admin-token.v1`; setting it here means the admin pages
/// render straight through without the token-entry card.
export async function seedAdminToken(
  context: BrowserContext,
  token: string = config.admin.token,
): Promise<void> {
  const payload = JSON.stringify({
    state: { token },
    version: 0,
  });

  await context.addInitScript(
    ({ key, value }) => {
      try {
        window.localStorage.setItem(key, value);
      } catch {
        // SecurityError on file:// or about:blank — ignore, we only
        // care about the http(s) origin our pages run on.
      }
    },
    { key: "tsundoku.admin-token.v1", value: payload },
  );
}

/// Authorization header for direct API calls (e.g. triggering a poll).
export function bearer(token: string = config.admin.token): {
  Authorization: string;
} {
  return { Authorization: `Bearer ${token}` };
}
