#!/usr/bin/env bash
# Build the Linux arm64 CLI tarball and optionally upload to a GitHub release.
#
# Usage:
#   build-linux-arm64.sh <tag>
#
# Environment:
#   GITHUB_TOKEN          — upload to release when set
#   OPENHUMAN_SENTRY_DSN  — optional Sentry DSN baked into the binary
#   UPLOAD_REPO           — GitHub repo slug (default: tinyhumansai/openhuman)
set -euo pipefail

TAG="${1:?Usage: build-linux-arm64.sh <tag>}"
VERSION="${TAG#v}"
TARGET="aarch64-unknown-linux-gnu"
UPLOAD_REPO="${UPLOAD_REPO:-tinyhumansai/openhuman}"

echo "[linux-arm64] Building neppy-core for $TARGET ..."
if command -v rustup >/dev/null 2>&1; then
  rustup target add "$TARGET"
fi
cargo build --release --bin neppy-core --target "$TARGET"

TARBALL="neppy-core-${VERSION}-${TARGET}.tar.gz"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cp "target/${TARGET}/release/neppy-core" "$WORK/"
chmod +x "$WORK/neppy-core"
tar -czf "$TARBALL" -C "$WORK" neppy-core
openssl dgst -sha256 -r "$TARBALL" | awk '{print $1}' > "${TARBALL}.sha256"

echo "[linux-arm64] Created $TARBALL (sha256: $(cat "${TARBALL}.sha256"))"

if [[ -n "${GITHUB_TOKEN:-}" ]]; then
  gh release upload "$TAG" \
    "$TARBALL" "${TARBALL}.sha256" \
    --repo "$UPLOAD_REPO" --clobber
  echo "[linux-arm64] Uploaded $TARBALL"
fi
