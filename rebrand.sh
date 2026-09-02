#!/usr/bin/env bash
# rebrand.sh — staged Neppy -> Neppy rename.
#
# Run from the repo root (/Users/alex/Neppy). Dry-run by default.
#
#   ./rebrand.sh docs            # preview
#   ./rebrand.sh docs --apply    # do it
#
# Categories, run them in this order, commit + `cargo check` between each:
#   docs      user-facing text, README, gitbooks, locales, prompts
#   env       OPENHUMAN_* -> NEPPY_* environment variables
#   paths     ~/.neppy -> ~/.neppy, ~/Neppy/projects -> ~/Neppy/projects
#   tauri     product name, window title, bundle identifier
#   crates    crate + package names (openhuman -> neppy)
#   moddir    src/neppy/ -> src/neppy/ and every import path (do this LAST)
#
# Everything in PROTECTED below is never touched. See §3.2 of NEPPY-BUILD-SPEC.md
# for why each one is on the list. Do not "simplify" this script by dropping it.

set -euo pipefail

CATEGORY="${1:-}"
APPLY="${2:-}"

if [[ -z "$CATEGORY" ]]; then
  sed -n '2,20p' "$0"
  exit 1
fi

command -v rg >/dev/null || { echo "needs ripgrep (brew install ripgrep)"; exit 1; }

# Directories that are never rewritten. vendor/ holds git submodules whose crate
# names are load-bearing path dependencies.
EXCLUDES=(
  --glob '!vendor/**'
  --glob '!.git/**'
  --glob '!target/**'
  --glob '!node_modules/**'
  --glob '!**/Cargo.lock'
  --glob '!.gitmodules'
)

# Identifiers that must survive verbatim. If a candidate line contains one of
# these, the script refuses to rewrite that line and prints it for manual review.
PROTECTED='tinyagents|tinycortex|tinyflows|tinychannels|tinybus|tinymcp|tinymemory|tinydocs|tinyvoice|tinyjuice|tinyruntime|tinywallet|tinyhumans-sdk|tinyjuice_retrieve|tokenjuice_retrieve|x-sdk-name|openhuman-skills|VITE_SKILLS_GITHUB_REPO|retire_local_whisper_stt|INFERENCE_COMPILED_IN'

case "$CATEGORY" in
  docs)   PATTERN='Neppy' ;      REPLACE='Neppy' ;      SCOPE=(README.md INSTALL.md CONTRIBUTING.md docs gitbooks app/src/locales src/neppy/agent/prompts) ;;
  env)    PATTERN='OPENHUMAN_' ;     REPLACE='NEPPY_' ;     SCOPE=(.) ;;
  paths)  PATTERN='\.neppy' ;    REPLACE='.neppy' ;     SCOPE=(.) ;;
  tauri)  PATTERN='Neppy' ;      REPLACE='Neppy' ;      SCOPE=(app/src-tauri/tauri.conf.json app/src-tauri/Cargo.toml) ;;
  crates) PATTERN='openhuman' ;      REPLACE='neppy' ;      SCOPE=(Cargo.toml app/src-tauri/Cargo.toml package.json app/package.json) ;;
  moddir) echo "moddir is a manual step, see below"; MODDIR=1 ;;
  *)      echo "unknown category: $CATEGORY"; exit 1 ;;
esac

if [[ "${MODDIR:-}" == "1" ]]; then
  cat <<'EOF'

The src/neppy/ -> src/neppy/ move is the largest diff in the whole rebrand
(upstream measured a comparable in-tree move at ~545 import rewrites). Do it by
hand so git tracks it as a rename:

  git mv src/openhuman src/neppy
  rg -l 'crate::neppy|neppy_core|use openhuman' --glob '!vendor/**' \
    | xargs sed -i '' \
        -e 's/crate::neppy/crate::neppy/g' \
        -e 's/neppy_core/neppy_core/g'
  GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml

Then fix the tests that hard-assert namespace strings. Fix the assertions, not
the code — RPC namespaces are deliberately decoupled from module paths and must
keep their original string values.

EOF
  exit 0
fi

echo "category: $CATEGORY"
echo "pattern:  $PATTERN  ->  $REPLACE"
echo

# Lines that match the pattern AND contain a protected identifier: report, skip.
echo "--- lines skipped (protected identifier present) ---"
rg -n "$PATTERN" "${EXCLUDES[@]}" "${SCOPE[@]}" 2>/dev/null \
  | rg -i "$PROTECTED" || echo "(none)"
echo

echo "--- lines that would change ---"
rg -n "$PATTERN" "${EXCLUDES[@]}" "${SCOPE[@]}" 2>/dev/null \
  | rg -iv "$PROTECTED" | head -80
TOTAL=$(rg -n "$PATTERN" "${EXCLUDES[@]}" "${SCOPE[@]}" 2>/dev/null | rg -civ "$PROTECTED" || true)
echo
echo "total: ${TOTAL:-0} lines"

if [[ "$APPLY" != "--apply" ]]; then
  echo
  echo "dry run. re-run with --apply to write."
  exit 0
fi

# Rewrite only files with no protected identifier anywhere in them. Files that
# mix both get listed for manual editing rather than half-rewritten.
MIXED=0
while IFS= read -r f; do
  if rg -qi "$PROTECTED" "$f"; then
    echo "MANUAL: $f (contains a protected identifier)"
    MIXED=$((MIXED+1))
  else
    sed -i '' "s/$PATTERN/$REPLACE/g" "$f"
  fi
done < <(rg -l "$PATTERN" "${EXCLUDES[@]}" "${SCOPE[@]}" 2>/dev/null || true)

echo
echo "done. $MIXED file(s) need manual review."
echo "now run: GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml && pnpm typecheck"
