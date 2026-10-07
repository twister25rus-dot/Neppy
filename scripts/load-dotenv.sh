#!/usr/bin/env bash
# Load .env file into environment variables.
# Usage:
#   source scripts/load-dotenv.sh [path/to/.env]
#   eval "$(scripts/load-dotenv.sh [path/to/.env])"
# Default path: .env (project root when run from repo root)
#
# Legacy names: OPENHUMAN_* / VITE_OPENHUMAN_* are still honoured. After the
# file is loaded, every OPENHUMAN_X (from the file or already exported) that has
# no NEPPY_X counterpart gets one, so the rest of the tooling can read only the
# canonical NEPPY_* names. A NEPPY_X that is set always wins.

set -e
FILE="${1:-.env}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
RESOLVED="${1:+$1}"
RESOLVED="${RESOLVED:-$ROOT_DIR/.env}"

if [[ ! -f "$RESOLVED" ]]; then
  echo "File not found: $RESOLVED" >&2
  exit 1
fi

exports=()
while IFS= read -r line || [[ -n "$line" ]]; do
  line="${line%%#*}"
  line="${line#"${line%%[![:space:]]*}"}"
  line="${line%"${line##*[![:space:]]}"}"
  [[ -z "$line" ]] && continue
  if [[ "$line" == export\ * ]]; then
    line="${line#export }"
  fi
  if [[ "$line" == *"="* ]]; then
    key="${line%%=*}"
    key="${key%"${key##*[![:space:]]}"}"
    value="${line#*=}"
    value="${value#\"}"
    value="${value%\"}"
    value="${value#\'}"
    value="${value%\'}"
    [[ -n "$key" ]] && exports+=("$(printf 'export %s=%q' "$key" "$value")")
  fi
done < "$RESOLVED"

# --- legacy-name shim: OPENHUMAN_X -> NEPPY_X, VITE_OPENHUMAN_X -> VITE_NEPPY_X ---
# (bash 3.2 compatible: no associative arrays.)
file_keys=" "
for e in "${exports[@]}"; do
  k="${e#export }"
  k="${k%%=*}"
  file_keys+="$k "
done

_legacy_to_canonical() {
  case "$1" in
    VITE_OPENHUMAN_?*) echo "VITE_NEPPY_${1#VITE_OPENHUMAN_}" ;;
    OPENHUMAN_?*) echo "NEPPY_${1#OPENHUMAN_}" ;;
  esac
}

shim=()
shim_seen=" "
# File-defined legacy names first (the file wins over the ambient environment),
# then ambient ones.
for e in "${exports[@]}"; do
  k="${e#export }"
  k="${k%%=*}"
  canon="$(_legacy_to_canonical "$k")"
  [[ -z "$canon" ]] && continue
  [[ "$file_keys" == *" $canon "* ]] && continue
  [[ "$shim_seen" == *" $canon "* ]] && continue
  shim_seen+="$canon "
  shim+=("${e/#export $k=/export $canon=}")
done
while IFS= read -r k; do
  canon="$(_legacy_to_canonical "$k")"
  [[ -z "$canon" ]] && continue
  [[ "$file_keys" == *" $canon "* ]] && continue
  [[ "$shim_seen" == *" $canon "* ]] && continue
  [[ -n "${!canon+x}" ]] && continue
  shim_seen+="$canon "
  shim+=("$(printf 'export %s=%q' "$canon" "${!k}")")
done < <(compgen -e | grep -E '^(VITE_)?OPENHUMAN_' || true)
if [[ ${#shim[@]} -gt 0 ]]; then
  exports+=("${shim[@]}")
fi

if [[ ${#exports[@]} -eq 0 ]]; then
  joined=""
else
  joined=$(printf '%s\n' "${exports[@]}")
fi

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  echo "$joined"
else
  eval "$joined"
fi
