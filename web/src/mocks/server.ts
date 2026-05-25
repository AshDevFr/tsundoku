import { setupServer } from "msw/node";
import { handlers } from "./handlers";

// Used by vitest (node environment).
export const server = setupServer(...handlers);
