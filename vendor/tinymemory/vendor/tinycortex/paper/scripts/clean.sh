#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if command -v latexmk >/dev/null 2>&1; then
  latexmk -C -output-directory=out main.tex || true
fi

rm -f out/arxiv-source.tar.gz
rm -f out/main.pdf out/main.aux out/main.bbl out/main.blg out/main.log out/main.out
rm -f out/main.html out/main.md

echo "Clean complete."
