import { HttpResponse, http } from "msw";

// Mock API handlers, shared by the browser worker (dev:mock) and the node
// server (vitest). Add a handler per endpoint you want to fake.
export const handlers = [
  http.get("/api/v1/health", () => HttpResponse.json({ status: "ok" })),
];
