import createClient from "openapi-fetch";
import { currentAdminToken } from "@/stores/auth";
import type { paths } from "@/types/api.generated";

// Typed against the generated OpenAPI paths. Regenerate after backend changes:
//   make openapi-all
//
// baseUrl is the current origin so requests stay same-origin: Vite proxies /api
// to the backend in dev, and rust-embed serves the SPA and the API from one
// binary in prod. (An absolute base is also required under jsdom, where the
// test fetch cannot resolve relative URLs.)
//
// The `fetch` wrapper defers to the global fetch at call time. openapi-fetch
// otherwise captures globalThis.fetch when the client is created, which would
// miss test mocks (MSW) that patch fetch after this module loads.
const baseUrl = typeof window !== "undefined" ? window.location.origin : "";

export const api = createClient<paths>({
  baseUrl,
  fetch: (...args) => fetch(...args),
});

// Attach the admin bearer when one is set, on EVERY request — not just writes.
// Writes require it; reads use it so admin-only response enrichment (the Codex
// presence overlay on series, and GET /codex/status) comes back. The server
// treats a valid bearer on a read as "admin" via its MaybeAdmin extractor; an
// absent/invalid token simply yields the public payload. (Reads remain
// unauthenticated by default — `read_requires_auth` is not surfaced in the UI.)
api.use({
  onRequest({ request }) {
    const token = currentAdminToken();
    if (token) request.headers.set("Authorization", `Bearer ${token}`);
    return request;
  },
});
