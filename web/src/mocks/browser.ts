import { setupWorker } from "msw/browser";
import { handlers } from "./handlers";

// Used in the browser when running `npm run dev:mock`.
// Requires the service worker file: `npx msw init public/ --save`.
export const worker = setupWorker(...handlers);
