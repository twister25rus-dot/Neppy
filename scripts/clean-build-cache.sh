#!/usr/bin/env bash
# Reclaim disk from regenerable build output. Source, `vendor/`, and tracked
# files are never touched.
#
#   scripts/clean-build-cache.sh              # safe tier (default)
#   scripts/clean-build-cache.sh --dry-run    # report only, delete nothing
#   scripts/clean-build-cache.sh --all        # also drop the warm dep caches
#
# Why this exists: a Rust workspace this size accumulates tens of gigabytes that
# no later build ever reads. The two worst offenders are
#
#   - `incremental/`, which only ever speeds up a *re*build of the local crates
#     and is rewritten wholesale when it does; and
#   - a `debug/` tree inside the private release target dir, which
#     `release-neppy.sh` can never produce — it only ever builds `--release`, so
#     a `debug/` there is always fallout from an interactive cargo command that
#     inherited the exported CARGO_TARGET_DIR.
#
# The safe tier keeps `target/debug/deps` warm, so the next build recompiles the
# workspace crates but not the whole dependency closure. `--all` drops that too
# and costs a full from-scratch rebuild.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# A `rm -rf` loop has no business running anywhere but this repo.
[ -f "$ROOT/Cargo.toml" ] && [ -d "$ROOT/.git" ] \
  || { echo "refusing to run outside the repo root ($ROOT)" >&2; exit 1; }

MODE=safe
DRY_RUN=0
ASSUME_YES=0
for arg in "$@"; do
  case "$arg" in
    --all)          MODE=all ;;
    --safe)         MODE=safe ;;
    --dry-run|-n)   DRY_RUN=1 ;;
    --yes|-y)       ASSUME_YES=1 ;;
    -h|--help)      sed -n '2,22p' "$0"; exit 0 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

# Kept identical to release-neppy.sh so both agree on which dir is private.
RELEASE_TARGET_DIR="${NEPPY_RELEASE_TARGET_DIR:-$ROOT/app/src-tauri/target-release}"

TARGETS=()
add() { if [ -e "$1" ] || [ -L "$1" ]; then TARGETS+=("$1"); fi; }

# ── Safe tier: output that no build reads again ─────────────────────────────
add target/debug/incremental
add target/doc
add app/src-tauri/target/debug/incremental
add app/src-tauri/target/release/incremental
add "$RELEASE_TARGET_DIR/release/incremental"
# Never a legitimate product of the release script; see the header.
add "$RELEASE_TARGET_DIR/debug"
# Temp dirs the agent-harness e2e suite leaves behind when a run is interrupted.
while IFS= read -r stray; do add "$stray"; done < <(
  find target -maxdepth 1 -name 'agent-harness-e2e-*' 2>/dev/null || true
)

# ── --all: the warm caches too ──────────────────────────────────────────────
if [ "$MODE" = all ]; then
  TARGETS=()
  add target
  add app/src-tauri/target
  add "$RELEASE_TARGET_DIR"

  BUNDLE="$RELEASE_TARGET_DIR/release/bundle/macos/Neppy.app"
  if [ -d "$BUNDLE" ] && [ "$DRY_RUN" -eq 0 ] && [ "$ASSUME_YES" -eq 0 ]; then
    VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' \
      "$BUNDLE/Contents/Info.plist" 2>/dev/null || echo unknown)"
    echo "--all also removes the built bundle Neppy $VERSION."
    echo "Published releases stay downloadable from GitHub; only this local copy goes."
    read -r -p "continue? [y/N] " reply
    [ "$reply" = y ] || [ "$reply" = Y ] || { echo "aborted"; exit 1; }
  fi
fi

free_gb() { df -g /System/Volumes/Data 2>/dev/null | awk 'NR==2{print $4}'; }
BEFORE="$(free_gb)"

if [ "${#TARGETS[@]}" -eq 0 ]; then
  echo "nothing to clean"
else
  for path in "${TARGETS[@]}"; do
    size="$(du -sh "$path" 2>/dev/null | cut -f1 || echo '?')"
    if [ "$DRY_RUN" -eq 1 ]; then
      printf 'would remove  %6s  %s\n' "$size" "${path#$ROOT/}"
    else
      printf 'removing      %6s  %s\n' "$size" "${path#$ROOT/}"
      rm -rf "$path"
    fi
  done
fi

# Finder droppings are gitignored, so they never show up in `git status` and
# accumulate unnoticed across every directory the user has ever opened.
if [ "$DRY_RUN" -eq 1 ]; then
  echo "would remove  $(find . -name .DS_Store 2>/dev/null | wc -l | tr -d ' ') .DS_Store files"
else
  find . -name .DS_Store -delete 2>/dev/null || true
fi

# Loose objects dominate `.git` long before the pack does; this repo has been
# observed at 4.6 GiB loose against a 186 MiB pack.
if [ "$DRY_RUN" -eq 1 ]; then
  echo "would run     git gc --prune=now  (.git is $(du -sh .git | cut -f1))"
else
  echo "packing git objects (.git is $(du -sh .git | cut -f1))"
  if ! git gc --prune=now --quiet 2>/tmp/neppy-gc.err; then
    # A gc killed mid-run leaves a gc.pid naming a process that no longer
    # exists. Forcing past a *live* gc would corrupt it, so check first.
    stale_pid="$(sed -n 's/.*pid \([0-9]\{1,\}\).*/\1/p' /tmp/neppy-gc.err | head -1)"
    if [ -n "$stale_pid" ] && ! ps -p "$stale_pid" >/dev/null 2>&1; then
      echo "clearing stale gc lock from dead pid $stale_pid"
      git gc --prune=now --quiet --force
    else
      cat /tmp/neppy-gc.err >&2
    fi
  fi
  rm -f /tmp/neppy-gc.err
fi

AFTER="$(free_gb)"
if [ "$DRY_RUN" -eq 1 ]; then
  echo "dry run; nothing was deleted"
else
  echo "free: ${BEFORE}Gi -> ${AFTER}Gi   repo now $(du -sh "$ROOT" 2>/dev/null | cut -f1)"
fi
