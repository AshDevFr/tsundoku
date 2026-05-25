// AUTO-GENERATED PLACEHOLDER — do not edit by hand.
//
// This file is normally produced from the backend OpenAPI spec by:
//   make openapi-all
// (cargo run -- openapi --output web/openapi.json && npm run generate:types)
//
// A minimal stub is checked in so the project type-checks before the first
// generation. It is overwritten the moment you run the command above.

export interface paths {
  "/api/v1/health": {
    get: {
      responses: {
        200: {
          content: {
            "application/json": { status: string };
          };
        };
      };
    };
  };
}

export type components = Record<string, never>;
