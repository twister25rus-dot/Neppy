#!/usr/bin/env bash
# Cut a Neppy release: build, sign, publish to GitHub, update the feed.
#
#   scripts/release-neppy.sh 0.64.1
#   scripts/release-neppy.sh 0.64.1 --dry-run
#
# After this runs, an installed Neppy sees the new version through the updater
# and upgrades itself — no manual .app copying.
#
# Requirements:
#   - ~/.neppy-updater/neppy.key   the minisign PRIVATE key (never in the repo)
#   - ~/.neppy-updater/token       a GitHub PAT with `repo` read (private feed)
#   - gh, pnpm, cargo, rust toolchain
set -euo pipefail

REPO="twister25rus-dot/Neppy"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KEYDIR="$HOME/.neppy-updater"
KEY="$KEYDIR/neppy.key"
VERSION="${1:-}"
DRY_RUN="${2:-}"

die() { echo "error: $*" >&2; exit 1; }

[ -n "$VERSION" ] || die "usage: $0 <version> [--dry-run]   e.g. $0 0.64.1"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version must be X.Y.Z, got '$VERSION'"
[ -f "$KEY" ] || die "missing signing key at $KEY (see: tauri signer generate)"
command -v gh >/dev/null || die "gh CLI not found"

cd "$ROOT"

# A dry run must not leave the tree pinned to a version that was never
# published. Snapshot the four manifests and restore them on any exit path.
if [ "$DRY_RUN" = "--dry-run" ]; then
  SNAP="$(mktemp -d)"
  for f in Cargo.toml app/src-tauri/Cargo.toml app/package.json \
           app/src-tauri/tauri.conf.json; do
    mkdir -p "$SNAP/$(dirname "$f")" && cp "$f" "$SNAP/$f"
  done
  restore_manifests() {
    for f in Cargo.toml app/src-tauri/Cargo.toml app/package.json \
             app/src-tauri/tauri.conf.json; do
      cp "$SNAP/$f" "$f"
    done
    rm -rf "$SNAP"
    echo "==> dry run: manifests restored to their pre-run versions"
  }
  trap restore_manifests EXIT
fi

# The updater compares against the version baked into the bundle, so every
# manifest must agree or the app will re-offer an update it already installed.
echo "==> pinning version $VERSION across all four manifests"
python3 - "$VERSION" <<'PY'
import json, re, sys
v = sys.argv[1]
for p in ("Cargo.toml", "app/src-tauri/Cargo.toml"):
    s = open(p).read()
    s = re.sub(r'^version = "[^"]+"', f'version = "{v}"', s, count=1, flags=re.M)
    open(p, "w").write(s)
for p in ("app/package.json", "app/src-tauri/tauri.conf.json"):
    s = open(p).read()
    s = re.sub(r'"version": "[^"]+"', f'"version": "{v}"', s, count=1)
    open(p, "w").write(s)
print("  pinned:", v)
PY

echo "==> building the bundle (this is the slow part)"
export GGML_NATIVE=OFF
# TAURI_SIGNING_PRIVATE_KEY takes the key CONTENT, not a path — the path form
# is the separate TAURI_SIGNING_PRIVATE_KEY_PATH. Passing a path here silently
# produces an unsigned bundle, which only surfaces as a missing .sig after the
# full (slow) release build.
export TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY")"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
# `app` only, deliberately. Adding `dmg` runs bundle_dmg.sh, which drives Finder
# through AppleScript and fails in a non-interactive shell — and because the DMG
# step runs BEFORE the updater artifacts are emitted, that failure leaves no
# .app.tar.gz/.sig at all. The updater needs the tarball, not a disk image.
( cd app && ./node_modules/.bin/tauri build --bundles app -- --bin Neppy )

BUNDLE_DIR="$ROOT/app/src-tauri/target/release/bundle"
TARBALL="$(find "$BUNDLE_DIR/macos" -name '*.app.tar.gz' -maxdepth 1 | head -1)"
SIGFILE="${TARBALL}.sig"
[ -n "$TARBALL" ] || die "no .app.tar.gz produced — is bundle.createUpdaterArtifacts true?"
[ -f "$SIGFILE" ] || die "no signature next to $TARBALL — was TAURI_SIGNING_PRIVATE_KEY set?"

# Apple silicon only. A universal/intel build would add its own platform key here.
PLATFORM="darwin-aarch64"
[ "$(uname -m)" = "arm64" ] || PLATFORM="darwin-x86_64"

TAG="v$VERSION"
if [ "$DRY_RUN" = "--dry-run" ]; then
  echo "==> DRY RUN — would publish $TAG with:"
  echo "    $TARBALL"
  echo "    $SIGFILE"
  exit 0
fi

echo "==> publishing $TAG to $REPO"
gh release create "$TAG" --repo "$REPO" --title "Neppy $VERSION" \
  --notes "Neppy $VERSION" "$TARBALL" "$SIGFILE" >/dev/null

# A PRIVATE repo's browser download URL will not serve the asset, so the feed
# must point at the REST asset endpoint, which honours the bearer token the app
# attaches (see app_update::updater_with_auth).
ASSET_ID="$(gh api "repos/$REPO/releases/tags/$TAG" \
  --jq ".assets[] | select(.name==\"$(basename "$TARBALL")\") | .id")"
[ -n "$ASSET_ID" ] || die "could not resolve the uploaded asset id for $TAG"
ASSET_URL="https://api.github.com/repos/$REPO/releases/assets/$ASSET_ID"

echo "==> writing updater/latest.json"
mkdir -p "$ROOT/updater"
python3 - "$VERSION" "$PLATFORM" "$ASSET_URL" "$SIGFILE" <<'PY'
import json, sys, datetime
version, platform, url, sigfile = sys.argv[1:5]
manifest = {
    "version": version,
    "notes": f"Neppy {version}",
    "pub_date": datetime.datetime.now(datetime.timezone.utc)
        .strftime("%Y-%m-%dT%H:%M:%SZ"),
    "platforms": {platform: {"signature": open(sigfile).read().strip(), "url": url}},
}
json.dump(manifest, open("updater/latest.json", "w"), indent=2)
open("updater/latest.json", "a").write("\n")
print("  feed ->", version, platform)
PY

echo "==> committing the version bump + feed"
git add -A
git commit -q -m "Release $VERSION

Published $TAG and pointed updater/latest.json at its signed asset.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
git push origin main

echo
echo "done: $TAG published, feed updated."
echo "An installed Neppy will offer $VERSION on its next update check."
