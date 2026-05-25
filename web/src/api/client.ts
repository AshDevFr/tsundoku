import createClient from "openapi-fetch";
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
