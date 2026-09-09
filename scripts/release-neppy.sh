#!/usr/bin/env bash
# Cut a Neppy release: build, sign, publish to GitHub, update the feed.
#
#   scripts/release-neppy.sh 0.64.1
#   scripts/release-neppy.sh 0.64.1 --dry-run
#   scripts/release-neppy.sh 0.64.1 --notes-file /path/to/curated-notes.md
#
# After this runs, an installed Neppy sees the new version through the updater
# and upgrades itself. No manual .app copying is needed.
#
# The only files this script uploads are the signed Neppy app tarball and its
# signature. GitHub adds "Source code (zip)" and "Source code (tar.gz)" links
# to every GitHub Release automatically; those snapshots cannot be disabled.
#
# Requirements:
#   - ~/.neppy-updater/neppy.key   the minisign PRIVATE key (never in the repo)
#   - gh authenticated for release creation
#   - git credentials allowed to push the release branch and tag
#   - gh, pnpm, cargo, rust toolchain
set -euo pipefail

REPO="twister25rus-dot/Neppy"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KEYDIR="$HOME/.neppy-updater"
KEY="$KEYDIR/neppy.key"
VERSION=""
DRY_RUN=""
NOTES_OVERRIDE=""
RELEASE_BRANCH="${NEPPY_RELEASE_BRANCH:-main}"
TAG=""
SNAP=""
NOTES_FILE=""

RELEASE_METADATA=(
  Cargo.toml
  Cargo.lock
  app/package.json
  app/src-tauri/Cargo.toml
  app/src-tauri/Cargo.lock
  app/src-tauri/tauri.conf.json
  CHANGELOG.md
)

die() { echo "error: $*" >&2; exit 1; }

render_commit_bullets() {
  local from_ref="$1"
  local to_ref="$2"
  local sha short_sha subject
  while IFS=$'\t' read -r sha short_sha subject; do
    case "$subject" in
      Release\ [0-9]*|Update\ updater\ feed\ for\ *|chore\(release\):*|chore\(staging\):*)
        continue
        ;;
    esac
    printf -- '- %s ([`%s`](https://github.com/%s/commit/%s))\n' \
      "$subject" "$short_sha" "$REPO" "$sha"
  done < <(git log --reverse --format='%H%x09%h%x09%s' "$from_ref..$to_ref")
}

cleanup() {
  local exit_code=$?
  set +e
  if [ -n "$NOTES_FILE" ]; then
    rm -f -- "$NOTES_FILE"
  fi
  if [ "$DRY_RUN" = "--dry-run" ] && [ -n "$SNAP" ] && [ -d "$SNAP" ]; then
    for file in "${RELEASE_METADATA[@]}"; do
      cp "$SNAP/$file" "$file"
    done
    rm -r -- "$SNAP"
    echo "==> dry run: release metadata restored to its pre-run versions"
  fi
  return "$exit_code"
}
trap cleanup EXIT

if [ "$#" -gt 0 ]; then
  VERSION="$1"
  shift
fi
while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run)
      [ -z "$DRY_RUN" ] || die "--dry-run was provided more than once"
      DRY_RUN="--dry-run"
      shift
      ;;
    --notes-file)
      [ -z "$NOTES_OVERRIDE" ] || die "--notes-file was provided more than once"
      [ "$#" -ge 2 ] || die "--notes-file requires a path"
      NOTES_OVERRIDE="$2"
      shift 2
      ;;
    *)
      die "unknown option '$1'"
      ;;
  esac
done

[ -n "$VERSION" ] || die "usage: $0 <version> [--dry-run] [--notes-file <path>]"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version must be X.Y.Z, got '$VERSION'"
TAG="v$VERSION"
[ -f "$KEY" ] || die "missing signing key at $KEY (see: tauri signer generate)"
command -v gh >/dev/null || die "gh CLI not found"
command -v node >/dev/null || die "node not found"

cd "$ROOT"

if [ -n "$NOTES_OVERRIDE" ]; then
  [ -s "$NOTES_OVERRIDE" ] || die "notes file is missing or empty: $NOTES_OVERRIDE"
  NOTES_OVERRIDE="$(cd "$(dirname "$NOTES_OVERRIDE")" && pwd)/$(basename "$NOTES_OVERRIDE")"
fi

CURRENT_BRANCH="$(git branch --show-current)"
[ "$CURRENT_BRANCH" = "$RELEASE_BRANCH" ] \
  || die "release must run from '$RELEASE_BRANCH' (currently '$CURRENT_BRANCH')"
[ -z "$(git status --porcelain)" ] \
  || die "working tree must be clean before cutting a release"

echo "==> refreshing $RELEASE_BRANCH and release tags"
git fetch origin "$RELEASE_BRANCH" --tags
git merge-base --is-ancestor "origin/$RELEASE_BRANCH" HEAD \
  || die "local $RELEASE_BRANCH is behind or has diverged from origin/$RELEASE_BRANCH"

if git show-ref --verify --quiet "refs/tags/$TAG"; then
  die "tag $TAG already exists"
fi
if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  die "GitHub Release $TAG already exists"
fi

# A dry run must not leave the tree pinned to a version that was never
# published. Snapshot every release metadata file and restore it on any exit.
if [ "$DRY_RUN" = "--dry-run" ]; then
  SNAP="$(mktemp -d)"
  for file in "${RELEASE_METADATA[@]}"; do
    mkdir -p "$SNAP/$(dirname "$file")"
    cp "$file" "$SNAP/$file"
  done
fi

BASE_COMMIT="$(git rev-parse HEAD)"
PREVIOUS_TAG="$(gh release view --repo "$REPO" --json tagName --jq '.tagName' 2>/dev/null || true)"
NOTES_FILE="$(mktemp "${TMPDIR:-/tmp}/neppy-release-notes.XXXXXX")"

echo "==> generating release notes"
if [ -n "$NOTES_OVERRIDE" ]; then
  cp "$NOTES_OVERRIDE" "$NOTES_FILE"
  echo "    using curated notes from $NOTES_OVERRIDE"
else
  NOTES_FROM=""
  if [ -n "$PREVIOUS_TAG" ]; then
    NOTES_FROM="$(git rev-parse "$PREVIOUS_TAG^{commit}")"
    PREVIOUS_VERSION="${PREVIOUS_TAG#v}"
    # Older local releases tagged the commit before their version bump. If the
    # matching release commit is after that tag, start after the release commit
    # so the next changelog does not repeat changes that were already shipped.
    PREVIOUS_RELEASE_COMMIT="$(
      git log --format='%H%x09%s' "$PREVIOUS_TAG..$BASE_COMMIT" \
        | awk -F $'\t' -v wanted="Release $PREVIOUS_VERSION" '$2 == wanted { print $1; exit }'
    )"
    if [ -n "$PREVIOUS_RELEASE_COMMIT" ]; then
      NOTES_FROM="$PREVIOUS_RELEASE_COMMIT"
    fi
  else
    NOTES_FROM="$(git rev-list --max-parents=0 "$BASE_COMMIT")"
  fi

  COMMIT_BULLETS="$(render_commit_bullets "$NOTES_FROM" "$BASE_COMMIT")"
  [ -n "$COMMIT_BULLETS" ] \
    || die "no releasable commits found after $NOTES_FROM; use --notes-file only for an intentional notes-only release"

  GH_NOTES_ARGS=(
    "repos/$REPO/releases/generate-notes"
    --method POST
    -f "tag_name=$TAG"
    -f "target_commitish=$BASE_COMMIT"
    --jq '.body'
  )
  if [ -n "$PREVIOUS_TAG" ]; then
    GH_NOTES_ARGS+=( -f "previous_tag_name=$PREVIOUS_TAG" )
  fi

  if ! gh api "${GH_NOTES_ARGS[@]}" >"$NOTES_FILE"; then
    echo "warning: GitHub note generation failed; using commit-based notes" >&2
    : >"$NOTES_FILE"
  fi
  if ! grep -Eq '^[*-][[:space:]]+' "$NOTES_FILE"; then
    {
      echo "## What's Changed"
      echo
      echo "$COMMIT_BULLETS"
      echo
      echo "**Full Changelog:** https://github.com/$REPO/compare/$NOTES_FROM...$BASE_COMMIT"
    } >"$NOTES_FILE"
  fi
fi
[ -s "$NOTES_FILE" ] || die "generated release notes are empty"

echo "==> updating CHANGELOG.md"
node scripts/release/update-changelog.mjs \
  --version "$VERSION" \
  --date "$(date -u +%F)" \
  --notes-file "$NOTES_FILE" \
  --changelog CHANGELOG.md

# The updater compares against the version baked into the bundle, so every
# manifest must agree or the app will re-offer an update it already installed.
echo "==> pinning version $VERSION across all four manifests"
python3 - "$VERSION" <<'PY'
import re, sys
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

# The root Cargo.lock records this crate's own version, and it is a separate
# Cargo world from app/src-tauri (two manifests, locks, and target dirs).
echo "==> refreshing the root Cargo.lock"
cargo metadata --manifest-path Cargo.toml --format-version 1 >/dev/null \
  || die "could not refresh Cargo.lock"

echo "==> building the bundle (this is the slow part)"
export GGML_NATIVE=OFF
# TAURI_SIGNING_PRIVATE_KEY takes the key content, not a path. The path form is
# the separate TAURI_SIGNING_PRIVATE_KEY_PATH variable.
export TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY")"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
# `app` only. Adding `dmg` runs an interactive Finder/AppleScript bundling step;
# the updater only needs the signed app tarball.
( cd app && ./node_modules/.bin/tauri build --bundles app -- --bin Neppy )

BUNDLE_DIR="$ROOT/app/src-tauri/target/release/bundle"
TARBALL="$(find "$BUNDLE_DIR/macos" -maxdepth 1 -name '*.app.tar.gz' | head -1)"
SIGFILE="${TARBALL}.sig"
[ -n "$TARBALL" ] || die "no .app.tar.gz produced; is bundle.createUpdaterArtifacts true?"
[ -f "$SIGFILE" ] || die "no signature next to $TARBALL; was TAURI_SIGNING_PRIVATE_KEY set?"

# Apple silicon only. A universal/intel build would add its own platform key.
PLATFORM="darwin-aarch64"
[ "$(uname -m)" = "arm64" ] || PLATFORM="darwin-x86_64"

if [ "$DRY_RUN" = "--dry-run" ]; then
  echo "==> DRY RUN; would commit release metadata, push it, and tag that commit as $TAG"
  echo "==> DRY RUN; would publish exactly these two custom assets:"
  echo "    $TARBALL"
  echo "    $SIGFILE"
  echo "    GitHub will also display its automatic source-code snapshots."
  exit 0
fi

echo "==> committing version $VERSION and its changelog"
git add -- "${RELEASE_METADATA[@]}"
if git diff --cached --quiet; then
  die "release metadata did not change"
fi
git commit -q -m "Release $VERSION"
RELEASE_COMMIT="$(git rev-parse HEAD)"

# Push the version/changelog commit first, then tag that exact commit. Creating
# the GitHub Release comes last so its tag can never point at the prior version.
echo "==> pushing release commit $RELEASE_COMMIT"
git push origin "HEAD:$RELEASE_BRANCH"
git tag -a "$TAG" -m "Neppy $VERSION" "$RELEASE_COMMIT"
git push origin "refs/tags/$TAG"

echo "==> publishing $TAG to $REPO"
# Do not add source archives here. GitHub provides its own generated source
# snapshots; Neppy explicitly uploads only the signed updater artifact pair.
gh release create "$TAG" --repo "$REPO" --verify-tag \
  --title "Neppy $VERSION" \
  --notes-file "$NOTES_FILE" \
  "$TARBALL" "$SIGFILE" >/dev/null

# A private repo's browser download URL will not serve the asset, so the feed
# points at the REST asset endpoint, which honours the app's bearer token.
ASSET_ID="$(gh api "repos/$REPO/releases/tags/$TAG" \
  --jq ".assets[] | select(.name==\"$(basename "$TARBALL")\") | .id")"
[ -n "$ASSET_ID" ] || die "could not resolve the uploaded asset id for $TAG"
ASSET_URL="https://api.github.com/repos/$REPO/releases/assets/$ASSET_ID"

echo "==> writing updater/latest.json"
mkdir -p "$ROOT/updater"
python3 - "$VERSION" "$PLATFORM" "$ASSET_URL" "$SIGFILE" "$NOTES_FILE" <<'PY'
import json, sys, datetime
version, platform, url, sigfile, notesfile = sys.argv[1:6]
manifest = {
    "version": version,
    "notes": open(notesfile).read().strip(),
    "pub_date": datetime.datetime.now(datetime.timezone.utc)
        .strftime("%Y-%m-%dT%H:%M:%SZ"),
    "platforms": {platform: {"signature": open(sigfile).read().strip(), "url": url}},
}
json.dump(manifest, open("updater/latest.json", "w"), indent=2)
open("updater/latest.json", "a").write("\n")
print("  feed ->", version, platform)
PY

echo "==> committing the updater feed"
git add -- updater/latest.json
git commit -q -m "Update updater feed for $VERSION"
git push origin "HEAD:$RELEASE_BRANCH"

echo
echo "done: $TAG published with release notes and its signed app assets."
echo "An installed Neppy will offer $VERSION on its next update check."
