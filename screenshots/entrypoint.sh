#!/bin/bash
set -e

# First-run install. node_modules is a named volume so the install only
# happens once per `down -v`.
if [ ! -d "/work/node_modules/tsx" ]; then
    echo "Installing dependencies..."
    npm install
fi

exec "$@"
