#!/usr/bin/env bash
# Install git pre-commit hooks via the `pre-commit` framework.
#
# Requires `pre-commit` (https://pre-commit.com/):
#   macOS:   brew install pre-commit
#   pipx:    pipx install pre-commit
#   pip:     pip install --user pre-commit
#
# Hooks are configured in .pre-commit-config.yaml.

set -euo pipefail

if ! command -v pre-commit >/dev/null 2>&1; then
  echo "Error: 'pre-commit' is not installed."
  echo ""
  echo "Install it with one of:"
  echo "  brew install pre-commit"
  echo "  pipx install pre-commit"
  echo "  pip install --user pre-commit"
  exit 1
fi

# Install the git hook script
pre-commit install
echo ""
echo "Pre-commit hooks installed. They run on every 'git commit'."
echo "Run them manually with: pre-commit run --all-files"
