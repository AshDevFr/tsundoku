#!/usr/bin/env bash
# Pre-commit hook: ensure the OpenAPI spec and generated TypeScript types are in
# sync with the backend. Regenerates the files and fails the commit if anything
# differs from what is currently staged.

set -e

OPENAPI_JSON="web/openapi.json"
OPENAPI_TYPES="web/src/types/api.generated.ts"

echo "Checking OpenAPI spec synchronization..."

echo "Regenerating OpenAPI spec and TypeScript types..."
if ! make openapi-all > /dev/null 2>&1; then
    echo ""
    echo "ERROR: Failed to regenerate OpenAPI files."
    echo "Run 'make openapi-all' manually to see the full error."
    exit 1
fi

if ! git diff --quiet -- "$OPENAPI_JSON" "$OPENAPI_TYPES"; then
    echo ""
    echo "ERROR: OpenAPI files are out of sync with the backend."
    echo ""
    git --no-pager diff --stat -- "$OPENAPI_JSON" "$OPENAPI_TYPES"
    echo ""
    echo "Please stage the updated files:"
    echo ""
    echo "  git add $OPENAPI_JSON $OPENAPI_TYPES"
    echo ""
    echo "Then try committing again."
    exit 1
fi

echo "OpenAPI files are in sync."
exit 0
