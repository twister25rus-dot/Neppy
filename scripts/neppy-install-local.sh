#!/usr/bin/env bash
# Replace the installed Neppy.app with a locally built one, with automatic
# rollback. Started detached by Debug Mode ("Install and restart"); also usable
# by hand once the app is closed.
#
#   neppy-install-local.sh <new_app_path> <app_pid>
#
# 1. waits (60 s) for <app_pid> to exit, 2. copies the installed app into the
# backup dir (newest 3 kept), 3. swaps the new app in, 4. opens it and waits
# (60 s) for the launch marker the shell writes to clear. If the marker never
# clears, or a new last-failed-launch.json appears, the backup is restored and
# reopened. The verdict goes to last-local-install.json.
#
# Env (the real paths are only defaults; tests point these at a sandbox):
#   NEPPY_INSTALL_APPS_DIR    default /Applications        NEPPY_INSTALL_APP_NAME   default Neppy.app
#   NEPPY_INSTALL_STATE_DIR   default <data>/debug_mode   NEPPY_INSTALL_BACKUP_DIR default <state>/known-good-app
#   NEPPY_INSTALL_MARKER_DIR  default <data>               NEPPY_INSTALL_OPEN_CMD   default "open -a"
#   NEPPY_INSTALL_EXIT_TIMEOUT / NEPPY_INSTALL_READY_TIMEOUT  seconds, default 60
# <data> is NEPPY_WORKSPACE, else ~/.neppy (~/.neppy-staging with NEPPY_APP_ENV=staging).
set -euo pipefail
trap '' HUP

NEW_APP="${1:-}"; APP_PID="${2:-}"
DATA="${NEPPY_WORKSPACE:-${OPENHUMAN_WORKSPACE:-}}"
if [ -z "$DATA" ]; then
  DATA="$HOME/.neppy"; [ "${NEPPY_APP_ENV:-}" = "staging" ] && DATA="$HOME/.neppy-staging"
fi
APPS_DIR="${NEPPY_INSTALL_APPS_DIR:-/Applications}"
TARGET="$APPS_DIR/${NEPPY_INSTALL_APP_NAME:-Neppy.app}"
STATE_DIR="${NEPPY_INSTALL_STATE_DIR:-$DATA/debug_mode}"
BACKUP_DIR="${NEPPY_INSTALL_BACKUP_DIR:-$STATE_DIR/known-good-app}"
MARKER_DIR="${NEPPY_INSTALL_MARKER_DIR:-$DATA}"
OPEN_CMD="${NEPPY_INSTALL_OPEN_CMD:-open -a}"
EXIT_TIMEOUT="${NEPPY_INSTALL_EXIT_TIMEOUT:-60}"
READY_TIMEOUT="${NEPPY_INSTALL_READY_TIMEOUT:-60}"
RESULT="$STATE_DIR/last-local-install.json"
PENDING="$MARKER_DIR/launch-pending.json"
FAILED="$MARKER_DIR/last-failed-launch.json"
LOG="$STATE_DIR/local-install.log"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
NEW_VER=""; BACKUP=""

mkdir -p "$STATE_DIR"
exec >>"$LOG" 2>&1
log() { echo "[$(date -u +%Y-%m-%dT%H:%M:%SZ)] neppy-install-local: $*"; }

json_str() { printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | tr '\n' ' '; }

write_result() { # write_result <status> <reason>
  local tmp="$RESULT.$$.tmp"
  printf '{"status":"%s","version":"%s","backup":"%s","ts":"%s","reason":"%s"}\n' \
    "$1" "$(json_str "$NEW_VER")" "$(json_str "$BACKUP")" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    "$(json_str "$2")" >"$tmp"
  mv "$tmp" "$RESULT"
  log "result: $1 ($2)"
}

die() { log "error: $*"; write_result failed "$*"; exit 1; }

app_version() { # CFBundleShortVersionString of an app bundle (XML plist)
  awk '/CFBundleShortVersionString/{getline; gsub(/<[^>]*>|[ \t\r]/,""); print; exit}' \
    "$1/Contents/Info.plist" 2>/dev/null || true
}

copy_app() { # copy_app <src> <dest>: ditto keeps signatures and xattrs intact
  if command -v ditto >/dev/null 2>&1; then ditto "$1" "$2"; else cp -R "$1" "$2"; fi
}

# Put <src> at $TARGET via a temp copy, keeping the old one until the swap is done.
swap_in() {
  local src="$1" new="$APPS_DIR/.install-new.$$.app" old="$APPS_DIR/.install-old.$$.app"
  rm -rf "$new" "$old"
  copy_app "$src" "$new" || { rm -rf "$new"; return 1; }
  if [ -e "$TARGET" ]; then mv "$TARGET" "$old" || { rm -rf "$new"; return 1; }; fi
  if ! mv "$new" "$TARGET"; then
    [ -e "$old" ] && mv "$old" "$TARGET"
    return 1
  fi
  rm -rf "$old"
}

wait_for_exit() {
  case "$APP_PID" in ''|*[!0-9]*) die "invalid app pid '$APP_PID'";; esac
  SECONDS=0
  while kill -0 "$APP_PID" 2>/dev/null; do
    [ "$SECONDS" -lt "$EXIT_TIMEOUT" ] || die "the app (pid $APP_PID) did not quit within ${EXIT_TIMEOUT}s; nothing was changed"
    sleep 0.2
  done
}

backup_current() {
  [ -d "$TARGET" ] || { log "no installed app at $TARGET; nothing to back up"; return 0; }
  local ver; ver="$(app_version "$TARGET")"
  mkdir -p "$BACKUP_DIR"
  local dest="$BACKUP_DIR/Neppy-${ver:-unknown}-$STAMP.app"
  copy_app "$TARGET" "$dest.tmp" && mv "$dest.tmp" "$dest" \
    || { rm -rf "$dest.tmp"; die "could not back up the installed app; nothing was changed"; }
  BACKUP="$dest"
  # keep the newest 3
  { ls -dt "$BACKUP_DIR"/Neppy-*.app 2>/dev/null || true; } | tail -n +4 | while IFS= read -r old; do
    log "pruning old backup $old"; rm -rf "$old"
  done
  log "backed up $TARGET to $BACKUP"
}

launch_and_wait() { # 0 = reached ready, 1 = failed (reason in $WHY)
  local seen=0 before_failed=""
  [ -f "$FAILED" ] && before_failed="$(cat "$FAILED")"
  rm -f "$PENDING"  # a stale marker from the exited app would read as a failed launch
  # shellcheck disable=SC2086
  $OPEN_CMD "$TARGET" || { WHY="could not open the new app"; return 1; }
  SECONDS=0
  while [ "$SECONDS" -lt "$READY_TIMEOUT" ]; do
    if [ -f "$FAILED" ] && [ "$(cat "$FAILED")" != "$before_failed" ]; then
      WHY="the new app recorded a failed launch"; return 1
    fi
    if [ -f "$PENDING" ]; then seen=1
    elif [ "$seen" = 1 ]; then return 0; fi
    sleep 0.2
  done
  if [ "$seen" = 1 ]; then WHY="the launch marker never cleared within ${READY_TIMEOUT}s"
  else WHY="the new app never wrote a launch marker within ${READY_TIMEOUT}s"; fi
  return 1
}

main() {
  [ -f "$NEW_APP/Contents/Info.plist" ] || die "not an app bundle: '$NEW_APP'"
  [ "$NEW_APP" != "$TARGET" ] || die "the new app is the installed app"
  NEW_VER="$(app_version "$NEW_APP")"
  log "installing $NEW_APP (version ${NEW_VER:-?}) over $TARGET; waiting for pid $APP_PID"
  wait_for_exit
  backup_current
  swap_in "$NEW_APP" || die "could not install the new app; the previous app is still in place"
  WHY=""
  if launch_and_wait; then write_result installed "ok"; return 0; fi
  log "new app failed: $WHY; restoring"
  # shellcheck disable=SC2009
  pkill -f "$TARGET/Contents/MacOS" 2>/dev/null || true
  sleep 1
  if [ -z "$BACKUP" ]; then die "$WHY; and there is no backup to restore"; fi
  swap_in "$BACKUP" || die "$WHY; and the backup could not be restored (it is at $BACKUP)"
  rm -f "$PENDING"
  # shellcheck disable=SC2086
  $OPEN_CMD "$TARGET" || true
  write_result restored "$WHY"
}

main
