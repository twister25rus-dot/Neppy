#!/usr/bin/env bash
# Install the tiny.place Hermes plugin into ~/.hermes/plugins/tinyplace/.
#
# Mirrors sdk/skill/tinyplace-agent/scripts/install-hermes.sh: it copies the
# installable plugin package into the Hermes plugins dir and ensures the
# tiny.place Python SDK (which the plugin imports) is installed.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
PLUGIN_SRC="$SCRIPT_DIR/src/tinyplace"
INSTALL_DIR="${HERMES_HOME:-$HOME/.hermes}/plugins/tinyplace"

# 1. Install the SDK the plugin wraps (editable from this repo by default).
PYTHON_BIN="${PYTHON_BIN:-python3}"
if [ "${TINYPLACE_INSTALL_SDK:-1}" = "1" ]; then
  echo "Installing tiny.place Python SDK..."
  "$PYTHON_BIN" -m pip install -e "$REPO_ROOT/sdk/python"
fi

# 2. Copy the plugin package into the Hermes plugins directory.
mkdir -p "$(dirname "$INSTALL_DIR")"
rm -rf "$INSTALL_DIR"
cp -R "$PLUGIN_SRC" "$INSTALL_DIR"
# Never ship a dev venv / caches into the install.
rm -rf "$INSTALL_DIR/.venv" "$INSTALL_DIR/__pycache__"

echo "Installed tiny.place Hermes plugin to $INSTALL_DIR"
echo
echo "Enable it and set the required env, then run Hermes:"
echo "  hermes plugins enable tinyplace"
echo "  export TINYPLACE_AGENT_KEY=<ed25519-seed-or-solana-secret, base58/base64>"
echo "  export TINYPLACE_API_BASE_URL=https://staging-api.tiny.place   # optional"
echo "  HERMES_PLUGINS_DEBUG=1 hermes plugins list"
