#!/usr/bin/env bash
# Neppy recovery tool (Debug Mode, spec section 37).
#
# Deliberately small and standalone: plain bash + git, no jq, no dependency on
# the app, Rust or node. It lives outside the app runtime and Debug Mode refuses
# to modify it. Never runs `reset --hard` or `clean`.
#
#   neppy-recover.sh status              launch markers, recent debug tasks, checkpoints
#   neppy-recover.sh logs [lines]        tail the newest app/core logs (default 80)
#   neppy-recover.sh restore <id>        restore a checkpoint (asks y/N, saves a safety ref first)
#   neppy-recover.sh known-good          list saved known-good neppy-core binaries
#   neppy-recover.sh run-known-good [p]  run the newest (or given) binary with `serve`
#
# Env: NEPPY_WORKSPACE (or OPENHUMAN_WORKSPACE) selects the workspace;
#      NEPPY_RECOVER_REPO overrides the repo (default: parent of scripts/).
set -euo pipefail

CP_PREFIX="refs/neppy-debug/checkpoints"
REC_PREFIX="refs/neppy-debug/recovery"

die() { echo "neppy-recover: $*" >&2; exit 1; }

repo_root() {
  local r="${NEPPY_RECOVER_REPO:-}"
  [ -n "$r" ] || r="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  git -C "$r" rev-parse --show-toplevel 2>/dev/null || die "not a git repository: $r"
}

# Data dir: where logs/ and the launch markers live.
data_dir() {
  local d="${NEPPY_WORKSPACE:-${OPENHUMAN_WORKSPACE:-}}"
  if [ -z "$d" ]; then
    d="$HOME/.neppy"
    [ "${NEPPY_APP_ENV:-}" = "staging" ] && d="$HOME/.neppy-staging"
  fi
  echo "$d"
}

# Workspace dir holding debug_mode/: the override, else the newest
# ~/.neppy/users/*/workspace.
workspace_dir() {
  local w="${NEPPY_WORKSPACE:-${OPENHUMAN_WORKSPACE:-}}" c
  if [ -n "$w" ]; then
    for c in "$w" "$w/workspace"; do
      if [ -d "$c/debug_mode" ]; then echo "$c"; return; fi
    done
    echo "$w"; return
  fi
  c="$(ls -dt "$(data_dir)"/users/*/workspace 2>/dev/null | head -n 1 || true)"
  echo "${c:-$(data_dir)/workspace}"
}

json_field() { # json_field <file> <key>: first string/number value of "key"
  sed -n "s/.*\"$2\": *\"\{0,1\}\([^\",}]*\)\"\{0,1\}.*/\1/p" "$1" 2>/dev/null | head -n 1
}

show_marker() { # show_marker <title> <file>
  if [ -f "$2" ]; then
    echo "$1: pid=$(json_field "$2" pid) version=$(json_field "$2" version)" \
      "started_at=$(json_field "$2" started_at) detected_at=$(json_field "$2" detected_at)"
  else
    echo "$1: none"
  fi
}

cmd_status() {
  local data ws repo hist
  data="$(data_dir)"; ws="$(workspace_dir)"; repo="$(repo_root)"; hist="$ws/debug_mode/history.json"
  echo "repo:      $repo"
  echo "workspace: $ws"
  echo "data dir:  $data"
  echo
  show_marker "launch pending (a launch that never reached ready)" "$data/launch-pending.json"
  show_marker "last failed launch" "$data/last-failed-launch.json"
  echo
  echo "Latest debug tasks (id  status  request):"
  if [ -f "$hist" ]; then
    awk -F'"' '/^    "id":/{id=$4} /^    "request":/{req=$4} /^    "status":/{print "  " id "  " $4 "  " substr(req,1,60)}' "$hist" | tail -n 5
  else
    echo "  (no history at $hist)"
  fi
  echo
  echo "Checkpoints (newest first):"
  git -C "$repo" for-each-ref --sort=-creatordate \
    --format='  %(creatordate:short) %(refname:strip=3)' "$CP_PREFIX/" 2>/dev/null \
    | grep -v -- '-index$' | sed -n '1,15p' || true
}

cmd_logs() {
  local n="${1:-80}" dir f found=0
  case "$n" in ''|*[!0-9]*) die "lines must be a number";; esac
  dir="$(data_dir)/logs"
  [ -d "$dir" ] || die "no log directory at $dir"
  for f in $(ls -t "$dir"/* 2>/dev/null | head -n 2); do
    found=1; echo "==== $f ===="; tail -n "$n" "$f"
  done
  [ "$found" = 1 ] || echo "no log files in $dir"
}

safety_ref() { # pin the current tree under refs/neppy-debug/recovery/<ts> via a temp index
  local repo="$1" ts tmp idx tree itree head commit
  ts="$(date -u +%Y%m%dT%H%M%SZ)"
  tmp="$(mktemp -d)"; idx="$tmp/index"
  local real; real="$(git -C "$repo" rev-parse --git-path index)"
  case "$real" in /*) ;; *) real="$repo/$real";; esac
  if [ -f "$real" ]; then cp "$real" "$idx"; fi
  itree="$(GIT_INDEX_FILE="$idx" git -C "$repo" write-tree)"
  GIT_INDEX_FILE="$idx" git -C "$repo" add -A
  tree="$(GIT_INDEX_FILE="$idx" git -C "$repo" write-tree)"
  head="$(git -C "$repo" rev-parse -q --verify HEAD || true)"
  commit="$(git -C "$repo" -c user.name=neppy-recover -c user.email=recover@neppy.local \
    -c commit.gpgsign=false commit-tree "$tree" ${head:+-p "$head"} -m "neppy-recover safety snapshot $ts")"
  git -C "$repo" update-ref "$REC_PREFIX/$ts" "$commit"
  git -C "$repo" update-ref "$REC_PREFIX/$ts-index" "$itree"
  rm -rf "$tmp"
  echo "$REC_PREFIX/$ts"
}

cmd_restore() {
  local id="${1:-}" repo ref answer saved
  [ -n "$id" ] || die "usage: restore <checkpoint-id>"
  case "$id" in *[!A-Za-z0-9._-]*|-*|*..*) die "invalid checkpoint id";; esac
  repo="$(repo_root)"; ref="$CP_PREFIX/$id"
  git -C "$repo" rev-parse -q --verify "$ref^{commit}" >/dev/null || die "unknown checkpoint: $id"
  echo "This restores the working tree of $repo to checkpoint $id."
  echo "Your current state is saved first under $REC_PREFIX/. Untracked files are never deleted."
  printf 'Continue? [y/N] '
  read -r answer || answer=""
  case "$answer" in y|Y|yes|YES) ;; *) echo "aborted"; return 1;; esac
  saved="$(safety_ref "$repo")"
  echo "saved current state: $saved"
  git -C "$repo" restore --source="$ref" --worktree -- . || die "restore failed; your state is at $saved"
  if git -C "$repo" rev-parse -q --verify "$ref-index^{tree}" >/dev/null; then
    git -C "$repo" read-tree "$ref-index"
  fi
  echo "restored checkpoint $id (undo with: git restore --source=$saved --worktree -- .)"
}

known_good_files() { ls -t "$(workspace_dir)"/debug_mode/known_good/neppy-core-* 2>/dev/null || true; }

cmd_known_good() {
  local list; list="$(known_good_files)"
  if [ -z "$list" ]; then echo "no saved known-good binaries in $(workspace_dir)/debug_mode/known_good"; return; fi
  echo "$list"
}

cmd_run_known_good() {
  local bin="${1:-}"
  [ -n "$bin" ] || bin="$(known_good_files | head -n 1)"
  [ -n "$bin" ] || die "no known-good binary saved yet"
  [ -x "$bin" ] || die "not executable: $bin"
  echo "starting $bin serve"
  exec "$bin" serve
}

main() {
  local sub="${1:-}"; [ $# -gt 0 ] && shift
  case "$sub" in
    status) cmd_status ;;
    logs) cmd_logs "$@" ;;
    restore) cmd_restore "$@" ;;
    known-good) cmd_known_good ;;
    run-known-good) cmd_run_known_good "$@" ;;
    ''|-h|--help|help) sed -n '2,15p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' ;;
    *) die "unknown command '$sub' (try --help)" ;;
  esac
}

# Wrapped so bash has read the whole script before `restore` can rewrite it.
main "$@"
exit $?
