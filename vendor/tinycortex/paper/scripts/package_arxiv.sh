#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

mkdir -p out

INCLUDE_ITEMS=(
  "main.tex"
  "references.bib"
)

if [ -d "figures" ]; then
  INCLUDE_ITEMS+=("figures")
fi

if [ -f "README.md" ]; then
  INCLUDE_ITEMS+=("README.md")
fi

tar -czf out/arxiv-source.tar.gz "${INCLUDE_ITEMS[@]}"

echo "Created: $ROOT_DIR/out/arxiv-source.tar.gz"
