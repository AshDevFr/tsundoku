/// <reference types="vitest/config" />
import path from "node:path";
import react from "@vitejs/plugin-react-swc";
import { defineConfig } from "vite";

const isMockApi = process.env.VITE_MOCK_API === "true";
const apiUrl = process.env.VITE_API_URL ?? "http://localhost:8080";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  server: {
    port: 5173,
    host: true,
    // In mock mode MSW intercepts requests in the browser, so no proxy.
    proxy: isMockApi
      ? undefined
      : {
          "/api": {
            target: apiUrl,
            changeOrigin: true,
            timeout: 60000,
            proxyTimeout: 60000,
            cookieDomainRewrite: { localhost: "localhost" },
            configure: (proxy) => {
              proxy.on("error", (err) => {
                console.log("Proxy error:", err.message);
              });
              // Keep Server-Sent Events streaming and unbuffered.
              proxy.on("proxyReq", (proxyReq, req) => {
                if (
                  req.url?.includes("/stream") ||
                  req.url?.includes("/events") ||
                  req.headers.accept?.includes("text/event-stream")
                ) {
                  proxyReq.setHeader("Cache-Control", "no-cache");
                  proxyReq.setHeader("Connection", "keep-alive");
                }
              });
            },
          },
          "/docs": { target: apiUrl, changeOrigin: true },
        },
  },
  build: {
    outDir: "dist",
    sourcemap: true,
  },
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: "./src/test/setup.ts",
    css: false,
  },
});
